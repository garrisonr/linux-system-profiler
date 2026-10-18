//! Updates hub — checking is read-only; applying is a separate, explicit,
//! per-group user action spawned by the GUI via polkit, never silent.
//! `collect_updates` itself never mutates the system: every command in it is
//! a read-only query. (The elevated metadata refresh behind an explicit
//! user-initiated Check lives in `metadata_refresh_command` — it refreshes
//! package indexes only, installs nothing.)
use crate::collectors::{cmd, which};
use crate::model::{UpdateItem, UpdateStatus, UpdatesInfo};

/// The elevated apply command for one update group, as (program, argv).
/// Lives in the core so the exact strings are unit-testable — the safety
/// contract is asserted in tests: NO distro path may ever contain
/// `dist-upgrade` or `full-upgrade` (both may REMOVE installed packages to
/// satisfy dependencies; Gemini audit 2026-10-14 caught the apt path doing
/// exactly that). apt applies with a plain `upgrade`, which installs
/// available upgrades and holds back anything that would require removals
/// or new dependencies — the safe behavior the hub promises.
/// pacman keeps `--noconfirm` deliberately: a GUI apply has no TTY, so an
/// interactive pacman would hang on prompts it can never receive.
pub fn apply_command(group: &str) -> Option<(&'static str, Vec<&'static str>)> {
    Some(match group {
        "system-arch" => ("pkexec", vec!["pacman", "-Syu", "--noconfirm"]),
        "system-dnf" => ("pkexec", vec!["dnf", "upgrade", "-y"]),
        "system-apt" => ("pkexec", vec!["sh", "-c",
            "DEBIAN_FRONTEND=noninteractive apt-get upgrade -y"]),
        "firmware" => ("pkexec", vec!["fwupdmgr", "update", "-y"]),
        "flatpak" => ("flatpak", vec!["update", "-y"]),
        _ => return None,
    })
}

/// The ONE full-upgrade path in the crate — deliberately NOT part of
/// the apply_command table above. The default apply stays a safe
/// `apt-get upgrade` forever (the v0.8.0 dist-upgrade scar: a silent
/// full upgrade may REMOVE packages). This command exists only for the
/// explicit "Install these too (full upgrade)" action Dickles ordered
/// 2026-10-17, and the GUI may only reach it behind a confirmation fed
/// by `full_upgrade_plan` — a live simulation naming every install,
/// every removal, every package even this can't fix, and the restart a
/// kernel in the set makes necessary. Tests pin this exact argv and
/// pin that no apply_command group contains it.
pub fn full_upgrade_command() -> (&'static str, Vec<&'static str>) {
    ("pkexec", vec!["sh", "-c",
        "DEBIAN_FRONTEND=noninteractive apt-get full-upgrade -y"])
}

/// Elevated command that refreshes package METADATA ONLY (installs
/// nothing), for an explicit user-initiated Check. apt is the distro family
/// that needs it: `apt list --upgradable` reads possibly-stale local
/// indexes, so a Check that never refreshes them reports old news. pacman
/// needs none (checkupdates syncs a temp DB itself) and dnf check-update
/// refreshes metadata on its own, so both return None. Elevation mirrors
/// deps::install_command: pkexec in a graphical session, else sudo.
pub fn metadata_refresh_command() -> Option<(String, Vec<String>)> {
    let pm = if which("pacman") { Some("pacman") }
        else if which("apt") { Some("apt") }
        else if which("dnf") { Some("dnf") } else { None };
    metadata_refresh_command_for(pm)
}

fn metadata_refresh_command_for(pm: Option<&str>) -> Option<(String, Vec<String>)> {
    if pm != Some("apt") { return None; }
    let graphical = std::env::var("WAYLAND_DISPLAY").is_ok() || std::env::var("DISPLAY").is_ok();
    let elevate = if graphical && which("pkexec") { "pkexec" }
        else if which("sudo") { "sudo" }
        else if which("pkexec") { "pkexec" } else { return None };
    Some((elevate.to_string(), vec!["apt-get".into(), "update".into()]))
}

/// How old apt's package lists may be before counts scanned from them
/// must be labelled stale. apt's own periodic refresh runs daily, so a
/// list older than a day means the daily refresh didn't happen (or the
/// machine was off) — exactly when silent staleness bites.
pub const METADATA_FRESH_WINDOW_SECS: u64 = 24 * 60 * 60;

/// Newest mtime among the files in /var/lib/apt/lists — the last time
/// `apt-get update` (manual, periodic, or ours) actually landed new
/// indexes. Falls back to the directory's own mtime. None when the
/// directory doesn't exist (not an apt system / never populated).
pub fn apt_lists_refreshed_at() -> Option<std::time::SystemTime> {
    apt_lists_refreshed_at_in(std::path::Path::new("/var/lib/apt/lists"))
}

fn apt_lists_refreshed_at_in(dir: &std::path::Path) -> Option<std::time::SystemTime> {
    let mut newest: Option<std::time::SystemTime> = None;
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            if let Ok(md) = e.metadata() {
                if md.is_file() {
                    if let Ok(m) = md.modified() {
                        if newest.map(|n| m > n).unwrap_or(true) { newest = Some(m); }
                    }
                }
            }
        }
    }
    newest.or_else(|| std::fs::metadata(dir).and_then(|m| m.modified()).ok())
}

/// Format unix seconds as ISO-8601 UTC (YYYY-MM-DDTHH:MM:SSZ). Pure —
/// civil-from-days algorithm, no `date` subprocess, so it's testable.
pub fn iso_from_unix(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let tod = secs % 86_400;
    // Howard Hinnant's civil_from_days (public-domain algorithm).
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if m <= 2 { y + 1 } else { y };
    format!("{year:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", tod / 3600, (tod / 60) % 60, tod % 60)
}

/// "just now" / "5 minutes" / "3 hours" / "2 days" — for humans.
pub fn human_age(secs: u64) -> String {
    if secs < 90 { "just now".into() }
    else if secs < 3600 { format!("{} minutes", secs / 60) }
    else if secs < 48 * 3600 { format!("{} hours", secs / 3600) }
    else { format!("{} days", secs / 86_400) }
}

/// Freshness verdict for one scan: (last-refresh ISO, stale?). Pure —
/// the caller supplies `now` and the on-disk refresh time. Stale when
/// there is no refresh on record at all, or it's older than the window.
pub fn freshness_fields(
    now: std::time::SystemTime,
    last: Option<std::time::SystemTime>,
) -> (Option<String>, bool) {
    match last {
        None => (None, true),
        Some(t) => {
            let age = now.duration_since(t).map(|d| d.as_secs()).unwrap_or(0);
            let iso = t.duration_since(std::time::UNIX_EPOCH)
                .map(|d| iso_from_unix(d.as_secs())).ok();
            (iso, age > METADATA_FRESH_WINDOW_SECS)
        }
    }
}

/// The plain-language stale note a scan carries when its counts rest on
/// old lists. Matches what the Updates surface shows — the counts may
/// be wrong and the app says so instead of implying they're current.
fn stale_note(now: std::time::SystemTime, last: Option<std::time::SystemTime>) -> String {
    match last {
        Some(t) => {
            let age = now.duration_since(t).map(|d| d.as_secs()).unwrap_or(0);
            format!(
                "Update counts may be out of date — the package lists were last refreshed {} ago. \"Check for updates\" refreshes the lists first, then counts.",
                human_age(age)
            )
        }
        None => "Update counts may be out of date — the package lists have never been refreshed on this machine. \"Check for updates\" refreshes the lists first, then counts.".into(),
    }
}

/// Stamp an UpdatesInfo collected right after a metadata refresh (the
/// explicit Check flow): this scan's counts are known-current. Keeps
/// the structured fields and the note coherent in one place.
pub fn mark_refreshed(info: &mut UpdatesInfo) {
    info.refreshed_this_check = true;
    info.metadata_stale = false;
    info.metadata_last_refresh = freshness_fields(std::time::SystemTime::now(), apt_lists_refreshed_at()).0
        .or_else(|| Some(iso_from_unix(std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0))));
    info.notes.retain(|n| !n.starts_with("Update counts may be out of date"));
    info.notes.insert(0, "Package lists were refreshed just now — these results are current.".into());
}

/// Package names apt says it deferred due to Ubuntu's phased rollout.
/// apt prints them either after the colon on the marker line or on the
/// following indented line(s):
///   The following upgrades have been deferred due to phasing:
///     apparmor libapparmor1
/// Both layouts are parsed. (v0.8.3 shipped a dialog that called such a
/// run "fully up to date" — the apply had installed nothing.)
pub fn parse_phased_deferred(output: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut in_block = false;
    for line in output.lines() {
        if line.contains("deferred due to phasing") {
            in_block = true;
            if let Some(tail) = line.rsplit(':').next() {
                if tail != line {
                    out.extend(tail.split_whitespace().map(|s| s.to_string()));
                }
            }
            continue;
        }
        if in_block {
            if line.starts_with(char::is_whitespace) && !line.trim().is_empty() {
                out.extend(line.split_whitespace().map(|s| s.to_string()));
            } else {
                in_block = false;
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

/// Names listed under an apt section header — they ride on the
/// indented lines that follow it, wrapped as needed:
///   The following packages have been kept back:
///     foo bar
///     baz
/// Sorted + deduped for stable comparisons.
pub fn parse_name_section(output: &str, header: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut in_block = false;
    for line in output.lines() {
        if line.trim() == header {
            in_block = true;
            continue;
        }
        if in_block {
            if line.starts_with(char::is_whitespace) && !line.trim().is_empty() {
                out.extend(line.split_whitespace().map(|s| s.to_string()));
            } else {
                in_block = false;
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

/// Packages apt's SAFE upgrade refuses to touch ("kept back") — the
/// section both simulations print when an update needs new companion
/// packages or a removal the plain `upgrade` may not perform.
pub fn parse_kept_back(output: &str) -> Vec<String> {
    parse_name_section(output, "The following packages have been kept back:")
}

/// One row of `apt list --upgradable`:
///   name/suite[,suite] available-version arch [upgradable from: current]
/// Until v0.8.9 the collector kept only the name and threw the versions
/// away — the check-only listing showed "None -> None" for every apt
/// package even though apt printed both versions on the same line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpgradableEntry {
    pub name: String,
    pub current: Option<String>,
    pub available: Option<String>,
}

pub fn parse_upgradable_list(output: &str) -> Vec<UpgradableEntry> {
    let mut out = Vec::new();
    for line in output.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with("Listing") { continue; }
        let name = line.split('/').next().unwrap_or("").to_string();
        if name.is_empty() { continue; }
        let mut tokens = line.split_whitespace();
        let _ = tokens.next(); // name/suite
        let available = tokens.next().map(|s| s.to_string());
        let mut current = None;
        let toks: Vec<&str> = line.split_whitespace().collect();
        for w in toks.windows(2) {
            if w[0] == "from:" {
                current = Some(w[1].trim_end_matches(']').to_string());
            }
        }
        out.push(UpgradableEntry { name, current, available });
    }
    out
}

/// (package, percent) pairs from `apt-cache policy` output. The
/// percentage rides on the CANDIDATE's version-table line:
///   snapd:
///     Candidate: 2.77.1+ubuntu24.04
///     Version table:
///        2.77.1+ubuntu24.04 500 (phased 60%)
/// Captured live on the G9 (snapd, 60%) — policy is the only apt output
/// that prints the number.
pub fn parse_phased_percents(output: &str) -> Vec<(String, u32)> {
    let mut out: Vec<(String, u32)> = Vec::new();
    let mut pkg: Option<String> = None;
    let mut candidate: Option<String> = None;
    for line in output.lines() {
        if !line.starts_with(char::is_whitespace) && line.trim_end().ends_with(':') {
            pkg = Some(line.trim().trim_end_matches(':').to_string());
            candidate = None;
            continue;
        }
        let t = line.trim();
        if let Some(c) = t.strip_prefix("Candidate:") {
            candidate = Some(c.trim().to_string());
            continue;
        }
        if let (Some(p), Some(cand)) = (&pkg, &candidate) {
            let body = t.strip_prefix("*** ").unwrap_or(t);
            if body.split_whitespace().next() == Some(cand.as_str()) {
                if let Some(pos) = body.find("(phased ") {
                    let rest = &body[pos + "(phased ".len()..];
                    if let Some(end) = rest.find('%') {
                        if let Ok(pct) = rest[..end].trim().parse::<u32>() {
                            if !out.iter().any(|(n, _)| n == p) {
                                out.push((p.clone(), pct));
                            }
                        }
                    }
                }
            }
        }
    }
    out
}

/// The tally line both simulations (and a real run) end with:
/// "5 upgraded, 12 newly installed, 0 to remove and 1 not upgraded."
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AptCounts {
    pub upgraded: u32,
    pub newly_installed: u32,
    pub to_remove: u32,
    pub not_upgraded: u32,
}

pub fn parse_apt_counts(output: &str) -> Option<AptCounts> {
    for line in output.lines() {
        let line = line.trim();
        if !line.contains(" upgraded, ") || !line.contains(" newly installed, ")
            || !line.contains(" to remove and ") || !line.contains(" not upgraded.") {
            continue;
        }
        let nums: Vec<u32> = line.split(|c: char| !(c.is_ascii_digit()))
            .filter(|s| !s.is_empty())
            .filter_map(|s| s.parse().ok())
            .collect();
        if nums.len() >= 4 {
            return Some(AptCounts {
                upgraded: nums[0], newly_installed: nums[1],
                to_remove: nums[2], not_upgraded: nums[3],
            });
        }
    }
    None
}

/// What apt's resolver plans to do, read from a simulation's text.
/// Names keep first-seen order (the order apt would act in).
#[derive(Debug, Clone, Default)]
pub struct SimPlan {
    /// Every package an "Inst" line installs (upgrades + brand-new).
    pub installs: Vec<String>,
    /// Installs that replace an installed version ("Inst name [old]").
    pub upgraded: Vec<String>,
    /// Installs with no installed version to replace — the new
    /// companion packages a full upgrade brings in.
    pub fresh: Vec<String>,
    /// Packages a "Remv" line would remove.
    pub removals: Vec<String>,
    /// The simulation's own kept-back section.
    pub kept_back: Vec<String>,
    pub counts: Option<AptCounts>,
}

fn push_unique(v: &mut Vec<String>, s: &str) {
    if !v.iter().any(|x| x == s) { v.push(s.to_string()); }
}

pub fn parse_sim_plan(output: &str) -> SimPlan {
    let mut plan = SimPlan { counts: parse_apt_counts(output), ..Default::default() };
    plan.kept_back = parse_kept_back(output);
    for line in output.lines() {
        let mut it = line.split_whitespace();
        match it.next() {
            Some("Inst") => {
                if let Some(name) = it.next() {
                    push_unique(&mut plan.installs, name);
                    // "Inst name [old] (new …)" upgrades; "Inst name (new …)" is new.
                    match it.next() {
                        Some(tok) if tok.starts_with('[') => push_unique(&mut plan.upgraded, name),
                        _ => push_unique(&mut plan.fresh, name),
                    }
                }
            }
            Some("Remv") => {
                if let Some(name) = it.next() { push_unique(&mut plan.removals, name); }
            }
            _ => {}
        }
    }
    plan
}

/// The plain clause behind an unmet dependency, from
/// `apt-get -s install <pkg>`:
///   containerd.io : Depends: libseccomp2 (>= 2.6.0) but 2.5.5-1ubuntu3.1 is to be installed
/// → "needs libseccomp2 2.6.0 or newer, but this system only offers
///    2.5.5-1ubuntu3.1". Other shapes fall back to apt's own words.
pub fn parse_unmet_dependency(output: &str) -> Option<String> {
    for line in output.lines() {
        let Some(pos) = line.find(" : Depends: ") else { continue };
        let tail = line[pos + " : Depends: ".len()..].trim();
        let mut parts = tail.splitn(2, " but ");
        let need = parts.next().unwrap_or("").trim();
        let but = parts.next().unwrap_or("").trim();
        let mut need_it = need.split_whitespace();
        let dep = need_it.next().unwrap_or("");
        let want = need_it.next().unwrap_or("").trim_matches(|c| c == '(' || c == ')');
        let want_ver = need_it.next().unwrap_or("").trim_matches(|c| c == '(' || c == ')');
        if !dep.is_empty() && want == ">=" && !want_ver.is_empty() {
            if let Some(avail) = but.strip_suffix(" is to be installed") {
                return Some(format!(
                    "needs {dep} {want_ver} or newer, but this system only offers {}",
                    avail.trim()));
            }
            if but.contains("not going to be installed") || but.contains("not installable") {
                return Some(format!(
                    "needs {dep} {want_ver} or newer, but that isn't available for this system"));
            }
        }
        if !need.is_empty() {
            return Some(format!("needs {need}"));
        }
    }
    None
}

/// Everything the apt classifier needs, gathered from apt's own
/// read-only simulations at check time.
#[derive(Debug, Default)]
pub struct AptSignals {
    pub kept_back: Vec<String>,
    pub phased: Vec<String>,
    pub phased_percents: Vec<(String, u32)>,
    pub held: Vec<String>,
    pub full_plan: SimPlan,
    /// Plain unmet-dependency clause per blocked package name.
    pub blocked_reasons: Vec<(String, String)>,
}

/// Classify one apt update into (status, plain-English row note,
/// phased percent). Precedence: a hold is the user's own instruction
/// and wins; phasing is Ubuntu's timing and no local action overrides
/// it; then the kept-back pair of truths — installable by a full
/// upgrade (KeptBack) vs. kept back even there (Blocked).
pub fn classify_apt(name: &str, sig: &AptSignals) -> (UpdateStatus, Option<String>, Option<u32>) {
    if sig.held.iter().any(|n| n == name) {
        return (UpdateStatus::Held, Some(
            "On hold — this package was put on hold, so it won't update until the hold is removed.".into()), None);
    }
    if sig.phased.iter().any(|n| n == name) {
        let pct = sig.phased_percents.iter().find(|(n, _)| n == name).map(|(_, p)| *p);
        let note = match pct {
            Some(p) => format!("Waiting — Ubuntu is releasing this update gradually (about {p}% of machines have it so far). It will install on a later check, on its own."),
            None => "Waiting — Ubuntu is releasing this update in phases and hasn't released it to this machine yet. It will install on a later check, on its own.".into(),
        };
        return (UpdateStatus::Phased, Some(note), pct);
    }
    if sig.kept_back.iter().any(|n| n == name) {
        if sig.full_plan.kept_back.iter().any(|n| n == name) {
            let note = match sig.blocked_reasons.iter().find(|(n, _)| n == name) {
                Some((_, r)) => format!("Can't install yet — it {r}. Even the full upgrade can't install it; it has to wait for a matching package to be published."),
                None => "Can't install yet — something it needs isn't available for this system, and even the full upgrade can't install it. It has to wait for a matching package to be published.".into(),
            };
            return (UpdateStatus::Blocked, Some(note), None);
        }
        return (UpdateStatus::KeptBack, Some(
            "Waiting — this update brings new companion packages, and the safe update isn't allowed to install those. The \u{201c}Install these too\u{201d} button below can install it.".into()), None);
    }
    (UpdateStatus::Ready, None, None)
}

/// Gather the classifier signals from apt's read-only simulations.
/// Every command is a query — the `-s` simulations install nothing.
pub fn apt_signals(upgradable: &[UpgradableEntry]) -> AptSignals {
    let mut sig = AptSignals::default();
    if let Some(sim) = cmd("apt-get", &["-s", "upgrade"]) {
        sig.kept_back = parse_kept_back(&sim);
        sig.phased = parse_phased_deferred(&sim);
    }
    if !upgradable.is_empty() {
        let names: Vec<String> = upgradable.iter().map(|e| e.name.clone()).collect();
        let refs: Vec<&str> = names.iter().map(|s| s.as_str()).collect();
        let mut args: Vec<&str> = vec!["policy"];
        args.extend(refs.iter().copied());
        if let Some(pol) = cmd("apt-cache", &args) {
            sig.phased_percents = parse_phased_percents(&pol);
        }
    }
    if let Some(full) = cmd("apt-get", &["-s", "full-upgrade"]) {
        sig.full_plan = parse_sim_plan(&full);
    }
    if let Some(holds) = cmd("apt-mark", &["showhold"]) {
        sig.held = holds.lines().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
    }
    // Unmet-dependency reasons only for names BOTH simulations keep
    // back — the one case where "waiting" needs a deeper truth.
    for name in &sig.kept_back {
        if sig.full_plan.kept_back.contains(name) && !sig.phased.contains(name) {
            if let Some(out) = cmd("apt-get", &["-s", "install", name]) {
                if let Some(r) = parse_unmet_dependency(&out) {
                    sig.blocked_reasons.push((name.clone(), r));
                }
            }
        }
    }
    sig
}

/// One kept-back update even a full upgrade can't install yet.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct BlockedUpdate {
    pub name: String,
    /// Plain clause ("needs libseccomp2 2.6.0 or newer, but …"), when
    /// apt's install simulation named the blocker.
    pub reason: Option<String>,
}

/// The full upgrade's plan, composed from the simulations BEFORE
/// anything runs — the confirmation the user reads is this struct,
/// so what he's told is what apt itself predicts.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct FullUpgradePlan {
    /// False on non-apt systems (nothing to plan).
    pub available: bool,
    /// Kept-back updates the full upgrade WILL install.
    pub waiting: Vec<String>,
    /// Brand-new companion packages it will install alongside.
    pub new_companions: Vec<String>,
    /// Packages it would REMOVE to make room — named, never a surprise.
    pub removals: Vec<String>,
    /// Kept-back updates even this can't install yet, with reasons.
    pub blocked: Vec<BlockedUpdate>,
    /// A kernel package is in the install set — a restart follows.
    pub needs_restart: bool,
}

pub fn plan_from_outputs(upgradable: &[UpgradableEntry], sig: &AptSignals) -> FullUpgradePlan {
    let mut plan = FullUpgradePlan { available: true, ..Default::default() };
    for e in upgradable {
        match classify_apt(&e.name, sig).0 {
            UpdateStatus::KeptBack => plan.waiting.push(e.name.clone()),
            UpdateStatus::Blocked => plan.blocked.push(BlockedUpdate {
                name: e.name.clone(),
                reason: sig.blocked_reasons.iter()
                    .find(|(n, _)| n == &e.name).map(|(_, r)| r.clone()),
            }),
            _ => {}
        }
    }
    plan.new_companions = sig.full_plan.fresh.clone();
    plan.removals = sig.full_plan.removals.clone();
    plan.needs_restart = sig.full_plan.installs.iter()
        .any(|n| n.starts_with("linux-image") || n.starts_with("linux-generic"));
    plan
}

/// Live preflight for the "Install these too" confirmation: parse the
/// current simulations into the plan the dialog will read aloud.
pub fn full_upgrade_plan() -> FullUpgradePlan {
    if !which("apt") { return FullUpgradePlan::default(); }
    let entries = cmd("apt", &["list", "--upgradable"])
        .map(|o| parse_upgradable_list(&o)).unwrap_or_default();
    let sig = apt_signals(&entries);
    plan_from_outputs(&entries, &sig)
}

/// What an apply actually achieved, established by RE-CHECKING the group
/// afterwards — never by the apply's exit code alone. v0.8.3 trusted
/// exit 0 and declared "fully up to date" while apt had deferred every
/// package to phasing and installed nothing.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ApplyReport {
    /// The apply's full raw output (the dialog's details / copy body).
    pub output: String,
    /// Items the post-apply re-check still lists for this group.
    pub remaining: usize,
    /// Remaining items held back by Ubuntu phasing, by name.
    pub deferred: Vec<String>,
    /// Remaining items kept back for new companion packages, by name.
    pub kept_back: Vec<String>,
    /// Remaining items on hold, by name.
    pub held: Vec<String>,
    /// Remaining items that can't install yet (unmet needs), by name.
    pub blocked: Vec<String>,
    /// Plain-English summary lines — the dialog leads with these; the
    /// raw output follows behind the details divider.
    pub summary: Vec<String>,
    /// The plain-language headline the dialog leads with.
    pub headline: String,
}

/// Compose the report from the apply output and the RE-CHECKED group
/// items. Remaining items are bucketed by their check-time status
/// (unioned, for the phased/kept-back buckets, with the sections the
/// apply output itself printed — filtered to items that actually
/// remain). Every claim derives from the re-check, never from the
/// apply's exit code. Dickles, 2026-10-17: the old fallback headline
/// ("N updates still remain — the output below shows what happened")
/// over a wall of raw apt text was the complaint that built this.
pub fn apply_report(output: String, remaining_items: &[UpdateItem]) -> ApplyReport {
    let from_output_phased = parse_phased_deferred(&output);
    let from_output_kept = parse_kept_back(&output);
    let remaining = remaining_items.len();
    let mut deferred: Vec<String> = Vec::new();
    let mut kept_back: Vec<String> = Vec::new();
    let mut held: Vec<String> = Vec::new();
    let mut blocked: Vec<String> = Vec::new();
    let mut other: Vec<String> = Vec::new();
    for i in remaining_items {
        match i.status {
            UpdateStatus::Phased => deferred.push(i.name.clone()),
            UpdateStatus::KeptBack => kept_back.push(i.name.clone()),
            UpdateStatus::Held => held.push(i.name.clone()),
            UpdateStatus::Blocked => blocked.push(i.name.clone()),
            UpdateStatus::Ready => {
                if from_output_phased.contains(&i.name) {
                    deferred.push(i.name.clone());
                } else if from_output_kept.contains(&i.name) {
                    kept_back.push(i.name.clone());
                } else {
                    other.push(i.name.clone());
                }
            }
        }
    }
    let s = if remaining == 1 { "" } else { "s" };
    let headline = if remaining == 0 {
        "Finished — this group is fully up to date.".to_string()
    } else if deferred.len() == remaining {
        format!(
            "Finished — but {} update{} not installed yet: Ubuntu releases some updates in phases, and {} {} been released to this machine yet. {} install on a later check, once Ubuntu releases {} to you.",
            remaining,
            if remaining == 1 { " was" } else { "s were" },
            deferred.join(", "),
            if remaining == 1 { "hasn't" } else { "haven't" },
            if remaining == 1 { "It will" } else { "They'll" },
            if remaining == 1 { "it" } else { "them" },
        )
    } else if kept_back.len() == remaining {
        format!(
            "Finished — but {} update{} not installed: {}. They bring new companion packages the safe update isn't allowed to install — the \u{201c}Install these too\u{201d} button on the Updates page can install them now.",
            remaining,
            if remaining == 1 { " was" } else { "s were" },
            kept_back.join(", "),
        )
    } else if held.len() == remaining {
        format!(
            "Finished — but {} update{} on hold ({}) and didn't install. They'll stay put until the hold is removed.",
            remaining,
            if remaining == 1 { " is" } else { "s are" },
            held.join(", "),
        )
    } else if blocked.len() == remaining {
        format!(
            "Finished — but {} update{} still can't install: {}. Something they need isn't available for this system yet — the summary below explains.",
            remaining,
            if remaining == 1 { "" } else { "s" },
            blocked.join(", "),
        )
    } else {
        let mut parts: Vec<String> = Vec::new();
        if !kept_back.is_empty() {
            parts.push(format!("{} waiting on new companion packages ({})",
                kept_back.len(), kept_back.join(", ")));
        }
        if !deferred.is_empty() {
            parts.push(format!("{} held back by Ubuntu's phased rollout ({})",
                deferred.len(), deferred.join(", ")));
        }
        if !held.is_empty() {
            parts.push(format!("{} on hold ({})", held.len(), held.join(", ")));
        }
        if !blocked.is_empty() {
            parts.push(format!("{} that can't install yet ({})",
                blocked.len(), blocked.join(", ")));
        }
        if !other.is_empty() {
            parts.push(format!("{} for another reason", other.len()));
        }
        format!(
            "Finished — {} update{} still remain: {}. The summary below explains each group.",
            remaining, s, parts.join("; "),
        )
    };
    // The plain-English summary the finished dialog leads with. The
    // installed count comes from the run's own tally line; everything
    // else comes from the re-check's buckets.
    let mut summary: Vec<String> = Vec::new();
    if let Some(c) = parse_apt_counts(&output) {
        let n = c.upgraded + c.newly_installed;
        if n > 0 {
            summary.push(format!("Installed {} package{}.", n, if n == 1 { "" } else { "s" }));
        } else if remaining > 0 {
            summary.push("Nothing was installed this time.".to_string());
        }
    }
    if remaining == 0 {
        summary.push("Everything in this group is now up to date.".to_string());
    }
    if !kept_back.is_empty() {
        summary.push(format!(
            "Still waiting — {}: they bring new companion packages the safe update isn't allowed to install. The \u{201c}Install these too\u{201d} button on the Updates page can install them now.",
            kept_back.join(", ")));
    }
    if !deferred.is_empty() {
        summary.push(format!(
            "Still waiting — {}: Ubuntu is releasing {} in phases and hasn't released {} to this machine yet. {} install on a later check.",
            deferred.join(", "),
            if deferred.len() == 1 { "this one" } else { "these" },
            if deferred.len() == 1 { "it" } else { "them" },
            if deferred.len() == 1 { "It will" } else { "They'll" }));
    }
    if !held.is_empty() {
        summary.push(format!(
            "Still waiting — {}: on hold, so they won't update until the hold is removed.",
            held.join(", ")));
    }
    if !blocked.is_empty() {
        summary.push(format!(
            "Still blocked — {}: something they need isn't available for this system yet, so even a full upgrade can't install them. The details below show what the package manager said.",
            blocked.join(", ")));
    }
    if !other.is_empty() {
        summary.push(format!(
            "Still remaining — {}: the details below show what the package manager said.",
            other.join(", ")));
    }
    ApplyReport { output, remaining, deferred, kept_back, held, blocked, summary, headline }
}

pub fn collect_updates() -> UpdatesInfo {
    let mut info = UpdatesInfo { checked: true, ..Default::default() };

    // System packages: Arch / Debian / Fedora families, check-only flags only.
    if which("pacman") {
        // Prefer checkupdates: it syncs a TEMP db copy, never the system DB.
        let checkupdates_out = if which("checkupdates") { cmd("checkupdates", &[]) } else { None };
        match checkupdates_out.or_else(|| cmd("pacman", &["-Qu"])) {
            Some(out) => for line in out.lines() {
                // format: "name old -> new" or "name old"
                let mut it = line.split_whitespace();
                let name = it.next().unwrap_or("").to_string();
                let cur = it.next().map(|s| s.to_string());
                let avail = if line.contains("->") { line.split("->").nth(1).map(|s| s.trim().to_string()) } else { None };
                if !name.is_empty() { info.system_updates.push(UpdateItem { name, current: cur, available: avail, source: "pacman".into(), note: None, status: UpdateStatus::Ready, phased_percent: None }); }
            },
            None => info.notes.push("pacman -Qu returned nothing (up to date, or checkupdates not run)".into()),
        }
        if which("checkupdates") { info.notes.push("Tip: `checkupdates` gives a sync-free count without root.".into()); }
    } else if which("apt") {
        // Versions come from the same `apt list --upgradable` line that
        // names the package (v0.8.9: the collector used to discard
        // them, and the listing showed "None -> None"). Every row is
        // then classified from apt's own read-only simulations — can
        // it install, and if not, exactly why (apt_signals/classify_apt).
        let entries = cmd("apt", &["list", "--upgradable"])
            .map(|o| parse_upgradable_list(&o)).unwrap_or_default();
        let sig = apt_signals(&entries);
        for e in entries {
            let (status, note, pct) = classify_apt(&e.name, &sig);
            info.system_updates.push(UpdateItem {
                name: e.name, current: e.current, available: e.available,
                source: "apt".into(), note, status, phased_percent: pct,
            });
        }
        // Freshness honesty (fix-claim #2 was PARTIAL at v0.8.7): this
        // scan reads whatever lists are on disk — it does NOT refresh
        // them. Carry the freshness on the data and SAY when the lists
        // are old, so launch-time counts never silently pose as
        // current. The GUI's initial flow runs the existing Check
        // (refresh first) automatically when this says stale.
        let now = std::time::SystemTime::now();
        let last = apt_lists_refreshed_at();
        let (iso, stale) = freshness_fields(now, last);
        info.metadata_last_refresh = iso;
        info.metadata_stale = stale;
        if stale { info.notes.push(stale_note(now, last)); }
    } else if which("dnf") {
        if let Some(out) = cmd("dnf", &["check-update", "--quiet"]) {
            for line in out.lines() { let n = line.split_whitespace().next().unwrap_or("");
                if !n.is_empty() { info.system_updates.push(UpdateItem { name: n.into(), current: None, available: None, source: "dnf".into(), note: None, status: UpdateStatus::Ready, phased_percent: None }); } }
        }
    } else {
        info.notes.push("No supported package manager found for update check.".into());
    }

    // Firmware via LVFS / fwupd — `get-updates` is a query, never an apply.
    if which("fwupdmgr") {
        match cmd("fwupdmgr", &["get-updates", "--json"]) {
            Some(out) => {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&out) {
                    if let Some(devs) = v.get("Devices").and_then(|d| d.as_array()) {
                        for d in devs {
                            let name = d.get("Name").and_then(|n| n.as_str()).unwrap_or("device").to_string();
                            if let Some(rels) = d.get("Releases").and_then(|r| r.as_array()) {
                                for r in rels {
                                    info.firmware_updates.push(UpdateItem { name: name.clone(),
                                        current: d.get("Version").and_then(|x| x.as_str()).map(|s| s.to_string()),
                                        available: r.get("Version").and_then(|x| x.as_str()).map(|s| s.to_string()),
                                        source: "fwupd/LVFS".into(), note: None, status: UpdateStatus::Ready, phased_percent: None });
                                }
                            }
                        }
                    }
                }
                if info.firmware_updates.is_empty() { info.notes.push("fwupd: no firmware updates reported.".into()); }
            }
            None => info.notes.push("fwupdmgr present but get-updates failed (daemon not running?).".into()),
        }
    } else {
        info.notes.push("fwupdmgr not installed — firmware updates via LVFS unavailable. On Arch: pacman -S fwupd".into());
    }

    // Flatpak — remote-ls --updates is a query only.
    if which("flatpak") {
        if let Some(out) = cmd("flatpak", &["remote-ls", "--updates", "--columns=application,version,branch"]) {
            for line in out.lines() {
                let mut c = line.split('\t');
                let name = c.next().unwrap_or("").to_string();
                if !name.is_empty() { info.flatpak_updates.push(UpdateItem { name,
                    current: None, available: c.next().map(|s| s.to_string()), source: "flatpak".into(), note: None, status: UpdateStatus::Ready, phased_percent: None }); }
            }
        }
    }
    info
}

#[cfg(test)]
mod tests {
    use super::*;

    const GROUPS: &[&str] = &["system-arch", "system-dnf", "system-apt", "firmware", "flatpak"];

    #[test]
    fn no_apply_path_ever_dist_upgrades() {
        // The v0.8.0 apt apply ran `apt-get dist-upgrade -y`, which may
        // REMOVE installed packages — the audit's critical finding. Pin the
        // whole table: no group, on any distro, may contain it (or dnf's
        // equivalent semantics under another name).
        for g in GROUPS {
            let (prog, args) = apply_command(g).unwrap_or_else(|| panic!("group {g} must exist"));
            let line = format!("{prog} {}", args.join(" "));
            assert!(!line.contains("dist-upgrade"), "{g}: {line}");
            assert!(!line.contains("full-upgrade"), "{g}: {line}");
            assert!(!line.contains("autoremove"), "{g}: {line}");
        }
        assert!(apply_command("bogus-group").is_none());
    }

    #[test]
    fn apt_apply_is_a_plain_safe_upgrade() {
        let (_, args) = apply_command("system-apt").unwrap();
        let line = args.join(" ");
        assert!(line.contains("apt-get upgrade -y"), "{line}");
        assert!(line.contains("DEBIAN_FRONTEND=noninteractive"), "{line}");
    }

    #[test]
    fn phased_deferred_parses_both_apt_layouts() {
        // Stock apt layout: names on the following indented line.
        let stock = "The following upgrades have been deferred due to phasing:\n  apparmor libapparmor1\n0 upgraded, 0 newly installed, 0 to remove and 2 not upgraded.\n";
        assert_eq!(parse_phased_deferred(stock), vec!["apparmor", "libapparmor1"]);
        // Same-line layout (names after the colon).
        let inline = "The following upgrades have been deferred due to phasing: apparmor libapparmor1\n";
        assert_eq!(parse_phased_deferred(inline), vec!["apparmor", "libapparmor1"]);
        // Wrapped across two indented lines, and no phasing at all.
        let wrapped = "The following upgrades have been deferred due to phasing:\n  aaa bbb\n  ccc\n2 upgraded.\n";
        assert_eq!(parse_phased_deferred(wrapped), vec!["aaa", "bbb", "ccc"]);
        assert!(parse_phased_deferred("5 upgraded, 0 newly installed.\n").is_empty());
    }

    fn item(name: &str, note: Option<&str>) -> UpdateItem {
        UpdateItem { name: name.into(), current: None, available: None,
            source: "apt".into(), note: note.map(|s| s.to_string()),
            status: UpdateStatus::Ready, phased_percent: None }
    }

    fn item_s(name: &str, status: UpdateStatus) -> UpdateItem {
        UpdateItem { name: name.into(), current: None, available: None,
            source: "apt".into(), note: None, status, phased_percent: None }
    }

    #[test]
    fn apply_report_never_claims_up_to_date_with_remainder() {
        // The v0.8.3 lie: exit 0, everything deferred, headline claimed
        // "fully up to date". The re-check drives the headline now.
        let out = "The following upgrades have been deferred due to phasing:\n  apparmor libapparmor1\n0 upgraded.\n".to_string();
        let remaining = vec![
            item_s("apparmor", UpdateStatus::Phased),
            item_s("libapparmor1", UpdateStatus::Phased),
        ];
        let r = apply_report(out, &remaining);
        assert_eq!(r.remaining, 2);
        assert_eq!(r.deferred, vec!["apparmor", "libapparmor1"]);
        assert!(r.headline.contains("phases"), "{}", r.headline);
        assert!(r.headline.contains("apparmor"), "{}", r.headline);
        assert!(!r.headline.contains("fully up to date"), "{}", r.headline);
    }

    #[test]
    fn apply_report_zero_remaining_is_the_only_up_to_date() {
        let r = apply_report("3 upgraded.\n".into(), &[]);
        assert_eq!(r.remaining, 0);
        assert!(r.headline.contains("fully up to date"));
        // Remaining for an unparsed reason: honest remainder, no phasing story.
        let r2 = apply_report("done\n".into(), &[item("kept-back-pkg", None)]);
        assert_eq!(r2.remaining, 1);
        assert!(r2.deferred.is_empty());
        assert!(r2.headline.contains("still remain"), "{}", r2.headline);
        assert!(!r2.headline.contains("fully up to date"));
        // Mixed: one phased, one other.
        let r3 = apply_report("done\n".into(), &[
            item_s("phased-pkg", UpdateStatus::Phased),
            item("other-pkg", None),
        ]);
        assert_eq!(r3.deferred, vec!["phased-pkg"]);
        assert!(r3.headline.contains("still remain"), "{}", r3.headline);
        assert!(r3.headline.contains("phased rollout"), "{}", r3.headline);
    }

    #[test]
    fn metadata_refresh_is_apt_only_and_installs_nothing() {
        let (elevate, args) = metadata_refresh_command_for(Some("apt")).unwrap();
        assert!(elevate == "pkexec" || elevate == "sudo");
        assert_eq!(args, vec!["apt-get".to_string(), "update".to_string()]);
        assert!(metadata_refresh_command_for(Some("pacman")).is_none());
        assert!(metadata_refresh_command_for(Some("dnf")).is_none());
        assert!(metadata_refresh_command_for(None).is_none());
    }

    #[test]
    fn iso_from_unix_known_values() {
        assert_eq!(iso_from_unix(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso_from_unix(1_700_000_000), "2023-11-14T22:13:20Z");
        assert_eq!(iso_from_unix(1_761_000_000), "2025-10-20T22:40:00Z");
        // Leap day: 2024-02-29 12:00:00 UTC.
        assert_eq!(iso_from_unix(1_709_208_000), "2024-02-29T12:00:00Z");
    }

    #[test]
    fn freshness_window_and_never_refreshed() {
        use std::time::{Duration, SystemTime};
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_761_000_000);
        // 1 hour old: fresh, ISO carried.
        let (iso, stale) = freshness_fields(now, Some(now - Duration::from_secs(3600)));
        assert!(!stale);
        assert_eq!(iso.as_deref(), Some("2025-10-20T21:40:00Z"));
        // 25 hours old: stale — past the daily-refresh window.
        let (_, stale) = freshness_fields(now, Some(now - Duration::from_secs(25 * 3600)));
        assert!(stale);
        // Exactly at the window edge: still fresh.
        let (_, stale) = freshness_fields(now, Some(now - Duration::from_secs(METADATA_FRESH_WINDOW_SECS)));
        assert!(!stale);
        // No refresh on record: stale, no ISO.
        let (iso, stale) = freshness_fields(now, None);
        assert!(stale);
        assert!(iso.is_none());
    }

    #[test]
    fn stale_note_names_the_age_and_the_remedy() {
        use std::time::{Duration, SystemTime};
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_761_000_000);
        let n = stale_note(now, Some(now - Duration::from_secs(3 * 3600)));
        assert!(n.contains("may be out of date"), "{n}");
        assert!(n.contains("3 hours"), "{n}");
        assert!(n.contains("Check for updates"), "{n}");
        let n = stale_note(now, None);
        assert!(n.contains("never been refreshed"), "{n}");
    }

    #[test]
    fn mark_refreshed_clears_staleness() {
        let mut info = UpdatesInfo {
            checked: true, metadata_stale: true,
            notes: vec!["Update counts may be out of date — old note.".into()],
            ..Default::default()
        };
        mark_refreshed(&mut info);
        assert!(info.refreshed_this_check);
        assert!(!info.metadata_stale);
        assert!(info.metadata_last_refresh.is_some());
        assert!(info.notes.iter().all(|n| !n.contains("may be out of date")));
        assert!(info.notes[0].contains("refreshed just now"));
    }

    #[test]
    fn human_age_buckets() {
        assert_eq!(human_age(30), "just now");
        assert_eq!(human_age(600), "10 minutes");
        assert_eq!(human_age(7200), "2 hours");
        assert_eq!(human_age(3 * 86_400), "3 days");
    }

    // ---- v0.8.9 plain-talk classification --------------------------------
    // Fixtures are FULL live captures (tests/fixtures/): the Dell on
    // 2026-10-17 with its six kept-back updates, and the G9 the same
    // evening with snapd mid-phased-rollout. Parsers soak on the real
    // corpus, never on samples typed from memory.

    const DELL_LIST: &str = include_str!("../tests/fixtures/dell-apt-list-upgradable.txt");
    const DELL_SIM_UPGRADE: &str = include_str!("../tests/fixtures/dell-aptget-s-upgrade.txt");
    const DELL_SIM_FULL: &str = include_str!("../tests/fixtures/dell-aptget-s-full-upgrade.txt");
    const DELL_POLICY: &str = include_str!("../tests/fixtures/dell-apt-cache-policy.txt");
    const DELL_INSTALL_CONTAINERD: &str = include_str!("../tests/fixtures/dell-aptget-s-install-containerd.txt");
    const G9_LIST: &str = include_str!("../tests/fixtures/g9-apt-list-upgradable.txt");
    const G9_SIM_UPGRADE: &str = include_str!("../tests/fixtures/g9-aptget-s-upgrade.txt");
    const G9_POLICY: &str = include_str!("../tests/fixtures/g9-apt-cache-policy.txt");

    /// Rebuild the Dell's signals from the captures, following the
    /// same gating apt_signals uses (unmet-dependency probes only for
    /// names both simulations keep back).
    fn dell_signals() -> AptSignals {
        let mut sig = AptSignals {
            kept_back: parse_kept_back(DELL_SIM_UPGRADE),
            phased: parse_phased_deferred(DELL_SIM_UPGRADE),
            phased_percents: parse_phased_percents(DELL_POLICY),
            held: vec![],
            full_plan: parse_sim_plan(DELL_SIM_FULL),
            blocked_reasons: vec![],
        };
        for name in &sig.kept_back {
            if sig.full_plan.kept_back.contains(name) {
                if let Some(r) = parse_unmet_dependency(DELL_INSTALL_CONTAINERD) {
                    sig.blocked_reasons.push((name.clone(), r));
                }
            }
        }
        sig
    }

    #[test]
    fn upgradable_list_carries_real_versions() {
        // The "None -> None" bug: apt printed both versions all along.
        let entries = parse_upgradable_list(DELL_LIST);
        assert_eq!(entries.len(), 6);
        let c = &entries[0];
        assert_eq!(c.name, "containerd.io");
        assert_eq!(c.current.as_deref(), Some("2.2.6-1~debian.13~trixie"));
        assert_eq!(c.available.as_deref(), Some("2.4.2-2~debian.13~trixie"));
        let nv = entries.iter().find(|e| e.name == "linux-modules-nvidia-580-generic-hwe-24.04").unwrap();
        assert_eq!(nv.current.as_deref(), Some("7.0.0-31.31~24.04.1+1"));
        assert_eq!(nv.available.as_deref(), Some("7.0.0-38.38~24.04.4+1"));
        let g9 = parse_upgradable_list(G9_LIST);
        assert_eq!(g9.len(), 1);
        assert_eq!(g9[0].name, "snapd");
        assert_eq!(g9[0].current.as_deref(), Some("2.76.3+ubuntu24.04"));
        assert_eq!(g9[0].available.as_deref(), Some("2.77.1+ubuntu24.04"));
    }

    #[test]
    fn kept_back_parses_the_dell_six_and_not_the_g9_phased() {
        assert_eq!(parse_kept_back(DELL_SIM_UPGRADE), vec![
            "containerd.io", "gnome-shell-extension-zorin-dash",
            "linux-generic-hwe-24.04", "linux-headers-generic-hwe-24.04",
            "linux-image-generic-hwe-24.04", "linux-modules-nvidia-580-generic-hwe-24.04",
        ]);
        // The G9 sim defers snapd to phasing and keeps NOTHING back —
        // the two sections must never bleed into each other.
        assert!(parse_kept_back(G9_SIM_UPGRADE).is_empty());
        assert_eq!(parse_phased_deferred(G9_SIM_UPGRADE), vec!["snapd"]);
        // The full-upgrade sim keeps back exactly one: containerd.io.
        assert_eq!(parse_kept_back(DELL_SIM_FULL), vec!["containerd.io"]);
    }

    #[test]
    fn phased_percents_come_from_policy() {
        // Live G9 capture: snapd's candidate line carries "(phased 60%)".
        assert_eq!(parse_phased_percents(G9_POLICY), vec![("snapd".to_string(), 60)]);
        // None of the Dell's six are phased; other versions in their
        // tables must not leak a percent onto the candidate.
        assert!(parse_phased_percents(DELL_POLICY).is_empty());
    }

    #[test]
    fn apt_counts_parse_the_real_tally_lines() {
        assert_eq!(parse_apt_counts(DELL_SIM_UPGRADE),
            Some(AptCounts { upgraded: 0, newly_installed: 0, to_remove: 0, not_upgraded: 6 }));
        assert_eq!(parse_apt_counts(DELL_SIM_FULL),
            Some(AptCounts { upgraded: 5, newly_installed: 12, to_remove: 0, not_upgraded: 1 }));
        assert_eq!(parse_apt_counts(G9_SIM_UPGRADE),
            Some(AptCounts { upgraded: 0, newly_installed: 0, to_remove: 0, not_upgraded: 1 }));
    }

    #[test]
    fn unmet_dependency_becomes_plain_english() {
        let r = parse_unmet_dependency(DELL_INSTALL_CONTAINERD).unwrap();
        assert_eq!(r, "needs libseccomp2 2.6.0 or newer, but this system only offers 2.5.5-1ubuntu3.1");
    }

    #[test]
    fn dell_six_classify_five_kept_back_one_blocked() {
        let sig = dell_signals();
        let entries = parse_upgradable_list(DELL_LIST);
        let mut kept = 0;
        for e in &entries {
            let (status, note, pct) = classify_apt(&e.name, &sig);
            assert_eq!(pct, None);
            if e.name == "containerd.io" {
                assert_eq!(status, UpdateStatus::Blocked, "{}", e.name);
                let n = note.unwrap();
                assert!(n.contains("libseccomp2 2.6.0"), "{n}");
                assert!(n.contains("Even the full upgrade can't install it"), "{n}");
            } else {
                assert_eq!(status, UpdateStatus::KeptBack, "{}", e.name);
                assert!(note.unwrap().contains("companion packages"));
                kept += 1;
            }
        }
        assert_eq!(kept, 5);
    }

    #[test]
    fn g9_snapd_classifies_phased_with_percent() {
        let sig = AptSignals {
            kept_back: parse_kept_back(G9_SIM_UPGRADE),
            phased: parse_phased_deferred(G9_SIM_UPGRADE),
            phased_percents: parse_phased_percents(G9_POLICY),
            held: vec![],
            full_plan: SimPlan::default(),
            blocked_reasons: vec![],
        };
        let (status, note, pct) = classify_apt("snapd", &sig);
        assert_eq!(status, UpdateStatus::Phased);
        assert_eq!(pct, Some(60));
        assert!(note.unwrap().contains("60%"));
    }

    #[test]
    fn held_wins_and_unknown_is_ready() {
        let mut sig = AptSignals::default();
        sig.held = vec!["snapd".into()];
        sig.phased = vec!["snapd".into()];
        sig.kept_back = vec!["snapd".into()];
        assert_eq!(classify_apt("snapd", &sig).0, UpdateStatus::Held);
        let (status, note, pct) = classify_apt("something-else", &AptSignals::default());
        assert_eq!(status, UpdateStatus::Ready);
        assert!(note.is_none());
        assert_eq!(pct, None);
    }

    #[test]
    fn full_upgrade_plan_names_everything() {
        let entries = parse_upgradable_list(DELL_LIST);
        let plan = plan_from_outputs(&entries, &dell_signals());
        assert!(plan.available);
        assert_eq!(plan.waiting, vec![
            "gnome-shell-extension-zorin-dash", "linux-generic-hwe-24.04",
            "linux-headers-generic-hwe-24.04", "linux-image-generic-hwe-24.04",
            "linux-modules-nvidia-580-generic-hwe-24.04",
        ]);
        assert_eq!(plan.blocked.len(), 1);
        assert_eq!(plan.blocked[0].name, "containerd.io");
        assert!(plan.blocked[0].reason.as_deref().unwrap().contains("libseccomp2"));
        assert_eq!(plan.new_companions.len(), 12);
        assert!(plan.new_companions.contains(&"gir1.2-dbusmenu-glib-0.4".to_string()));
        assert!(plan.new_companions.contains(&"linux-image-7.0.0-38-generic".to_string()));
        assert!(plan.removals.is_empty());
        assert!(plan.needs_restart, "a new kernel is in the install set");
    }

    #[test]
    fn apply_report_explains_kept_back_in_plain_english() {
        // Dickles' screenshot, rebuilt: the safe apply installed
        // nothing and all six remained.
        let out = "0 upgraded, 0 newly installed, 0 to remove and 6 not upgraded.\n".to_string();
        let remaining = vec![
            item_s("containerd.io", UpdateStatus::Blocked),
            item_s("gnome-shell-extension-zorin-dash", UpdateStatus::KeptBack),
            item_s("linux-generic-hwe-24.04", UpdateStatus::KeptBack),
            item_s("linux-headers-generic-hwe-24.04", UpdateStatus::KeptBack),
            item_s("linux-image-generic-hwe-24.04", UpdateStatus::KeptBack),
            item_s("linux-modules-nvidia-580-generic-hwe-24.04", UpdateStatus::KeptBack),
        ];
        let r = apply_report(out, &remaining);
        assert_eq!(r.remaining, 6);
        assert_eq!(r.kept_back.len(), 5);
        assert_eq!(r.blocked, vec!["containerd.io"]);
        assert!(r.headline.contains("still remain"), "{}", r.headline);
        assert!(r.headline.contains("waiting on new companion packages"), "{}", r.headline);
        assert!(r.headline.contains("can't install yet"), "{}", r.headline);
        assert!(!r.headline.contains("fully up to date"), "{}", r.headline);
        assert!(r.summary.iter().any(|l| l.contains("Nothing was installed this time.")), "{:?}", r.summary);
        assert!(r.summary.iter().any(|l| l.starts_with("Still waiting —")), "{:?}", r.summary);
        assert!(r.summary.iter().any(|l| l.starts_with("Still blocked — containerd.io")), "{:?}", r.summary);
        // Kept-back only: the headline itself names the remedy.
        let r2 = apply_report("done\n".into(), &[
            item_s("a-pkg", UpdateStatus::KeptBack),
            item_s("b-pkg", UpdateStatus::KeptBack),
        ]);
        assert!(r2.headline.contains("Install these too"), "{}", r2.headline);
        assert!(r2.headline.contains("a-pkg"), "{}", r2.headline);
    }

    #[test]
    fn full_upgrade_command_is_fenced_and_exact() {
        // The ONLY full-upgrade in the crate lives behind this name,
        // with this exact argv. The default apply table (pinned by
        // no_apply_path_ever_dist_upgrades above) must never grow it.
        let (prog, args) = full_upgrade_command();
        assert_eq!(prog, "pkexec");
        assert_eq!(args, vec!["sh", "-c",
            "DEBIAN_FRONTEND=noninteractive apt-get full-upgrade -y"]);
        let (_, safe) = apply_command("system-apt").unwrap();
        assert!(!safe.join(" ").contains("full-upgrade"));
    }
}
