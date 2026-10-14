# DESIGN — Linux System Profiler

Apple design language is LAW here, same as Fable Files. Source: Apple WWDC design
talks via the `apple-design` skill + `~/workspace/fable-files/DESIGN.md`.

## Form factors — one app, every screen (Dickles, 2026-10-13)
Rust core + adaptive UI. The same binary/layout system must work on:
- **Desktop** (ultrawide included): 3-pane — sidebar tree | detail | health rail
- **Laptop**: 2-pane — sidebar tree | detail, health as a banner row
- **Tablet**: 2-pane collapsible — sidebar overlays as a sheet, detail full-width
- **Phone**: 1-pane — card list, drill-in navigation, bottom action bar
Breakpoints are layout modes, not separate apps. All spacing in rem so text-size
scaling never breaks layout. Touch targets ≥ 2.75rem on coarse pointers.

## Layout — modeled on macOS System Information
- Left sidebar = the unified topology tree: Overview, Hardware (CPU, Memory,
  Storage, Graphics/Displays, Network, Power, PCI, USB, Audio), Software,
  Updates, Health.
- Selecting a node swaps only the detail pane. Enter/exit along the same path;
  detail panes originate from their sidebar row (spatial consistency).

## Control geometry — shared, never per-dialog (Fable Files law)
- One height: **2rem** for buttons, inputs, selects.
- Input radius **9px**; pill buttons from the shared family only.
- Finder-style rounded swatches for health: green / amber / red are the ONLY
  status colors, always paired with a text label (never color alone).
- System font first; negative tracking on large display headings (-0.02em),
  body tracking near 0, tight leading on headings, comfortable on body.

## Motion (springs, not scripts)
- Default spring: critically damped (damping 1.0, response 0.3–0.4) — no overshoot.
- Bounce (damping ~0.8) ONLY after a momentum gesture (flick/drag release).
- Feedback on pointer-down, continuous during drags, never only at the end.
- Every animation interruptible from its current on-screen value; no input lockout.
- Velocity handoff + momentum projection for sidebar sheets and card stacks.

## Materials
- Sidebar + toolbar = translucent layer (`backdrop-filter: blur(20px)
  saturate(180%)`), content scrolls underneath. Heavier material for the
  sidebar (structural), lighter for interactive chips. Never stack light-on-light.
- Scroll-edge fade, not a hard 1px divider, where chrome overlaps content.

## Accessibility — non-negotiable
- `prefers-reduced-motion`: cross-fades instead of slides/springs.
- `prefers-reduced-transparency`: solid/frostier surfaces, blur dropped.
- `prefers-contrast: more`: near-solid surfaces + defined borders.

## Updates UX (the dangerous part — design for trust)
- Updates pane is CHECK-first: counts, versions, sources (pacman / LVFS / Flatpak).
- Applying is always an explicit, per-group action with a preview of exactly
  what will run, polkit elevation, live log, and a cancel that actually cancels.
- Never auto-apply. Never bundle a reboot silently. Firmware gets its own
  scarier confirm ("on AC power" check first).
