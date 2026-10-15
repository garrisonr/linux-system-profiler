use crate::model::*;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::process::Command;

pub fn read_trim(path: &str) -> Option<String> {
    fs::read_to_string(path).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}
pub fn read_first(paths: &[&str]) -> Option<String> { paths.iter().find_map(|p| read_trim(p)) }
/// Cheap OEM firmware often leaves literal placeholder strings in DMI.
/// Treat them as absent — and let the health layer flag it.
pub fn dmi_clean(path: &str) -> Option<String> {
    let v = read_trim(path)?;
    if is_dmi_junk(&v) { None } else { Some(v) }
}

/// True for the literal placeholder strings cheap OEM firmware leaves in
/// DMI fields. Shared by the sysfs reader and the dmidecode text parsers.
pub fn is_dmi_junk(v: &str) -> bool {
    let junk = ["Default string", "To be filled by O.E.M.", "Not Applicable", "Not Specified", "None", "Unknown", "SerialNumber"];
    junk.iter().any(|j| v.trim().eq_ignore_ascii_case(j))
}
pub fn cmd(program: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(program).args(args).output().ok()?;
    if out.stdout.is_empty() { None } else { Some(String::from_utf8_lossy(&out.stdout).to_string()) }
}
/// Like cmd(), but only returns output when the process exits 0.
/// Some tools (docker info) render a plausible-looking template to stdout
/// even while failing — trusting that output invents data.
pub fn cmd_ok(prog: &str, args: &[&str]) -> Option<String> {
    let out = std::process::Command::new(prog).args(args).output().ok()?;
    if !out.status.success() { return None; }
    Some(String::from_utf8_lossy(&out.stdout).to_string())
}

pub fn which(program: &str) -> bool { cmd("which", &[program]).is_some() }

fn parse_kv_file(path: &str, sep: char) -> BTreeMap<String, String> {
    let mut m = BTreeMap::new();
    if let Ok(text) = fs::read_to_string(path) {
        for line in text.lines() {
            if let Some((k, v)) = line.split_once(sep) { m.insert(k.trim().to_string(), v.trim().trim_matches('"').to_string()); }
        }
    }
    m
}

pub fn collect_system() -> SystemInfo {
    let os = parse_kv_file("/etc/os-release", '=');
    let dmi = "/sys/class/dmi/id";
    SystemInfo {
        manufacturer: dmi_clean(&format!("{dmi}/sys_vendor")),
        product_name: dmi_clean(&format!("{dmi}/product_name")),
        product_version: dmi_clean(&format!("{dmi}/product_version")),
        board_vendor: dmi_clean(&format!("{dmi}/board_vendor")),
        board_name: dmi_clean(&format!("{dmi}/board_name")),
        bios_vendor: dmi_clean(&format!("{dmi}/bios_vendor")),
        bios_version: dmi_clean(&format!("{dmi}/bios_version")),
        bios_date: dmi_clean(&format!("{dmi}/bios_date")),
        chassis_type: dmi_clean(&format!("{dmi}/chassis_type")),
        os_name: os.get("PRETTY_NAME").cloned(),
        os_id: os.get("ID").cloned(),
        os_version_id: os.get("VERSION_ID").cloned(),
        kernel: cmd("uname", &["-r"]).map(|s| s.trim().to_string()),
        arch: cmd("uname", &["-m"]).map(|s| s.trim().to_string()),
        uptime_seconds: fs::read_to_string("/proc/uptime").ok()
            .and_then(|s| s.split_whitespace().next()?.parse::<f64>().ok()).map(|f| f as u64),
        secure_boot: secure_boot(),
        session_type: std::env::var("XDG_SESSION_TYPE").ok(),
        desktop: std::env::var("XDG_CURRENT_DESKTOP").ok()
            .or_else(|| std::env::var("DESKTOP_SESSION").ok()),
    }
}

fn secure_boot() -> Option<String> {
    // EFI SecureBoot efivar: last byte 1 = enabled
    let p = "/sys/firmware/efi/efivars/SecureBoot-8be4df61-93ca-11d2-aa0d-00e098032b8c";
    if let Ok(bytes) = fs::read(p) {
        if let Some(b) = bytes.last() { return Some(if *b == 1 { "enabled".into() } else { "disabled".into() }); }
    }
    cmd("mokutil", &["--sb-state"]).map(|s| s.trim().to_string())
}

pub fn collect_cpu() -> CpuInfo {
    let CPU_TEMP = cpu_temp();
    let text = fs::read_to_string("/proc/cpuinfo").unwrap_or_default();
    let mut model = None; let mut vendor = None; let mut flags = Vec::new();
    let mut phys_ids = std::collections::BTreeSet::new();
    let mut core_pairs = std::collections::BTreeSet::new();
    let mut cur_phys: Option<String> = None;
    let mut cores_per: Option<u32> = None; let mut threads = 0u32;
    for line in text.lines() {
        let Some((k, v)) = line.split_once(':') else { continue };
        match k.trim() {
            "model name" if model.is_none() => model = Some(v.trim().to_string()),
            "vendor_id" if vendor.is_none() => vendor = Some(v.trim().to_string()),
            "physical id" => { let p = v.trim().to_string(); phys_ids.insert(p.clone()); cur_phys = Some(p); }
            "core id" => { if let Some(p) = &cur_phys { core_pairs.insert((p.clone(), v.trim().to_string())); } }
            "cpu cores" if cores_per.is_none() => cores_per = v.trim().parse().ok(),
            "processor" => threads += 1,
            "flags" | "Features" if flags.is_empty() => {
                flags = v.split_whitespace().take(24).map(|s| s.to_string()).collect();
            }
            _ => {}
        }
    }
    if model.is_none() { // ARM etc.
        if let Some(out) = cmd("lscpu", &[]) {
            for line in out.lines() {
                if line.starts_with("Model name:") { model = Some(line.split(':').nth(1).unwrap_or("").trim().to_string()); }
            }
        }
    }
    let sockets = if phys_ids.is_empty() { None } else { Some(phys_ids.len() as u32) };
    // v0.6: count UNIQUE (physical id, core id) pairs — sockets × cores-per
    // misreports hybrid CPUs (P/E clusters report differing "cpu cores");
    // fall back to the product only when no core ids exist (ARM etc.).
    let cores = if !core_pairs.is_empty() { Some(core_pairs.len() as u32) }
        else { match (sockets, cores_per) { (Some(s), Some(c)) => Some(s * c), _ => None } };
    let virt = if flags.iter().any(|f| f == "vmx" || f == "svm") { Some("hardware (vmx/svm)".into()) }
        else if text.contains("hypervisor") { Some("running under hypervisor".into()) } else { None };
    CpuInfo {
        model_name: model, vendor,
        sockets, cores_physical: cores,
        threads_logical: if threads > 0 { Some(threads) } else { None },
        max_mhz: read_trim("/sys/devices/system/cpu/cpu0/cpufreq/cpuinfo_max_freq")
            .and_then(|s| s.parse::<f64>().ok()).map(|khz| khz / 1000.0),
        cur_mhz: read_trim("/sys/devices/system/cpu/cpu0/cpufreq/scaling_cur_freq")
            .and_then(|s| s.parse::<f64>().ok()).map(|khz| khz / 1000.0),
        cache_l3: cmd("lscpu", &[]).and_then(|o| o.lines().find(|l| l.starts_with("L3 cache:"))
            .map(|l| l.split(':').nth(1).unwrap_or("").trim().to_string())),
        flags_sample: flags, temp_c: CPU_TEMP.0, tjmax_c: CPU_TEMP.1, virtualization: virt,
    }
}

pub fn parse_dmidecode_memory(out: &str) -> Vec<MemoryModule> {
    let mut modules = Vec::new();
    let mut cur: BTreeMap<String, String> = BTreeMap::new();
    let flush = |cur: &BTreeMap<String, String>, modules: &mut Vec<MemoryModule>| {
        if let Some(loc) = cur.get("Locator") {
            if cur.get("Size").map(|s| !s.contains("No Module")).unwrap_or(false) {
                modules.push(MemoryModule { locator: loc.clone(),
                    size: cur.get("Size").cloned(), mem_type: cur.get("Type").cloned(),
                    speed: cur.get("Speed").cloned(), manufacturer: cur.get("Manufacturer").cloned(),
                    part_number: cur.get("Part Number").cloned() });
            }
        }
    };
    for line in out.lines() {
        if line.starts_with("Memory Device") && !cur.is_empty() { flush(&cur, &mut modules); cur.clear(); }
        if let Some((k, v)) = line.trim().split_once(':') { cur.insert(k.trim().to_string(), v.trim().to_string()); }
    }
    if !cur.is_empty() { flush(&cur, &mut modules); }
    modules
}

/// System serial from a full dmidecode dump (System Information block).
pub fn parse_dmidecode_serial(out: &str) -> Option<String> {
    let mut in_system = false;
    for line in out.lines() {
        if line.starts_with("System Information") { in_system = true; continue; }
        if in_system {
            if line.starts_with("Handle ") || (!line.starts_with([' ', '\t']) && !line.trim().is_empty() && !line.contains(':')) { break; }
            if let Some((k, v)) = line.trim().split_once(':') {
                if k.trim() == "Serial Number" {
                    let v = v.trim().to_string();
                    if !v.is_empty() && !is_dmi_junk(&v) { return Some(v); }
                }
            }
        }
    }
    None
}

/// Passwordless-sudo availability, probed ONCE per process (v0.6).
/// Collectors may use `sudo -n` only when this passes: on machines without
/// a NOPASSWD grant a bare probe fails instantly, and collectors then skip
/// every silent-elevation attempt instead of spraying auth failures into
/// the system log on every refresh (Gemini review P1).
pub fn sudo_n_ok() -> bool {
    use std::sync::OnceLock;
    static OK: OnceLock<bool> = OnceLock::new();
    *OK.get_or_init(|| cmd_ok("timeout", &["4", "sudo", "-n", "true"]).is_some())
}

pub fn collect_memory() -> MemoryInfo {
    let kv = parse_kv_file("/proc/meminfo", ':');
    let get = |k: &str| kv.get(k).and_then(|v| v.split_whitespace().next()?.parse::<u64>().ok());
    // dmidecode needs root. Plain call works as root; otherwise try a
    // NON-INTERACTIVE sudo (never prompts; skipped where a password is
    // required — the GUI offers a one-shot polkit unlock for those users).
    // Note: an unprivileged dmidecode can exit 0 with EMPTY data (sysfs is
    // root-only), so "no modules parsed" — not just failure — must fall
    // through to the sudo attempt.
    let mut modules = cmd("dmidecode", &["-t", "memory"])
        .map(|out| parse_dmidecode_memory(&out))
        .unwrap_or_default();
    if modules.is_empty() && sudo_n_ok() {
        modules = cmd_ok("timeout", &["6", "sudo", "-n", "dmidecode", "-t", "memory"])
            .map(|out| parse_dmidecode_memory(&out))
            .unwrap_or_default();
    }
    MemoryInfo { total_kb: get("MemTotal"), available_kb: get("MemAvailable"),
        swap_total_kb: get("SwapTotal"), modules }
}

/// Wear percentage for SATA SSDs / eMMC from ATA SMART attributes (v0.6):
/// 231 SSD_Life_Left (raw = % remaining), 233 Media_Wearout_Indicator and
/// 177 Wear_Leveling_Count (normalized value = % remaining). NVMe keeps
/// using its own percentage_used log; this only fills the gap.
pub fn sata_wear_percent(v: &serde_json::Value) -> Option<i64> {
    let attrs = v.pointer("/ata_smart_attributes/table")?.as_array()?;
    for a in attrs {
        let id = a.get("id").and_then(|n| n.as_i64());
        match id {
            Some(231) => {
                if let Some(r) = a.pointer("/raw/value").and_then(|n| n.as_i64()) {
                    return Some((100 - r).clamp(0, 100));
                }
            }
            Some(233) | Some(177) => {
                if let Some(n) = a.get("value").and_then(|n| n.as_i64()) {
                    return Some((100 - n).clamp(0, 100));
                }
            }
            _ => {}
        }
    }
    None
}

pub fn collect_storage() -> Vec<StorageDevice> {
    let mut devs = Vec::new();
    let Ok(entries) = fs::read_dir("/sys/class/block") else { return devs };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if name.starts_with("loop") || name.starts_with("ram") || name.starts_with("dm-") || name.starts_with("zram") { continue; }
        let base = format!("/sys/class/block/{name}");
        if Path::new(&format!("{base}/partition")).exists() { continue; } // skip partitions, keep whole disks
        let size = read_trim(&format!("{base}/size")).and_then(|s| s.parse::<u64>().ok()).map(|s| s * 512);
        let model = read_first(&[&format!("{base}/device/model"), &format!("{base}/device/name")]);
        let mut d = StorageDevice { name: name.clone(), model,
            serial: read_trim(&format!("{base}/device/serial")),
            size_bytes: size,
            rotational: read_trim(&format!("{base}/queue/rotational")).map(|s| s == "1"),
            tran: None, link: None, mountpoints: vec![], fstype: None,
            smart_health: None, smart_temp_c: None, power_on_hours: None, percentage_used: None };
        // Enrich from lsblk + smartctl when present (graceful if absent / no permission)
        if let Some(out) = cmd("lsblk", &["-J", "-o", "NAME,MODEL,SERIAL,TRAN,MOUNTPOINTS,FSTYPE,LINK"]) {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&out) {
                if let Some(arr) = v.get("blockdevices").and_then(|b| b.as_array()) {
                    for item in arr { if item.get("name").and_then(|n| n.as_str()) == Some(&name) {
                        d.tran = item.get("tran").and_then(|x| x.as_str()).map(|s| s.to_string());
                        d.link = item.get("link").and_then(|x| x.as_str()).map(|s| s.to_string());
                        if d.model.is_none() { d.model = item.get("model").and_then(|x| x.as_str()).map(|s| s.trim().to_string()); }
                        if let Some(mps) = item.get("mountpoints").and_then(|x| x.as_array()) {
                            d.mountpoints = mps.iter().filter_map(|m| m.as_str().map(|s| s.to_string())).collect();
                        }
                        d.fstype = item.get("fstype").and_then(|x| x.as_str()).map(|s| s.to_string());
                    }}
                }
            }
        }
        if which("smartctl") {
            if let Some(out) = cmd("smartctl", &["-j", "-a", &format!("/dev/{name}")]) {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&out) {
                    d.smart_health = v.pointer("/smart_status/passed").and_then(|b| b.as_bool())
                        .map(|p| if p { "PASSED".into() } else { "FAILED".into() });
                    d.smart_temp_c = v.pointer("/temperature/current").and_then(|n| n.as_i64());
                    d.power_on_hours = v.pointer("/power_on_time/hours").and_then(|n| n.as_i64());
                    d.percentage_used = v.pointer("/nvme_smart_health_information_log/percentage_used").and_then(|n| n.as_i64());
                    if d.percentage_used.is_none() { d.percentage_used = sata_wear_percent(&v); }
                }
            }
        }
        devs.push(d);
    }
    devs.sort_by(|a, b| a.name.cmp(&b.name));
    devs
}

fn classify_iface(name: &str, base: &str) -> (String, bool) {
    if Path::new(&format!("{base}/wireless")).exists() || Path::new(&format!("{base}/phy80211")).exists() {
        return ("Wi-Fi".into(), false);
    }
    if Path::new(&format!("{base}/bridge")).exists()
        || name.starts_with("docker") || name.starts_with("br-") || name.starts_with("virbr") {
        return ("Bridge".into(), true);
    }
    if name.starts_with("veth") { return ("Virtual Ethernet".into(), true); }
    if name.starts_with("tailscale") || name.starts_with("wg") || name.starts_with("tun")
        || name.starts_with("tap") || name.starts_with("proton") || name.contains("leak") {
        return ("VPN / Tunnel".into(), true);
    }
    if Path::new(&format!("{base}/device")).exists() { return ("Ethernet".into(), false); }
    ("Virtual".into(), true)
}

pub fn collect_network() -> Vec<NetworkInterface> {
    let mut ifs = Vec::new();
    let Ok(entries) = fs::read_dir("/sys/class/net") else { return ifs };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if name == "lo" { continue; }
        let base = format!("/sys/class/net/{name}");
        let (kind, is_virtual) = classify_iface(&name, &base);
        let driver = fs::read_link(format!("{base}/device/driver")).ok()
            .and_then(|p| p.file_name().map(|s| s.to_string_lossy().to_string()));
        let mut addresses = Vec::new();
        if let Some(out) = cmd("ip", &["-j", "addr", "show", &name]) {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&out) {
                if let Some(ai) = v.get(0).and_then(|x| x.get("addr_info")).and_then(|a| a.as_array()) {
                    addresses = ai.iter().filter_map(|a| {
                        let ip = a.get("local")?.as_str()?; let pre = a.get("prefixlen")?.as_u64()?;
                        Some(format!("{ip}/{pre}"))
                    }).collect();
                }
            }
        }
        // IPv4 first, then global IPv6, link-local last — humans read v4 first.
        addresses.sort_by_key(|a| {
            if !a.contains(':') { 0 } else if a.to_lowercase().starts_with("fe80") { 2 } else { 1 }
        });
        ifs.push(NetworkInterface { name: name.clone(), kind, is_virtual,
            mac: read_trim(&format!("{base}/address")),
            operstate: read_trim(&format!("{base}/operstate")),
            // Virtual interfaces report fantasy speeds (10 Gb/s docker bridges); only trust hardware.
            speed_mbps: if is_virtual { None } else {
                read_trim(&format!("{base}/speed")).and_then(|s| s.parse::<i64>().ok()).filter(|v| *v > 0)
            },
            driver, addresses,
            wifi: if Path::new(&format!("{base}/wireless")).exists() {
                Some(WifiInfo { phy_modes: read_trim(&format!("{base}/uevent")),
                    signal_dbm: cmd("iw", &["dev", &name, "link"]).and_then(|o| o.lines()
                        .find(|l| l.contains("signal:")).map(|l| l.trim().to_string())) })
            } else { None } });
    }
    // Physical first, then virtual; alphabetical within each group.
    ifs.sort_by(|a, b| a.is_virtual.cmp(&b.is_virtual).then(a.name.cmp(&b.name)));
    ifs
}

pub fn collect_graphics() -> Vec<GpuInfo> {
    let mut gpus = Vec::new();
    if let Ok(entries) = fs::read_dir("/sys/class/drm") {
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if !name.starts_with("card") || name.contains('-') { continue; }
            let base = e.path();
            let vendor_device = {
                let v = fs::read_to_string(base.join("device/vendor")).unwrap_or_default();
                let d = fs::read_to_string(base.join("device/device")).unwrap_or_default();
                let s = format!("{}:{}", v.trim(), d.trim());
                if s == ":" { None } else { Some(s) }
            };
            let driver = fs::read_link(base.join("device/driver")).ok()
                .and_then(|p| p.file_name().map(|s| s.to_string_lossy().to_string()));
            gpus.push(GpuInfo { card: name, vendor_device, driver_in_use: driver, vram_mb: None });
        }
    }
    if let Some(out) = cmd("lspci", &["-k"]) {
        for chunk in out.split("\n\n") {
            if chunk.contains("VGA compatible controller") || chunk.contains("3D controller") || chunk.contains("Display controller") {
                let drv = chunk.lines().find(|l| l.contains("Kernel driver in use:"))
                    .map(|l| l.split(':').last().unwrap_or("").trim().to_string());
                if let Some(g) = gpus.iter_mut().find(|g| g.driver_in_use.is_none() || drv.is_some()) {
                    if g.vendor_device.is_some() { g.driver_in_use = drv.clone(); }
                }
            }
        }
    }
    gpus
}

fn parse_edid(data: &[u8], connector: &str) -> DisplayInfo {
    let mut d = DisplayInfo { connector: connector.to_string(), ..Default::default() };
    if data.len() < 128 { return d; }
    let v = ((data[8] as u16) << 8) | data[9] as u16;
    let letter = |n: u16| if (1..=26).contains(&n) { char::from(b'A' + (n as u8) - 1) } else { '?' };
    let mfr: String = [letter((v >> 10) & 0x1f), letter((v >> 5) & 0x1f), letter(v & 0x1f)].iter().collect();
    if mfr != "???" { d.manufacturer = Some(mfr); }
    let year = data[17] as u32 + 1990;
    if (1990..=2035).contains(&year) { d.manufacture_year = Some(year); }
    if data[21] > 0 { d.width_cm = Some(data[21] as u32); }
    if data[22] > 0 { d.height_cm = Some(data[22] as u32); }
    for off in [54usize, 72, 90, 108] {
        if data[off] == 0 && data[off + 1] == 0 && data[off + 2] == 0 {
            let raw = &data[off + 5..off + 18];
            let s = String::from_utf8_lossy(raw).split('\n').next().unwrap_or("").trim().to_string();
            match data[off + 3] {
                0xfc if !s.is_empty() => d.name = Some(s),
                0xff if !s.is_empty() => d.serial = Some(s),
                _ => {}
            }
        } else if d.refresh_hz.is_none() {
            // Detailed TIMING descriptor (nonzero pixel clock): the first
            // one is the panel's preferred mode — derive its refresh rate.
            let clock_hz = (u16::from_le_bytes([data[off], data[off + 1]]) as u64) * 10_000;
            let h_total = ((data[off + 2] as u64) | (((data[off + 4] as u64) & 0xf0) << 4))
                + ((data[off + 3] as u64) | (((data[off + 4] as u64) & 0x0f) << 8));
            let v_total = ((data[off + 5] as u64) | (((data[off + 7] as u64) & 0xf0) << 4))
                + ((data[off + 6] as u64) | (((data[off + 7] as u64) & 0x0f) << 8));
            if clock_hz > 0 && h_total > 0 && v_total > 0 {
                // Round, don't floor: real modes are 59.94/59.97-style rates
                // and every other tool reports them as the marketed number.
                let denom = h_total * v_total;
                let hz = (clock_hz + denom / 2) / denom;
                if (20..=500).contains(&hz) { d.refresh_hz = Some(hz as u32); }
            }
        }
    }
    d
}

pub fn collect_displays() -> Vec<DisplayInfo> {
    let mut out = Vec::new();
    if let Ok(entries) = fs::read_dir("/sys/class/drm") {
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if !name.starts_with("card") || !name.contains('-') { continue; }
            let base = e.path();
            let connected = read_trim(base.join("status").to_str().unwrap_or("")) .as_deref() == Some("connected");
            let edid = fs::read(base.join("edid")).unwrap_or_default();
            if !connected && edid.is_empty() { continue; }
            let connector = name.splitn(2, '-').nth(1).unwrap_or(&name).to_string();
            let mut d = parse_edid(&edid, &connector);
            d.resolution = read_trim(base.join("modes").to_str().unwrap_or(""))
                .and_then(|s| s.lines().next().map(|l| l.trim().to_string()));
            out.push(d);
        }
    }
    if out.is_empty() { // fallback: xrandr summary lines
        if let Some(text) = cmd("xrandr", &["--current"]) {
            for line in text.lines().filter(|l| l.contains(" connected")) {
                let connector = line.split_whitespace().next().unwrap_or("display").to_string();
                let res = line.split_whitespace().find(|t| t.contains('x') && t.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false))
                    .map(|s| s.to_string());
                out.push(DisplayInfo { connector, resolution: res, ..Default::default() });
            }
        }
    }
    out.sort_by(|a, b| a.connector.cmp(&b.connector));
    out
}

pub fn collect_power() -> Vec<BatteryInfo> {
    let mut bats = Vec::new();
    let Ok(entries) = fs::read_dir("/sys/class/power_supply") else { return bats };
    for e in entries.flatten() {
        let base = e.path();
        if read_trim(base.join("type").to_str().unwrap_or("")) .as_deref() != Some("Battery") { continue; }
        let g = |k: &str| read_trim(base.join(k).to_str().unwrap_or("")).and_then(|s| s.parse::<i64>().ok());
        let full = g("charge_full").or_else(|| g("energy_full"));
        let design = g("charge_full_design").or_else(|| g("energy_full_design"));
        let health = match (full, design) { (Some(f), Some(d)) if d > 0 => Some(f as f64 / d as f64 * 100.0), _ => None };
        bats.push(BatteryInfo { name: e.file_name().to_string_lossy().to_string(),
            status: read_trim(base.join("status").to_str().unwrap_or("")),
            capacity_percent: g("capacity"), charge_full: full, charge_full_design: design,
            health_percent: health, cycle_count: g("cycle_count"),
            manufacturer: read_trim(base.join("manufacturer").to_str().unwrap_or("")),
            model_name: read_trim(base.join("model_name").to_str().unwrap_or("")) });
    }
    bats
}

pub fn classify_pci(class: &str) -> &'static str {
    let c = class.to_lowercase();
    if c.contains("vga") || c.contains("3d") || c.contains("display") { "Graphics & Displays" }
    else if c.contains("network") || c.contains("ethernet") || c.contains("wireless") { "Networking" }
    else if c.contains("mass storage") || c.contains("sata") || c.contains("non-volatile") || c.contains("scsi") || c.contains("raid") || c.contains("nvme") { "Storage" }
    else if c.contains("audio") || c.contains("multimedia") { "Audio & Multimedia" }
    else if c.contains("usb") || c.contains("serial bus") || c.contains("input") { "Peripherals & USB" }
    else if c.contains("bridge") || c.contains("smbus") || c.contains("ram memory") || c.contains("system") || c.contains("communication") || c.contains("pic") || c.contains("dma") || c.contains("encryption") || c.contains("signal") { "System & Bridges" }
    else { "Other" }
}

pub fn classify_usb(id: &str, desc: &str) -> &'static str {
    let d = desc.to_lowercase();
    if id.starts_with("1d6b:") || d.contains("root hub") || d.ends_with(" hub") || d.contains(" hub ") { "Internal Controllers" }
    else if d.contains("bluetooth") || d.contains("wireless") || d.contains("802.11") || d.contains("ethernet") || d.contains("network") { "Networking & Wireless" }
    else if d.contains("keyboard") || d.contains("mouse") || d.contains("trackball") || d.contains("receiver") || d.contains("camera") || d.contains("webcam") || d.contains("headset") || d.contains("microphone") || d.contains("gamepad") || d.contains("joystick") { "Peripherals & Input" }
    else if d.contains("storage") || d.contains("card reader") || d.contains("flash") || d.contains("disk") { "Storage" }
    else { "Other Devices" }
}

/// (temp_c, tjmax_c) from hwmon — coretemp/k10temp/zenpower package sensor.
pub fn cpu_temp() -> (Option<f64>, Option<f64>) {
    let Ok(hwmons) = fs::read_dir("/sys/class/hwmon") else { return (None, None) };
    for e in hwmons.flatten() {
        let base = e.path();
        let name = read_trim(base.join("name").to_str().unwrap_or("")).unwrap_or_default();
        if !["coretemp", "k10temp", "zenpower", "cpu_thermal"].contains(&name.as_str()) { continue; }
        let mut pkg: Option<(f64, Option<f64>)> = None;
        let mut cores: Vec<f64> = Vec::new();
        for i in 1..=64 {
            let Some(raw) = read_trim(base.join(format!("temp{i}_input")).to_str().unwrap_or(""))
                .and_then(|s| s.parse::<f64>().ok()) else { continue };
            let label = read_trim(base.join(format!("temp{i}_label")).to_str().unwrap_or("")).unwrap_or_default();
            let crit = read_trim(base.join(format!("temp{i}_crit")).to_str().unwrap_or(""))
                .and_then(|s| s.parse::<f64>().ok()).map(|v| v / 1000.0);
            if label.contains("Package") || label.contains("Tctl") || label.contains("Tdie") {
                pkg = Some((raw / 1000.0, crit));
            } else { cores.push(raw / 1000.0); }
        }
        if let Some((t, crit)) = pkg { return (Some(t), crit); }
        if let Some(m) = cores.iter().copied().reduce(f64::max) { return (Some(m), None); }
    }
    (None, None)
}

pub fn collect_power_detail() -> PowerDetail {
    let mut d = PowerDetail::default();
    if let Ok(entries) = fs::read_dir("/sys/class/power_supply") {
        for e in entries.flatten() {
            let base = e.path();
            if read_trim(base.join("type").to_str().unwrap_or("")).as_deref() == Some("Mains") {
                d.ac_online = read_trim(base.join("online").to_str().unwrap_or("")).map(|s| s == "1");
            }
        }
    }
    d.governor = read_trim("/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor");
    d.epp = read_trim("/sys/devices/system/cpu/cpu0/cpufreq/energy_performance_preference");
    if which("upower") {
        if let Some(list) = cmd("timeout", &["5", "upower", "-e"]) {
            for path in list.lines().map(|s| s.trim()).filter(|s| s.contains("ups") || s.contains("UPS")) {
                if let Some(info) = cmd("timeout", &["5", "upower", "-i", path]) {
                    let get = |k: &str| info.lines().find(|l| l.trim_start().starts_with(k))
                        .and_then(|l| l.split(':').nth(1)).map(|s| s.trim().to_string());
                    d.ups = Some(UpsInfo {
                        model: get("model"),
                        state: get("state"),
                        percentage: get("percentage").and_then(|s| s.trim_end_matches('%').parse::<f64>().ok()),
                    });
                }
            }
        }
    }
    d
}

pub fn collect_bus() -> (Vec<BusDevice>, Vec<BusDevice>) {
    let mut pci = Vec::new(); let mut usb = Vec::new();
    if let Some(out) = cmd("lspci", &["-mm"]) {
        for line in out.lines() {
            let parts: Vec<&str> = line.split('"').collect();
            if parts.len() >= 6 {
                let class = parts[1].to_string();
                pci.push(BusDevice { id: parts[0].trim().to_string(),
                    description: format!("{}: {}", class, parts[5]), driver: None,
                    category: Some(classify_pci(&class).to_string()) });
            }
        }
    } else if let Ok(entries) = fs::read_dir("/sys/bus/pci/devices") {
        for e in entries.flatten() {
            let p = e.path();
            let v = fs::read_to_string(p.join("vendor")).unwrap_or_default();
            let d = fs::read_to_string(p.join("device")).unwrap_or_default();
            pci.push(BusDevice { id: e.file_name().to_string_lossy().to_string(),
                description: format!("pci {}:{}", v.trim(), d.trim()),
                driver: fs::read_link(p.join("driver")).ok().and_then(|x| x.file_name().map(|s| s.to_string_lossy().to_string())),
                category: Some("System & Bridges".to_string()) });
        }
    }
    if let Some(out) = cmd("lsusb", &[]) {
        for line in out.lines() {
            let id = line.split_whitespace().nth(5).unwrap_or("").to_string();
            let desc = line.splitn(7, ' ').nth(6).unwrap_or(line).to_string();
            let category = classify_usb(&id, &desc).to_string();
            usb.push(BusDevice { id, description: desc, driver: None, category: Some(category) });
        }
    }
    (pci, usb)
}

pub fn collect_audio() -> Vec<String> {
    let mut v = Vec::new();
    if let Some(out) = cmd("pactl", &["list", "short", "sinks"]) { v = out.lines().map(|s| s.to_string()).collect(); }
    else if let Ok(entries) = fs::read_dir("/sys/class/sound") {
        v = entries.flatten().map(|e| e.file_name().to_string_lossy().to_string()).collect();
    }
    v
}

/// Docker daemon status ladder: never report a bare "0 images" when the
/// daemon is actually running but its socket is out of reach — say so.
pub fn collect_docker_status() -> Option<DockerStatus> {
    if !which("docker") { return None; }
    let parse_info = |o: &str| {
        let f: Vec<&str> = o.trim().split('|').collect();
        if f.len() < 3 { return None; }
        Some(DockerStatus { state: "running".into(),
            images: f[0].parse().unwrap_or(0), containers: f[1].parse().unwrap_or(0),
            containers_running: f[2].parse().unwrap_or(0) })
    };
    let fmt = "{{.Images}}|{{.Containers}}|{{.ContainersRunning}}";
    if let Some(o) = cmd_ok("timeout", &["5", "docker", "info", "--format", fmt]).filter(|s| !s.trim().is_empty()) {
        if let Some(s) = parse_info(&o) { return Some(s); }
    }
    if sudo_n_ok() { if let Some(o) = cmd_ok("timeout", &["6", "sudo", "-n", "docker", "info", "--format", fmt]).filter(|s| !s.trim().is_empty()) {
        if let Some(s) = parse_info(&o) { return Some(s); }
    } }
    let socket = std::path::Path::new("/var/run/docker.sock").exists();
    let bridge = std::path::Path::new("/sys/class/net/docker0").exists();
    // Rootless Docker keeps its socket under $XDG_RUNTIME_DIR, and a set
    // DOCKER_HOST means the daemon lives somewhere non-default — both make
    // "not-running" a guess we can't stand behind, so they join the
    // "inaccessible" (can't tell) side of the ladder.
    let rootless_socket = std::env::var_os("XDG_RUNTIME_DIR")
        .map(|d| std::path::Path::new(&d).join("docker.sock").exists()).unwrap_or(false);
    let docker_host = std::env::var_os("DOCKER_HOST").map(|v| !v.is_empty()).unwrap_or(false);
    Some(DockerStatus {
        state: docker_unreachable_state(socket, bridge, rootless_socket, docker_host).into(),
        containers: 0, containers_running: 0, images: 0 })
}

/// When `docker info` failed: any sign of a daemon we simply cannot reach
/// (system socket, bridge interface, rootless socket, custom DOCKER_HOST)
/// means "inaccessible"; only a total absence of evidence earns
/// "not-running".
fn docker_unreachable_state(socket: bool, bridge: bool, rootless_socket: bool, docker_host: bool) -> &'static str {
    if socket || bridge || rootless_socket || docker_host { "inaccessible" } else { "not-running" }
}

/// Parse "name version" lines (pacman -Qe, dpkg-query -W, rpm -qa --qf all
/// produce this shape) into NamedVersions.
fn parse_pkg_lines(out: &str) -> Vec<NamedVersion> {
    out.lines().filter_map(|l| {
        let mut it = l.splitn(2, ' ');
        let name = it.next()?.trim().to_string();
        if name.is_empty() { return None; }
        Some(NamedVersion { name, version: it.next().unwrap_or("").trim().to_string() })
    }).collect()
}

/// Last "[ALPM] upgraded" timestamp from pacman.log contents.
fn parse_pacman_log_last_upgrade(log: &str) -> Option<String> {
    log.lines().rev().find(|l| l.contains("[ALPM] upgraded"))
        .map(|l| l.trim_start_matches('[').split(']').next().unwrap_or("").to_string())
}

/// Last upgrade timestamp from dpkg.log contents. Lines look like
/// `2026-10-01 12:34:56 upgrade pkg:amd64 1.0 1.1` — the leading
/// date + time fields are the timestamp.
fn parse_dpkg_log_last_upgrade(log: &str) -> Option<String> {
    log.lines().rev().find(|l| {
        let f: Vec<&str> = l.split_whitespace().collect();
        f.len() >= 3 && f[2] == "upgrade"
    }).map(|l| {
        let f: Vec<&str> = l.split_whitespace().collect();
        format!("{} {}", f[0], f[1])
    })
}

pub fn collect_software() -> SoftwareInfo {
    let modules: Vec<String> = fs::read_to_string("/proc/modules").unwrap_or_default()
        .lines().filter_map(|l| l.split_whitespace().next().map(|s| s.to_string())).collect();
    let (pkg_count, pm) = if which("pacman") { (cmd("pacman", &["-Q"]).map(|o| o.lines().count()), Some("pacman".to_string()))
        } else if which("dpkg-query") { (cmd("dpkg-query", &["-l"]).map(|o| o.lines().count()), Some("dpkg".to_string()))
        } else if which("rpm") { (cmd("rpm", &["-qa"]).map(|o| o.lines().count()), Some("rpm".to_string()))
        } else { (None, None) };
    // Explicit (user-chosen) packages per distro family — v0.8.0 only did
    // pacman, so apt/rpm users got an empty card on the profiler's home
    // turf (Gemini audit 2026-10-14). Debian's analog of `pacman -Qe` is
    // apt-mark's manual set joined with dpkg's version table; Fedora's is
    // dnf's userinstalled set, falling back to the full rpm list.
    let explicit_packages = if which("pacman") {
        cmd("pacman", &["-Qe"]).map(|o| parse_pkg_lines(&o).into_iter().take(600).collect()).unwrap_or_default()
    } else if which("dpkg-query") {
        let manual: std::collections::HashSet<String> = cmd("apt-mark", &["showmanual"])
            .map(|o| o.lines().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect())
            .unwrap_or_default();
        cmd("dpkg-query", &["-W", "-f=${Package} ${Version}\n"])
            .map(|o| parse_pkg_lines(&o).into_iter()
                .filter(|p| manual.is_empty() || manual.contains(&p.name))
                .take(600).collect())
            .unwrap_or_default()
    } else if which("rpm") {
        let userinstalled = if which("dnf") {
            cmd("timeout", &["10", "dnf", "repoquery", "--userinstalled", "--qf", "%{name} %{version}-%{release}"])
                .filter(|o| !o.trim().is_empty())
        } else { None };
        userinstalled.or_else(|| cmd("rpm", &["-qa", "--qf", "%{name} %{version}-%{release}\n"]))
            .map(|o| parse_pkg_lines(&o).into_iter().take(600).collect())
            .unwrap_or_default()
    } else { Vec::new() };
    let last_upgrade = fs::read_to_string("/var/log/pacman.log").ok()
        .and_then(|log| parse_pacman_log_last_upgrade(&log))
        .or_else(|| fs::read_to_string("/var/log/dpkg.log").ok()
            .and_then(|log| parse_dpkg_log_last_upgrade(&log)))
        .or_else(|| {
            // rpm: `rpm -qa --last` sorts newest-first; the first line's
            // trailing date is the most recent package change.
            cmd("rpm", &["-qa", "--last"]).and_then(|o| {
                let line = o.lines().next()?.trim().to_string();
                let mut it = line.splitn(2, char::is_whitespace);
                it.next()?;
                it.next().map(|d| d.trim().to_string()).filter(|d| !d.is_empty())
            })
        });
    let snaps = if which("snap") {
        cmd("timeout", &["5", "snap", "list"]).map(|o| o.lines().skip(1).filter_map(|l| {
            let t: Vec<&str> = l.split_whitespace().collect();
            if t.len() < 2 { return None; }
            Some(SnapInfo { name: t[0].to_string(), version: t[1].to_string(),
                publisher: t.get(4).map(|s| s.to_string()) })
        }).collect()).unwrap_or_default()
    } else { Vec::new() };
    let docker_status = collect_docker_status();
    let docker_args = ["images", "--format", "{{.Repository}}:{{.Tag}}\t{{.Size}}\t{{.ID}}"];
    let docker_out = if which("docker") {
        // The docker socket is group-restricted; most users can't read it.
        // Try plainly first, then a NON-INTERACTIVE sudo (never prompts —
        // silently skipped where sudo needs a password).
        cmd_ok("timeout", &["5", "docker", "images", "--format", "{{.Repository}}:{{.Tag}}\t{{.Size}}\t{{.ID}}"])
            .filter(|o| !o.trim().is_empty())
            .or_else(|| if sudo_n_ok() { cmd_ok("timeout", &["6", "sudo", "-n", "docker", "images", "--format", "{{.Repository}}:{{.Tag}}\t{{.Size}}\t{{.ID}}"]) } else { None })
    } else { None };
    let _ = docker_args;
    let docker_images = docker_out
            .map(|o| o.lines().filter_map(|l| {
                let mut it = l.split('\t');
                let rt = it.next()?.to_string();
                if rt.is_empty() { return None; }
                Some(DockerImage { repository_tag: rt,
                    size: it.next().map(|s| s.to_string()), id: it.next().map(|s| s.to_string()) })
            }).collect()).unwrap_or_default();
    SoftwareInfo {
        explicit_packages, last_upgrade, snaps, docker_images, docker_status,
        glibc: cmd("ldd", &["--version"]).and_then(|o| o.lines().next().map(|s| s.to_string())),
        gcc_runtime: cmd("ldconfig", &["-p"]).and_then(|o| o.lines().find(|l| l.contains("libstdc++")).map(|s| s.trim().to_string())),
        loaded_modules_count: Some(modules.len()),
        loaded_modules_sample: modules.iter().take(40).cloned().collect(),
        packages_count: pkg_count, package_manager: pm,
        flatpaks_count: cmd("flatpak", &["list"]).map(|o| o.lines().count()),
        flatpaks_list: if which("flatpak") {
            cmd("timeout", &["5", "flatpak", "list", "--columns=name,version"])
                .map(|o| o.lines().filter_map(|l| {
                    let mut it = l.split('\t');
                    let name = it.next()?.trim().to_string();
                    if name.is_empty() { return None; }
                    Some(NamedVersion { name, version: it.next().unwrap_or("").trim().to_string() })
                }).collect()).unwrap_or_default()
        } else { Vec::new() },
        running_services_count: cmd("systemctl", &["list-units", "--type=service", "--state=running", "--no-legend"])
            .map(|o| o.lines().count()),
        extra: BTreeMap::new(),
    }
}

// ---------- v0.3: Belarc-parity collectors ----------

pub fn collect_volumes() -> Vec<VolumeInfo> {
    let mut out = Vec::new();
    let keep = ["ext2", "ext3", "ext4", "btrfs", "xfs", "f2fs", "vfat", "exfat", "ntfs", "ntfs3"];
    let mut subvols: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    if let Ok(mounts) = fs::read_to_string("/proc/mounts") {
        for line in mounts.lines() {
            let f: Vec<&str> = line.split_whitespace().collect();
            if f.len() >= 4 {
                for opt in f[3].split(',') {
                    if let Some(sv) = opt.strip_prefix("subvol=") {
                        subvols.insert(f[1].replace("\\040", " "), sv.to_string());
                    }
                }
            }
        }
    }
    if let Some(text) = cmd("df", &["-P", "-T"]) {
        for line in text.lines().skip(1) {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() < 7 { continue; }
            let fs_type = parts[1].to_string();
            if !keep.contains(&fs_type.as_str()) { continue; }
            let mount = parts[6..].join(" ");
            if mount.starts_with("/var/lib/docker") || mount.starts_with("/snap") { continue; }
            let kb = |i: usize| parts.get(i).and_then(|s| s.parse::<u64>().ok()).map(|k| k * 1024);
            out.push(VolumeInfo { mount: mount.clone(), fs_type,
                device: Some(parts[0].to_string()),
                subvolume: subvols.get(&mount).cloned(),
                size_bytes: kb(2), used_bytes: kb(3), avail_bytes: kb(4),
                use_percent: parts.get(5).and_then(|s| s.trim_end_matches('%').parse::<u32>().ok()) });
        }
    }
    out.sort_by(|a, b| a.mount.cmp(&b.mount));
    out
}

pub fn collect_printers() -> Vec<PrinterInfo> {
    let mut out = Vec::new();
    let default = cmd("timeout", &["5", "lpstat", "-d"])
        .and_then(|s| s.rsplit(':').next().map(|x| x.trim().to_string()))
        .filter(|s| !s.is_empty() && !s.contains("no system default"));
    if let Some(text) = cmd("timeout", &["5", "lpstat", "-p"]) {
        for line in text.lines() {
            if !line.starts_with("printer ") { continue; }
            let name = line.split_whitespace().nth(1).unwrap_or("").to_string();
            if name.is_empty() { continue; }
            let state = if line.contains("is idle") { Some("idle".to_string()) }
                else if line.contains("now printing") { Some("printing".to_string()) }
                else if line.contains("disabled") { Some("disabled".to_string()) }
                else { None };
            out.push(PrinterInfo { is_default: default.as_deref() == Some(name.as_str()), name, state });
        }
    }
    out
}

pub fn collect_bluetooth() -> Vec<PairedDevice> {
    let mut out = Vec::new();
    if let Some(text) = cmd("timeout", &["5", "bluetoothctl", "devices", "Paired"]) {
        for line in text.lines() {
            let rest = line.strip_prefix("Device ").unwrap_or("");
            let mut it = rest.splitn(2, ' ');
            let addr = it.next().unwrap_or("").to_string();
            let name = it.next().unwrap_or("").trim().to_string();
            if !addr.is_empty() { out.push(PairedDevice { name, address: addr }); }
        }
    }
    out
}

pub fn collect_accounts() -> Vec<UserAccount> {
    let mut admins: std::collections::BTreeSet<String> = Default::default();
    if let Ok(groups) = fs::read_to_string("/etc/group") {
        for line in groups.lines() {
            let mut parts = line.split(':');
            let gname = parts.next().unwrap_or("");
            if gname == "wheel" || gname == "sudo" || gname == "adm" {
                if let Some(members) = parts.nth(2) {
                    for m in members.split(',') { if !m.trim().is_empty() { admins.insert(m.trim().to_string()); } }
                }
            }
        }
    }
    let mut last_map: std::collections::BTreeMap<String, String> = Default::default();
    if let Some(text) = cmd("last", &["-n", "300"]) {
        for line in text.lines() {
            let user = line.split_whitespace().next().unwrap_or("").to_string();
            if user.is_empty() || user == "reboot" || user == "wtmp" { continue; }
            last_map.entry(user).or_insert_with(|| line.trim().to_string());
        }
    }
    let mut out = Vec::new();
    if let Ok(passwd) = fs::read_to_string("/etc/passwd") {
        for line in passwd.lines() {
            let f: Vec<&str> = line.split(':').collect();
            if f.len() < 7 { continue; }
            let uid: u32 = f[2].parse().unwrap_or(0);
            if !(uid == 0 || (1000..=60000).contains(&uid)) { continue; }
            let name = f[0].to_string();
            out.push(UserAccount {
                admin: uid == 0 || admins.contains(&name),
                last_login: last_map.get(&name).cloned(),
                name, uid,
                home: Some(f[5].to_string()),
                shell: Some(f[6].to_string()),
            });
        }
    }
    out.sort_by_key(|a| a.uid);
    out
}

pub fn collect_security() -> SecurityInfo {
    let active = |svc: &str| cmd("systemctl", &["is-active", svc])
        .map(|s| s.trim() == "active");
    let mut firewall_service = None; let mut firewall_active = None;
    for svc in ["ufw", "firewalld", "nftables", "iptables"] {
        if let Some(a) = active(svc) {
            if a { firewall_service = Some(svc.to_string()); firewall_active = Some(true); break; }
            if firewall_active.is_none() { firewall_active = Some(false); }
        }
    }
    let mut luks = 0usize;
    if let Some(text) = cmd("lsblk", &["-J", "-o", "FSTYPE"]) {
        luks = text.matches("crypto_LUKS").count();
    }
    SecurityInfo { firewall_active, firewall_service, luks_volumes: luks,
        sshd_active: active("sshd").or_else(|| active("ssh")) }
}


#[cfg(test)]
mod parser_tests {
    use super::*;

    #[test]
    fn pkg_lines_parse_all_distro_shapes() {
        let dpkg = "bash 5.1-6ubuntu1\ncoreutils 9.4-3ubuntu6\nlonely\n";
        let v = parse_pkg_lines(dpkg);
        assert_eq!(v.len(), 3);
        assert_eq!((v[0].name.as_str(), v[0].version.as_str()), ("bash", "5.1-6ubuntu1"));
        assert_eq!(v[2].version, "");
        let rpm = "bash 5.1.8-6.el9\n";
        assert_eq!(parse_pkg_lines(rpm)[0].version, "5.1.8-6.el9");
    }

    #[test]
    fn dpkg_log_last_upgrade_takes_newest_upgrade_line() {
        let log = "2026-09-01 10:00:00 startup archives install\n\
                   2026-09-02 11:00:00 upgrade bash:amd64 5.1 5.2\n\
                   2026-09-03 09:00:00 install curl:amd64 <none> 8.0\n\
                   2026-09-04 08:30:00 upgrade curl:amd64 8.0 8.1\n";
        assert_eq!(parse_dpkg_log_last_upgrade(log).as_deref(), Some("2026-09-04 08:30:00"));
        assert_eq!(parse_dpkg_log_last_upgrade("2026-09-01 10:00:00 startup archives install\n"), None);
    }

    #[test]
    fn pacman_log_last_upgrade_still_parses() {
        let log = "[2026-09-01T10:00:00+0000] [ALPM] upgraded bash (5.1 -> 5.2)\n[2026-09-02T10:00:00+0000] [ALPM] installed curl (8.0)\n";
        assert_eq!(parse_pacman_log_last_upgrade(log).as_deref(), Some("2026-09-01T10:00:00+0000"));
    }

    #[test]
    fn docker_unreachable_state_ladder() {
        assert_eq!(docker_unreachable_state(false, false, false, false), "not-running");
        assert_eq!(docker_unreachable_state(true, false, false, false), "inaccessible");
        assert_eq!(docker_unreachable_state(false, true, false, false), "inaccessible");
        assert_eq!(docker_unreachable_state(false, false, true, false), "inaccessible"); // rootless socket
        assert_eq!(docker_unreachable_state(false, false, false, true), "inaccessible");  // DOCKER_HOST set
    }

    #[test]
    fn dmidecode_memory_parses_populated_slot_only() {
        let dump = "Memory Device\n\tSize: 16 GB\n\tLocator: Controller0-ChannelA-DIMM0\n\tType: DDR4\n\tSpeed: 3200 MT/s\n\tManufacturer: Lexar Co Limited\n\tPart Number: LD4AS016G-3200ST\n\nMemory Device\n\tSize: No Module Installed\n\tLocator: Controller0-ChannelB-DIMM0\n\tType: Unknown\n";
        let mods = parse_dmidecode_memory(dump);
        assert_eq!(mods.len(), 1);
        assert_eq!(mods[0].locator, "Controller0-ChannelA-DIMM0");
        assert_eq!(mods[0].size.as_deref(), Some("16 GB"));
        assert_eq!(mods[0].part_number.as_deref(), Some("LD4AS016G-3200ST"));
    }

    #[test]
    fn dmidecode_serial_skips_oem_placeholders() {
        let good = "System Information\n\tManufacturer: ACME\n\tSerial Number: SN-12345\n";
        assert_eq!(parse_dmidecode_serial(good).as_deref(), Some("SN-12345"));
        let junk = "System Information\n\tSerial Number: To be filled by O.E.M.\n";
        assert_eq!(parse_dmidecode_serial(junk), None);
        let junk2 = "System Information\n\tSerial Number: Default string\n";
        assert_eq!(parse_dmidecode_serial(junk2), None);
    }

    #[test]
    fn edid_preferred_timing_gives_refresh_and_name() {
        let mut e = [0u8; 128];
        e[..8].copy_from_slice(&[0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00]);
        e[8] = 0x10; e[9] = 0xAC; // DEL
        e[17] = 34; // 2024
        e[21] = 52; e[22] = 29;
        // detailed timing @54: 148.5 MHz, 1920x1080 active, 2200x1125 total = 60 Hz
        e[54] = 0x02; e[55] = 0x3A;
        e[56] = 0x80; e[57] = 0x18; e[58] = 0x71; e[59] = 0x38; e[60] = 0x2D; e[61] = 0x40;
        // name descriptor @72
        e[72] = 0; e[73] = 0; e[74] = 0; e[75] = 0xFC; e[76] = 0;
        e[77..90].copy_from_slice(b"DELL U4021QW\n");
        let d = parse_edid(&e, "HDMI-A-1");
        assert_eq!(d.manufacturer.as_deref(), Some("DEL"));
        assert_eq!(d.name.as_deref(), Some("DELL U4021QW"));
        assert_eq!(d.refresh_hz, Some(60));
        assert_eq!(d.manufacture_year, Some(2024));
    }

    #[test]
    fn sata_wear_from_smart_attributes() {
        let v231 = serde_json::json!({"ata_smart_attributes": {"table": [
            {"id": 231, "raw": {"value": 90}}]}});
        assert_eq!(sata_wear_percent(&v231), Some(10));
        let v233 = serde_json::json!({"ata_smart_attributes": {"table": [
            {"id": 233, "value": 97, "raw": {"value": 0}}]}});
        assert_eq!(sata_wear_percent(&v233), Some(3));
        let empty = serde_json::json!({"ata_smart_attributes": {"table": []}});
        assert_eq!(sata_wear_percent(&empty), None);
    }

    #[test]
    fn iface_classification_names() {
        let base = "/definitely/not/a/real/path";
        assert_eq!(classify_iface("docker0", base).0, "Bridge");
        assert_eq!(classify_iface("veth1234", base).0, "Virtual Ethernet");
        assert_eq!(classify_iface("wg0", base).0, "VPN / Tunnel");
        assert_eq!(classify_iface("tailscale0", base).0, "VPN / Tunnel");
    }
}
