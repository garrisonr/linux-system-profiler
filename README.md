# Linux System Profiler (working name)

Apple's System Information, for Linux — plus the thing Apple never gave it:
a single place to see and apply **driver, firmware, and software updates**.

**Status: v0.9.1.** CLI core + Tauri GUI shipped and dogfooded
on real hardware; under active development. v0.9.1 takes over the
Power Statistics seat: the Power view now covers every UPower device
(batteries, UPSes, wireless peripherals) with per-device detail and
the recorded power history UPower keeps. v0.9.0 "Live Wire" made Profiler live:
a Rust sampler (the webview only paints) drives new Live, Processes
(end a process with pid+start-time identity re-verified at signal
time), History (a 30-minute in-session scrub), Services and Startup
views; failed updates now leave a persistent plain-English failure
record with Retry; and a small vizsla keeps you company while updates
run. Previously — v0.8.11, the Vizsla style pass:
the GUI inherits the Vizsla OS Style Guide — one type system (Inter UI ·
Fraunces display · Newsreader reading · Victor Mono data, OFL) and one
kit button geometry in every theme (themes are palettes/materials
only), update status notes render as banners, and a group whose
updates are all blocked no longer offers an Apply button that can't
succeed. (v0.8.9 added the plain-English update classifier; v0.8.1's
audit fixes — safe apt apply, metadata refresh on Check, dpkg/rpm
inventory, off-thread Copy & Share — all still stand.)
Apple design language, adaptive desktop / laptop / tablet / phone layouts
(see `gui/` + `DESIGN.md`). Nineteen theme engines (Auto matches your
system), Report a Bug / Request a Feature in the About pane.

**Dependencies self-heal (Dickles' rule — no half-broken software):** if probe
tools are missing (`usbutils`, `smartmontools`, `pciutils`, `fwupd`,
`pacman-contrib`), the profiler installs them itself on first run via your
package manager — targeted installs (never a surprise full-system upgrade),
elevated with pkexec in graphical sessions / sudo headless. `--no-deps` opts
out. The GUI shows the same check as a one-click banner — and if the system's
password agent can't be reached from the app's session, it says so calmly
with a retry and a copyable terminal command instead of dumping raw errors.

## Why this exists
Linux already has `lshw`, `inxi`, Hardinfo2, `hw-probe`, `fastfetch`… scattered,
inconsistent, none of them Apple-clean, and none of them closes the loop from
*"here's your hardware"* to *"here's what's out of date and how to fix it."*
This tool unifies them: one topology tree, health flags with hints, and an
updates hub (pacman / fwupd-LVFS / Flatpak) that checks on load and applies
only on an explicit, elevated action per group.

## Run it
```bash
cargo build --release
./target/release/linux-system-profiler                 # tree view (auto-installs missing probe tools first)
./target/release/linux-system-profiler --format json --out profile.json
./target/release/linux-system-profiler --format forum --out forum-post.md
./target/release/linux-system-profiler --resume        # reuse checkpoints after an interrupted run
```

## Coverage (from Lumo's checklist)
1. Hardware topology from `/sys`, `/proc`, `lspci`, `lsusb` — unified tree ✔
2. Firmware & power: DMI/BIOS, Secure Boot, battery health + cycles ✔ (dmidecode detail elevates via polkit in the GUI)
3. Storage: lsblk hierarchy + SMART health / temp / endurance via smartctl ✔ (when installed + permitted)
4. Network: state, negotiated speed, driver, addresses, Wi-Fi link ✔
5. Graphics: DRM cards, driver-in-use, displays (Wayland `wlr-randr` / X11 `xrandr`) ✔
6. Software: kernel modules, glibc/libstdc++, package counts, services ✔
7. Exports: JSON (SP-inspired schema), Markdown ✔
8. Troubleshooting: sanitized Copy-for-Forum export ✔ · health hints ✔
9. Dashboard: color-coded health flags in tree view ✔ · full interactive GUI ✔
10. Compatibility: LVFS firmware-update surfacing ✔ · Linux Hardware DB lookups = roadmap

**Update rule (standing):** the hub checks automatically; nothing is ever
applied without an explicit, elevated user action, one group at a time.

## Rules this project follows
- Rust only. Resume-able everything (atomic checkpoints). Verify values, not counts.
- Apple design guidelines for every UI surface. No per-dialog invented controls.
- No bundled copyrighted assets. Original code, MIT license.
- Preliminary name screen done 2026-10-13 — see `docs/LEGAL-SCREEN.md`.
  Still a working name; no domains registered under it.

## v0.3 — QA fixes, Belarc parity, speed tests
- **Clipboard that actually copies**: Copy for Forum now writes to the real
  system clipboard (X11 + Wayland) and confirms with a toast; if the
  clipboard is unreachable you get a select-all text view instead of silence.
- **Network, redesigned**: physical interfaces as a clean health-style list
  with kind chips and sane speeds; Docker bridges, veth pairs and VPN tunnels
  grouped under Virtual interfaces. IPv4 first, link-local last.
- **Belarc-parity additions** (the parts that make sense on Linux): display
  inventory from EDID (model, size, year, resolution), CUPS printers, user
  accounts with admin flags and last logins, a security-posture section
  (firewall, LUKS volumes, sshd, Secure Boot), mounted volumes with free
  space, explicitly-installed package list, last full-upgrade date, and
  paired Bluetooth devices. Sanitized exports hash usernames and drop
  home paths/login lines entirely.
- **Speed tests, on demand only**: disk sequential write/read with O_DIRECT
  (`--disk-test`, Storage pane) and network latency/download/upload against
  Cloudflare (`--speed-test`, Network pane). They never run during profiling.
- About pane; sidebar footer buttons stack so labels never overflow.

## v0.8.0 — close-out (2026-10-14)
- **New app icon** in the shared icon family (Fable Files is the anchor):
  full-bleed rounded Bondi-blue tile, one bold flat white gauge glyph —
  SVG master + PNG/ICO/ICNS set under `gui/src-tauri/icons/`.
- **Chrome that respects the desktop**: the window paints a solid base in
  the active engine's own color edge-to-edge (no wallpaper bleed-through or
  clashing frame), and native controls/scrollbars follow `color-scheme`.
- **Real theme dropdown**: custom themed listbox, keyboard accessible
  (arrows, Home/End, Enter, Esc, type-ahead) — the native `<select>` is gone.
- **Loading state**: skeleton Overview in the engine's colors, a live stage
  line narrated by the collectors ("reading storage & SMART health…",
  "checking for updates…"), sections rising in as the profile lands.
  Static skeletons under reduced-motion.
- **Elevation grace**: unreachable password agent ⇒ calm in-app guidance,
  a retry, and a copyable manual command. Raw tool output never reaches a
  dialog; it lives behind an explicit Details view.
- **Report a Bug / Request a Feature** in the About pane — bug reports are
  pre-filled with this machine's basics (versions + hardware only; serials,
  MACs, and addresses are never included).
