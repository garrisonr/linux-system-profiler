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
                if !name.is_empty() { info.system_updates.push(UpdateItem { name, current: cur, available: avail, source: "pacman".into() }); }
            },
            None => info.notes.push("pacman -Qu returned nothing (up to date, or checkupdates not run)".into()),
        }
        if which("checkupdates") { info.notes.push("Tip: `checkupdates` gives a sync-free count without root.".into()); }
    } else if which("apt") {
        if let Some(out) = cmd("apt", &["list", "--upgradable"]) {
            for line in out.lines().skip(1) {
                let name = line.split('/').next().unwrap_or("").to_string();
                if !name.is_empty() { info.system_updates.push(UpdateItem { name, current: None, available: None, source: "apt".into() }); }
            }
        }
    } else if which("dnf") {
        if let Some(out) = cmd("dnf", &["check-update", "--quiet"]) {
            for line in out.lines() { let n = line.split_whitespace().next().unwrap_or("");
                if !n.is_empty() { info.system_updates.push(UpdateItem { name: n.into(), current: None, available: None, source: "dnf".into() }); } }
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
                                        source: "fwupd/LVFS".into() });
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
                    current: None, available: c.next().map(|s| s.to_string()), source: "flatpak".into() }); }
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
    fn metadata_refresh_is_apt_only_and_installs_nothing() {
        let (elevate, args) = metadata_refresh_command_for(Some("apt")).unwrap();
        assert!(elevate == "pkexec" || elevate == "sudo");
        assert_eq!(args, vec!["apt-get".to_string(), "update".to_string()]);
        assert!(metadata_refresh_command_for(Some("pacman")).is_none());
        assert!(metadata_refresh_command_for(Some("dnf")).is_none());
        assert!(metadata_refresh_command_for(None).is_none());
    }
}
