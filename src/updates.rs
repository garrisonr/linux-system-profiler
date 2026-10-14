//! Updates hub — CHECK ONLY in v0.1.
//! Applying updates is a separate, explicit user action (GUI phase will spawn
//! the distro's own tools via polkit, never silently). This module never
//! mutates the system: every command below is a read-only query.
use crate::collectors::{cmd, which};
use crate::model::{UpdateItem, UpdatesInfo};

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
