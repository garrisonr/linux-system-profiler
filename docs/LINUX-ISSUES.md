# Linux issues we design against — living list

Reviewed twice daily (cron `linux-issues-review`, ~morning + evening ET).
Each review: diff this file against upstream reality (kernel, LVFS,
linux-hardware.org, distro tooling), note what changed, suggest one concrete
thing Dickles + Grizz can do to improve the ecosystem.

## Standing issues (seeded 2026-10-13 from Dickles' profiler brief)
1. **No unified system profile.** Hardware truth is scattered across /sys,
   /proc, udev, and a dozen tools with incompatible output. → This project.
2. **No single updates surface.** Packages (pacman/apt/dnf), firmware (LVFS),
   and Flatpaks each have their own tool, none aware of the others.
3. **Driver visibility is poor.** Users can't easily see driver-in-use vs
   available, mainline vs staging vs out-of-tree, or PCIe link negotiated
   vs advertised.
4. **SMART data is expert-only.** Endurance %, temps, and failure prediction
   exist but no friendly health surface reads them.
5. **Privilege friction.** dmidecode / SMART need root; GUIs have no clean
   polkit pattern for read-only hardware queries.
6. **Forum support posts leak privacy.** Serials, MACs, hostnames, public IPs
   pasted raw into bug reports every day. Sanitized export should be default.
7. **EDID / display pipeline opacity.** "Why is my monitor at the wrong res?"
   has no user-facing answer on either X11 or Wayland.
8. **Wayland/X11 tool fragmentation.** Display, input, and capture tooling
   still splits by session type with no unified fallback layer.

## Changelog
- 2026-10-13: List created. Baseline incumbents noted: Hardinfo2 (GUI, no
  updates hub), hw-probe/linux-hardware.org (upload-based probing), fwupd/LVFS
  (firmware only). Profiler v0.1 CLI shipped locally addressing #1, #2 (check
  side), #4, #6.
