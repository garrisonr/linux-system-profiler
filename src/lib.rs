pub mod collectors;
pub mod live;
pub mod deps;
pub mod elevate;
pub mod model;
pub mod report;
pub mod speedtest;
pub mod sanitize;
pub mod updates;

use model::Profile;
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

/// Atomic write: temp file in same dir, verify by re-reading, then rename.
pub fn atomic_write(path: &PathBuf, contents: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() { fs::create_dir_all(parent)?; }
    // Unique tmp name (v0.6): a fixed ".tmp" suffix let two concurrent
    // exports to the same path collide and clobber each other's staging file.
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos()).unwrap_or(0);
    let tmp = path.with_extension(format!("tmp-{}-{}", std::process::id(), nanos));
    fs::write(&tmp, contents)?;
    let back = fs::read_to_string(&tmp)?;
    if back != contents {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "write verification failed"));
    }
    fs::rename(&tmp, path)
}

fn now_iso() -> String {
    collectors::cmd("date", &["-u", "+%Y-%m-%dT%H:%M:%SZ"])
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "1970-01-01T00:00:00Z".into())
}

/// Dependency self-heal: if probe tools are missing and `auto_install` is set,
/// install them via the distro package manager (pkexec/sudo elevation).
/// Returns a log line describing what happened. Never fails the profile.
pub fn ensure_deps(auto_install: bool) -> String {
    let st = deps::dep_status();
    if st.missing.is_empty() { return "deps: all probe tools present".into(); }
    let msg = format!("deps: missing {} ({})", st.missing.join(", "), st.missing_features.join("; "));
    if !auto_install {
        return format!("{msg} — auto-install off; packages needed: {}", st.packages_to_install.join(" "));
    }
    match deps::install_deps(&st) {
        Ok(log) => format!("{msg} — installed automatically: {}", st.packages_to_install.join(" ")),
        Err(e) => format!("{msg} — auto-install failed: {} ({})", e.message, e.detail),
    }
}

/// Collect a full profile. When `cache_dir` is given, each collector is
/// checkpointed atomically as it completes; with `resume`, checkpointed
/// collectors from an interrupted run are reused instead of re-probed.
pub fn collect_profile(check_updates: bool, cache_dir: Option<PathBuf>, resume: bool) -> Profile {
    collect_profile_with_progress(check_updates, cache_dir, resume, &|_| {})
}

/// Same as `collect_profile`, but `progress` is called with a stage key
/// ("system", "cpu", "memory", "storage", …) as each collector STARTS, so a
/// UI can narrate the wait honestly instead of showing a dead screen.
pub fn collect_profile_with_progress(
    check_updates: bool,
    cache_dir: Option<PathBuf>,
    resume: bool,
    progress: &dyn Fn(&str),
) -> Profile {
    let checkpoint_path = cache_dir.as_ref().map(|d| d.join("checkpoint.json"));
    let mut done: BTreeMap<String, serde_json::Value> = BTreeMap::new();
    if resume {
        if let Some(cp) = &checkpoint_path {
            if let Ok(text) = fs::read_to_string(cp) {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
                    if let Some(obj) = v.as_object() {
                        for (k, val) in obj { done.insert(k.clone(), val.clone()); }
                    }
                }
            }
        }
    }
    macro_rules! collect {
        ($name:literal, $ty:ty, $fn:expr) => {{
            if let Some(saved) = done.get($name) {
                serde_json::from_value::<$ty>(saved.clone()).unwrap_or_default()
            } else {
                progress($name);
                let v: $ty = $fn;
                if let Some(cp) = &checkpoint_path {
                    done.insert($name.to_string(), serde_json::to_value(&v).unwrap_or(serde_json::Value::Null));
                    let text = serde_json::to_string_pretty(&serde_json::Value::Object(
                        done.iter().map(|(k, v)| (k.clone(), v.clone())).collect())).unwrap_or_default();
                    let _ = atomic_write(cp, &text);
                }
                v
            }
        }};
    }
    // Degradation notes from the collectors that can fail silently
    // (storage/network/partitions/accounts report variants). Loaded
    // back from the checkpoint on resume so a resumed profile keeps
    // the warnings its collectors earned.
    let mut coll_warnings: Vec<String> = done.get("collection_warnings")
        .and_then(|v| serde_json::from_value::<Vec<String>>(v.clone()).ok())
        .unwrap_or_default();

    let mut p = Profile {
        schema_version: 4,
        tool_version: env!("CARGO_PKG_VERSION").to_string(),
        generated_at: now_iso(),
        hostname: collectors::read_trim("/proc/sys/kernel/hostname")
            .or_else(|| collectors::cmd("hostname", &[]).map(|s| s.trim().to_string()))
            .unwrap_or_default(),
        ..Default::default()
    };
    p.system = collect!("system", model::SystemInfo, collectors::collect_system());
    p.cpu = collect!("cpu", model::CpuInfo, collectors::collect_cpu());
    p.memory = collect!("memory", model::MemoryInfo, collectors::collect_memory());
    p.storage = collect!("storage", Vec<model::StorageDevice>, {
        let (v, w) = collectors::collect_storage_report();
        if let Some(w) = w { coll_warnings.push(w); }
        v
    });
    p.network = collect!("network", Vec<model::NetworkInterface>, {
        let (v, w) = collectors::collect_network_report();
        if let Some(w) = w { coll_warnings.push(w); }
        v
    });
    p.graphics = collect!("graphics", Vec<model::GpuInfo>, collectors::collect_graphics());
    p.displays = collect!("displays", Vec<model::DisplayInfo>, collectors::collect_displays());
    p.volumes = collect!("volumes", Vec<model::VolumeInfo>, collectors::collect_volumes());
    p.disk_partitions = collect!("disk_partitions", Vec<model::DiskPartition>, {
        let (v, w) = collectors::collect_disk_partitions_report();
        if let Some(w) = w { coll_warnings.push(w); }
        v
    });
    p.printers = collect!("printers", Vec<model::PrinterInfo>, collectors::collect_printers());
    p.bluetooth = collect!("bluetooth", Vec<model::PairedDevice>, collectors::collect_bluetooth());
    p.accounts = collect!("accounts", Vec<model::UserAccount>, {
        let (v, w) = collectors::collect_accounts_report();
        if let Some(w) = w { coll_warnings.push(w); }
        v
    });
    p.security = collect!("security", model::SecurityInfo, collectors::collect_security());
    p.power = collect!("power", Vec<model::BatteryInfo>, collectors::collect_power());
    p.power_detail = collectors::collect_power_detail();
    p.upower_devices = collectors::collect_upower_devices();
    p.power_history = collectors::collect_power_history();
    progress("devices");
    let (pci, usb) = collectors::collect_bus();
    p.pci = pci; p.usb = usb;
    progress("audio");
    p.audio = collectors::collect_audio();
    p.software = collect!("software", model::SoftwareInfo, collectors::collect_software());
    p.updates = if check_updates {
        collect!("updates", model::UpdatesInfo, updates::collect_updates())
    } else { model::UpdatesInfo::default() };
    // The package-list truncation note rides inside SoftwareInfo.extra
    // (checkpoint-safe); mirror it into the warnings so it surfaces as
    // a health flag next to the collector degradations.
    if let Some(note) = p.software.extra.get("explicit_packages_truncated") {
        if !coll_warnings.contains(note) { coll_warnings.push(note.clone()); }
    }
    p.collection_warnings = coll_warnings.clone();
    if let Some(cp) = &checkpoint_path {
        done.insert("collection_warnings".to_string(),
            serde_json::to_value(&coll_warnings).unwrap_or(serde_json::Value::Null));
        let text = serde_json::to_string_pretty(&serde_json::Value::Object(
            done.iter().map(|(k, v)| (k.clone(), v.clone())).collect())).unwrap_or_default();
        let _ = atomic_write(cp, &text);
    }
    p.health = report::health_flags(&p);
    p
}
