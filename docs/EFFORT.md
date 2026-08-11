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
| **Total** | **~23** | **~4.8 M** | **84** | **M0–M3, 31.6 k lines inserted** |

¹ 2026-08-07 and 2026-08-11 are one continuous session and the transcript does not separate them.

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

## Forecast, on this evidence

| Milestone | Plan | Naive extrapolation | Confidence |
|---|---:|---:|---|
| M4 follow, rotation, stdin | 5 wk | ~4–5 h | Medium — rotation is fiddly and has real corpus risk |
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
