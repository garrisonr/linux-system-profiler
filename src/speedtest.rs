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
    /// The directory the test ran in (the volume's mount point).
    pub path: String,
    /// The block device behind the volume, when known — so a pasted
    /// result names the drive unambiguously ("Tested /dev/sda1 at
    /// /media/…/Desk SSD").
    #[serde(default)]
    pub device: Option<String>,
    pub size_mb: u64,
    pub write_mbps: f64,
    pub read_mbps: f64,
    /// Plain-language caveats the user must see with the numbers: the
    /// test file was shrunk for tight free space, or direct I/O wasn't
    /// available on this filesystem.
    #[serde(default)]
    pub note: Option<String>,
}

/// Test-file sizes the auto picker walks, largest first.
pub const SIZE_TIERS_MB: &[u64] = &[1024, 512, 256, 128, 64];
/// The size the picker aims for when space is ample.
pub const PREFERRED_SIZE_MB: u64 = 1024;

/// Pick the largest tier that fits `free` bytes with headroom (the test
/// file plus a margin of max(64 MiB, tier/4) must fit). None when even
/// the smallest tier doesn't fit.
pub fn pick_size_mb(free: u64) -> Option<u64> {
    let mib = 1024 * 1024u64;
    for &tier in SIZE_TIERS_MB {
        let need = tier * mib + std::cmp::max(64 * mib, tier * mib / 4);
        if free >= need {
            return Some(tier);
        }
    }
    None
}

/// Disk test with the size chosen from the volume's actual free space:
/// the preferred 1 GiB when there's room, a smaller file (with a note
/// in the result) when there isn't, and a plain refusal only when no
/// tier fits at all. The temp file lands on the target volume and is
/// removed on every path (TempGuard).
pub fn disk_speed_test_auto(dir: &Path, progress: impl FnMut(&str, f64)) -> Result<DiskSpeed, String> {
    let size = match free_bytes(dir) {
        Some(free) => pick_size_mb(free).ok_or_else(|| {
            format!(
                "Not enough free space on {} for a speed test — even a small test file needs about 128 MB free.",
                dir.display()
            )
        })?,
        None => 256,
    };
    let mut r = disk_speed_test_p(dir, size, progress)?;
    if size < PREFERRED_SIZE_MB {
        let shrink = format!("Test file shrunk to {size} MB — free space on this drive is tight.");
        r.note = Some(match r.note.take() {
            Some(n) => format!("{shrink} {n}"),
            None => shrink,
        });
    }
    Ok(r)
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
    // Safety floor only — choosing a size that fits with real headroom
    // is disk_speed_test_auto's job (pick_size_mb).
    if let Some(free) = free_bytes(dir) {
        if free < total + 64 * 1024 * 1024 {
            return Err(format!("not enough free space on {} (need test file + 64 MiB margin)", dir.display()));
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
        // Direct I/O where the filesystem allows it; some filesystems
        // (exFAT variants, tmpfs configurations) refuse O_DIRECT — fall
        // back to buffered I/O and SAY SO in the result note rather
        // than failing a test the user asked for.
        let (mut f, direct) = match std::fs::OpenOptions::new().write(true).create(true).truncate(true)
            .custom_flags(libc::O_DIRECT).open(&path)
        {
            Ok(f) => (f, true),
            Err(_) => (
                std::fs::OpenOptions::new().write(true).create(true).truncate(true)
                    .open(&path).map_err(|e| format!("open: {e}"))?,
                false,
            ),
        };
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
        let mut f = if direct {
            std::fs::OpenOptions::new().read(true)
                .custom_flags(libc::O_DIRECT).open(&path).map_err(|e| format!("open read: {e}"))?
        } else {
            std::fs::OpenOptions::new().read(true)
                .open(&path).map_err(|e| format!("open read: {e}"))?
        };
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
        Ok(DiskSpeed { path: dir.display().to_string(), device: None, size_mb: size,
            write_mbps: (total as f64 / 1_000_000.0) / write_s,
            read_mbps: (total as f64 / 1_000_000.0) / read_s,
            note: if direct { None } else {
                Some("Direct I/O isn't available on this filesystem — numbers may be flattered by the page cache.".into())
            } })
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

#[cfg(test)]
mod tests {
    use super::*;
    const MIB: u64 = 1024 * 1024;

    #[test]
    fn pick_size_walks_tiers_with_headroom() {
        assert_eq!(pick_size_mb(20 * 1024 * MIB), Some(1024)); // ample
        assert_eq!(pick_size_mb(1280 * MIB), Some(1024)); // exactly tier + tier/4
        assert_eq!(pick_size_mb(1279 * MIB), Some(512));
        assert_eq!(pick_size_mb(300 * MIB), Some(128)); // 256 needs 320; 128 needs 192
        assert_eq!(pick_size_mb(192 * MIB), Some(128));
        assert_eq!(pick_size_mb(191 * MIB), Some(64)); // 64 needs 128
        assert_eq!(pick_size_mb(128 * MIB), Some(64));
        assert_eq!(pick_size_mb(127 * MIB), None); // nothing fits honestly
        assert_eq!(pick_size_mb(0), None);
    }

    #[test]
    fn auto_test_on_real_dir_reports_and_cleans_up() {
        // Smoke: the temp file lands in the target dir and is gone after.
        let dir = std::env::temp_dir();
        let before: Vec<_> = std::fs::read_dir(&dir).unwrap().filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with(".lsp-speedtest-"))
            .collect();
        assert!(before.is_empty());
        let r = disk_speed_test_auto(&dir, |_, _| {}).expect("auto test runs");
        assert!(r.write_mbps > 0.0 && r.read_mbps > 0.0);
        assert_eq!(r.path, dir.display().to_string());
        let after: Vec<_> = std::fs::read_dir(&dir).unwrap().filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with(".lsp-speedtest-"))
            .collect();
        assert!(after.is_empty(), "temp file must be cleaned up");
    }
}
