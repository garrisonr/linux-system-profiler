//! Elevation routing — getting pkexec in front of the session's graphical
//! polkit agent no matter how this process was launched.
//!
//! Diagnosis (G9, 2026-10-14, v0.8.1 hotfix): GNOME Shell registers itself
//! as the polkit authentication agent for the user's logind session — and
//! ONLY that session. pkexec asks polkitd for the agent of the CALLER's
//! session. A GUI process launched from anywhere else (an SSH shell, a
//! debugger, a different VT) belongs to a foreign session with no agent, so
//! pkexec falls back to its built-in TEXTUAL agent — which needs a
//! controlling terminal a GUI child doesn't have — and dies instantly:
//! "Error creating textual authentication agent: Error opening current
//! controlling terminal for the process ('/dev/tty')". The process
//! environment was never the problem (DISPLAY & friends were all set);
//! session membership was.
//!
//! The vehicle that works: a transient user SERVICE
//! (`systemd-run --user --wait --pipe`). Services are children of the
//! user's session manager, so they inherit the graphical login's audit
//! session identity, polkitd maps them to the desktop session, and the
//! shell's graphical agent answers. Verified live on the G9: direct pkexec
//! from a foreign session failed in <1 s (exit 127); the same pkexec via a
//! transient user service waited on the agent and exited 0.
//! (`systemd-run --scope` does NOT work: the scope stays in the caller's
//! process tree with a fresh audit identity.)
//!
//! Routing rule: graphical session + no controlling terminal (the GUI
//! case) + systemd-run + pkexec ⇒ wrap. Everything else ⇒ direct pkexec,
//! exactly as before — a terminal keeps its fast path, headless callers
//! keep sudo semantics (callers only route pkexec invocations here).
use crate::collectors::which;
use std::io::IsTerminal;

/// Session variables the elevated child needs to reach the same desktop
/// the user is looking at. Passed explicitly through the service boundary
/// (services start with a scrubbed environment), in this fixed order.
pub const SESSION_ENV_KEYS: &[&str] = &[
    "DISPLAY",
    "WAYLAND_DISPLAY",
    "DBUS_SESSION_BUS_ADDRESS",
    "XDG_RUNTIME_DIR",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ElevationContext {
    /// A desktop is advertised (DISPLAY or WAYLAND_DISPLAY set).
    pub graphical: bool,
    /// stdin is a terminal — pkexec's textual fallback could work, and a
    /// same-session terminal launch reaches the agent directly anyway.
    pub interactive_tty: bool,
    pub systemd_run: bool,
    pub pkexec: bool,
}

impl ElevationContext {
    pub fn detect() -> Self {
        Self {
            graphical: std::env::var_os("WAYLAND_DISPLAY").is_some()
                || std::env::var_os("DISPLAY").is_some(),
            interactive_tty: std::io::stdin().is_terminal(),
            systemd_run: which("systemd-run"),
            pkexec: which("pkexec"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElevationRoute {
    /// Spawn pkexec as a direct child (historical behavior).
    Direct,
    /// Spawn pkexec inside a transient user service so it inherits the
    /// graphical session's identity and reaches its polkit agent.
    UserSession,
}

pub fn route(ctx: &ElevationContext) -> ElevationRoute {
    if ctx.graphical && !ctx.interactive_tty && ctx.systemd_run && ctx.pkexec {
        ElevationRoute::UserSession
    } else {
        ElevationRoute::Direct
    }
}

/// Build the (program, argv) that executes `pkexec <pkexec_args…>` via the
/// routed path. `env` is the caller's session environment (name/value
/// pairs); only SESSION_ENV_KEYS are forwarded, in canonical order.
pub fn command_for(
    pkexec_args: &[String],
    ctx: &ElevationContext,
    env: &[(String, String)],
) -> (String, Vec<String>) {
    match route(ctx) {
        ElevationRoute::Direct => ("pkexec".to_string(), pkexec_args.to_vec()),
        ElevationRoute::UserSession => {
            let mut a: Vec<String> = ["--user", "--wait", "--pipe", "--collect", "--quiet"]
                .iter()
                .map(|s| s.to_string())
                .collect();
            for key in SESSION_ENV_KEYS {
                if let Some((_, v)) = env.iter().find(|(k, _)| k == key) {
                    a.push(format!("--setenv={key}={v}"));
                }
            }
            a.push("pkexec".to_string());
            a.extend(pkexec_args.iter().cloned());
            ("systemd-run".to_string(), a)
        }
    }
}

/// The caller's current session environment (present keys only).
pub fn current_env() -> Vec<(String, String)> {
    SESSION_ENV_KEYS
        .iter()
        .filter_map(|k| std::env::var(k).ok().map(|v| (k.to_string(), v)))
        .collect()
}

/// Convenience for call sites: detect context + env, return the runnable
/// (program, argv) for `pkexec <pkexec_args…>`.
pub fn run_command(pkexec_args: &[String]) -> (String, Vec<String>) {
    command_for(pkexec_args, &ElevationContext::detect(), &current_env())
}

/// Standalone graphical authentication agents, in preference order, for
/// the self-heal path. GNOME needs none (the shell is the agent); these
/// cover the other desktops and post-shell-crash sessions where the
/// session's agent died and never came back (Dickles' 20:00 OOM case).
pub const AGENT_CANDIDATES: &[&str] = &[
    "/usr/lib/policykit-1-gnome/polkit-gnome-authentication-agent-1",
    "/usr/libexec/polkit-gnome-authentication-agent-1",
    "/usr/lib/polkit-gnome/polkit-gnome-authentication-agent-1",
    "/usr/libexec/polkit-mate-authentication-agent-1",
    "/usr/libexec/polkit-kde-authentication-agent-1",
    "/usr/lib/x86_64-linux-gnu/libexec/polkit-kde-authentication-agent-1",
    "/usr/bin/lxqt-policykit-agent",
    "/usr/libexec/agent-polkit",
];

/// First candidate that exists on this system (pure — testable).
pub fn agent_candidate(exists: &dyn Fn(&str) -> bool) -> Option<&'static str> {
    AGENT_CANDIDATES.iter().copied().find(|p| exists(p))
}

/// Whether a failed elevation earns the self-heal retry: only the
/// agent-unavailable signature, and only once.
pub fn should_heal(kind: ElevationFailure, already_retried: bool) -> bool {
    kind == ElevationFailure::AgentUnavailable && !already_retried
}

/// Best-effort: start the session's graphical authentication agent,
/// detached, with our GUI environment, so it registers with polkitd for
/// OUR session and can render its prompt on the desktop we're displaying
/// to. Returns true if an agent binary was found and spawned. (If one is
/// already registered for the session the newcomer exits harmlessly.)
pub fn try_start_agent() -> bool {
    let Some(path) = agent_candidate(&|p| std::path::Path::new(p).exists()) else {
        return false;
    };
    let mut cmd = std::process::Command::new("setsid");
    cmd.arg(path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    for (k, v) in current_env() {
        cmd.env(k, v);
    }
    match cmd.spawn() {
        Ok(_) => {
            // Give the agent a beat to register with polkitd.
            std::thread::sleep(std::time::Duration::from_millis(700));
            true
        }
        Err(_) => false,
    }
}

/// Run `pkexec <pkexec_args…>` to completion via the routed command,
/// with the self-heal retry: if the attempt fails with the
/// agent-unavailable signature, start the graphical agent into our own
/// session and retry ONCE with a direct pkexec (caller and agent then
/// share a session). Returns the final attempt's output.
pub fn output(pkexec_args: &[String]) -> std::io::Result<std::process::Output> {
    let (prog, args) = run_command(pkexec_args);
    let out = std::process::Command::new(&prog).args(&args).output()?;
    if out.status.success() {
        return Ok(out);
    }
    let text = format!(
        "{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    if should_heal(classify(&text, out.status.code()), false) && try_start_agent() {
        return std::process::Command::new("pkexec").args(pkexec_args).output();
    }
    Ok(out)
}

/// Why an elevated action did not complete, classified from its combined
/// output + exit code so the UI can lead with plain language and demote
/// the raw text to a details section.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElevationFailure {
    /// No authentication agent could show the sign-in prompt (pkexec's
    /// textual fallback died on /dev/tty, or no agent is registered).
    AgentUnavailable,
    /// The user dismissed or was denied at the prompt (pkexec exit 126).
    Dismissed,
    /// The elevated command ran (or failed to start) for another reason.
    Failed,
}

pub fn classify(output: &str, code: Option<i32>) -> ElevationFailure {
    let t = output.to_lowercase();
    if t.contains("textual authentication agent")
        || t.contains("/dev/tty")
        || t.contains("no authentication agent")
        || t.contains("error creating authentication agent")
    {
        return ElevationFailure::AgentUnavailable;
    }
    if code == Some(126) || t.contains("dismissed") || t.contains("not authorized") {
        return ElevationFailure::Dismissed;
    }
    ElevationFailure::Failed
}

impl ElevationFailure {
    /// The plain-language headline shown FIRST, before any raw output.
    /// Never contains tool jargon.
    pub fn headline(&self) -> &'static str {
        match self {
            ElevationFailure::AgentUnavailable => {
                "Couldn't get administrator permission — the sign-in prompt couldn't be shown. Nothing was changed."
            }
            ElevationFailure::Dismissed => {
                "Administrator sign-in was cancelled — nothing was changed."
            }
            ElevationFailure::Failed => {
                "The administrator action didn't finish — the details below show where it stopped."
            }
        }
    }
}

/// The failure text handed to the frontend: plain-language headline first,
/// raw output demoted behind a Details divider.
pub fn failure_text(output: &str, code: Option<i32>) -> String {
    let kind = classify(output, code);
    let detail = output.trim();
    if detail.is_empty() {
        kind.headline().to_string()
    } else {
        format!("{}\n\n—— Details ——\n{}", kind.headline(), detail)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TTY_ERROR: &str = "Error creating textual authentication agent: Error opening current controlling terminal for the process (`/dev/tty'): No such device or address";

    fn ctx(graphical: bool, tty: bool, sdrun: bool, pkexec: bool) -> ElevationContext {
        ElevationContext { graphical, interactive_tty: tty, systemd_run: sdrun, pkexec }
    }

    #[test]
    fn route_matrix() {
        // The GUI case (v0.8.1 failure): graphical, no TTY → user session.
        assert_eq!(route(&ctx(true, false, true, true)), ElevationRoute::UserSession);
        // A terminal launch keeps the direct fast path.
        assert_eq!(route(&ctx(true, true, true, true)), ElevationRoute::Direct);
        // Headless: never wrap.
        assert_eq!(route(&ctx(false, false, true, true)), ElevationRoute::Direct);
        assert_eq!(route(&ctx(false, true, true, true)), ElevationRoute::Direct);
        // Missing tools degrade to direct (today's behavior), never panic.
        assert_eq!(route(&ctx(true, false, false, true)), ElevationRoute::Direct);
        assert_eq!(route(&ctx(true, false, true, false)), ElevationRoute::Direct);
    }

    #[test]
    fn direct_command_is_plain_pkexec() {
        let (prog, args) = command_for(
            &["apt-get".into(), "update".into()],
            &ctx(true, true, true, true),
            &[],
        );
        assert_eq!(prog, "pkexec");
        assert_eq!(args, vec!["apt-get".to_string(), "update".to_string()]);
    }

    #[test]
    fn user_session_command_wraps_with_env_passthrough() {
        let env = vec![
            ("XDG_RUNTIME_DIR".to_string(), "/run/user/1000".to_string()),
            ("DISPLAY".to_string(), ":0".to_string()),
            ("UNRELATED".to_string(), "nope".to_string()),
        ];
        let (prog, args) = command_for(
            &["sh".into(), "-c".into(), "apt-get upgrade -y".into()],
            &ctx(true, false, true, true),
            &env,
        );
        assert_eq!(prog, "systemd-run");
        // Canonical key order (DISPLAY before XDG_RUNTIME_DIR), unrelated
        // variables NOT forwarded, pkexec + payload last and intact.
        assert_eq!(args, vec![
            "--user", "--wait", "--pipe", "--collect", "--quiet",
            "--setenv=DISPLAY=:0",
            "--setenv=XDG_RUNTIME_DIR=/run/user/1000",
            "pkexec", "sh", "-c", "apt-get upgrade -y",
        ]);
    }

    #[test]
    fn classify_tonights_failure_as_agent_unavailable() {
        assert_eq!(classify(TTY_ERROR, Some(127)), ElevationFailure::AgentUnavailable);
        // The exact failure must map to the exact plain-language headline
        // Dickles approved — pinned verbatim.
        assert_eq!(
            failure_text(TTY_ERROR, Some(127)),
            format!(
                "Couldn't get administrator permission — the sign-in prompt couldn't be shown. Nothing was changed.\n\n—— Details ——\n{TTY_ERROR}"
            )
        );
    }

    #[test]
    fn classify_dismissal_and_plain_failures() {
        assert_eq!(classify("", Some(126)), ElevationFailure::Dismissed);
        assert_eq!(
            classify("Error executing command as another user: Not authorized", Some(126)),
            ElevationFailure::Dismissed
        );
        assert_eq!(
            classify("E: Unable to locate package foo", Some(100)),
            ElevationFailure::Failed
        );
        assert_eq!(classify("", None), ElevationFailure::Failed);
        // Empty output → headline only, no dangling Details divider.
        assert!(!failure_text("", Some(126)).contains("Details"));
    }

    #[test]
    fn agent_candidate_picks_first_existing() {
        // Nothing installed (the G9's GNOME case) → no heal spawn.
        assert_eq!(agent_candidate(&|_| false), None);
        // MATE-only system → the MATE agent, not an earlier absent path.
        assert_eq!(
            agent_candidate(&|p| p.contains("mate")),
            Some("/usr/libexec/polkit-mate-authentication-agent-1")
        );
        // GNOME agent present → preferred over later candidates.
        assert_eq!(
            agent_candidate(&|p| p.contains("gnome") || p.contains("mate")),
            Some("/usr/lib/policykit-1-gnome/polkit-gnome-authentication-agent-1")
        );
    }

    #[test]
    fn heal_only_for_agent_failures_and_only_once() {
        assert!(should_heal(ElevationFailure::AgentUnavailable, false));
        assert!(!should_heal(ElevationFailure::AgentUnavailable, true));
        assert!(!should_heal(ElevationFailure::Dismissed, false));
        assert!(!should_heal(ElevationFailure::Failed, false));
    }

    #[test]
    fn headlines_carry_no_jargon() {
        for kind in [
            ElevationFailure::AgentUnavailable,
            ElevationFailure::Dismissed,
            ElevationFailure::Failed,
        ] {
            let h = kind.headline().to_lowercase();
            for jargon in ["textual", "/dev/tty", "polkit", "pkexec", "stderr"] {
                assert!(!h.contains(jargon), "headline leaks jargon {jargon}: {h}");
            }
        }
    }

    /// Live acceptance (manual): run on a desktop as the session user —
    /// `cargo test -p linux-system-profiler live_elevated -- --ignored`.
    /// Success OR a dismissal both pass (the agent engaged); the v0.8.1
    /// failure mode (AgentUnavailable) fails the test.
    #[test]
    #[ignore = "live: engages the session's graphical polkit prompt"]
    fn live_elevated_true_roundtrip() {
        let (prog, args) = run_command(&["/bin/true".to_string()]);
        let out = std::process::Command::new(&prog)
            .args(&args)
            .output()
            .expect("spawn elevated /bin/true");
        let text = format!(
            "{}\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        assert_ne!(
            classify(&text, out.status.code()),
            ElevationFailure::AgentUnavailable,
            "the graphical agent must be reachable: {text}"
        );
    }
}
