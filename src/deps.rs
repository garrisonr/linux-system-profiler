//! Dependency self-healing — Dickles' rule (2026-10-13): never ship
//! half-broken software that silently degrades when a probe tool is missing.
//!
//! The profiler's collectors lean on a small set of standard tools. If any are
//! missing, the profiler (a) says so loudly, and (b) installs them itself via
//! the distro package manager, elevated with pkexec (GUI/polkit) or sudo.
//! Installs are never silent about WHAT is being installed: the plan is
//! printed/logged first, and `--no-deps` opts out entirely.
use crate::collectors::which;
use serde::{Deserialize, Serialize};
use std::process::Command;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DepTool {
    pub binary: &'static str,
    pub feature: &'static str,
    pub pacman: &'static str,
    pub apt: &'static str,
    pub dnf: &'static str,
}

pub const DEP_TOOLS: &[DepTool] = &[
    DepTool { binary: "lsusb", feature: "USB device inventory", pacman: "usbutils", apt: "usbutils", dnf: "usbutils" },
    DepTool { binary: "smartctl", feature: "SMART health, temperature, NVMe endurance", pacman: "smartmontools", apt: "smartmontools", dnf: "smartmontools" },
    DepTool { binary: "lspci", feature: "PCI device names + driver-in-use", pacman: "pciutils", apt: "pciutils", dnf: "pciutils" },
    DepTool { binary: "fwupdmgr", feature: "Firmware updates via LVFS", pacman: "fwupd", apt: "fwupd", dnf: "fwupd" },
    DepTool { binary: "checkupdates", feature: "Sync-free Arch update check", pacman: "pacman-contrib", apt: "", dnf: "" },
    DepTool { binary: "dmidecode", feature: "Memory module details + system serial (SMBIOS)", pacman: "dmidecode", apt: "dmidecode", dnf: "dmidecode" },
];

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DepStatus {
    pub missing: Vec<String>,       // binaries
    pub missing_features: Vec<String>,
    pub package_manager: Option<String>,
    pub packages_to_install: Vec<String>,
    /// Features whose helper tool has no package on this distro because the
    /// distro's own tooling covers the job (e.g. checkupdates is Arch-only —
    /// apt/dnf check updates natively). Reported honestly as "not needed
    /// here" instead of the old behavior: an empty package string and
    /// silence (Gemini audit 2026-10-14).
    #[serde(default)]
    pub not_needed: Vec<String>,
}

pub fn dep_status() -> DepStatus {
    let pm = if which("pacman") { Some("pacman".to_string()) }
        else if which("apt") { Some("apt".to_string()) }
        else if which("dnf") { Some("dnf".to_string()) } else { None };
    let mut st = DepStatus { package_manager: pm.clone(), ..Default::default() };
    for t in DEP_TOOLS {
        if !which(t.binary) {
            let pkg = match pm.as_deref() {
                Some("pacman") => t.pacman, Some("apt") => t.apt, Some("dnf") => t.dnf, _ => "",
            };
            if !pkg.is_empty() {
                st.missing.push(t.binary.to_string());
                st.missing_features.push(t.feature.to_string());
                if !st.packages_to_install.contains(&pkg.to_string()) {
                    st.packages_to_install.push(pkg.to_string());
                }
            } else if pm.is_some() {
                // Known distro, empty cell: the tool isn't missing, it's
                // unnecessary here — say so instead of dropping it silently.
                st.not_needed.push(format!("{} ({})", t.feature, t.binary));
            }
        }
    }
    st
}

/// Build the (elevation, argv) install command for the detected distro.
/// pacman syncs ONLY the needed packages (`-S --needed`), never a full `-Syu`
/// upgrade — installing probe tools must never upgrade the user's system.
pub fn install_command(st: &DepStatus) -> Option<(String, Vec<String>)> {
    if st.packages_to_install.is_empty() { return None; }
    // pkexec needs a polkit agent, which only exists in a graphical session.
    // In a GUI session use pkexec (native password prompt); headless/SSH, sudo.
    let graphical = std::env::var("WAYLAND_DISPLAY").is_ok() || std::env::var("DISPLAY").is_ok();
    let elevate = if graphical && which("pkexec") { "pkexec" }
        else if which("sudo") { "sudo" }
        else if which("pkexec") { "pkexec" } else { return None };
    let args: Vec<String> = match st.package_manager.as_deref() {
        Some("pacman") => {
            let mut a = vec!["pacman".into(), "-S".into(), "--needed".into(), "--noconfirm".into()];
            a.extend(st.packages_to_install.iter().cloned()); a
        }
        Some("apt") => {
            // Chain a metadata refresh first: on stale package lists a bare
            // `apt-get install` dies with "Unable to locate package"
            // (Gemini audit 2026-10-14). Runs under `sh -c` beneath the
            // elevation tool, same shape as the updates apply path.
            let pkgs = st.packages_to_install.join(" ");
            vec!["sh".into(), "-c".into(),
                 format!("DEBIAN_FRONTEND=noninteractive apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y {pkgs}")]
        }
        Some("dnf") => {
            let mut a = vec!["dnf".into(), "install".into(), "-y".into()];
            a.extend(st.packages_to_install.iter().cloned()); a
        }
        _ => return None,
    };
    Some((elevate.to_string(), args))
}

/// Why an install did not complete — classified so callers can respond
/// calmly and specifically instead of dumping raw tool output on the user.
/// Dickles' law (2026-10-14, strengthened): no published software may EVER
/// show a user a dependency error. Raw pkexec/package-manager stderr lives
/// only in `detail`, behind an explicit details view — never in `message`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallErrorKind {
    /// No graphical authentication agent could be reached (classic case:
    /// the app was launched detached from the user's login session, so
    /// pkexec fell back to a textual agent with no terminal to talk to).
    NoAgent,
    /// The user dismissed or denied the password prompt.
    Cancelled,
    /// No supported package manager / elevation tool on this system.
    Unavailable,
    /// The install ran but did not finish.
    Failed,
}

impl InstallErrorKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            InstallErrorKind::NoAgent => "no-agent",
            InstallErrorKind::Cancelled => "cancelled",
            InstallErrorKind::Unavailable => "unavailable",
            InstallErrorKind::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone)]
pub struct InstallError {
    pub kind: InstallErrorKind,
    /// Calm, user-facing summary. Never contains raw tool output.
    pub message: String,
    /// Technical detail for an explicit details/log view only.
    pub detail: String,
}

impl std::fmt::Display for InstallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}
impl std::error::Error for InstallError {}

/// The manual equivalent of the automated install, for the details view —
/// if the app can't elevate from this session, the user can still run one
/// command in a terminal instead of being stranded.
pub fn manual_command(st: &DepStatus) -> Option<String> {
    if st.packages_to_install.is_empty() { return None; }
    let pkgs = st.packages_to_install.join(" ");
    match st.package_manager.as_deref() {
        Some("pacman") => Some(format!("sudo pacman -S --needed {pkgs}")),
        Some("apt") => Some(format!("sudo apt-get update && sudo apt-get install -y {pkgs}")),
        Some("dnf") => Some(format!("sudo dnf install -y {pkgs}")),
        _ => None,
    }
}

fn tail_lines(text: &str, n: usize, max_chars: usize) -> String {
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    let mut out = lines[lines.len().saturating_sub(n)..].join("\n");
    if out.chars().count() > max_chars {
        out = out.chars().rev().take(max_chars).collect::<String>()
            .chars().rev().collect();
    }
    out
}

/// Run the install. On success returns a human log of what happened.
/// On failure returns a classified InstallError: `message` is always calm
/// and safe to show; raw tool output is confined to `detail`.
/// In the GUI this runs behind the polkit prompt pkexec shows; in the CLI it
/// uses pkexec when present, else sudo (which may prompt for a password).
pub fn install_deps(st: &DepStatus) -> Result<String, InstallError> {
    let (elevate, args) = install_command(st).ok_or_else(|| InstallError {
        kind: InstallErrorKind::Unavailable,
        message: "These tools can't be installed automatically on this system.".into(),
        detail: "No supported package manager (pacman/apt/dnf) or elevation tool (pkexec/sudo) was found.".into(),
    })?;
    let plan = format!("{} {}", elevate, args.join(" "));
    let out = Command::new(&elevate).args(&args).output().map_err(|e| InstallError {
        kind: InstallErrorKind::Failed,
        message: "The install couldn't be started.".into(),
        detail: format!("failed to launch `{plan}`: {e}"),
    })?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    if out.status.success() {
        return Ok(format!("installed via `{plan}`\n{stdout}"));
    }
    // Classify — pkexec exits 126 when the user dismisses/denies the prompt,
    // and its no-agent fallback dies complaining about a textual
    // authentication agent with no controlling terminal. Both are session
    // problems, not dependency errors, and are presented that way.
    let err_text = format!("{stdout}\n{stderr}").to_lowercase();
    let manual = manual_command(st).unwrap_or_default();
    if err_text.contains("textual authentication agent")
        || err_text.contains("no authentication agent")
        || err_text.contains("authentication agent")
    {
        Err(InstallError {
            kind: InstallErrorKind::NoAgent,
            message: "The tools are ready to install, but this session can't show the \
                      administrator password prompt — the app may have been launched \
                      outside your desktop session.".into(),
            detail: format!(
                "The system password agent wasn't reachable from this session, so the \
                 elevated install never started. Launching the app from your desktop's \
                 app menu usually fixes this. Plan was: `{plan}`. \
                 Manual equivalent: `{manual}`"),
        })
    } else if out.status.code() == Some(126)
        || err_text.contains("dismissed")
        || err_text.contains("not authorized")
    {
        Err(InstallError {
            kind: InstallErrorKind::Cancelled,
            message: "Install cancelled — nothing was changed.".into(),
            detail: format!("The password prompt was dismissed or denied. Plan was: `{plan}`"),
        })
    } else {
        Err(InstallError {
            kind: InstallErrorKind::Failed,
            message: "The install didn't finish — the missing sections stay empty until \
                      the tools are installed.".into(),
            detail: format!("Plan was: `{plan}`\n{}", tail_lines(&stderr, 4, 400)),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn st(pm: &str, pkgs: &[&str]) -> DepStatus {
        DepStatus {
            package_manager: Some(pm.to_string()),
            packages_to_install: pkgs.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn apt_install_refreshes_metadata_first() {
        // Regression pin (Gemini audit 2026-10-14): a bare `apt-get install`
        // fails with "Unable to locate package" on stale lists. The update
        // must be chained BEFORE the install.
        let (_, args) = install_command(&st("apt", &["usbutils", "fwupd"])).unwrap();
        let line = args.join(" ");
        let upd = line.find("apt-get update").expect("apt-get update missing");
        let inst = line.find("apt-get install -y usbutils fwupd").expect("install missing");
        assert!(upd < inst, "{line}");
        assert!(!line.contains("dist-upgrade"), "{line}");
    }

    #[test]
    fn pacman_and_dnf_installs_stay_targeted() {
        let (_, args) = install_command(&st("pacman", &["usbutils"])).unwrap();
        let line = args.join(" ");
        assert!(line.contains("pacman -S --needed --noconfirm usbutils"), "{line}");
        assert!(!line.contains("-Syu"), "dep install must never upgrade the system: {line}");
        let (_, args) = install_command(&st("dnf", &["usbutils"])).unwrap();
        assert_eq!(args.join(" "), "dnf install -y usbutils");
    }

    #[test]
    fn manual_command_matches_automated_shape() {
        let m = manual_command(&st("apt", &["usbutils"])).unwrap();
        assert!(m.contains("apt-get update") && m.contains("apt-get install -y usbutils"), "{m}");
        assert!(manual_command(&st("apt", &[])).is_none());
        assert!(install_command(&st("apt", &[])).is_none());
    }
}
