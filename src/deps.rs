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
            let mut a = vec!["apt-get".into(), "install".into(), "-y".into()];
            a.extend(st.packages_to_install.iter().cloned()); a
        }
        Some("dnf") => {
            let mut a = vec!["dnf".into(), "install".into(), "-y".into()];
            a.extend(st.packages_to_install.iter().cloned()); a
        }
        _ => return None,
    };
    Some((elevate.to_string(), args))
}

/// Run the install. Returns a human log of what happened.
/// In the GUI this runs behind the polkit prompt pkexec shows; in the CLI it
/// uses pkexec when present, else sudo (which may prompt for a password).
pub fn install_deps(st: &DepStatus) -> Result<String, String> {
    let (elevate, args) = install_command(st)
        .ok_or_else(|| "No supported package manager / elevation tool found".to_string())?;
    let plan = format!("{} {}", elevate, args.join(" "));
    let out = Command::new(&elevate).args(&args).output()
        .map_err(|e| format!("failed to launch install ({plan}): {e}"))?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    if out.status.success() {
        Ok(format!("installed via `{plan}`\n{stdout}"))
    } else {
        Err(format!("install failed via `{plan}`\n{stdout}\n{stderr}"))
    }
}
