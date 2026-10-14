use linux_system_profiler::{atomic_write, collect_profile, ensure_deps, report, sanitize};
use std::path::PathBuf;
use std::process::ExitCode;

fn usage() -> &'static str {
    "linux-system-profiler 0.6.0 — Apple System Information, for Linux\n\n\
USAGE:\n  lsp [--format tree|json|md|forum] [--out FILE] [--no-updates]\n      [--cache-dir DIR] [--resume] [--sanitize] [--no-deps]\n\n\
Formats:\n  tree   human tree view (default, stdout)\n  json   full machine-readable profile (SP-inspired schema)\n  md     Markdown report\n  forum  sanitized Markdown, safe to paste into GitHub / Arch Wiki / Reddit\n\n\
Dependencies: missing probe tools (usbutils, smartmontools, pciutils, fwupd)\n  are installed AUTOMATICALLY via your package manager on first run.\n  Pass --no-deps to opt out.\n\
Updates are CHECK-ONLY — nothing is ever applied without you."
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut format = "tree".to_string();
    let mut out: Option<PathBuf> = None;
    let mut check_updates = true;
    let mut force_sanitize = false;
    let mut resume = false;
    let mut auto_deps = true;
    let mut cache_dir = PathBuf::from(".lsp-cache");
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--format" => { i += 1; format = args.get(i).cloned().unwrap_or(format); }
            "--out" => { i += 1; out = args.get(i).map(PathBuf::from); }
            "--no-updates" => check_updates = false,
            "--sanitize" => force_sanitize = true,
            "--resume" => resume = true,
            "--no-deps" => auto_deps = false,
            "--disk-test" => {
                let dir = std::env::var("HOME").unwrap_or_else(|_| ".".into());
                println!("Running disk speed test in {dir} (256 MiB, O_DIRECT) ...");
                match linux_system_profiler::speedtest::disk_speed_test(std::path::Path::new(&dir), 256) {
                    Ok(r) => println!("Write: {:.0} MB/s · Read: {:.0} MB/s ({} MiB test file)", r.write_mbps, r.read_mbps, r.size_mb),
                    Err(e) => println!("Disk test failed: {e}"),
                }
                return ExitCode::SUCCESS;
            }
            "--speed-test" => {
                println!("Running network speed test (speed.cloudflare.com) ...");
                match linux_system_profiler::speedtest::network_speed_test() {
                    Ok(r) => println!("Latency: {:.0} ms · Download: {:.1} Mbps · Upload: {:.1} Mbps", r.latency_ms, r.download_mbps, r.upload_mbps),
                    Err(e) => println!("Network test failed: {e}"),
                }
                return ExitCode::SUCCESS;
            }
            "--cache-dir" => { i += 1; if let Some(d) = args.get(i) { cache_dir = PathBuf::from(d); } }
            "-h" | "--help" => { println!("{}", usage()); return ExitCode::SUCCESS; }
            other => { eprintln!("unknown argument: {other}\n\n{}", usage()); return ExitCode::FAILURE; }
        }
        i += 1;
    }
    if format == "forum" { force_sanitize = true; }

    // Self-heal dependencies FIRST, so no section silently comes back empty.
    let dep_log = ensure_deps(auto_deps);
    if !dep_log.contains("all probe tools present") { eprintln!("{dep_log}"); }

    let p = collect_profile(check_updates, Some(cache_dir), resume);

    let mut json_val = serde_json::to_value(&p).unwrap_or(serde_json::Value::Null);
    if force_sanitize { sanitize::sanitize_json(&mut json_val); }
    let sanitized_profile: linux_system_profiler::model::Profile = if force_sanitize {
        serde_json::from_value(json_val.clone()).unwrap_or(p.clone())
    } else { p.clone() };

    let rendered = match format.as_str() {
        "json" => serde_json::to_string_pretty(&json_val).unwrap_or_default(),
        "md" => report::markdown(&sanitized_profile, force_sanitize),
        "forum" => report::markdown(&sanitized_profile, true),
        _ => report::tree_text(&sanitized_profile),
    };
    match out {
        Some(path) => {
            if let Err(e) = atomic_write(&path, &rendered) {
                eprintln!("failed to write {}: {e}", path.display());
                return ExitCode::FAILURE;
            }
            eprintln!("wrote {}", path.display());
        }
        None => println!("{rendered}"),
    }
    ExitCode::SUCCESS
}
