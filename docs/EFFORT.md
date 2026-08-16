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
| **Total** | **~35.9** | **~7.0 M** | **127**³ | **M0–M4, plus 4 of M5's 10 items** |

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

## Forecast, on this evidence

| Milestone | Plan | Naive extrapolation | Confidence |
|---|---:|---:|---|
| ~~M4 follow, rotation, stdin~~ | 5 wk | forecast 13.5 h, **actual 9.9 h**; **all five criteria**, the last closed by M5's first item | **Delivered** 2026-08-14 |
| M5 search, highlight, filter | 10 wk | ~9–10 h | **Low** — regex over 10 GB is a different kind of problem from anything done so far |
| M6–M9 structure, shell, ship | 41 wk | — | **Very low.** Packaging, signing, accessibility and support have no agent-time analogue in what has been measured |

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
