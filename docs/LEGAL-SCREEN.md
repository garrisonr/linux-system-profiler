# Preliminary legal screen — 2026-10-13 (NOT legal advice)

Per Dickles' standing order: screen BEFORE any spend or publishing.

1. **USPTO / trademark:** Web screen found no live "Linux System Profiler"
   mark for software. "System Profiler" alone is a generic/descriptive term
   (Apple's own `system_profiler` is a product feature name, not the project
   name here). Full USPTO TESS search still required before any public launch.
2. **Existing projects with same/similar name (collision check):**
   - PySysCheck — describes itself as "a Linux system profiler" (descriptive use, different name). No collision.
   - flametui — a "Linux system profiler" but that's a perf/flamegraph tool (different meaning of profiler). No name collision.
   - Hardinfo / Hardinfo2, lshw, inxi, CPU-X, hw-probe — the incumbents. Different names; our differentiator is the updates hub + Apple-style presentation.
   - No project found shipping under the exact name "Linux System Profiler".
3. **License compatibility:** MIT (chosen). Deps: serde / serde_json
   (MIT OR Apache-2.0), sha2 (MIT OR Apache-2.0). All compatible with MIT.
4. **Bundled assets:** None. No fonts, icons, or stock media bundled; UI will
   use system fonts and original/emoji-free glyphs only.

**Verdict:** Safe to build locally under the working name. NOT cleared for
domain purchase, GitHub publication, or branding — the Fable Files lesson
applies: name equity is the expensive part, so the final name gets a full
USPTO search first, before a single cent is spent.

## Full USPTO search — 2026-10-13 (official USPTO Trademark Search, tmsearch.uspto.gov)
Run per Dickles' strengthened rule (full search at name-suggestion time).
Syntax validated against a known mark, so zero results are genuine.
- "Linux System Profiler" (exact phrase): **0 results** — no live or dead mark.
- "LinuxSystemProfiler" (concatenated): **0 results**.
- "System Profiler" (exact phrase): **0 marks contain the phrase**. Both-words
  search: 3 results, 1 live — 3-D PROFILER SYSTEM (Somero Enterprises, IC 009,
  computerized *concrete placement* systems; unrelated field).
- "Profiler" in IC 009/042: 291 total, **62 live**. Closest live marks:
  VPROFILER (Modus Cyberandi, IC 042, network-security consulting),
  BREACHPROFILER (pending, Infinnium, IC 009/042, cybersecurity ML software),
  NETPROFILER (X-Rite, color-calibration software), DATIRIS PROFILER
  (data profiling, IC 009), IPROFILER (pending, chip design). All differ in
  formative + goods; none judged a realistic refusal/opposition risk for a
  Linux hardware/firmware profiling utility.
- **Real weakness is descriptiveness, not conflict:** "Linux" + "System" +
  "Profiler" each describe exactly what the software does. Expect a
  merely-descriptive objection to registration on the Principal Register;
  the name is clear to USE, but weak as protectable brand equity.
- Preliminary screen only — not legal advice.
