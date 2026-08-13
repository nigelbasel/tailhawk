# CLEANROOM.md — provenance record

Tailhawk ships under **MIT OR Apache-2.0**. The closest prior art in this product category is
GPL-licensed: **klogg** (GPL-3.0), **TailBlazer** (GPL-3.0), **SnakeTail** (GPL-3.0). Reading their
implementation source and then writing equivalent code would make the derivative-work question a
real one rather than a theoretical one.

This file is the record that we didn't. It has to stay **ahead of the code** — an entry written
after the fact is worth very little.

**Populated 2026-07-29, before any implementation code exists.**

---

## 1. The rule

1. **Do not read GPL-licensed implementation source** for any component in the register in §4.
2. **Permitted inputs:** published documentation, public specifications, academic papers, vendor API
   reference, textbooks, your own measurements, and the *observable behaviour* of any tool — what it
   does, never how it does it.
3. **Prose is not code.** An algorithm described in a project's README, design doc or header-comment
   prose is usable; the implementation below it is not. Any such use gets logged in §5 with the URL
   and the commit hash it was read at, so the exact text can be re-examined later.
4. **Whoever reads, doesn't write.** Any contributor — human or agent — who has read forbidden
   source for a component is disqualified from writing that component. This is the whole mechanism;
   everything else is bookkeeping.
5. **Log before you code.** The §5 entry for a component is written and committed *before* the first
   line of that component, not alongside it.

## 2. Known contamination

**`docs/RESEARCH.md` §5.3 is contaminated and is reference-forbidden for implementation.**

The section carries its own warning. It cites internal constants that could only have come from
reading GPLv3 implementation source — in the same section that mandates a clean-room process, and
without recording who read what. That combination is why it cannot be relied on: not because the
design is necessarily wrong, but because its provenance is unknown and unreconstructable.

- **Affected component:** the block-sparse line-offset index.
- **Consequence:** the index design must be **re-derived from §3 sources only**, with the
  re-derivation logged in §5, before any index code is written.
- **Do not** implement from §5.3 directly, and do not quote it into a design document that then
  feeds implementation.

**⚠ The contamination had already propagated, and this list was too narrow until 2026-08-04.**
Naming `RESEARCH.md` §5.3 alone implied everything else was clean. It was not — the design and at
least one of its constants had reached `SPEC.md`, which carried no warning:

| Where | What | Status |
|---|---|---|
| `docs/RESEARCH.md` §5.3 | The original. Provenance unknown and unreconstructable. | **CONTAMINATED**, unchanged |
| `docs/SPEC.md` §5.3 "Line index" | Same number and topic. Provenance never established. Read and edited in session 8 (`571eb2e`) with no §5 entry. | **SUPERSEDED 2026-08-04** — replaced by a clean re-derivation, entry 2026-08-04 in §5 |
| `docs/SPEC.md` §11.2, memory table | Stated `block-sparse, 128/block` — a design constant, in a section with no warning | **Corrected 2026-08-04** to the re-derived figure |

The general lesson, which is the reason this table exists rather than a one-line fix: **a
contamination notice attached to one section does not travel with the content.** When a design is
quoted onward, the warning stays behind. Naming the *component* in §4 is what catches this; naming
the *section* is not enough.

`docs/RESEARCH.md` §11 additionally records that klogg, TailBlazer and SnakeTail are all GPL and
that a clean-room rule was needed. This file is that rule.

## 3. Source allow-list

| Source | Licence | Status |
|---|---|---|
| klogg | GPL-3.0 | **Source FORBIDDEN.** Observable behaviour, published docs and README prose permitted, logged. |
| TailBlazer | GPL-3.0 | **Source FORBIDDEN.** Same carve-out. |
| SnakeTail | GPL-3.0 | **Source FORBIDDEN.** Same carve-out. |
| LogExpert | MIT | Source permitted. If code is used or closely followed, attribute it and record it here. |
| BareTail, Hoo WinTail | proprietary | **Observable behaviour only.** No disassembly, no decompilation. |
| Microsoft Learn / Win32 & DirectWrite / D3D11 reference | MS docs terms | Permitted. |
| Unicode, IETF RFC, W3C, OpenTelemetry specifications | open | Permitted. |
| Rust crate documentation and source | per crate | Permitted subject to each crate's own licence; `cargo-deny` governs what we *link*, which is a separate question from what we *read*. |
| Academic papers, textbooks | per publisher | Permitted; cite in §5. |
| Our own measurements and experiments (`PLAN.md` §3, Phase 0) | ours | Permitted, and preferred. |

## 4. Component register

Components are clean by default. Only those listed here carry a restriction.

| Component | Status | Cleared to implement? | Notes |
|---|---|---|---|
| Line-offset index | **RE-DERIVED CLEAN**, 2026-08-04 | **Yes** — implement from `SPEC.md` §5.3 as rewritten | Derived from §3 sources only; §5 entry and §6 attestation both filed. `RESEARCH.md` §5.3 remains contaminated and reference-forbidden. The re-derivation **rejected** the superseded design's delta encoding and its 128 stride, so reproducing either from memory is a regression, not a shortcut. |
| Everything else | Clean | Yes | Add a row here the moment that stops being true. |

## 5. Re-derivation and consultation log

Append-only. Newest last.

| Date | Component | Who | Sources consulted | Notes |
|---|---|---|---|---|
| 2026-07-29 | *(none — file creation)* | Claude Opus 5, session 2 | `docs/RESEARCH.md` lines 456–465 and 493 only — the §5.3 contamination notice and the "GPL hazard" line | **The technical content of §5.3 was deliberately not read.** Only the warning text and the identification of the affected component were consulted, specifically so this agent remains eligible to implement the index under §1.4. |
| 2026-07-30 | Glyph atlas, colour-glyph composition, atlas eviction | Claude Opus 5, session 5 | Microsoft Learn only: `IDWriteGlyphRunAnalysis`, `IDWriteFactory2::TranslateColorGlyphRun`, `DWRITE_COLOR_GLYPH_RUN`, `D3D11_RENDER_TARGET_BLEND_DESC` / dual-source blending, `D3D11_QUERY_TIMESTAMP`. Plus the `windows` crate's own generated bindings (MIT OR Apache-2.0) to confirm signatures. | **No third-party implementation source of any kind was read** — not klogg/TailBlazer/SnakeTail (forbidden), and not the permitted ones either. The atlas design in `experiments/g4-glyph-atlas` and the rules derived into `SPEC.md` §3.2 come from vendor API reference plus this experiment's own measurements. Recorded because §1.5 asks for the entry before the code, and the grid renderer is the component this feeds. |
| 2026-07-29 | Virtualised grid — scroll position model | Claude Opus 5, session 4 | **egui 0.17.0**, `egui/src/containers/scroll_area.rs`, at tag `0.17.0` via `raw.githubusercontent.com/emilk/egui/0.17.0/...`; plus `github.com/emilk/egui/issues/1391`. Functions read: `ScrollArea::begin`, `show_rows`, `show_viewport`/`show_viewport_dyn`, `Prepared::end`, `struct State`, and `emath::remap`/`remap_clamp`/`lerp` as quoted within them. | **Permitted — egui is MIT OR Apache-2.0, §3 "Rust crate documentation and source".** Read for G7 (`PLAN.md` §3), to diagnose a *defect* in that code. `experiments/g7-egui-scroll/src/main.rs` deliberately **replicates** the arithmetic in order to measure it, and quotes the original above each function; that file is an experiment and **must not be linked into Tailhawk**. The rules derived into `SPEC.md` §6.4 are the *inverse* of what egui does and were not copied from it. No GPL source was consulted. |
| 2026-07-31 | Encoding detection and incremental line decoding (M1) | Claude Opus 5, session 8 | `docs/SPEC.md` §5.6, and §5.3's code-unit alignment invariant — both ours. The Unicode Standard's BOM signatures. The WHATWG Encoding Standard, consulted for what it *excludes*: UTF-16 and UTF-32 detection and UTF-32 decoding are outside it, which is why both are hand-written here rather than delegated. `encoding_rs` and `chardetng` API documentation (both Apache-2.0 OR MIT, §3 "Rust crate documentation and source"). | **No log-viewer implementation source of any kind was read** — not klogg/TailBlazer/SnakeTail (forbidden), and not LogExpert either (permitted, but unread, so the §1.4 disqualification does not attach). The NUL-position-parity probe, the head/tail disagreement rule and the pending-CR carry are derived from `SPEC.md` §5.6, which was written from the standards above and from Corpus B's observed behaviour. Logged because §1.5 asks for the entry before the code, even though the component is clean under §4. |

| 2026-08-04 | **Line-offset index — the clean re-derivation** | Claude Opus 5, session 9 | **First principles**, plus: our own measurements of the two dogfood corpora taken this session (mean line length 116.8 and 84.2 bytes — §3 "our own measurements", which §3 prefers); `SPEC.md` §5.2 (no mmap), §6.4 (`u64` scroll model), §10.3 (very long lines), §11.2's whole-system memory claim and §11.3's "UI never blocks"; and this repo's own `crates/tailhawk-core/src/encoding.rs`, for `code_unit()` and `is_random_access_decodable()` and the two exhaustive `0x0A` tests behind them. | **Neither `RESEARCH.md` §5.3 nor `SPEC.md` §5.3 was read** — not before, during or after. The replacement text was spliced in by line range (394–436, boundaries asserted against the §5.3 and §5.4 headings) precisely so the superseded text never entered this agent's context. No GPL implementation source of any kind was read; no log-viewer source of any kind was read, permitted or otherwise. **Two disclosures, because a clean-room claim is worth nothing if it overstates itself:** (1) `SPEC.md` §11.2's memory table was read *before* the contamination was recognised, and it contained the string `block-sparse, 128/block` — so the terms "block-sparse" and the number 128 were known going in; (2) a `git show` hunk header for `571eb2e` exposed the single word `delta-encoded` from the superseded section. Both anchors were therefore **argued against explicitly rather than silently avoided**: the derivation rejects delta encoding on a ~9x memory margin and selects a stride of 64 on cold-read latency. Had it landed on 128 with delta encoding, that coincidence would have had to be disclosed here instead. |

| 2026-08-05 | **Line-offset index — the parallel indexer (E4)** — `indexer.rs` | Claude Opus 5, session 10 | `SPEC.md` §5.3 **as rewritten on 2026-08-04** — which §4 does not merely permit but directs ("implement from `SPEC.md` §5.3 as rewritten") — plus §5.2 (no mmap, positional reads), §11.2's memory claim, and this repo's own `encoding.rs`, `index.rs`, `lines.rs` and `file.rs`. | **No source outside this repository was consulted at all** — no GPL implementation source, no log-viewer source permitted or otherwise, no vendor documentation. `RESEARCH.md` §5.3 was not read, and its technical content has still never entered this agent's context. The one design decision not already settled by §5.3 — that a worker anchors from its own first line, giving `LineIndex` a segment directory — was taken from §5.3's own "Construction" paragraph and from a memory argument stated in the commit: buffering every line start so the merge could pick globally aligned ones costs 8 bytes per line of transient buffer, which an 8 MB chunk of empty lines turns into 64 MB per worker. §5.3's "The structure" was extended to describe the directory. Logged because §1.5 asks for the entry before the code, and because session 8's unlogged edit to this very section is the recorded reason the rule exists. |

| 2026-08-05 | **Record model + OTel mapping + severity tables (E8)** — `record.rs` | Claude Opus 5, session 10 | The **OpenTelemetry logs data model** and its **Appendix A/B**, at `opentelemetry.io/docs/specs/otel/logs/data-model/` and `…/data-model-appendix/` — §3 "OpenTelemetry specifications", permitted. Plus our own `SPEC.md` §6.1, §6.2, §7.2, §10.2, §10.3 and `UI-DESIGN.md` §11.2. | **No implementation source of any kind was read** — no log viewer, no logging framework, no OTel SDK. The severity tables for syslog, log4j, Zap, Windows Event Log and `java.util.logging` are **transcribed from Appendix B**, which is a specification document, not code. Transcribing them was deliberate rather than writing them from knowledge: the first draft of this file was written from memory and had **syslog Emergency at 23 and Alert/Critical in the FATAL band** (the appendix says 21, 19 and 18) and **`java.util.logging` FINER at 3** (the appendix says 5) — plausible, confident and wrong, in a table whose whole purpose is cross-format consistency. The HTTP-status aliases are **ours**, labelled as such in the code, because Appendix A describes the Apache access log but assigns it no severity. **⚠ Filed after the code was written, not before, which §1.5 requires.** The omission was caught by a review pass rather than by the process; recorded as a miss rather than papered over, and it is the second time this exact rule has been broken (see the session-8 note in §2). |

| 2026-08-05 | **Cell model (V4)** | Claude Opus 5, session 10 | `SPEC.md` §3.3, §5.6 and §13.4 — ours. The **`unicode-segmentation`** and **`unicode-width`** crates, for their documented behaviour and public API (§3 "Rust crate documentation and source"; both Apache-2.0 OR MIT, so `deny.toml` admits them). | **No implementation source of any kind was read**, including the two crates' internals — they are used as libraries, through their public API, not as a design to copy. **UAX #29 (grapheme cluster boundaries) and UAX #11 (East Asian Width) were not read directly**; the crates implement them and this module delegates, which is the point of taking the dependency rather than shipping and ageing our own copy of the Unicode character database. The rules layered *on top* — width from the cluster's base rather than summed across it, presentation selectors winning, regional-indicator pairs, a control character taking a visible cell per §5.6, and zero-width characters becoming one cell under §13.4's reveal toggle — are derived from our own specification sections named above. |

| 2026-08-05 | **Device-removed recovery (V1)** — `gpu.rs` | Claude Opus 5, session 11 | `SPEC.md` §3.2 — ours. The **`windows` crate's own generated bindings** (MIT OR Apache-2.0, §3 "Rust crate documentation and source"), read for the four `DXGI_ERROR_DEVICE_*` constants and the `GetDeviceRemovedReason` signature. The **Windows SDK headers** on this machine, grepped to check one constant. | **No implementation source of any kind was read** — no log viewer, no graphics engine, no vendor sample. The policy (retry on the same rung once, then WARP, then give up; only a frame that needed no recovery clears the streak) is ours and is argued in the module. **One from-memory constant was written and then removed.** A first draft carried `D3DDDIERR_DEVICEREMOVED = 0x88760870`, recalled rather than looked up; grepping the 10.0.22621 and 10.0.26100 SDK headers found no such symbol, so it was deleted rather than shipped unverified — the four DXGI codes are each taken from the bindings, and asking the device directly covers what a code list cannot. This is the same failure mode as E8's severity tables, caught before commit this time rather than by review. **⚠ Filed after the code was written, not before, which §1.5 requires** — the third time that rule has slipped, and the pattern is now that it slips whenever a component starts as "a small addition to something that already exists" rather than as a new file. |

| 2026-08-05 | **Glyph atlas — the slot allocator, and the DirectWrite rasteriser (V2)** | Claude Opus 5, session 11 | Our own `experiments/g4-glyph-atlas/RESULTS.md`, `experiments/g4b-batched-raster/RESULTS.md` and the **source of `g4b-batched-raster`** — all ours (§3 "our own measurements", which §3 prefers) — plus `SPEC.md` §3.2 and §3.3, and the `windows` crate's generated DirectWrite bindings (MIT OR Apache-2.0). | **No source outside this repository was consulted at all**, and no vendor documentation was read this session; the DirectWrite call sequence in `raster.rs` was taken from our own experiment's source rather than from memory, which is the practice that keeps constants and argument orders honest. Every rule in `atlas.rs` and `raster.rs` — uniform slots, the O(1) intrusive victim list, never evicting a slot touched in the current frame, caching the absence of ink, deriving the cell from measured bounds rather than em size — is a conclusion of one of those two experiments, and the module cites the measurement beside each. **§1.5 is satisfied in advance for this component**: the entry dated 2026-07-30 above was filed before any atlas code existed, for exactly this reason, and its scope (glyph atlas, colour-glyph composition, atlas eviction) covers this work. This row is the pointer, not a late filing. |

| 2026-08-06 | **Glyph pass — the sheet, the dual-source blend state and the instanced draw (V2)** — `sheet.rs`, `text.rs` | Claude Opus 5, session 11 | Our own `experiments/g4-glyph-atlas` — its `RESULTS.md`, and the source of its `grid.rs` and `atlas.rs`, which is where the HLSL and the blend description come from — plus `SPEC.md` §3.2. All ours (§3 "our own measurements"). The `windows` crate's generated D3D11 bindings (MIT OR Apache-2.0). | **No source outside this repository was consulted at all**, and no vendor documentation was read this session. `shaders/glyphs.hlsl` is **adapted from G4's own shader**, which was written for this purpose from the Microsoft Learn references already logged in the 2026-07-30 entry above; the adaptation is offline compilation via `fxc` instead of `D3DCompile`, plus the comment about the unbound colour sheet. The blend description and the `E_INVALIDARG` trap likewise come from G4's source rather than from memory. Covered in advance by that 2026-07-30 entry, which named this component before any of its code existed. |

| 2026-08-06 | **Text shaping — cluster → glyph ids, the V2/V3 bridge** | Claude Fable 5, session 12 | **Microsoft Learn only**, fetched this session before any code: `IDWriteTextAnalyzer::AnalyzeScript`, `::GetGlyphs`, `::GetGlyphPlacements`, `IDWriteTextAnalysisSource` (interface page and `GetTextAtPosition`), `IDWriteTextAnalysisSink::SetScriptAnalysis` — §3 "Microsoft Learn / Win32 & DirectWrite / D3D11 reference", permitted. Plus our own `SPEC.md` §3.2 and §3.3, our own `cell.rs`, `glyphs.rs` and `raster.rs`, and the `windows` crate's generated DirectWrite bindings (MIT OR Apache-2.0), including its `implement` machinery for authoring the COM source and sink. | **No implementation source of any kind was read** — no log viewer (forbidden or permitted), no text engine, no HarfBuzz, no browser, no terminal emulator, and no vendor *sample* code either: the pages consulted are API reference, and the call sequence (AnalyzeScript through a source/sink pair, then GetGlyphs with the 3n/2+16 buffer estimate and the `ERROR_INSUFFICIENT_BUFFER` retry, then GetGlyphPlacements) is taken from those reference pages, which state it. The design layered on top — shaping per script run, deriving each grapheme cluster's glyph range from `clusterMap` boundaries, and keeping the module device-free so it is testable headless like `raster.rs` — is ours, argued in the module. Filed **before** the first line of `shape.rs`, which §1.5 requires and which has slipped three times when a component started as "a small addition"; this one starts as a new file precisely so the entry comes first. |

| 2026-08-06 | **Virtualised grid — the scroll model, row layout and hit-test (V3)** | Claude Opus 5, session 13 | Our own `SPEC.md` §6.4 (the three scroll rules) and §3.3, and our own `experiments/g7-egui-scroll/RESULTS.md` — §3 "our own measurements", which §3 prefers. Plus this repo's `cell.rs`, `shape.rs` and `index.rs`. | **No source outside this repository was consulted at all** — no log viewer (forbidden or permitted), no UI toolkit, and **egui was not read this session in any form**. That is the point of this entry rather than a bare pointer to the 2026-07-29 row: that row records session 4 reading egui 0.17.0's `scroll_area.rs` under §3's permission, and it also records that `experiments/g7-egui-scroll/src/main.rs` **deliberately replicates that arithmetic** and quotes the original above each function. **That file was deliberately not opened this session.** The implementation is taken from `RESULTS.md`'s "Consequences for Tailhawk" and from §6.4, both of which are our own prose stating the rules as the *inverse* of what egui does — the same posture §4 directs for the index ("implement from `SPEC.md` §5.3 as rewritten"). Filed **before** the first line of `grid.rs`, per §1.5. |

| 2026-08-07 | **Selection, visual reordering, horizontal scrolling and the viewport (V3, the rest)** — `selection.rs`, `bidi.rs`, `hgrid.rs`, `view.rs`, and `cell.rs`'s `byte_span` / `word_at_cell` | Claude Opus 5, sessions 14–15 | Our own `SPEC.md` §3.3, §5.6, §6.4, §10.3 and §13.4, `UI-DESIGN.md` §12, and this repo's `cell.rs`, `grid.rs`, `shape.rs` and `index.rs`. For `bidi.rs`: **UAX #9 rule L2**, an open Unicode specification (§3 "Unicode … specifications", permitted), quoted verbatim in the module. The `unicode-segmentation` crate through its public API only, already covered by the 2026-08-05 cell-model row. | **⚠ Filed retroactively, which is weaker than what §1.5 asks for, and the difference is stated rather than glossed.** §1.5 wants the entry *before* the code; this row covers four modules across two sessions, and only `hgrid.rs` and `view.rs` were written by the agent filing it. **For `selection.rs`, `bidi.rs` and the `cell.rs` additions this row records what the code and its commits show about their sources, not a first-hand attestation of what was read** — a contemporaneous entry is evidence, a reconstructed one is inference, and §6 is where that distinction is normally kept. What can be said first-hand: **no source outside this repository was consulted at all** for `hgrid.rs` or `view.rs` — no log viewer (forbidden or permitted), no UI toolkit, no vendor documentation. Both derive from `SPEC.md` §6.4's rule 3, §10.3's render cap and §3.3's extent, all ours. **This is the fourth slip of §1.5, and the pattern named in the 2026-08-05 device-recovery row now has a second form:** that row said the rule slips when a component starts as "a small addition to something that already exists"; these were four *new files*, and it slipped anyway — because each felt like the continuation of a V3 row already filed. A pointer row covering a component in advance (as the 2026-07-30 atlas entry legitimately does) only works when the later work is inside the scope the earlier row named. "Virtualised grid — the scroll model, row layout and hit-test" did not name selection, reordering or the horizontal axis. |

| 2026-08-07 | **The renderer's text pass (M3)** — `paint.rs`, and the `Renderer`/`gpu.rs` changes it needs | Claude Opus 5, session 15 | Our own `SPEC.md` §3.2 (never block a frame on rasterisation; device-removed recovery) and §3.3 (the cell grid wins over a disagreeing advance), and this repo's `glyphs.rs`, `text.rs`, `sheet.rs`, `shape.rs`, `view.rs` and `gpu.rs` — all ours. The `windows` crate's generated D3D11 bindings (MIT OR Apache-2.0). | **Filed before the first line of `paint.rs`**, per §1.5 and the CI gate added alongside it. No source outside this repository is to be consulted for this component: no log viewer (forbidden or permitted), no graphics engine, no terminal emulator, no vendor sample. The pieces it joins were each logged when they were built (the 2026-07-30 atlas row, the 2026-08-06 glyph-pass and shaping rows); this component is the join and the device-lifetime rule around it — that text resources are keyed to `gpu.rs`'s device generation, because a `GlyphCache` holds a `Sheet` that a rebuilt device invalidates. That rule is derived from `gpu.rs`'s own recovery path and §3.2, not from anywhere outside. |

| 2026-08-07 | **Random-access row text — the index/decoder/viewport join (M3)** — `rows.rs`, and the shell wiring that feeds it to `Renderer::paint_rows` | Claude Opus 5, session 15 | Our own `SPEC.md` §5.3 (R1, line number → byte offset) and §11.3 (a row not in memory draws nothing rather than blocking), and this repo's `index.rs`, `indexer.rs`, `lines.rs`, `file.rs` and `view.rs` — all ours. | **Filed before the first line of `rows.rs`**, per §1.5 and the CI gate. No source outside this repository is to be consulted: no log viewer (forbidden or permitted), no editor, no text buffer library, no vendor sample. The one design decision worth naming in advance is ours and follows from §5.3's own structure: a viewport is a run of **consecutive** rows, so the component resolves the *first* visible row through `offset_of_line` and then decodes **forward** through the rest, rather than resolving each row independently. `offset_of_line` costs an anchor lookup plus a forward scan — §5.3 puts that scan at 6.3 KB — so doing it per row would pay that cost fifty times for a screenful whose bytes are already contiguous. |

| 2026-08-07 | **Navigation input — wheel and keyboard driving the scroll model (M3)** — the `WM_MOUSEWHEEL` / `WM_KEYDOWN` handling in the shell and the `Navigate` command it produces | Claude Opus 5, session 15 | Our own `UI-DESIGN.md` §12's keyboard map (`Shift+wheel`, `Home`/`End`/`Ctrl+Home`/`Ctrl+End`, `Space`/`b`) and its "scrolling up while following auto-pauses follow" rule, our own `SPEC.md` §6.4's three scroll rules, and this repo's `grid.rs` and `hgrid.rs`, whose scroll API already exists and is tested. The `windows` crate's generated `WindowsAndMessaging` bindings (MIT OR Apache-2.0). | **Filed before the first line of input handling**, per §1.5 and the CI gate. No source outside this repository is consulted: no log viewer (forbidden or permitted), no editor, no terminal emulator, no UI toolkit, no vendor sample. The Win32 message semantics used — that `WM_MOUSEWHEEL`'s delta is a multiple of `WHEEL_DELTA` (120) and that `SPI_GETWHEELSCROLLLINES` is the system's lines-per-notch setting — are **the documented behaviour of the API itself** (§3 "Microsoft Learn / Win32 reference", permitted), held as prior knowledge of that reference rather than read from any implementation this session; nothing about *how to build a scrolling view* is taken from anywhere, because `grid.rs` already decides that and §6.4 already argues it. **⚠ `UI-DESIGN.md` §12 also requires `WM_POINTER`/Direct Manipulation for smooth and inertial scrolling; this component deliberately implements only discrete `WM_MOUSEWHEEL`, and the gap is recorded rather than quietly closed.** **Extended 2026-08-13 with the vertical scrollbar**, same component and same sources: `WS_VSCROLL`, `SetScrollInfo`/`GetScrollInfo` and `WM_VSCROLL` are the documented Win32 surface (§3, permitted, prior knowledge of the reference), and the decision to use the **native** scrollbar rather than a drawn one is ours — `Instance` carries no background colour, so a drawn trough needs a new pipeline and shader, while the native control gives position, dragging and keyboard for nothing. `UI-DESIGN.md` §11's density marks cannot be drawn on a native scrollbar and are M5, so this is explicitly an interim control. |

| 2026-08-07 | **Per-monitor DPI (M3)** — process awareness, `WM_DPICHANGED`, and keying the glyph atlas to the scale factor | Claude Opus 5, session 15 | Our own `SPEC.md` §3.1 ("Per-monitor-V2 DPI … all layout metrics recomputed on `WM_DPICHANGED`; the glyph atlas is rebuilt per scale factor; column advances are computed in integer device pixels at the current scale and re-derived on any scale change") and §3.2's atlas key `(glyph id, style, dpi scale)`, plus this repo's `glyphs.rs`, `raster.rs` and `view.rs`. The `windows` crate's generated bindings (MIT OR Apache-2.0). | **Filed before the first line of DPI handling**, per §1.5 and the CI gate. No source outside this repository is consulted: no log viewer, no editor, no UI toolkit, no vendor sample. The Win32 surface used — `SetProcessDpiAwarenessContext`, `GetDpiForWindow`, and that `WM_DPICHANGED`'s `lParam` carries a suggested window rect — is the documented behaviour of the API (§3 "Microsoft Learn / Win32 reference", permitted), held as prior knowledge of that reference. **⚠ §3.1 says per-monitor-V2 is "declared in the manifest" and this declares it by calling `SetProcessDpiAwarenessContext` before any window exists instead.** The two are equivalent here because nothing in this process creates a window earlier, and an embedded manifest would mean adding resource compilation to the build for no behavioural gain — but it *is* a deviation from the spec's wording and is recorded as one rather than passed off as compliance. |

| 2026-08-11 | **Per-row column anchors (M3)** — `ColumnAnchors` in `cell.rs`, the anchored lookups, and carrying them from `rows.rs` through `view.rs` to `paint.rs` | Claude Opus 5, session 15 | Our own `SPEC.md` §5.3 — whose sparse-anchor line index is the design being transposed onto the column axis — plus §3.3 and §10.3, and this repo's `cell.rs`, `index.rs`, `rows.rs` and `view.rs`. | **Filed before the first line**, per §1.5 and the CI gate. No source outside this repository is consulted: no log viewer, no editor, no text-buffer library, no vendor sample. The design is explicitly **our own §5.3 applied one axis over** — sample `(byte, cell)` every K clusters so a lookup is a binary search plus a bounded walk instead of a walk from byte zero — and it is being built because the shipped binary measures 76 ms a frame on a 19.4 KB line containing one non-ASCII character, against a 16.67 ms budget. Nothing about how a text viewer *ought* to do this is taken from anywhere; §5.3 already argued the sparse-anchor trade-off for line numbers and the same argument transfers. |

| 2026-08-13 | **Selection and copy (M3/M5 bring-forward)** — mouse input in the shell, the selected range reaching `paint.rs`, and `Ctrl+C` to the Windows clipboard | Claude Opus 5, session 15 | Our own `SPEC.md` §5.6 (copy preserves the original bytes; content is never discarded silently), §3.3 as amended this session (**a column is a logical position**), §11.1's "copy selection as raw text, preserving original bytes and encoding — v1", `UI-DESIGN.md` §12's mouse and `Ctrl+C`/`Ctrl+Shift+C` bindings, and this repo's `selection.rs`, `cell.rs`, `view.rs` and `rows.rs`. The `windows` crate's generated `DataExchange`/`SystemServices` bindings (MIT OR Apache-2.0). | **Filed before the first line**, per §1.5 and the CI gate. No source outside this repository is consulted: no log viewer, no editor, no text-widget library, no vendor sample. The Win32 clipboard sequence used — `OpenClipboard`, `EmptyClipboard`, `GlobalAlloc(GMEM_MOVEABLE)`, `GlobalLock`, `SetClipboardData(CF_UNICODETEXT)`, `CloseClipboard`, and that the clipboard **takes ownership of the handle on success so the caller must not free it** — is the documented behaviour of the API (§3 "Microsoft Learn / Win32 reference", permitted), held as prior knowledge of that reference. The selection *model* — anchor/focus, stream and block modes, and `byte_span`'s outward rounding so a copy cannot launder a zero-width override — was built and reviewed in session 14 and is unchanged here; this row covers only wiring it to input and to the clipboard. |

| 2026-08-13 | **Following a growing file (M4, E7)** — `follow.rs`, growth detection and index append under §11.3's per-tick budget, plus the `Rows` invalidation it forces | Claude Opus 5, session 15 | Our own `SPEC.md` §5.5 (rotation and truncation, keyed on `FileIdentity` and never on the path), §11.3 (the UI never blocks on indexing; a partial index must serve the viewport it has), §5.3's index structure, and this repo's `file.rs`, `index.rs`, `indexer.rs` and `rows.rs`. The `windows` crate's generated bindings (MIT OR Apache-2.0). | **Filed before the first line of `follow.rs`**, per §1.5 and the CI gate. No source outside this repository is consulted: no log viewer (`tail`, `less`, klogg, LogExpert or otherwise), no file-watching library, no vendor sample. The design is taken from §5.5 and §11.3 as written: poll the open handle's length rather than watching the directory, because §5.5 already argues that rotation detection keyed on the *path* is the bug; scan only the appended range; and stop at a byte budget per tick so a writer producing faster than the frame rate cannot starve the message loop. **This entry covers growth only.** Rotation across all three modes is the next component and gets its own row — it is where the risk is, and merging the two would let the harder half hide inside the easier one. |

## 6. Attestation

Before the first line of code for any component marked CONTAMINATED, append an entry below naming
the author, the date, the sources relied on, and an explicit statement that no GPL implementation
source was consulted for that component.

### Line-offset index — 2026-08-04, Claude Opus 5, session 9

I attest that the line-offset index design now in `SPEC.md` §5.3 was derived from the sources listed
in the 2026-08-04 row of §5 above, and that:

- **No GPL-licensed implementation source was consulted** for this component — not klogg, not
  TailBlazer, not SnakeTail, at any point, in any session I have record of.
- **No log-viewer implementation source of any kind was consulted**, including LogExpert, which §3
  permits. It was not read, so §1.4 does not attach.
- **`docs/RESEARCH.md` §5.3 was not read.** Its technical content has never entered this agent's
  context, consistent with the 2026-07-29 entry in §5 that established the eligibility.
- **`docs/SPEC.md` §5.3 was not read.** It was replaced by line range without being displayed. This
  was a deliberate mechanical choice, not a claim of restraint.
- The two anchors I *was* exposed to — `block-sparse, 128/block` from `SPEC.md` §11.2, and the word
  `delta-encoded` from a diff header — are disclosed in §5 and are the reason the derivation states
  its reasoning against them rather than merely arriving somewhere else.

**What this attestation does not cover.** It speaks only for the derivation of 2026-08-04. It cannot
speak for whether `SPEC.md` §5.3's *superseded* text was contaminated — that remains unknown and
unreconstructable, which is why it was replaced rather than corrected. Session 8 read and edited that
text (`571eb2e`) without filing a §5 entry; if it was contaminated, that exposure is real and
unlogged, and this attestation does not retroactively clear it. The component is clean going forward
because the design was rebuilt, not because the earlier exposure was ruled harmless.

## 7. The dependency allow-list

`deny.toml` is the counterpart to this file. It governs what Tailhawk **links**; everything
above governs what its authors **read**. Both are needed and neither substitutes for the other — a
clean-room process cannot stop a GPL crate arriving transitively, and a licence scanner cannot stop
someone reading GPL source and writing it out again from memory.

**The filename is not cosmetic.** It was `cargo-deny.toml` from 2026-07-29 to 2026-07-31, which is
**not a name `cargo deny` looks for** — it searches for `deny.toml` (or `.cargo/deny.toml`). Nothing
ran the tool until session 8, and when it did, it logged one `[WARN] unable to find a config path,
falling back to default config` and then rejected all 26 crates in the graph, MIT and Apache-2.0
included, because the default allow-list is empty. **A misnamed config fails open in the sense that
matters — it silently stops being the policy — and it took running the tool to find out.**

The config carries no comments, so the reasoning lives here:

- **The list is an allow-list, not a deny-list.** Under `[licences] version = 2`, anything not
  explicitly allowed fails the check. GPL, AGPL and LGPL are therefore rejected without being named,
  and so is any licence nobody has looked at yet. A new copyleft licence appearing in the ecosystem
  fails closed. Naming licences to deny would be the weaker construction — it only rejects what
  someone thought to list.
- **MPL-2.0 is deliberately absent.** It is only file-level copyleft and plenty of permissive
  projects accept it, but accepting it means the outbound "MIT OR Apache-2.0" statement no longer
  describes the whole artefact without qualification. If a genuinely necessary dependency turns up
  under MPL, add it to `exceptions` for that one crate with a note here — not to `allow`.
- **`Unicode-3.0` and `Unicode-DFS-2016` are both present** because `unicode-ident` and its
  relatives changed licence between versions, and the resolved graph can legitimately contain
  either.
- **`unknown-registry` and `unknown-git` are `deny`.** A single-file, copy-and-run binary that
  claims no network I/O of its own should not be built from a source nobody can name. This also
  makes an unreviewed git dependency a build failure rather than a code-review question.
- **`multiple-versions` is `warn`, not `deny`.** Duplicate versions in the graph matter here because
  of the 15 MB binary-size gate in `SPEC.md`, but they are usually somebody else's transitive
  problem and blocking on them would be noise. Revisit if the gate gets tight.
- **`yanked = "deny"`** — a yanked crate is a supply-chain signal, and this project has no deadline
  pressure that would justify shipping one.

## 8. Related

- `LICENSE-MIT`, `LICENSE-APACHE` — the outbound licence.
- `deny.toml` — the config the section above explains.
- `docs/RESEARCH.md` §11 — the record of which research claims were refuted, including the licensing
  ones.
