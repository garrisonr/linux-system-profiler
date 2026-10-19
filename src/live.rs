//! Live sampling — the v0.9.0 heart.
//!
//! Everything here reads /proc, /sys and systemctl directly; nothing is
//! cached from the static profile and nothing is invented: a sensor the
//! machine doesn't expose comes back as an empty list / None and the UI
//! says so in plain words. The sampler owns the interval math (deltas
//! between successive reads), the history ring buffer, and the
//! pid+start-time identity rule for process actions: a recycled PID must
//! refuse, never signal the wrong process.

use serde::Serialize;
use std::collections::{HashMap, VecDeque};
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

fn now_unix() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

fn read(path: &str) -> Option<String> {
    fs::read_to_string(path).ok()
}

fn clk_tck() -> f64 {
    static TCK: std::sync::OnceLock<f64> = std::sync::OnceLock::new();
    *TCK.get_or_init(|| {
        let v = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
        if v > 0 { v as f64 } else { 100.0 }
    })
}

fn page_kb() -> u64 {
    static PG: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    *PG.get_or_init(|| {
        let v = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
        if v > 0 { (v / 1024) as u64 } else { 4 }
    })
}

fn usernames() -> &'static HashMap<u32, String> {
    static MAP: std::sync::OnceLock<HashMap<u32, String>> = std::sync::OnceLock::new();
    MAP.get_or_init(|| {
        let mut m = HashMap::new();
        if let Some(text) = read("/etc/passwd") {
            for line in text.lines() {
                let mut it = line.split(':');
                if let (Some(name), _, Some(uid)) = (it.next(), it.next(), it.next()) {
                    if let Ok(uid) = uid.parse::<u32>() { m.insert(uid, name.to_string()); }
                }
            }
        }
        m
    })
}

// ---------------------------------------------------------------- /proc/stat

#[derive(Clone, Default)]
struct CpuTimes { total: u64, idle: u64 }

fn parse_stat_cpu() -> Option<(CpuTimes, Vec<CpuTimes>)> {
    let text = read("/proc/stat")?;
    let mut total = None;
    let mut cores = Vec::new();
    for line in text.lines() {
        let mut parts = line.split_whitespace();
        let name = parts.next()?;
        let nums: Vec<u64> = parts.map(|p| p.parse().unwrap_or(0)).collect();
        if nums.len() < 5 { continue; }
        let t = CpuTimes { total: nums.iter().sum(), idle: nums[3] + nums.get(4).copied().unwrap_or(0) };
        if name == "cpu" { total = Some(t); }
        else if name.starts_with("cpu") && name[3..].chars().all(|c| c.is_ascii_digit()) { cores.push(t); }
        else if !name.starts_with("cpu") { break; }
    }
    Some((total?, cores))
}

// ---------------------------------------------------------------- processes

/// Raw per-process counters, before interval math.
#[derive(Clone)]
struct ProcRaw {
    pid: u32, ppid: u32, comm: String, state: char,
    jiffies: u64, rss_kb: u64, start_ticks: u64,
    uid: Option<u32>, read_bytes: Option<u64>, write_bytes: Option<u64>,
    cmdline: String,
}

/// Parse /proc/<pid>/stat. comm sits in parens and may itself contain
/// spaces or parens, so split at the LAST ')'. After comm, split-fields
/// are stat fields 3.. — index 0 = state (3), 1 = ppid (4),
/// 11 = utime (14), 12 = stime (15), 19 = starttime (22).
fn parse_proc_stat(text: &str) -> Option<(String, Vec<&str>)> {
    let open = text.find('(')?;
    let close = text.rfind(')')?;
    let comm = text[open + 1..close].to_string();
    let rest: Vec<&str> = text[close + 2..].split_whitespace().collect();
    if rest.len() < 20 { return None; }
    Some((comm, rest))
}

fn read_proc(pid: u32) -> Option<ProcRaw> {
    let base = format!("/proc/{pid}");
    let stat_text = read(&format!("{base}/stat"))?;
    let (comm, f) = parse_proc_stat(&stat_text)?;
    let state = f[0].chars().next().unwrap_or('?');
    let ppid: u32 = f[1].parse().unwrap_or(0);
    let utime: u64 = f[11].parse().unwrap_or(0);
    let stime: u64 = f[12].parse().unwrap_or(0);
    let start_ticks: u64 = f[19].parse().unwrap_or(0);
    // RSS + uid from status (VmRSS is in kB there; stat's rss field is pages).
    let mut rss_kb = 0u64;
    let mut uid = None;
    if let Some(status) = read(&format!("{base}/status")) {
        for line in status.lines() {
            if let Some(v) = line.strip_prefix("VmRSS:") {
                rss_kb = v.trim().trim_end_matches("kB").trim().parse().unwrap_or(0);
            } else if let Some(v) = line.strip_prefix("Uid:") {
                uid = v.split_whitespace().next().and_then(|s| s.parse().ok());
            }
        }
    }
    // IO counters: often unreadable for other users' processes — None is
    // the honest answer there, never a fabricated zero.
    let (mut rb, mut wb) = (None, None);
    if let Some(io) = read(&format!("{base}/io")) {
        for line in io.lines() {
            if let Some(v) = line.strip_prefix("read_bytes:") { rb = v.trim().parse().ok(); }
            if let Some(v) = line.strip_prefix("write_bytes:") { wb = v.trim().parse().ok(); }
        }
    }
    let cmdline = read(&format!("{base}/cmdline"))
        .map(|c| {
            let s: String = c.split('\0').filter(|p| !p.is_empty()).collect::<Vec<_>>().join(" ");
            let s = if s.is_empty() { comm.clone() } else { s };
            s.chars().take(160).collect()
        })
        .unwrap_or_else(|| comm.clone());
    Some(ProcRaw { pid, ppid, comm, state, jiffies: utime + stime, rss_kb, start_ticks, uid, read_bytes: rb, write_bytes: wb, cmdline })
}

fn list_pids() -> Vec<u32> {
    let mut out = Vec::new();
    if let Ok(rd) = fs::read_dir("/proc") {
        for e in rd.flatten() {
            if let Ok(pid) = e.file_name().to_string_lossy().parse::<u32>() { out.push(pid); }
        }
    }
    out
}

/// The process start-time identity, re-read at the moment of acting.
pub fn proc_start_ticks(pid: u32) -> Option<u64> {
    let text = read(&format!("/proc/{pid}/stat"))?;
    let (_, f) = parse_proc_stat(&text)?;
    f[19].parse().ok()
}

/// Signal a process ONLY if it is still the process the user pointed at:
/// (pid, start_ticks) are re-verified against /proc at signal time. A
/// recycled PID refuses with a plain sentence. TMOG's identity rule, ours
/// now too.
pub fn signal_process(pid: u32, start_ticks: u64, force: bool) -> Result<String, String> {
    let name = read_proc(pid).map(|p| p.comm).unwrap_or_else(|| format!("process {pid}"));
    match proc_start_ticks(pid) {
        None => Err(format!("{name} (PID {pid}) is already gone — nothing to end.")),
        Some(actual) if actual != start_ticks => Err(format!(
            "PID {pid} isn't the process you picked anymore — that one already ended and the number was reused. Nothing was signalled.")),
        Some(_) => {
            let sig = if force { libc::SIGKILL } else { libc::SIGTERM };
            let rc = unsafe { libc::kill(pid as libc::pid_t, sig) };
            if rc == 0 {
                Ok(if force {
                    format!("Force-ended {name} (PID {pid}).")
                } else {
                    format!("Asked {name} (PID {pid}) to quit.")
                })
            } else {
                let e = std::io::Error::last_os_error();
                Err(if e.raw_os_error() == Some(libc::EPERM) {
                    format!("The system wouldn't let Profiler end {name} (PID {pid}) — it belongs to another user or to the system itself.")
                } else {
                    format!("Couldn't end {name} (PID {pid}): {e}")
                })
            }
        }
    }
}

// ---------------------------------------------------------------- samples

#[derive(Serialize, Clone)]
pub struct CoreSample {
    pub pct: f64,
    pub freq_mhz: Option<u32>,
    /// "performance" / "efficiency" ONLY where the machine exposes a real
    /// capacity split; None everywhere homogeneous (N95, the Xeon).
    pub kind: Option<String>,
}

#[derive(Serialize, Clone)]
pub struct ProcInfo {
    pub pid: u32,
    pub ppid: u32,
    pub name: String,
    pub cmdline: String,
    pub user: String,
    pub state: String,
    /// Percent of TOTAL cpu capacity (all cores = 100).
    pub cpu_pct: f64,
    pub rss_kb: u64,
    pub read_bps: Option<u64>,
    pub write_bps: Option<u64>,
    pub start_ticks: u64,
}

#[derive(Serialize, Clone)]
pub struct SysSample {
    pub ts: f64,
    pub cpu_pct: f64,
    pub cores: Vec<CoreSample>,
    pub mem_total_kb: u64,
    pub mem_used_kb: u64,
    pub swap_total_kb: u64,
    pub swap_used_kb: u64,
    pub disk_read_bps: u64,
    pub disk_write_bps: u64,
    pub net_rx_bps: u64,
    pub net_tx_bps: u64,
    pub load1: f64,
    pub uptime_s: u64,
    /// (label, °C) from hwmon — empty when the machine exposes nothing.
    pub temps: Vec<(String, f64)>,
    pub power_w: Option<f64>,
    pub procs: Vec<ProcInfo>,
    pub self_pid: u32,
    /// What the PREVIOUS sample cost to collect, milliseconds.
    pub sampler_ms: f64,
}

#[derive(Serialize, Clone)]
pub struct TopProc { pub pid: u32, pub name: String, pub cpu_pct: f64, pub rss_kb: u64 }

#[derive(Serialize, Clone)]
pub struct HistoryPoint {
    pub ts: f64,
    pub cpu_pct: f64,
    pub mem_pct: f64,
    pub disk_read_bps: u64,
    pub disk_write_bps: u64,
    pub net_rx_bps: u64,
    pub net_tx_bps: u64,
    pub top: Vec<TopProc>,
}

/// ~30 minutes at the fast-lane 1 s rate. Bounded, session-scoped.
pub const HISTORY_CAP: usize = 1800;

#[derive(Default)]
pub struct History {
    points: VecDeque<HistoryPoint>,
}

impl History {
    pub fn push(&mut self, s: &SysSample) {
        let mut ranked: Vec<&ProcInfo> = s.procs.iter().collect();
        ranked.sort_by(|a, b| b.cpu_pct.partial_cmp(&a.cpu_pct).unwrap_or(std::cmp::Ordering::Equal));
        let top = ranked.iter().take(10)
            .map(|p| TopProc { pid: p.pid, name: p.name.clone(), cpu_pct: p.cpu_pct, rss_kb: p.rss_kb })
            .collect();
        self.points.push_back(HistoryPoint {
            ts: s.ts, cpu_pct: s.cpu_pct,
            mem_pct: if s.mem_total_kb > 0 { s.mem_used_kb as f64 / s.mem_total_kb as f64 * 100.0 } else { 0.0 },
            disk_read_bps: s.disk_read_bps, disk_write_bps: s.disk_write_bps,
            net_rx_bps: s.net_rx_bps, net_tx_bps: s.net_tx_bps, top,
        });
        while self.points.len() > HISTORY_CAP { self.points.pop_front(); }
    }
    pub fn snapshot(&self) -> Vec<HistoryPoint> { self.points.iter().cloned().collect() }
    pub fn len(&self) -> usize { self.points.len() }
}

// ---------------------------------------------------------------- sampler

struct Prev {
    cpu: CpuTimes,
    cores: Vec<CpuTimes>,
    procs: HashMap<u32, (u64, Option<u64>, Option<u64>, u64)>, // pid → (jiffies, read, write, start)
    disk: (u64, u64),
    net: (u64, u64),
    rapl_uj: Option<u64>,
    at: std::time::Instant,
}

pub struct Sampler {
    prev: Option<Prev>,
    last_cost_ms: f64,
}

impl Default for Sampler {
    fn default() -> Self { Self::new() }
}

impl Sampler {
    pub fn new() -> Self { Sampler { prev: None, last_cost_ms: 0.0 } }

    pub fn sample(&mut self) -> SysSample {
        let started = std::time::Instant::now();
        let out = self.sample_inner();
        self.last_cost_ms = started.elapsed().as_secs_f64() * 1000.0;
        out
    }

    fn sample_inner(&mut self) -> SysSample {
        let ts = now_unix();
        let (cpu_now, cores_now) = parse_stat_cpu().unwrap_or((CpuTimes::default(), Vec::new()));
        // --- memory / swap
        let (mut mem_total, mut mem_avail, mut swap_total, mut swap_free) = (0u64, 0u64, 0u64, 0u64);
        if let Some(mi) = read("/proc/meminfo") {
            for line in mi.lines() {
                let mut it = line.split_whitespace();
                let (k, v) = (it.next().unwrap_or(""), it.next().and_then(|s| s.parse::<u64>().ok()).unwrap_or(0));
                match k { "MemTotal:" => mem_total = v, "MemAvailable:" => mem_avail = v,
                          "SwapTotal:" => swap_total = v, "SwapFree:" => swap_free = v, _ => {} }
            }
        }
        // --- disk + net totals
        let disk_now = disk_totals();
        let net_now = net_totals();
        // --- processes
        let raws: Vec<ProcRaw> = list_pids().into_iter().filter_map(read_proc).collect();
        // --- core kinds + freq
        let kinds = core_kinds(cores_now.len());
        let cores: Vec<CoreSample> = cores_now.iter().enumerate().map(|(i, c)| {
            let pct = match &self.prev {
                Some(p) if i < p.cores.len() => pct_between(&p.cores[i], c),
                _ => 0.0,
            };
            CoreSample { pct, freq_mhz: core_freq_mhz(i), kind: kinds.get(i).cloned().unwrap_or(None) }
        }).collect();
        let cpu_pct = match &self.prev {
            Some(p) => pct_between(&p.cpu, &cpu_now),
            None => 0.0,
        };
        // --- rates + per-process math need the previous snapshot
        let (mut disk_r, mut disk_w, mut net_r, mut net_t) = (0u64, 0u64, 0u64, 0u64);
        let mut power_w = None;
        let procs: Vec<ProcInfo> = match &self.prev {
            Some(p) => {
                let dt = p.at.elapsed().as_secs_f64().max(0.001);
                disk_r = (disk_now.0.saturating_sub(p.disk.0) as f64 / dt) as u64;
                disk_w = (disk_now.1.saturating_sub(p.disk.1) as f64 / dt) as u64;
                net_r = (net_now.0.saturating_sub(p.net.0) as f64 / dt) as u64;
                net_t = (net_now.1.saturating_sub(p.net.1) as f64 / dt) as u64;
                power_w = rapl_watts(p.rapl_uj, dt);
                let total_delta = cpu_now.total.saturating_sub(p.cpu.total) as f64;
                raws.iter().map(|r| {
                    let (cpu_pct, rb, wb) = match p.procs.get(&r.pid) {
                        Some((pj, pr, pw, pstart)) if *pstart == r.start_ticks => {
                            let pct = if total_delta > 0.0 {
                                (r.jiffies.saturating_sub(*pj)) as f64 / total_delta * 100.0
                            } else { 0.0 };
                            let rate = |now: &Option<u64>, was: &Option<u64>| -> Option<u64> {
                                match (now, was) { (Some(n), Some(w)) => Some((n.saturating_sub(*w) as f64 / dt) as u64), _ => None }
                            };
                            (pct, rate(&r.read_bytes, pr), rate(&r.write_bytes, pw))
                        }
                        _ => (0.0, None, None), // new process: rates unknown until next sample — honest blank
                    };
                    proc_info(r, cpu_pct, rb, wb)
                }).collect()
            }
            None => raws.iter().map(|r| proc_info(r, 0.0, None, None)).collect(),
        };
        let load1 = read("/proc/loadavg").and_then(|s| s.split_whitespace().next()?.parse().ok()).unwrap_or(0.0);
        let uptime_s = read("/proc/uptime").and_then(|s| s.split_whitespace().next()?.parse::<f64>().ok()).unwrap_or(0.0) as u64;
        let sample = SysSample {
            ts, cpu_pct, cores,
            mem_total_kb: mem_total, mem_used_kb: mem_total.saturating_sub(mem_avail),
            swap_total_kb: swap_total, swap_used_kb: swap_total.saturating_sub(swap_free),
            disk_read_bps: disk_r, disk_write_bps: disk_w, net_rx_bps: net_r, net_tx_bps: net_t,
            load1, uptime_s, temps: read_temps(), power_w, procs,
            self_pid: std::process::id(), sampler_ms: self.last_cost_ms,
        };
        self.prev = Some(Prev {
            cpu: cpu_now.clone(), cores: cores_now,
            procs: raws.iter().map(|r| (r.pid, (r.jiffies, r.read_bytes, r.write_bytes, r.start_ticks))).collect(),
            disk: disk_now, net: net_now, rapl_uj: rapl_energy_uj(), at: std::time::Instant::now(),
        });
        sample
    }
}

fn proc_info(r: &ProcRaw, cpu_pct: f64, rb: Option<u64>, wb: Option<u64>) -> ProcInfo {
    ProcInfo {
        pid: r.pid, ppid: r.ppid, name: r.comm.clone(), cmdline: r.cmdline.clone(),
        user: r.uid.and_then(|u| usernames().get(&u).cloned()).unwrap_or_else(|| r.uid.map(|u| u.to_string()).unwrap_or_default()),
        state: r.state.to_string(), cpu_pct, rss_kb: r.rss_kb,
        read_bps: rb, write_bps: wb, start_ticks: r.start_ticks,
    }
}

fn pct_between(prev: &CpuTimes, now: &CpuTimes) -> f64 {
    let dt = now.total.saturating_sub(prev.total) as f64;
    if dt <= 0.0 { return 0.0; }
    let di = now.idle.saturating_sub(prev.idle) as f64;
    ((dt - di) / dt * 100.0).clamp(0.0, 100.0)
}

fn core_freq_mhz(i: usize) -> Option<u32> {
    let khz: u64 = read(&format!("/sys/devices/system/cpu/cpu{i}/cpufreq/scaling_cur_freq"))?.trim().parse().ok()?;
    Some((khz / 1000) as u32)
}

/// P/E labels only when cpu_capacity shows a genuine split (a hybrid
/// part). Homogeneous chips expose one capacity (or none) → all None.
fn core_kinds(n: usize) -> Vec<Option<String>> {
    let caps: Vec<Option<u64>> = (0..n).map(|i| {
        read(&format!("/sys/devices/system/cpu/cpu{i}/cpu_capacity")).and_then(|s| s.trim().parse().ok())
    }).collect();
    let known: Vec<u64> = caps.iter().flatten().copied().collect();
    if known.len() != n || n == 0 { return vec![None; n]; }
    let max = *known.iter().max().unwrap_or(&0);
    let min = *known.iter().min().unwrap_or(&0);
    if max == 0 || min as f64 >= max as f64 * 0.8 { return vec![None; n]; }
    caps.iter().map(|c| match c {
        Some(v) if *v == max => Some("performance".to_string()),
        Some(_) => Some("efficiency".to_string()),
        None => None,
    }).collect()
}

fn whole_disk(name: &str) -> bool {
    let b = name.as_bytes();
    if name.starts_with("nvme") {
        // nvme0n1 = disk, nvme0n1p2 = partition (the pN suffix is the tell)
        let after_n = name.rfind('n').map(|i| &name[i + 1..]).unwrap_or("");
        return !after_n.contains('p');
    }
    if name.starts_with("mmcblk") { return !name.contains('p'); }
    // sdX / vdX / xvdX: disk name ends in a letter, partitions end in a digit
    (name.starts_with("sd") || name.starts_with("vd") || name.starts_with("xvd"))
        && b.last().map(|c| c.is_ascii_alphabetic()).unwrap_or(false)
}

fn disk_totals() -> (u64, u64) {
    let (mut r, mut w) = (0u64, 0u64);
    if let Some(text) = read("/proc/diskstats") {
        for line in text.lines() {
            let f: Vec<&str> = line.split_whitespace().collect();
            if f.len() < 14 { continue; }
            let name = f[2];
            if !whole_disk(name) { continue; }
            // fields: 5 = sectors read, 9 = sectors written (512 B sectors)
            r += f[5].parse::<u64>().unwrap_or(0) * 512;
            w += f[9].parse::<u64>().unwrap_or(0) * 512;
        }
    }
    (r, w)
}

fn net_totals() -> (u64, u64) {
    let (mut r, mut t) = (0u64, 0u64);
    if let Some(text) = read("/proc/net/dev") {
        for line in text.lines().skip(2) {
            let Some((iface, rest)) = line.split_once(':') else { continue };
            if iface.trim() == "lo" { continue; }
            let f: Vec<&str> = rest.split_whitespace().collect();
            if f.len() < 16 { continue; }
            r += f[0].parse::<u64>().unwrap_or(0);
            t += f[8].parse::<u64>().unwrap_or(0);
        }
    }
    (r, t)
}

fn read_temps() -> Vec<(String, f64)> {
    let mut out = Vec::new();
    let Ok(hwmons) = fs::read_dir("/sys/class/hwmon") else { return out };
    for hw in hwmons.flatten() {
        let dir = hw.path();
        let chip = fs::read_to_string(dir.join("name")).map(|s| s.trim().to_string()).unwrap_or_default();
        let Ok(entries) = fs::read_dir(&dir) else { continue };
        for e in entries.flatten() {
            let fname = e.file_name().to_string_lossy().to_string();
            if !fname.starts_with("temp") || !fname.ends_with("_input") { continue; }
            let Ok(raw) = fs::read_to_string(e.path()) else { continue };
            let Ok(milli) = raw.trim().parse::<f64>() else { continue };
            let stem = fname.trim_end_matches("_input");
            let label = fs::read_to_string(dir.join(format!("{stem}_label")))
                .map(|s| s.trim().to_string())
                .unwrap_or_else(|_| if chip.is_empty() { stem.to_string() } else { format!("{chip} {stem}") });
            out.push((label, milli / 1000.0));
            if out.len() >= 12 { return out; }
        }
    }
    out
}

fn rapl_energy_uj() -> Option<u64> {
    // intel-rapl:0 = package domain on most machines; AMD exposes the same
    // powercap tree on newer kernels. Missing tree → None, honestly.
    for i in 0..4 {
        if let Some(s) = read(&format!("/sys/class/powercap/intel-rapl:{i}/energy_uj")) {
            if let Ok(v) = s.trim().parse::<u64>() { return Some(v); }
        }
    }
    None
}

fn rapl_watts(prev_uj: Option<u64>, dt: f64) -> Option<f64> {
    let now = rapl_energy_uj()?;
    let was = prev_uj?;
    if dt <= 0.0 { return None; }
    let delta = now.checked_sub(was)? as f64; // counter wrap → skip this sample
    Some(delta / 1_000_000.0 / dt)
}

// ---------------------------------------------------------------- services

#[derive(Serialize, Clone)]
pub struct ServiceInfo {
    pub name: String,
    pub load: String,
    pub active: String,
    pub sub: String,
    pub description: String,
}

/// Parse `systemctl list-units --type=service --all --plain --no-legend`
/// output: UNIT LOAD ACTIVE SUB DESCRIPTION… (description may have spaces).
pub fn parse_services(text: &str) -> Vec<ServiceInfo> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim_start_matches(['●', ' ']);
        let mut it = line.split_whitespace();
        let (Some(name), Some(load), Some(active), Some(sub)) = (it.next(), it.next(), it.next(), it.next()) else { continue };
        if !name.ends_with(".service") { continue; }
        let description: String = it.collect::<Vec<_>>().join(" ");
        out.push(ServiceInfo {
            name: name.to_string(), load: load.to_string(),
            active: active.to_string(), sub: sub.to_string(), description,
        });
    }
    out
}

pub fn list_services() -> Result<Vec<ServiceInfo>, String> {
    let out = std::process::Command::new("systemctl")
        .args(["list-units", "--type=service", "--all", "--plain", "--no-legend", "--no-pager"])
        .output()
        .map_err(|e| format!("Couldn't read the service list: {e}"))?;
    if !out.status.success() {
        return Err("systemctl couldn't list services on this machine.".into());
    }
    Ok(parse_services(&String::from_utf8_lossy(&out.stdout)))
}

/// A unit name is safe to hand to systemctl only if it looks like a unit
/// name from the live list — no shell metacharacters, no paths, no flags.
pub fn valid_unit_name(name: &str) -> bool {
    !name.is_empty() && name.len() < 256 && name.ends_with(".service")
        && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '@' | ':' | '.' | '_' | '-' | '\\'))
}

// ---------------------------------------------------------------- startup

#[derive(Serialize, Clone)]
pub struct StartupEntry {
    pub name: String,
    pub exec: String,
    /// Human source label: whose folder / which mechanism owns it.
    pub source: String,
    pub kind: String, // "Application" | "User service"
    pub hidden: bool,
}

fn parse_desktop_entry(text: &str) -> (Option<String>, Option<String>, bool) {
    let (mut name, mut exec, mut hidden) = (None, None, false);
    let mut in_entry = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') { in_entry = line == "[Desktop Entry]"; continue; }
        if !in_entry { continue; }
        if let Some(v) = line.strip_prefix("Name=") { if name.is_none() { name = Some(v.to_string()); } }
        else if let Some(v) = line.strip_prefix("Exec=") { if exec.is_none() { exec = Some(v.to_string()); } }
        else if line == "Hidden=true" || line == "X-GNOME-Autostart-enabled=false" { hidden = true; }
    }
    (name, exec, hidden)
}

pub fn list_startup() -> Vec<StartupEntry> {
    let mut out: Vec<StartupEntry> = Vec::new();
    let mut seen_files: std::collections::HashSet<String> = std::collections::HashSet::new();
    let home = std::env::var("HOME").unwrap_or_default();
    // User folder first: a same-named user file shadows the system one.
    let mut dirs: Vec<(String, String)> = vec![
        (format!("{home}/.config/autostart"), "Your autostart folder".to_string()),
    ];
    let xdg_dirs = std::env::var("XDG_CONFIG_DIRS").unwrap_or_else(|_| "/etc/xdg".to_string());
    for d in xdg_dirs.split(':').filter(|s| !s.is_empty()) {
        dirs.push((format!("{d}/autostart"), "System autostart".to_string()));
    }
    for (dir, source) in &dirs {
        if let Ok(rd) = fs::read_dir(dir) {
            for e in rd.flatten() {
                let fname = e.file_name().to_string_lossy().to_string();
                if !fname.ends_with(".desktop") || !seen_files.insert(fname) { continue; }
                let Ok(text) = fs::read_to_string(e.path()) else { continue };
                let (name, exec, hidden) = parse_desktop_entry(&text);
                if let Some(name) = name {
                    out.push(StartupEntry {
                        name, exec: exec.unwrap_or_default(), source: source.clone(),
                        kind: "Application".to_string(), hidden,
                    });
                }
            }
        }
    }
    // Enabled systemd user services.
    let units = std::process::Command::new("systemctl")
        .args(["--user", "list-unit-files", "--type=service", "--state=enabled", "--no-legend", "--no-pager"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default();
    for line in units.lines() {
        if let Some(name) = line.split_whitespace().next() {
            if name.ends_with(".service") {
                out.push(StartupEntry {
                    name: name.trim_end_matches(".service").to_string(),
                    exec: name.to_string(), source: "systemd, your account".to_string(),
                    kind: "User service".to_string(), hidden: false,
                });
            }
        }
    }
    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    out
}

// ---------------------------------------------------------------- tests

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stat_parse_handles_parens_in_comm() {
        // comm contains a space AND a paren — the last-')' split must win.
        let line = "4242 (weird) name) S 1 4242 4242 0 -1 4194304 100 0 0 0 30 10 0 0 20 0 1 0 987654 5000000 200";
        let (comm, f) = parse_proc_stat(line).unwrap();
        assert_eq!(comm, "weird) name");
        assert_eq!(f[0], "S");
        assert_eq!(f[1], "1");
        assert_eq!(f[19], "987654");
    }

    #[test]
    fn identity_refuses_recycled_pid() {
        // A pid that certainly doesn't exist: gone, not signalled.
        let err = signal_process(999_999, 1, false).unwrap_err();
        assert!(err.contains("already gone"), "{err}");
        // Our own pid with a WRONG start time: recycled-pid refusal, no signal sent.
        let me = std::process::id();
        let real = proc_start_ticks(me).unwrap();
        let err = signal_process(me, real + 1, false).unwrap_err();
        assert!(err.contains("isn't the process you picked"), "{err}");
    }

    #[test]
    fn history_is_bounded() {
        let mut h = History::default();
        let s = SysSample {
            ts: 0.0, cpu_pct: 1.0, cores: vec![], mem_total_kb: 100, mem_used_kb: 40,
            swap_total_kb: 0, swap_used_kb: 0, disk_read_bps: 0, disk_write_bps: 0,
            net_rx_bps: 0, net_tx_bps: 0, load1: 0.0, uptime_s: 0, temps: vec![],
            power_w: None, procs: vec![], self_pid: 1, sampler_ms: 0.0,
        };
        for _ in 0..(HISTORY_CAP + 50) { h.push(&s); }
        assert_eq!(h.len(), HISTORY_CAP);
    }

    #[test]
    fn services_parse() {
        let text = "cron.service loaded active running Regular background program processing daemon\nfoo.service loaded failed failed Some failed thing\n  ● bar.service loaded active exited Done thing\n";
        let v = parse_services(text);
        assert_eq!(v.len(), 3);
        assert_eq!(v[0].name, "cron.service");
        assert_eq!(v[0].description, "Regular background program processing daemon");
        assert!(valid_unit_name("cron.service"));
        assert!(!valid_unit_name("cron.service; rm -rf /"));
        assert!(!valid_unit_name("--all"));
    }

    #[test]
    fn desktop_entry_parse() {
        let text = "[Desktop Entry]\nName=My App\nExec=/usr/bin/myapp --flag\nHidden=false\n[Other]\nName=Nope\n";
        let (n, e, h) = parse_desktop_entry(text);
        assert_eq!(n.as_deref(), Some("My App"));
        assert_eq!(e.as_deref(), Some("/usr/bin/myapp --flag"));
        assert!(!h);
        let (_, _, h2) = parse_desktop_entry("[Desktop Entry]\nName=X\nHidden=true\n");
        assert!(h2);
    }

    #[test]
    fn whole_disk_names() {
        assert!(whole_disk("sda"));
        assert!(!whole_disk("sda1"));
        assert!(whole_disk("nvme0n1"));
        assert!(!whole_disk("nvme0n1p2"));
        assert!(whole_disk("mmcblk0"));
        assert!(!whole_disk("mmcblk0p1"));
        assert!(!whole_disk("loop0"));
    }
}
