# Roadmap

## v0.1 — CLI core (SHIPPED 2026-10-13)
Collectors (sysfs/proc/lspci/lsusb/lsblk/smartctl), unified tree, JSON + Markdown
+ sanitized forum exports, health flags, check-only updates (checkupdates/
pacman, fwupd, flatpak), atomic checkpoint/resume. Verified live on Omarchy
(Intel N95) and in the sandbox VM.

## v0.2 — deps self-heal + GUI (2026-10-13)
- Automatic dependency detection/install in core + CLI + GUI banner (shipped)
- Tauri GUI: sidebar tree, detail panes, health rail, updates pane with
  explicit per-group apply (pkexec), Copy-for-Forum, adaptive layouts (shipped)
- SMART/dmidecode elevated one-shot helper — still open (user-level SMART
  returns empty on NVMe; root run verified PASSED/40°C/10h/0% on Omarchy)

## v0.25 — data quality
- Group virtual NICs (docker/veth/bridges/VPN) under a collapsed "Virtual" node
- Missing-tool detection surfaced explicitly (lsusb/smartctl absent on Omarchy)
- dmidecode memory modules via polkit helper; PCIe link negotiated vs max (LnkSta vs LnkCap)
- Linux Hardware DB lookup links per PCI/USB id; driver mainline/staging/out-of-tree tags
- SMART attribute detail + NVMe error log

## v0.3 — Tauri GUI (Apple design law, adaptive layouts)
- Sidebar tree + detail panes per DESIGN.md; desktop/laptop/tablet/phone modes
- Updates pane: per-group apply via polkit, live log, firmware extra-confirm
- History: diff this run vs last run (flag discovery changes)

## v0.4 — ecosystem give-back
- One-click upstream bug-report bundles (sanitized) for Arch/kernel/LVFS
- hw-probe compatible export; submit-with-consent flow
