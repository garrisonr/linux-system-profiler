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
    /// The volume the user picked (its mount point).
    pub path: String,
    /// The directory the test file actually ran in, when that isn't
    /// the mount point itself: a system drive's root (/) is not
    /// user-writable, so a folder on the same filesystem stood in
    /// (see resolve_test_dir). None when the file ran at the mount.
    #[serde(default)]
    pub test_dir: Option<String>,
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

/// Device id of the filesystem containing `p` (st_dev) — the proof
/// that two directories sit on the same filesystem.
fn fs_dev(p: &Path) -> Option<u64> {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(p).ok().map(|m| m.dev())
}

/// Cheap writability probe: create a one-byte file, delete it. Never
/// guess from permission bits — filesystems (and ACLs, and read-only
/// mounts) lie; an actual create is the truth.
fn probe_writable(dir: &Path) -> bool {
    let probe = dir.join(format!(
        ".lsp-probe-{}-{}.tmp",
        std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos()).unwrap_or(0)
    ));
    match std::fs::OpenOptions::new().write(true).create_new(true).open(&probe) {
        Ok(mut f) => {
            let ok = f.write_all(&[0]).is_ok();
            drop(f);
            let _ = std::fs::remove_file(&probe);
            ok
        }
        Err(_) => false,
    }
}

/// Where the test file for the volume mounted at `mount` can actually
/// live. The mount root itself wins when the user's account can write
/// there (data drives). A system drive's root (/) normally isn't
/// user-writable — then the first writable directory ON THE SAME
/// FILESYSTEM (proven by device id) stands in: the app's cache folder,
/// the home folder, the system temp dir. Measuring through any of
/// those measures the same physical drive; measuring through a
/// different filesystem would measure the wrong disk, so those are
/// never accepted.
pub fn resolve_test_dir(mount: &Path) -> Result<PathBuf, String> {
    let want_dev = fs_dev(mount).ok_or_else(|| format!(
        "{} isn't available anymore — the drive may have been unplugged or unmounted. \
         Re-run the profile and pick it again.",
        mount.display()
    ))?;
    if probe_writable(mount) { return Ok(mount.to_path_buf()); }
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(xdg) = std::env::var("XDG_CACHE_HOME") {
        if !xdg.is_empty() {
            candidates.push(PathBuf::from(xdg).join("linux-system-profiler"));
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        if !home.is_empty() {
            candidates.push(PathBuf::from(&home).join(".cache").join("linux-system-profiler"));
            candidates.push(PathBuf::from(&home));
        }
    }
    candidates.push(std::env::temp_dir());
    for cand in candidates {
        let _ = std::fs::create_dir_all(&cand);
        if fs_dev(&cand) != Some(want_dev) { continue; }
        if probe_writable(&cand) { return Ok(cand); }
    }
    Err(format!(
        "The speed test found nowhere to write on the drive mounted at {}. \
         The top of that drive isn't writable by your account, and no folder on the \
         same drive passed the write check either. If this is your own drive, check \
         its permissions — or mount it somewhere your account can write and try again.",
        mount.display()
    ))
}

/// Translate an I/O failure into what happened + what it means + what
/// to do. A raw "open: Permission denied (os error 13)" tells the user
/// nothing (Dickles, 2026-10-15 — the boot-drive test "failed" with
/// exactly that). The technical detail rides at the end so a copied
/// error still helps a bug report.
pub fn plain_io_error(context: &str, path: &Path, e: &std::io::Error) -> String {
    let detail = format!("Technical detail: {e}");
    match e.raw_os_error() {
        Some(13) | Some(1) => format!(
            "Couldn't {context} in {} — permission denied. Your user account isn't allowed to write there, \
             and nothing was changed. If this is your own drive, check the folder's permissions, or pick a \
             drive your account can write to. {detail}",
            path.display()
        ),
        Some(28) => format!(
            "Couldn't {context} in {} — the drive ran out of room mid-test. Free some space on this drive \
             and try again. {detail}",
            path.display()
        ),
        Some(30) => format!(
            "Couldn't {context} in {} — that drive is mounted read-only, and the test has to write a \
             temporary file. Remount the drive read-write to test it. {detail}",
            path.display()
        ),
        Some(2) => format!(
            "Couldn't {context} in {} — that folder isn't there anymore. The drive may have been unplugged \
             or unmounted; plug it back in, re-run the profile, and pick it again. {detail}",
            path.display()
        ),
        _ => format!("Couldn't {context} in {}: {e}", path.display()),
    }
}

/// Disk test with the size chosen from the volume's actual free space:
/// the preferred 1 GiB when there's room, a smaller file (with a note
/// in the result) when there isn't, and a plain refusal only when no
/// tier fits at all. The temp file lands on the target volume (in the
/// resolved test dir) and is removed on every path (TempGuard).
pub fn disk_speed_test_auto(dir: &Path, progress: impl FnMut(&str, f64)) -> Result<DiskSpeed, String> {
    let test_dir = resolve_test_dir(dir)?;
    let size = match free_bytes(&test_dir) {
        Some(free) => pick_size_mb(free).ok_or_else(|| {
            format!(
                "Not enough free space on {} for a speed test — even a small test file needs about 128 MB free.",
                dir.display()
            )
        })?,
        None => 256,
    };
    let mut r = run_disk_test(dir, &test_dir, size, progress)?;
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

pub fn disk_speed_test_p(dir: &Path, size_mb: u64, progress: impl FnMut(&str, f64)) -> Result<DiskSpeed, String> {
    let test_dir = resolve_test_dir(dir)?;
    run_disk_test(dir, &test_dir, size_mb, progress)
}

/// The test itself: writes + reads the temp file in `test_dir` (a
/// directory on the same filesystem as the picked volume `mount`,
/// resolved by resolve_test_dir) and reports against `mount`.
fn run_disk_test(mount: &Path, test_dir: &Path, size_mb: u64, mut progress: impl FnMut(&str, f64)) -> Result<DiskSpeed, String> {
    use std::os::unix::fs::OpenOptionsExt;
    let size = size_mb.clamp(64, 4096);
    let total = size * 1024 * 1024;
    // Safety floor only — choosing a size that fits with real headroom
    // is disk_speed_test_auto's job (pick_size_mb).
    if let Some(free) = free_bytes(test_dir) {
        if free < total + 64 * 1024 * 1024 {
            return Err(format!(
                "Not enough free space on {} for a speed test — the test file plus a 64 MB safety margin has to fit. Free some space on this drive and try again.",
                mount.display()
            ));
        }
    }
    let path = test_dir.join(format!(".lsp-speedtest-{}.tmp", std::process::id()));
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
                    .open(&path).map_err(|e| plain_io_error("create the test file", &path, &e))?,
                false,
            ),
        };
        let t0 = Instant::now();
        let mut written = 0u64;
        while written < total {
            f.write_all(abuf.as_slice()).map_err(|e| plain_io_error("write the test file", &path, &e))?; written += chunk as u64;
            let el = t0.elapsed().as_secs_f64();
            if el > 0.0 { progress("write", (written as f64 / 1_000_000.0) / el); }
        }
        f.sync_all().map_err(|e| plain_io_error("finish writing the test file", &path, &e))?;
        let write_s = t0.elapsed().as_secs_f64();
        drop(f);
        let mut f = if direct {
            std::fs::OpenOptions::new().read(true)
                .custom_flags(libc::O_DIRECT).open(&path).map_err(|e| plain_io_error("read the test file back", &path, &e))?
        } else {
            std::fs::OpenOptions::new().read(true)
                .open(&path).map_err(|e| plain_io_error("read the test file back", &path, &e))?
        };
        let mut rbuf: &mut [u8] = abuf.as_mut_slice();
        let t1 = Instant::now();
        let mut read_total = 0u64;
        loop {
            let n = f.read(&mut rbuf).map_err(|e| plain_io_error("read the test file", &path, &e))?;
            if n == 0 { break; }
            read_total += n as u64;
            let el = t1.elapsed().as_secs_f64();
            if el > 0.0 { progress("read", (read_total as f64 / 1_000_000.0) / el); }
            if read_total >= total { break; }
        }
        let read_s = t1.elapsed().as_secs_f64();
        // Honest notes: buffered-I/O flattery, and a test file that had
        // to run from a stand-in folder because the mount root itself
        // isn't user-writable (still the same drive — same filesystem).
        let mut notes: Vec<String> = Vec::new();
        if !direct {
            notes.push("Direct I/O isn't available on this filesystem — numbers may be flattered by the page cache.".into());
        }
        let fell_back = test_dir != mount;
        if fell_back {
            notes.push(format!(
                "The test file ran from {} — the top of this drive isn't writable by your account. \
                 That folder is on the same drive, so these numbers still measure this drive.",
                test_dir.display()
            ));
        }
        Ok(DiskSpeed { path: mount.display().to_string(),
            test_dir: if fell_back { Some(test_dir.display().to_string()) } else { None },
            device: None, size_mb: size,
            write_mbps: (total as f64 / 1_000_000.0) / write_s,
            read_mbps: (total as f64 / 1_000_000.0) / read_s,
            note: if notes.is_empty() { None } else { Some(notes.join(" ")) } })
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
    fn probe_and_resolve_on_writable_dir() {
        let dir = std::env::temp_dir();
        assert!(probe_writable(&dir), "temp dir must probe writable");
        assert!(!probe_writable(Path::new("/definitely/not/a/real/dir")));
        // A writable mount root is used as-is — no fallback.
        let r = resolve_test_dir(&dir).expect("temp dir resolves");
        assert_eq!(r, dir);
    }

    #[test]
    fn plain_errors_speak_plainly() {
        let p = Path::new("/some/drive");
        let eacces = plain_io_error("create the test file", p, &std::io::Error::from_raw_os_error(13));
        assert!(eacces.contains("permission denied"), "{eacces}");
        assert!(eacces.contains("your account") || eacces.contains("Your user account"), "{eacces}");
        assert!(!eacces.starts_with("open:"), "never the raw os error: {eacces}");
        assert!(eacces.contains("Technical detail"), "{eacces}");
        let enospc = plain_io_error("write the test file", p, &std::io::Error::from_raw_os_error(28));
        assert!(enospc.contains("ran out of room"), "{enospc}");
        let erofs = plain_io_error("create the test file", p, &std::io::Error::from_raw_os_error(30));
        assert!(erofs.contains("read-only"), "{erofs}");
        let enoent = plain_io_error("create the test file", p, &std::io::Error::from_raw_os_error(2));
        assert!(enoent.contains("isn't there anymore"), "{enoent}");
    }

    #[test]
    fn resolve_rejects_a_vanished_mount() {
        let err = resolve_test_dir(Path::new("/definitely/not/a/real/dir")).unwrap_err();
        assert!(err.contains("isn't available anymore"), "{err}");
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
