//! On-demand speed tests — NEVER run during profile collection.
//! Disk: sequential write/read with O_DIRECT so the page cache can't flatter
//! the numbers. Network: Cloudflare's public speed-test endpoints
//! (speed.cloudflare.com), latency + download + upload.

use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DiskSpeed {
    pub path: String,
    pub size_mb: u64,
    pub write_mbps: f64,
    pub read_mbps: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct NetSpeed {
    pub server: String,
    pub latency_ms: f64,
    pub download_mbps: f64,
    pub upload_mbps: f64,
}

fn free_bytes(dir: &Path) -> Option<u64> {
    let c = std::ffi::CString::new(dir.to_string_lossy().as_bytes()).ok()?;
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c.as_ptr(), &mut st) } != 0 { return None; }
    Some(st.f_bavail as u64 * st.f_frsize as u64)
}

struct TempGuard(PathBuf);
impl Drop for TempGuard { fn drop(&mut self) { let _ = std::fs::remove_file(&self.0); } }

pub fn disk_speed_test(dir: &Path, size_mb: u64) -> Result<DiskSpeed, String> {
    disk_speed_test_p(dir, size_mb, |_, _| {})
}

pub fn disk_speed_test_p(dir: &Path, size_mb: u64, mut progress: impl FnMut(&str, f64)) -> Result<DiskSpeed, String> {
    use std::os::unix::fs::OpenOptionsExt;
    let size = size_mb.clamp(64, 4096);
    let total = size * 1024 * 1024;
    if let Some(free) = free_bytes(dir) {
        if free < total + 512 * 1024 * 1024 {
            return Err(format!("not enough free space on {} (need test file + 512 MiB margin)", dir.display()));
        }
    }
    let path = dir.join(format!(".lsp-speedtest-{}.tmp", std::process::id()));
    let _guard = TempGuard(path.clone());
    let chunk = 1024 * 1024usize;
    // 4096-aligned 1 MiB buffer for O_DIRECT, RAII-owned (v0.6): the old
    // manual alloc/dealloc pair freed correctly on the error path but was
    // one refactor or panic away from leaking — Drop can't be skipped.
    let mut abuf = AlignedBuffer::new(chunk, 4096)?;

    let result = (|| -> Result<DiskSpeed, String> {
        let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true)
            .custom_flags(libc::O_DIRECT).open(&path).map_err(|e| format!("open (O_DIRECT): {e}"))?;
        let t0 = Instant::now();
        let mut written = 0u64;
        while written < total {
            f.write_all(abuf.as_slice()).map_err(|e| format!("write: {e}"))?; written += chunk as u64;
            let el = t0.elapsed().as_secs_f64();
            if el > 0.0 { progress("write", (written as f64 / 1_000_000.0) / el); }
        }
        f.sync_all().map_err(|e| format!("sync: {e}"))?;
        let write_s = t0.elapsed().as_secs_f64();
        drop(f);
        let mut f = std::fs::OpenOptions::new().read(true)
            .custom_flags(libc::O_DIRECT).open(&path).map_err(|e| format!("open read: {e}"))?;
        let mut rbuf: &mut [u8] = abuf.as_mut_slice();
        let t1 = Instant::now();
        let mut read_total = 0u64;
        loop {
            let n = f.read(&mut rbuf).map_err(|e| format!("read: {e}"))?;
            if n == 0 { break; }
            read_total += n as u64;
            let el = t1.elapsed().as_secs_f64();
            if el > 0.0 { progress("read", (read_total as f64 / 1_000_000.0) / el); }
            if read_total >= total { break; }
        }
        let read_s = t1.elapsed().as_secs_f64();
        Ok(DiskSpeed { path: dir.display().to_string(), size_mb: size,
            write_mbps: (total as f64 / 1_000_000.0) / write_s,
            read_mbps: (total as f64 / 1_000_000.0) / read_s })
    })();
    result
}

/// Page-aligned heap buffer whose allocation is released by Drop on every
/// path — success, error return, or panic unwind.
struct AlignedBuffer {
    ptr: *mut u8,
    layout: std::alloc::Layout,
    size: usize,
}
impl AlignedBuffer {
    fn new(size: usize, align: usize) -> Result<Self, String> {
        let layout = std::alloc::Layout::from_size_align(size, align).map_err(|e| e.to_string())?;
        let ptr = unsafe { std::alloc::alloc(layout) };
        if ptr.is_null() { return Err("alloc failed".into()); }
        unsafe { std::ptr::write_bytes(ptr, 0xA5, size) };
        Ok(Self { ptr, layout, size })
    }
    fn as_slice(&self) -> &[u8] { unsafe { std::slice::from_raw_parts(self.ptr, self.size) } }
    fn as_mut_slice(&mut self) -> &mut [u8] { unsafe { std::slice::from_raw_parts_mut(self.ptr, self.size) } }
}
impl Drop for AlignedBuffer {
    fn drop(&mut self) { unsafe { std::alloc::dealloc(self.ptr, self.layout) } }
}

const CF_BASE: &str = "https://speed.cloudflare.com";

pub fn network_speed_test() -> Result<NetSpeed, String> {
    network_speed_test_p(|_, _| {})
}

pub fn network_speed_test_p(mut progress: impl FnMut(&str, f64)) -> Result<NetSpeed, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(30)))
        .user_agent("LinuxSystemProfiler/0.3")
        .build().into();
    // latency: best of 5 tiny requests
    let mut best = f64::MAX;
    for _ in 0..5 {
        let t = Instant::now();
        agent.get(&format!("{CF_BASE}/__down?bytes=0")).call().map_err(|e| format!("cannot reach speed test server: {e}"))?
            .body_mut().read_to_vec().map_err(|e| e.to_string())?;
        best = best.min(t.elapsed().as_secs_f64() * 1000.0);
    }
    progress("latency", best);
    // download: 25 MB, streamed and counted
    let dl_bytes = 25_000_000u64;
    let mut resp = agent.get(&format!("{CF_BASE}/__down?bytes={dl_bytes}")).call()
        .map_err(|e| format!("download test failed: {e}"))?;
    let t = Instant::now();
    let mut got = 0u64;
    let mut rbuf = vec![0u8; 64 * 1024];
    let mut reader = resp.body_mut().as_reader();
    loop {
        let n = reader.read(&mut rbuf).map_err(|e| format!("download read: {e}"))?;
        if n == 0 { break; }
        got += n as u64;
        let el = t.elapsed().as_secs_f64();
        if el > 0.05 { progress("download", (got as f64 * 8.0 / 1_000_000.0) / el); }
    }
    let dl_s = t.elapsed().as_secs_f64();
    // upload: 10 MB POST
    let ul_bytes = 10_000_000usize;
    let payload = vec![0x42u8; ul_bytes];
    let t = Instant::now();
    agent.post(&format!("{CF_BASE}/__up")).send(&payload[..])
        .map_err(|e| format!("upload test failed: {e}"))?
        .body_mut().read_to_vec().map_err(|e| e.to_string())?;
    let ul_s = t.elapsed().as_secs_f64();
    progress("upload", (ul_bytes as f64 * 8.0 / 1_000_000.0) / ul_s);
    Ok(NetSpeed { server: "speed.cloudflare.com".into(), latency_ms: best,
        download_mbps: (got as f64 * 8.0 / 1_000_000.0) / dl_s,
        upload_mbps: (ul_bytes as f64 * 8.0 / 1_000_000.0) / ul_s })
}
