//! Updates hub — checking is read-only; applying is a separate, explicit,
//! per-group user action spawned by the GUI via polkit, never silent.
//! `collect_updates` itself never mutates the system: every command in it is
//! a read-only query. (The elevated metadata refresh behind an explicit
//! user-initiated Check lives in `metadata_refresh_command` — it refreshes
//! package indexes only, installs nothing.)
use crate::collectors::{cmd, which};
use crate::model::{UpdateItem, UpdatesInfo};

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
    /// The plain-language headline the dialog leads with.
    pub headline: String,
}

/// Compose the report from the apply output and the RE-CHECKED group
/// items. Deferred names come from the items' check-time phasing notes,
/// unioned with names parsed from the apply output itself (both filtered
/// to items that actually remain).
pub fn apply_report(output: String, remaining_items: &[UpdateItem]) -> ApplyReport {
    let from_output = parse_phased_deferred(&output);
    let deferred: Vec<String> = remaining_items
        .iter()
        .filter(|i| i.note.is_some() || from_output.contains(&i.name))
        .map(|i| i.name.clone())
        .collect();
    let remaining = remaining_items.len();
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
    } else if !deferred.is_empty() {
        format!(
            "Finished — {} update{} still remain. {} of them ({}) are being held back by Ubuntu's phased rollout and will install on a later check; the output below shows the rest.",
            remaining,
            if remaining == 1 { "" } else { "s" },
            deferred.len(),
            deferred.join(", "),
        )
    } else {
        format!(
            "Finished — {} update{} still remain. The output below shows what happened.",
            remaining,
            if remaining == 1 { "" } else { "s" },
        )
    };
    ApplyReport { output, remaining, deferred, headline }
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
                if !name.is_empty() { info.system_updates.push(UpdateItem { name, current: cur, available: avail, source: "pacman".into(), note: None }); }
            },
            None => info.notes.push("pacman -Qu returned nothing (up to date, or checkupdates not run)".into()),
        }
        if which("checkupdates") { info.notes.push("Tip: `checkupdates` gives a sync-free count without root.".into()); }
    } else if which("apt") {
        if let Some(out) = cmd("apt", &["list", "--upgradable"]) {
            for line in out.lines().skip(1) {
                let name = line.split('/').next().unwrap_or("").to_string();
                if !name.is_empty() { info.system_updates.push(UpdateItem { name, current: None, available: None, source: "apt".into(), note: None }); }
            }
        }
        // Ubuntu phased rollouts: `apt list --upgradable` lists packages
        // apt will REFUSE to install yet ("deferred due to phasing" in a
        // simulated upgrade). Mark them at check time so the row tells
        // the truth instead of showing bare version cells, and so a
        // post-apply re-check can name them. Simulation is read-only.
        if let Some(sim) = cmd("apt-get", &["-s", "upgrade"]) {
            let deferred = parse_phased_deferred(&sim);
            if !deferred.is_empty() {
                for item in &mut info.system_updates {
                    if deferred.contains(&item.name) {
                        item.note = Some("Deferred by Ubuntu phasing — not yet released to this machine".into());
                    }
                }
            }
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
                if !n.is_empty() { info.system_updates.push(UpdateItem { name: n.into(), current: None, available: None, source: "dnf".into(), note: None }); } }
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
                                        source: "fwupd/LVFS".into(), note: None });
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
                    current: None, available: c.next().map(|s| s.to_string()), source: "flatpak".into(), note: None }); }
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
            source: "apt".into(), note: note.map(|s| s.to_string()) }
    }

    #[test]
    fn apply_report_never_claims_up_to_date_with_remainder() {
        // The v0.8.3 lie: exit 0, everything deferred, headline claimed
        // "fully up to date". The re-check drives the headline now.
        let out = "The following upgrades have been deferred due to phasing:\n  apparmor libapparmor1\n0 upgraded.\n".to_string();
        let remaining = vec![
            item("apparmor", Some("Deferred by Ubuntu phasing — not yet released to this machine")),
            item("libapparmor1", Some("Deferred by Ubuntu phasing — not yet released to this machine")),
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
            item("phased-pkg", Some("Deferred by Ubuntu phasing — not yet released to this machine")),
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
}
