// Linux System Profiler — GUI backend.
// All real work lives in the linux-system-profiler core library; this shell
// exposes it to the frontend, owns the profile cache (so Copy & Share is
// instant instead of re-probing the machine), the update-apply actions
// (always explicit, per-group, elevated via pkexec), speed tests with live
// progress events, and the share actions (clipboard / save / email).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use linux_system_profiler::deps::{self, DepStatus};
use linux_system_profiler::model::Profile;
use linux_system_profiler::speedtest::{self, DiskSpeed, NetSpeed};
use linux_system_profiler::{collect_profile, report, sanitize};
use std::process::Command;
use std::sync::Mutex;
use tauri::{Emitter, State};

#[derive(Default)]
struct ProfileCache(Mutex<Option<Profile>>);

/// Rendered Copy & Share output, keyed by report kind. Rendering a report
/// (serialize + sanitize + format) was the v0.4 gaps memo's Copy & Share
/// lag: it ran synchronously on every click. Now it renders once, off the
/// UI thread, and repeat clicks reuse the text. Cleared whenever the
/// cached profile changes.
#[derive(Default)]
struct RenderCache(std::sync::Arc<Mutex<std::collections::HashMap<String, String>>>);

fn render_cached(rc: &std::sync::Arc<Mutex<std::collections::HashMap<String, String>>>, p: &Profile, kind: &str) -> String {
    if let Some(t) = rc.lock().unwrap().get(kind) { return t.clone(); }
    let t = render_report(p, kind);
    rc.lock().unwrap().insert(kind.to_string(), t.clone());
    t
}

fn clear_render_cache(renders: &RenderCache) {
    renders.0.lock().unwrap().clear();
}

fn cached_or_collect(cache: &State<ProfileCache>) -> Profile {
    if let Some(p) = cache.0.lock().unwrap().clone() { return p; }
    let p = collect_profile(true, None, false);
    *cache.0.lock().unwrap() = Some(p.clone());
    p
}

#[tauri::command]
async fn get_profile(app: tauri::AppHandle, cache: State<'_, ProfileCache>, renders: State<'_, RenderCache>) -> Result<Profile, String> {
    // v0.6: collection probes dozens of tools and can take seconds on slow
    // buses — run it off the IPC thread so the window never freezes on load.
    // (Tauri requires async commands with reference inputs to return Result.)
    // v0.8: each collector stage is emitted as a "profile-progress" event so
    // the loading screen can narrate the wait (skeletons + stage labels)
    // instead of sitting on a dead gray slab.
    let progress_app = app.clone();
    let p = tauri::async_runtime::spawn_blocking(move || {
        linux_system_profiler::collect_profile_with_progress(true, None, false, &|stage| {
            let _ = progress_app.emit("profile-progress", stage);
        })
    })
        .await.map_err(|e| e.to_string())?;
    *cache.0.lock().unwrap() = Some(p.clone());
    clear_render_cache(&renders);
    Ok(p)
}

#[tauri::command]
fn get_dep_status() -> DepStatus {
    deps::dep_status()
}

/// Outcome of a one-click dependency install. Failures are CLASSIFIED, never
/// raw: `message` is calm and user-safe; tool output lives only in `detail`,
/// which the UI shows solely behind an explicit details view (Dickles' law:
/// no user ever sees a dependency error).
#[derive(serde::Serialize)]
struct InstallOutcome {
    ok: bool,
    kind: String, // installed | already | no-agent | cancelled | unavailable | failed
    message: String,
    detail: String,
    packages: Vec<String>,
    manual_command: Option<String>,
}

/// One-click install of missing probe tools. pkexec shows the system's own
/// password prompt — the app never sees or handles the user's password.
/// Runs on a blocking thread: the pkexec prompt can sit open for as long as
/// the user takes, and v0.8.0 ran this synchronously on the IPC thread,
/// freezing the window behind the prompt (Gemini audit 2026-10-14).
#[tauri::command]
async fn install_deps() -> InstallOutcome {
    tauri::async_runtime::spawn_blocking(install_deps_blocking).await
        .unwrap_or_else(|e| InstallOutcome {
            ok: false, kind: "failed".into(),
            message: "The install couldn't be started.".into(),
            detail: format!("install task failed: {e}"),
            packages: vec![], manual_command: None,
        })
}

fn install_deps_blocking() -> InstallOutcome {
    let st = deps::dep_status();
    let manual = deps::manual_command(&st);
    if st.missing.is_empty() {
        return InstallOutcome {
            ok: true, kind: "already".into(),
            message: "All probe tools are already installed.".into(),
            detail: String::new(), packages: vec![], manual_command: manual,
        };
    }
    match deps::install_deps(&st) {
        Ok(log) => InstallOutcome {
            ok: true, kind: "installed".into(),
            message: format!("Installed: {}", st.packages_to_install.join(", ")),
            detail: log, packages: st.packages_to_install.clone(), manual_command: manual,
        },
        Err(e) => InstallOutcome {
            ok: false, kind: e.kind.as_str().into(),
            message: e.message, detail: e.detail,
            packages: st.packages_to_install.clone(), manual_command: manual,
        },
    }
}

/// Copy arbitrary text (e.g. the manual install command) to the clipboard.
/// Returns false when the clipboard is unreachable — the caller then shows
/// the text in a selectable view instead of failing silently.
#[tauri::command]
fn copy_text(text: String) -> bool {
    clipboard_set(&text)
}

/// Open one of THIS app's own web pages (issue tracker). Allowlisted to the
/// project repo so the frontend can never turn it into a generic launcher.
#[tauri::command]
fn open_url(url: String) -> Result<(), String> {
    const PREFIX: &str = "https://github.com/garrisonr/linux-system-profiler/";
    if !url.starts_with(PREFIX) {
        return Err("refusing to open an unexpected address".into());
    }
    Command::new("xdg-open").arg(&url).spawn()
        .map_err(|e| format!("could not open your browser: {e}"))?;
    Ok(())
}

fn sanitized_copy(p: &Profile) -> Profile {
    let mut v = serde_json::to_value(p).unwrap_or(serde_json::Value::Null);
    sanitize::sanitize_json(&mut v);
    serde_json::from_value::<Profile>(v).unwrap_or_else(|_| p.clone())
}

fn render_report(p: &Profile, kind: &str) -> String {
    match kind {
        "full-md" => report::markdown(p, false),
        "json" => serde_json::to_string_pretty(p).unwrap_or_default(),
        _ => report::markdown(&sanitized_copy(p), true),
    }
}

#[derive(serde::Serialize)]
struct CopyResult { copied: bool, text: String, chars: usize }

fn clipboard_set(text: &str) -> bool {
    arboard::Clipboard::new()
        .and_then(|mut cb| cb.set_text(text.to_string()))
        .is_ok()
}

/// Copy & Share — copy action. Uses the CACHED profile; the report renders
/// OFF the UI thread and is cached per kind, so repeat clicks are instant
/// (the v0.4 gaps memo's synchronous re-render lag, still open in v0.8.0 —
/// Gemini audit 2026-10-14). Kinds: "sanitized" (Markdown, forum-safe),
/// "full-md", "json".
async fn share_copy_impl(cache: &State<'_, ProfileCache>, renders: &State<'_, RenderCache>, kind: String) -> CopyResult {
    let p = cached_or_collect(cache);
    let rc = renders.0.clone();
    let kind2 = kind.clone();
    let joined = tauri::async_runtime::spawn_blocking(move || {
        let text = render_cached(&rc, &p, &kind2);
        let chars = text.chars().count();
        let copied = clipboard_set(&text);
        CopyResult { copied, text, chars }
    }).await;
    match joined {
        Ok(r) => r,
        Err(_) => {
            let p = cached_or_collect(cache);
            let text = render_report(&p, &kind);
            let chars = text.chars().count();
            CopyResult { copied: clipboard_set(&text), text, chars }
        }
    }
}

#[tauri::command]
async fn share_copy(cache: State<'_, ProfileCache>, renders: State<'_, RenderCache>, kind: String) -> Result<CopyResult, String> {
    Ok(share_copy_impl(&cache, &renders, kind).await)
}

/// Backward-compatible name used by the v0.3 frontend.
#[tauri::command]
async fn copy_forum(cache: State<'_, ProfileCache>, renders: State<'_, RenderCache>) -> Result<CopyResult, String> {
    Ok(share_copy_impl(&cache, &renders, "sanitized".into()).await)
}

/// Copy & Share — save action. Native save dialog (rfd/GTK); writes
/// atomically via the core helper. Returns the saved path, or None if the
/// user cancelled the dialog.
#[tauri::command]
async fn share_save(cache: State<'_, ProfileCache>, renders: State<'_, RenderCache>, kind: String) -> Result<Option<String>, String> {
    let p = cached_or_collect(&cache);
    // Render off the UI thread (cached per kind) — v0.8.0 rendered
    // synchronously here on every click, the Copy & Share lag.
    let rc = renders.0.clone();
    let (text, filename, filter_name, ext) = {
        tauri::async_runtime::spawn_blocking(move || {
            match kind.as_str() {
                "full-md" => (render_cached(&rc, &p, "full-md"), "system-profile.md", "Markdown", "md"),
                "json" => (render_cached(&rc, &p, "json"), "system-profile.json", "JSON", "json"),
                _ => (render_cached(&rc, &p, "sanitized"), "system-profile-forum.md", "Markdown", "md"),
            }
        }).await.map_err(|e| e.to_string())?
    };
    let picked = tauri::async_runtime::spawn_blocking(move || {
        rfd::FileDialog::new()
            .set_file_name(filename)
            .add_filter(filter_name, &[ext])
            .save_file()
    }).await.map_err(|e| e.to_string())?;
    match picked {
        Some(path) => {
            linux_system_profiler::atomic_write(&path, &text)
                .map_err(|e| format!("could not write {}: {e}", path.display()))?;
            Ok(Some(path.display().to_string()))
        }
        None => Ok(None),
    }
}

/// Copy & Share — email action: opens the mail client with a short summary
/// (the full report belongs in Copy/Save, not pasted into a mailto body).
#[tauri::command]
fn share_email(cache: State<ProfileCache>) -> Result<(), String> {
    let p = cached_or_collect(&cache);
    let enc = |s: &str| s.chars().map(|c| match c {
        'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' => c.to_string(),
        _ => format!("%{:02X}", c as u32),
    }).collect::<String>();
    let subject = format!("System Profile — {}", if p.hostname.is_empty() { "this computer" } else { &p.hostname });
    let body = format!(
        "Machine: {} {}\nOS: {} · kernel {}\nCPU: {}\nMemory: {:.1} GiB\nStorage: {}\nUpdates pending: {} system · {} firmware · {} flatpak\n\n(Generated locally by Linux System Profiler v{} — full report attached separately.)",
        p.system.manufacturer.clone().unwrap_or_default(), p.system.product_name.clone().unwrap_or_default(),
        p.system.os_name.clone().unwrap_or_default(), p.system.kernel.clone().unwrap_or_default(),
        p.cpu.model_name.clone().unwrap_or_default(),
        p.memory.total_kb.unwrap_or(0) as f64 * 1024.0 / 1_073_741_824.0,
        p.storage.iter().map(|d| d.model.clone().unwrap_or(d.name.clone())).collect::<Vec<_>>().join(", "),
        p.updates.system_updates.len(), p.updates.firmware_updates.len(), p.updates.flatpak_updates.len(),
        p.tool_version);
    let url = format!("mailto:?subject={}&body={}", enc(&subject), enc(&body));
    Command::new("xdg-open").arg(&url).spawn()
        .map_err(|e| format!("could not open mail client: {e}"))?;
    Ok(())
}

#[derive(serde::Serialize, Clone)]
struct SpeedProgress { test: String, phase: String, value: f64 }

#[tauri::command]
async fn run_disk_test(app: tauri::AppHandle) -> Result<DiskSpeed, String> {
    let dir = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    tauri::async_runtime::spawn_blocking(move || {
        speedtest::disk_speed_test_p(std::path::Path::new(&dir), 256, |phase, value| {
            let _ = app.emit("speed-progress", SpeedProgress {
                test: "disk".into(), phase: phase.into(), value });
        })
    }).await.map_err(|e| e.to_string())?
}

#[tauri::command]
async fn run_network_test(app: tauri::AppHandle) -> Result<NetSpeed, String> {
    tauri::async_runtime::spawn_blocking(move || {
        speedtest::network_speed_test_p(|phase, value| {
            let _ = app.emit("speed-progress", SpeedProgress {
                test: "net".into(), phase: phase.into(), value });
        })
    }).await.map_err(|e| e.to_string())?
}

/// Apply ONE update group. Explicit user action only — the frontend must
/// confirm before calling, and firmware requires its own stronger confirm.
/// Returns the command's combined output for the live log pane.
#[derive(serde::Serialize)]
struct ElevatedDmi {
    modules: Vec<linux_system_profiler::model::MemoryModule>,
    serial_number: Option<String>,
}

/// One-shot elevated dmidecode via pkexec (the system's own password
/// prompt — the app never handles credentials). Fills the two things a
/// regular user can't read: memory-module detail and the system serial.
/// The user explicitly clicks for this; nothing elevates silently.
/// Runs on a blocking thread — the polkit prompt can wait on the user
/// indefinitely, and on the IPC thread it froze the whole window
/// (Gemini audit 2026-10-14).
#[tauri::command]
async fn elevated_dmi() -> Result<ElevatedDmi, String> {
    tauri::async_runtime::spawn_blocking(elevated_dmi_blocking).await
        .map_err(|e| format!("elevation task failed: {e}"))?
}

fn elevated_dmi_blocking() -> Result<ElevatedDmi, String> {
    let out = Command::new("pkexec").args(["dmidecode"]).output()
        .map_err(|e| format!("failed to launch pkexec: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(if err.is_empty() { "elevation cancelled or denied".into() } else { err });
    }
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    Ok(ElevatedDmi {
        modules: linux_system_profiler::collectors::parse_dmidecode_memory(&text),
        serial_number: linux_system_profiler::collectors::parse_dmidecode_serial(&text),
    })
}

static APPLYING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// v0.5.2: apply runs OFF the UI thread (a sync command froze the window for
/// minutes during an 800MB flatpak update and GNOME flagged the app as not
/// responding), streams each output line to the frontend as it happens, and
/// refuses to start a second run while one is in flight.
#[tauri::command]
async fn apply_updates(app: tauri::AppHandle, group: String) -> Result<String, String> {
    use std::sync::atomic::Ordering;
    if APPLYING.swap(true, Ordering::SeqCst) {
        return Err("An update is already running — wait for it to finish.".to_string());
    }
    let result = tauri::async_runtime::spawn_blocking(move || run_apply(&app, &group)).await;
    APPLYING.store(false, Ordering::SeqCst);
    match result {
        Ok(r) => r,
        Err(e) => Err(format!("update task failed: {e}")),
    }
}

fn run_apply(app: &tauri::AppHandle, group: &str) -> Result<String, String> {
    use std::io::{BufRead, BufReader};
    use std::process::Stdio;
    use tauri::Emitter;
    // v0.6: distro-aware apply; the command table lives in the core
    // (updates::apply_command) where tests pin the safety contract — no
    // distro path may ever dist-upgrade/full-upgrade. v0.8.0's apt arm ran
    // `apt-get dist-upgrade -y`, which can REMOVE packages; it now runs a
    // plain `apt-get upgrade -y`, and the index refresh it used to smuggle
    // in moved to the explicit Check (refresh_update_check).
    let (prog, args) = linux_system_profiler::updates::apply_command(group)
        .ok_or_else(|| format!("unknown update group: {group}"))?;
    let mut child = Command::new(prog).args(&args)
        .stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()
        .map_err(|e| format!("failed to launch {prog}: {e}"))?;
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    let mut readers = Vec::new();
    if let Some(out) = child.stdout.take() {
        let tx = tx.clone();
        readers.push(std::thread::spawn(move || {
            for line in BufReader::new(out).lines().map_while(Result::ok) { let _ = tx.send(line); }
        }));
    }
    if let Some(err) = child.stderr.take() {
        let tx = tx.clone();
        readers.push(std::thread::spawn(move || {
            for line in BufReader::new(err).lines().map_while(Result::ok) { let _ = tx.send(line); }
        }));
    }
    drop(tx);
    let mut full = String::new();
    for line in rx {
        let _ = app.emit("update-progress", line.clone());
        full.push_str(&line);
        full.push('\n');
    }
    for r in readers { let _ = r.join(); }
    let status = child.wait().map_err(|e| format!("update process error: {e}"))?;
    if status.success() { Ok(full) } else { Err(full) }
}

/// Explicit Check from the Updates view. On apt, a check is only as fresh
/// as the local package indexes — v0.8.0 read them stale and hid the
/// refresh inside Apply. Now an explicit Check performs the elevated
/// metadata refresh first (installs nothing), then re-collects and swaps
/// the fresh updates into the cached profile so every view agrees.
#[tauri::command]
async fn refresh_update_check(cache: State<'_, ProfileCache>, renders: State<'_, RenderCache>) -> Result<linux_system_profiler::model::UpdatesInfo, String> {
    let joined = tauri::async_runtime::spawn_blocking(move || {
        let mut refreshed = false;
        if let Some((elevate, args)) = linux_system_profiler::updates::metadata_refresh_command() {
            let out = Command::new(&elevate).args(&args).output()
                .map_err(|e| format!("couldn't start the package-list refresh: {e}"))?;
            if !out.status.success() {
                return Err("The package-list refresh didn't complete — results may be out of date. (The password prompt may have been dismissed.)".to_string());
            }
            refreshed = true;
        }
        let mut info = linux_system_profiler::updates::collect_updates();
        if refreshed {
            info.notes.insert(0, "Package lists were refreshed just now — these results are current.".into());
        }
        Ok(info)
    }).await.map_err(|e| e.to_string())?;
    let info = joined?;
    if let Some(p) = cache.0.lock().unwrap().as_mut() {
        p.updates = info.clone();
        clear_render_cache(&renders);
    }
    Ok(info)
}

fn main() {
    tauri::Builder::default()
        .manage(ProfileCache::default())
        .manage(RenderCache::default())
        .invoke_handler(tauri::generate_handler![
            get_profile, get_dep_status, install_deps, copy_forum,
            share_copy, share_save, share_email, copy_text, open_url,
            run_disk_test, run_network_test, apply_updates, elevated_dmi,
            refresh_update_check
        ])
        .run(tauri::generate_context!())
        .expect("error while running Linux System Profiler");
}
