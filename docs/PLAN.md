# Tailhawk — Development Plan

**Version:** 0.1 (draft for adversarial review)
**Date:** 2026-07-28
**Companion documents:** [`RESEARCH.md`](RESEARCH.md) · [`SPEC.md`](SPEC.md) · [`UI-DESIGN.md`](UI-DESIGN.md)

---

## 1. How this plan estimates

The research produced effort *multipliers* (1.15×, 1.3×, 2.2×) over an assumed 16–24 person-week
baseline. **The critics showed that model does not close arithmetically**: the named components of the
recommended architecture consumed the entire claimed baseline before the indexer, regex engine or
columniser existed. Multipliers are therefore discarded. This plan estimates **bottom-up from
components**, and every number is a named line item that can be argued with.

### 1.1 The agentic adjustment — stated explicitly

Agent-assisted development changes the cost structure, but not uniformly, and the plan is built on
that asymmetry:

| Activity | Effect of agent assistance |
|---|---|
| **Authoring** code against a documented API | **Large reduction.** Win32/D3D11/DirectWrite boilerplate, format definitions, the catalogue, CLI parsing, test fixtures |
| **Transcribing** a known algorithm into a new API | **Large reduction** |
| **Reviewing** code in a domain the developer knows | Moderate reduction |
| **Reproducing** a defect | **No reduction** |
| **Verifying** something visual, timing-dependent or device-dependent | **No reduction** |
| **Deciding** an architecture or resolving a contradiction | **No reduction** |

Consequently: **components dominated by authorship are estimated aggressively; components dominated by
verification are estimated conservatively.** The renderer, the follow state machine and accessibility
are verification-heavy and are *not* discounted. The format catalogue and CLI surface are
authorship-heavy and are.

Two ranges are given per component: **A** (solo developer, agent-assisted, the working assumption) and
**T** (traditional solo developer, for calibration against outside estimates).

### 1.2 Calibration anchors

- **Zed** spent four engineers × six weeks on the Windows renderer alone — on top of an already-working
  cross-platform GPU renderer. That is ~24 engineer-weeks for less scope than §3 below.
- **klogg** — one maintainer, mature Qt doing the widget layer — managed one stable release in 2022 and
  stopped entirely in 2024. This is the cautionary anchor for sustained solo delivery, not a green
  light.

---

## 2. Component estimates

### 2.1 Engine (portable Rust, no UI)

| # | Component | A (weeks) | T (weeks) | Verification weight |
|---|---|---|---|---|
| E1 | Source abstraction, open/share modes, writer-safety guarantee | 1.0 | 2.0 | High — the rotation-loop test is the guarantee |
| E2 | Overlapped `ReadFile` layer on IOCP | 1.0 | 2.0 | Medium |
| E3 | Line index: sparse anchors + forward scan (re-derived 2026-08-04; group-varint rejected, see SPEC §5.3) | 1.5 | 3.0 | Medium |
| E4 | Background + parallel indexer with the code-unit alignment invariant | 1.5 | 3.0 | **High** — silent corruption class |
| E5 | Encoding detection (BOM, NUL-parity, UTF-8 validation, chardetng) | 1.5 | 3.0 | **High** — silent corruption class |
| E6 | Incremental streaming decode, carry across boundaries | 1.0 | 2.0 | **High** |
| E7 | Follow / rotation / truncation state machine | 2.0 | 3.5 | **High** — races, hardest to test |
| E8 | Record model + OTel mapping + severity tables | 1.0 | 1.5 | Low |
| E9 | Format detection pipeline (5 stages, scoring, resilience) | 2.5 | 4.0 | Medium |
| E10 | Format catalogue: 20–40 built-ins with self-testing samples | 1.5 | 4.0 | Low — authorship-dominated |
| E11 | Template compiler (Serilog / NLog / log4net / Logback) + config scan | 1.5 | 3.5 | Low |
| E12 | Pattern DSL compiler | 1.0 | 2.0 | Low |
| E13 | Highlight rule engine, precedence, span merge, per-frame budget | 1.5 | 2.5 | Medium |
| E14 | Filter / sub-view with streaming and cancellation | 2.0 | 3.0 | Medium |
| E15 | Search: parallel chunked, streaming, engine policy, caps | 2.0 | 3.0 | Medium |
| E16 | stdin spill source | 0.5 | 1.0 | Low |
| E17 | CLI grammar, glob expansion, single-instance IPC | 1.5 | 2.5 | Low |
| E18 | Settings, state, three-tier portability, stateless mode | 1.0 | 1.5 | Low |
| | **Engine subtotal** | **25.5** | **47.0** | |

### 2.2 View and shell (Windows)

| # | Component | A (weeks) | T (weeks) | Verification weight |
|---|---|---|---|---|
| V1 | D3D11 device, swapchain, device-removed recovery, WARP chain | 1.5 | 2.5 | **High** — device zoo |
| V2 | DirectWrite glyph atlas, per-DPI rebuild, colour glyph path | 3.0 | 5.0 | **High** — visual |
| V3 | Virtualised grid: u64 scroll model, hit-test, selection | 3.5 | 5.0 | **High** — visual + perf |
| V4 | Cell model: grapheme clusters, East Asian Width, fallback | 2.0 | 3.0 | **High** — visual |
| V5 | Columns: resize, reorder, hide, per-column filter | 1.5 | 2.0 | Medium |
| V6 | RDP Direct2D-on-DC path with scroll-blit *(**scheduled after M7**, not in M4 — it must render the command bar, chips, columns, tabs, palette and detail pane, none of which exist earlier. Building it first guarantees rebuilding it three times.)* | 2.0 | 3.0 | **High** — needs real RDP |
| V7 | Window, Mica chrome, tab strip, drag-reorder, drag-out-to-split | 3.0 | 4.5 | Medium |
| V8 | Command bar, command palette, dialogs, settings surface | 2.5 | 3.5 | Low |
| V9 | Rules editor + format wizard with live preview | 2.5 | 3.5 | Medium |
| V10 | Record detail pane, long-line handling, JSON pretty-print | 1.0 | 1.5 | Low |
| V11 | Per-monitor-V2 DPI, integer cell advances, drift test | 1.0 | 1.5 | **High** — visual |
| V12 | `WM_POINTER` smooth/inertial scroll | 1.0 | 1.5 | **High** — feel |
| V13 | Theming, dark/light, High Contrast suppression | 1.0 | 1.5 | Medium |
| | **View/shell subtotal** | **25.0** | **37.5** | |

### 2.3 Cross-cutting

| # | Component | A (weeks) | T (weeks) |
|---|---|---|---|
| X1 | Test infrastructure: seeded generator, VFS trait, fuzz targets, golden images | 3.0 | 4.5 |
| X2 | CI, build matrix (x64 + ARM64), size gate, perf gate | 1.5 | 2.5 |
| X3 | Signing, packaging, scoop/winget manifests, release process | 1.5 | 2.0 |
| X4 | Documentation, website, format-authoring guide | 1.5 | 2.5 |
| | **Cross-cutting subtotal** | **7.5** | **11.5** |

### 2.3b Components missed by the first draft *(added after adversarial review)*

The cross-document reviewer found ~12 v1 obligations in SPEC and UI-DESIGN with **no line item at
all** — including two on the owner's daily-use list. The widget layer is the largest and most
embarrassing omission: UI-DESIGN draws eleven text fields, a colour picker, context menus, tooltips
and a drop target **on a bare D3D11 surface with no edit control**, and every one of caret, selection,
clipboard, undo and IME is hand-written.

| # | Component | A (weeks) | T (weeks) | Note |
|---|---|---|---|---|
| V14 | **Widget & text-input layer** — text fields (caret, selection, clipboard, undo, IME), colour picker, context menus, tooltips, drop target | **4.0** | 6.5 | **Precondition of V8 and V9** |
| E19 | File sets + watched folders | 1.0 | 1.5 | **Owner daily-use** |
| E20 | Bookmarks with content-hash anchoring | 1.0 | 1.5 | |
| E21 | Export / copy, incl. live tee-matching-lines-to-file | 1.5 | 2.0 | Hoo WinTail parity |
| E22 | Column sort (capped) + top-N | 1.0 | 1.5 | |
| E23 | Zero-config semantic highlight catalogue | 1.0 | 1.5 | |
| E24 | ANSI / bidi / reveal-invisibles sanitisation | 1.0 | 1.5 | Security-relevant |
| E25 | Multi-line record assembly + continuation predicates | 1.5 | 2.0 | |
| E26 | **Filter expression grammar + parser** | 1.5 | 2.0 | Currently undefined — see §7 |
| E27 | View-state history (Alt+←/→) | 0.5 | 0.5 | |
| E28 | i18n string externalisation | 0.5 | 0.5 | |
| E29 | **`--stdout` GNU tail parity + differential test harness** | 3.0 | 4.5 | RESEARCH §7.3 costed this at 3–5 weeks and no line item carried it. **v2** — v1 ships `-n -f -F -c` only |
| V15 | **Minimal UIA chrome provider** — the only automated interaction-test surface v1 has | 2.0 | 3.0 | Was deferred wholesale to v2 |
| **E30** | **Rolling sets (SPEC §5.5b)** — pattern inference from siblings, ordering detection incl. log4net's descending case, continuous scrollback across members, index spanning the set, drain-then-switch on roll, bounded eager history with on-demand backfill, retention-deletion tolerance | **2.5** | **4.0** | **v1.** Serilog's and NLog's *default* rotation creates new filenames rather than rewriting a path — without this the tool sits on a dead file against a default-configured .NET app |
| | **Subtotal (v1 items only — E29 is v2)** | **19.0** | **28.0** | |

### 2.4 Totals

| | A (agent-assisted) | T (traditional) |
|---|---|---|
| Gating experiments (§3) | 2.5 | 3.0 |
| Engine | 25.5 | 47.0 |
| View / shell | 25.0 | 37.5 |
| **Missed components (§2.3b, v1 only)** | **19.0** | **28.0** |
| V6 re-estimate (+0.5) | 0.5 | 0.5 |
| Phase 0 re-estimate (+1.5) | 1.5 | 1.5 |
| Cross-cutting | 7.5 | 11.5 |
| **v1 total** | **81.5 person-weeks** | **128 person-weeks** |
| | **≈ 19 months solo at a sustainable pace** | ≈ 29 months |

*Cross-check: the milestone cumulative weeks in §4 sum to the same 81.5. If you edit either, edit both.*

**The first draft said 60.5 weeks. It was wrong by 24%**, because the cost basis omitted a dozen
things the spec and the UI design both require. That is exactly the failure the bottom-up method was
supposed to prevent, and it was caught only by a reviewer checking the component list against the
other two documents feature by feature. Treat the 75 as still likely low.

**v2** (merged view, trace correlation, process sources, archives, comparison, alerts, full `--stdout`
parity, **accessibility**) adds **≈ 28–36 weeks A**, of which accessibility alone is 6–10.

**v3** adds **≈ 16–22 weeks A**.

### 2.5 Honesty about these numbers

- They are **estimates, not commitments**, and the ±40% band applies.
- The largest single risk is **V2+V3+V4 (8.5 weeks A)** — the renderer and grid. Zed's calibration
  point suggests this could be double. If the **M3** milestone (§4) overruns by more than 50%, the
  plan says stop and reconsider rather than push on. *(This said M2 until session 15 — residue from
  the decode-before-index resequencing that moved the grid out of M2. The gate itself is stated
  correctly at §4's M3; only the back-reference was stale.)*
- **Nothing here is discounted for "agents will write it".** The A column already contains that
  discount; applying it twice is how plans fail.
- The owner's stated appetite is *full vision, however long it takes*. This plan therefore optimises
  for **a coherent shippable product at each milestone**, not for compressing the total.

---

## 3. Phase 0 — gating experiments (4 weeks)

*An earlier draft said 2.5 weeks for all six. That was not credible: G5 alone is provisioning a clean
machine, building a defined 30-file corpus and measuring two incumbents with cold/warm separation, and
G6 is a hands-on study of a week's real usage. 4 weeks, and G6 runs in the background throughout
rather than blocking.*

**Nothing in §4 starts until these complete.** Each has a defined pass/fail and each can invalidate a
design decision that would otherwise be expensive to unwind.

| # | Experiment | Pass criterion | If it fails |
|---|---|---|---|
| **G1** | **SMB stale size.** Writer on host A appends to a share; reader on host B polls `GetFileSizeEx` on its own open handle. Measure observed latency vs. actual writes, with and without a periodic handle reopen. | Handle-based size reflects appends within one poll interval | UNC design gains mandatory periodic handle close/reopen; §5.4 of SPEC is rewritten and the UNC latency budget increases |
| **G2** | **Read throughput.** Overlapped `ReadFile` vs mmap on Windows 11 + NVMe, 10 GB, cold and warm. *(The no-mmap decision is already made on correctness grounds — a section handle blocks the writer's rotation. This measures the cost of that decision so it is stated honestly, not so it can be reversed.)* | **Informational — no pass threshold.** RESEARCH verified `pread` at ~⅔ of mmap on Linux; an earlier draft invented a "within 25%" threshold with no source, on the experiment evidencing the largest architectural decision in the spec | Nothing. The rotation-blocking risk cannot be eliminated, so a large gap changes the *marketing*, not the design |
| **G3** | **Binary size floor.** Build three hello-worlds — windows-rs+D2D, eframe+glow, eframe+wgpu — with `opt-level="z"`, `lto="fat"`, `panic="abort"`, `strip=true`, `+crt-static`. Record real `.exe` bytes and cold-start-to-first-pixel. | windows-rs+D2D under 2 MB, and **under 40 ms** to first pixel for an empty window *(an earlier draft set 100 ms here, which is SPEC §11.3's budget for the **whole app** painting real content — a hello-world consuming the entire app budget is a fail, not a pass)* | The 15 MB CI gate and SPEC §11.3's first-paint budget are both re-derived from measurement |
| **G4** | **Glyph atlas composition — colour and mono in one pass.** Rasterise a monochrome alpha atlas via DirectWrite *and* a colour atlas via `TranslateColorGlyphRun` (COLR/CBDT emoji), and render a viewport mixing both. Measure atlas eviction under a CJK-heavy fixture that overflows the atlas mid-frame. | One instanced draw per viewport is preserved, or the cost of the extra pass is measured and accepted; eviction does not stall a frame | Re-cost V2. *(This experiment was previously a ClearType pixel-diff. The owner de-prioritised text antialiasing, and spending Phase-0 on it while leaving the genuine V2 risk untested was backwards: a premultiplied colour atlas **cannot share the mono atlas's blend state**, so it breaks the one-instanced-draw rule that the whole renderer design rests on.)* |
| **G5** | **Incumbent re-measurement.** BareTail 3.50a and LogExpert 1.41.0, identical hardware, defined 30-file corpus. Record private working set **and** commit size, plus wall-clock open/close. | — | Establishes the only honest competitive baseline; all current claims rest on a 2019 forum comment |
| **G6** | **Hoo WinTail hands-on.** The owner's installed copy: which features are used in a week, and exactly how its encoding detection behaves against a crafted file set. *(Runs in the background across Phase 0 — it is a week of real usage, not a task.)* | — | May add or remove v1 scope |
| **G7** | **Reproduce egui #1391.** Confirm whether the >2M-row jitter is f32 precision in the thumb mapping, in accumulated scroll offset, or in row-height accumulation. | The actual cause is identified | Adopting "u64 thumb, no f32 accumulation" is correct regardless — but knowing *which* f32 broke it tells us which one to avoid in our own grid. Cheap; the whole grid design rests on the diagnosis, which RESEARCH §3.4 marks **[L]**, not [V] |

**G1 and G4 are the two that can change the architecture.** G1 gates the UNC design; G4 gates whether
the one-instanced-draw renderer survives colour emoji.

---

## 4. Milestones

Each milestone has a **demo** (what you can show), a **definition of done**, and an explicit statement
of what remains reversible.

### M0 — Skeleton (2 weeks, cumulative 6)
Cargo workspace with the core/shell split from commit one. A window that opens, a D3D11 device with
the WARP fallback chain, an embedded shader, `+crt-static`, CI producing a signed-placeholder x64 and
ARM64 `.exe` under the size gate.
**Done:** `tailhawk.exe` opens a window on a clean Windows 10 1809 VM with no runtime installed.
**Reversible:** everything. No engine decisions committed.

> **⚠ The first draft of this sequence had a dependency inversion that adversarial review caught.**
> SPEC §5.3 states encoding must be resolved **before chunk assignment**, yet M1 built and signed off
> the indexer while encoding did not land until M4 — thirteen weeks later. M1's and M2's done-criteria
> would both have passed against ASCII fixtures and then silently failed on the first UTF-16LE file at
> week 26.5, throwing away completed, *tested* work. The sequence below is corrected: **decode before
> index.**

### M1 — Read and decode (4.5 weeks, cum. 10.5)
E1, E2, **E5, E6**. Headless. Opens a file, detects encoding, streams decoded lines with correct carry
across read boundaries. The writer-safety rotation-loop test passes.
**Done:** the encoding fixture matrix (BOM'd, BOM-less UTF-8, BOM-less UTF-16LE, UTF-32LE, mixed,
truncated-mid-sequence, binary-embedded, DBCS) decodes correctly; fuzz targets clean; rotation loop
shows no sharing violation on the writer side.
**Committed:** no-mmap, share modes.

### M2 — Index (4 weeks, cum. 14.5)
E3, E4, E8. The block-sparse index — **re-derived clean-room per RESEARCH §5.3's contamination
notice**, with `CLEANROOM.md` populated *before* a line is written. Parallel indexing honours the
code-unit alignment invariant because encoding already exists.
**Done:** index a 10 GB fixture with bounded memory (§11.2 of SPEC); a 4 GB BOM-less UTF-16LE fixture
indexed on 8 threads produces byte-identical offsets to a single-threaded run.

### M3 — Grid (11 weeks, cum. 25.5) ⚠ **highest-risk milestone**
V1–V4, V11. The virtualised grid rendering real decoded content at 60 fps with per-token colour,
correct grapheme/EAW cell model, per-DPI atlas, column-drift test passing.
**Done:** scroll a 50M-line file smoothly; drag between 100% and 150% monitors with no drift; the
CJK/RTL/emoji fixture renders correctly; **no f32 accumulation anywhere in scroll state** (see the
egui #1391 gating experiment).
**Gate:** if this overruns by >50% (>16.5 weeks), **stop and reconsider the stack** — the
egui-chrome-plus-custom-grid fallback is still cheap here, because the grid is hand-written either way.

### M4 — Follow, rolling sets and stdin (5 weeks, cum. 30.5)
E7, **E30 (rolling sets)**, **E16 (moved forward from M8)**. Live tailing with correct rotation,
truncation and the per-tick work budget; the rolling-set source; pipe ingestion. *(V6, the RDP path,
has moved to M7b — it needs a UI to render.)*

**E30 belongs here, not later.** All three rotation modes are one state machine, and roll-to-new-name
is the mode that Serilog and NLog use *by default* — so without it M4's done-criterion ("tail through
rotation without losing a byte") is only true for log4net-style writers.
**Rationale for moving stdin:** Docker and Azure Container Apps are one of the owner's three stated
workloads, and the pipe is the container integration surface — **Tailhawk never speaks a backend
protocol.** At M8 the tool could not touch two thirds of the stated workload until week 50.
**Done:** tail through **all three** rotation modes — copy-truncate, rename-and-recreate, and
roll-to-new-name — without losing a byte; a default-configured Serilog rolling sink is followed across
a roll with continuous scrollback into the previous member; 50 MB/s for 60 s without dropped frames;
`docker logs -f svc | tailhawk -` pipes in and scrolls back.
**Specify explicitly:** `az containerapp logs show --follow` **terminates and exits 0 on replica
restart**, which currently renders as a cleanly completed stream. A pipe source must distinguish
*writer finished* from *writer died mid-stream*, and offer reconnect for the process-spawn source
in v2.

### M5 — Search, highlight, filter (10 weeks, cum. 40.5)
E13, E14, E15, **E23 (semantic highlights), E24 (ANSI/bidi), E26 (filter grammar + parser)**, V5, and
the command bar. **First daily-useful milestone.**
**Done:** full-file regex search streams first match on a 10 GB fixture; a pathological lookaround
degrades to "pattern too slow, truncated" rather than hanging; include + exclude + multiple composing
text filters all expressible.

### M6 — Structure (10.5 weeks, cum. 51)
E9–E12, **E25 (multi-line assembly)**, V9. Format detection, catalogue, template import, config
scanning, format wizard.
**Done:** an unmodified real-world Serilog file, a log4net file, an NLog file and an IIS W3C file each
columnise on open with no configuration; MEL Simple two-line records assemble correctly; the
build-time detector cross-matching test passes.

### M7 — Shell completion (16 weeks, cum. 67)
**V14 (widget & text-input layer) first — it is a precondition of V8 and V9** — then V7, V8, V10,
V12, V13, **V15 (minimal UIA chrome provider, built alongside the chrome so it can test it)**, plus
**E19 (file sets, watched folders), E20 (bookmarks), E21 (export/copy), E22 (sort + top-N),
E27 (view-state history)**.
**Done:** the owner's daily workflow from Hoo WinTail is fully reproducible, and the chrome has
automated interaction coverage rather than one person dragging a mouse.

### M7b — RDP path (2 weeks, cum. 69)
V6. The Direct2D-on-DC reduced-fidelity path with scroll-region blitting, against the now-complete UI.
**Done:** a real RDP session over a bandwidth-limited link renders the full UI at ~15 Hz with
scroll-blit; no hover-only affordance exists anywhere to defeat it (UI-DESIGN §10b).

### M8 — CLI and portability (3 weeks, cum. 72)
E17, E18, E28. Tail-compatible flags, single instance, three-tier settings, stateless mode, i18n
externalisation.
**Done:** `tailhawk -n 100 -f app.log` behaves; running from a read-only UNC share works with the
stateless indicator.

### M9 — Harden and ship (9.5 weeks, cum. 81.5)
X1–X4, security controls (SPEC §13), fuzzing, the perf gate, packaging, documentation.
*(V15, the minimal UIA chrome provider, lands in **M7** rather than here — it is the automated
interaction-test surface, so it must exist while the chrome is being built, not after.)*
**Done:** v1.0 published and installable via `scoop install tailhawk`. Signing added when a route is
confirmed (§7) — **it does not gate this milestone.**

---

## 5. Points of no return

| Decision | Reversible until | Cost of reversing after |
|---|---|---|
| **Stack (windows-rs + D3D11 + DirectWrite)** | **End of M3** | Most of the view layer — ~8–10 weeks |
| **No-mmap read path** | M1 | Low — the read layer is behind a trait |
| **Line index codec** | M1 | Low — index is rebuildable |
| **OTel record model** | M4 | Medium — every format mapping touches it |
| **Product name** | **Now — it is committed** | SmartScreen reputation reset, package IDs, org rename |
| **Signing identity** | Before first signed release | Publisher reputation resets to zero |
| **Second platform shell** | Indefinitely | The core/leaf seam keeps it open; adding one is additive |

**The name and the signing identity are the two that are effectively permanent**, and the signing
identity is unresolved (§7).

---

## 6. Risk register

| # | Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|---|
| R1 | **M3 renderer overruns** (Zed: 4 engineers × 6 weeks) | High | High | Explicit 50% overrun gate; egui-chrome fallback stays viable because the grid is hand-written either way |
| R2 | **SMB polling is unreliable** (G1 fails) | Medium | High | Gating experiment before design is committed; periodic-reopen fallback designed in advance |
| R3 | **No code-signing route confirmed.** Azure Artifact Signing excludes South Africa entirely; Certum OSS eligibility unconfirmed; any signed route needs a physical token shipped internationally | **High** | Medium — blocks the first *signed* release, not the first release | Confirm Certum eligibility now; price one OV fallback; **ship v1 unsigned via scoop** and add signing when a route is confirmed. Choose the identity once and never change it |
| R4 | **Scope drift toward observability** | Medium | High | OTLP receiver explicitly deferred and gated; positioning statement in SPEC §1.1 |
| R5 | **Solo maintainer burnout** — klogg's exact failure mode | Medium | Fatal | Each milestone ships something usable; M5 is daily-useful at week 32 |
| R6 | **Accessibility deferred to v2 blocks adoption** | Medium | Medium | Stated openly; UIA is also the only UI-test surface, so it is scheduled, not dropped |
| R7 | **Colour emoji / CJK breaks the cell model late** | Medium | High | V4 is a first-class M3 component with a fixture, not an afterthought |
| R8 | **A GPL clean-room violation** from klogg-derived design | Low | **Severe** — legal | `CLEANROOM.md`, specification from published docs only, `cargo-deny` licence gate |
| R9 | **Performance claims cannot be substantiated** | Medium | Medium | All targets [TBM]; dedicated perf box; no published number without a measurement |
| R10 | **WSL / VS Code "good enough"** wins adoption | Medium | Medium | Differentiate on columnisation + multi-GB + merged view, which neither does |
| R11 | **The owner's day job.** This plan assumes a sustainable solo pace for 18 months on a personal project alongside full-time work | **High** | **High** | Milestones are sized so each is independently completable; M5 (week 38) is the first daily-useful build. Treat 18 months as elapsed-time-at-part-time, not 18 months of full-time effort |
| R12 | **Losing interest after the interesting part.** The renderer (M3) is the intellectually engaging work; packaging, docs, support and the long tail of M9 are not | **High** | High | M5 being daily-useful means the tool earns its keep long before M9. Publish early and unsigned rather than holding for a polished 1.0 |
| R13 | **A single unmaintained crate** in the dependency graph (`encoding_rs`, `chardetng`, `windows-rs`) | Low | Medium | `cargo-deny` gates on maintenance status; vendored for release builds; each has a documented hand-roll fallback path |
| R14 | **First ten users all want different things**, pulling scope in ten directions | Medium | Medium | SPEC §1.3 non-goals are explicit and public. The success bar (§8) is the owner's own use, not user count |

---

## 7. Immediate actions (before M0)

1. **Resolve the code-signing route.** **Azure Artifact Signing is ruled out** — its geographic
   eligibility covers neither South African organizations nor South African individuals, so no
   legal-entity decision unlocks it. Confirm **Certum Open Source Code Signing** eligibility for a
   South African individual (from €69, hardware token shipped), and price one conventional OV
   alternative as a fallback. Note the token must be physically shipped and received, which is
   itself a lead time nobody has counted. **Shipping unsigned for v1 is a legitimate posture** —
   scoop imposes no signing requirement and SmartScreen warns on new *signed* binaries anyway — so
   this blocks the first *signed* release, not the first release.
2. **Check `tailhawk.com` / `.dev` / `.io`** at a registrar. Never verified.
3. **Claim the namespaces:** GitHub org `tailhawk`, crates.io `tailhawk`, scoop and winget IDs. Verified
   free as of the naming round, but "verified free" decays.
4. **Fix the reference perf machine** so [TBM] targets can become numbers.
5. **Run G6** (Hoo WinTail hands-on) — it is free, it may change v1 scope, and it is the only real user
   research available.
6. **Decide the repo licence** and add `LICENSE-MIT`, `LICENSE-APACHE`, `CLEANROOM.md`, `deny.toml`.

---

## 8. What success looks like

**v1 is successful if the owner stops opening Hoo WinTail.** That is the honest bar, it is measurable,
and it is more useful than any download count.

Secondary, in order:
1. Opens and follows a 10 GB log with bounded memory, on a UNC share, without breaking the writer's
   rotation — the thing no incumbent does correctly.
2. Columnises an unmodified real-world Serilog file on open with no configuration.
3. Runs from a copied `.exe` on a machine with no runtime, no install and no admin rights.
4. Initiates **no outbound connection of its own** — no telemetry, no update ping, no CDN fetch —
   verifiably, with network I/O occurring only to sources the user explicitly opened. *(Not "zero
   network connections": UNC is a v1 source.)*
5. Opens a `docker logs -f` pipe and scrolls back through it — the container third of the stated
   workload, which had no success criterion at all in the first draft.
