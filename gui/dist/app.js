// Linux System Profiler frontend — vanilla JS, no build step, no dependencies.
// Talks to the Rust core through Tauri commands (src-tauri/src/main.rs).
const invoke = window.__TAURI__.core.invoke;
const listen = window.__TAURI__.event.listen;

const $ = (id) => document.getElementById(id);
const esc = (s) => String(s ?? "—").replace(/[&<>"']/g, (c) =>
  ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]));
const gb = (bytes) => bytes == null ? "—" : (bytes / 1073741824).toFixed(1) + " GiB";
const o = (v) => v ?? "—";
const reducedMotion = window.matchMedia("(prefers-reduced-motion: reduce)").matches;

let P = null;
let current = "overview";

const SECTIONS = [
  ["overview", "Overview"],
  ["live", "Live"],
  ["processes", "Processes"],
  ["history", "History"],
  ["cpu", "CPU"],
  ["memory", "Memory"],
  ["storage", "Storage"],
  ["graphics", "Graphics & Displays"],
  ["network", "Network"],
  ["power", "Power"],
  ["pci", "PCI Devices"],
  ["usb", "USB Devices"],
  ["software", "Software"],
  ["services", "Services"],
  ["startup", "Startup"],
  ["accounts", "Accounts & Security"],
  ["updates", "Updates"],
  ["themes", "Themes"],
  ["about", "About"],
];

/* ---------------- themes ---------------- */
// Engines = distinct design languages. Distros = DistroWatch top 20 (plus
// Dickles' picks); several distros share a language, noted in the gallery.
const ENGINES = {
  bondi: "Bondi", cupertino: "Cupertino", omarchy: "Omarchy", ubuntu: "Ubuntu · Yaru", fedora: "Fedora · Adwaita",
  zorin: "Zorin OS", elementary: "elementary OS", deepin: "Deepin", pop: "Pop!_OS · COSMIC",
  mint: "Linux Mint · Mint-Y", breeze: "KDE · Breeze", opensuse: "openSUSE", manjaro: "Manjaro",
  liquidglass: "Liquid Glass", materialyou: "Material You",
  endeavour: "EndeavourOS", garuda: "Garuda", peppermint: "Peppermint", mx: "MX Linux", phosh: "Phosh (mobile)",
};
const ENGINE_PREVIEW = { // bg, ink, accent — theme cards preview their own theme
  bondi: ["#232326", "#e8e8ea", "#2ba3c7"],
  cupertino: ["#f5f5f7", "#1d1d1f", "#2ba3c7"], omarchy: ["#1a1b26", "#c0caf5", "#7aa2f7"],
  ubuntu: ["#f7f7f7", "#2c2c2c", "#e95420"], fedora: ["#f6f5f4", "#241f19", "#3584e4"],
  zorin: ["#f2f4f7", "#1b1d21", "#175ddc"], elementary: ["#fafafa", "#333333", "#3689e6"],
  deepin: ["#eef4fb", "#1f2329", "#0081ff"], pop: ["#2b2b2b", "#f0efed", "#faa41a"],
  mint: ["#f4f6f1", "#2e3436", "#92b372"], breeze: ["#eff0f1", "#232627", "#3daee9"],
  opensuse: ["#f3f6ee", "#26331f", "#73ba25"], manjaro: ["#22262a", "#e8ecef", "#16a085"],
  endeavour: ["#282a36", "#f8f8f2", "#7b5ea7"], garuda: ["#1e1e2e", "#e5e9f0", "#e2436b"],
  peppermint: ["#2a2326", "#f2ecec", "#a31f34"], mx: ["#eceff1", "#263238", "#546e7a"],
  phosh: ["#242424", "#eeeeec", "#3584e4"],
  liquidglass: ["#1a202c", "#f5f9ff", "#6fd3ff"], materialyou: ["#141218", "#e6e0e9", "#cfbcff"],
};
const DISTROS = [ // [name, engine, shared-language note] — alphabetical (Dickles: sorted, always)
  ["AnduinOS", "zorin", "GNOME-based; closest family: Zorin"],
  ["Arch Linux", "cupertino", "DIY desktop — neutral baseline"],
  ["Bazzite", "breeze", "KDE Plasma / Breeze family"],
  ["BigLinux", "breeze", "KDE Plasma / Breeze family"],
  ["Bluestar", "breeze", "KDE Plasma / Breeze family"],
  ["CachyOS", "breeze", "KDE Plasma / Breeze family"],
  ["Debian", "fedora", "GNOME / Adwaita family"],
  ["Deepin", "deepin", ""],
  ["elementary OS", "elementary", ""],
  ["EndeavourOS", "endeavour", ""],
  ["Fedora", "fedora", ""],
  ["Garuda Linux", "garuda", ""],
  ["KDE neon", "breeze", ""],
  ["Linux Mint", "mint", ""],
  ["Manjaro", "manjaro", ""],
  ["MX Linux", "mx", ""],
  ["NixOS", "fedora", "GNOME default; Adwaita family"],
  ["Nobara", "fedora", "Fedora-based; Adwaita family"],
  ["Omarchy", "omarchy", ""],
  ["openSUSE", "opensuse", ""],
  ["Peppermint OS", "peppermint", ""],
  ["Phosh (mobile shell)", "phosh", ""],
  ["Pop!_OS", "pop", ""],
  ["Ubuntu", "ubuntu", ""],
  ["Zorin OS", "zorin", ""],
];
const OSID_THEME = {
  cachyos: "breeze", linuxmint: "mint", mint: "mint", mx: "mx", debian: "fedora",
  pop: "pop", endeavouros: "endeavour", zorin: "zorin", manjaro: "manjaro", fedora: "fedora",
  ubuntu: "ubuntu", anduinos: "zorin", opensuse: "opensuse", "opensuse-leap": "opensuse",
  "opensuse-tumbleweed": "opensuse", nobara: "fedora", arch: "cupertino", bluestar: "breeze",
  bazzite: "breeze", elementary: "elementary", nixos: "fedora", biglinux: "breeze", neon: "breeze",
  peppermint: "peppermint", garuda: "garuda", omarchy: "omarchy", deepin: "deepin",
};
function detectTheme(p) {
  const id = (p.system.os_id || "").toLowerCase();
  const desk = (p.system.desktop || "").toLowerCase();
  if (desk.includes("phosh")) return "phosh";
  if (desk.includes("dde") || id === "deepin") return "deepin";
  return OSID_THEME[id] || "cupertino";
}
function applyTheme(pref, p) {
  const resolved = !pref || pref === "auto" ? (p ? detectTheme(p) : "cupertino") : pref;
  if (resolved === "cupertino") delete document.documentElement.dataset.theme;
  else document.documentElement.dataset.theme = resolved;
  return resolved;
}

/* ---------------- theme dropdown (custom — never a native select) -------
   v0.8: the native <select> rendered as a giant white box under dark
   schemes. This is the house pattern instead: a 2rem button + listbox in
   app tokens, full keyboard support (arrows, Home/End, Enter, Esc,
   type-ahead). Sorted A–Z with Auto pinned, same as the old list. */
const THEME_OPTIONS = [
  ["auto", "Auto (this system)"], ["bondi", "Bondi"], ["cupertino", "Cupertino"],
  ["deepin", "Deepin"], ["elementary", "elementary"], ["endeavour", "EndeavourOS"],
  ["fedora", "Fedora"], ["garuda", "Garuda"], ["breeze", "KDE Breeze"],
  ["liquidglass", "Liquid Glass"], ["manjaro", "Manjaro"], ["materialyou", "Material You"],
  ["mint", "Mint"], ["mx", "MX Linux"], ["omarchy", "Omarchy"],
  ["opensuse", "openSUSE"], ["peppermint", "Peppermint"], ["phosh", "Phosh"],
  ["pop", "Pop!_OS"], ["ubuntu", "Ubuntu"], ["zorin", "Zorin OS"],
];
const themeControl = {
  value: "auto",
  get() { return this.value; },
  set(v) { // silent: control state + label only — callers persist/apply
    this.value = v;
    const found = THEME_OPTIONS.find(([id]) => id === v);
    $("themeLabel").textContent = found ? found[1] : v;
    $("themePop").querySelectorAll(".dropdown-opt").forEach((el) =>
      el.setAttribute("aria-selected", String(el.dataset.value === v)));
  },
};
function buildThemeDropdown() {
  $("themePop").innerHTML = THEME_OPTIONS.map(([v, label]) =>
    `<button type="button" class="dropdown-opt" role="option" data-value="${v}" aria-selected="${v === themeControl.value}"><span class="check" aria-hidden="true">✓</span><span>${esc(label)}</span></button>`).join("");
  $("themePop").querySelectorAll(".dropdown-opt").forEach((el) =>
    el.addEventListener("click", () => { pickTheme(el.dataset.value); closeThemePop(); $("themeBtn").focus(); }));
}
function themePopOpen() { return !$("themePop").classList.contains("hidden"); }
function openThemePop() {
  $("themePop").classList.remove("hidden");
  $("themeBtn").setAttribute("aria-expanded", "true");
  setActiveOpt(themeControl.value);
}
function closeThemePop() {
  $("themePop").classList.add("hidden");
  $("themeBtn").setAttribute("aria-expanded", "false");
}
function setActiveOpt(v) {
  let target = null;
  $("themePop").querySelectorAll(".dropdown-opt").forEach((el) => {
    const on = el.dataset.value === v;
    el.classList.toggle("active", on);
    if (on) target = el;
  });
  if (target) target.scrollIntoView({ block: "nearest" });
}
function pickTheme(v) {
  themeControl.set(v);
  try { localStorage.setItem("lsp-theme", v); } catch (err) {}
  const resolved = applyTheme(v, P);
  showToast("Theme: " + (v === "auto" ? ENGINES[resolved] + " (auto-matched)" : ENGINES[v]));
}
function themeKeyNav(e) {
  const opts = [...$("themePop").querySelectorAll(".dropdown-opt")];
  if (!opts.length) return;
  const cur = opts.findIndex((el) => el.classList.contains("active"));
  if (e.key === "Escape") { if (themePopOpen()) { closeThemePop(); $("themeBtn").focus(); } return; }
  if (!themePopOpen()) {
    if (["ArrowDown", "ArrowUp", "Enter", " "].includes(e.key)) { e.preventDefault(); openThemePop(); }
    return;
  }
  let n = null;
  if (e.key === "ArrowDown") n = Math.min(opts.length - 1, cur + 1);
  else if (e.key === "ArrowUp") n = Math.max(0, cur - 1);
  else if (e.key === "Home") n = 0;
  else if (e.key === "End") n = opts.length - 1;
  else if (e.key === "Enter" || e.key === " ") {
    const a = opts[Math.max(0, cur)];
    if (a) { pickTheme(a.dataset.value); closeThemePop(); }
    e.preventDefault();
    return;
  } else if (/^[a-z0-9]$/i.test(e.key)) {
    const k = e.key.toLowerCase();
    const f = opts.findIndex((el) => el.textContent.trim().toLowerCase().startsWith(k));
    if (f >= 0) n = f;
  }
  if (n != null) { e.preventDefault(); setActiveOpt(opts[n].dataset.value); }
}

/* ---------------- glossary: one hint component, curated copy ----------------
   Plain-English glosses for the terms a smart non-expert would trip on
   (Dickles, v0.8.5: "What does flags mean? If I don't know, other people
   probably don't either… but make it beautiful. Don't overwhelm the
   user."). A small muted "?" lives in the label — nowhere else — and the
   gloss floats in the app's material. Copy is hand-written, kept in this
   one map, zero marketing. */
const GLOSSARY = {
  sockets: { t: "Sockets, cores & threads", d: "A socket is a physical CPU slot on the board. Each CPU holds several cores — independent processors — and most cores run two threads: two instruction streams sharing one core's muscle.", w: "Threads help when many programs work at once; a single app's speed still leans on how fast one core is." },
  l3: { t: "L3 cache", d: "The largest of the CPU's built-in memory shelves, shared by every core. It keeps the data the cores reach for most within a hair's breadth, instead of trekking out to the much slower RAM." },
  flags: { t: "CPU flags", d: "The processor's capability list — each flag names something the chip can do in hardware. This row shows a sample, not the full list. Famous ones: vmx / svm run virtual machines, avx2 crunches media and AI math quickly, aes encrypts in hardware.", w: "Programs check the flags before taking a hardware shortcut — no flag, no shortcut." },
  tjmax: { t: "TjMax & thermal headroom", d: "TjMax is the temperature ceiling the chip's maker set for it. Headroom is how far below that ceiling the CPU is running right now — near the ceiling, the chip slows itself down to stay safe." },
  governor: { t: "CPU governor", d: "The kernel's policy for how eagerly the processor speeds up and slows down. “powersave” favors quiet and battery; “performance” holds the clock speeds high." },
  epp: { t: "Energy performance preference", d: "A dial the system hands the CPU, from “give me everything” to “sip power”. The chip picks its clock speeds inside that hint." },
  swap: { t: "Swap", d: "A reserved patch of disk the system uses as overflow memory when RAM fills up.", w: "Swap is far slower than RAM — a machine swapping hard feels like it's wading through mud." },
  smart: { t: "S.M.A.R.T.", d: "Every modern drive grades its own health — error rates, reallocated sectors, wear — and reports a verdict. PASSED means the drive itself sees no signs of trouble.", w: "A failing grade is a back-up-now alarm, not a someday item. Drives rarely recover." },
  wear: { t: "Endurance used", d: "How much of a solid-state drive's rated write lifetime has been spent, as the drive counts it. At 100% the warranty's write budget is used up — the drive usually keeps working, on borrowed time." },
  virtual: { t: "Virtual interfaces", d: "Network connections that exist only in software, created by Docker, VPNs and virtual machines rather than by a physical port. They carry real traffic between containers, tunnels and guests." },
  lvfs: { t: "LVFS & fwupd", d: "The Linux Vendor Firmware Service is the shared library where hardware makers publish firmware for Linux. fwupd checks it against your devices and installs what they're offered — system, SSD, dock and peripheral updates alike." },
  phased: { t: "Phased updates", d: "Ubuntu releases some updates to a slice of machines at a time, widening the rollout over days to catch problems early. “Deferred due to phasing” means this machine hasn't been dealt in yet — the update installs on a later check, on its own." },
  luks: { t: "LUKS encryption", d: "Linux's built-in full-disk encryption: the drive's contents stay scrambled until the passphrase unlocks them at boot. A stolen, powered-off machine gives up nothing." },
};
function hintBtn(key) {
  const g = GLOSSARY[key];
  if (!g) return "";
  return `<button type="button" class="hint-q" data-hint="${key}" aria-haspopup="dialog" aria-expanded="false" aria-label="Explain: ${esc(g.t)}">?</button>`;
}
let glossAnchor = null;
let glossJustOpened = false;
function closeGloss() {
  const c = $("glossCard");
  if (c) c.classList.add("hidden");
  if (glossAnchor) glossAnchor.setAttribute("aria-expanded", "false");
  glossAnchor = null;
}
function openGloss(anchor, viaToggle) {
  const g = GLOSSARY[anchor.dataset.hint];
  const c = $("glossCard");
  if (!g || !c) return;
  if (glossAnchor === anchor && !c.classList.contains("hidden")) {
    // A click toggles shut — unless this click is the tail of the
    // focus/hover that just opened it (same gesture, not a new intent).
    if (viaToggle && !glossJustOpened) closeGloss();
    return;
  }
  closeGloss();
  glossAnchor = anchor;
  glossJustOpened = true;
  setTimeout(() => { glossJustOpened = false; }, 350);
  anchor.setAttribute("aria-expanded", "true");
  c.innerHTML = `<h4>${esc(g.t)}</h4><p>${esc(g.d)}</p>` + (g.w ? `<p class="gloss-why">${esc(g.w)}</p>` : "");
  c.classList.remove("hidden");
  const r = anchor.getBoundingClientRect();
  const w = Math.min(320, window.innerWidth - 24);
  c.style.width = w + "px";
  c.style.left = Math.min(Math.max(12, r.left - 8), window.innerWidth - w - 12) + "px";
  const below = r.bottom + 8;
  c.style.top = (below + 180 < window.innerHeight ? below : Math.max(12, r.top - 8 - c.offsetHeight)) + "px";
}

/* ---------------- shared render helpers ---------------- */
function kvTable(rows) {
  const body = rows.filter(([, v]) => v !== undefined && v !== null && v !== "")
    .map(([k, v, hintKey]) => `<tr><th>${esc(k)}${hintKey ? hintBtn(hintKey) : ""}</th><td>${esc(v)}</td></tr>`).join("");
  return body ? `<table class="kv">${body}</table>` : `<p class="empty">Nothing reported.</p>`;
}
function cards(items, render) {
  return items.length ? items.map(render).join("") : `<p class="empty">Nothing detected.</p>`;
}
const FLAG_GOTO = { Security: "accounts", Storage: "storage", Updates: "updates", Firmware: "updates",
  Battery: "power", Memory: "memory", System: "overview" };
function flagHtml(f) {
  const goto = FLAG_GOTO[f.component];
  return `<div class="flag"${goto ? ` data-goto="${goto}" role="button" tabindex="0" title="Open ${esc(f.component)} details"` : ""}><span class="dot ${esc(f.severity)}"></span><div>` +
    `<strong>${esc(f.component)}:</strong> ${esc(f.message)}` +
    (f.hint ? `<span class="hint">${esc(f.hint)}</span>` : "") + `</div></div>`;
}

/* ---------------- gauges (Ookla-style, spring-driven) ---------------- */
// One shared rAF loop steps every live gauge's spring toward its target —
// critically damped (Apple default), animating from the presentation value.
const gauges = new Map();
function gaugeMarkup(max, unit) {
  const cx = 100, cy = 100, r = 82;
  const pt = (a) => [cx + r * Math.cos(a * Math.PI / 180), cy + r * Math.sin(a * Math.PI / 180)];
  const arc = (a0, a1) => { const [x0, y0] = pt(a0), [x1, y1] = pt(a1);
    return `M ${x0.toFixed(1)} ${y0.toFixed(1)} A ${r} ${r} 0 ${a1 - a0 > 180 ? 1 : 0} 1 ${x1.toFixed(1)} ${y1.toFixed(1)}`; };
  const arcLen = 2 * Math.PI * r * (240 / 360);
  return `<svg class="gauge-svg" viewBox="0 0 200 168" aria-hidden="true">
    <path d="${arc(150, 390)}" class="g-track"/>
    <path d="${arc(150, 390)}" class="g-fill" stroke-dasharray="${arcLen.toFixed(1)}" stroke-dashoffset="${arcLen.toFixed(1)}"/>
    <g class="g-needle"><line x1="100" y1="100" x2="100" y2="38"/></g>
    <circle cx="100" cy="100" r="7" class="g-hub"/></svg>
  <div class="g-read"><span class="g-num">0</span><span class="g-unit">${esc(unit)}</span></div>
  <div class="g-phase">Ready</div>`;
}
function gaugeInit(el, max) {
  gauges.set(el, { disp: 0, vel: 0, target: 0, max, el });
  return gauges.get(el);
}
function gaugeSet(el, value) { const g = gauges.get(el); if (g) g.target = Math.max(0, value); }
function gaugeStep(now) {
  let live = false;
  for (const g of gauges.values()) {
    if (Math.abs(g.target - g.disp) > 0.01 || Math.abs(g.vel) > 0.01) live = true; else continue;
    const k = 90, c = 2 * Math.sqrt(k); // critically damped
    const dt = 1 / 60;
    g.vel += (-k * (g.disp - g.target) - c * g.vel) * dt;
    g.disp += g.vel * dt;
    if (Math.abs(g.disp - g.target) < 0.05 && Math.abs(g.vel) < 0.05) { g.disp = g.target; g.vel = 0; }
    const frac = Math.log10(1 + Math.max(0, g.disp)) / Math.log10(1 + g.max);
    const fill = g.el.querySelector(".g-fill");
    const len = parseFloat(fill.getAttribute("stroke-dasharray"));
    fill.style.strokeDashoffset = (len * (1 - Math.min(1, frac))).toFixed(1);
    g.el.querySelector(".g-needle").style.transform = `rotate(${(-120 + frac * 240).toFixed(1)}deg)`;
    g.el.querySelector(".g-num").textContent = g.disp >= 100 ? Math.round(g.disp) : g.disp.toFixed(1);
  }
  if (live || gauges.size) requestAnimationFrame(gaugeStep);
}

/* ---------------- views ---------------- */
function dockerText(p) {
  const s = p.software.docker_status;
  if (!s) return p.software.docker_images.length ? String(p.software.docker_images.length) : null;
  if (s.state === "running") return `${s.images} images · ${s.containers} containers (${s.containers_running} running)`;
  if (s.state === "inaccessible") return "daemon running — socket not accessible";
  if (s.state === "not-running") return "installed — daemon not running";
  return String(p.software.docker_images.length);
}
function linkCard(section, title, lines, badge) {
  return `<button type="button" class="link-card" data-goto="${section}">
    <span class="lc-main"><span class="lc-title">${esc(title)}</span>` +
    lines.filter(Boolean).map((l) => `<span class="lc-line">${esc(l)}</span>`).join("") +
    `</span>${badge ? `<span class="lc-badge">${esc(badge)}</span>` : ""}<span class="lc-chev">›</span></button>`;
}
const VIEWS = {
  overview(p) {
    const sys = [p.system.manufacturer, p.system.product_name].filter(Boolean).join(" ") || "This machine";
    const root = p.volumes.find((v) => v.mount === "/");
    const prim = p.network.find((n) => !n.is_virtual && n.operstate === "up") || p.network[0];
    const upd = p.updates.system_updates.length + p.updates.firmware_updates.length + p.updates.flatpak_updates.length;
    let h = linkCard("about", sys, [
      [p.system.os_name, p.system.kernel ? "kernel " + p.system.kernel : null].filter(Boolean).join(" · "),
      [p.system.bios_vendor, p.system.bios_version].filter(Boolean).join(" ")]);
    h += linkCard("cpu", "CPU", [p.cpu.model_name,
      [p.cpu.cores_physical ? p.cpu.cores_physical + " cores" : null, p.cpu.threads_logical ? p.cpu.threads_logical + " threads" : null,
       p.cpu.max_mhz ? "max " + p.cpu.max_mhz.toFixed(0) + " MHz" : null].filter(Boolean).join(" · ")]);
    h += linkCard("memory", "Memory", [
      p.memory.total_kb != null ? gb(p.memory.total_kb * 1024) + " total" : null,
      p.memory.available_kb != null ? gb(p.memory.available_kb * 1024) + " available" : null]);
    h += linkCard("storage", "Storage", [
      p.storage.map((d) => (d.model || d.name) + " · " + gb(d.size_bytes)).join(", "),
      root ? root.mount + " — " + gb(root.avail_bytes) + " free of " + gb(root.size_bytes) : null],
      p.storage.length ? String(p.storage.length) : null);
    if (p.displays.length || p.graphics.length)
      h += linkCard("graphics", "Graphics & Displays", [
        p.graphics.map((g) => g.card + " · " + o(g.driver_in_use)).join(", "),
        p.displays.map((d) => (d.name || d.connector) + " " + o(d.resolution)).join(", ")]);
    if (prim) h += linkCard("network", "Network", [
      prim.name + " (" + prim.kind + ") — " + o(prim.operstate),
      (prim.addresses || []).slice(0, 2).join("  ")]);
    h += linkCard("accounts", "Accounts & Security", [
      "Firewall: " + (p.security.firewall_active ? "active (" + o(p.security.firewall_service) + ")" : p.security.firewall_active === false ? "none active" : "—") +
        " · Secure Boot: " + o(p.system.secure_boot),
      p.accounts.length + " user accounts · " + p.security.luks_volumes + " encrypted volumes"]);
    h += linkCard("software", "Software", [
      (p.software.packages_count ?? "—") + " packages · " + (p.software.snaps.length) + " snaps · " + (dockerText(p) || "no docker"),
      p.software.last_upgrade ? "last full upgrade " + p.software.last_upgrade : null]);
    h += linkCard("updates", "Updates", [
      upd ? upd + " updates available" : "Everything up to date",
      updatesAsOf(p.updates) ? "counts as of " + updatesAsOf(p.updates).when : (p.updates.checked ? "checked" : null)], upd ? String(upd) : null);
    if (p.power.length) h += linkCard("power", "Power", p.power.map((b) =>
      b.name + " " + o(b.capacity_percent) + "% · health " + (b.health_percent != null ? b.health_percent.toFixed(0) + "%" : "—")));
    h += `<p class="empty">Profile generated ${esc(p.generated_at)} · tool v${esc(p.tool_version)} — tap any card for details.</p>`;
    return h;
  },
  cpu(p) {
    return kvTable([
      ["Model", p.cpu.model_name], ["Vendor", p.cpu.vendor],
      ["Sockets", p.cpu.sockets, "sockets"], ["Physical cores", p.cpu.cores_physical],
      ["Logical threads", p.cpu.threads_logical],
      ["Max frequency", p.cpu.max_mhz ? p.cpu.max_mhz.toFixed(0) + " MHz" : null],
      ["L3 cache", p.cpu.cache_l3, "l3"], ["Virtualization", p.cpu.virtualization],
      ["Flags (sample)", p.cpu.flags_sample.join(" "), "flags"],
    ]);
  },
  memory(p) {
    let h = kvTable([
      ["Total", p.memory.total_kb != null ? gb(p.memory.total_kb * 1024) : null],
      ["Available", p.memory.available_kb != null ? gb(p.memory.available_kb * 1024) : null],
      ["Swap total", p.memory.swap_total_kb != null ? gb(p.memory.swap_total_kb * 1024) : null, "swap"],
    ]);
    if (p._dmiSerial) h += kvTable([["System serial (via dmidecode)", p._dmiSerial]]);
    if (p.memory.modules.length) {
      h += cards(p.memory.modules, (m) =>
        `<div class="card"><h3>${esc(m.locator)}</h3><p>${esc(m.size)} · ${esc(m.mem_type)} · ${esc(m.speed)}</p><p>${esc(m.manufacturer)} ${esc(m.part_number)}</p></div>`);
    } else {
      h += `<div class="card"><h3>Module details are locked</h3>
        <p>Reading DIMM details (size, speed, manufacturer, part number) needs dmidecode with administrator rights — plus your system serial, which is root-only too. One click, one system password prompt, nothing runs silently.</p>
        <div class="row"><button class="pill primary" data-elevate type="button">Unlock with admin…</button></div></div>`;
    }
    return h;
  },
  storage(p) {
    let h = cards(p.storage, (d) => {
      const parts = [gb(d.size_bytes), d.tran || (d.name.startsWith("nvme") ? "NVMe" : null),
        d.rotational ? "rotational (HDD)" : "solid state",
        d.mountpoints && d.mountpoints.length ? "mounted " + d.mountpoints.join(", ") : null].filter(Boolean);
      const smartTail = [d.smart_temp_c != null ? d.smart_temp_c + "°C" : null,
        d.power_on_hours != null ? d.power_on_hours + " power-on hours" : null,
        d.percentage_used != null ? `endurance${hintBtn("wear")} ${d.percentage_used}% used` : null].filter(Boolean);
      const smartLine = [d.smart_health ? `SMART${hintBtn("smart")}: ${esc(d.smart_health)}` : null,
        ...smartTail].filter(Boolean).join(" · ");
      return `<div class="card"><h3>${esc(d.name)} — ${esc(d.model)}</h3><p>${parts.join(" · ")}</p>` +
        (smartLine ? `<p>${smartLine}</p>` : "") + `</div>`;
    });
    if (p.volumes.length) {
      // Deduplicate by physical device: a Btrfs pool mounts many subvolumes
      // from ONE device — present the pool once, subvolumes nested under it.
      const pools = new Map();
      for (const v of p.volumes) {
        const key = v.device || v.mount;
        if (!pools.has(key)) pools.set(key, []);
        pools.get(key).push(v);
      }
      h += `<h3>Volumes</h3>`;
      for (const [dev, vols] of pools) {
        const head = vols[0];
        h += `<div class="card"><h3 class="mono">${esc(dev)}</h3>` +
          `<p>${esc(head.fs_type)} pool · ${gb(head.avail_bytes)} free of ${gb(head.size_bytes)} · ${esc(head.use_percent)}% used</p>` +
          vols.map((v) =>
            `<div class="flag"><span class="dot ${v.use_percent >= 90 ? "red" : v.use_percent >= 75 ? "amber" : "green"}"></span><div>` +
            `<strong>${esc(v.mount)}</strong>${v.subvolume ? ` <span class="chip">${esc(v.subvolume)}</span>` : ""}</div></div>`).join("") +
          `</div>`;
      }
    }
    const volOpts = (p.volumes || []).map((v) =>
      `<option value="${esc(v.mount)}" data-device="${esc(v.device || "")}"${v.read_only ? " disabled" : ""}${v.mount === "/" ? " selected" : ""}>${esc(v.device || "device")} · ${esc(v.mount)} · ${esc(v.fs_type)} · ${gb(v.avail_bytes)} free${v.read_only ? " · read-only" : ""}</option>`).join("");
    // Unmounted partitions (enumerated via lsblk) are listed too —
    // disabled, with their state spelled out. Before v0.8.6 they were
    // simply invisible here: an unmounted eMMC rescue install did not
    // exist as far as this picker was concerned.
    const unmountedOpts = (p.disk_partitions || []).filter((d) => !d.mount).map((d) =>
      `<option disabled>${esc(d.device)} · ${gb(d.size_bytes)}${d.fs_type ? " · " + esc(d.fs_type) : ""} · ${d.fs_type ? "Not mounted — mount this drive to run a test" : "No filesystem — nothing to test until it's formatted"}</option>`).join("");
    const allOpts = volOpts + unmountedOpts;
    h += `<div class="card gauge-card"><h3>Disk speed test</h3>
      <p>Sequential write + read on the drive you pick — the test file lands on that drive and is deleted after. Direct I/O where the filesystem allows it; if free space is tight, the test file shrinks and the result says so. Drives that aren't mounted are listed with their state — mount one to test it. Write and read each get their own meter, like the network test.</p>
      ${allOpts ? `<div class="row"><label>Drive&nbsp; <select class="vol-select" data-disk-volume>${allOpts}</select></label></div>` : ""}
      <div class="gauge-pair">
        <div class="gauge" data-gauge="disk" data-phase="write"><div class="g-label">Write</div>${gaugeMarkup(2000, "MB/s")}</div>
        <div class="gauge" data-gauge="disk" data-phase="read"><div class="g-label">Read</div>${gaugeMarkup(2000, "MB/s")}</div>
      </div>
      <div class="row"><button class="pill primary" data-speed="disk" type="button">Run disk test</button>
      <span class="speed-final"></span></div></div>`;
    return h;
  },
  graphics(p) {
    let h = cards(p.graphics, (g) =>
      `<div class="card"><h3>${esc(g.card)}</h3><p>PCI ${esc(g.vendor_device)} · driver: ${esc(g.driver_in_use)}</p></div>`);
    if (p.displays.length) {
      h += `<h3>Displays</h3>` + cards(p.displays, (d) =>
        `<div class="card"><h3>${esc(d.name || d.connector)}</h3>` +
        `<p>${esc(d.resolution)}${d.refresh_hz ? " @ " + d.refresh_hz + " Hz" : ""} · ${esc(d.manufacturer)}${d.width_cm ? " · " + d.width_cm + "×" + d.height_cm + " cm" : ""}${d.manufacture_year ? " · made " + d.manufacture_year : ""}</p>` +
        `<p>Connector: ${esc(d.connector)}</p></div>`);
    }
    return h;
  },
  network(p) {
    const row = (n) =>
      `<div class="flag"><span class="dot ${n.operstate === "up" ? "green" : "info"}"></span><div>` +
      `<strong>${esc(n.name)}</strong> <span class="chip">${esc(n.kind)}</span>` +
      `<span class="hint">${[n.driver ? "driver " + n.driver : null, n.speed_mbps ? n.speed_mbps + " Mb/s" : null, n.operstate].filter(Boolean).join(" · ")}</span>` +
      (n.addresses.length ? `<span class="hint mono">${esc(n.addresses.join("  "))}</span>` : "") + `</div></div>`;
    const real = p.network.filter((n) => !n.is_virtual);
    const virt = p.network.filter((n) => n.is_virtual);
    let h = `<div class="card gauge-card"><h3>Network speed test</h3>
      <p>Against speed.cloudflare.com — download and upload get their own meters, Ookla-style.</p>
      <div class="gauge-pair">
        <div class="gauge" data-gauge="net" data-phase="download"><div class="g-label">Download</div>${gaugeMarkup(1000, "Mbps")}</div>
        <div class="gauge" data-gauge="net" data-phase="upload"><div class="g-label">Upload</div>${gaugeMarkup(1000, "Mbps")}</div>
      </div>
      <div class="row"><button class="pill primary" data-speed="net" type="button">Run network test</button>
      <span class="speed-final"></span></div></div>`;
    h += real.length ? real.map(row).join("") : `<p class="empty">No physical interfaces detected.</p>`;
    if (virt.length) h += `<h3>Virtual interfaces${hintBtn("virtual")}</h3>` + virt.map(row).join("");
    return h;
  },
  power(p) {
    const d = p.power_detail || {};
    let h = kvTable([
      ["CPU package temperature", p.cpu.temp_c != null ? p.cpu.temp_c.toFixed(0) + "°C" : null],
      ["Thermal headroom", p.cpu.temp_c != null && p.cpu.tjmax_c != null ? (p.cpu.tjmax_c - p.cpu.temp_c).toFixed(0) + "°C to TjMax " + p.cpu.tjmax_c.toFixed(0) + "°C" : null, "tjmax"],
      ["AC adapter", d.ac_online == null ? null : (d.ac_online ? "online — running on AC power" : "offline")],
      ["CPU governor", d.governor, "governor"],
      ["Energy performance preference", d.epp, "epp"],
      ["UPS", d.ups ? [d.ups.model || "UPS", d.ups.state, d.ups.percentage != null ? d.ups.percentage + "%" : null].filter(Boolean).join(" · ") : "none detected"],
    ]);
    if (p.power.length) h += p.power.map((b) =>
      `<div class="card"><h3>${esc(b.name)} — ${esc(b.status)}</h3>` +
      `<p>Charge ${esc(b.capacity_percent)}% · health ${b.health_percent != null ? b.health_percent.toFixed(0) + "%" : "—"} · cycles ${esc(b.cycle_count)}</p>` +
      `<p>${esc(b.manufacturer)} ${esc(b.model_name)}</p></div>`).join("");
    return h;
  },
  pci(p) {
    const order = ["Graphics & Displays", "Networking", "Storage", "Audio & Multimedia", "Peripherals & USB", "System & Bridges", "Other"];
    const groups = new Map();
    for (const d of p.pci) { const k = d.category || "Other"; if (!groups.has(k)) groups.set(k, []); groups.get(k).push(d); }
    let h = "";
    for (const k of order) {
      if (!groups.has(k)) continue;
      h += `<h3>${esc(k)} (${groups.get(k).length})</h3><table class="kv">${groups.get(k).map((d) =>
        `<tr><th class="mono">${esc(d.id)}</th><td>${esc(d.description)}${d.driver ? " · " + esc(d.driver) : ""}</td></tr>`).join("")}</table>`;
    }
    return h || `<p class="empty">No PCI devices detected.</p>`;
  },
  usb(p) {
    const open = ["Peripherals & Input", "Networking & Wireless", "Storage", "Other Devices"];
    const groups = new Map();
    for (const d of p.usb) { const k = d.category || "Other Devices"; if (!groups.has(k)) groups.set(k, []); groups.get(k).push(d); }
    const table = (items) => `<table class="kv">${items.map((d) =>
      `<tr><th class="mono">${esc(d.id)}</th><td>${esc(d.description)}</td></tr>`).join("")}</table>`;
    let h = "";
    for (const k of open) if (groups.has(k)) h += `<h3>${esc(k)} (${groups.get(k).length})</h3>` + table(groups.get(k));
    if (groups.has("Internal Controllers"))
      h += `<details class="card"><summary>Internal Controllers (${groups.get("Internal Controllers").length}) — root hubs & host controllers</summary>` +
        table(groups.get("Internal Controllers")) + `</details>`;
    return h || `<p class="empty">No USB devices detected.</p>`;
  },
  software(p) {
    let h = kvTable([
      ["Packages", p.software.packages_count != null ? p.software.packages_count + " (" + o(p.software.package_manager) + ")" : null],
      ["Flatpaks", p.software.flatpaks_count],
      ["Snaps", p.software.snaps.length],
      ["Docker images", dockerText(p)],
      ["glibc", p.software.glibc], ["libstdc++", p.software.gcc_runtime],
      ["Kernel modules loaded", p.software.loaded_modules_count],
      ["Services running", p.software.running_services_count],
      ["Last full upgrade", p.software.last_upgrade],
      ["Modules (sample)", p.software.loaded_modules_sample.join(", ")],
    ]);
    const details = (title, items, render) => items.length
      ? `<details class="card"><summary>${esc(title)} (${items.length})</summary>` + items.map(render).join("") + `</details>` : "";
    h += `<h3>Installed software, by type</h3>`;
    h += details("System packages — explicitly installed", p.software.explicit_packages,
      (x) => `<p class="mono">${esc(x.name)} ${esc(x.version)}</p>`);
    h += details("Flatpaks", p.software.flatpaks_list,
      (x) => `<p class="mono">${esc(x.name)} ${esc(x.version)}</p>`);
    h += details("Snaps", p.software.snaps,
      (x) => `<p class="mono">${esc(x.name)} ${esc(x.version)}${x.publisher ? " · " + esc(x.publisher) : ""}</p>`);
    h += details("Docker images", p.software.docker_images,
      (x) => `<p class="mono">${esc(x.repository_tag)} ${esc(x.size)}</p>`);
    if (!p.software.explicit_packages.length && !p.software.flatpaks_list.length && !p.software.snaps.length && !p.software.docker_images.length)
      h += `<p class="empty">No itemized software found.</p>`;
    return h;
  },
  accounts(p) {
    let h = kvTable([
      ["Secure Boot", p.system.secure_boot],
      ["Firewall", p.security.firewall_active == null ? null : (p.security.firewall_active ? "active (" + o(p.security.firewall_service) + ")" : "NONE ACTIVE")],
      ["Encrypted (LUKS) volumes", String(p.security.luks_volumes), "luks"],
      ["SSH server running", p.security.sshd_active == null ? null : (p.security.sshd_active ? "yes" : "no")],
    ]);
    h += `<h3>User accounts</h3>` + (p.accounts.length
      ? `<table class="kv">${p.accounts.map((a) =>
          `<tr><th>${esc(a.name)}${a.admin ? ' <span class="chip">admin</span>' : ""}</th>` +
          `<td>uid ${a.uid} · ${esc(a.shell)}<br><span class="hint">${esc(a.last_login || "no login recorded")}</span></td></tr>`).join("")}</table>`
      : `<p class="empty">No accounts found.</p>`);
    if (p.printers.length) {
      h += `<h3>Printers</h3>` + p.printers.map((pr) =>
        `<div class="flag"><span class="dot green"></span><div><strong>${esc(pr.name)}</strong>${pr.is_default ? ' <span class="chip">default</span>' : ""}<span class="hint">${esc(pr.state)}</span></div></div>`).join("");
    }
    if (p.bluetooth.length) {
      h += `<h3>Paired Bluetooth</h3>` + p.bluetooth.map((b) =>
        `<div class="flag"><span class="dot info"></span><div><strong>${esc(b.name || "device")}</strong><span class="hint mono">${esc(b.address)}</span></div></div>`).join("");
    }
    return h;
  },
  about(p) {
    let h = `<div class="about-badge">◉</div>` +
      `<h3 class="about-name">Linux System Profiler</h3>` +
      `<p class="empty">Version ${esc(p.tool_version)} · profile schema v${esc(p.schema_version)}</p>` +
      `<p>Your whole machine at a glance — hardware, firmware, software, health, and updates — ` +
      `in the spirit of Apple's System Information and Belarc Advisor, built for Linux.</p>` +
      kvTable([
        ["Collects", "CPU, memory, storage + SMART, displays (EDID), network, GPU, power, PCI, USB, audio, printers, Bluetooth, accounts, security posture, snaps, Docker images"],
        ["Updates", "Check-only by default: system (pacman), firmware (LVFS/fwupd), Flatpak. Applying is always an explicit, elevated action — never silent."],
        ["Speed tests", "On-demand disk (direct I/O) and network (Cloudflare) tests with live gauges. They never run on their own."],
        ["Privacy", "Everything is gathered locally on this machine. The only network traffic is update checks and the speed test. Copy & Share's sanitized report strips serials, MACs, private addresses and usernames before anything leaves the app."],
        ["Dependencies", "Missing probe tools (lsusb, smartctl, lspci, fwupdmgr) are detected and can be installed with one click — the app fixes its own dependencies."],
        ["License", "MIT"],
        ["Fonts", "Inter (UI) · Fraunces (display) · Newsreader (reading) · Victor Mono (data) — all SIL Open Font License 1.1. Fraunces, Newsreader and Victor Mono are bundled with the app; Inter comes from your system. Set in the Vizsla OS type program."],
      ]);
    h += `<h3>Help &amp; feedback</h3>
      <p>Found something wrong, or wishing for a feature? File it where the work happens. The bug form arrives pre-filled with this machine's basics — versions and hardware, never serials or addresses.</p>
      <div class="row" style="display:flex;gap:.5rem;flex-wrap:wrap;margin:.625rem 0">
        <button type="button" class="pill secondary" data-feedback="bug">Report a Bug</button>
        <button type="button" class="pill secondary" data-feedback="feature">Request a Feature</button>
      </div>
      <p class="empty">github.com/garrisonr/linux-system-profiler/issues</p>`;
    h += `<p class="empty">Profile generated ${esc(p.generated_at)}</p>`;
    return h;
  },
  themes(p) {
    // The gallery's own home since v0.8.5 (it was buried in About). The
    // sidebar footer's distro dropdown stays as the quick-switch mirror.
    let h = `<p>Themes are organized by <em>design language</em>. Distros that share one (say, Debian and Fedora, both GNOME/Adwaita) are covered by the same theme — the note under each says which family it belongs to. Auto matches this machine's distro. A theme changes the colors and materials only — the Vizsla fonts and button shapes stay the same in every theme.</p>
      <div class="theme-grid">` +
      DISTROS.map(([name, engine, note], i) => {
        const pv = ENGINE_PREVIEW[engine];
        return `<button type="button" class="theme-card" data-engine-pick="${engine}" style="--pv-bg:${pv[0]};--pv-ink:${pv[1]};--pv-accent:${pv[2]}">` +
          `<span class="sw"></span><span class="tc-main"><span class="tc-name">${esc(name)}</span>` +
          (note ? `<span class="tc-note">${esc(note)}</span>` : `<span class="tc-note">${esc(ENGINES[engine])}</span>`) +
          `</span></button>`;
      }).join("") + `</div>` +
      `<p class="empty">Profile generated ${esc(p.generated_at)}</p>`;
    return h;
  },
  updates(p) {
    /* v0.8.11: a group only earns an Apply button when something in it
       can actually install. Installable = status is not "blocked"
       (model.rs UpdateStatus: ready / kept_back / phased / held all
       travel the safe path — v0.8.9's post-apply report classifies
       what remains; kept_back also has its own full-upgrade card
       below). A blocked-only group (e.g. containerd alone on the
       Dell) used to render a big blue Apply that could never succeed:
       now NO button, one plain sentence instead. The status note is
       a banner (.update-note), never prose wrapped in a chip. */
    const group = (title, items, key, danger, hintKey) => {
      const installable = items.filter((u) => u.status !== "blocked");
      return `<div class="card"><h3>${esc(title)}${hintKey ? hintBtn(hintKey) : ""} — ${items.length} available</h3>` +
      (items.length
        ? items.slice(0, 60).map((u) => {
          /* v0.8.9: versions AND the plain-English state — the note
             used to REPLACE the versions, and apt rows carried no
             versions at all ("None -> None" in the text profile). */
          const ver = u.current && u.available ? ` ${esc(u.current)} → ${esc(u.available)}` : "";
          return `<p class="mono">${esc(u.name)}${ver}</p>` +
            (u.note ? `<p class="update-note">${esc(u.note)}</p>` : "");
        }).join("") +
          (installable.length
            ? `<div class="row"><button class="pill ${danger ? "danger" : "primary"}" data-apply="${key}" type="button">Apply ${esc(title)} updates</button></div>`
            : `<p>Nothing here can install yet — each update above says what it's waiting for. The apply button comes back when one of them can.</p>`)
        : `<p>Up to date.</p>`) + `</div>`;
    };
    const sysKey = { pacman: "system-arch", apt: "system-apt", dnf: "system-dnf" }[
      (p.updates.system_updates[0] || {}).source] || "system-arch";
    let h = `<div class="row"><button class="pill secondary" data-check-updates type="button">Check for updates</button>` +
      `<span class="empty" data-check-status></span></div>`;
    /* Recount (v0.8.10): the counts say when they were counted —
       a snapshot's age is part of its truth. Past 15 minutes the
       line takes the stale styling; opening this view with counts
       older than 10 minutes recounts them automatically. */
    const asOf = updatesAsOf(p.updates);
    if (asOf) {
      h += `<p class="${asOf.old ? "stale-note" : "empty"}">${asOf.old ? "⚠ " : ""}Counts as of ${esc(asOf.when)} — counted ${esc(asOf.ago)}.${asOf.old ? " This tab recounts on its own when the counts get old; “Check for updates” also refreshes the package lists." : ""}</p>`;
    }
    // Stale counts SAY they're stale (v0.8.8): the launch scan reads
    // the lists on disk; when they're older than apt's daily refresh
    // rhythm the banner names it instead of letting old counts pose
    // as current. The initial load already tried the refresh path
    // once (autoRefreshIfStale) — if this banner is still here, that
    // refresh didn't complete and Check is the manual route.
    if (p.updates.metadata_stale) {
      h += `<p class="stale-note">⚠ These counts may be out of date — the package lists were last refreshed ` +
        `${p.updates.metadata_last_refresh ? esc(new Date(p.updates.metadata_last_refresh).toLocaleString()) : "never"}. ` +
        `Check for updates refreshes the lists first, then counts.</p>`;
    }
    h += group("System packages", p.updates.system_updates, sysKey, false,
      p.updates.system_updates.some((u) => u.status === "phased") ? "phased" : null);
    /* v0.8.9: kept-back updates get their remedy on the same page —
       the safe apply above will leave them behind every single time,
       and until now nothing on screen said why or what could. */
    const kept = p.updates.system_updates.filter((u) => u.status === "kept_back");
    if (kept.length) {
      h += `<div class="card"><h3>${kept.length} update${kept.length === 1 ? "" : "s"} waiting on new companion packages</h3>` +
        `<p>These updates bring new packages along with them, and the safe update isn't allowed to install new packages — so it leaves them behind every time. A full upgrade can install them, together with the companions they need. You'll see the full plan before anything runs.</p>` +
        `<div class="row"><button class="pill primary" data-full-upgrade type="button">Install these too (full upgrade)</button></div></div>`;
    }
    h += group("Firmware (LVFS)", p.updates.firmware_updates, "firmware", true, "lvfs");
    h += group("Flatpak", p.updates.flatpak_updates, "flatpak", false);
    if (p.updates.notes.length) h += p.updates.notes.map((n) => `<p class="empty">${esc(n)}</p>`).join("");
    return h;
  },
};


/* ================= v0.9.0 — Live / Processes / History / Services / Startup
   The Rust sampler owns the clock and the data; these views only paint
   what the "live-sample" events hand them. No JS polling, ever. */

let liveSample = null;
const liveTrail = []; // display-only trail of aggregates for the Live graphs
let procFilter = "", procTree = false;
let procSort = { key: "cpu_pct", dir: -1 };
const forceReady = new Set(); // "pid:start" pairs that ignored a polite quit
let histPoints = [];
let svcResult = null;

const rate = (b) => b == null ? "—" : b >= 1048576 ? (b / 1048576).toFixed(1) + " MiB/s"
  : b >= 1024 ? (b / 1024).toFixed(0) + " KiB/s" : b + " B/s";
const pct1 = (v) => (Math.round(v * 10) / 10) + "%";

function sparkSvg(points, get, maxV, label) {
  // points: [{ts, ...}] — x from REAL timestamps; a silence longer than
  // 6 s breaks the line (a gap stays a gap, never a fake bridge).
  if (points.length < 2) return `<p class="empty">Collecting…</p>`;
  const t0 = points[0].ts, t1 = points[points.length - 1].ts;
  const span = Math.max(1, t1 - t0);
  const max = maxV || Math.max(1, ...points.map((pt) => get(pt) || 0)) * 1.15;
  let d = "", started = false, prevTs = null;
  for (const pt of points) {
    const v = get(pt);
    if (v == null) { started = false; prevTs = null; continue; }
    const x = ((pt.ts - t0) / span * 300).toFixed(1);
    const y = (56 - Math.min(1, v / max) * 50).toFixed(1);
    if (!started || (prevTs != null && pt.ts - prevTs > 6)) { d += `M${x},${y} `; started = true; }
    else d += `L${x},${y} `;
    prevTs = pt.ts;
  }
  return `<svg class="spark" viewBox="0 0 300 60" preserveAspectRatio="none" role="img" aria-label="${esc(label)}"><path d="${d}" fill="none" stroke="var(--accent)" stroke-width="1.6"/></svg>`;
}

listen("live-sample", (e) => {
  liveSample = e.payload;
  liveTrail.push({ ts: liveSample.ts, cpu_pct: liveSample.cpu_pct,
    mem_pct: liveSample.mem_total_kb ? liveSample.mem_used_kb / liveSample.mem_total_kb * 100 : 0,
    disk: liveSample.disk_read_bps + liveSample.disk_write_bps,
    net: liveSample.net_rx_bps + liveSample.net_tx_bps });
  if (liveTrail.length > 300) liveTrail.shift();
  if (current === "live") renderLiveBody();
  else if (current === "processes") renderProcBody();
}).catch(() => {});

function liveHooks(id) {
  invoke("set_live_lane", { fast: ["live", "processes", "history"].includes(id) }).catch(() => {});
  if (id === "live") renderLiveBody();
  if (id === "processes") renderProcBody();
  if (id === "history") fillHistory();
  if (id === "services") fillServices();
  if (id === "startup") fillStartup();
  if (id === "updates") fillFailureCards();
}

/* ---------------- Waiting Vizsla (update progress sheet) ----------------
   An original little hound in the icon family's stroke language. She
   runs while an update runs, sits beside the error when one fails, and
   under reduced-motion the global rule stills her — she just sits. */
function vizShow() { const v = $("logVizsla"); if (v) { v.hidden = false; v.classList.remove("sitting"); } }
function vizHide() { const v = $("logVizsla"); if (v) { v.hidden = true; v.classList.remove("sitting"); } }
function vizSit() { const v = $("logVizsla"); if (v) { v.hidden = false; v.classList.add("sitting"); } }

/* ---------------- Live ---------------- */
VIEWS.live = () => `<div id="liveBody"><p class="empty">Collecting the first look…</p></div>`;

function renderLiveBody() {
  const el = $("liveBody");
  if (!el || !liveSample) return;
  const s = liveSample;
  const memPct = s.mem_total_kb ? s.mem_used_kb / s.mem_total_kb * 100 : 0;
  const top = [...s.procs].sort((a, b) => b.cpu_pct - a.cpu_pct).slice(0, 5);
  const cores = s.cores.map((c, i) =>
    `<div class="core-row"><span class="core-name">CPU ${i}${c.kind ? ` <span class="chip">${esc(c.kind)}</span>` : ""}</span>` +
    `<span class="core-bar"><span class="core-fill" style="width:${Math.min(100, c.pct).toFixed(0)}%"></span></span>` +
    `<span class="core-val">${pct1(c.pct)}${c.freq_mhz ? ` · ${c.freq_mhz} MHz` : ""}</span></div>`).join("");
  const temps = s.temps.length
    ? s.temps.map(([n, v]) => `${esc(n)} ${v.toFixed(0)}°C`).join(" · ")
    : "No temperature sensors are exposed on this machine.";
  el.innerHTML =
    `<div class="live-grid">
      <div class="card"><h3>CPU — ${pct1(s.cpu_pct)}</h3>${sparkSvg(liveTrail, (pt) => pt.cpu_pct, 100, "CPU history")}
        <p>Load ${s.load1.toFixed(2)} · up ${Math.floor(s.uptime_s / 3600)}h ${Math.floor(s.uptime_s % 3600 / 60)}m</p></div>
      <div class="card"><h3>Memory — ${pct1(memPct)}</h3>${sparkSvg(liveTrail, (pt) => pt.mem_pct, 100, "Memory history")}
        <p>${gb(s.mem_used_kb * 1024)} of ${gb(s.mem_total_kb * 1024)} used${s.swap_total_kb ? ` · swap ${gb(s.swap_used_kb * 1024)} of ${gb(s.swap_total_kb * 1024)}` : " · no swap"}</p></div>
      <div class="card"><h3>Disk — ${rate(s.disk_read_bps + s.disk_write_bps)}</h3>${sparkSvg(liveTrail, (pt) => pt.disk, null, "Disk history")}
        <p>Read ${rate(s.disk_read_bps)} · Write ${rate(s.disk_write_bps)}</p></div>
      <div class="card"><h3>Network — ${rate(s.net_rx_bps + s.net_tx_bps)}</h3>${sparkSvg(liveTrail, (pt) => pt.net, null, "Network history")}
        <p>Down ${rate(s.net_rx_bps)} · Up ${rate(s.net_tx_bps)}</p></div>
    </div>
    <div class="card"><h3>Per core</h3>${cores || `<p class="empty">Per-core data isn't available.</p>`}</div>
    <div class="card"><h3>Busiest right now</h3>
      ${top.map((pr) => `<p class="mono">${esc(pr.name)} — ${pct1(pr.cpu_pct)} cpu · ${gb(pr.rss_kb * 1024)}</p>`).join("") || `<p class="empty">Nothing to show yet.</p>`}
      <p class="empty">Heat & power: ${temps}${s.power_w != null ? ` · package power ${s.power_w.toFixed(1)} W` : " · Power draw isn't exposed on this machine."}</p>
      <p class="empty">Profiler itself: PID ${s.self_pid} · each look costs about ${s.sampler_ms.toFixed(1)} ms · ${s.procs.length} processes running.</p>
    </div>`;
}

/* ---------------- Processes ---------------- */
VIEWS.processes = () =>
  `<div class="row proc-controls">
     <input type="search" class="text-input" id="procFilter" placeholder="Filter by name, user or PID…" value="${esc(procFilter)}">
     <button type="button" class="pill ${procTree ? "primary" : "secondary"}" id="procTreeBtn">${procTree ? "Tree: on" : "Tree: off"}</button>
     <span class="empty" id="procCount"></span>
   </div>
   <div id="procBody"><p class="empty">Collecting the first look…</p></div>`;

function sortedProcs(list) {
  const k = procSort.key, d = procSort.dir;
  return [...list].sort((a, b) => {
    const av = a[k], bv = b[k];
    if (typeof av === "string") return d * av.localeCompare(bv);
    return d * ((av ?? -1) - (bv ?? -1));
  });
}

function procRow(pr, depth) {
  const self = liveSample && pr.pid === liveSample.self_pid;
  const key = `${pr.pid}:${pr.start_ticks}`;
  const force = forceReady.has(key);
  return `<tr>
    <td style="padding-left:${0.5 + (depth || 0) * 1.1}rem">${esc(pr.name)}${self ? ` <span class="chip">Profiler itself</span>` : ""}<span class="hint-line">${esc(pr.cmdline)}</span></td>
    <td class="num">${pr.pid}</td><td>${esc(pr.user)}</td>
    <td class="num">${pct1(pr.cpu_pct)}</td>
    <td class="num">${gb(pr.rss_kb * 1024)}</td>
    <td class="num">${pr.read_bps == null && pr.write_bps == null ? "—" : rate((pr.read_bps || 0) + (pr.write_bps || 0))}</td>
    <td>${esc(pr.state)}</td>
    <td>${self ? "" : `<button type="button" class="mini-btn" data-end="${pr.pid}:${pr.start_ticks}" data-name="${esc(pr.name)}">${force ? "Force end" : "End"}</button>`}</td></tr>`;
}

function renderProcBody() {
  const el = $("procBody");
  if (!el || !liveSample) return;
  let list = liveSample.procs;
  const q = procFilter.trim().toLowerCase();
  if (q) list = list.filter((pr) => pr.name.toLowerCase().includes(q) || pr.user.toLowerCase().includes(q)
    || String(pr.pid) === q || pr.cmdline.toLowerCase().includes(q));
  const count = $("procCount");
  if (count) count.textContent = `${list.length} of ${liveSample.procs.length} processes`;
  const head = `<table class="proc-table"><thead><tr>
    ${[["name", "Name"], ["pid", "PID"], ["user", "User"], ["cpu_pct", "CPU"], ["rss_kb", "Memory"], ["disk", "Disk"], ["state", "State"]].map(([k, label]) =>
      `<th data-sort="${k}" class="${["pid", "cpu_pct", "rss_kb", "disk"].includes(k) ? "num" : ""}">${label}${procSort.key === k ? (procSort.dir < 0 ? " ↓" : " ↑") : ""}</th>`).join("")}<th></th></tr></thead><tbody>`;
  let rows = "";
  if (procTree && !q) {
    const byParent = new Map();
    const ids = new Set(list.map((pr) => pr.pid));
    for (const pr of list) {
      const parent = ids.has(pr.ppid) && pr.ppid !== pr.pid ? pr.ppid : 0;
      if (!byParent.has(parent)) byParent.set(parent, []);
      byParent.get(parent).push(pr);
    }
    const walk = (ppid, depth, seen) => {
      for (const pr of sortedProcs(byParent.get(ppid) || [])) {
        if (seen.has(pr.pid)) continue;
        seen.add(pr.pid);
        rows += procRow(pr, depth);
        walk(pr.pid, depth + 1, seen);
      }
    };
    walk(0, 0, new Set());
  } else {
    rows = sortedProcs(list).map((pr) => procRow(pr, 0)).join("");
  }
  el.innerHTML = head + rows + `</tbody></table>`;
}

async function endProcess(btn) {
  const [pidS, startS] = btn.dataset.end.split(":");
  const pid = Number(pidS), start = Number(startS);
  const name = btn.dataset.name || `process ${pid}`;
  const force = forceReady.has(btn.dataset.end);
  const ok = await askConfirm(force ? `Force-end ${name}?` : `End ${name}?`,
    force
      ? `${name} (PID ${pid}) didn't quit when asked. Force-ending kills it on the spot — unsaved work in it is lost.`
      : `Profiler will ask ${name} (PID ${pid}) to quit politely first. If it refuses, you can force it from this list.`,
    force ? "Force end" : "End process", true);
  if (!ok) return;
  try {
    const msg = await invoke("end_process", { pid, startTicks: start, force });
    showToast(msg);
    if (!force) forceReady.add(btn.dataset.end);
    else forceReady.delete(btn.dataset.end);
  } catch (e) {
    showToast(String(e));
    forceReady.delete(btn.dataset.end);
  }
}

/* ---------------- History ---------------- */
VIEWS.history = () =>
  `<div class="card"><h3>What just happened</h3>
   <p>This covers the time System Profiler has been open — up to the last 30 minutes. Drag the slider back to see what the machine was doing then, and which processes were busiest at that moment.</p>
   <div class="row"><input type="range" class="hist-slider" id="histSlider" min="0" max="0" value="0">
   <button type="button" class="pill secondary" id="histRefresh">Refresh</button>
   <button type="button" class="pill primary" data-goto="live">Back to live</button></div>
   <div id="histBody"><p class="empty">Reading the record…</p></div></div>`;

async function fillHistory() {
  try { histPoints = await invoke("get_history"); } catch (e) { histPoints = []; }
  const slider = $("histSlider");
  if (slider) { slider.max = Math.max(0, histPoints.length - 1); slider.value = slider.max; }
  renderHistPoint(histPoints.length - 1);
}

function renderHistPoint(idx) {
  const el = $("histBody");
  if (!el) return;
  if (!histPoints.length || idx < 0) {
    el.innerHTML = `<p class="empty">Nothing recorded yet — the record starts when Profiler opens. Give it a few seconds.</p>`;
    return;
  }
  const pt = histPoints[idx];
  const when = new Date(pt.ts * 1000).toLocaleTimeString();
  const ago = Math.max(0, Math.round(histPoints[histPoints.length - 1].ts - pt.ts));
  const sparkAt = (get, maxV, label) => {
    const upto = histPoints.slice(0, idx + 1);
    return sparkSvg(upto, get, maxV, label);
  };
  el.innerHTML =
    `<p><strong>${esc(when)}</strong> — ${ago === 0 ? "the latest look" : `${ago} seconds before the latest look`}</p>
     <div class="live-grid">
       <div><h4>CPU — ${pct1(pt.cpu_pct)}</h4>${sparkAt((q) => q.cpu_pct, 100, "CPU")}</div>
       <div><h4>Memory — ${pct1(pt.mem_pct)}</h4>${sparkAt((q) => q.mem_pct, 100, "Memory")}</div>
       <div><h4>Disk — ${rate(pt.disk_read_bps + pt.disk_write_bps)}</h4>${sparkAt((q) => q.disk_read_bps + q.disk_write_bps, null, "Disk")}</div>
       <div><h4>Network — ${rate(pt.net_rx_bps + pt.net_tx_bps)}</h4>${sparkAt((q) => q.net_rx_bps + q.net_tx_bps, null, "Network")}</div>
     </div>
     <h4>Busiest at that moment</h4>
     ${pt.top.map((tp) => `<p class="mono">${esc(tp.name)} (PID ${tp.pid}) — ${pct1(tp.cpu_pct)} cpu · ${gb(tp.rss_kb * 1024)}</p>`).join("") || `<p class="empty">No process data at that moment.</p>`}`;
}

/* ---------------- Services ---------------- */
VIEWS.services = () =>
  `<div id="svcResult"></div><div id="svcBody"><p class="empty">Reading services…</p></div>`;

async function fillServices() {
  let list;
  try { list = await invoke("list_services"); }
  catch (e) {
    const el = $("svcBody");
    if (el) el.innerHTML = `<p class="empty">${esc(String(e))}</p>`;
    return;
  }
  const el = $("svcBody");
  if (!el || current !== "services") return;
  const res = $("svcResult");
  if (res) res.innerHTML = svcResult
    ? `<div class="card"><p>${esc(svcResult)}</p></div>` : "";
  el.innerHTML = `<table class="proc-table"><thead><tr><th>Service</th><th>State</th><th>What it is</th><th></th></tr></thead><tbody>` +
    list.map((sv) => {
      const running = sv.active === "active";
      const actions = running
        ? `<button type="button" class="mini-btn" data-svc="stop:${esc(sv.name)}">Stop</button> <button type="button" class="mini-btn" data-svc="restart:${esc(sv.name)}">Restart</button>`
        : `<button type="button" class="mini-btn" data-svc="start:${esc(sv.name)}">Start</button>`;
      return `<tr><td class="mono">${esc(sv.name.replace(/\.service$/, ""))}</td>
        <td>${running ? `<span class="chip">running</span>` : esc(sv.active)}</td>
        <td>${esc(sv.description)}</td><td>${actions}</td></tr>`;
    }).join("") + `</tbody></table>`;
}

async function serviceAction(btn) {
  const [action, name] = btn.dataset.svc.split(/:(.+)/).filter(Boolean);
  const verb = { start: "Start", stop: "Stop", restart: "Restart" }[action];
  const ok = await askConfirm(`${verb} ${name.replace(/\.service$/, "")}?`,
    `This asks the system to ${action} the ${name} service. It needs your admin password, and the result — good or bad — stays on this page.`,
    verb, action !== "start");
  if (!ok) return;
  btn.disabled = true;
  try { svcResult = await invoke("service_action", { name, action }); }
  catch (e) { svcResult = String(e); }
  fillServices();
}

/* ---------------- Startup ---------------- */
VIEWS.startup = () =>
  `<div class="card"><h3>What starts with your session</h3>
   <p>Everything below launches on its own when you sign in. This version is a window, not a switchboard — turning entries off safely (with a way back) is coming in a later version.</p></div>
   <div id="startupBody"><p class="empty">Looking…</p></div>`;

async function fillStartup() {
  let list;
  try { list = await invoke("list_startup"); } catch (e) { list = []; }
  const el = $("startupBody");
  if (!el || current !== "startup") return;
  if (!list.length) {
    el.innerHTML = `<p class="empty">Nothing is set to start with your session — a clean desk.</p>`;
    return;
  }
  el.innerHTML = list.map((en) =>
    `<div class="card"><h3>${esc(en.name)}${en.hidden ? ` <span class="chip">switched off</span>` : ""}</h3>
     <p>${esc(en.kind)} · from ${esc(en.source)}</p>
     ${en.exec ? `<p class="mono">${esc(en.exec)}</p>` : ""}</div>`).join("");
}

/* ---------------- Updates failure cards ---------------- */
async function fillFailureCards() {
  let fails;
  try { fails = await invoke("get_update_failures"); } catch (e) { return; }
  const body = $("detailBody");
  if (!body || current !== "updates" || !fails.length) return;
  const old = $("failCards");
  if (old) old.remove();
  const div = document.createElement("div");
  div.id = "failCards";
  div.innerHTML = fails.map((f) => {
    const when = new Date(f.when_unix * 1000).toLocaleString();
    return `<div class="card fail-card"><h3>${esc(f.label)} update didn't finish</h3>
      <p>${esc(f.headline)}</p>
      <p class="empty">Failed ${esc(when)}. This note stays until the ${esc(f.label.toLowerCase())} update succeeds or you dismiss it.</p>
      <details><summary>Details — what the system said</summary><pre class="log">${esc(f.detail)}</pre></details>
      <div class="row"><button type="button" class="pill primary" data-fail-retry="${esc(f.group)}:${esc(f.kind)}">Try again</button>
      <button type="button" class="pill secondary" data-fail-dismiss="${esc(f.group)}">Dismiss</button></div></div>`;
  }).join("");
  body.prepend(div);
  div.querySelectorAll("[data-fail-dismiss]").forEach((b) => b.addEventListener("click", async () => {
    await invoke("dismiss_update_failure", { group: b.dataset.failDismiss }).catch(() => {});
    div.remove();
  }));
  div.querySelectorAll("[data-fail-retry]").forEach((b) => b.addEventListener("click", () => {
    const [group, kind] = b.dataset.failRetry.split(":");
    if (kind === "full-upgrade") applyFullUpgrade();
    else applyGroup(group);
  }));
}

/* ---------------- wireDetail wrap: the new views' controls ---------------- */
const _wireDetailV090 = wireDetail;
wireDetail = function () {
  _wireDetailV090();
  if (current === "processes") {
    const f = $("procFilter");
    if (f) f.addEventListener("input", () => { procFilter = f.value; renderProcBody(); });
    const tb = $("procTreeBtn");
    if (tb) tb.addEventListener("click", () => { procTree = !procTree; select("processes"); });
    const body = $("procBody");
    if (body) body.addEventListener("click", (ev) => {
      const th = ev.target.closest("[data-sort]");
      if (th) {
        const k = th.dataset.sort;
        if (k === "disk") return;
        if (procSort.key === k) procSort.dir *= -1; else procSort = { key: k, dir: k === "name" || k === "user" ? 1 : -1 };
        renderProcBody();
        return;
      }
      const btn = ev.target.closest("[data-end]");
      if (btn) endProcess(btn);
    });
  }
  if (current === "history") {
    const slider = $("histSlider");
    if (slider) slider.addEventListener("input", () => renderHistPoint(Number(slider.value)));
    const rf = $("histRefresh");
    if (rf) rf.addEventListener("click", fillHistory);
  }
  if (current === "services") {
    const body = $("svcBody");
    if (body) body.addEventListener("click", (ev) => {
      const btn = ev.target.closest("[data-svc]");
      if (btn) serviceAction(btn);
    });
  }
};

/* ---------------- navigation ---------------- */
function renderNav() {
  const counts = { storage: P.storage.length, pci: P.pci.length, usb: P.usb.length,
    network: P.network.length, updates: P.updates.system_updates.length + P.updates.firmware_updates.length + P.updates.flatpak_updates.length };
  $("tree").innerHTML = SECTIONS.map(([id, label]) =>
    `<button type="button" data-section="${id}" aria-current="${id === current ? "page" : "false"}">${esc(label)}` +
    (counts[id] ? `<span class="count">${counts[id]}</span>` : "") + `</button>`).join("");
  document.querySelectorAll("[data-section]").forEach((b) =>
    b.addEventListener("pointerdown", () => select(b.dataset.section)));
}

function wireDetail() {
  document.querySelectorAll("[data-check-updates]").forEach((b) =>
    b.addEventListener("click", () => checkUpdates(b)));
  document.querySelectorAll("[data-apply]").forEach((b) =>
    b.addEventListener("click", () => applyGroup(b.dataset.apply)));
  document.querySelectorAll("[data-full-upgrade]").forEach((b) =>
    b.addEventListener("click", () => applyFullUpgrade()));
  document.querySelectorAll("[data-speed]").forEach((b) =>
    b.addEventListener("click", () => runSpeed(b)));
  document.querySelectorAll("[data-elevate]").forEach((b) =>
    b.addEventListener("click", async () => {
      b.disabled = true; b.textContent = "Waiting for password prompt…";
      try {
        const r = await invoke("elevated_dmi");
        P.memory.modules = r.modules;
        if (r.serial_number) P._dmiSerial = r.serial_number;
        showToast(r.modules.length ? `Unlocked — ${r.modules.length} memory modules` : "Unlocked, but no modules reported");
        select("memory");
      } catch (e) {
        showToast("Unlock failed: " + String(e));
        b.disabled = false; b.textContent = "Unlock with admin…";
      }
    }));
  document.querySelectorAll("[data-goto]").forEach((el) => {
    el.addEventListener("click", () => select(el.dataset.goto));
    el.addEventListener("keydown", (e) => { if (e.key === "Enter" || e.key === " ") { e.preventDefault(); select(el.dataset.goto); } });
  });
  document.querySelectorAll("[data-engine-pick]").forEach((b) =>
    b.addEventListener("click", () => {
      const v = b.dataset.enginePick;
      try { localStorage.setItem("lsp-theme", v); } catch (e) {}
      themeControl.set(v);
      applyTheme(v, P);
      showToast("Theme: " + ENGINES[v]);
    }));
  document.querySelectorAll("[data-feedback]").forEach((b) =>
    b.addEventListener("click", () => openFeedback(b.dataset.feedback)));
  document.querySelectorAll(".gauge").forEach((el) =>
    gaugeInit(el, el.dataset.gauge === "disk" ? 2000 : 1000));
  if (gauges.size && !reducedMotion) requestAnimationFrame(gaugeStep);
}

function select(id) {
  closeGloss();
  current = id;
  const label = SECTIONS.find(([s]) => s === id)[1];
  $("detailTitle").textContent = label;
  $("detailSub").textContent = id === "overview" ? (P.system.os_name || "") : "";
  gauges.clear();
  $("detailBody").innerHTML = VIEWS[id](P);
  renderNav();
  renderRail();
  wireDetail();
  $("app").classList.add("show-detail");
  if (id === "updates") maybeAutoRecount();
  liveHooks(id);
}

function miniFlag(dot, html, goto) {
  return `<div class="flag"${goto ? ` data-goto="${goto}" role="button" tabindex="0"` : ""}><span class="dot ${dot}"></span><div>${html}</div></div>`;
}
function renderRail() {
  const title = $("railTitle"), body = $("railBody");
  let h = "";
  if (current === "overview" || current === "accounts") {
    title.textContent = "Health";
    h = P.health.map(flagHtml).join("");
    const u = P.updates;
    const n = u.system_updates.length + u.firmware_updates.length + u.flatpak_updates.length;
    h += `<h2 class="rail-sub">Updates</h2>` + miniFlag(n ? "amber" : "green",
      `${u.system_updates.length} system · ${u.firmware_updates.length} firmware · ${u.flatpak_updates.length} flatpak`, "updates");
  } else if (current === "cpu") {
    title.textContent = "Thermal";
    const t = P.cpu.temp_c, tj = P.cpu.tjmax_c;
    h = t != null
      ? miniFlag(t >= 90 ? "red" : t >= 75 ? "amber" : "green",
          `Package ${t.toFixed(0)}°C` + (tj != null ? `<span class="hint">thermal headroom ${(tj - t).toFixed(0)}°C (TjMax ${tj.toFixed(0)}°C)</span>` : ""))
      : `<p class="empty">No CPU temperature sensor reported.</p>`;
    h += miniFlag("info", `${o(P.cpu.cores_physical)} cores · ${o(P.cpu.threads_logical)} threads`);
  } else if (current === "memory") {
    title.textContent = "Memory";
    const tot = P.memory.total_kb, av = P.memory.available_kb;
    const pct = tot && av != null ? Math.round((1 - av / tot) * 100) : null;
    h = pct != null ? miniFlag(pct >= 90 ? "red" : pct >= 75 ? "amber" : "green", `${pct}% in use<span class="hint">${gb(av * 1024)} available of ${gb(tot * 1024)}</span>`)
      : `<p class="empty">No memory data.</p>`;
  } else if (current === "storage") {
    title.textContent = "Drive health";
    h = P.storage.map((d) => miniFlag(
      d.smart_health === "PASSED" ? "green" : d.smart_health ? "amber" : "info",
      `<strong>${esc(d.name)}</strong> ${esc(d.smart_health || "SMART —")}` +
      `<span class="hint">${[d.smart_temp_c != null ? d.smart_temp_c + "°C" : null,
        d.percentage_used != null ? "wear " + d.percentage_used + "%" : null,
        d.power_on_hours != null ? d.power_on_hours + " h" : null].filter(Boolean).join(" · ")}</span>`)).join("")
      || `<p class="empty">No drives detected.</p>`;
  } else if (current === "graphics") {
    title.textContent = "Displays";
    h = P.displays.map((d) => miniFlag("info", `<strong>${esc(d.name || d.connector)}</strong><span class="hint">${esc(d.resolution)}</span>`)).join("")
      + P.graphics.map((g) => miniFlag("info", `${esc(g.card)}<span class="hint">driver ${esc(g.driver_in_use)}</span>`)).join("");
  } else if (current === "network") {
    title.textContent = "Interfaces";
    const up = P.network.filter((n) => n.operstate === "up");
    h = miniFlag(up.length ? "green" : "amber", `${up.length} up · ${P.network.length} total`) +
      P.network.filter((n) => !n.is_virtual).map((n) => miniFlag(n.operstate === "up" ? "green" : "info",
        `<strong>${esc(n.name)}</strong> ${esc(n.kind)}<span class="hint">${esc((n.addresses || [])[0] || n.operstate)}</span>`)).join("");
  } else if (current === "power") {
    title.textContent = "Power";
    const d = P.power_detail || {};
    h = (P.cpu.temp_c != null ? miniFlag(P.cpu.temp_c >= 90 ? "red" : P.cpu.temp_c >= 75 ? "amber" : "green",
        `CPU ${P.cpu.temp_c.toFixed(0)}°C` + (P.cpu.tjmax_c != null ? `<span class="hint">headroom ${(P.cpu.tjmax_c - P.cpu.temp_c).toFixed(0)}°C (TjMax ${P.cpu.tjmax_c.toFixed(0)}°C)</span>` : "")) : "") +
      miniFlag(d.ac_online ? "green" : "info", d.ac_online == null ? "AC state unknown" : d.ac_online ? "On AC power" : "On battery / UPS") +
      (d.governor ? miniFlag("info", `Governor: ${esc(d.governor)}` + (d.epp ? `<span class="hint">EPP ${esc(d.epp)}</span>` : "")) : "") +
      (d.ups ? miniFlag("green", `UPS ${esc(d.ups.state || "")} ${d.ups.percentage != null ? d.ups.percentage + "%" : ""}`) : "");
  } else if (current === "pci" || current === "usb") {
    title.textContent = "Categories";
    const items = current === "pci" ? P.pci : P.usb;
    const counts = {};
    for (const d of items) counts[d.category || "Other"] = (counts[d.category || "Other"] || 0) + 1;
    h = Object.entries(counts).map(([k, v]) => miniFlag("info", `${esc(k)}<span class="hint">${v} devices</span>`)).join("");
  } else if (current === "software") {
    title.textContent = "Software";
    h = miniFlag("info", `${o(P.software.packages_count)} packages`) +
      miniFlag("info", `${P.software.snaps.length} snaps · ${P.software.docker_images.length} docker images`) +
      miniFlag("info", `${o(P.software.loaded_modules_count)} modules · ${o(P.software.running_services_count)} services`);
  } else if (current === "updates") {
    /* No totals here: the detail cards on this very screen already carry
       every count ("— N available"), and repeating them in the rail was
       the same numbers twice in one view (Dickles, 2026-10-14). Totals
       live on the Overview rail, where they signpost INTO this detail. */
    title.textContent = "Updates";
    h = miniFlag("info", `Applied per group, one at a time<span class="hint">each apply shows the password prompt first — nothing installs itself</span>`);
  } else {
    title.textContent = "About";
    h = miniFlag("info", `Version ${esc(P.tool_version)}<span class="hint">schema v${esc(P.schema_version)}</span>`);
  }
  body.innerHTML = h;
  body.querySelectorAll("[data-goto]").forEach((el) => {
    el.addEventListener("click", () => select(el.dataset.goto));
    el.addEventListener("keydown", (e) => { if (e.key === "Enter" || e.key === " ") { e.preventDefault(); select(el.dataset.goto); } });
  });
}

/* ---------------- dialogs, toast ---------------- */
/* What the log dialog is currently showing, for the Copy button: the
   action attempted and the plain-language headline. The body text is
   read live from the dialog at copy time. */
let logMeta = { action: "", headline: "" };

/* The shareable text block behind "Copy output": app + version, when,
   what was attempted, the plain-language headline, then the full raw
   output — formatted to paste straight into a bug report. Pure: every
   input arrives as a parameter (verified under node with fixtures). */
function composeOutputText(appLine, when, action, headline, body) {
  return [
    appLine,
    when,
    "Action: " + action,
    "",
    headline || "Completed.",
    "",
    "—— Output ——",
    body && body.trim() ? body : "(no output)",
  ].join("\n");
}

async function copyLogOutput(btn) {
  const text = composeOutputText(
    `Linux System Profiler v${(P && P.tool_version) || "?"}`,
    new Date().toLocaleString(),
    logMeta.action || $("logTitle").textContent,
    logMeta.headline || $("logStatus").textContent,
    $("logBody").textContent);
  const ok = await invoke("copy_text", { text }).catch(() => false);
  if (ok) {
    const old = btn.textContent;
    btn.textContent = "Copied";
    setTimeout(() => { btn.textContent = old; }, 1600);
  } else {
    showToast("Couldn't copy — select the output text and copy it manually.");
  }
}

function showLog(title, text) {
  logMeta = { action: title, headline: "" };
  $("logTitle").textContent = title;
  $("logBody").textContent = text;
  $("logSheet").showModal();
}
let toastTimer = null;
function showToast(msg) {
  const t = $("toast");
  t.textContent = msg;
  t.classList.add("show");
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => t.classList.remove("show"), 3600);
}

/* ---------------- actions ---------------- */
function askConfirm(title, body, okLabel, danger) {
  return new Promise((resolve) => {
    $("confirmTitle").textContent = title;
    $("confirmBody").textContent = body;
    const ok = $("confirmOk");
    ok.textContent = okLabel;
    ok.className = "pill " + (danger ? "danger" : "primary");
    const sheet = $("confirmSheet");
    const done = (v) => { sheet.close(); ok.onclick = null; $("confirmCancel").onclick = null; resolve(v); };
    ok.onclick = () => done(true);
    $("confirmCancel").onclick = () => done(false);
    sheet.oncancel = () => resolve(false);
    sheet.showModal();
  });
}

/* Recount (v0.8.10): an updates snapshot knows when it was counted.
   Render that age wherever its counts appear — "checked" is a
   timestamp, not a boolean. Returns null for snapshots that predate
   the stamp: their age is honestly unknown, so nothing is claimed. */
function updatesAsOf(u) {
  if (!u || !u.collected_at) return null;
  const t = Date.parse(u.collected_at);
  if (isNaN(t)) return null;
  const secs = Math.max(0, (Date.now() - t) / 1000);
  const when = new Date(t).toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });
  let ago;
  if (secs < 90) ago = "just now";
  else if (secs < 3600) ago = Math.floor(secs / 60) + " min ago";
  else if (secs < 48 * 3600) ago = Math.floor(secs / 3600) + " h ago";
  else ago = Math.floor(secs / 86400) + " d ago";
  return { when, ago, secs, old: secs > 900 };
}

/* Recount (v0.8.10): opening the Updates view with counts older
   than 10 minutes recounts them on the spot — unelevated, so no
   password prompt; refreshing the package LISTS stays behind the
   explicit Check. A window left open across an update run can no
   longer keep offering updates the system already applied (the
   phantom panel, 2026-10-18). Guarded to at most once per 10
   minutes; failures stay silent because the as-of line above the
   counts already tells the truth. */
let lastAutoRecount = 0;
async function maybeAutoRecount() {
  if (!P || !P.updates) return;
  const a = updatesAsOf(P.updates);
  if (!a || a.secs < 600) return;
  const now = Date.now();
  if (now - lastAutoRecount < 600000) return;
  lastAutoRecount = now;
  try {
    P.updates = await invoke("recount_updates");
    renderNav();
    if (current === "updates") {
      select("updates");
      const st = document.querySelector("[data-check-status]");
      if (st) st.textContent = "These counts had aged — recounted automatically just now.";
    }
  } catch (e) { /* older backend or a failed collect: the as-of line stands */ }
}

/* Explicit Check: on apt the backend first refreshes the package lists
   (elevated, installs nothing) so results are current, then re-collects.
   Older backends without the command just re-run a plain load. */
async function checkUpdates(btn) {
  const status = btn.parentElement.querySelector("[data-check-status]");
  btn.disabled = true;
  if (status) status.textContent = "Checking… on some systems this asks for your password to refresh the package lists.";
  try {
    P.updates = await invoke("refresh_update_check");
    showToast("Update check complete");
    renderNav();
    select(current);
  } catch (e) {
    if (String(e).includes("unknown command") || String(e).includes("not found")) {
      await load();
      return;
    }
    showToast(String(e));
    btn.disabled = false;
    if (status) status.textContent = "";
  }
}

let applying = false;
async function applyGroup(group) {
  const names = { "system-arch": "system packages", "system-apt": "system packages",
    "system-dnf": "system packages", firmware: "firmware", flatpak: "Flatpak apps" };
  const label = names[group];
  if (applying) { showToast("An update is already running — wait for it to finish."); return; }
  const noCancel = "Once an update starts it can't be cancelled. You can close the " +
    "progress window and keep using the app — the update continues in the background.";
  let ok;
  if (group === "firmware") {
    ok = await askConfirm("Update firmware?",
      "Firmware updates can brick a device if power is lost mid-update. Only continue " +
      "on stable power. " + noCancel, "Update firmware", true);
  } else {
    ok = await askConfirm(`Update ${label}?`,
      `This downloads and installs all available ${label} updates now. ${noCancel}`,
      "Update now", false);
  }
  if (!ok) return;
  applying = true;
  document.querySelectorAll("[data-apply], [data-full-upgrade]").forEach((b) => { b.disabled = true; });
  logMeta = { action: `Apply ${label} updates`, headline: "" };
  $("logTitle").textContent = `Updating ${label}…`;
  $("logStatus").textContent = "Running — new output appears below as it happens.";
  $("logBody").textContent = "";
  if (!$("logSheet").open) $("logSheet").showModal();
  vizShow();
  let unlisten = null;
  try { /* progress can never block the update itself */
    unlisten = await listen("update-progress", (e) => {
      const body = $("logBody");
      body.textContent += String(e.payload) + "\n";
      body.scrollTop = body.scrollHeight;
    });
  } catch (err) { /* events unavailable: final output still arrives at the end */ }
  try {
    const report = await invoke("apply_updates", { group });
    /* The headline comes from the backend's POST-APPLY RE-CHECK, never
       from the apply's exit code: v0.8.3 hard-coded "fully up to date"
       here and lied when apt deferred everything to phasing. */
    logMeta.headline = report.headline;
    $("logTitle").textContent = report.remaining === 0
      ? `${label[0].toUpperCase() + label.slice(1)} updates complete`
      : `${label[0].toUpperCase() + label.slice(1)} updates finished`;
    $("logStatus").textContent = logMeta.headline;
    /* v0.8.9: the finished dialog LEADS with the backend's
       plain-English summary (installed / still waiting + why); the
       raw package-manager text follows behind a divider, where the
       Copy button still picks up everything. Dickles' complaint was
       a wall of raw apt output with "6 updates still remain" on top. */
    const summary = (report.summary || []).join("\n");
    if (summary) {
      const raw = $("logBody").textContent || report.output || "";
      $("logBody").textContent = summary +
        (raw ? "\n\n—— Details: what the package manager said ——\n" + raw : "");
    } else if (!$("logBody").textContent && report.output) {
      $("logBody").textContent = report.output;
    }
    vizHide();
  } catch (e) {
    vizSit();
    /* The backend error is composed: plain-language headline first, raw
       output behind a Details divider. REPLACE the body with it — the
       streamed lines are already inside the details, and v0.8.1's
       append-on-error showed the same raw block twice. */
    const msg = String(e);
    logMeta.headline = msg.split("\n")[0];
    $("logTitle").textContent = "Update failed";
    $("logStatus").textContent = logMeta.headline;
    $("logBody").textContent = msg;
  } finally {
    if (unlisten) unlisten();
    applying = false;
    document.querySelectorAll("[data-apply], [data-full-upgrade]").forEach((b) => { b.disabled = false; });
  }
  await load();
}

/* v0.8.9: "Install these too (full upgrade)". The confirmation is fed
   by the backend's live simulation (full_upgrade_preflight), so the
   plan Dickles reads — installs, new companions, removals, the ones
   even this can't fix, the restart — is apt's own prediction, not a
   promise typed into the frontend. Only after he confirms does the
   fenced full-upgrade command run, through the same progress dialog
   and re-checked report as every other apply. */
async function applyFullUpgrade() {
  if (applying) { showToast("An update is already running — wait for it to finish."); return; }
  let plan;
  try {
    plan = await invoke("full_upgrade_preflight");
  } catch (e) {
    showToast(String(e));
    return;
  }
  if (!plan || !plan.available || !plan.waiting.length) {
    showToast("Nothing is waiting on a full upgrade right now.");
    return;
  }
  const parts = [];
  parts.push(`The safe update couldn't install: ${plan.waiting.join(", ")}.`);
  if (plan.new_companions.length) {
    const shown = plan.new_companions.slice(0, 6).join(", ") +
      (plan.new_companions.length > 6 ? ", …" : "");
    parts.push(`Installing them also brings ${plan.new_companions.length} new companion package${plan.new_companions.length === 1 ? "" : "s"} (${shown}).`);
  }
  if (plan.removals.length) {
    parts.push(`It would also REMOVE ${plan.removals.join(", ")} to make room.`);
  }
  for (const b of plan.blocked) {
    parts.push(`Even this can't install ${b.name} yet${b.reason ? ` — it ${b.reason}` : ""}. It will keep waiting.`);
  }
  if (plan.needs_restart) {
    parts.push("A new Linux kernel is included, so this machine will need a restart afterwards.");
  }
  parts.push("Once it starts it can't be cancelled. You can close the progress window and keep using the app — the update continues in the background.");
  const ok = await askConfirm("Install the waiting updates?", parts.join(" "),
    "Install them", plan.removals.length > 0);
  if (!ok) return;
  applying = true;
  document.querySelectorAll("[data-apply], [data-full-upgrade]").forEach((b) => { b.disabled = true; });
  logMeta = { action: "Install waiting updates (full upgrade)", headline: "" };
  $("logTitle").textContent = "Installing the waiting updates…";
  $("logStatus").textContent = "Running — new output appears below as it happens.";
  $("logBody").textContent = "";
  if (!$("logSheet").open) $("logSheet").showModal();
  vizShow();
  let unlisten = null;
  try {
    unlisten = await listen("update-progress", (e) => {
      const body = $("logBody");
      body.textContent += String(e.payload) + "\n";
      body.scrollTop = body.scrollHeight;
    });
  } catch (err) { /* events unavailable: final output still arrives at the end */ }
  try {
    const report = await invoke("apply_full_upgrade");
    logMeta.headline = report.headline;
    $("logTitle").textContent = report.remaining === 0
      ? "System packages updates complete"
      : "System packages updates finished";
    $("logStatus").textContent = logMeta.headline;
    const summary = (report.summary || []).join("\n");
    if (summary) {
      const raw = $("logBody").textContent || report.output || "";
      $("logBody").textContent = summary +
        (raw ? "\n\n—— Details: what the package manager said ——\n" + raw : "");
    } else if (!$("logBody").textContent && report.output) {
      $("logBody").textContent = report.output;
    }
    vizHide();
  } catch (e) {
    vizSit();
    const msg = String(e);
    logMeta.headline = msg.split("\n")[0];
    $("logTitle").textContent = "Update failed";
    $("logStatus").textContent = logMeta.headline;
    $("logBody").textContent = msg;
  } finally {
    if (unlisten) unlisten();
    applying = false;
    document.querySelectorAll("[data-apply], [data-full-upgrade]").forEach((b) => { b.disabled = false; });
  }
  await load();
}

const PHASE_LABEL = { write: "Writing…", read: "Reading…", latency: "Pinging…", download: "Downloading…", upload: "Uploading…" };
async function runSpeed(btn) {
  const kind = btn.dataset.speed;
  const card = btn.closest(".card");
  const finalEl = card.querySelector(".speed-final");
  const gaugeEls = [...card.querySelectorAll(".gauge")];
  const labelFor = (el) => el.querySelector(".g-phase");
  const gaugeFor = (phase) => gaugeEls.find((el) => !el.dataset.phase || el.dataset.phase === phase) || gaugeEls[0];
  btn.disabled = true;
  const label = btn.textContent;
  btn.textContent = "Running…";
  finalEl.textContent = "";
  gaugeEls.forEach((el) => { gaugeSet(el, 0); labelFor(el).textContent = "Starting…"; });
  // The disk test writes first: its read meter waits at rest, needle at
  // zero, until the read phase starts (same patience the network pair
  // shows its upload meter — one test, one live needle at a time).
  if (kind === "disk") labelFor(gaugeFor("read")).textContent = "Waiting…";
  // Progress events are decoration — if the event channel is unavailable the
  // test must still run (this exact failure shipped in v0.3.1: no Tauri
  // capability file meant listen() was denied and the test never started).
  let unlisten = null;
  try {
    unlisten = await listen("speed-progress", (e) => {
      if (e.payload.test !== kind) return;
      const el = gaugeFor(e.payload.phase);
      if (labelFor(el)) labelFor(el).textContent = PHASE_LABEL[e.payload.phase] || e.payload.phase;
      if (e.payload.phase === "latency") finalEl.textContent = "Ping " + Math.round(e.payload.value) + " ms · measuring…";
      else gaugeSet(el, e.payload.value);
    });
  } catch (e) { /* no live needle; final numbers still land */ }
  try {
    if (kind === "disk") {
      const sel = card.querySelector("[data-disk-volume]");
      const deviceHint = sel && sel.selectedOptions && sel.selectedOptions[0]
        ? sel.selectedOptions[0].dataset.device || null : null;
      const r = await invoke("run_disk_test", { mount: sel ? sel.value : null, deviceHint });
      gaugeSet(gaugeFor("write"), r.write_mbps);
      gaugeSet(gaugeFor("read"), r.read_mbps);
      gaugeEls.forEach((el) => { labelFor(el).textContent = "Done"; });
      finalEl.innerHTML = `<strong>Write ${r.write_mbps.toFixed(0)} MB/s · Read ${r.read_mbps.toFixed(0)} MB/s</strong>` +
        `<br>${esc(r.device || "drive")} at ${esc(r.path)} · ${r.size_mb} MB test file` +
        (r.note ? `<br>${esc(r.note)}` : "");
    } else {
      const r = await invoke("run_network_test");
      gaugeSet(gaugeFor("download"), r.download_mbps);
      gaugeSet(gaugeFor("upload"), r.upload_mbps);
      gaugeEls.forEach((el) => { labelFor(el).textContent = "Done"; });
      finalEl.innerHTML = `<strong>↓ ${r.download_mbps.toFixed(1)} · ↑ ${r.upload_mbps.toFixed(1)} Mbps · ${r.latency_ms.toFixed(0)} ms</strong>`;
    }
  } catch (e) {
    gaugeEls.forEach((el) => { labelFor(el).textContent = "Failed"; });
    // A failed test explains itself in plain language (backend's job)
    // and carries a Copy button (standing law: an error you can't
    // copy is an error you can't report).
    const msg = String(e);
    finalEl.innerHTML = `${esc(msg)} <button class="mini-btn" type="button" data-copy-speed-error>Copy</button>`;
    const copyBtn = finalEl.querySelector("[data-copy-speed-error]");
    if (copyBtn) copyBtn.addEventListener("click", async () => {
      const ok = await invoke("copy_text", { text: msg }).catch(() => false);
      copyBtn.textContent = ok ? "Copied" : "Couldn't copy";
      setTimeout(() => { copyBtn.textContent = "Copy"; }, 1600);
    });
  }
  if (unlisten) unlisten();
  btn.disabled = false;
  btn.textContent = label;
}

/* ---------------- feedback: Report a Bug / Request a Feature ------------
   Universal law (2026-10-14): every app carries both. They open the
   project's GitHub issue forms in the browser; the bug form is pre-filled
   with this machine's basics (never serials, MACs, or addresses). */
async function openFeedback(kind) {
  const base = "https://github.com/garrisonr/linux-system-profiler/issues/new";
  const themeNow = themeControl.get() === "auto"
    ? (document.documentElement.dataset.theme || "cupertino") : themeControl.get();
  const machine = [P.system.manufacturer, P.system.product_name].filter(Boolean).join(" ") || "—";
  let title, body;
  if (kind === "bug") {
    title = "[Bug] ";
    body = ["## What happened", "", "<!-- What did you see, and what did you expect instead? -->", "",
      "## This machine (auto-filled — edit freely)",
      `- Profiler: v${P.tool_version} (profile schema v${P.schema_version})`,
      `- OS: ${P.system.os_name || "—"} · kernel ${P.system.kernel || "—"}`,
      `- Machine: ${machine}`,
      `- CPU: ${P.cpu.model_name || "—"}`,
      `- Theme: ${themeNow}`, ""].join("\n");
  } else {
    title = "[Feature] ";
    body = ["## What would you like the Profiler to do?", "", "<!-- The job it would do for you, in your own words. -->", "",
      "## Anything else", "",
      `- Profiler: v${P.tool_version} · OS: ${P.system.os_name || "—"}`, ""].join("\n");
  }
  const url = base + "?labels=" + (kind === "bug" ? "bug" : "enhancement") +
    "&title=" + encodeURIComponent(title) + "&body=" + encodeURIComponent(body);
  try {
    await invoke("open_url", { url });
    showToast(kind === "bug" ? "Opening the bug report form in your browser…" : "Opening the feature request form in your browser…");
  } catch (e) {
    const copied = await invoke("copy_text", { text: url }).catch(() => false);
    showToast(copied
      ? "Couldn't open your browser — the form link is on your clipboard instead"
      : "File it at github.com/garrisonr/linux-system-profiler/issues");
  }
}

/* ---------------- dependencies: self-heal, presented calmly --------------
   The backend classifies every failure; this UI never shows raw tool
   output. A missing polkit agent (classic when the app was launched
   outside the desktop session) becomes guidance + a copyable manual
   command, not an error dump. */
function setDepActions(actions) {
  const box = $("depBannerActions");
  box.innerHTML = "";
  for (const a of actions) {
    const b = document.createElement("button");
    b.type = "button";
    b.className = "pill " + (a.primary ? "primary" : "secondary");
    b.textContent = a.label;
    b.addEventListener("click", a.run);
    box.appendChild(b);
  }
}

async function checkDeps() {
  try {
    const st = await invoke("get_dep_status");
    if (st.missing.length) {
      $("depBannerText").textContent =
        `Missing probe tools: ${st.missing.join(", ")} — ${st.missing_features.join("; ")}. ` +
        `Sections would come back empty without them.` +
        (st.not_needed && st.not_needed.length
          ? ` Not needed on this system: ${st.not_needed.join("; ")} — this system's own tools cover those jobs.`
          : ``);
      setDepActions([{ label: "Install missing tools", primary: true, run: runDepInstall }]);
      $("depBanner").classList.remove("hidden");
    } else {
      $("depBanner").classList.add("hidden");
    }
  } catch (e) { /* older backend without the command: stay silent */ }
}

async function runDepInstall() {
  const box = $("depBannerActions");
  box.querySelectorAll("button").forEach((b) => { b.disabled = true; });
  const primary = box.querySelector(".primary");
  if (primary) primary.textContent = "Installing…";
  let r;
  try {
    r = await invoke("install_deps");
  } catch (e) { // the IPC call itself failed — still calm, still no raw dump
    $("depBannerText").textContent = "The install couldn't be started from the app — nothing was changed.";
    setDepActions([{ label: "Try again", primary: true, run: runDepInstall }]);
    return;
  }
  if (r.ok) {
    showToast(r.kind === "already" ? "All probe tools are already installed" : "Probe tools installed — refreshing");
    $("depBanner").classList.add("hidden");
    await load();
    return;
  }
  if (r.kind === "cancelled") {
    showToast(r.message);
    setDepActions([{ label: "Install missing tools", primary: true, run: runDepInstall }]);
    return;
  }
  // no-agent / unavailable / failed — guidance, a way forward, details on request
  $("depBannerText").textContent = r.message;
  const acts = [{ label: "Try again", primary: true, run: runDepInstall }];
  if (r.manual_command) {
    acts.push({ label: "Copy install command", run: async () => {
      const copied = await invoke("copy_text", { text: r.manual_command }).catch(() => false);
      if (copied) showToast("Copied — paste it into a terminal to install the tools");
      else showLog("Install from a terminal", "Run this command in a terminal:\n\n" + r.manual_command);
    }});
  }
  acts.push({ label: "Details", run: () => showLog("Install details", r.detail || r.message) });
  setDepActions(acts);
}

/* ---------------- loading: skeleton + live stage narration ---------------
   The skeleton Overview is static HTML (first paint is already the right
   shape, in the engine's colors). Collector stages stream in as
   "profile-progress" events and narrate the stage line; when the profile
   lands, the real sections rise in staggered. Progress is decoration — if
   the event channel is unavailable the profile still arrives (v0.3 lesson). */
const STAGE_LABELS = {
  system: "reading system information", cpu: "reading the CPU", memory: "reading memory",
  storage: "reading storage & SMART health", network: "reading network interfaces",
  graphics: "reading graphics", displays: "reading displays", volumes: "reading volumes",
  printers: "reading printers", bluetooth: "reading Bluetooth", accounts: "reading accounts",
  security: "checking security posture", power: "reading power & batteries",
  devices: "reading PCI & USB devices", audio: "reading audio",
  software: "reading installed software", updates: "checking for updates",
};
let profileLoading = false;
async function watchProgress() {
  try {
    await listen("profile-progress", (e) => {
      if (!profileLoading) return;
      const el = $("loadStage");
      if (el) el.textContent = (STAGE_LABELS[e.payload] || e.payload) + "…";
    });
  } catch (err) { /* no narration; the wait is the same, the profile still lands */ }
}

/* The startup loading block (static HTML in index.html) as a template,
   so Refresh presents the wait EXACTLY like first launch — skeleton +
   stage narration — instead of v0.8.4's invisible swap behind a fade
   (Dickles: "a small animation and nothing else; I cannot tell it did
   anything"). Functionally Refresh always re-collected in full; now it
   looks like it too. Keep this markup in sync with index.html. */
function loadingMarkup() {
  return `<p class="load-stage" role="status"><span class="load-dot" aria-hidden="true"></span><span>Gathering profile — <span id="loadStage">starting…</span></span></p>
    <div class="skel-card"><span class="skel-bar w40"></span><span class="skel-bar w70"></span><span class="skel-bar w55"></span></div>
    <div class="skel-card"><span class="skel-bar w40"></span><span class="skel-bar w70"></span></div>
    <div class="skel-card"><span class="skel-bar w55"></span><span class="skel-bar w70"></span><span class="skel-bar w40"></span></div>
    <div class="skel-card"><span class="skel-bar w40"></span><span class="skel-bar w55"></span></div>
    <div class="skel-card"><span class="skel-bar w70"></span><span class="skel-bar w40"></span></div>`;
}

async function load(isRefresh) {
  if (profileLoading) return;
  profileLoading = true;
  const btn = $("refreshBtn");
  if (isRefresh) {
    closeGloss();
    $("detailBody").innerHTML = loadingMarkup();
    $("detailBody").setAttribute("aria-busy", "true");
    btn.disabled = true;
    btn.textContent = "Refreshing…";
  }
  try {
    P = await invoke("get_profile");
  } finally {
    profileLoading = false;
    btn.disabled = false;
    btn.textContent = "Refresh";
  }
  let pref = "auto";
  try { pref = localStorage.getItem("lsp-theme") || "auto"; } catch (e) {}
  themeControl.set(pref);
  applyTheme(pref, P);
  $("machineName").textContent = P.hostname || "This Computer";
  $("machineSub").textContent = [P.system.os_name, P.cpu.model_name].filter(Boolean).join(" · ");
  $("detailBody").removeAttribute("aria-busy");
  $("detailBody").classList.add("reveal");
  select(current);
  setTimeout(() => $("detailBody").classList.remove("reveal"), 900);
  if (isRefresh) {
    // Newness he can SEE: the generated time, announced — the same value
    // the Overview and About lines render from the fresh profile.
    const t = new Date(P.generated_at);
    showToast("Profile refreshed — generated " + (isNaN(t) ? P.generated_at : t.toLocaleTimeString()));
  }
  await checkDeps();
  if (!isRefresh) await autoRefreshIfStale();
}

/* v0.8.8: the launch scan reads apt's lists as they lie on disk. When
   the backend says they're stale, run the EXISTING Check path
   (refresh_update_check — elevated metadata refresh, then re-count)
   once, automatically, so first-screen counts are current. If the
   refresh can't complete (password prompt dismissed, agent missing),
   the stale banner + note stay on the Updates view: the counts are
   labelled, never silently wrong. Once per launch, and never after a
   manual Refresh — a dismissed prompt is an answer, not a retry cue. */
let autoChecked = false;
async function autoRefreshIfStale() {
  if (autoChecked || !P || !P.updates || !P.updates.metadata_stale) return;
  autoChecked = true;
  try {
    P.updates = await invoke("refresh_update_check");
    renderNav();
    select(current);
    showToast("The package lists were out of date — refreshed them, so update counts are current.");
  } catch (e) {
    showToast("Couldn't refresh the package lists — update counts may be out of date. See the Updates tab.");
  }
}

/* ---------------- wiring ---------------- */
$("refreshBtn").addEventListener("click", () => load(true));

/* Glossary card: one delegated listener set for every [data-hint]
   affordance, wherever it's rendered. Click toggles, keyboard focus
   opens (buttons are natively focusable; Enter/Space click), hover
   opens on fine pointers, Esc / outside-click closes. */
let kbdNav = false;
document.addEventListener("keydown", (e) => {
  if (e.key === "Tab") kbdNav = true;
  if (e.key === "Escape") closeGloss();
});
document.addEventListener("pointerdown", (e) => {
  kbdNav = false;
  if (glossAnchor && !e.target.closest("[data-hint]") && !$("glossCard").contains(e.target)) closeGloss();
});
document.addEventListener("click", (e) => {
  const b = e.target.closest("[data-hint]");
  if (b) { e.preventDefault(); openGloss(b, true); }
});
document.addEventListener("focusin", (e) => {
  const b = e.target.closest("[data-hint]");
  if (b && kbdNav) openGloss(b, false);
});
document.addEventListener("pointerover", (e) => {
  if (!window.matchMedia("(hover: hover) and (pointer: fine)").matches) return;
  const b = e.target.closest("[data-hint]");
  if (b && glossAnchor !== b) openGloss(b, false);
});
$("backBtn").addEventListener("click", () => $("app").classList.remove("show-detail"));
$("logClose").addEventListener("click", () => $("logSheet").close());
$("logCopy").addEventListener("click", (e) => copyLogOutput(e.currentTarget));
$("themeBtn").addEventListener("click", () => (themePopOpen() ? closeThemePop() : openThemePop()));
$("themeDropdown").addEventListener("keydown", themeKeyNav);
document.addEventListener("pointerdown", (e) => {
  if (themePopOpen() && !$("themeDropdown").contains(e.target)) closeThemePop();
});

// Copy & Share sheet
$("exportBtn").addEventListener("click", () => $("shareSheet").showModal());
$("shareClose").addEventListener("click", () => $("shareSheet").close());
document.querySelectorAll("[data-share]").forEach((b) =>
  b.addEventListener("click", async () => {
    const [action, kind] = b.dataset.share.split(":");
    $("shareSheet").close();
    if (action === "copy") {
      const r = await invoke("share_copy", { kind });
      if (r.copied) showToast(`Copied — ${r.chars.toLocaleString()} characters on your clipboard`);
      else { $("exportText").value = r.text; $("exportSheet").showModal(); }
    } else if (action === "save") {
      try {
        const path = await invoke("share_save", { kind });
        if (path) showToast("Saved to " + path);
      } catch (e) { showLog("Save failed", String(e)); }
    } else if (action === "email") {
      try { await invoke("share_email"); showToast("Opening your mail client…"); }
      catch (e) { showLog("Email failed", String(e)); }
    }
  }));
$("exportSelect").addEventListener("click", () => { $("exportText").focus(); $("exportText").select(); });
$("exportClose").addEventListener("click", () => $("exportSheet").close());
buildThemeDropdown();
watchProgress();

load().catch((e) => { $("detailBody").innerHTML = `<p class="empty">Failed to load profile: ${esc(String(e))}</p>`; });
