use crate::model::*;

fn gb(bytes: Option<u64>) -> String {
    match bytes { Some(b) => format!("{:.1} GiB", b as f64 / 1_073_741_824.0), None => "—".into() }
}
fn o(s: &Option<String>) -> String { s.clone().unwrap_or_else(|| "—".into()) }

pub fn health_flags(p: &Profile) -> Vec<HealthFlag> {
    let mut f = Vec::new();
    let mut push = |sev: &str, comp: &str, msg: String, hint: Option<String>| {
        f.push(HealthFlag { severity: sev.into(), component: comp.into(), message: msg, hint });
    };
    // Collector degradations the profile recorded (a probe tool
    // missing/failing, a capped list) — shown, never hidden behind
    // plausible emptiness.
    for w in &p.collection_warnings {
        push("amber", "Data collection", w.clone(), None);
    }
    for b in &p.power {
        if let Some(h) = b.health_percent {
            let sev = if h >= 80.0 { "green" } else if h >= 60.0 { "amber" } else { "red" };
            push(sev, "Battery", format!("{} health {:.0}% ({} cycles)", b.name, h, b.cycle_count.map(|c| c.to_string()).unwrap_or_else(|| "?".into())),
                if sev != "green" { Some("Consider battery replacement planning; check `upower -i` history".into()) } else { None });
        }
    }
    for d in &p.storage {
        if let Some(s) = &d.smart_health {
            if s != "PASSED" { push("red", "Storage", format!("{} ({}) SMART FAILED", d.name, o(&d.model)), Some("Back up now; replace the drive".into())); }
        }
        if let Some(u) = d.percentage_used {
            if u >= 90 { push("red", "Storage", format!("{} NVMe endurance {}% used", d.name, u), Some("Plan replacement; endurance nearly exhausted".into())); }
            else if u >= 75 { push("amber", "Storage", format!("{} NVMe endurance {}% used", d.name, u), None); }
        }
        if let Some(t) = d.smart_temp_c { if t >= 70 { push("amber", "Storage", format!("{} running hot at {}°C", d.name, t), Some("Check airflow / heatsink".into())); } }
    }
    if !p.updates.system_updates.is_empty() {
        push("amber", "Updates", format!("{} system package update(s) available", p.updates.system_updates.len()),
            Some("Review in the Updates tab; apply with your package manager".into()));
    }
    if !p.updates.firmware_updates.is_empty() {
        push("amber", "Firmware", format!("{} firmware update(s) on LVFS", p.updates.firmware_updates.len()),
            Some("Apply with fwupdmgr update (GUI phase: one-click via polkit)".into()));
    }
    if p.security.firewall_active == Some(false) {
        push("amber", "Security", "No active firewall service detected (ufw/firewalld/nftables)".into(),
            Some("Enable one: sudo systemctl enable --now ufw (or your distro's firewall)".into()));
    }
    if p.security.firewall_active == Some(true) {
        push("green", "Security", format!("Firewall active ({})", o(&p.security.firewall_service)), None);
    }
    match p.system.secure_boot.as_deref() {
        Some("disabled") => push("info", "Security", "Secure Boot is disabled".into(), None),
        Some("enabled") => push("green", "Security", "Secure Boot enabled".into(), None),
        _ => {}
    }
    if let (Some(t), Some(a)) = (p.memory.total_kb, p.memory.available_kb) {
        if t > 0 && (a as f64 / t as f64) < 0.05 { push("amber", "Memory", "Less than 5% memory available at snapshot time".into(), None); }
    }
    if p.system.manufacturer.is_none() && p.system.product_name.is_none() {
        push("info", "Firmware", "DMI identity fields are OEM placeholders ('Default string') — this firmware doesn't report board identity".into(),
            Some("Common on budget mini-PCs; rely on BIOS version/date below for firmware tracking".into()));
    }
    for (tool, why) in [("lsusb", "USB inventory"), ("smartctl", "SMART health/endurance"), ("lspci", "PCI friendly names")] {
        let _ = (tool, why); // presence is probed via empty sections in v0.1; explicit probe lands in v0.2
    }
    if f.is_empty() {
        f.push(HealthFlag { severity: "green".into(), component: "System".into(),
            message: "No issues flagged by v0.1 checks".into(), hint: None });
    }
    f
}

pub fn tree_text(p: &Profile) -> String {
    let mut s = String::new();
    s += &format!("{}\n", if p.hostname.is_empty() { "this computer".to_string() } else { p.hostname.clone() });
    s += &format!("├─ System      {} {} · {} · kernel {}\n", o(&p.system.manufacturer), o(&p.system.product_name), o(&p.system.os_name), o(&p.system.kernel));
    s += &format!("├─ CPU         {} · {}c/{}t{}\n", o(&p.cpu.model_name),
        p.cpu.cores_physical.map(|c| c.to_string()).unwrap_or_else(|| "?".into()),
        p.cpu.threads_logical.map(|c| c.to_string()).unwrap_or_else(|| "?".into()),
        p.cpu.max_mhz.map(|m| format!(" · max {:.0} MHz", m)).unwrap_or_default());
    s += &format!("├─ Memory      {} total · {} modules\n", gb(p.memory.total_kb.map(|k| k * 1024)), p.memory.modules.len());
    s += "├─ Storage\n";
    for d in &p.storage { s += &format!("│    ├─ {} {} {} {}{}\n", d.name, o(&d.model), gb(d.size_bytes),
        d.tran.as_deref().unwrap_or(""), d.smart_health.as_deref().map(|h| format!(" SMART:{h}")).unwrap_or_default()); }
    s += "├─ Graphics\n";
    for g in &p.graphics { s += &format!("│    ├─ {} {} driver:{}\n", g.card, o(&g.vendor_device), o(&g.driver_in_use)); }
    for d in &p.displays { s += &format!("│    └─ display: {} {} {}\n", d.name.clone().unwrap_or_else(|| d.connector.clone()),
        d.resolution.clone().unwrap_or_default(), d.manufacturer.clone().unwrap_or_default()); }
    s += "├─ Network\n";
    for n in &p.network { s += &format!("│    ├─ {} {} {} {}\n", n.name, o(&n.operstate),
        n.speed_mbps.map(|v| format!("{v} Mb/s")).unwrap_or_default(), n.addresses.join(" ")); }
    for v in &p.volumes { s += &format!("│    ├─ vol {} {} {} free of {}\n", v.mount, v.fs_type,
        gb(v.avail_bytes), gb(v.size_bytes)); }
    s += "├─ Power\n";
    for b in &p.power { s += &format!("│    ├─ {} {}% health:{}\n", b.name, b.capacity_percent.map(|c| c.to_string()).unwrap_or_else(|| "?".into()),
        b.health_percent.map(|h| format!("{h:.0}%")).unwrap_or_else(|| "—".into())); }
    for d in &p.upower_devices { s += &format!("│    ├─ {} ({}) {} {}\n", d.label, d.kind,
        d.state.clone().unwrap_or_default(), d.percentage.map(|v| format!("{v:.0}%")).unwrap_or_default()); }
    for hst in &p.power_history { s += &format!("│    ├─ history {} — charge {} pts · rate {} pts · time-empty {} pts · time-full {} pts\n",
        hst.label, hst.charge.len(), hst.rate.len(), hst.time_empty.len(), hst.time_full.len()); }
    s += &format!("├─ PCI devices  {}\n", p.pci.len());
    s += &format!("├─ USB devices  {}\n", p.usb.len());
    if !p.software.snaps.is_empty() || !p.software.docker_images.is_empty() || p.software.docker_status.is_some() {
        let dst = p.software.docker_status.as_ref().map(|d| match d.state.as_str() {
            "running" => format!("{} docker images · {} containers ({} running)", d.images, d.containers, d.containers_running),
            "inaccessible" => "docker daemon running, socket not accessible".to_string(),
            "not-running" => "docker installed, daemon not running".to_string(),
            _ => format!("{} docker images", p.software.docker_images.len()),
        }).unwrap_or_else(|| format!("{} docker images", p.software.docker_images.len()));
        s += &format!("│    ├─ {} snaps · {}\n", p.software.snaps.len(), dst);
    }
    s += &format!("├─ Software     {} packages ({}) · {} modules loaded\n",
        p.software.packages_count.map(|c| c.to_string()).unwrap_or_else(|| "?".into()), o(&p.software.package_manager),
        p.software.loaded_modules_count.map(|c| c.to_string()).unwrap_or_else(|| "?".into()));
    s += &format!("└─ Updates      {} system · {} firmware · {} flatpak (check-only)\n",
        p.updates.system_updates.len(), p.updates.firmware_updates.len(), p.updates.flatpak_updates.len());
    s += "\nHealth\n";
    for h in &p.health { s += &format!("  [{}] {}: {}{}\n", h.severity.to_uppercase(), h.component, h.message,
        h.hint.as_deref().map(|x| format!(" — hint: {x}")).unwrap_or_default()); }
    s
}

pub fn markdown(p: &Profile, sanitized: bool) -> String {
    let mut s = String::new();
    s += "# System Profile\n\n";
    s += &format!("- **Machine:** {} {}\n- **OS:** {} · kernel {} · {}\n- **CPU:** {} ({}c/{}t)\n- **Memory:** {}\n",
        o(&p.system.manufacturer), o(&p.system.product_name), o(&p.system.os_name), o(&p.system.kernel), o(&p.system.arch),
        o(&p.cpu.model_name), p.cpu.cores_physical.map(|c| c.to_string()).unwrap_or_else(|| "?".into()),
        p.cpu.threads_logical.map(|c| c.to_string()).unwrap_or_else(|| "?".into()), gb(p.memory.total_kb.map(|k| k * 1024)));
    s += "\n## Storage\n\n";
    for d in &p.storage { s += &format!("- {} — {} — {} — SMART {} — endurance {:?}%\n", d.name, o(&d.model), gb(d.size_bytes), o(&d.smart_health), d.percentage_used); }
    s += "\n## Graphics / Network / Power\n\n";
    for g in &p.graphics { s += &format!("- GPU {} {} driver {}\n", g.card, o(&g.vendor_device), o(&g.driver_in_use)); }
    for n in &p.network { s += &format!("- NIC {} {} {}\n", n.name, o(&n.mac), n.addresses.join(", ")); }
    for b in &p.power { s += &format!("- Battery {} health {:?}%\n", b.name, b.health_percent.map(|h| h.round())); }
    if !p.displays.is_empty() {
        s += "\n## Displays\n\n";
        for d in &p.displays { s += &format!("- {} ({}) {} {}\n", o(&d.name), d.connector,
            o(&d.resolution), d.manufacturer.clone().map(|m| format!("mfr {m}")).unwrap_or_default()); }
    }
    if !p.volumes.is_empty() {
        s += "\n## Volumes\n\n";
        for v in &p.volumes { s += &format!("- {} {} — {} free of {} ({}% used)\n", v.mount, v.fs_type,
            gb(v.avail_bytes), gb(v.size_bytes), v.use_percent.map(|u| u.to_string()).unwrap_or_else(|| "?".into())); }
    }
    if !p.printers.is_empty() {
        s += "\n## Printers\n\n";
        for pr in &p.printers { s += &format!("- {}{}{}\n", pr.name, if pr.is_default { " (default)" } else { "" }, pr.state.as_deref().map(|x| format!(" — {x}")).unwrap_or_default()); }
    }
    if !p.software.snaps.is_empty() {
        s += "\n## Snaps\n\n";
        for x in &p.software.snaps { s += &format!("- {} {}\n", x.name, x.version); }
    }
    if !p.software.docker_images.is_empty() {
        s += "\n## Docker images\n\n";
        for x in &p.software.docker_images { s += &format!("- {} {}\n", x.repository_tag, o(&x.size)); }
    }
    s += "\n## Security posture\n\n";
    s += &format!("- Secure Boot: {} · Firewall: {} · Encrypted (LUKS) volumes: {} · SSH server running: {}\n",
        o(&p.system.secure_boot),
        if p.security.firewall_active == Some(true) { format!("active ({})", o(&p.security.firewall_service)) }
        else if p.security.firewall_active == Some(false) { "none active".into() } else { "unknown".into() },
        p.security.luks_volumes,
        p.security.sshd_active.map(|b| if b { "yes" } else { "no" }).unwrap_or("unknown"));
    s += &format!("- User accounts: {} ({} admin)\n", p.accounts.len(), p.accounts.iter().filter(|a| a.admin).count());
    s += "\n## Updates (check-only)\n\n";
    s += &format!("- System: {} · Firmware: {} · Flatpak: {}\n", p.updates.system_updates.len(), p.updates.firmware_updates.len(), p.updates.flatpak_updates.len());
    for u in p.updates.system_updates.iter().take(50) { s += &format!("  - {} {} -> {}{}\n", u.name,
        u.current.as_deref().unwrap_or("—"), u.available.as_deref().unwrap_or("—"),
        u.note.as_deref().map(|n| format!(" — {n}")).unwrap_or_default()); }
    s += "\n## Health flags\n\n";
    for h in &p.health { s += &format!("- **{}** {}: {}\n", h.severity, h.component, h.message); }
    if sanitized { s += "\n_Sanitized export: MACs masked, serials + hostname hashed._\n"; }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collection_warnings_become_health_flags() {
        // The house-rule fix: a degraded collector must be VISIBLE in
        // the profile's warnings surface, not hidden behind an empty
        // section. health_flags is that surface (UI + exports).
        let mut p = Profile::default();
        p.collection_warnings = vec![
            "Couldn't read the disk partition list — lsblk failed.".into(),
            "Couldn't read SMART drive health — smartctl isn't installed.".into(),
        ];
        let flags = health_flags(&p);
        let data_flags: Vec<_> = flags.iter().filter(|f| f.component == "Data collection").collect();
        assert_eq!(data_flags.len(), 2);
        assert!(data_flags.iter().all(|f| f.severity == "amber"));
        assert!(data_flags[0].message.contains("lsblk failed"));
        // And a clean profile grows no such flags.
        let clean = health_flags(&Profile::default());
        assert!(clean.iter().all(|f| f.component != "Data collection"));
    }
}
