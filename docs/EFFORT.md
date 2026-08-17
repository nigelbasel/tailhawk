# Effort — estimated against actual

`PLAN.md` estimates in **person-weeks**. Nothing recorded what the work actually cost, so "are we
ahead or behind" has been unanswerable. This file closes that, and is meant to be **appended to at
the end of every session** — one row, four numbers.

## What the numbers mean, and what they do not

**Active hours** is the sum of gaps between consecutive transcript messages, **capped at 5 minutes**.
A gap longer than that is the machine idle between prompts, not work, and counting it would let a
session left open overnight report 14 hours. It is a proxy for *engaged agent time* and it is the
honest one to compare across sessions.

**⚠ It excludes the owner's own time** — reading, deciding, reviewing, the thinking between sessions.
For a solo project driven by prompts that is a real omission and probably the larger number. Treat
active hours as *machine* effort, not project effort.

**Output tokens** is what the model generated: code, prose, tool calls. It tracks work done far
better than total tokens, because **cached input dwarfs everything** — 1.17 B of it against 4.8 M
output, a 245:1 ratio. Cached reads are the cheap part; quoting the billion is misleading.

Sessions before 2026-08-11 are **reconstructed** from `~/.claude/projects/*.jsonl`, not recorded at
the time. The method is above and is reproducible, but it was derived after the fact.

## Actual, to date

| Date | Active h | Output tok | Commits | Delivered |
|---|---:|---:|---:|---|
| 2026-07-28 | 1.5 | 563 k | 1 | Research, spec skeleton |
| 2026-07-29 | 4.3 | 984 k | 16 | SPEC/PLAN/RESEARCH, cleanroom process, egui + D3D11 experiments |
| 2026-07-30 | 1.3 | 334 k | 14 | Glyph atlas experiment, M0 window + device + two-stage paint |
| 2026-07-31 | 1.4 | 272 k | 9 | Encoding detection, file source (M1) |
| 2026-08-04 | 2.2 | 259 k | 4 | Line decoder, rotation and share-mode handling |
| 2026-08-05 | 4.1 | 747 k | 10 | Line index, parallel indexer (M2) |
| 2026-08-06 | 2.5 | 679 k | 14 | Cell model, glyph pass, shaping, raster |
| 2026-08-07 | 5.9¹ | 940 k¹ | 11 | Grid, selection, bidi, hgrid, view, text pass |
| 2026-08-11 | ¹ | ¹ | 5 | Rows, DPI, input, anchors, 50M-line run |
| 2026-08-11 night | ¹ | ¹ | 7 | Offscreen test path, ASCII fast walk, review, effort record |
| 2026-08-13 | ² | ² | 12 | Follow, rotation by identity, scrollbar, selection wiring, activity log |
| 2026-08-14 | ² | ² | 19 | M4 finished (follow tick, E30, E16, off-thread scan); M5 E26, E15, E13, MODE_SOLID |
| 2026-08-16 | 1.2⁴ | 287 k⁴ | 6 | Search wired to the UI: the span pass, `find.rs`, the keys, the 10 GB worker criterion |
| 2026-08-17 | 13.5⁴ | 2.6 M⁴ | 105 | The find harness run at last; E23; E14; E24; open-at-tail, Ctrl+O + drop, the wndproc guard; M5 scored; **M6 delivered less V9**; M6 scored, M7 begun — **V14** text field + focus, the **command bar** on real fields, IME, headless snapshot, **tabs**, status bar, chips toggle, E19 watched folders, E28 settings, `--filter/--exclude`; then the **gutter + E20 bookmarks**, **V8 command palette**, colour labels, **E27 history**, **V10 detail pane**, **E21 export/tee** + TSV copy; **split view**, **V13 theming**, E17/E18, glyphs, smooth wheel, chip edit, column resize, drag-reorder, **V15 UIA + verify-uia.ps1**, user rules file; **M7 scored**; welcome surface, saved column widths, `docs/USING.md` |
| **Total** | **~50.6** | **~10.0 M** | **238**³ | **M0–M7 (less E22, V9's editor UI), plus M8's E17, E18, E28** |

⁴ **Measured while the session was still running**, so it is a floor rather than a total — 710 turns
between 14:31 and 15:49, by the method above. Recorded now rather than at the end because the last
row was written at a pause that then lasted two days; a floor in the table beats an exact figure that
never gets added. It also excludes the on-screen verification, which could not be run here.

³ From `git rev-list --count HEAD`, which is the authority. It exceeds the per-row sum by 3 — the
early rows were reconstructed by date and two 2026-08-12 commits never got a row at all. The rows are
left as recorded rather than back-fitted to the total.

² 2026-08-13 and 2026-08-14 are likewise **one continuous session** — session 16, measuring **11.4
active hours and 1.94 M output tokens** in total. It cannot be split by day either. All of M4 was
done by the 9.9 h / 1.66 M mark, which is the figure M4 is scored against below; the remaining
1.5 h / 0.28 M is the four M5 items.

¹ 2026-08-07 and 2026-08-11 are **one continuous session**, and the transcript does not separate
them. Its combined figure is **6.6 active hours and 1.12 M output tokens** across 2,355 turns, which
is the largest single session by a wide margin and covers most of M3 plus the whole overnight run.
Splitting it would need per-turn attribution the extraction does not attempt.

**⚠ The overnight portion is agent time with no owner time at all**, which makes it the cleanest
datapoint in the table for "what does an agent-hour deliver" — and the least representative of how
the project actually runs, since every other row had a human steering it turn by turn.

## Estimated against actual

`PLAN.md` budgets **25.5 person-weeks** for M0–M3. At a nominal 40 h week that is ~1,020 hours,
against **~23 active hours** here.

**Do not read that as a 44× speed-up.** It is not comparing like with like:

- The plan's weeks include work not yet done inside those milestones — no scrollbars, no selection
  wiring, no clipboard, no file-open dialog. The *milestone exit criteria* are largely met; the
  milestones as a human would scope them are not.
- RTL placement is unimplemented and disclosed, not delivered.
- It excludes all owner time, which for a prompt-driven project is the real constraint.
- Quality is not held equal by assumption. What supports the comparison here is that the exit
  criteria were **measured on the shipped binary** — 50M lines, DPI, frame budgets — rather than
  asserted.

The defensible statement is narrower and more useful: **M0–M3's exit criteria cost ~23 hours of
agent time and ~4.8 M output tokens.** That is the number to extrapolate from, and the honest
multiplier against `PLAN.md` is unknown until a milestone is delivered *complete* rather than to its
exit criteria.

## 📌 M4 — forecast registered 2026-08-11, **before starting**

Bottom-up, in the same units, so it can be scored rather than remembered generously. Each item is
sized against a **measured** analogue from the table above rather than from feel.

| Item | Analogue used | Active h | Output tok |
|---|---|---:|---:|
| Follow state machine — growth detection, index append, §11.3's per-tick budget | line index, 2026-08-05 | 2.5 | 400 k |
| Rotation, all three modes, keyed on `FileIdentity` | file source + share modes, 2026-07-31/08-04, **harder** | 3.5 | 550 k |
| Rolling sets (E30) — a set of files as one stream with scrollback across members | no close analogue; new modelling | 3.0 | 450 k |
| stdin / pipe ingestion (E16), including the §4.2 spill | line decoder, 2026-08-04 | 2.0 | 300 k |
| Shell wiring — follow toggle, `⬤ Following` chip, `Rows` cache invalidation | input handling, this session | 1.0 | 150 k |
| Verification — 50 MB/s × 60 s, a real Serilog rolling fixture, a rotating writer harness | the 50M-line run, this session | 1.5 | 250 k |
| **Total** | | **13.5** | **2.1 M** |

**This is 3× the naive extrapolation below, and the naive one is wrong.** Scaling the plan's weeks
linearly gives ~4.5 h, because M0–M3 delivered *exit criteria* in 23 h. M4's criteria are harder to
satisfy than to implement: "tail through three rotation modes without losing a byte" and "50 MB/s for
60 s without dropped frames" both need **harnesses that do not exist** — a writer that rotates on
demand, and a sustained-throughput rig. Roughly a quarter of the estimate is test apparatus.

**Confidence: medium-low.** Named risks, in order:

1. **Rolling sets ripple into the index.** `LineIndex` and `offset_of_line` assume one byte space. A
   set spanning members either needs a member-aware offset or a virtual concatenation, and that
   reaches `rows.rs` too. This is the item most likely to double.
2. **The `Rows` fetch cache is knowingly wrong for following.** Its key is
   `(first, count, line_count, anchored)`, which cannot see §5.5's copy-truncate — contents changing
   under a stable line count. I flagged that when I wrote it; M4 must invalidate explicitly.
3. **50 MB/s may be the first real load on the indexer's append path**, which has only ever been
   exercised by a batch build.
4. Corpus A rotates naturally under observation, which is a genuine asset for (1) and an
   identity-scrubbing hazard for anything written down.

Score this against actuals when M4 lands, and record **why** it was wrong rather than only by how
much.

### Scored: M4 — forecast 13.5 h / 2.1 M, actual **9.9 h / 1.66 M**

**Both under, by about a quarter, and the forecast was the first one written before the work rather
than after it.** Four of the five done-criteria were met inside M4; the fifth — "without dropped
frames" — was closed a few hours later by the **first item of M5's forecast**, which was sequenced
first for exactly that reason. Its 2.5 h sits in M5's column, so M4's 9.9 h is four fifths of the
milestone and the honest way to read the pair is 12.4 h for all of it against a 13.5 h forecast.

Where it went, and why the shape is more useful than the total:

| | Forecast | Comment |
|---|---:|---|
| Follow state machine | 2.5 h | About right |
| Rotation, three modes | 3.5 h | Two modes came in here; the third arrived free with E30 |
| **Rolling sets (E30)** | **3.0 h** | Named "most likely to double". Cheapest item in the milestone |
| **stdin (E16)** | **2.0 h** | Also cheap, for the same reason |
| Shell wiring | 1.0 h | Real, but folded into E30 and E16 rather than separate |
| Verification | 1.5 h | Understated if anything — three harnesses, and they found three defects |

**The two items forecast as hardest were the two that went fastest, and it was the same cause both
times: a spec sentence that let a new source sit *beside* the existing machinery instead of inside
it.** §5.5b orders members oldest-first, so a rolling set is a prefix sum over per-member indices and
`index.rs` never changes. §4.2 says the spill "reuses the same index path as a real file", so a pipe
becomes a file and `stdin.rs` is a byte-copier. In both cases the estimate's implicit model was "a
new source kind needs new source machinery", and in both cases the spec had already said it does not.

**What the forecast missed entirely:** the presentation of state that has started changing. The title
had to survive a roll, a truncation, a member retirement and an end-of-stream, and the frozen version
of it was a real defect found by screenshot. Nothing in the estimate had a line for that, and it
follows mechanically from making anything dynamic — worth a line in the next one.

**Risks 2 and 3 cost nothing, for reasons worth keeping.** Risk 2 — the `Rows` cache keyed on
`line_count`, blind to copy-truncate — never needed the explicit invalidation it was forecast to
need: a truncated member is *replaced wholesale* by `reseat_live`, and its `Rows` goes with it, so
the stale cache cannot survive the event that would have made it wrong. Risk 3 was real but landed
before the milestone as a separate finding: the append path was fine, and what capped throughput at
~40 MB/s was one byte-budgeted scan per timer tick — a *scheduling* bound, not an indexing one.

Across all four named risks, none cost what was feared and the two real costs (the throughput ceiling
and the dynamic title) were unnamed. **The forecast's risk list was better at listing what looked
hard than at predicting what would be expensive** — which argues for keeping it (it is cheap and it
is a record) while sizing from the *item* estimates rather than inflating them for named risk.

### Interim: M4 is feature-complete — 2026-08-14

| Item | Forecast h | Forecast tok | Actual |
|---|---:|---:|---|
| Follow state machine | 2.5 | 400 k | done 2026-08-13 |
| Rotation, three modes | 3.5 | 550 k | done 2026-08-13; the third mode actually landed with E30 |
| **Rolling sets (E30)** | **3.0** | **450 k** | **~0.6 h wall clock, 4 commits** — see below |
| **stdin (E16)** | **2.0** | **300 k** | **~0.9 h wall clock, 3 commits** |
| Shell wiring | 1.0 | 150 k | folded into E30 and E16 rather than separate |
| Verification | 1.5 | 250 k | the 50 MB/s rig, the rolling-set harness, the pipe harness |

Wall clock is not active-agent hours and is not comparable to the table above — the session-level
figure comes from the jsonl parse at the end. It is recorded because it is what exists now, and
because the *ordering* it shows is the interesting part: the two items forecast as hardest were the
two that went fastest.

**Why E16 was cheap, and it is the same reason as E30.** §4.2 says the spill "reuses the same index
path as a real file". Taken literally that makes `stdin.rs` a byte-copier — no tailing, indexing,
decoding or rendering — because the pipe becomes a file and everything already built applies. Both
items came in low by **declining to build a parallel path**, and in both cases the spec sentence that
made it possible was already there to be read.

The estimate's implicit model was "a new source kind needs new source machinery". Neither did.

### Interim: E30 landed, and risk 1 did not happen — 2026-08-14

| | Forecast | Actual |
|---|---:|---|
| Rolling sets (E30) | **3.0 h** | ~0.6 h of wall clock across four commits (10:17 → 10:51), plus the shell wiring the forecast counted separately |

**Risk 1 was named as "the item most likely to double" and it cost less than a quarter.** The
forecast assumed a set spanning members needs "a member-aware offset or a virtual concatenation, and
that reaches `rows.rs` too". It needs neither: **one `LineIndex` per member and a prefix sum on top**
leaves `index.rs`, `indexer.rs` and `rows.rs` untouched, because §5.5b's own ordering rule — members
oldest-first — puts the growing member last, where growth appends and renumbers nothing.

The lesson is about how the risk was framed, not about optimism. The forecast reasoned from "the
index assumes one byte space" to "so the index must change". The actual move was to **not** span the
index at all, and that option was invisible from inside the framing. **A risk stated as a property of
existing code ("`LineIndex` assumes X") predicts cost only if the new feature has to live inside that
code.** Worth asking, next time a risk is written down: is this a constraint, or is it a boundary the
new thing can sit beside?

Two things the forecast did *not* name and that did cost real time: the **stale title** (found by
screenshot, not by any test — see `HANDOFF.md`), and getting `describe()`, the byte count and the
encoding flag to survive a roll. All three are "the presentation of state that now changes", which is
a category the estimate had no line for and which follows mechanically from making anything dynamic.

## 📌 M5 — forecast registered 2026-08-14, **before starting**

Same units, same rule: written before the work so it can be scored. Sized against measured analogues
in the table above, and against what M4 taught — item estimates, not inflated-for-risk ones.

| Item | Analogue used | Active h | Output tok |
|---|---|---:|---:|
| **Off-thread work** — a cancellable background pass that streams results to the UI | none; architectural. Also **closes M4's unmet criterion** | 2.5 | 400 k |
| E26 — filter grammar + parser (§7.2) | `pattern.rs` this session (0.6 h) scaled for a real grammar; `record.rs` | 2.0 | 300 k |
| E15 — search: parallel chunked, streaming, two-engine policy, caps (§7.4) | the parallel indexer, 2026-08-05 | 3.0 | 450 k |
| E14 — filter / sub-view, streaming and cancellable (§7.3) | `set.rs`'s row space (0.6 h), plus a *derived* row space and both view modes | 2.5 | 400 k |
| E13 — highlight rule engine: precedence, span merge, per-frame budget (§7.1) | `cell.rs` column anchors; reaches `paint.rs` | 2.0 | 300 k |
| E23 — zero-config semantic highlight catalogue | boilerplate-shaped; `record.rs`'s severity tables | 1.5 | 200 k |
| E24 — ANSI / bidi / reveal-invisibles sanitisation | `bidi.rs` exists; the ANSI parser does not | 1.5 | 200 k |
| V5 — columns: resize, reorder, hide, per-column filter | `hgrid.rs`; **see risk 2** | 2.5 | 350 k |
| The command bar | input handling, 2026-08-11; new shell UI | 2.0 | 300 k |
| Verification — 10 GB search fixture, a pathological lookaround, composing filters | the 50 MB/s rig, this session | 2.0 | 300 k |
| **Total** | | **21.5** | **3.2 M** |

`PLAN.md` budgets M5 at **10 weeks**, twice M4's five, and this forecast is 2.2× M4's actual — so it
is roughly in proportion, which is the first time a forecast here has had a delivered milestone to be
proportional *to*.

**Confidence: low**, and lower than M4's was, for reasons that are about this milestone rather than
about forecasting:

1. **Highlighting needs a render capability that does not exist.** `Instance` carries no background
   colour; selection is a foreground re-tint standing in for a highlight, and `SPEC.md` §3.2's plan
   is one instanced draw carrying both. E13 and E23 both want a real background span, so **the shader
   and the offline `fxc` build change**. This is the item most likely to double, and unlike M4's
   named risk it is a capability gap rather than an assumption about existing code.
2. **V5 columns has no producer.** Columns come from a detected format, and format detection is
   `PLAN.md`'s E9/E10 in **M6**. Either V5 slips out of M5 or a minimal detector is pulled forward;
   the plan does not say which, and it is the sort of dependency that is cheapest to name now.
3. **Off-thread work touches everything.** `Document` lives on the window thread and owns the
   `LogSet`. A background pass needs its own reader over the same bytes — `LogFile` is `Send + Sync`
   and reads positionally, which is what makes this possible at all, but the ownership question is
   real and is the first thing to settle.
4. **`fancy-regex` is a new dependency** for §7.4's lookaround escape hatch, so it needs a
   `deny.toml` licence decision before any code assumes it.

Score this when M5 lands, and record **why** — M4's scoring found the risk list was better at naming
what looked hard than at predicting what would be expensive, and one milestone is not enough to know
whether that repeats.

### Scored: M5 — forecast 21.5 h / 3.2 M, actual **~5.9 h / ~1.1 M** — 2026-08-17

The four M5 items of 2026-08-14 (1.5 h / 0.28 M), session 17 (1.2 h / 0.29 M) and session 18
(~3.2 h / ~0.5 M to this point). **Nine of the ten items landed; V5 columns did not, and could not** —
risk 2 said so before the milestone started: columns have no producer until M6's detection, and V5
moves into M6 formally below.

**Against the plan's own criteria:** full-file regex search streams its first match on 10 GB
(0.062 s, whole pass 4.1 s) ✅; a pathological lookaround degrades to "truncated" rather than
hanging ✅; include + exclude + composing text filters expressible ✅ — and beyond "expressible", the
filter runs over 10 GB (first survivor 0.117 s, 7.5 s whole) and hides rows on screen. Highlighting
reaches pixels beneath search matches. Every one of those was **observed on the shipped binary**, not
inferred.

**Why the forecast was 3.6× high, and it is the same shape as M4:** the risk list named what looked
hard, not what was expensive. Risk 1 (backgrounds need a shader change) did not happen — `MODE_SOLID`
was one instance mode. Risk 3 (off-thread ownership) was solved once by `find.rs`'s snapshot and then
reused twice. Risk 4 (`fancy-regex`) was a `deny.toml` line. **What actually cost time was not on the
list:** four separate harness traps (DPI virtualisation, a bare `Alt` swallowing keys, `Add-Content`
against a tailed file, a stale title instrument), a `RefCell` re-entered under a modal dialog, and a
Windows PowerShell round-trip that mojibake'd a source file. Verification and tooling, not the
engineering, is where the unforecast hours went — and they were still a small fraction of the
forecast's engineering hours. Two milestones now say the same thing: **halve the engineering
estimate and add a fixed verification tax**, roughly 20 % of the total, for the shipped-binary checks
this project insists on.

**Recorded honestly:** M5's chrome is deviations, not features — no find bar, no chip row, no
debounce, typed-into-the-window fields — all noted in `CLEANROOM.md`; and E24's SGR colours are
parsed and not drawn.

## 📌 M6 — forecast registered 2026-08-17, **before starting**

Same units, same rule. Halved engineering against the plan's item weeks, per the two scorings above,
plus the verification tax.

| Item | Analogue used | Active h | Output tok |
|---|---|---:|---:|
| E9 — detection pipeline (§6.1: sample window, self-describing short-circuits, catalogue scoring, resilience) | `pattern.rs` (0.6 h) for the shape; `record.rs` exists; **the 5-stage design is written** | 2.0 | 300 k |
| E10 — format catalogue: built-ins with self-testing samples (Serilog, log4net, NLog, MEL, IIS W3C, syslog, CLF, JSON lines, logfmt) | `semantic.rs` (1.0 h): boilerplate-shaped, authorship-dominated; each format is a regex/template plus a sample the build tests | 2.0 | 300 k |
| E11 — template compiler for Serilog / NLog / log4net / Logback output templates, and config scanning | `filter.rs`'s parser (2.0 h) is the analogue for a small grammar; config scanning is file I/O | 2.0 | 300 k |
| E12 — pattern DSL compiler | E11's cousin | 0.8 | 120 k |
| E25 — multi-line record assembly + continuation predicates | `lines.rs` carry (0.5 h) scaled; touches the index (records vs lines) — **risk 1** | 1.5 | 250 k |
| V5 — columns: resize, reorder, hide, per-column filter, now with a producer | `hgrid.rs`; painter draws cells per column; **no widget layer**, so resize/reorder are keys | 2.0 | 300 k |
| V9 — rules editor + format wizard with live preview | **needs V14 (M7)** — a wizard is a form. Expect the same deviation as the find bar: a typed, title-echoed stand-in, or defer | 1.0 | 150 k |
| Verification — the four real files of the done-criterion, the cross-matching detector test, `verify-columns.ps1` | ~20 % | 2.5 | 350 k |
| **Total** | | **13.8** | **2.1 M** |

**Confidence: low-medium.** Higher than M5's was, because the analogues are real now — a parser, a
catalogue, a worker pass, a painter that takes spans. Lower than it could be for these reasons:

1. **Records versus lines is the structural risk.** Everything today addresses a *line*: the index,
   the search, the sieve, the row space. E25's multi-line records (a stack trace under its ERROR
   line) either introduce a second row space — records over lines, the way `kept` sits over file
   rows — or make the index record-aware. The first is cheaper and consistent with how the filter
   was done; decide it *first*, before E9, because the record boundary is what the detector emits.
2. **The catalogue is authorship**, and the done-criterion is "unmodified real-world files": the
   owner's own Nexus / NDC / JobDispatcher logs are the corpus (`tailhawk-dogfood-corpus` memory),
   and their formats decide which templates matter. Sample early.
3. **V9 has no widget layer.** Say so up front rather than discover it at week three.
4. **The 150 ms open budget** (§6.1's latency rule): detection must not delay first paint. The
   worker that opens a document is where it runs; the shell must paint from the head sample and
   re-render if the decision changes.

Score this when M6 lands.

### Scored: M6 — forecast 13.8 h / 2.1 M, actual **~1.8 h / ~0.3 M**, V9 outstanding — 2026-08-17

Same session as the forecast, a few hours later. **E9, E10, E11, E12, E25 and V5 landed and every
line of the done-criterion is met** — Serilog, log4net (the owner's real file), NLog and IIS W3C
columnise on open, MEL Simple two-line records assemble under collapse, the cross-matching build
test passes. **V9** (rules editor + format wizard) is not started: it is a form, and forms are V14.

**7.7× high, and the reason is not the same as M4's and M5's.** Those two over-forecast because
verification cost what engineering did not. This one over-forecast because **the analogues were
already the work**: E9's scoring is a fold over `Format::validity`; E10 is a table; E11's three
compilers share one builder; E25 and V5 are `kept`'s row space and `Highlighter::beneath` again;
the header is a `View` inset. Every M6 item was "the shape from M5, with different work in the
middle", and a shape that exists costs a fraction of a shape being found. **Halve again for a
milestone whose shapes exist; keep the verification tax; expect the *first* item of a milestone
that needs a new shape (V14 in M7) to cost what the whole of M6 did.**

Two things worth the record: **§6.3 contradicted itself** (specificity inside a score that had to
reach 0.75 — six formats could never be accepted) and was amended when the detector was built; and
the owner's own log detects as a log4net layout no built-in knew until it was added — the catalogue
is authorship and the corpus decides what to author.

## 📌 M7 — forecast registered 2026-08-17, **before starting**

Same units, same rule. **This is the milestone the shortcut runs out on**: every field, chip, tab
and dialog so far has been typed into the window and echoed in the title, and V14 — a text-input
layer with caret, selection, clipboard, undo and IME — is a shape this codebase does not have.
Everything after it is that shape reused.

| Item | Analogue used | Active h | Output tok |
|---|---|---:|---:|
| **V14** widget & text-input layer — a focus model, a text field (caret, selection, clipboard, undo, IME composition), buttons/chips/labels, tooltips, context menus, hit-testing, drawn by the same painter | none that fits: `selection.rs` for the caret model, `paint.rs` for drawing; IME is Win32 (`WM_IME_*`) — **the new shape** | 4.0 | 600 k |
| **V8** command bar: the find field, the chip row (add/edit/toggle/reorder/remove), the format chip with its dropdown, the palette (`Ctrl+K`), dialogs | V14 reused; every action already exists behind a key | 2.0 | 300 k |
| **V7** window chrome: tab strip (several documents), drag-reorder, drag-out-to-split, Mica where available | `Shell` holds one `Document`; a `Vec` and an active index; the split is a second `View` over a second `Document` | 2.5 | 400 k |
| **V10** record detail pane, long-line handling, JSON pretty-print | a second painter viewport; `record.rs` | 1.0 | 150 k |
| **V12** `WM_POINTER` smooth/inertial scroll | `grid.rs`'s `(u64, f32)` scroll is built for sub-row deltas | 0.8 | 120 k |
| **V13** theming: dark/light, High Contrast suppression | every colour is a `lib.rs` const today; a `Theme` struct and two tables | 0.8 | 120 k |
| **V15** minimal UIA provider for the chrome | Win32 `IRawElementProviderSimple`; the harnesses become tests | 1.5 | 250 k |
| **E19** file sets + watched folders, **E20** bookmarks, **E21** export/copy + tee, **E22** sort/top-N, **E27** view-state history | `set.rs`, `sieve.rs`, `find.rs` shapes | 2.5 | 400 k |
| **V9** (from M6) rules editor + format wizard, on V14 | `highlight.rs`, `template.rs` exist; the wizard is a form | 1.2 | 200 k |
| Verification — a harness per surface, on the shipped binary; the frame instrument with chrome on | ~20 % | 3.5 | 500 k |
| **Total** | | **19.8** | **3.0 M** |

**Confidence: low**, for a reason the two scorings above make specific: this is the first milestone
since M3 whose first item is a **new shape**, and the record says new shapes cost what whole
milestones of reused ones do. V14 is where the estimate is most likely to be *under*, not over.
The rest is the reuse pattern and is more likely over than under.

Score this when M7 lands — and score V14 on its own when it does, since it decides the rest.

### Scored: M7 — forecast 19.8 h / 3.0 M, actual **~7 h / ~1.3 M**, E22 and V9's editor UI outstanding — 2026-08-18

The same day as the forecast, one long session. **Landed:** V14 (fields, focus, IME), V8 (bar, chips
with toggle/edit/drag, **palette**), V7 (tabs, drag-reorder, middle-click close, **split view**), V10
(detail pane, JSON re-indent), V12-lite (eased wheel; no `WM_POINTER`), V13 (dark/light/High
Contrast, `--theme`), **V15** (UIA chrome provider + `tools/verify-uia.ps1`, which passes on the
shipped binary without a free desktop), E19, E20 (+ severity glyphs), E21 (export + live tee, TSV),
E27; and from M8, E17, E18, E28. **Not landed:** E22 sort/top-N; V9's rules *editor* and format
wizard as UI — the rules file (`tailhawk.rules.toml`) and the config import stand in; column
reorder and saved widths; Mica.

**~2.8× high, and the shape held.** V14 — the new shape — cost about 1.5 h, not 4: a text field is
a well-known thing, and the caret model was `selection.rs`. Everything after it was the reuse
pattern (each item 20–60 min). The forecast's verification line (3.5 h) was mostly not spent on the
desktop — it was busy all evening — but on the headless snapshot path and the UIA harness, which
between them cost ~1 h and are now the standing verification for the chrome. **The one thing this
scoring adds to the record:** a session that does not stop between items lands about three items
an hour of this size; the earlier scorings' hours were dominated by the stops.

Two defects the work found, worth the record: **the gutter stretched every frame** for two commits
(the shader was told the grid's width without the gutter — seen only because a headless shot was
compared with the previous one); and **a whole-line background flattened every rule under it**
(the highlighter's claim took the ink with the background) — fixed with `claim_bg`, which tints
under the ink.


## Forecast, on this evidence

| Milestone | Plan | Naive extrapolation | Confidence |
|---|---:|---:|---|
| ~~M4 follow, rotation, stdin~~ | 5 wk | forecast 13.5 h, **actual 9.9 h**; **all five criteria**, the last closed by M5's first item | **Delivered** 2026-08-14 |
| ~~M5 search, highlight, filter~~ | 10 wk | forecast 21.5 h, **actual ~5.9 h**; 9 of 10 items, V5 moved to M6 with its producer | **Delivered** 2026-08-17 |
| ~~M6 structure~~ | 10.5 wk | forecast 13.8 h, **actual ~1.8 h**; every done-criterion line met, V9 to M7 with V14 | **Delivered** 2026-08-17 |
| M7 shell | 16 wk | **forecast 19.8 h / 3.0 M** (registered above) | **Low** — V14 is a new shape |
| M7b–M9 RDP, CLI, ship | 14.5 wk | — | **Very low.** Packaging, signing, accessibility and support have no agent-time analogue in what has been measured |

The extrapolation is linear in the plan's own weeks, which assumes the remaining work resembles the
work done. **M6–M9 does not** — it is packaging, docs, installers and a long support tail, where
agent time is not the binding constraint.

## Keeping this current

At the end of a session, from `~/.claude/projects/C--dev-git-TailHawk/`:

```bash
perl -ne '
  BEGIN{ use Time::Local; @t=(); $out=0; }
  if (/"timestamp":"(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2}):(\d{2})/) { push @t, timegm($6,$5,$4,$3,$2-1,$1); }
  if (/"output_tokens":(\d+)/) { $out+=$1; }
  END{ @t = sort { $a <=> $b } @t; my $a=0;
       for my $i (1..$#t) { my $d=$t[$i]-$t[$i-1]; $a+=$d if $d<=300; }
       printf "active_h=%.1f output_tok=%d\n", $a/3600, $out; }
' <session-id>.jsonl
```

Then add one row. **Estimate first, in the same units, before starting a milestone** — a forecast
written afterwards is not a forecast, and the point of this file is to find out how wrong the
estimates are.

