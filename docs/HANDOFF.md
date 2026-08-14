# Handoff — resume here

## ✅ Rolling sets (E30, §5.5b) — 2026-08-14, session 16

**Tailhawk follows a set of files as one log**, which §5.5b says no incumbent does — Hoo WinTail's
folder monitoring opens rolled files as *separate documents*. This is the M4 item the forecast
flagged as most likely to double, and it did not.

| Verified on screen | |
|---|---|
| Serilog-shaped set, opened at `log_002.txt` | all 4 lines of both members, in writing order |
| Roll onto `log_003.txt` | **`gen2 LAST WORDS written after the roll decision` is in the scrollback** |
| Still following afterwards | 9 lines across 3 files, view pinned to the tail |
| log4net rename-and-recreate | **`gen1 LAST WORDS after the rename` sits between gen1 and gen2** |
| Title after either roll | tracks the set: `3 files — oldest is log_001.txt, newest is log_003.txt` |

### The drain finally does something

The previous entry (below) said the drain was ordered correctly and achieved nothing, because
`reattach` threw the drained index away. **That is now false, and the two "LAST WORDS" rows above are
the evidence.** Lines written to a file *after* the writer decided to roll — and, for log4net, after
the file had already been renamed away — reach the screen. §5.5's "this is where naive tools lose the
last KB" is delivered rather than merely ordered for.

The roll test is what holds it: `follow` re-seats onto the new member, so anything not drained first
is not late, it is **gone**. Removing the `drain_live` call fails that test and no other.

### Three files, three rules

- **`pattern.rs`** — members and order from names alone. Two families with different rules, because
  §5.5b's trap is that date sets count up and log4net's backups count *down*, and "getting this
  backwards silently presents history in reverse". `members()` is always oldest-first so no caller
  carries the direction. Numbers compare as **numbers** — an unpadded sequence reads 1, 10, 2 under a
  byte sort, which is the same silent reversal from the other side.
- **`set.rs`** — one `LineIndex` per member and a prefix sum on top. A single index across the set
  would need a synthetic byte space and a translation on every lookup, in the path that runs per
  frame; the prefix sum is a binary search over a handful of `u64`s, and because members are
  oldest-first the live one is last, so growth appends and moves nothing.
- **`main.rs`** — `Document` holds a `LogSet` and nothing else source-shaped. A lone file is a set of
  one, so there is one path and not two.

### ⚠ What is not done, and is not hidden

- **On-demand backfill.** §5.5b's eager bound (newest 10 members or 512 MB) is enforced; members past
  it are excluded and counted in the title (`; N older not indexed`), not backfilled as the user
  scrolls in. One amendment to §5.5b is argued in `set.rs`: **the eager window always contains the
  file the user opened**, or somebody who double-clicks `log-20260101.txt` in a year-long set gets
  the December files and not that one.
- **The event-driven directory watch.** §5.5b wants `FILE_NOTIFY_CHANGE_FILE_NAME`; a re-listing
  throttled to 1 s stands in, because `ReadDirectoryChangesW` needs a thread the portable half cannot
  own. A roll is noticed up to a second late — late, not lost, because the drain reads to EOF
  whenever it happens.
- **The separator row and the per-member gutter.** `LogSet::locate` returns the member index, the
  per-member line number and `starts_member`; nothing draws any of it. Rendering feature.
- **NLog's unpadded mid-name sequence** (`log.1.txt`) is not recognised as a set, and §5.5b's
  `by-mtime` fallback is never *inferred*. Both are argued at `FIELD_MIN_DIGITS`: `service2.log`
  beside `service3.log` is two services, and ordering files we cannot identify by a timestamp that a
  copy rewrites is worse than saying "one file".
- **Members in a sibling directory** (§5.5b's NLog `archive/` row). One directory listing only.
- **Archived `.gz`/`.zip` members** stay out until §4.3 exists to read them.

### What the screenshot caught that the tests could not

The title's set description was built from the inference made at open, so a window showing three
files went on saying "2 files … newest is `log_002.txt`". §5.5b wants the set **shown for
confirmation**, and a confirmation that has stopped being true is worse than none — it invites a
check against a list that has moved on. Now built from the members actually held, with two tests.
Same pass: the encoding-disagreement flag had been dropped in the port, and the byte count reported
the live member's alone, so it *fell* at every roll and read as data lost.

**This is the fourth time visual verification has found something no test would.** The running tally
is in the traps section.

---

## ✅ M4 so far — following and rotation — 2026-08-13, session 15

**Tailhawk tails.** It follows a growing file, survives two of §5.5's three rotation modes, and has
been pointed at its own activity log while that log was being written.

| | |
|---|---|
| Following a live writer | 1 line/16 bytes → 61 lines/2047 bytes, view pinned to the bottom |
| Rename-and-recreate | reattached, then followed generation 2 |
| Copy-truncate | reattached, then followed generation 3 |
| Tailing `logs/agent.log` | 48 lines/5717 bytes → 56/6509 while being appended to |

**Ordering is the whole of both features.** `is_following` is derived from being at the bottom, so it
must be read **before** `set_total_rows` takes the new count — after it, the old position is never
the bottom and a tail would never pin. Rotation is checked **before** growth, or a replaced file's
length gets applied to the old file's scan position.

The `Rows` cache needed no explicit invalidation for growth: its key carries `line_count`. It still
cannot see copy-truncate under a stable count, which is why rotation is detected separately.

### ⚠ The drain is ordered correctly and achieves nothing yet — **superseded 2026-08-14**

> Kept as written because it is the record of what was true then. Rolling sets closed it; see the
> entry above.


§5.5 says the old handle is drained to EOF before switching — "this is where naive tools lose the
last KB" — and it is. **But the drained lines go into an index that `reattach` immediately discards**,
because in a single-file view the previous file is no longer the document. `rotation.rs`'s test
proves the bytes are *reachable* through the old handle after the rename; nothing yet keeps them.

The requirement only becomes real with **§5.5b rolling sets**, where the old member stays in the
scrollback. The drain is kept because it is the correct order and would have to be reinstated
verbatim — not because "never lose the last KB" is delivered. It is not.

### ⚠ Also outstanding in M4

- **Roll-to-new-name — Serilog's and NLog's *default*.** Invisible to file-side detection: the
  written-to path never changes identity and never shrinks. `a_roll_to_a_new_name_is_invisible_to_
  file_side_detection` asserts that rather than leaving it looking covered. Needs §5.5b.
- **The "file truncated" separator row** §5.5 asks for. A rendering feature, not attempted.
- ~~**Rolling sets (E30)**~~ — done 2026-08-14, see the entry above. **stdin (E16)** — untouched.
### 📏 50 MB/s × 60 s — run, and it found a real ceiling

The suspicion in the previous entry was right and worse than stated. One `FOLLOW_BUDGET_BYTES` scan
per 100 ms tick is ~40 MB/s, so **the design could not have met the criterion however fast the
machine was** — and `Poll::Grew::more` already existed to say "call me again" while nothing did.

`Follow::poll_for` now loops the byte-budgeted steps under a **time** budget, which is the bound
§11.3 actually asks for: the UI must stay responsive, not each scan be small.

| Tick budget | Written | Indexed | UI p95 | UI max |
|---|---|---|---|---|
| 8 ms | 3,000 MB @ 50 MB/s | 2,464 MB — **fell behind** | 47.7 ms | 238.5 ms |
| **30 ms** | 3,000 MB @ 50 MB/s | **3,145,710,009 B — the whole file** | 44.1 ms | **90.3 ms** |

The worst stall *improved* when the budget grew, because it was no longer perpetually catching up.
32,430,001 lines indexed.

**⚠ Half the criterion is met and half is not.** "50 MB/s for 60 s" — yes, fully indexed and level
with the writer. "**without dropped frames**" — **no**: a p95 of 44 ms is about 22 fps during the
flood, and a 60 fps frame is 16.67 ms. The scan and the paint share one thread, so a 30 ms scan tick
and a 16.67 ms vsync cannot both fit in a frame. Fixing it properly means moving the scan off the UI
thread, which is a real change and is not attempted.

## 📓 There is an activity log now — `logs/agent.log`

The owner asked to be able to see what is being done while it happens, rather than only in the
summary afterwards. `tools/agentlog.sh` appends one timestamped line per meaningful action:

```bash
tools/agentlog.sh INFO edit "cell.rs — hoisted the ASCII singleton test"
tail -f logs/agent.log        # or open it in Tailhawk
```

**`logs/` is gitignored.** It is a running record for the owner, not project history — the commit
messages and this file are where history belongs, and duplicating it would guarantee the two drift.

**The format is deliberately an ordinary log rather than JSON**, because the point of it is to be
read by *Tailhawk itself* once following lands in M4: ISO-8601 UTC with milliseconds, a level, a
short action word, and free text. It exercises timestamp parsing, level colouring and long lines on
content the owner actually cares about — the best dogfood fixture available, and one that cannot
leak employer or customer identity the way the real corpora do.

**⚠ It only works if it is written to as work happens.** A log filled in retrospectively at the end
of a session is a summary wearing a timestamp, and is worth less than nothing because it looks live.

## 🌙 Overnight session, 2026-08-11 → 12 — **decisions taken without you, for review**

The owner left the session running and asked for assumptions and decisions to be written down rather
than presented as finished work. Everything in this block is a judgement call made **without
confirmation**, and each says what would have to be true for it to be wrong. Read this before the
entries below it.

**Everything is committed to `master` in small steps, tests green at each.** Nothing here is
irreversible; every item is a normal revert.

### The plan, and why this order

1. **Make the cluster walk cheap for mostly-ASCII lines.** This is the last *measured* gap: vertical
   paging while scrolled right costs ~44 ms. Caching anchors by row number — the fix named in the
   previous entry — **would not have worked**, and noticing that is the first decision of the night:
   a page-down shows 48 rows never seen before, so there is nothing to hit in a cache. The
   irreducible cost is one walk per new row, so the walk itself has to get cheaper.
2. **Cache resolved bidi levels per row.** The smallest honest step toward RTL, and independently
   correct: it stops shaping being a function of horizontal scroll position.
3. Adversarial review of both, then documentation and effort figures.

### ✅ 1. The cluster walk got cheap, and the last measured gap closed

**Every horizontal case is now vsync-bound.** On the 19.4 KB non-ASCII fixture:

| | Before anchors | With anchors | With the fast walk |
|---|---|---|---|
| Vertical paging while scrolled right | 76.44 ms | 44.49 ms | **16.12 ms** |
| Horizontal scrolling at the line end | 76 ms | 15.91 ms | 16.63 ms |
| Column 0 | 15.88 ms | 16.24 ms | 15.84 ms |

**The mechanism.** Building a row's anchors is one full walk, and `grapheme_indices` plus a
`unicode-width` lookup **per cluster** is what it cost — ~19,400 of each per row. But for a log line
almost every cluster is a single ASCII byte whose answer needs no library call: UAX #29 breaks
between any two ASCII characters except GB3's `CR × LF`, and every single-ASCII-character cluster is
one cell wide in **both** cell models. So a byte whose neighbours are both ASCII is emitted directly,
and only the neighbourhoods of non-ASCII characters go through real segmentation.

**Both neighbours are checked and both controls fire.** Dropping the *following* byte's check breaks
`CR LF`; dropping the *preceding* byte's breaks GB9b, where a `Prepend` character such as `U+0605`
absorbs the ASCII after it. Both controls happened to fail first on the same fixture, which would
have left GB9b looking covered while untested — so the test now asserts up front that both awkward
clusters really are clusters before it compares anything.

`the_fast_walk_and_the_plain_one_agree` checks the fast walk against `cells` — the canonical one —
over 22 fixtures under both models, chosen to attack the argument rather than confirm it.

**Re-verified at 50M rather than argued.** The fast walk only runs while anchors are being built, and
anchors are only built while scrolled right, so the 50M vertical case *cannot* have regressed — which
is exactly the sort of reasoning this project distrusts. Re-ran it: 5.24 GB, **67 MB resident, 119
rows a frame at 17.35 ms**, viewport at the true end of the file. Index took 3.32 s against 2.35 s
and 2.55 s previously, and page-down 17.35 ms against 16.74 ms; **the adversarial review was running
fifteen agents building concurrently**, so that drift is the machine and not the change. Recorded
rather than quietly reporting the better earlier figures.

### ⚠ 1b. The review found the fast walk was a **regression** on non-ASCII lines, and it was right

A 12-agent adversarial review returned **one confirmed finding**, and it is the useful kind: the
verifier measured rather than argued.

The fallback branch buffered every cluster of a span into a `Vec` before yielding any, and on a line
with **no ASCII singleton anywhere** — all-CJK, or any non-ASCII/ASCII alternation where every ASCII
byte has a non-ASCII neighbour — that span is the whole line. Measured on 3 MB: ~10–16× the line's
bytes held transiently, and **26% slower than the plain walk it replaced** (67.2 ms against 53.2 ms).
A pure regression, time and memory, on exactly the shape §10.3 names as the one klogg hangs "deadly"
on.

**Fixing the memory half was not enough, which is the part worth knowing.** Dropping the `Vec` and
segmenting one cluster at a time made it O(1) in memory and *still* 18% slower — building a fresh
grapheme iterator per cluster costs more than the single cursor `cells` carries. There is no
per-byte repair; the choice has to be made before the walk starts.

So `walk` now decides **per line**: a vectorised count of non-ASCII bytes, and anything above a
quarter goes to the plain `cells`. The threshold is a threshold, not a measurement — a cluster is a
singleton only if *both* neighbours are ASCII, so singletons collapse well before the byte count
does, and the two shapes it must separate sit at opposite ends of it.

| 3 MB unless noted | `cells()` | `walk()` |
|---|---|---|
| all-CJK | 50.9 ms | 53.3 ms — delegates |
| CJK/ASCII alternating | 48.9 ms | 48.0 ms — delegates |
| ASCII | 119.7 ms | **9.7 ms** |
| mostly ASCII | 118.9 ms | **20.9 ms** |
| **19.4 KB real log line** | 794 µs | **62.7 µs** |

**Two of the reviewer's throwaway probes were kept**, renamed and documented, because they are
stronger than what they were checking:
`every_ascii_character_is_one_cell_in_both_models` enumerates all 128 rather than sampling, and
`the_fast_walk_agrees_on_every_short_hostile_combination` runs **244,904 strings** over a
segmentation-hostile alphabet against both models. One `git add -A` swept them into an unrelated
commit while the workflow was still running — a process failure now in the traps table.

### ⛔ 2. RTL was **not** attempted, and that is the night's main judgement call

The plan above had "cache resolved bidi levels per row" next. On looking at what it is *for* I
stopped, and the reasoning matters more than the outcome:

- Caching levels has **no consumer** until RTL placement exists, so on its own it is code written for
  a use that has not been decided.
- Doing the placement as well would be a **partial** implementation. `paint.rs` would put RTL runs in
  visual columns while `View::position_at` — the hit-test — still returned logical ones, so **clicks
  would land on the wrong character.** Today every part of the system is consistently logical:
  visually wrong for RTL, but coherent, and disclosed through `Laid::rtl_runs`. A half-change trades
  a known, announced gap for a disagreement between what is painted and what is clicked.
- Fixing *that* is the coordinate-space question put to the owner earlier — is a column a logical or
  a visual position — and the answer decides work in `selection.rs`, `cell.rs` and `view.rs`. "Go
  ahead with your recommendations" was said about the 50M run, not about this.

**So RTL is unchanged: unimplemented, disclosed, and the last M3 exit criterion outstanding.** The
recommendation on the table is also unchanged — logical `Position`, a per-row bidi map used only by
paint and hit-test, and whole-line level resolution as its first step, because bidi cannot be
resolved from a horizontal slice.

### ⚠ Assumptions a reviewer should challenge

- **That making the walk faster beats precomputing columns at index time — untested, not settled.**
  The alternative records `(byte, cell)` anchors while indexing, making a deep column O(1) instead of
  a walk. It was rejected in one sentence — "costs memory on every line of a 50M-line file for a case
  most files never hit" — and **that sentence was never measured**. It is a plausible argument, which
  is the category this module's history says to distrust.

  The naive version really is expensive: anchors for all 50M lines would dwarf the 6.3 MB line index.
  But three cheaper variants were dismissed without being considered, and the third looks close to
  free: anchor only lines **above a length threshold**; anchor only lines that are **not all-ASCII**,
  since ASCII columns are already O(1); or build them **lazily on first deep access** and cache.

  **Left alone deliberately, owner's call 2026-08-13.** Every measured case is now vsync-bound —
  column 0, line end, horizontal scrolling, and 50M vertical — so this would be optimising against a
  hypothetical. If a real corpus later shows deep-column scrolling hurting (CJK-heavy, or lines well
  past 19 KB), **the lazy cache is the first thing to try** and is cheap to add then.
- **That RTL's design decision stands as recommended** — logical `Position`, a per-row bidi map used
  only by paint and hit-test. That was put to the owner and the reply was "go ahead with your
  recommendations", which was in the context of the 50M run rather than an explicit ruling on RTL.
  **Step 2 is deliberately chosen to be useful under either answer** and commits to nothing.

## ✅ Column anchors — the non-ASCII half of the horizontal cost — 2026-08-11, session 15

`ColumnAnchors` is `SPEC.md` §5.3's sparse line index transposed one axis over: sample `(byte, cell)`
every 64 clusters when a row is fetched, so a column lookup is a binary search plus a bounded walk
instead of a walk from byte zero. `Rows` builds them, a new `RowSource` trait carries them to the
painter, and `view.rs` uses them for both of the lookups it makes per row per frame.

**Measured on the shipped binary, 19.4 KB lines each opening with one non-ASCII character:**

| | Before | After |
|---|---|---|
| **Horizontal scrolling at the line end** | 76 ms | **15.91 ms** — vsync-bound |
| Vertical paging at column 0 | 15.88 ms | **16.24 ms** — unchanged |
| Vertical paging *while* scrolled right | 76.44 ms | **44.49 ms**, then **16.12 ms** once the walk itself got cheaper — see the overnight entry |

**The middle row is the one that nearly went wrong.** A first version built anchors on every fetch,
which regressed column-0 paging from 16 ms to **39 ms** — because `byte_span`'s early exit stops
after the ~150 clusters the viewport shows, while building anchors visits all 19,400. It was more
work in the overwhelmingly common case to speed up a rarer one. The fix is that `anchored` is part of
the fetch cache key: anchors are built only while the view is scrolled right, and changing that flag
forces the one refetch that builds them.

### ⚠ The third row is still over budget, and the reason is structural

Vertical paging while scrolled right rebuilds every row's anchors, because the rows change. Building
anchors is **one full cluster walk** — the same order as the lookup it replaces — so this halves the
cost (two walks become one walk plus two cheap lookups) and cannot remove it. Closing it needs either
anchors cached **by row number across fetches**, or a cheaper `cluster_width`. Neither is done, and
2.5× over budget while paging through long non-ASCII lines is the honest current state.

### Two things this turned up

- **`Rows` no longer re-reads a viewport it already holds.** `Document::lay_out` calls `fetch` every
  frame, and a horizontal scroll changes no row — so every frame was re-reading and re-decoding a
  megabyte to arrive at what it already had. The cache key is `(first, count, line_count, anchored)`;
  it deliberately does **not** detect §5.5's copy-truncate, where contents change under a stable line
  count, and following (M4) will have to invalidate explicitly.
- **The `lay_out` closure became a trait.** `FnMut(u64) -> Option<String>` could return neither the
  text by reference nor the anchors at all, so it allocated a `String` per row per frame. `RowSource`
  returns both by reference and `Rows` implements it.

**The `before_cell` control is worth reading before touching that binary search.** Using `<=` instead
of `<` makes `byte_span(0..1)` on `"\u{202E}abc"` — §13.4's Trojan Source line — return `3..4`
instead of `0..4`, **silently dropping the attacker-supplied bidi override from the copied bytes**.
That is the §5.6 loss `byte_span`'s outward rounding exists to prevent, reintroduced through an
off-by-one in an index lookup, and `an_anchor_never_changes_an_answer` catches it.

## ✅ The horizontal scroll cost, measured in the shipped binary and half fixed — 2026-08-11, session 15

The residual layout cost had been an *estimate* since the adversarial review. It is now a
measurement, taken on the real binary against a 10,000-line corpus of **19.4 KB lines** — §10.3's
supported inline size, the shape it calls klogg "deadly" on.

| Fixture, viewport at the end of the line | Before | After |
|---|---|---|
| 19.4 KB **ASCII** lines | **75.96 ms** | **16.17 ms** — vsync-bound |
| The same lines with **one non-ASCII character** at the start | 75.96 ms | **76.44 ms** — unchanged |
| Either, at column 0 | ~16 ms | ~16 ms |

Column 0 was always fine and returning to it always recovers, so the cost tracks the offset exactly
as predicted. **4.7× over budget at 13 fps, and it is now 60 fps — for ASCII.**

**The fix is an identity, not a heuristic.** For a line of ASCII containing no `\n`, column *n* **is**
byte *n*: every ASCII byte is its own grapheme cluster (`CR LF` is the only ASCII exception and a
line cannot hold `\n`), and every ASCII cluster is one cell — printable by width, control by
`cluster_width`'s first rule, which fires *before* the zero-width check and so holds under §13.4's
reveal toggle too. `cell_at_byte`, `byte_at_cell`, `byte_span` and `cell_count` return the offset
directly instead of walking. Two vectorised byte scans replace two O(clusters) walks over a line that
was about to be shaped anyway.

`a_line_of_ascii_agrees_with_the_full_walk` checks that claim against the general path over every
offset and every range of nine fixtures — controls, a lone `\r`, tabs — under **both** cell models,
and asserts the guard rejects `café`, `日本`, `a\u{0301}`, `a\nb`, `\r\n` and `👍🏻`.

### ⚠ **One non-ASCII character disqualifies the whole line**, and that is the open half

An em-dash at the start of a 19.4 KB line costs **76 ms a frame** — the fast path is all-or-nothing
per line, so a log with a customer name, a `—`, or any UTF-8 JSON in a long record gets nothing from
it. This is not a rounding error on the fix; it is half the problem still standing, and the dogfood
corpora are exactly the kind of logs that contain non-ASCII.

The general fix is unchanged from what the review named: **a per-row cluster anchor**, the same shape
as the line index's anchors, one axis over — sample `(byte, cell)` every K clusters when a row is
fetched, so a lookup is O(K) instead of O(line). It belongs in `Rows`, next to the decoded text it
would be built from, and it is **not done**.

## ✅ 50M lines, 5.24 GB — M3's headline done-criterion, measured — 2026-08-11, session 15

A real 5,235,180,551-byte corpus of **50,000,001 lines** was built and opened, with a unique
`FINAL-LINE-MARKER` on the last line so "did it reach the end" is a fact rather than an impression.

| | Measured | `SPEC.md` §11's budget |
|---|---|---|
| Index build | **2.35 s** (2.2 GB/s) | — |
| Working set after indexing | **43 MB** | line index ~6.3 MB + device 30–60 MB |
| Working set after scrolling | **77–82 MB** | + atlas 4–16 MB |
| Page-down, 48-row window | **16.58 ms** | 16.67 ms frame budget |
| Page-down, 119-row window | **16.74 ms** | " |
| `Ctrl+End` | lands on `FINAL-LINE-MARKER`, line 50,000,000 | — |

**The frame numbers are vsync, not our cost, and the second row is what proves it.** 16.58 ms is
suspiciously close to a 60 Hz vblank and `present` uses vsync, so the figure alone proves nothing.
Widening the window to **119 rows — 2.5× the rows fetched, decoded and shaped per frame — moved it by
1%**. Work that scales 2.5× for 1% more time is not the thing being measured. The per-frame CPU cost
is under a vblank at 50M lines and this test cannot resolve it further without disabling vsync.

**Memory lands inside the envelope §11 predicted**, which is worth saying because that table was
written before any of this existed: 43 MB total against a budget that allots 30–60 MB to the D3D11
device alone. The sparse index really is ~0.125 B/line.

### ⚠ Three things this does *not* show

- **The index time is a warm-cache number.** The file was written immediately before the run on a
  machine with 64 GB of RAM, so 5.24 GB was in the page cache. 2.2 GB/s is therefore a floor on the
  work, not a disk measurement; a genuinely cold open is bounded by the NVMe instead. Indexing is
  parallel across cores, so it may well be CPU-bound on the newline scan either way — but that is a
  guess and is not measured.
- **Horizontal scrolling was never exercised.** The fixture's lines are ~104 characters, so the
  viewport never left column 0. **The residual layout cost is linear in the *horizontal* offset**,
  which means this run says nothing about it. Vertical scrolling through 50M lines is fine; scrolling
  right into a 32 KB line is the untested path and is still an open M3 item.
- **One encoding, one shape of line.** ASCII-range UTF-8, uniform width, no CJK, no RTL, no
  combining marks, no 32 KB records.

## ✅ Per-monitor DPI — M3's second done-criterion — 2026-08-11, session 15

`SPEC.md` §3.1 requires per-monitor-V2, all layout metrics recomputed on `WM_DPICHANGED`, the glyph
atlas rebuilt per scale factor, and column advances in integer device pixels re-derived on any scale
change. **None of it existed** — no awareness declaration, no `WM_DPICHANGED` handler, `px_per_em`
pinned at 16. Without awareness Windows bitmap-stretches the client area on a non-96-DPI monitor, so
every glyph is resampled: the one outcome the whole atlas exists to avoid.

**Verified end to end on the real binary** by sending the exact message Windows sends on a
monitor drag: `WM_DPICHANGED` at 144 dpi with a suggested rect. **Row pitch went 18 → 27 px, exactly
1.5×**, the window took the suggested rect, and the text came back sharp rather than upscaled. At
96 dpi the pitch is unchanged at 18, so the 100% path did not move.

**The scale is now the second half of the atlas key**, alongside the device generation —
`Option<(u32, u16, Painter)>`. That is the same mechanism as device-loss recovery rather than a
second one: `ensure_painter` sees the new `px_per_em` on the next frame and rebuilds then, which is
also why `set_dpi` does no work itself. Control: dropping `px_per_em` from the comparison leaves the
100% atlas in place across the change — 16 px rasters in 24 px cells, blurry *and* progressively out
of column, which is precisely the drift §3.1's integer-advance rule exists to prevent.

Two details worth keeping:

- **The em size is rounded to a whole device pixel in `set_dpi`**, not carried as a float. §3.1 is
  explicit that fractional per-glyph rounding "accumulates drift and visibly misaligns columns across
  a wide window", and everything downstream is integer device pixels at the current scale.
- **The initial DPI is read when the renderer is adopted**, not only on `WM_DPICHANGED`. The renderer
  is built on a worker before any window exists, so it starts at 100% — and a window *opened* on a
  150% monitor never receives a change message, because nothing changed. Without that read the first
  frame there would be wrong and would stay wrong.

### ⚠ A deliberate deviation from the spec's wording

§3.1 says per-monitor-V2 is "**declared in the manifest**". This calls
`SetProcessDpiAwarenessContext` as the first statement in `main` instead. The two are equivalent
*here* — nothing in this process creates a window earlier, and awareness cannot be changed once one
exists — and an embedded manifest would mean adding resource compilation to the build for no
behavioural gain. It is recorded as a deviation in `CLEANROOM.md` rather than passed off as
compliance, and it stops being equivalent the moment anything creates a window before `main` runs.

### Where M3's done-criteria now stand

| Criterion | State |
|---|---|
| No `f32` accumulation anywhere in scroll state | **Done** — `(u64, f32)`, tested at 10⁸ rows |
| Drag between 100% and 150% monitors with no drift | **Done** — the mechanism is verified; an actual two-monitor drag has not been performed, and this machine has no scaled monitor to do it on |
| Scroll a 50M-line file smoothly | **Vertically yes** — 50M lines / 5.24 GB, vsync-bound at 119 rows a frame, 43 MB resident, `Ctrl+End` exact. **Horizontally, only for ASCII** — 19.4 KB ASCII lines are vsync-bound; the same lines with one non-ASCII character cost 76 ms a frame |
| The CJK/RTL/emoji fixture renders correctly | **No** — RTL column placement is unimplemented and disclosed through `Laid::rtl_runs` |

## ✅ It scrolls — wheel and keyboard navigation — 2026-08-07, session 15

`UI-DESIGN.md` §12's navigation map is bound: wheel, `Shift+wheel` and the tilt wheel, arrows,
`PageUp`/`PageDown`, `Space`/`b`, `Home`/`End` and `Ctrl+Home`/`Ctrl+End`. **Verified against the
17 MB fixture by posting keys to the real window and reading the pixels back**: 10 pages lands on
line 410, 4,800 pages on line 196,800 — exactly 41 rows a page each time — and 6,000 pages clamps to
the last screenful with the sub-row offset visible at the top, which is §6.4 rule 1 doing its job.

**Nothing here computes a scroll position.** §3.1 puts the grid in the core and §6.4 spent two
experiments arguing that arithmetic, so a message becomes a `Navigate` and `grid.rs`/`hgrid.rs` do
the rest. Three things in that translation are worth keeping:

- **The wheel scrolls in pixels, not rows.** §6.4 rule 1 carries the remainder into the row index, so
  a precision touchpad's sub-notch delta moves the view instead of rounding to zero.
- **Lines-per-notch is read from `SPI_GETWHEELSCROLLLINES` every time**, not cached and not
  hard-coded to 3 — that setting is what makes the app scroll at the same speed as every other
  window, and it can change while the app runs.
- **`navigate` returns whether anything moved**, and only a move invalidates. A held arrow key at the
  top of the file would otherwise burn a frame per repeat, and a frame here is a full re-fetch and
  re-shape of the viewport. Control: returning `true` unconditionally fails both navigation tests.

**Follow needed no code.** `Grid::is_following` is *derived* from being at the bottom, so
"scrolling up auto-pauses follow" and "`Ctrl+End` re-enables it" both fall out of the scroll model.
It is now `pub` because the shell needs it for §12's `⬤ Following` chip.

`VIRTUAL_KEY` is a newtype, so its constants **cannot appear in a match pattern** — a bare `VK_UP`
arm binds a variable and matches every key. The bindings compare raw `u16` codes through local
`const`s for that reason.

### ⚠ Still not done on this axis

- **No smooth or inertial scrolling.** `UI-DESIGN.md` §12 requires `WM_POINTER`/Direct Manipulation
  and says why — discrete wheel steps are what make a hand-rolled Win32 app feel homemade next to
  Edge and Terminal. This implements discrete `WM_MOUSEWHEEL` only, and the `CLEANROOM.md` row
  records that gap deliberately rather than quietly.
- **No scrollbars**, so there is no drag-to-scroll and no position feedback at all.
- **The `WM_KEYDOWN` → `Navigate` mapping has no test** — it needs a message pump. Only
  `Document::navigate`, the half that turns an intent into a movement, is covered.

## ✅ Tailhawk renders a real log file — 2026-08-07, session 15

**A 17 MB, 200,000-line file opens, indexes, and draws as text in the grid.** That is M3's first
end-to-end frame, and getting there took two bugs that only a real file could surface.

`rows.rs` is the new join: given a reader, an index and a charset it answers *what does row N say*.
`Document` in the shell holds file + index + view + rows, is built on a worker, and crosses the
channel whole. `Shell::paint` sets the metrics from the measured face **every frame** (§3.1 wants
them re-derived, and a DPI change between frames is exactly what a cached cell gets wrong), lays the
view out, fetches the visible rows and calls `Renderer::paint_rows`.

### The design claim in `rows.rs`, and why it is measured in bytes

A viewport is a run of **consecutive** rows, so `fetch` resolves the *first* through `offset_of_line`
and decodes forward, rather than resolving each row. **Measured: 69,088 bytes for a screenful at row
4,000, against 3,340,670 for the same fifty rows one at a time — 48×.** The per-row version is
*correct*, which is why no other test can catch it; the guard counts bytes rather than time, so it is
deterministic and cannot flake the way the paint.rs duration assertion did. Its control reports the
two arms as byte-identical.

### ⚠ Two bugs that no test would have found

**The window rendered a perfect grid of placeholder boxes and no text.** §3.2 puts rasterisation
*after* the present, so a cold atlas draws a box in every cell — and nothing asked for a second
frame, because an idle window gets one `WM_PAINT` and then silence. The first fix was wrong in an
instructive way: calling `InvalidateRect` from inside `paint` does nothing at all, because `WM_PAINT`
calls `ValidateRect` **after** `paint` returns and wipes the region. The flag is now raised in `paint`
and acted on by the handler after it validates.

**Then the background went pure black.** `TextPipeline::draw` binds a dual-source blend state and
never restores it, and `draw_background` set none — so frame 1 inherited the default and was right,
and every frame after it blended the background against an undefined second source. This is **M0 code
that had been correct for fifteen sessions** because nothing else had ever bound a blend state. At
RGB(18,20,23) against RGB(0,0,0) the difference is invisible; it was caught by sampling the
screenshot, which is the procedure the traps table already prescribed for this palette. Verified:
**92,756 pixels at exactly `background_rgb8()` and zero black**, against 92,711 black before.

**Neither has an automated regression test, and that is the honest state.** `Gpu::draw_background`
needs a swapchain, so the background path cannot be driven from the offscreen harness that covers the
text pass — making it testable means letting `Gpu` render to an offscreen RTV, which is the next
piece of work here and is not done. Both are in the traps table instead, which is weaker.

### What still is not there

Nothing scrolls. There is no input handling at all — no wheel, no keys, no scrollbar — so the
viewport is pinned at row 0. M3's done-criterion is "scroll a 50M-line file smoothly", and the frame
now exists to scroll; the input that would move it does not. `flush_misses` also still repaints once
per cold frame rather than budgeting rasterisation across frames.

## ✅ The renderer owns the text pass, keyed to the device that built it — 2026-08-07, session 15

`Renderer` now holds `Option<(u32, Painter)>` — the painter **and the device generation it was built
against** — plus `paint_rows`, which draws a viewport of text over the background and presents it.
`CLEANROOM.md`'s paint.rs row already named this work in advance, so no new provenance was needed.

**The generation is stored rather than an invalidation flag being set**, and the reason is the
failure mode: a `Painter` owns a `GlyphCache` owns a `Sheet`, which is a texture belonging to one
`ID3D11Device`. Drawing from a released device's sheet **does not fail** — it produces nothing and
reports success. A flag has to be remembered at every site that could invalidate it; a comparison
cannot be forgotten. Control: replacing the comparison with `slot.is_none()` leaves the painter keyed
to generation 1 after a loss, and the test fails on that identity.

**The rebuild check runs inside the draw callback, not before the frame.** `gpu.rs`'s recovery
replaces a device *mid-frame* and redraws, so a check done beforehand would be correct for the first
attempt and wrong for the retry — the exact frame that matters. `render_frame_with` therefore hands
the callback the device, its context, **and the generation**, and `ensure_painter` is called from
inside. The disjoint-field destructure in `paint_rows` is what makes that borrow legal.

`flush_misses` is now wired, after the present, which is what `render_frame_with` returning means.
Without it the atlas never gains a glyph: every frame would queue the same misses and draw the same
placeholder boxes for ever. Control: removing the call makes the second frame queue 27 glyphs again.

### Two things this turned up that were not the point

- **`Laid::rasterised` counts glyphs that gained *ink*, not glyphs that were *resolved*.** A blank
  glyph — the space — is recorded via `insert_blank` so it is never asked for again, and `continue`s
  without incrementing. My first assertion demanded `rasterised == queued` and failed 26 against 27;
  the test was wrong about the code, not the other way round. Resolution is what the *next* frame's
  `queued == 0` proves, and that is what the test asserts now.
- **`cell()` builds the text resources on demand**, because otherwise the caller cannot break the
  chicken-and-egg: a `View` needs the cell metrics before it can say which rows are visible, and the
  painter that measures the cell would only exist after a frame had been drawn.

### ⚠ What is deliberately not covered

- **The pixels, on this path.** A `Renderer` with no window holds no render target, so
  `a_rebuilt_device_gets_rebuilt_text_resources` can only check the generation identity;
  `paint.rs`'s `a_viewport_of_rows_reaches_real_pixels` covers pixels against an `Offscreen`. The gap
  matters *because* the guarded failure is silent — a stale painter and a correct one are
  indistinguishable from the return value, which is why the identity is asserted directly rather
  than inferred from a frame that returned `Ok`.
- **A draw error is never classified as device loss.** `FrameError` keeps device loss as a
  `windows::core::Error` all the way to `was_lost`, which needs the `HRESULT`; the text pass reports
  a `crate::Error` with no `HRESULT` left to read, so such a frame returns `Err` and does not
  recover. The next frame meets the same loss in `present`, which is typed, and recovers there —
  one dropped frame, not a stuck renderer. Widening it means giving `crate::Error` somewhere to
  carry an `HRESULT`.
- **Nothing shows a real file yet.** The shell still opens the log only to count lines for the title
  bar. Connecting `FileSource` → `build_index` → `LineDecoder` → `View` → `paint_rows` is the next
  M3 step and is untouched here.

## ✅ Adversarial review of the M3 viewport — three live bugs, one of them pre-existing — 2026-08-07, session 15

A 12-agent workflow reviewed `hgrid.rs`, `view.rs` and `paint.rs` across four independent lenses
(precision, content loss, device lifetime, state transitions), each candidate then handed to a
verifier told to **refute** it. **Seven confirmed, one refuted.** Every fix below has a negative
control that was applied, observed and reverted.

**This is the second time in one session that review found what my own tests could not**, and the
pattern is worth naming: both times the tests were *correct* and *insufficient* in the same way —
they used small fixtures near the origin, where an asymptotic cost and a boundary overrun are both
invisible.

### ⚠ The text pass was quadratic, and four of the four lenses found it independently

`lay_out_row` asked `CellModel::cell_at_byte` for a column **once per cluster**, and that call
re-walks the line's graphemes from byte zero. So a frame cost O(visible columns × horizontal scroll
offset). Measured, release: **a screenful of 32 KB lines scrolled to the middle took 16.4 seconds**,
against a 16.67 ms budget — and §10.3 puts exactly those lines in scope, citing klogg hanging
"deadly" on them as the behaviour to avoid. An ordinary 2 KB JSON record at column 1,024 cost
651 ms.

The column is now carried across clusters and advanced by each cluster's width, which is exact
because `shape.rs` and `cell.rs` segment with the same `grapheme_indices(true)` and `byte_span`
rounds the slice outwards to whole clusters. `RowSlice` carries the starting column it was already
computing. A **second, independent** term came from `CellModel::byte_span` having no early exit —
190 ms/frame at horizontal offset *zero* — now stopped once the clusters are past the range.

**⚠ It is faster, and it is still not fast enough. Do not read the passing test as a budget.** The
residual walk is linear in the scroll offset — roughly **200 ms** for that frame against 16.67 ms.
Removing it needs the cell model to stop starting from byte zero: a per-row cluster anchor, the same
shape as the line index's anchors, one axis over. **That is an open M3 item, not a solved one.**

### ⚠ A cluster's glyphs could be painted over the next column — §13.4

`a` followed by U+E0067 TAG LATIN SMALL LETTER G is **one** cluster the cell model calls one cell
wide, and it shapes to **two** glyphs each carrying a full advance, the second being `.notdef` — a
hollow box with real ink. The within-cluster pen had no bound, so the box landed in the next column.
`"a" + "\u{E0067}".repeat(20) + "SECRET"` painted a box over each of the next twenty columns:
attacker-supplied invisibles blotting out the text after them, which is §13.4's hidden-text vector
rendered as a viewer that can be made to lie.

§3.3 gives the cell grid the last word *between* clusters; nothing said what happens **within** one,
and the answer is the same authority. The pen and each glyph's x now clamp to the cluster's own last
cell. **Control: 25 quads land past column 1 without the clamp, where only 6 characters live.**

### ⚠ Minimising the window destroyed the reading position and follow — and this one predates today

`grid.rs`, from session 13, reviewed twice before. `max_scroll` answered `Scroll::TOP` for a
zero-height viewport; `TOP` is the *lower* bound, so `clamp` read it as a cap and rewrote the
position to row 0, and on restore `is_following` had already been false-d out by its own viewport
guard, so nothing re-pinned. **Tailing a 50M-line log, minimise, restore: top of the file, follow
silently off.** Every Windows app gets a 0×0 client area on minimise, so this needed no unusual
input at all — it was latent only because nothing drives the grid yet.

**The controls are the interesting part here.** Reverting `max_scroll` fails 4 tests. Reverting
`is_following` fails **none** — so by this project's own standard that half was unjustified. It is
not: it is load-bearing for *the file growing while minimised*, which is the case a tail tool exists
for and which no test covered. The missing test now exists, and the control fires on it. **A control
that does not fire is a statement about the tests, not about the change.**

### One cluster can be the whole line

`"a" + "\u{0301}".repeat(16000)` is 32 KB — §10.3's supported inline size — as **one** cluster in
**one** cell, shaping to 16,001 glyphs and emitting 16,001 quads stacked on column zero. The column
window cannot narrow it, because it is one cell wide. `View::slice` now caps the bytes handed to the
shaper, snapping **inward** to a `char` boundary — outward would restore the whole cluster and
defeat the cap.

### The refuted one, and what it left behind

"Shaping the visible slice rather than the whole line makes bidi levels a function of scroll
position" — refuted, because `visual_glyphs` is never called from `paint.rs` and placement is
logical, so resolved levels affect nothing drawn. **But that is only true while RTL placement is
unimplemented.** When it lands, shaping a slice becomes a real defect: the reading order of what is
on screen would depend on where the window is scrolled. Recorded here so the fix and the trap arrive
together.

### ⚠ Three findings were dropped unverified by the cap — all three were real

`view.rs:113` and `paint.rs:157` (twice, from two lenses). **The workflow's per-lens verification cap
is a real limit, not a filter.** It logged what it dropped, and all three dropped items turned out to
be genuine — one of them the second-largest finding in the run.

The `view.rs` one was **live-bug** severity: the `byte_span` full scan, fixed above.

The `paint.rs:157` pair was recovered from the run log, which had kept the *location* and the *lens*
(`content-loss`, `state-machine`, both "latent") but **not the finding text**. That was enough to
re-derive both, and both were right:

- **`View::slice`'s 8 KB byte cap can truncate a row that is on screen.** "An ordinary line never
  reaches it" is true and is not the claim that matters. A cell costs `1 + 2n` bytes with `n`
  combining marks, so around twenty marks per base character puts a **300-column** row past the cap —
  far narrower than any real viewport, and §13.4 makes hostile text an explicit threat model. The
  right-hand part of the row then draws nothing and the row looks like it simply ends. The cap stays,
  because the 16,001-quads-on-one-column case it exists for is worse; it now reports itself through
  `RowSlice::truncated` and `Laid::truncated_rows`. `a_row_whose_visible_bytes_are_capped_says_so`
  pins the reachable case, and it earned its keep by failing informatively when the first fixture was
  wrong.
- **One row that failed to shape abandoned the whole frame.** `lay_out` propagated `?` from
  `lay_out_row`, so fifty rows already laid out were discarded and the caller, seeing `Err`, drew
  nothing — one malformed line freezing a viewer that is following a live log. The `None` arm one
  line up already had the right answer for exactly this shape of problem. The row is now skipped and
  `Laid::failed_rows` counts it.

**⚠ `failed_rows` has no test that exercises it, and the reason is worth carrying forward.** A
negative control on that change does not fire, so by this project's rule the question was put
directly: what legal `&str` makes DirectWrite fail? Six hostile fixtures were tried — 5,000 combining
marks in one run, private-use, unassigned and noncharacter code points, tag characters, 2,000 stacked
bidi overrides — and **all six shape**. The error path is reachable only on a system-level failure
(COM error, corrupt font, allocation failure), which no test can produce without a seam, and none was
added. `hostile_text_shapes_rather_than_failing` records that evidence rather than asserting an
excuse; if a fixture there ever *does* error, `failed_rows` becomes testable that day.

---

## ✅ Doc consistency sweep — six stale claims, two of them normative — 2026-08-07, session 15

A 12-agent workflow swept all six documents plus `CLEANROOM.md` for claims still stated as current
fact that a measurement, a later correction or the code contradicts. Four finders, then an
adversarial verifier per candidate told to **refute** it and to default to "not real" when unsure.
**Six confirmed, two refuted**, all six independently re-checked against the files before editing.

### The finding behind the finding: a correction has to be *swept for*, not applied where it was noticed

**Three of the six are the same error.** Session 8 measured that `0x0A` is never a DBCS trail byte
and corrected `SPEC.md` §5.3 — and left **three other copies standing**: `RESEARCH.md` §5.8 (stamped
**`[V]`**, the document's strongest confidence marker), `RESEARCH.md` §11's refuted-claims table (the
section §0 tells readers to consult *before quoting anything*), and this file's own traps table,
three rows above the entry refuting it. The wrong rationale outlived its correction by seven
sessions in the two places most likely to be trusted.

Same shape as the fourth: `PLAN.md` §2.5 still pointed the stop-and-reconsider gate at **M2**, residue
from the decode-before-index resequencing that moved the grid to M3. §4 was corrected; four
back-references were not. All four now read M3.

### The two normative ones, both in `SPEC.md`

| Was | Now |
|---|---|
| §3.3: "**Combining marks** occupy 0 additional cells" | Split into `Mn`/`Me` (0 cells) and **`Mc` spacing marks**, which carry their own advance — `कि` is one cluster of **2 cells**. `cell.rs` has known this since V4's review; the spec did not. **Devanagari is named in §3.3's own acceptance test**, so an implementer following the spec would have rendered that fixture with every following column a cell out. |
| §6.4: "That one assertion catches all three" | Withdrawn. It is true of egui's architecture, not ours: with an exact `(u64, f32)` position the two content-magnitude terms are the *same expression* and cancel at `i = 0`, so a faithful rule-2 reintroduction leaves the **first** row exact. The required test now asserts every visible row. Measured by mutation in `grid.rs` in session 13; the spec kept the refuted sentence until now. |

### The two refutations were both right, and one caught my own misreading

- **"Branching starts once the remote exists"** (this file, session-9 planning text) — refuted:
  the working agreement fifteen lines below it already records "commit directly to `master`, no
  branches, no PRs — tried once, not worth it", and the newer current-state section repeats it. The
  verifier also caught that the finder had misidentified which bullet list the line belonged to.
- **`CLEANROOM.md` §1.5 "log before you code"** — refuted on the right grounds: a rule is not
  falsified by being broken, and the document already records three of its own slips by name.
  What survives is an observation about this session's discipline, not a documentation defect —
  **`hgrid.rs`, `view.rs` and `bidi.rs` have no §5 entry**, and §5's last row still stops at V3.

### The `CLEANROOM.md` §1.5 debt is paid, and the rule now executes

`selection.rs`, `bidi.rs`, `hgrid.rs` and `view.rs` had **no §5 entry at all** — the fourth slip of
"log before you code". The entry is filed, and **marked as the retroactive thing it is**: only
`hgrid.rs` and `view.rs` were written by the agent filing it, so for the other two the row records
what the code and commits show rather than a first-hand attestation. That distinction is the whole
value of the file and collapsing it would have been worse than the omission.

**The pattern named in the device-recovery row has a second form.** That row said §1.5 slips when a
component starts as "a small addition to something that already exists". These were four *new
files*, and it slipped anyway — because each felt like the continuation of a V3 row already filed.
A pointer row only works when the later work is inside the scope the earlier row named, and
"the scroll model, row layout and hit-test" named none of them.

**So §1.5 is now a CI gate** (`provenance`, in `ci.yml`): every module in `crates/tailhawk-core/src`
must be named in `CLEANROOM.md`. This is the file's own recorded lesson applied to itself — "a config
file nothing executes is documentation" — and it asserts only the weakest mechanical form: it cannot
check that the entry came first, is accurate, or is honest. **It would not have caught the `cell.rs`
additions either**, since that file was already named. What it catches is the failure that has
actually happened every time: a new module arriving with no entry. Verified by deleting the new row —
all four modules are reported.

### ⚠ Four findings were dropped unverified, and they are not "no"

The workflow verifies the two most severe per document and logged what it dropped rather than
truncating silently: `HANDOFF.md:786`, `HANDOFF.md:1463`, `RESEARCH.md:593`, `LOKI.md:118`,
`PLAN.md:176`, `PLAN.md:324`, `SPEC.md:265`. Unverified is unverified — they were the *less* severe
of their batch, not refuted.

---

## ◐ The consumer has started — `view.rs`, the portable half — 2026-08-07, session 15

**293 tests, all passing**, fmt and clippy clean on the two shipped crates. `view.rs` composes
`Grid`, `HGrid` and `CellModel` into one viewport, so **the four modules that were exported and
unwired are wired**. It is still device-free and headless: it holds no font, no device and no
window, and nothing in it draws.

### The two joins that are silent when wrong, encoded once

- **`View::slice`** returns the bytes of a line to draw and the x to draw them at. `byte_span`'s
  outward rounding — built for §5.6, so a copy cannot launder a zero-width override — returns a
  straddling wide cluster **whole**, and the slice is then placed at **that cluster's own column**
  rather than the one the viewport asked for. Drawing it at `visible_columns().start` shifts the
  whole line right by the part that is off screen.
- **`View::position_at`** pairs the two hit-tests into `selection::Position`, so a caller cannot
  combine a row from one frame with a column from another.

**Two negative controls, applied, observed and reverted:** the slice drawn at the asked-for column
(1 fail, the CJK straddle); the byte range composed as `byte_at_cell(start)..byte_at_cell(end)`
(1 fail, the Trojan Source line's override dropped from the drawn bytes).

### ▶ Resume here: the device half, and one design constraint already found

`Renderer::paint` is still the background clear — its own doc says "the grid arrives at M3". The
constraint that shapes what comes next, found by reading `gpu.rs` rather than by being bitten:

**⚠ A `GlyphCache` holds a `Sheet`, which is a texture owned by the device.** `gpu.rs`'s recovery
rebuilds a lost device and bumps `device_generation()`, so text resources held *alongside* the
renderer would keep drawing from a dead sheet after a TDR or a driver update — silently, and §3.2
forbids making that a panic. So `Renderer` must own `Shaper` + `GlyphCache` + `TextPipeline` and key
them to the generation counter. `View` stays portable and holds none of it.

Two more things settled by that reading:

- **The pen is cell-driven, not advance-driven.** §3.3 says that where a fallback face's advance
  disagrees with the primary, the cell grid wins and the glyph is centred in its cell. So a cluster
  is placed at `x_of_column`, and the shaper's offsets position glyphs *within* the cluster.
- **⚠ Bidi placement is a gap, and must not be shipped silently wrong.** `visual_glyphs` orders the
  glyphs *inside* a run, but which column an RTL run's clusters occupy is a separate question that
  `cell.rs` answers in logical order. The first painting slice should be correct for LTR and
  explicit about RTL rather than drawing Arabic in the wrong columns and looking finished.

---

## ✅ V3 is complete — horizontal scrolling, `hgrid.rs` — 2026-08-07, session 15

**286 tests, all passing**, fmt and clippy clean on the two shipped crates. `PLAN.md`'s V3 row is
"u64 scroll model, hit-test, selection"; with `grid.rs`, `selection.rs`, `bidi.rs` and now
`hgrid.rs`, **every V3 item is done.** This is the piece §3.3's extent fields were captured for and
the one §6.4 cut `--wrap` from v1 in favour of.

### The two axes are not symmetric, and the reason is §10.3

`Scroll` cannot hold a content-pixel offset because a document has 2⁶⁴ rows. **The horizontal axis is
bounded**, so `HGrid` holds a plain `offset_px: f32` — and what bounds it is §10.3's **32 KB inline
render cap**, not good manners. No encoding produces more cells than bytes (§3.3), so a rendered line
is at most 32,768 cells however long the line in the file is; with a bounded cell width the widest
content this module can form is 2²⁴ px exactly, the last integer `f32` represents.

**That is a dependency between two sections that did not previously reference each other**, and
`SPEC.md` §3.3 now records it: raising the render cap to "unlimited" is not a rendering-only change,
because it puts §6.4's `(index, remainder)` obligation back on this axis. The assertion that fails is
`the_render_cap_is_what_keeps_every_column_exact`.

### ⚠ Rule 3 survives boundedness, and it was a live bug in my first version

Having argued that rules 1 and 2 do not apply here, the third one still did. `column_at_x` resolved a
click as `x + offset_px` — a screen-space coordinate added to a content-magnitude one, which is
exactly what §6.4 rule 3 forbids — and **it hit-tested one column to the right**: at a 96,624 px
offset one ULP is 0.0078 px, so a click at `x = 231.99881` inside a column drawn at 232 rounds *up*
onto the boundary. Both terms exact, sum not. Found by `hit_test_is_exactly_the_inverse_of_layout` on
its first run, not by review.

`first_visible()` is where the content magnitude now stops: the offset resolves to the leftmost
column once, and layout, hit-test and `visible_columns` all measure from there in viewport-sized
numbers. `reveal` compares `end_px − offset` against the viewport rather than `offset + viewport` for
the same reason. **§6.4 now says rule 3 is not excused by a bounded axis** — the note a future
implementer needs, since the natural inference from "bounded" is that none of the three apply.

### The offset is stored exactly and drawn snapped, and both halves are load-bearing

Layout rounds the offset to a whole device pixel because a glyph bitmap placed at a fractional x
resamples ClearType's **horizontal** subpixel coverage — colour fringing, not blur. It rounds it
**once**, shared by every column, so adjacent columns stay exactly one cell apart rather than 7 px
and then 8. Rounding the *stored* offset instead is the worse bug and has its own test: a trackpad
delivering 0.4 px per frame would round to zero every frame and the view would never move at all.

### Six negative controls, each applied, observed and reverted

| Control | Fails |
|---|---|
| Layout does not snap the offset | **3** — including the spacing test and the sub-pixel drag |
| The *stored* offset is rounded instead | 1 — the sub-pixel drag vanishes |
| Hit-test adds the click to the content offset (rule 3) | 1 — the inverse test, as above |
| The clamp only guards the near edge | **4** — a refined extent and a widened window both strand the view |
| A cell-width change keeps the pixel offset, not the column | 1 — §3.3's column drift, one axis over |
| `reveal` always aligns to the left edge | 2 — every keystroke jerks the view |

### Two things this hands to the painter rather than solving

- **A wide cluster straddling the left edge starts before `visible_columns().start`.** Painting from
  `start` cuts it in half — but `CellModel::byte_span` already rounds both ends outwards, so it
  returns the cluster whole, and `x_of_column` is deliberately unclamped so the painter can place it
  at a negative x. The two pieces compose; nothing new was needed.
- **`set_columns` is never to be fed from the lines currently on screen.** §3.3 says so and names the
  symptom — the horizontal-thumb jitter every other viewer has. Nothing enforces it structurally.

### ▶ Resume here

**V3 is done, so the next thing is a consumer.** `Grid`, `HGrid`, `Selection` and `visual_glyphs`
are all exported and **all four are unwired** — every defect found in the last three sessions was
latent, and they become live the moment a viewport is attached. The static exe is still unchanged,
so every size figure quoted since M2 remains an LTO artefact. Attaching a viewport is what settles
both at once.

---

## ✅ Visual reordering — `bidi.rs`, and the level `shape.rs` was throwing away — 2026-08-06, session 14

**267 tests, all passing**, fmt and clippy clean on the two shipped crates. `shape.rs` measured in
session 13 that `GetGlyphs` returns a right-to-left run's glyphs in **logical** order and recorded
that "visual reordering is the drawing code's job, and V3 owes it". **That debt is now paid.**

`crates/tailhawk-core/src/bidi.rs` is UAX #9 rule L2 and nothing else — portable, device-free,
testable against the standard without a font. `Shaped::visual_glyphs()` is the DirectWrite-side entry
point, and the real-font test asserts the exact inverse of session 13's finding: alef came back at
glyph index 0 from `GetGlyphs`, and must come back **last** from `visual_glyphs`.

### ⚠ `shape.rs` was storing `right_to_left: bool` and the magnitude of the level was lost

L2 needs the *number*, not the parity. An odd/even bit cannot tell a left-to-right phrase embedded in
a right-to-left sentence (level 2 inside level 1) from an unrelated left-to-right run at level 0, and
the two paint in different places. `Run` now carries `level: u8` with `right_to_left()` derived from
it, and `Shaped::runs` exposes it. `AnalyzeBidi` was already reporting the resolved level correctly —
it was being discarded one line after arriving.

### Three findings, each from a control that fired

| Claim | How it was checked |
|---|---|
| **Per-*glyph* levels, not per-run** — reordering runs alone puts the runs in the right places and leaves the glyphs *inside* a right-to-left run in logical order. The bug moves from the line to the word. | Rewrote `visual_glyphs` to reorder runs only: **2 real-font tests fail**, including the Arabic word. |
| **"Lowest odd level" is not "lowest level."** Taking the minimum level starts the loop at 0, whose window is the whole line — **it reverses every left-to-right line in the file.** | **8 tests fail**, including `visual_glyphs_leaves_a_left_to_right_line_alone`. |
| **The loop counts *down*.** Ascending reverses the outer run first and then flips the embedded phrase back inside it, giving a plausible line with the embedded fragment backwards. | 3 fail, including the mixed real-font line. |

### ⚠ A symmetric nesting cannot see the pass order, and two fixtures were symmetric

Reversing a window and then reversing a sub-window **symmetric about its centre** gives the same
answer either way round. `ABokCD` at levels `[1,1,2,2,1,1]` agrees under both orders — it reads as a
passing test of a property it cannot observe, and the ascending control did not fire on
`a_doubly_nested_embedding_carries_its_inner_block_as_a_unit` for exactly that reason. Both fixtures
are now off-centre, and the control fires on both. Same shape as `raster.rs`'s `tighten` and
`shape.rs`'s `glyph_range`: **a control that does not fire is a statement about the fixtures.**

Two of my own test *expectations* were also wrong and the code was right — `[124,125,125,124]`
reorders only the level-125 pair, because 124 is even. Corrected in the test, not the algorithm.

### ▶ Resume here: the last V3 piece

- **Horizontal scrolling**, which §3.3's extent fields exist to size. It is the only V3 item left.
- **Still nothing consumes `Grid`, `Selection` or `visual_glyphs`.** All three are exported and
  unwired, so their defects stay latent until a viewport is attached — and the static exe is
  unchanged, so every size figure quoted since M2 remains an LTO artefact.

---

## ✅ V3's selection model — `selection.rs`, plus `cell.rs`'s two new functions — 2026-08-06, session 14

**251 tests, all passing** (251 core + the shell's one), fmt and clippy clean on the two shipped
crates. Stream and block selection, double-click words, triple-click lines, and the resolution from
cell columns to bytes. `PLAN.md`'s V3 row is "u64 scroll model, hit-test, selection" — **the third
item is now done**, and the two remaining V3 pieces are visual reordering for RTL runs and horizontal
scrolling.

### The axes are not symmetric, and the API shape follows from that

A row is a `u64` and a column is a `usize`, because a document has up to 2⁶⁴ rows and §10.3 bounds a
line. **`Selection::row_span(row)` is O(1) and this module never iterates its own rows** — a caller
paints by asking `Grid::visible` which rows are on screen and then asking about each, so selecting a
whole 10 GB file costs a viewport, not a document. `last_row` is inclusive for the same reason the
grid avoids content magnitudes: the exclusive form is `last + 1`, which overflows on a selection one
Ctrl+A wide.

`RowEnd::ToLineEnd` is a symbolic value rather than a column, and cannot be replaced by one: an
interior stream row runs past its last character to take the line terminator, and this module does
not know how long any line is.

### ⚠ `byte_at_cell(start)..byte_at_cell(end)` is not the byte range of a selection — it is §5.6

The obvious composition **drops zero-width clusters from the copied bytes**, because `byte_at_cell`
skips them by design (they cannot be clicked). `"\u{202E}abc"` — the Trojan Source line §13.4 names —
puts the override and `a` both at column 0, so selecting the visible line yields `abc` and **silently
launders the attacker-supplied override out of what the user pastes**. §5.6 forbids discarding content
silently, and this is the worst version of it: the paste reads differently from the selection.

`CellModel::byte_span` rounds the two ends **outwards** instead — the start takes the lowest byte at
its column, the end the highest. A zero-width cluster on an interior boundary therefore belongs to
**both** neighbours, deliberately: copying an invisible character twice is cosmetic, losing one
changes what the text means. Reverting to the naive composition fails
`selecting_a_line_carries_its_zero_width_content` with `3..6` against `0..6`.

### 🔍 Self-review found three defects before the code was committed

| Defect | Effect |
|---|---|
| **The outward rounding contradicted the empty case** | A zero-width cluster on the caret's own column satisfies *both* ends of a `c..c` range, so **a caret copied the bidi override next to it**. An empty column range is now empty in bytes, and `byte_range`'s special case disappeared with it. |
| **`Selection::line` saturated at `u64::MAX`** | `row + 1` saturating collapses focus onto anchor, so a triple-click on the last representable row selected **nothing at all** rather than the line — silently. It now runs to the far end of the row's own content and takes no break, there being no following row to separate it from. |
| **`start()`/`end()` were not mode-aware** | For a block dragged from `(2, 9)` to `(5, 3)`, `anchor.min(focus)` answers `(2, 9)` — a corner that is neither the rectangle's top-left nor anywhere the user pointed. A caller placing a caret or scrolling to the selection lands off to the right of the block it means to show. |

**Four negative controls, each applied, observed and reverted:** the naive byte composition (1 fail,
on the §5.6 case); block claiming the line break (2 fail); the empty-selection guard removed from
`row_span` (2 fail); and the three fixes reverted together (**3 fail, one per fix, no overlap**).

### Word granularity is UAX #29's, and that is a deliberate cost

`CellModel::word_at_cell` takes boundaries from the same segmenter as the grapheme clusters, so
`192.168.1.1` and `foo_bar` stay whole while **`foo-bar` and `2026-08-06` split at the hyphens** —
the one a log reader will notice. The alternative is a hand-written character-class table, and this
module's history is that every hand-written rule about text was wrong in at least four ways. A
log-aware granularity that treats a timestamp or a path as a unit is a separate later thing.

**Not modelled, and recorded rather than papered over:** *sticky* granularity — dragging after a
double-click continuing to snap to whole words — which `UI-DESIGN.md` §12 implies. Extension itself
is `set_focus` and belongs to the caller's input handling; autoscroll-on-drag is the grid's.

### ▶ Resume here

- **Visual reordering for right-to-left runs** — `shape.rs` established this is the drawing code's job.
- **Horizontal scrolling**, which §3.3's extent fields exist to size.
- **Still nothing consumes `Grid` or `Selection`.** Both are exported and unwired, so their defects
  stay latent until a viewport is attached — and the static exe is still unchanged, so every size
  figure quoted since M2 remains an LTO artefact.

---

## ✅ V3 has started — the scroll model, and the extent fields it owed — 2026-08-06, session 13

`crates/tailhawk-core/src/grid.rs`, plus `SPEC.md` §3.3's extent fields in `index.rs`/`indexer.rs`.
**229 tests, all passing** (229 core + the shell's one), fmt and clippy clean, CI green on x64 and
ARM64. **Both of V3's two prerequisites from session 11's note are now done.**

### The grid: `u64` scroll, row layout, hit-test

`SPEC.md` §6.4's three rules, enforced structurally. [`Scroll`] is `(u64 row, f32 sub_row_px)` and
**cannot represent a content-pixel offset**; `visible()` computes each row's `y` from a loop counter,
never from `row * row_height − offset`. §6.4's required CI test is in CI.

**⚠ §6.4's claim that the first-row assertion "catches all three" is not true of *this* grid, and
the test was strengthened because of it.** Reintroducing rule 2 faithfully leaves the **first** row
exact — with an exact `(u64, f32)` position the two content-magnitude terms are the same expression
and cancel perfectly at `i = 0`. The damage lands on every row after it. egui's first row is
misplaced only because its `offset` is an independently-rounded `f32`, which is rule 1's violation as
well. **The sweep now asserts every visible row.** Verified by mutation both ways.

`experiments/g7-egui-scroll/src/main.rs` was **deliberately not opened** — it replicates egui's
arithmetic and `CLEANROOM.md` says it must not be linked in. Everything came from §6.4 and G7's own
`RESULTS.md`, which state the rules as the inverse of what egui does. The §5 entry was filed
**before** the code (`5adea80`), and says exactly that.

### 🔍 Review found four defects in the grid, and the first two were live bugs

| Defect | Effect |
|---|---|
| **`clamp` was one-sided** | It only pulled the position *back* when past the end. **Shrinking** the viewport moves the end *up*, so nothing fired: dragging an 800 px window to 600 px while tailing left the newest 10 rows off screen with `is_at_bottom` quietly false. A row-height/DPI increase did the same. §5.4's whole feature, broken by an ordinary resize. Fixed: a viewport that *was* following stays pinned. |
| **One `NaN` blanked the grid permanently** | `NaN.clamp(0,1)` is `NaN`, so it reached `sub_row_px`; `visible()` then computed a row count of zero and **drew nothing, for ever**, with no scroll able to clear it (`NaN + delta` is `NaN`). A scrollbar dividing by a not-yet-laid-out track height produces one on frame 1. `INFINITY` reached the same state through `0.0 * inf`. Fixed, and `f32::max` alone is *not* enough — it neutralises `NaN` but passes `INFINITY`. |
| **Hit-test had no lower bound** | Only the top edge was guarded, so `row_at_y(900)` on an 800 px viewport returned row 47 when the last drawn row was 42 — a drag-select leaving the bottom selects rows nobody can see. |
| **The module doc overclaimed** | "No content-magnitude number is formed anywhere in this module" is false: `thumb_fraction`/`scroll_to_fraction` form them deliberately in `f64` (and G7 exonerated exactly that mapping). Reworded to what is true. |

A fifth finding was the sharpest: **`the_sub_row_remainder_never_leaves_its_band` could not observe
the failure mode it is named for**, because `clamp` rewrote an out-of-band remainder before the test
could read it. The carry arithmetic is now the free function `carry`, tested on its **unrepaired**
output. Same shape as `raster.rs`'s `tighten` and `shape.rs`'s `glyph_range`: extract the arithmetic
so a test can see it before anything downstream tidies up.

**One live bug was found by my own test before review**: scrolling far up saturated the row at 0 but
left the remainder behind, landing at `(0, 16 px)` — a *lower* position than the top. The row and the
remainder have to be clamped as a pair, which is why `step` is a function.

**Four negative controls, each applied, observed and reverted:** rule 2 reintroduced faithfully
(2 fail, and the strengthened required test makes 3); the one-sided clamp restored (**2 fail**); the
`NaN`/infinity guards removed (1 fail); the hit-test lower bound removed (2 fail).

### §3.3's horizontal extent, captured during the scan

`Extent` on `LineIndex`, fed by `LineScanner` — one `OR` per byte for `all_ascii`, and a running max
for `max_byte_len`. §3.3 says these are free during the newline scan and that recovering them later
means a second pass over 10 GB.

**The subtlety is that a long line straddles a chunk boundary.** Workers keep no per-line buffer by
design, so the max has to be tracked as the bytes go past — and **taking the max of two half-lines
understates the answer**, which would make §3.3's "upper bound" not one and let content be wider than
the scrollbar admits. `Extent::merge` carries a head and tail fragment and adds them across the join.
Tested against an independent scan at chunk sizes 1–4096 and 1–8 threads.

**`all_ascii` is a statement about bytes, and `exact_cells` is where the encoding is resolved.** A
UTF-16LE file of pure ASCII has every byte below `0x80` — the high halves are `0x00` — but its byte
length is twice its cell count, so it returns `None` rather than claiming exactness. §3.3 is also
explicit that a max *cell* count is **not** free here: it needs grapheme segmentation at 10–50× the
cost, and the cell model does not exist when the index is built.

**⚠ Following does not widen the extent yet.** `push_line` cannot: a line start alone gives no
length, since the line is not over until the next one begins. That belongs with M4's follow work and
is asserted in the doc rather than left to be discovered.

### ▶ Resume here: the rest of V3

- **Selection** — `PLAN.md`'s V3 row is "u64 scroll model, hit-test, selection", and selection is the
  piece not started. `cell.rs::byte_at_cell` is the horizontal half; the vertical half is `Grid`.
- **Visual reordering for right-to-left runs**, which `shape.rs` established is the drawing code's
  job (see below).
- **Horizontal scrolling**, which is what the extent above exists to size. §6.4 cut `--wrap` from v1
  in its favour.
- **Nothing consumes `Grid` yet** — it is exported but unwired, so the review's findings were latent
  rather than live. They become live the moment a viewport is attached.

**And the size question is now genuinely answerable**, because the grid is the thing that was going
to reference the atlas and the shaper. It still does not: `crates/tailhawk` references neither, so
the static exe is unchanged and every figure quoted since M2 remains an LTO artefact.

---

## ✅ The shaping bridge is built — `shape.rs`, 2026-08-06, session 13

`crates/tailhawk-core/src/shape.rs`. **199 tests, all passing** (198 core + the shell's one), fmt and
clippy clean, CI green on x64 and ARM64. **The V2/V3 gap is closed.** `glyphs.rs`'s test-only
`glyphs_for` — one code point, one glyph — now has a real replacement: `Shaper::shape` takes a line
and a face and returns, per grapheme cluster, which glyph ids draw it. **V3 may be built on this.**

**It holds no device and no window**, like `raster.rs`, so every test runs in-process. The call
sequence is `AnalyzeScript` + `AnalyzeBidi` → `GetGlyphs` → `GetGlyphPlacements`, and the module
authors both COM callbacks (`IDWriteTextAnalysisSource` / `IDWriteTextAnalysisSink`) as one object.

### ⚠ `GetGlyphs` returns glyphs in *logical* order, even with `isRightToLeft = true`

**This was written from memory the other way round and two tests failed.** The obvious guess is that
a right-to-left run comes back already reversed. It does not — the array is in logical order and
`clusterMap` still rises. **Visual reordering is the drawing code's job, and V3 owes it.**

The discriminator is worth keeping: **alef joins to its right but not its left**, so `ابب` is
unambiguously `[alef, beh-initial, beh-final]` logically and the reverse visually, and alef's glyph
is identifiable from the `.cmap`. It came back at index 0. `directwrite_returns_glyphs_in_logical_order`
pins it; `an_arabic_run_resolves_to_a_right_to_left_bidi_level` proves the run really was marked RTL,
so the finding is about DirectWrite rather than about a run that was never flagged.

`glyph_range` deliberately does **not** depend on the answer: it reads the map's direction from the
map. That is why the fix was a doc correction rather than a rewrite.

### 🔍 The adversarial review found two real defects, and both were live bugs

The owner's standing condition, and it paid again. Neither was caught by the 25 tests that existed.

| Defect | Effect |
|---|---|
| **`glyph_range` asked only whether the cluster's *first* position was shared** | The two clusterings do not nest. A grapheme cluster whose first position was swallowed by a preceding ligature but whose *later* positions open a DirectWrite cluster of their own was declared absorbed, and **its glyph was claimed by nobody and never drawn**. `\u{200B}\u{FE0F}\u{E0067}` is a real line that does it — and the dropped glyph was a **tag character**, the hidden-text vector §13.4 names and §5.6 forbids discarding silently. Fixed: ownership is decided by where a DirectWrite cluster *opens*. |
| **`snap_to_cluster` moved a straddling boundary *backwards*, which deleted it** | A cluster's start is usually already a cut, so the moved boundary collapsed onto it and vanished — and the run then swallowed everything to the next surviving cut. `بि` is one cluster (`U+093F` is `Mc`), so `بिक्षि tail` shaped the **whole Devanagari word as right-to-left Arabic**: conjunct ligature lost, matra not reordered, cluster **77% wider** — column drift against §3.3's own acceptance test. It reached past the script too: `GET /a بिक्षि 200` shaped the trailing `200` as Arabic. **Six attacker-supplied bytes changing the rest of the line is exactly what §13.4 calls a real defect.** Fixed: boundaries snap *forward* to the cluster's end, a position no earlier cut can occupy. |

Two more findings were about honesty and are corrected: the doc for `glyph_range`/`ClusterGlyphs`
stated the buggy rule as if it were the right one, and **`every_glyph_is_claimed_by_exactly_one_cluster`
did not test the property it claimed to** — it drove only one-code-unit clusters, and multi-unit
clusters are the entire reason shaping exists. It now sweeps every monotonic map of up to four
positions, rising *and* falling, crossed with **every partition of those positions into clusters**;
that cross product is what surfaced the dropped glyph. A `u32` truncation guard on absurd line
lengths and one genuinely dead filter went with them.

**Both fixes were verified by reverting them**: restoring the old absorbed rule fails 3 tests
including the real-text sweep; restoring backward snapping fails 3 including the Devanagari one.

### Seven negative controls on the original, each applied, observed and reverted

| Mutation | Result |
|---|---|
| ligature "absorbed" guard removed | 2 fail — both clusters claim one glyph |
| neighbour search made forward-only (the left-to-right assumption) | 2 fail |
| forward scan past equal `clusterMap` entries dropped | 2 fail |
| UTF-16 length replaced by byte length | **8 fail** |
| `E_NOTIMPL` returned from `GetLocaleName` | **9 fail** — every DirectWrite test |
| wrong HRESULT compared in the retry loop | 1 fail |
| forward scan started at `start+1` rather than `start+len` | **did not fire** |

**The one that did not fire changed the code, again.** It was a no-op on every fixture, but it is not
redundant: a grapheme cluster DirectWrite splits into two of its own gives a rising map *inside* one
cluster, and the two forms differ there. `a_cluster_directwrite_splits_still_takes_all_of_its_glyphs`
is that case, and the control fires with it present. Same shape as `raster.rs`'s `tighten` — **a
control that does not fire is a statement about the fixtures.** It is now three for three.

### Two claims that were measured rather than asserted

- **Which source callbacks actually run.** Instrumenting all five showed `AnalyzeScript`/`AnalyzeBidi`
  calling `GetTextAtPosition`, **`GetLocaleName` and `GetParagraphReadingDirection`** — and *not*
  `GetTextBeforePosition` or `GetNumberSubstitution`. `E_NOTIMPL` from the latter two is harmless;
  from `GetLocaleName` it fails the whole analysis. The comment now says only that.
- **The retry loop is exercised, not assumed.** `shape_from` takes the first buffer guess, so
  starving it to one glyph drives the real `ERROR_INSUFFICIENT_BUFFER` path — the same trick
  `rasterise_in_batches` uses for batch size. A bounded-growth test covers the give-up path too.

### What shaping still owes

- **Visual reordering for right-to-left runs.** Established above as the caller's job; nothing does
  it yet. V3.
- **Font fallback.** `shape` takes one face and a codepoint it lacks shapes to `.notdef`, exactly as
  `Face::glyph_indices` behaves. Noticing that and trying the next face is the caller's, and
  `GlyphKey` is keyed by face precisely so a fallback glyph cannot collide.
- **The locale is fixed at `en-us`.** It selects language-specific forms (`locl`); a log file carries
  no language tag, so a stable wrong answer beats a guessed one — but it is a limitation, not a
  decision.
- **Nothing consumes `Shaped` yet**, so `advances` and `offsets` are produced and unused. They are
  what places marks *within* a cluster; §3.3 still gives the cell grid the final say on where a
  cluster starts.

**⚠ Static `tailhawk.exe` is 506,368 bytes — byte-for-byte the previous figure, and it means
nothing.** Nothing in `crates/tailhawk` references shaping, so LTO strips the whole module. Session 8
recorded this trap and it still applies: **the real size cost lands at V3**, when the grid references
the atlas and the shaper together.

**CLEANROOM needs nothing.** Session 12 filed the §5 entry (`04cd386`) *before* the code, which is
what §1.5 asks and what had slipped three times. No source outside the repo was consulted this
session at all — the ordering finding came from a probe against the running DirectWrite, not a page.

### ▶ Resume here: V3, the grid

~~Everything V3 needed from V2 now exists. The two things `SPEC.md` §3.3 still names: the **u64
scroll model, hit-test and selection**, and the **`max_byte_len` / `all_ascii` extent fields
captured during the index scan**.~~ — **the scroll model, hit-test and the extent fields are all
done; see the top of this file. Selection is what remains of V3.**

---

**Paused:** 2026-08-06, session 13. **M1 and M2 are complete** — E3, E4 and E8 all done, CI green on
both architectures. Only M2's two large-fixture done-criteria remain unrun. **M3 is under way: V4
(the cell model), V1 (the device), V2's monochrome path, the shaping bridge and V3's scroll model
are all done. What is left in M3: V3's selection, V2's colour path and per-DPI rebuild, and wiring
any of it to the shell — nothing in `crates/tailhawk` references the grid, the atlas or the shaper
yet.** `PLAN.md` marks M3 the highest-risk milestone, with a stop-and-reconsider gate at >50%
overrun. **Everything below is on disk and pushed. Nothing is held in a chat session.**

---

## ✅ V2's mono path is joined up end to end — 2026-08-06, session 11

`crates/tailhawk-core/src/glyphs.rs`. **171 tests, all passing**, fmt and clippy clean. Glyph id in,
quad out: `GlyphCache` owns the rasteriser, the atlas, the sheet and the face, and
`real_text_reaches_real_pixels` drives "Hi" all the way from a code point to dark pixels on light
paper in an offscreen target. **The monochrome half of V2 works.**

**The rule it exists to enforce is `SPEC.md` §3.2's *a frame must never block on rasterisation*, and
it is enforced structurally rather than by discipline.** `quad` cannot rasterise — it has no way to.
A glyph that is not resident returns a **placeholder** quad and joins a queue; `flush_misses` does
the rasterising, and the caller runs it **after presenting**. That ordering is the whole requirement:
a genuinely cold 1,500-glyph viewport is 162 ms, 8–10 frames, because DirectWrite's cross-process
font cache is empty for those glyphs the first time a machine ever draws them.

**The placeholder is a hollow box, and it lives outside every slot the atlas can hand out.** The
sheet is 1024×1024 but the atlas is told it is one cell-row shorter, so the reserved cell cannot be
evicted no matter how hard the atlas thrashes — which is what stops a full sheet from having nothing
to draw a miss with. A hollow box rather than a solid block because a screen of solid blocks reads as
corruption while a screen of outlines reads as loading, and it is what a terminal already shows for a
glyph it cannot render.

**Three negative controls, each applied, observed and reverted:**

| Mutation | Result |
|---|---|
| the atlas is given the placeholder's row | **all 6 fail** — and loudly, because `Sheet::upload` refuses the out-of-sheet write rather than corrupting a slot. The reserved row is load-bearing |
| a no-ink glyph is not recorded as blank | `a_space_is_recorded_as_blank_and_stops_being_a_miss` fails — G4's 440-misses-per-frame bug |
| *(the placeholder path, tested by the two assertions in `asking_for_a_glyph_never_rasterises_it`: the placeholder occupies exactly the cell the real glyph will, and the second frame samples a different slot)* | |

**⚠ Shaping is not solved and nothing here pretends otherwise.** ~~The one-code-point-one-glyph
mapping used by the tests is `glyphs_for`, confined to the test module **on purpose**, with a comment
saying it is wrong for Devanagari, Arabic and anything else §3.3 names. **V3 must not be built on
it.**~~ — **solved in session 13 by `shape.rs`; see the top of this file.** `glyphs_for` is still
test-only and still wrong, and still must not be used; `Shaper::shape` is what V3 builds on. What
`GlyphCache` takes is a glyph id, which was already the right currency — deciding *which* glyph ids
draw a cluster is what the shaper now answers.

**Still owed by V2:** the colour path through `TranslateColorGlyphRun` into a second sheet; §3.2's
per-DPI rebuild (`Atlas::clear` exists for it, nothing calls it, and `GlyphCache` has no way to change
`px_per_em` yet); re-priming the placeholder after a device rebuild (`prime` is separate from `new`
precisely so it can be, but nothing wires it to the recovery path); and moving rasterisation to a
worker rather than merely after the present.

---

## 🚧 V2's glyph pass draws, and the pixels were read back — 2026-08-06, session 11

`crates/tailhawk-core/src/text.rs`, `sheet.rs`, `shaders/glyphs.hlsl`, plus an offscreen test
harness in `gpu.rs`. **166 tests, all passing**, fmt and clippy clean. `SPEC.md` §3.2's single
instanced draw now exists: one `DrawInstanced` over a structured buffer of glyph quads, with
per-instance foreground colour, so per-token colouring is free.

**The dual-source blend state is real and verified against actual pixels.** `SrcBlend = ONE`,
`DestBlend = INV_SRC1_COLOR`, the pixel shader emitting premultiplied colour in `SV_Target0` and
per-channel coverage in `SV_Target1`, so the hardware computes `dest = c0 + dest * (1 - c1)`. The
trap G4 found is in the code with a comment on it: **the alpha slots take `INV_SRC1_ALPHA`; a
`*_COLOR` factor in an alpha slot fails `CreateBlendState` with a bare `E_INVALIDARG`.**

**Everything is tested by drawing into an offscreen target and reading the pixels back** — no window
and no swapchain, via `gpu::offscreen`. That is the only way to know: every call can succeed while
producing the wrong composite, and a flip-model back buffer is undefined after `Present`.

### ⚠ The ClearType test passed for the wrong reason, and the negative control is what caught it

The first version drew a **light tint over a dark backdrop** and asserted channel spread. It passed —
and it **also passed with the blend rewired to a single alpha**, which is exactly the state the test
exists to reject. The spread was coming from `c0.rgb = tint.rgb * cov`, the *source* term, which is
per-channel no matter what the destination factor does.

The arrangement that discriminates is **black ink on light neutral paper**: with `tint.rgb = 0` the
source contributes nothing, so every channel of the result comes from `dest * (1 - c1)` alone.
Rewired to a single alpha it now returns `[109, 109, 109]` — perfectly neutral, the collapse made
visible. It also asserts the *ordering*, not just the spread: the channel given the most coverage is
attenuated most, so it comes back darkest.

This is the second time this session that **a control which did not fire changed the code rather than
being written off** (the first was the rasteriser's cell extent). It is the most useful habit in this
project: a test that passes tells you nothing until you have watched it fail.

| Mutation | Result |
|---|---|
| `DestBlendAlpha` set to `INV_SRC1_COLOR` | **all 7 fail** — `CreateBlendState` rejects it outright, which is G4's documented trap |
| `DestBlend` set to `INV_SRC1_ALPHA` (one alpha, not three coverages) | the ClearType test fails — **only after it was rewritten**; the original passed |
| `Instance` fields reordered, same size | 2 fail. Same-size layout drift is the real hazard: a structured buffer is read as raw bytes, so it does not fail to compile, it draws nonsense |

### Four decisions worth not re-litigating

- **The device now demands feature level 11_0** and nothing lower. The glyph pass reads its quads
  from a `StructuredBuffer` in the *vertex* shader, which is shader model 5; on a feature-level 10
  device the shaders simply fail to create, and there is no text. Better to fall to WARP, which is
  always 11_0. **11_1 is deliberately not requested** — a runtime that does not know it rejects the
  whole array with `E_INVALIDARG` rather than skipping the entry.
- **The sheet is `D3D11_USAGE_DEFAULT` with `UpdateSubresource`, not `DYNAMIC` with `Map`.** A
  dynamic texture must be mapped `WRITE_DISCARD`, which throws away every glyph already resident.
  The whole point of an atlas is that they stay.
- **Point sampling, clamped.** A cell is addressed in whole texels at the scale it was rasterised
  for, so filtering can only blur it — and bleed the neighbouring slot's ink in at the edges.
- **The colour sheet is not created yet, and `t2` is left unbound on purpose.** An unbound SRV
  samples as zero, so a stray `MODE_COLOUR` instance draws nothing rather than garbage — which a
  test asserts. Creating an empty 4 MB texture nothing writes to would be worse.

**Static `tailhawk.exe` is 506,368 bytes** — still 3.2% of §11.2's 15 MB gate, and still not a
measurement of the atlas, because nothing in the shell references it yet. The build runs and comes up
on the hardware rung with feature level 11_0 enforced.

---

## 🚧 V2 has started — the atlas allocator and the rasteriser, 2026-08-05, session 11

`crates/tailhawk-core/src/atlas.rs` and `crates/tailhawk-core/src/raster.rs`. **159 tests, all
passing** (158 core + the shell's one), fmt and clippy clean. Between them: which glyph lives in
which cell, which cell is given up when the sheet is full, which glyphs have no ink, and what the
pixels of a cell actually are. **Neither is wired to a device yet** — the sheets, the upload and the
instanced draw are the next thing to build.

**Nothing here was designed this session.** Every rule is a conclusion of `g4-glyph-atlas` or
`g4b-batched-raster`, and the module cites the measurement beside each one:

| Rule | The measurement behind it |
|---|---|
| Uniform slots, one glyph each | A variant allowing a glyph to span adjacent slots cost **106 ms/frame** — the victim search became O(capacity × span) and had to find a free *run* |
| O(1) intrusive victim list | **4–9 ms/frame** scanning for the oldest against **0.17–0.37 ms** for the list. 20–28x in every run |
| A slot touched this frame is never evicted | Otherwise the frame overwrites a cell it is about to draw from |
| Cache the absence of ink | G4's first run: exactly **440 spurious misses per frame** from ten spaces per row |
| The slot size comes from measured bounds, never from em | G4b guessed 20×26 for em 14 and clipped **1,086 of 1,500** glyphs — and the clipped cells then compared *equal*, which reads as a correctness pass |

**Vacant slots start at the head of the LRU list**, so allocation and eviction are the same operation
and neither needs a search or a separate free list. The head is either an empty cell or the oldest
resident; if the head was used this frame then every slot was, and `insert` reports
`SheetFullThisFrame` rather than corrupting the frame.

**Three decisions the experiments did not settle.**

- **The key carries the face and a synthetic-style flag**, not just §3.2's "(glyph id, style, dpi
  scale)". Glyph ids are face-local, and font fallback (§3.3) means one viewport draws from several
  faces at once — glyph 42 of the CJK fallback is not glyph 42 of the primary. Bold and italic are
  normally separate faces and so are already covered; what is not is a *synthetic* bold or oblique
  applied to the same face, which is the same glyph id at the same size with a different raster.
- **The size in the key is whole device pixels**, not points and not a scale factor, because §3.2
  requires integer device-pixel advances and a float key could hold two rasters that round to one
  cell.
- **The blank set is bounded and dropped wholesale when it overflows.** A blank costs no slot, so
  nothing else bounds it, and a viewer left open for days on logs full of unusual codepoints would
  grow the map for ever. Rebuilding a blank costs one miss, not a rasterisation, so it does not earn
  an LRU of its own.

**Three negative controls, each applied, observed and reverted:**

| Mutation | Result |
|---|---|
| the never-evict-this-frame guard removed | `a_slot_used_in_this_frame_is_never_evicted` fails |
| `lookup` no longer touches — i.e. FIFO, not LRU | `the_glyph_nobody_asked_for_is_the_one_evicted` fails |
| eviction forgets to drop the victim's map entry | **3 tests fail**, including the thrashing one, where two keys end up pointing at one slot |

### The rasteriser — `raster.rs`, mono only

`Rasteriser` holds an `IDWriteFactory2` and needs **no device and no window**, which is why all of
this is testable in-process. `resolve` walks a candidate family list; `measure_cell` derives the
uniform cell from measured bounds; `rasterise` fills one cell per glyph as RGBA — **R, G, B are the
three ClearType coverages and A is their average**, so one sheet serves subpixel and greyscale at no
extra memory (G4).

**`BATCH = 16`**, inside G4b's measured 4–64 window and well clear of the 256 mark where the larger
intermediate bitmap makes batching *slower* than not batching.

**G4b's headline finding is now a regression test in the product**:
`batched_rasterisation_is_bit_identical_to_one_glyph_at_a_time` compares batch 4, 16 and 64 against
one-glyph-at-a-time over printable ASCII. Grid fitting and the ClearType filter are both
position-dependent, so this could legitimately have been false; if a future DirectWrite makes it
false, batching has to go, and that test is what will say so.

**A glyph with no ink is `None`, decided by inspecting the cell rather than the strip.** G4b detected
blanks per *chunk*; within a non-empty strip an individual space is an all-zero cell, and missing it
is G4's 440-spurious-misses-per-frame bug.

**Two negative controls fired; one did not, and that one changed the code.**

| Mutation | Result |
|---|---|
| the run advance no longer matches the slicing step | the bit-identical test fails |
| empty rectangles are allowed into the measured extent | 2 tests fail — **but only after the extent fold was extracted** |

The extent fold is now the free function `tighten`, tested on synthetic rectangles, because **on the
faces this machine has, the mutation was a no-op**: an empty `RECT{0,0,0,0}` already sits inside the
ink extent of printable ASCII, so including it changes nothing and no test over real glyphs can tell
the two behaviours apart. It matters on a face whose ink box does not contain the origin, and since
the cell origin *is* every glyph's placement offset, getting it wrong shifts the whole grid. Worth
remembering as a shape: **a control that does not fire is a statement about the fixture, not about
the code.**

**⚠ No size figure is quoted for the DirectWrite dependency, deliberately.** Nothing in
`crates/tailhawk` references the atlas or the rasteriser, so LTO strips both and the exe would
measure unchanged. Session 8 recorded exactly this trap when `encoding_rs` first went in. **The real
cost lands when the grid references them**, which is V3.

**⚠ What V2 still owed at the time of this section** — the sheet, the blend state and the draw
have since been built; see the section above. Left after that: the colour path and the join. ~~the
two sheets and the upload; the dual-source blend state and the instanced draw
(`SrcBlend = ONE`, `DestBlend = INV_SRC1_COLOR`, and **the alpha slots take `INV_SRC1_ALPHA` or
`CreateBlendState` fails with a bare `E_INVALIDARG`**); the colour path through
`TranslateColorGlyphRun`; and the placeholder-and-fill-in-later behaviour that keeps rasterisation
off the paint path.~~ **Read both `RESULTS.md` files before writing any of it** — G4's absolute
µs/glyph figures are cache-thrash figures and are superseded by G4b.

---

## ✅ V1 is done — device-removed recovery, 2026-08-05, session 11

`crates/tailhawk-core/src/gpu.rs`. **136 tests, all passing** (135 core + the shell's one), fmt and
clippy clean. The device, swapchain and WARP chain were already M0's; what V1 owed was the recovery,
and `SPEC.md` §3.2 states it as a prohibition — "**Never panic on device-removed** — that is the
exact bug that crashes wgpu apps on driver auto-update."

**A lost device is now rebuilt inside `Renderer::paint`, and the shell is never told.** A driver
auto-update, a TDR, a GPU reset, unplugging an external monitor, a laptop switching between its
integrated and discrete GPUs — all of them arrive as the same four `DXGI_ERROR_DEVICE_*` codes, none
is a program error, and each is answered by building a new device and redrawing the frame.

**The policy is separated from the D3D calls so it can be tested without a GPU**, and it has one
subtlety worth not re-deriving:

| Rule | Why |
|---|---|
| First loss retries **the same rung** | A driver update leaves the hardware perfectly able to host the next device. Dropping to WARP over one blip trades the GPU away for nothing. |
| Second loss with no clean frame in between **pins WARP** | A device that dies again immediately is not a blip, and WARP cannot be removed by a driver. |
| **Only a frame that needed no recovery clears the streak** | Otherwise a device that dies and is rebuilt on *every* frame resets the count each time and rebuilds for ever. |
| Three rebuilds in one unbroken streak, then **give up** | Rebuilding a D3D11 device on every `WM_PAINT` is worse for the user than a static window. Giving up returns `Err`; the shell drops to the flat background it painted before a device existed, which is still not a panic. |

**The DXGI factory is rebuilt with the swapchain rather than cached, and that is the actual trap.** A
factory created before a device was removed is stale; a renderer that reuses it comes back
"recovered" while presenting to nothing. `present()` also *fails* rather than silently succeeding
when a window is attached but no swapchain exists — which is precisely the state a rebuild that
forgot to re-attach leaves behind.

### How something D3D11 gives you no way to trigger got tested anyway

D3D11 has no `RemoveDevice` — D3D12 does — and the real causes are a driver update or a TDR, neither
of which a test can arrange. So a `test-hooks` feature injects the `DXGI_ERROR_DEVICE_REMOVED`, and
**everything after that point is the real path**: the same classification, the same policy, a real
device rebuild, a real `Present`. Under resolver 2 a feature turned on by a dev-dependency does not
reach `cargo build`, and `cargo tree -e normal` confirms the shipped binary does not carry the hook.

**The swapchain half of recovery is tested from the *shell* crate, not the core.** The core may not
own a window (§3.1), so its own device tests rebuild with nothing attached — and a rebuild that drops
the swapchain passes all of them. `crates/tailhawk/src/main.rs` has the one test that attaches a real
hidden window, loses the device, and then resizes and presents again.

**Three negative controls, each applied, observed and reverted:**

| Mutation | Result |
|---|---|
| `rebuild` does not re-create the swapchain | **Only the shell test fails.** All 135 core tests pass. That failure is the entire justification for testing this from the shell. |
| escalation removed (always retry the current rung) | 2 tests fail, including the one that checks a *real* rebuilt device landed on WARP |
| the clean-frame reset removed | `losses_separated_by_clean_frames_are_recovered_from_indefinitely` fails — the realistic case, a machine that loses its device occasionally over a long session |

**One from-memory constant was written and deleted before commit.** The first draft carried
`D3DDDIERR_DEVICEREMOVED = 0x88760870`, recalled rather than looked up. Grepping both installed
Windows SDKs found no such symbol, so it went. The four DXGI codes are each taken from the `windows`
crate's generated bindings, and `was_lost` asks `GetDeviceRemovedReason()` as well — which covers a
call that fails with some other code while the device is being removed underneath it, and is why the
exotic constant bought nothing anyway. Same failure mode as E8's severity tables; caught before
commit this time rather than by review.

**Two smaller things came with it.** `Renderer::driver()` is now live rather than fixed at startup —
recovery can move the device to WARP and the title bar is the only place this build says which rung
it is on, so the shell re-reads it after each paint. And `#![windows_subsystem = "windows"]` is now
`cfg_attr(not(test), …)`: a GUI-subsystem test harness has nowhere to print.

**Static `tailhawk.exe` is 505,856 bytes**, against 502,784 at M1 and V4. Still 3.2% of §11.2's
15 MB gate. Smoke-tested: the static build opens, comes up on the hardware rung, and titles itself
`Tailhawk — hardware`.

---

## 🚧 M3 has started — V4, the cell model, 2026-08-05, session 10

`crates/tailhawk-core/src/cell.rs`. **125 tests, all passing**, fmt and clippy clean.

**M3 is `PLAN.md`'s highest-risk milestone** (11 weeks, with a stop-and-reconsider gate at >50%
overrun), so it starts with the piece that is portable, headless and needs no device: §3.3's cell
model. V1's device work is largely M0's already; V2 (the atlas) and V3 (the grid) both sit on top of
this arithmetic, and §3.3's acceptance test — *columns line up* — is decided here rather than in
DirectWrite.

**The rule: delegate to `unicode-width`'s cluster-aware `UnicodeWidthStr::width`, one cluster at a
time.** It already handles ZWJ sequences, regional-indicator pairs, keycaps, presentation selectors,
conjoining jamo and spacing marks. Exactly two things are layered on top: a control character takes a
visible cell (§5.6 forbids dropping it), and an emoji-modifier sequence is two cells rather than the
2 + 2 `unicode-width` gives it.

**⚠ The first version of this file hand-rolled that algorithm and was wrong four ways** — see the
review section below. The reason is worth keeping: the premise was that a cluster's width is its
*base's* width, on the theory that anything cluster-aware must be summing code points and would call
`👨‍👩‍👧‍👦` eight cells. `UnicodeWidthStr::width` in `unicode-width` 0.2 does not sum — it segments — and
that premise cost four defects, three deleted special cases and a test that asserted the wrong
answer for a script §3.3's own acceptance test names.

**Two dependencies were added**, `unicode-segmentation` and `unicode-width` — pure Rust, no C
toolchain, Apache-2.0 OR MIT, so `deny.toml` admits them without a new entry. The alternative is
shipping and then ageing our own copy of the Unicode character database, which is not a maintainable
position for a project that expects to see CJK, Devanagari, RTL and emoji in real logs.

**§13.4's "reveal invisibles" toggle is a width question, not only a painting one** — a revealed
zero-width space occupies a cell — so `CellModel` carries the flag. That is also what makes a Trojan
Source `U+202E` override visible rather than merely isolated.

**⚠ But §13.4 cannot yet be claimed, and the gap is asserted rather than hidden.** The toggle reveals
an invisible that forms its *own* cluster — `U+200B`, `U+202E` — and **not** one absorbed into a
preceding cluster: `a` + ZWJ, `a` + a tag character (`U+E0067`, the hidden-text vector), `a` + a
variation selector. Closing it needs `General_Category` data to tell a `Cf` character from a
legitimate combining mark, which must *not* be revealed, and probably needs reveal mode to segment
differently rather than only measure differently.
`revealing_does_not_yet_reach_an_invisible_inside_a_cluster` pins the current behaviour so the gap
stays visible.

### V4's review found six defects, and the two hit-test ones were the dangerous pair

| Defect | Effect |
|---|---|
| **A stray `U+FE0F` was treated as "make this two cells"** on *any* base | VS16 means something only on an `Emoji=Yes` base. **928,985** `<base> U+FE0F` clusters were over-counted. Appending three bytes anywhere in a line shifted every column to its right — attacker-controllable, and §13.4 calls a viewer that can be made to lie a real defect. |
| **`U+FE0E` likewise, under-counting** | A wide ideograph followed by a stray VS15 got one cell and painted over its neighbour. 182,716 under-counts. |
| **Multi-jamo Hangul** | `ᄀᄀ가` is one cluster of four cells; base-width said two. |
| **Devanagari and Thai spacing marks** | `कि` is two cells, not one. §3.3's "combining marks occupy 0 additional cells" is true of `Mn`/`Me`, **not** of `Mc` — and a *test of ours asserted the wrong answer*, for a script §3.3's acceptance test names by name. |
| **A zero-width cluster stole the next cluster's column** in `byte_at_cell` (`width.max(1)`) | 57 of 104 visible clusters hit-tested to the wrong byte. `"\u{202E}abc"` — §13.4's own Trojan Source example — returned the override's offset for a click on `a`. A caret placed from a click did not sit where the user clicked. |
| **Reveal-invisibles misses invisibles inside a cluster** | Documented above rather than fixed. |

**The reviewer verified all of it empirically** — a probe crate outside the repo, on the same locked
crate versions, sweeping every scalar value and fuzzing 500,000 strings — rather than reasoning about
Unicode from memory. That is the difference between this review and a plausible-sounding one, and it
is worth insisting on for anything touching Unicode.

It also confirmed **no panic, no non-char-boundary result, and `cell_count <= line.len()` across
every scalar value and both fuzz corpora** — which is the property §3.3's `max_byte_len` upper bound
for the scrollbar depends on.

**⚠ The binary did not grow, and that figure means nothing yet.** The static `tailhawk.exe` is
**502,784 bytes — byte-for-byte the M1 figure**, with two Unicode-table crates newly linked. That is
LTO stripping code the *shell* never references: nothing in `crates/tailhawk` calls the cell model,
the record model or the indexer. Session 8 recorded exactly this trap when `encoding_rs` first went
in, and it applies to everything M2 and M3 have added so far. **The real size cost of the Unicode
tables lands the moment the grid references them**, and that is the measurement to take at V3 — not
this one.

**CI is green on all three of session 11's commits** — `6d38a0e` (V1), `51a1bcf` (the allocator) and
`d66d551` (the rasteriser), runs `31011208042`, `31012052003` and `31013496467`, x64 and **ARM64**
both. DirectWrite cross-compiles to ARM64 without complaint, and the runner ran the same 158 + 1
tests this machine does.

### ✅ The device and font tests really do run on the CI runner — checked, not assumed

**The `Test` step runs with `-- --nocapture`, and that is load-bearing.** Several tests skip loudly
rather than fail when there is no D3D11 device, no window station or no usable font — but `cargo
test` captures the output of *passing* tests, so without `--nocapture` a skip and a real run are
indistinguishable in the log and a skipped device test reads as a green one. **A green device test is
not evidence the device path ran** unless the skip message is absent.

Run `31013808780` (`06005e0`) is the first with the messages visible: **no `SKIPPED` line anywhere in
the log**, against 8 `test result: ok` lines from the same fetch, so the log was genuinely read. So
the GitHub `windows-latest` runner has a D3D11 device, a window station and one of the candidate
faces, and it exercised device-removed recovery with a real swapchain and rasterised real glyphs.
That was an open question — the tests were written to degrade rather than fail precisely because a
headless runner was a real possibility.

**This paragraph replaces a claim I made without checking.** Session 11 first asserted the runner had
not skipped them, on the strength of the test *counts* — which cannot show it either way, because a
skipped test still counts as passed. The counts were right and the inference was not.

### ▶ Resume here: V3, and the shaping gap in front of it

~~V1 still owes device-removed recovery~~; ~~V2 owes the sheets and the draw~~; ~~nothing connects the
four pieces~~ — **V1 is done and V2's mono path is joined up end to end**, all session 11. See the
sections at the top of this file.

**V3 is the grid: the u64 scroll model, hit-test and selection, plus §3.3's `max_byte_len` /
`all_ascii` extent fields.** Two things to settle before writing much of it:

1. ~~**Cluster → glyph ids.** `GlyphCache` speaks glyph ids and `cell.rs` speaks grapheme clusters,
   and nothing bridges them.~~ — **done, session 13.** `shape.rs` bridges them via
   `IDWriteTextAnalyzer`. What V3 inherits from it: **glyphs come back in logical order even for
   right-to-left runs, so V3 owes the visual reordering**, and font fallback is the caller's job.
2. ~~**The extent fields must be captured during the index scan** — §3.3 says they are free there,
   and the reason is that recovering them later means a second pass over 10 GB. `index.rs` has
   neither field.~~ — **done, session 13.** `Extent` on `LineIndex`, fed by `LineScanner`, stitched
   across chunk boundaries. See the top of this file.

**And the size question finally becomes answerable at V3.** Every figure quoted since M2 has been
LTO stripping code the shell never references. The moment the grid references the atlas, the real cost
of `encoding_rs`, the two Unicode-table crates and DirectWrite all land at once.

- **V3 (grid)** owes §3.3's `max_byte_len` / `all_ascii` extent fields. **These must be captured
  during the index scan** — §3.3 says they are free there, and the reason is that recovering them
  later means a second pass over 10 GB. `index.rs` has neither field yet. Do this *with* V3's
  scrollbar, not before, but do not let V3 ship without it.
- **V2 (atlas)** is the higher-risk half and has the most prior art already measured:
  `experiments/g4-glyph-atlas/RESULTS.md` and `experiments/g4b-batched-raster/RESULTS.md` settle the
  eviction policy, the batching win (~1.8x, saturating at 4) and the cross-process font-cache
  behaviour. **Read both before writing any rasterisation code** — G4's absolute µs/glyph figures are
  cache-thrash figures and are superseded by G4b.

**Out of scope for this file, deliberately:** font fallback and the colour-glyph atlas are V2;
§3.3's `max_byte_len` / `all_ascii` horizontal-extent rule belongs with V3's scrollbar, because it is
the scrollbar that needs a conservative upper bound before layout has run. `cell_count_never_exceeds_byte_length`
is the test that pins the property that rule depends on.

---

## 🚧 E8 is done — the record model, 2026-08-05, session 10

`crates/tailhawk-core/src/record.rs`. **106 tests, all passing**, fmt and clippy clean.

The OTel-shaped record of §6.1, the severity banding of §6.2, and the tables every format detector
(E9/E10) will resolve through. Nothing here parses anything — this is the shape the parsers fill.

**The severity tables were transcribed from the OpenTelemetry spec, not written from knowledge, and
that decision paid for itself immediately.** The first draft was written from memory and had **syslog
Emergency at 23 with Alert and Critical in the FATAL band** — the appendix says **21, 19 and 18** —
and **`java.util.logging` FINER at 3** where the appendix says **5**, a whole band out. Every one of
those was plausible, confident and wrong, in the one table whose entire purpose is that a WARN means
the same number in every format. `CLEANROOM.md` §5 carries the entry.

| Table | Source |
|---|---|
| syslog (RFC 5424), log4j, Zap, Windows Event Log, `java.util.logging` | **OTel Appendix B, transcribed** |
| HTTP-status aliases (§6.2) | **Ours** — Appendix A describes the Apache access log but assigns it no severity. Opt-in, and labelled as ours in the code. |
| the generic level-word table | Ours, a union of the above plus the abbreviations real writers emit |

**Three decisions worth not re-litigating.**

- **Absent severity is `None`, and zero is not representable.** The OTel spec allows both a zero and
  omission; carrying both would be two spellings of one state. `UI-DESIGN.md` §11.2 renders it blank,
  never INFO.
- **`resource` is deliberately not on `Record`** — §6.1's own field table says it "belongs to the
  pane, not the row", which is what makes §8.3's merged-view column problem answerable.
- **`raw` is decoded text where §6.1 says "the original bytes".** Every consumer works in decoded
  text. But it *is* a loss — U+FFFD does not invert — so `Record::span` carries the byte offset and
  length, and **that** is what keeps §10.2's "copy preserving original bytes" and §10.3's "search and
  copy operate on the untruncated bytes" reachable. `raw` is the convenient form, not the
  authoritative one.

**`verbose` and `critical` are genuinely ambiguous across frameworks** — Serilog's `Verbose` is its
lowest level, Windows Event Log's is DEBUG (5); `Critical` is FATAL (21) in .NET and ERROR2 (18) in
Windows Event Log. Both are resolved against the *normative* table, since Appendix B has a Windows
table and no .NET one.

---

## 🔍 Both components were reviewed adversarially before commit — and the review found real defects

Two independent review agents were run over E4 and E8. **This is the practice the owner asked for on
2026-08-05, and it is the condition attached to working without checkpoints — keep doing it.** It was
not ceremonial; four real defects came out of it, none of which the test suites had caught.

| Found in | Defect | Status |
|---|---|---|
| `indexer.rs` | **A file that shrank after being sized invented a line.** `end` is sampled before the scan, so a copy-truncating writer (§5.5, one of the three rotation modes) leaves the file shorter. A range that read *nothing* still reported one line at `start`, and the trailing-terminator test compared against `end` rather than the bytes actually read, so `a\nb\n` sized at 40 reported **3 lines where there are 2**. | **Fixed**, with `a_file_that_shrank_after_it_was_sized_reports_only_what_is_there`. Reverting the fix reproduces the 3-vs-2 failure. |
| `indexer.rs` | `from + chunk_bytes` overflowed on an absurd caller-supplied chunk size — panic in debug, and in release it wraps to `to < from` and **silently drops the chunk's lines**. | **Fixed** with `saturating_add`, plus a test. |
| `record.rs` | **`java.util.logging` FINER and FINE were a band out** (see above). | **Fixed** against the spec, with the mapping test. |
| `record.rs` | **`level >= Critical` excluded the rows whose level reads `Critical`** — the band name parsed as FATAL (21) while a `Critical` row resolved to 18. §6.2's whole purpose is that this cannot happen. | **Fixed**, and `a_name_means_the_same_thing_however_it_is_resolved` now asserts the invariant for *every* name both resolvers accept. |

Two further findings were about honesty rather than behaviour, and both are corrected: the
`CLEANROOM.md` §5 entry for E8 **had not been filed** while the code claimed it had (§1.5 requires it
*before* the code — this is the second time that rule has slipped, and the entry says so), and
`LineScanner`'s comment about a failed partial match **misdescribed its own mechanism**.

**The reviewer also confirmed, by building an out-of-tree differential fuzzer, that the parallel and
serial indexes agree** — exhaustively over short UTF-8 and UTF-16 strings at every chunk size, thread
count and stride, plus randomised UTF-32. That is the strongest evidence yet for M2's headline
criterion, short of the 4 GB fixture.

---

## 🚧 E4 is done — the parallel indexer, 2026-08-05, session 10

`crates/tailhawk-core/src/indexer.rs`, plus a segment directory added to `index.rs`. **85 tests, all
passing**, fmt and clippy clean.

**Session 9's machine crashed mid-session.** Nothing was lost — the tree was clean and `6b76992` was
already pushed. Worth knowing only because "commit often; the history is the artefact" is what made
the crash a non-event.

| M2 criterion (`PLAN.md` §4) | State |
|---|---|
| **E3** — line index | **Done**, session 9. |
| **E4** — background + parallel indexer, code-unit alignment invariant | **Parallel: done.** `build_index` splits the file into code-unit-aligned chunks, one `LineScanner` per chunk on `std::thread::scope`, merged by prefix sum. **"Background": not done** — it is a synchronous call, see below. |
| **E8** — record model + OTel mapping + severity tables | **Done**, session 10 — see the E8 section above. |
| **Done:** index a 10 GB fixture with bounded memory | **Not run** — needs 10 GB of scratch disk. Peak indexing memory is now *asserted by construction* rather than hoped for: `read_bytes × threads` (2 MB at the defaults) plus the anchors, with **no per-chunk buffer of line starts**. |
| **Done:** 4 GB UTF-16LE on 8 threads is byte-identical to serial | **Run in miniature, not at 4 GB.** `a_real_file_indexes_the_same_on_many_threads_as_on_one` drives a real `LogFile` on all cores over a 20,000-line UTF-16LE fixture with a BOM and misaligned-`0x0A` decoys, and compares serial against parallel line by line *and* against the decoder. The property is tested; the scale is not. |

**CI is green on `87d1cae`** — run `31000029072`, 2m15s, all three jobs: `deny`, x64 and **ARM64**,
with both unsigned artefacts produced. `std::thread::scope` and the indexer cross-compile to ARM64
without complaint.

### `gh` is installed now — stop reading the Actions page in a browser

`winget install --id GitHub.cli`, done in session 10. Authenticated as `nigelbasel`, scopes
`gist`/`read:org`/`repo`.

**A session that has not been restarted since the install will not find it on `PATH`** — use the full
path, which needs the quotes:

```bash
"/c/Program Files/GitHub CLI/gh.exe" run list --limit 5
"/c/Program Files/GitHub CLI/gh.exe" run view <run-id>
```

Two notes for whoever sets this up again. `gh auth login` **exceeds a 120 s tool timeout and lands in
the background** — that is normal, not a hang: it prints a one-time code and then polls until the code
is entered at `github.com/login/device`. Nothing is written to `~/.config/gh` until that happens, so
`gh auth status` reports "not logged into any hosts" for the whole waiting period even though the
flow is working. Logging in to github.com in a browser is *not* the same step as submitting the code.

### The one design decision, and why it went the way it did

**A worker cannot know the global line number its chunk starts at** — that is settled only once every
earlier chunk has finished counting. So a worker anchors from *its own* first line, and the merge
records the base. `LineIndex` therefore grew a **segment directory**: one small entry per chunk,
binary-searched on lookup, and a serially built or followed index is a single segment where lookup
degenerates to the old `n / S`.

**The alternative was rejected on memory.** Workers could instead buffer every line start so the merge
picks the globally aligned ones, which would make the parallel index bit-identical to the serial one.
That costs 8 bytes per line of transient buffer — an 8 MB chunk of *empty lines* is 64 MB per worker,
8 workers is half a gigabyte, against §11.2's 120 MB whole-process claim. Empty lines are not exotic
input. `a_file_of_nothing_but_terminators_indexes_correctly` is the test that stands where that design
would have failed.

`SPEC.md` §5.3 already specified this in "Construction" ("emits anchors at its own local stride. A
prefix sum over the per-chunk line counts converts local anchor numbering to global") — the segment
directory is that sentence made concrete. §5.3's "The structure" has been extended to describe it, and
`CLEANROOM.md` §5 carries the entry §1.5 asks for.

### What "background" still means, and it is genuinely not done

`build_index` is **synchronous**: it blocks until the whole range is indexed. §5.3's **R5** — anchors
published as they are produced, the UI never blocking, the line count a lower bound until the scan
completes — **is not implemented**. Nothing consumes an index yet, so a progressive-publication API
built now would be built blind against a caller that arrives at M3. The pieces are shaped for it: the
merge is already a fold over independent per-chunk results in file order.

### Three tests worth knowing about

- `a_terminator_is_never_split_by_an_aligned_boundary` — the whole parallel scan rests on
  `line_terminator().len() == code_unit()` for every charset, which is what makes a code-unit-aligned
  boundary unable to fall *inside* a terminator. If an encoding ever arrives where that is false,
  chunked scanning needs a carry between workers and this is the test that says so.
- `a_misaligned_0a_survives_every_chunk_boundary` — E3's silent-corruption test, now driven at every
  legal chunk size rather than one.
- `a_real_file_indexes_the_same_on_many_threads_as_on_one` — the only test that exercises `LogFile` as
  a `ChunkReader`. Concurrent positional reads through one handle are a property of `LogFile`, not of
  the indexer, and no in-memory reader can test it.

**Three negative controls were run rather than assumed** — each mutation was applied, the failure
observed, and the mutation reverted:

| Mutation | Result |
|---|---|
| alignment guard disabled, odd chunk size in UTF-16LE | **2 lines found where there are 3** — silent corruption, the exact class `PLAN.md` marks E4 High risk. The guard is load-bearing. |
| first chunk stops owning the file's opening line | 7 of 11 fail |
| the trailing-terminator `pop_line` removed | 7 of 11 fail |

**One thing the negative controls exposed:** `many_threads_and_one_thread_agree_anchor_for_anchor`
**passed under two of the three mutations**. It is a *consistency* test, so it cannot catch a bug both
paths share. The correctness comes from `a_parallel_index_agrees_with_the_decoder_on_the_line_count`
and from resolving every line against an independent scan. Worth remembering before trusting a
serial-vs-parallel comparison to prove anything on its own.

### Deliberate omissions

- **R2 (byte offset → line number) is still not implemented.** `offset_of_line` covers R1; the inverse
  has no caller until "go to offset" and §5.5's re-anchoring after truncation.
- **`memchr` is still absent**, as in E3. The scan is a scalar loop.
- **Truncation and rotation remain M4.** A chunk read that returns zero stops rather than spins, and
  that is all the indexer owes until E7.

---

## 🚧 M2 started — E3 (the index structure) is done, 2026-08-04, session 9

`crates/tailhawk-core/src/index.rs`. **73 tests, all passing**, fmt and clippy clean.

| M2 criterion (`PLAN.md` §4) | State |
|---|---|
| **E3** — line index | **Done.** `LineIndex` (sparse anchors, blocked allocation) + `LineScanner` (terminator matching with cross-chunk carry), per the re-derived `SPEC.md` §5.3. |
| **E4** — background + parallel indexer, code-unit alignment invariant | **Not started.** The scanner is already the right shape for it: it holds no index and no I/O, so one per chunk shares nothing. |
| **E8** — record model + OTel mapping + severity tables | **Done**, session 10 — see the E8 section above. |
| **Done:** index a 10 GB fixture with bounded memory | **Not run** — needs 10 GB of scratch disk. The per-line cost *is* asserted (`the_index_costs_what_section_11_2_says_it_does`, < 0.14 B/line over 1 M lines), so the budget is tested even though the fixture is not. |
| **Done:** 4 GB UTF-16LE indexed on 8 threads is byte-identical to serial | **Not run** — needs E4. |

**Two deliberate omissions.** Truncation/rotation handling is **not** in `LineIndex` — that is M4
(§5.5, E7), and building it now would be speculative. `memchr` is **not** used: the scan is a scalar
loop, which is correct and simple; if the 10 GB criterion or M4's 50 MB/s needs it, optimise then,
with a measurement.

**Three tests worth knowing about:**

- `a_misaligned_0a_is_not_a_terminator_in_utf16` — the silent-corruption class `PLAN.md` marks E4
  **High** risk. `U+4A00` encodes LE as `00 4A`, so a scanner ignoring alignment finds a `0x0A` and
  splits the file in the wrong place. This is the test that will fail if someone "simplifies" the
  scanner to a `memchr` over raw bytes.
- `the_index_line_count_agrees_with_the_decoder` — the index scans *bytes*, the decoder decodes
  *characters*, and they must agree on how many lines exist. Different routes to the same number, so
  it is a real cross-check. Covers the trailing-terminator case where a line start at EOF is not a
  line.
- `line_starts_are_identical_however_the_bytes_are_chunked` — the E4 precondition. If a scan depends
  on where reads landed, a parallel indexer and a serial one disagree.

**`PLAN.md`'s E3 row was stale and is corrected** — it still described "block-sparse, group-varint,
u64 overflow fallback", which the re-derivation rejected.

---

## ✅ M2 is unblocked — the index was re-derived clean, 2026-08-04, session 9

**`SPEC.md` §5.3 has been replaced by a clean-room re-derivation, and the index is cleared to
implement.** `CLEANROOM.md` §4 now reads **RE-DERIVED CLEAN / Yes**, with the §5 entry and the §6
attestation both filed. **M2 can start.**

**The owner's decision was "re-derive clean, replace §5.3"** — not "log the risk and proceed", and
not the strictest quarantine option. Recorded because the cheaper option was genuinely available and
was declined.

**Why it was not the owner's to answer as originally framed, which was my error.** The four questions
below were put to the owner as provenance questions. He rightly refused them: he was never the
reader — agent sessions wrote `RESEARCH.md` and `SPEC.md`, so he had no basis to attest to anything.
Three of the four then turned out not to be decisions at all. `CLEANROOM.md` §2 already records the
provenance as "unknown and unreconstructable", which *is* the finding; unreconstructable provenance
means fail closed, and §2 already specified the remedy. Only the risk-appetite question was ever his.
**Lesson: do not escalate a question whose answer is already written down, and check who could
possibly know before asking anyone.**

### What the re-derivation actually changed — it did not reproduce the old design

| | Superseded | Re-derived |
|---|---|---|
| Structure | block-sparse, delta-encoded | **sparse anchors + forward scan**; blocking kept for *allocation*, not compression |
| Stride | 128 lines/block | **64 lines/anchor**, chosen against cold-read latency |
| Index size, 10 GB | ~56 MB | **~6.3 MB** |
| Delta encoding | yes | **rejected** — ~9x worse than sparse anchoring, and long lines (§10.3) escape in every delta scheme |

**The finding that drove it: memory is not the binding constraint.** Every sparse variant is
negligible against §11.2's 120 MB claim, so the stride is chosen on the *cold* forward-scan read —
6.3 KB at S=64 against 100 KB at S=1024 — not on size. Derived from the two corpora's measured mean
line lengths (116.8 and 84.2 bytes), which §3 permits and prefers.

**Neither §5.3 was read.** The replacement was spliced by line range (394–436, boundaries asserted
against the §5.3/§5.4 headings) so the superseded text never entered context. Two anchors *were*
disclosed rather than hidden — `block-sparse, 128/block` from §11.2, read before the problem was
recognised, and the word `delta-encoded` from a diff header — and the derivation argues against both
explicitly. Had it landed on 128 with delta encoding, that coincidence would have needed disclosing
instead.

### The original blocker, retained because the containment failure is the reusable lesson

`CLEANROOM.md` §2 named `docs/RESEARCH.md` §5.3 as
*the* contaminated section and requires the index to be re-derived from §3 sources before any code.
Session 2 logged that §5.3's technical content was **deliberately not read**, specifically to keep
this agent eligible to write the index under §1.4's "whoever reads, doesn't write".

**That containment is understated, and the design has propagated into `SPEC.md`:**

| Where | What |
|---|---|
| `SPEC.md` §5.3 "Line index" | Same section number *and* topic as the contaminated `RESEARCH.md` §5.3. **Not read this session.** Its provenance is unestablished. |
| `SPEC.md` §11.2, memory budget table | States **"Line index (block-sparse, 128/block)"** — a specific design constant, in a section carrying no contamination warning. **This was read**, while deliberately avoiding §5.3. |

§2 explicitly forbids quoting §5.3 "into a design document that then feeds implementation", which is
what appears to have happened. Whether `128` was derived independently or traces to GPL source is
exactly the thing nobody recorded — the original sin §2 describes, now one document further on.

**Why this cannot be resolved by just pressing on.** Rule §1.4 is irreversible: reading contaminated
content permanently disqualifies the reader from writing that component. Guessing wrong costs either
this agent's eligibility for M2 or puts unestablished provenance into the codebase, and neither is
recoverable after the fact.

**How the four questions resolved:**

1. ~~Is `SPEC.md` §5.3 a restatement, or independently derived?~~ — **moot.** Replaced rather than
   adjudicated. Unknowable either way, which was itself the answer.
2. ~~Does `128/block` have establishable provenance?~~ — **no, and it is gone.** The re-derivation
   landed on 64 for reasons it states.
3. ~~Widen §2's affected-section list?~~ — **done.** §2 now carries a propagation table naming both
   `SPEC.md` sections, and §4's register is updated.
4. ~~Is this agent still eligible?~~ — **yes, with disclosure.** Neither §5.3 was read; the two
   anchors that were seen are recorded in §5 and §6 and argued against in the derivation.

**The reusable lesson, now in `CLEANROOM.md` §2:** *a contamination notice attached to one section
does not travel with the content.* When a design is quoted onward, the warning stays behind — which
is exactly how `SPEC.md` came to carry the design with no warning on it. **Naming the component in
§4 catches this; naming the section does not.** Session 8 read and edited `SPEC.md` §5.3 (`571eb2e`)
without filing a §5 entry, and nothing flagged it at the time.

**⚠ What the attestation deliberately does not cover.** It speaks only for the 2026-08-04
derivation. If the superseded `SPEC.md` §5.3 *was* contaminated, session 8's exposure was real and
unlogged, and nothing here retroactively clears it. The component is clean going forward because the
design was **rebuilt**, not because the earlier exposure was ruled harmless.

---

## ✅ Dogfooded against both real corpora — 2026-08-04, session 9. The encoding gate passes.

Both corpora were opened with the M1 build and their reported figures checked against independently
computed ground truth. **Both are exactly right.** Paths stay in project memory, not here.

| | Tailhawk reported | Independently computed | |
|---|---|---|---|
| Corpus A | `UTF-8, 85,763 lines, 10,015,454 bytes` | 85,762 `LF` + an unterminated trailing line; CRLF throughout; zero non-ASCII | ✅ |
| Corpus B | `UTF-8, 109,855 lines, 9,246,865 bytes` | 109,855 `LF`, ends with `LF`, no trailing partial; strict UTF-8 decode succeeds | ✅ |

**The encoding regression this was built to catch did not fire.** Corpus B is now **9.2 MB of
BOM-less UTF-8** carrying **47,101 U+2014 em dashes and 643 U+2713 check marks** (143,232 non-ASCII
bytes, exactly 3 bytes each). It was reported as UTF-8. A CP1252 default would have mangled every
one of them, which is the failure `Get-Content` still exhibits on this file.

**The `+1` on corpus A is correct, not an off-by-one.** `Decoder::finish` deliberately flushes a
trailing unterminated line at end-of-stream, while `pump` holds it during streaming. Corpus B, which
ends with `LF`, shows no such adjustment — the two together are the evidence the rule is right.

**E1's writer-safety guarantee paid off against a real writer.** `[IO.File]::ReadAllBytes` **could
not open corpus A at all** — sharing violation, because the live writer holds it — while Tailhawk
read it without trouble. Until now that guarantee was only exercised by its own unit test and its
negative control.

### Both corpora have outgrown their description, and corpus A rotates hard

The figures in the Dogfooding section below were sampled 2026-07-29 and are **stale**:

| | as sampled | 2026-08-04 |
|---|---|---|
| Corpus A | ~450 KB, *idle* | **grew 2.7 → 10 MB in ~3 minutes, then rotated back to 1.2 MB while being watched** |
| Corpus B | ~3.3 MB | **9.2 MB** |

Corpus A is no longer an idle-file test — it is a **fast-growing, actively-rotating** one, and it
rotated under observation without prompting. That makes it a far better **M4** subject than the
handoff previously assumed, and it is the only rotation specimen available that is real rather than
synthetic. M1 does not follow, so Tailhawk simply read what existed at open time.

### Owner direction, session 9: keep the milestone order — no UI work brought forward

M1 has no visible surface and was never scoped to have one: `paint()` draws `BACKGROUND` and nothing
else, and the title bar is the only place the file result surfaces. Confirmed this session by
screenshot — every pixel of the client area is **RGB(18, 20, 23)**, exactly `background_rgb8()`, on
the hardware driver. A correctly-painted M1 window and a dead one are indistinguishable to the eye,
because the v1 palette is near-black.

Three options were put to the owner: keep the order, build a throwaway visible spike (~1 day,
DirectWrite straight to the window, no index), or genuinely reorder M3 ahead of M2. **The owner chose
to keep the order**, and added that **there is no need to dogfood again until real logs actually
render** — i.e. at M3. This is consistent with the session-7 decision to prefer robustness over
reaching a usable UI sooner, now taken a second time with the visible consequence in front of them.

**So: no further dogfooding is owed until M3.** What was owed — "do the corpora decode correctly
before the index and grid are built on top of them" — is discharged above.

---

## 🚀 M1 is most of the way done — 2026-07-31, session 8. It reads real files.

```
cargo run --release -p tailhawk -- C:\path\to\some.log
```

opens a window whose title reports what the core made of the file — encoding, line count, bytes. That
is M1's demo. It is headless work, so the title bar is the only surface it has until the grid arrives
at M3.

Verified end to end on the static build:

| File | Title reported |
|---|---|
| BOM-less UTF-8 with em dashes (Corpus B's shape) | `UTF-8, 5 lines, 171 bytes` |
| UTF-8 with BOM | `UTF-8, 2 lines, 107 bytes` |
| UTF-16LE with BOM | `UTF-16LE, 3 lines, 228 bytes` |

**56 tests, all passing.** `cargo test -p tailhawk-core`.

| M1 criterion (`PLAN.md` §4) | State |
|---|---|
| **E5** — encoding detection (BOM, NUL-parity, UTF-8 validation, chardetng) | **Done** — `crates/tailhawk-core/src/encoding.rs` |
| **E6** — incremental streaming decode, carry across boundaries | **Done** — `crates/tailhawk-core/src/lines.rs` |
| **E1** — source abstraction, open/share modes, writer-safety guarantee | **Done** — `crates/tailhawk-core/src/file.rs`, with the rotation-loop test *and* a negative control |
| **E2** — overlapped `ReadFile` layer **on IOCP** | **Half done.** Overlapped reads with explicit `OVERLAPPED.Offset` work and are tested. **The IOCP with 4–8 outstanding requests is not built** — reads are issued one at a time. That is a throughput property with no consumer until M4's 50 MB/s criterion, so it was left rather than built blind. |
| Encoding fixture matrix decodes correctly | **Done** — BOM'd ×5, BOM-less UTF-8/16LE/16BE/32LE/32BE, head-vs-tail mixed, truncated-mid-sequence, binary-embedded, DBCS |
| Rotation loop shows no sharing violation on the writer side | **Done** — and paired with a negative control that opens `FILE_SHARE_READ` only and asserts the writer *is* locked out |
| Fuzz targets clean | **Done, session 9** — `arbitrary_bytes_decode_the_same_however_they_are_chunked` in `crates/tailhawk-core/src/lines.rs`. |

### ✅ M1 is complete — the fuzz criterion closed, session 9

**Option 1 was taken: an ordinary `#[test]`, not `cargo-fuzz`.** The deciding argument was not the
MSVC awkwardness the previous session flagged but **ARM64** — libFuzzer is effectively absent on
Windows ARM64 and CI builds both architectures, so `cargo-fuzz` would have covered one leg of the
matrix at best. The test runs in the existing job on both, with no new tooling.

**It needs no dependency at all.** A ten-line xorshift64* in the test module replaced `arbitrary`,
which keeps the `deny.toml` allow-list out of the decision entirely. A fuzzer needs a *reproducible
seed* far more than a good distribution, and a fixed iteration count keeps CI deterministic where a
wall-clock budget would not. `TAILHAWK_FUZZ_ITERS` raises it for a local soak.

**Most bytes are drawn from an interesting alphabet, not uniformly.** Uniform random bytes almost
never produce a `CRLF` pair, a valid multi-byte sequence or a BOM — so a uniform fuzzer explores none
of the states that actually carry across chunk boundaries, which is the whole point.

Three properties, none of which may depend on framing: no input panics; no emitted line contains
`\r` or `\n`; and the lines are identical however the same bytes are chunked, **including invalid
ones** — replacement characters are output too, and must land in the same places.

**Soaked at 40,000 iterations × 11 charsets × 7 framings — about 3.1 million decode runs, clean**, in
17 s release. That includes the two cases most likely to break the property: `ISO-2022-JP`, which is
stateful, and UTF-32, where a truncated unit at EOF was one of the three original bugs. CI runs 400
iterations (~1.9 s).

**59 tests, all passing.** `fmt` and `clippy` clean on the product crates.

The pre-existing boundary-invariance tests feed *fixed* inputs at every possible split; this feeds
*arbitrary content* at several splits. They are complementary, and the older ones remain the source
of the three real bugs found so far.

### Choices worth not re-litigating

- **UTF-32 is hand-written, and it is not gold-plating.** The WHATWG Encoding Standard excludes UTF-16
  and UTF-32 *detection* and excludes UTF-32 entirely, so `encoding_rs` cannot be delegated to for the
  parts §5.6 cares most about on Windows. `Charset` is therefore an enum wrapping `&'static Encoding`
  plus two UTF-32 variants, not a bare `&'static Encoding`.
- **`Charset::code_unit()` and `Charset::allows_parallel_indexing()` exist now, in M1.** §5.3 requires
  encoding to be resolved before *chunk assignment*, not merely before indexing. Putting the types M2
  needs into M1 is what makes "decode before index" structural rather than a note in a plan.
- **`detect` takes the codepage fallback as a parameter.** The core may not name a Win32 function
  (§3.1), so `system_codepage()` lives behind `cfg(windows)` and is passed in.
- **Line emission is a callback**, not an iterator. A line that spans chunks lives in the decoder's
  `pending` buffer and one that does not is borrowed from the scratch buffer; a callback serves both
  without allocating per line.
- **`FileSource::pump` holds a trailing partial line rather than emitting it.** A writer that has
  flushed half a line is the normal state of a live log.
- **Following, polling and rotation *handling* are absent on purpose.** They are M4. What M1 owes is
  that the bytes arrive correctly and the writer is never impeded.

### ✅ `SPEC.md` §5.3's DBCS exception is withdrawn — owner-approved, session 8

The spec disabled parallel indexing for codepages 932/936/950/949 on the grounds that `0x0A` is a
legal trail byte in them. **It is not**, and the property turned out to be far more general than the
DBCS carve-out it was raised against — so the exception was **deleted rather than narrowed**.

Two exhaustive tests in `crates/tailhawk-core/src/encoding.rs`, over eight multi-byte encodings and
ten single-byte representatives:

| | What it drives | Result |
|---|---|---|
| `a_0a_byte_is_always_a_terminator_in_every_byte_oriented_encoding` | every code point through every encoder | no character but U+000A produces a `0x0A`; U+000A always does |
| `a_0a_byte_is_never_consumed_as_a_trail_byte_by_any_decoder` | all 65,536 two-byte prefixes into every decoder, followed by `0x0A` | the newline survives every time |

**The second one is the test that matters, and the first version of this work did not have it.** A
scan reads arbitrary bytes off disk, not bytes some encoder produced, so the question is whether a
`0x0A` can be *consumed* as a trail byte — not whether one can be *written* there. Decoders accept
sequences encoders never emit. Measuring only the encoder side would have been a plausible,
confident, and insufficient basis for a normative change.

**What replaced it.** `allows_parallel_indexing()` is gone: it could only return `true`, and
`code_unit()` now carries the whole rule. In its place is `is_random_access_decodable()`, which is
false for **`ISO-2022-JP` alone** — escape-driven, so a line start is a *character* boundary but not
a *decoder* boundary. That constrains **viewport decode only, never the scan**, and it is the last
of what §5.3 used to claim.

### Size: the binary doubled, and it is still 3.2% of the gate

| | static `tailhawk.exe` |
|---|---|
| M0, session 7 | 249,344 |
| M1, with `encoding_rs` + `chardetng` linked | **502,784** |
| CI gate (`SPEC.md` §11.2) | 15,728,640 |

Worth recording rather than acting on. Note that the figure only moved once the **shell** referenced
the core's new code — before that LTO stripped both crates entirely and the exe was byte-identical to
M0's, which is a misleading way to measure a dependency.

---

## ✅ CI is green, and the ARM64 unknown is closed — 2026-07-31, session 8

Session 7 left this as the first thing to check. It has run twice and both runs succeeded.
**The ARM64 leg linked** — run #2 (`dbe45b6`) produced `tailhawk-arm64-unsigned` alongside the x64
artefact. The ⚠ is deleted from the M0 table below. ~~There is still no `gh` CLI on this machine; the
Actions page was read through the browser.~~ — **`gh` is installed as of session 10**, see below.

**Run #3 carried all of M1's code and passed on both architectures** — clippy, `fmt`, 56 tests, the
size gate, and both binary-content assertions (no `d3dcompiler`, no CRT redistributable).
`encoding_rs` and `chardetng` cross-compile to ARM64 without complaint.

**`cargo-deny` now runs, and it caught its own config on the first try.** The job was added in session
8 and failed immediately — not on a dependency, but because the config was named `cargo-deny.toml`,
which `cargo deny` does not look for. Renamed to `deny.toml`; the job is green and the allow-list is
now genuinely enforced rather than described. Both halves of that are in the traps table.

The only warning on any run is the Node 20 deprecation notice on `actions/checkout`, `actions/cache`
and `actions/upload-artifact`. Cosmetic, and it will resolve itself when those actions bump.

---

## 🚀 M0 is done — 2026-07-30, session 7. There is a running application.

`cargo run --release -p tailhawk` opens a window. That is the command to use whenever you want to
start something up and look at it; it will show progressively more as M1–M3 land.

**Owner direction, session 7:** keep to the planned milestone order — **M2 is not deferred**. This is
a side project, it does not need to be in daily use, and **robustness is preferred over reaching a
usable UI sooner**. The dogfood-first reordering (skip M2, trim M3) was offered and declined.

| M0 criterion (`PLAN.md` §4) | State |
|---|---|
| Cargo workspace with the core/shell split from commit one | **Done** — `crates/tailhawk-core` (portable, owns rendering) + `crates/tailhawk` (window, message loop) |
| A window that opens | **Done** — verified opening, responding, titled with the driver it got |
| D3D11 device with the WARP fallback chain | **Done** — hardware → WARP; comes up on hardware here |
| An embedded shader | **Done** — `fxc` at build time via `build.rs`, DXBC embedded, CI asserts no `d3dcompiler` import |
| `+crt-static` | **Done** — 249,344 bytes, imports **only OS DLLs**; the dynamic build needs `vcruntime140.dll`, the static one does not |
| CI producing x64 and ARM64 under the size gate | **Done and verified, session 8.** Both legs succeeded on the runner; the ARM64 artefact exists. This machine's VS install has no ARM64 linker, so CI is the only place that leg is provable — and it proved. |
| **Done:** opens on a clean Windows 10 1809 VM with no runtime installed | **Not literally tested** — no such VM here. The dependency surface that criterion is really about *is* verified: only OS DLLs. |

**Choices worth not re-litigating:**

- **The core cannot name an `HWND`.** The drawable crosses the seam as an opaque `WindowHandle(isize)`,
  so `SPEC.md` §3.1's portability rule is enforced by the type rather than by discipline.
- **Leaf backends are modules inside the core, not separate crates.** Splitting them buys nothing
  until a second platform exists, and it is a mechanical move when one does.
- **Both paint stages take their colour from one constant** in the core, with a unit test asserting
  the `f32` and 8-bit forms agree. They are necessarily written twice — GDI wants bytes, the render
  target wants floats — and drift would show as a flash on every cold start.
- **Stage one is the class background brush**, which the system draws during `ShowWindow` before any
  handler runs. §3.2 measured a brush as *equivalent* to a `FillRect`, so this is the simpler of two
  equal options.
- **M0 draws through the shader rather than `ClearRenderTargetView`.** Same pixels, more cost — the
  point is that the offline-compile path M3 depends on is proven now rather than then.
- **`clippy` and `fmt` in CI are scoped to the product crates.** The experiment crates trip four
  pre-existing lints; they are throwaway measurement code whose hand-formatting is part of what they
  record. They are still compiled by the workspace build, so they cannot rot into not building.

~~**Start the next session by checking CI and then starting M1**~~ — **both done, session 8.** CI is
green including ARM64; M1 is most of the way through. See the two sections above.

---

## ✅ Batched rasterisation is done — 2026-07-30, session 7

The highest-value remaining experiment ran, as `experiments/g4b-batched-raster`. Full write-up in
`experiments/g4b-batched-raster/RESULTS.md`. Machine quiet throughout (2–5% CPU, zero leaked
subjects). It answered its own question and then overturned the premise behind it.

**Batching works and is not the win.** Batched cells are **bit-identical** to per-glyph cells at any
inter-cell gap including zero, so it is safe for an atlas. It is worth **~1.8x on cold glyphs**,
saturating at **4 glyphs per analysis** — real, worth taking, but not the order of magnitude that made
it the top-ranked item. Past 256 glyphs per analysis it becomes a *pessimisation*. On warm glyphs it
wins nothing.

**What actually dominates: a cross-process, capacity-limited system font cache.**

| | µs/glyph |
|---|---|
| first process on the machine to rasterise a glyph | **86 – 108** |
| any later process, same glyph and size | **~3** |
| capacity | **8,000 – 16,000 distinct glyphs** (bracketed, not pinned) |

36x, and it survives process exit — so it is system state, not process state.
`Windows Font Cache Service` is the inferred mechanism (not verified; confirming it means stopping a
system service).

**Three earlier conclusions are corrected.**

1. **G4's 330–388 µs/glyph is a cache-thrash figure.** Its fixture cycles 20,992 distinct CJK glyphs
   to force *atlas* eviction — which was the point — and incidentally never fits the font cache
   either. Re-run today the same binary gives **92–97 µs/glyph**. The eviction conclusions (the O(1)
   LRU requirement, the 20–28x ratio) are untouched and stand.
2. **Session 6's anomaly is explained, and its mechanism was wrong.** Session 6 saw the quiet
   post-reboot re-take land at the *top* of the range and concluded "a cold run does not bound this
   cost favourably — the spread is not load". Right observation, wrong cause: **a reboot empties the
   font cache**, so that run was the most cache-cold and therefore the slowest. It was never about CPU
   load in either direction.
3. **`SPEC.md` §3.2's "rasterisation off the paint path" survives but needs re-scoping — owner's
   call.** It is a **first-run** cost, not a per-viewport one: a genuinely cold 1,500-glyph viewport is
   **162.5 ms** (8–10 frames, so placeholders stay a v1 requirement), but steady state is the same
   viewport in **4.4 ms**, inside one frame. The spec currently implies the former is permanent. Not
   edited — that is a normative change.

**Method note that cost the session an hour and a wrong answer:** the first version of this experiment
reported 2.5 µs/glyph and concluded G4 was wrong by 150x. It was measuring cache hits left behind by
its own previous run. An in-process "is there a cache?" probe cannot detect a cross-process cache and
returned a confident 0.99x. See the traps table.

---

## ✅ The cold set is taken — 2026-07-30, session 6

The reboot happened and the set was taken in the first ten minutes, on the existing
`target-verify-static\release\` binaries with no rebuild. **Six 11-run sets: D2D, D3D11 serial and
D3D11 earlypaint, each in two conditions** (boot churn at 36% CPU, then quiet at 0–6% CPU), plus a
quiet G4. Leaked-subject count verified at **0** before and after every set. Full write-ups are in
`experiments/g3-d3d11/RESULTS.md`, `experiments/g3-d2d/RESULTS.md` and
`experiments/g4-glyph-atlas/RESULTS.md`.

**The branch that held: background load explains the whole absolute spread.** Every quiet figure lands
at or below the fast end of session 5's range. So G5's reference machine is *not* needed to settle the
shape of the result — it is still needed before any number is published in `SPEC.md` §11.3.

| | session 5 (loaded) | quiet, session 6 |
|---|---|---|
| **earlypaint first pixel** | 54.7 / 66.3 ms p50 | **13.1 ms p50, 14.5 p90** |
| D3D11 serial, total | 126.4 ms p50 | 68.6 ms p50 |
| D2D, total | 156.7 ms p50 | 75.5 ms p50 |
| `CreateWindowExW` | 8.5 – 11.9 ms | 3.2 – 3.9 ms |

**Three conclusions changed.**

1. **The "~50–60 ms window-presentation floor" does not exist — it was load.** `first_pixel` measures
   `main()` entry → `FillRect` returning in the first `WM_PAINT`, so `ShowWindow` and paint dispatch are
   inside the measured region: they cost **~10 ms beyond window creation**, not 50–60. Session 5's
   "the window is the bottleneck" reframing is **withdrawn**.
2. **G3's 40 ms first-pixel criterion passes with the two-stage paint** — 13.1 ms p50, 14.5 ms p90,
   14.5 ms worst of 11 — and fails ~1.7x without it (68.6 ms). **The criterion is a test of paint order,
   not of the graphics stack**, and `SPEC.md` §3.2's two-stage requirement is now measured as
   sufficient rather than merely helpful. Still one machine, not the G5 reference machine, and
   time-to-`FillRect` rather than time-to-photon.
3. **G4 went the other way and this is the important one.** Rasterisation on the quiet machine is
   **494–582 ms/frame — the *top* of the earlier 227–582 range**, i.e. **330–388 µs/glyph**. "The spread
   tracks load" was wrong, a cold run does not bound the cost favourably, and the pessimistic end is the
   honest figure. **This raises the value of the batched-rasterisation experiment below**, which is now
   unambiguously the highest-value remaining item.

**Two method notes worth keeping:**

- **Post-reboot is not quiet, and uptime is the wrong thing to record.** The first four runs of the
  first set gave 174, 225, 264 and **1604 ms** before settling — boot-time service churn is itself load.
  The quiet window opened at about six minutes' uptime. **Record CPU load, and re-check it after every
  set.**
- **`-OutFile` must match the filename the binary hardcodes** (`g3-d2d-first-pixel.txt`,
  `g3-d3d11-<mode>.txt`). Any other name and `measure.ps1` polls for a file nobody writes, warns eleven
  times and throws — a whole set measured nothing before this was spotted.
- **G4 is `windows_subsystem = "windows"`, so PowerShell does not wait for it.** `& .\g4-glyph-atlas.exe`
  returns in 0.1 s while the process runs on for ~100 s. Poll for `%TEMP%\g4-glyph-atlas.txt`, then kill
  the process — it holds a D3D device and is otherwise a leaked subject by the same mechanism as G3's.

---

## Directory rename — done, but as `TailHawk`. Settle the capital H or leave it alone.

The rename happened. The working directory is **`C:\dev\git\TailHawk`** — **capital H**, where the
plan said lowercase. Everything works: the project key is `C--dev-git-TailHawk`, memory lives there
and loads, and session 8 ran entirely from it.

**It is only cosmetic, and changing it is not free.** The project key is the literal path string, so
another rename orphans the memory again and needs the same copy step. The remote
(`github.com/nigelbasel/tailhawk`) and every document already say lowercase, so the directory is the
only place the capital H appears — and nothing reads the directory name.

**Recommendation: leave it.** If it is ever changed anyway, the procedure is unchanged from before —
it cannot be done from inside a session, because the harness resets the shell CWD into the repo after
every tool call and Windows will not rename a directory a process has as its CWD:

```powershell
cd C:\dev\git
Rename-Item TailHawk Tailhawk
Copy-Item -Recurse -Force "$env:USERPROFILE\.claude\projects\C--dev-git-TailHawk\*" `
                          "$env:USERPROFILE\.claude\projects\C--dev-git-Tailhawk"
```

**Decide once.** Every change of spelling — even just capitalisation — orphans the memory again.

---

## Where things stand

The project is **Tailhawk** (command `tailhawk`) — a Windows desktop log tailer/viewer.
Research, specification, UI design and development plan are complete and adversarially reviewed.
**Phase 0 is effectively closed, M0 is done, and M1 is most of the way through — the application
opens real log files and decodes them correctly, now verified against both real corpora (session 9)
rather than fixtures alone.** Nothing left in Phase 0 is both runnable and
blocking: G2 is informational, G3's `eframe` legs are moot, and G1/G5/G6 are owner-gated. Five
experiments were built and written up (G3 ×2, G4, G4b, G7).

Repo: **`github.com/nigelbasel/tailhawk`, private. CI is green on both x64 and ARM64.** Working
directly on `master` — no branches, no PRs (tried once, not worth it solo). Commit often; the history
is the artefact.

### The documents

| File | State |
|---|---|
| `docs/RESEARCH.md` | Complete. §11 records every claim critics refuted. §12 lists the gating experiments. **§5.3 is GPL-contaminated — see `CLEANROOM.md`.** |
| `docs/SPEC.md` | Complete. §16 traces both review rounds. §17 lists open decisions. |
| `docs/UI-DESIGN.md` | Complete. Phase-tagged `[v1]`/`[v2]`/`[v3]` throughout — **`SPEC.md` §15 is authoritative on phasing, not this document.** |
| `docs/PLAN.md` | Complete. v1 = 81.5 person-weeks — **the owner considers the estimates extremely conservative and does not treat them as a schedule.** Gates are things to answer, not a timeline. |
| `docs/LOKI.md` | Complete. Loki-as-a-source design, adversarially reviewed. Decisions settled; not yet folded into `SPEC.md`/`PLAN.md`. |
| `CLEANROOM.md` | Live. Provenance rule, source allow-list, component register, append-only log. |

### Repo furniture in place

`LICENSE-MIT`, `LICENSE-APACHE` (dual, copyright asserted personally), `deny.toml`
(allow-list, so GPL/AGPL/LGPL fail without being named — rationale in `CLEANROOM.md` §7),
`.gitignore`, `rust-toolchain.toml`, workspace `Cargo.toml`, tracked `Cargo.lock`.

### Decisions locked in

- **Name: Tailhawk.** Chosen for the "watch like a hawk" idiom. Tagline: *watch your logs like a hawk*.
- **Stack:** Rust + windows-rs + D3D11 + DirectWrite. Shared core owns the whole grid *including rendering*; thin shell owns window/input/IME. Per-platform leaf backends for rasterisation and presentation.
- **Windows-only v1.** Cross-platform stays possible via the leaf-backend seam but is unpromised.
- **No memory mapping**, ever — a section handle blocks the writer's log rotation.
- **Polling is the correctness mechanism** for following; change notification is an accelerator that may fail silently.
- **Record model is OTel-shaped**, extended with `raw`/`format_id`/`parse_state`.
- **Merged timeline + trace correlation stay v2** (owner-confirmed).
- **Personal project**, personal GitHub, no employer identity anywhere in the repo.
- **Licence: MIT OR Apache-2.0.**
- **Loki is a client, not a viewer** (2026-07-29). The near-free `logcli | tailhawk -` path is
  rejected — depending on another CLI binary contradicts the self-contained copy-and-run promise.
  Staged client-lite → full client. Cost is not a constraint on this project.
- **v2 order:** §8.3 merged view (local only) → Loki client-lite → §9 trace correlation → Loki
  stages 2–3. The merged view goes first because it is entirely offline and keeps §13.2's
  "no sockets, ever" CI assertion at full strength until the last possible moment — a one-way door.

---

## What is worth doing next, as of session 5

Phase 0 is **4 of 7 done or dispositioned**. What remains, and an honest read on each:

0. ~~**Batched glyph rasterisation — the highest-value unblocked experiment**~~ — **done, session 7.**
   See the section at the top and `experiments/g4b-batched-raster/RESULTS.md`. Batching is
   bit-identical and worth ~1.8x saturating at batch 4; the order of magnitude is not there. The
   premise — that per-analysis granularity was the dominant cost — is refuted: a cross-process font
   cache is, and G4's figure was a thrash figure. **Nothing further is owed on this line of work.**
   What it leaves behind is one decision for the owner: whether to re-scope `SPEC.md` §3.2 from a
   permanent requirement to a first-run one.
1. **G2 — read throughput.** Unblocked and doable solo, but **it blocks nothing**: `PLAN.md` §3 marks it
   *"Informational — no pass threshold"* and its "if it fails" column reads *"Nothing."* The no-mmap
   decision is already locked on correctness grounds, so G2 only quantifies what that costs. Note it
   wants **10 GB of scratch disk**, and its cold half needs a reboot or an elevated standby-list purge —
   so it is realistically warm-only unless run right after a restart.
2. **G3's `eframe` legs — argue for skipping them.** G3 exists to *compare* three stacks, but the
   stack decision is already locked in (windows-rs + D3D11 + DirectWrite), `RESEARCH.md` §3.3 already
   rejects egui/eframe for the grid on text-AA grounds, and **G7 has now independently confirmed
   egui's scroll model breaks at exactly the row counts Tailhawk targets.** Building two eframe
   hello-worlds would measure binary size for a stack that is triply rejected. The honest move is to
   record the legs as **deliberately not run**, with that reasoning, rather than leave them looking
   outstanding. Owner's call.
3. ~~**Re-take G3's numbers** once the desktop C++ workload is installed~~ — **fully done.** Sizes
   (unchanged, byte-identical), the A/B comparisons, and now the **cold and quiet sets** (session 6,
   above). Nothing further is owed on this machine; the remaining absolute-figure work is G5's reference
   machine, which is open question 3.
4. ~~**Test the one surviving first-paint direction:** paint something cheap before the D3D device
   exists~~ — **done and it works.** It is `earlypaint` in `experiments/g3-d3d11`, it reaches first pixel
   in **13.1 ms p50** on a quiet machine, and it is what makes G3's 40 ms criterion pass. `SPEC.md` §3.2
   requires it for v1.
5. **G1** needs two hosts, **G5** needs a decision on downloading third-party binaries, **G6** is a
   week of the owner's real usage. All three are owner-gated, not work-gated.

~~**A free by-product worth collecting:** adding G3-style first-pixel instrumentation to the D3D11 +
DXGI path~~ — **done**, as `experiments/g3-d3d11`, and it paid for itself twice: it refuted the
worker-thread fix and showed the specified stack is ~30 ms faster than the D2D one G3 measured.

## Resume here tomorrow

### 1. ~~Finish the Loki source design~~ — **done 2026-07-29, see `docs/LOKI.md`**

The workflow was re-run and completed (4 agents, ~30 min). Results are written up in
**`docs/LOKI.md`** and are **not** yet folded into `SPEC.md` or `PLAN.md`, deliberately.

Headlines:
- **The architectural crux resolved cleanly.** A Loki source materialises pages into the §4.2 stdin
  spill as CLEF NDJSON; the grid, index, filters, search, sort and export run unchanged. The
  viewport does **not** become cursor-based.
- **One thing must land in v1** even though no Loki code ships in v1: `ExtentState` and an opaque
  `RowId(u64)` (§4 of `LOKI.md`). ~0.5 PW now against a ~3 PW rewrite after the grid ships.
- **Three research claims died under re-verification** — the binary-size objection to an HTTP stack
  (measured at +1.68 MB against a 15 MB gate), the choice of `rustls` (pulls a C toolchain via
  `ring`, contradicting §5.3), and the headline latency figure (an artifact of a pathological
  selector; a tight selector is 60–130x faster).
- **The credential design as researched is unsound** — keying by source name is an exfiltration
  primitive, and "DPAPI or Credential Manager" is not a design.
- **Decided 2026-07-29: Tailhawk is a Loki client, staged client-lite first.** The near-free
  `logcli … | tailhawk -` viewer path is **rejected** — a dependency on another CLI binary
  contradicts the self-contained copy-and-run promise. Cost is not a constraint on this project, so
  the "86–110% of the v2 budget" objection does not bind. The cost reconciliation is still owed to
  `PLAN.md` §2.3b, but it no longer gates proceeding. See `LOKI.md` §8 for the three stages.
- **Order within v2:** §8.3 merged view (local only) → Loki client-lite → §9 trace correlation →
  Loki stages 2–3. The merged view goes first because it is entirely offline and keeps §13.2's
  "no sockets, ever" CI assertion at full strength until the last possible moment — that weakening
  is a one-way door. Reasoning in `LOKI.md` §8.

### 2. Then, in order

**Owner direction, 2026-07-29: get the whole app running locally, put it in a GitHub repo, publish
later.** A **private** GitHub repo is not publishing, so it is compatible with everything below.
Everything genuinely publication-shaped is deferred, and the queue is reordered around that.

Three things follow from pushing to GitHub at all, private or not:

- **Git history is permanent.** Anything committed to a private repo is still in the history when
  that repo later goes public. The scrubbing rules — no employer identity, no customer names, no
  dogfood paths or sample lines (see below) — apply from the **first commit**, not from the publish
  date. There is no later cleanup that is not a history rewrite.
- **Branching starts.** The standing "commit straight to `master`" rule was explicitly scoped to
  "until there is a remote". Once the GitHub remote exists, branch-per-change and PRs apply.
- **Open question 2 (employment IP) gets closer.** It is easier to establish a position before code
  exists in a hosted repo than after. Still the owner's to resolve.

1. ~~**Populate `CLEANROOM.md`**~~ — **done 2026-07-29.** `CLEANROOM.md` is at the repo root. It
   records the rule, the allow-list, the component register and an append-only consultation log.
   The block-sparse line-offset index is registered **CONTAMINATED and not cleared to implement** —
   it needs a re-derivation entry plus an attestation in `CLEANROOM.md` §6 **before its first line
   of code**.
2. ~~**Add `LICENSE-MIT`, `LICENSE-APACHE`, `deny.toml`**~~ — **done 2026-07-29.** All three
   are at the repo root. `deny.toml` is an allow-list, so GPL/AGPL/LGPL fail without being
   named and an unreviewed licence fails closed; the reasoning is in `CLEANROOM.md` §7. (`git init`,
   the first commit and the push to `github.com/nigelbasel/tailhawk` — private — are also done.)
3. **Run Phase 0** (`PLAN.md` §3) — **in progress.** See the next section.

**Working agreement:** commit directly to `master`, no branches, no PRs — tried once, not worth it
for a solo repo. Commit often; the history is the artefact.

---

## Toolchain — resolved, 2026-07-30

**The desktop C++ workload is installed and `.cargo/config.toml` is deleted.** The whole OneCore
workaround is gone: toolset `14.51.36231` now has `lib\x64`, `lib\x86` and `include`, the version did
not bump, and `cargo build --release` links with no `LIB` override. If a future `LNK1104` appears,
that file no longer exists to be the suspect.

**Two things learned during the transition, worth keeping:**

- **Do not build or measure while a Visual Studio installer is resident.** A `+crt-static` link failed
  once with `LNK1104: cannot open file 'libucrt.lib'` for two crates while succeeding for the other
  two, then succeeded for all four on an immediate retry — files moving underneath the linker. Every
  session-5 timing was taken with `setup.exe` resident, which is why they are provisional.
- **⚠ A measurement subject that exits on its own is never reaped, and it corrupts the numbers.** This
  is the single most expensive thing session 5 learned. The dead process keeps its D3D device;
  `tasklist` lists it, `taskkill` says *"no running instance"*, `cargo build` fails with
  `Access is denied (os error 5)`, and — the part that actually costs you —
  **`D3D11CreateDevice` degraded from 55 ms to over 1200 ms as 49 of them piled up**, silently, with no
  error. It invalidated a whole round of G3 conclusions before it was spotted.
  **The fix is in the code: experiment binaries must not `PostQuitMessage` after reporting.** They
  report and wait to be killed by `measure.ps1`. `g3-d2d` did this by accident and leaked zero across
  22 runs; `g3-d3d11` self-terminated and leaked one per run until it was changed.
  `$p.Dispose()` in PowerShell does **not** help. If a measurement looks inexplicably slow, count the
  leaked processes before believing it. If some are already resident, they can only be cleared by
  killing the holder or rebooting — and meanwhile build into a scratch dir
  (`$env:CARGO_TARGET_DIR="target-verify"`, git-ignored) or verify with `cargo check --workspace`.

### Re-take procedure

Still owed: a **post-reboot (cold)** set and a **quiet-machine (warm)** set, because of the
reproducibility problem below. Sizes are done and did not move.

```powershell
cargo build --release            # or $env:RUSTFLAGS="-C target-feature=+crt-static"
.\experiments\measure.ps1 -Exe target\release\g3-d2d.exe   -OutFile g3-d2d-first-pixel.txt `
    -Columns "factory,window,target,draw,total"
.\experiments\measure.ps1 -Exe target\release\g3-d3d11.exe -OutFile g3-d3d11-serial.txt `
    -ExeArgs serial     -Columns "mode,window,device,swapchain,draw,total,driver"
.\experiments\measure.ps1 -Exe target\release\g3-d3d11.exe -OutFile g3-d3d11-concurrent.txt `
    -ExeArgs concurrent -Columns "mode,window,device_wait,swapchain,draw,total,driver"
```

**Quote `-Columns`** — PowerShell parses a bare comma-separated list as an array and the script
rejects it. Then re-run G4 (`cargo run --release -p g4-glyph-atlas`, ~100 s) and drop its OneCore
caveat.

---

## Phase 0 — where it got to

| Gate | State |
|---|---|
| **G3** — binary size floor + first pixel | **Size passes and is settled. First pixel now passes too — with the two-stage paint (13.1 ms p50 against 40 ms); it fails ~1.7x without it.** Absolutes settled as far as one machine allows; publishing them still waits on G5. Two legs done on the desktop CRT: D2D (`experiments/g3-d2d/RESULTS.md`) and D3D11+DXGI (`experiments/g3-d3d11/RESULTS.md`). `eframe` legs not started — see the argument below that they are moot. |
| **G1** — SMB stale size | Not started. Needs two hosts and a share; can't be done solo on one machine. |
| **G2** — read throughput | Not started. Informational only, no pass threshold. |
| **G4** — colour-glyph atlas | **Done. Passes**, and it refuted the objection it was built to test. See `experiments/g4-glyph-atlas/RESULTS.md`. Its rasterisation *absolutes* are superseded by G4b (session 7) as cache-thrash figures; its atlas and eviction conclusions stand. |
| **G4b** — batched rasterisation | **Done, session 7.** Not a `PLAN.md` gate — a follow-up closing G4's open caveat. Batching is bit-identical and worth ~1.8x; the dominant cost is a cross-process font cache. See `experiments/g4b-batched-raster/RESULTS.md`. |
| **G5** — incumbent re-measurement | Not started. Needs BareTail 3.50a and LogExpert 1.41.0 installed — **owner decision, involves downloading third-party binaries.** |
| **G6** — Hoo WinTail hands-on | Owner task, runs in the background across Phase 0. |
| **G7** — reproduce egui #1391 | **Done. Passes** — the cause is identified. See `experiments/g7-egui-scroll/RESULTS.md`. |

### G3 result in one line

**Size passes by ~8x. First pixel passes by ~3x if something paints before the device exists, and fails
by ~1.7x if you wait for it — so the gate measures paint order, not the graphics stack.**

The paragraphs below were written before session 6's quiet set and quote the loaded absolutes (126 ms
total, 117 ms graphics init, a 50–60 ms window-presentation floor). **The ratios and the A/B conclusions
all stand; the absolute numbers are ~1.8x too high and the window-presentation floor is refuted.**
Corrected figures are in the session-6 section at the top and in `experiments/g3-d3d11/RESULTS.md`.

**Size is done.** 243,712 bytes with `+crt-static`, 146,432 dynamic, against a 2 MB criterion — and
**byte-for-byte identical between the OneCore and desktop CRTs**, so the re-take changed nothing. The
15 MB CI gate has vastly more headroom than assumed. Static costs a flat +97,280 bytes.

**First pixel fails, and the shape of the failure changed completely.** On the specified stack
(D3D11 + DXGI, `+crt-static`, desktop CRT) it is **126 ms** against a 40 ms criterion. The breakdown
is the finding: **drawing is 2.6 ms** and **graphics initialisation is 117 ms, or 92% of the total** —
`D3D11CreateDevice` 60 ms plus swapchain and RTV creation 57 ms. So the conclusion *"graphics device
creation must come off the critical path"* is **strengthened**.

**The fix G3 proposed works, but only just.** On a **clean machine**, 10 **paired interleaved** trials:
concurrent device creation totals **145.7 ms** against serial's **154.2 ms** and wins **7 of 10** — a
**~8.5 ms, 5.5% saving.** Real, reproducible, and small, exactly what `min(window, device)` allows when
window creation is only ~10–25 ms. Worth taking; nowhere near enough on its own.

Under **GPU-context pressure** it matters far more: with ~35–49 leaked D3D devices resident, serial
device creation degraded to a **1155 ms** median while concurrent held at **135 ms**, winning 8/8. The
likely mechanism — `D3D11CreateDevice` stalling on the `HWND`-owning thread when the driver wants it to
pump messages — is unconfirmed, but it argues for off-thread device creation as cheap insurance on
loaded machines, which is where a log viewer lives.

**The bigger lever is measured and it works: paint before the device exists.** A GDI fill in the first
`WM_PAINT`, with D3D coming up on a worker, cuts first pixel to **48–54% of the naive order** — 12 of 12
paired trials, reproduced across two runs. `SPEC.md` §3.2 now requires this two-stage paint for v1.

Two results worth not re-litigating:
- **A class background brush is equivalent, not better** (4 of 12 pairs). The mechanism does not
  matter; only that *something* paints without waiting for D3D.
- **The residual ~50–60 ms floor is window presentation, not graphics.** Window creation is ~7 ms, yet
  first pixel lands at 55–70 ms even when the system does the fill during `ShowWindow` with no handler
  running. `ShowWindow` + DWM composition + first-paint dispatch is the cost. **G3 was built to ask
  whether the graphics stack could paint fast enough; once you stop waiting for it, the window is the
  bottleneck.** Further first-paint work belongs outside the renderer.

**⚠ Two earlier conclusions here were wrong.** This file previously said concurrency was *refuted* at
11% slower. That was an artifact of always measuring serial first while leaked D3D devices accumulated
between the arms. The opposite over-correction (8/8, ~5x) was real only for the heavily-contaminated
regime. **Rule that came out of it: never compare two configurations in a fixed order** when anything
can accumulate between them.

**Two of session 3's conclusions are withdrawn:**

- **The "~113 ms `CreateWindowExW` floor" does not exist.** The byte-identical binary on the same
  machine gives **9.3 ms** (7.3 – 12.5), a 13x discrepancy with non-overlapping ranges. The cause is
  not established — installer load is the leading hypothesis but session 5 also had an installer
  resident, which argues against it.
- **Using the spec's own stack is worth ~30 ms**, unprompted: D3D11 + DXGI reaches first pixel in
  126 ms where D2D's `HwndRenderTarget` takes 157 ms. The D2D leg was measuring a configuration
  Tailhawk was never going to ship.

~~**Still owed:** a post-reboot (cold) set and a quiet-machine set.~~ **Both taken, session 6.** Every
session-5 timing was taken with a VS installer resident, which is why they read ~1.8x high. Variance
still means any first-paint budget must be a percentile, not a mean — but **40 ms does not need
re-deriving**: it is met at 13.1 ms p50 / 14.5 ms p90 once something paints before the device exists.

### G4 result in one line

**The one-instanced-draw rule survives colour emoji.** `PLAN.md` §3 asserted that a premultiplied
colour atlas *"cannot share the mono atlas's blend state"*; it can, via **dual-source blending** —
`SV_Target0` premultiplied colour, `SV_Target1` per-channel coverage, `dest = src + dest*(1-cov)`,
which is simultaneously the correct ClearType blend and the correct premultiplied composite. Confirmed
by reading back the frame with a forced-neutral tint, where channel spread can only come from
per-channel coverage: **ClearType and colour glyphs both present in one draw**. No V2 re-costing owed.
Unified is also 25–32% cheaper on CPU than a two-pass split, and the split cannot do ClearType at all.

**Two findings the gate did not set out to make, both now in `SPEC.md` §3.2:**

- **Eviction passes only with an O(1) policy.** Scanning slots for the oldest costs 4–8 ms/frame under
  thrashing; an intrusive LRU list costs 0.17–0.37 ms. An earlier variable-width-span variant cost
  **106 ms/frame**. Uniform single-glyph slots are what make O(1) possible.
- **⚠ Superseded by session 7 — these are cache-miss figures, and the mechanism is a system font
  cache, not analysis granularity.** Cold is 86–108 µs/glyph and warm is ~3; see the top section. The
  eviction conclusion below is unaffected. Original text follows.
- **Rasterisation is the real stall: 145–210 µs per glyph, and 330–388 µs on a quiet machine.** A cold
  viewport of ~1,500 CJK glyphs needs 220–580 ms — 13–35 frames. So glyph rasterisation must be **off the
  paint path**, with placeholders filled in over later frames. That is now a v1 requirement and
  `SPEC.md` did not previously say it. Eviction is three orders of magnitude cheaper than the
  rasterisation it triggers. **Session 6 note: the quiet figure is the *slow* end, so this cost is not a
  load artefact** — and two phases doing identical work in one quiet process differed by ~18%, so the
  fixed-order rule applies inside G4's binary too.

### G7 result in one line

**The cause is `ScrollArea::State::offset` — an f32 holding an absolute content-pixel coordinate.**
The thumb mapping is exonerated (forward f32 error measures exactly zero) and row-height accumulation
is not a cause because egui never accumulates.

The finding worth having run the gate for: it breaks in **two independent ways, and fixing one leaves
the other**. Delta accumulation (`offset -= delta` discards a 2 px drag at 4M rows) was anticipated.
Row *layout* — `(inner_top - offset) + min_row as f32 * row_h`, two content-magnitude f32s differenced
to give a sub-row result — was not, and it **survives an exact scroll position**. So `SPEC.md` §6.4's
`u64` mandate is necessary and not sufficient; a plausible-looking `row * row_h - offset_px` in the
layout path reproduces the entire bug inside an otherwise correct grid. §6.4 now carries all three
derived rules and a CI assertion that catches them.

Both of the reporter's thresholds — 2M for onset, 100M for "very broken" — are **predicted from the
arithmetic alone**, by a model never fitted to them. `RESEARCH.md` §3.4 is now `[V]`.

### Reopened and then closed properly: the reboot happened, and it did settle something

**Superseded by session 6 — the owner did reboot, and the set was worth taking.** Session 5 wrote this
section off on the grounds that absolute figures were blocked on G5's reference machine rather than on a
reboot. That was half right: G5 is still required before any `SPEC.md` §11.3 figure is *published*, but
the quiet set refuted a design conclusion (the window-presentation floor) and flipped a gate verdict
(G3's first pixel). Neither of those needed a reference machine — they needed the load removed once.

The diagnosis below stands and explains the spread:

The cause of the instability is identified: **this machine carries a variable ~40% background load** from
a normal working set (Teams, Edge WebView2, Docker, OneDrive, Outlook), which is not going away. A 21-run
D2D set spanning it gave a p50 of 297 ms across a 117–783 ms range, with fast runs clustered wherever the
load happened to dip.

So the same static build has legitimately produced 96, 112, 126, 139, 154 and 297 ms — **and 75.5 ms
quiet, which is below all of them.** The lesson to carry: a quiet-machine set is cheap, and it is the
only way to tell a floor from a queue.

**What to do instead, and it is already the practice:** decide with **paired interleaved A/B** on this
machine — reliable, reproduced across runs, and immune to both load drift and accumulation effects.
Quote absolutes only as percentiles with the machine state stated, and never as a target.

The historical detail, retained because the discrepancy was large enough to mislead twice:

The G3 summary above records the 13x `CreateWindowExW` discrepancy. It is **still unexplained**, and it
is the reason no first-pixel figure has been promoted into `SPEC.md` §11.3.

| `CreateWindowExW`, same binary, same machine | median (range) |
|---|---|
| session 3, OneCore CRT | **113 ms** (21 – 144) |
| session 5, OneCore CRT, before the install | **8.5 ms** (6.7 – 11.4) |
| session 5, desktop CRT, `+crt-static` | **9.3 ms** (7.3 – 12.5) |
| session 5, desktop CRT, dynamic | **11.9 ms** (9.9 – 26.5) |

Three of four agree closely; session 3 is the outlier, and its range does not overlap the others. So
the CRT change is **not** the cause — the 8.5 ms measurement was taken before the install, on the same
OneCore toolchain that produced 113 ms.

**What settled it, session 6:** a post-reboot set and a quiet set (no VS installer, no Docker, no
browser) both give `CreateWindowExW` at 3.2–3.9 ms. With session 5's 8.5–11.9 ms that is three agreeing
sets against session 3's 113 ms. **Session 3 is the outlier, its cause is unexplained, and it is no
longer worth explaining.** The A/B comparisons remain the sound part of session 5 and are unaffected:
serial vs concurrent, D2D vs D3D11 + DXGI, static vs dynamic.

**Identity sweep: done 2026-07-29.** `RESEARCH.md`, `SPEC.md`, `PLAN.md`, `UI-DESIGN.md`, `LOKI.md`
and `HANDOFF.md` were swept for employer names, customer names, internal hostnames, service names,
email addresses, private IPs and local paths. **One hit** — a local profile path in this file's
Session artefacts section — now genericised. The surviving "employer" references are the policy
statements themselves (`SPEC.md` §2 and the rules below) and name nobody.

**Deferred until the app runs locally and going online is actually on the table:**

- **The code-signing route** (`PLAN.md` §7.1). Azure Artifact Signing is **ruled out** — its
  eligibility covers neither South African organizations nor individuals. Certum Open Source Code
  Signing eligibility for a South African individual is unconfirmed (from €69, hardware token
  shipped internationally). Not a blocker for v1 — ship unsigned via scoop; it blocks the first
  *signed* release only. **Note the linkage:** `LOKI.md` §7 argues signing becomes a prerequisite of
  the **network feature** specifically, because unsigned + portable + share-distributed + outbound
  TLS to file-specified hosts is a dropper signature that SmartScreen will never grant reputation
  to. That is a v2 concern, well after local.
- **`tailhawk.com` / `.dev` / `.io`** at a registrar. Never verified.

**Not deferred, because it decays:** claiming the **namespaces** — GitHub org `tailhawk`,
crates.io, scoop, winget. Verified free during naming, but nothing holds them. Losing `tailhawk`
after four documents and a repo are written around the name means a rename, not an inconvenience.
Cheap insurance, no publishing implied — an empty GitHub org and a reserved crate name are not a
release.

---

## Dogfooding — first runnable build

**✅ Done, session 9 — the encoding gate passed on both corpora. See the section at the top for the
measured result, and note the size/behaviour figures in this section are the 2026-07-29 sample and
are now stale.** Nothing further is owed here until **M3**, by owner decision: no need to dogfood
again until real logs actually render.

~~**Now actionable, as of session 8.**~~ `cargo run --release -p tailhawk -- <path>` opens a file,
detects its encoding and streams every line, reporting the result in the title bar. That is enough to
run against both corpora and see whether the encoding and line count are right — it is not enough to
*read* them, which needs M3.

~~**Worth doing early rather than late:**~~ **— and it was.** The value of the two corpora is that the
correct answer is already known, and a wrong answer this session is far cheaper than a wrong answer
after the index and grid are built on top of it. Corpus B in particular should report **UTF-8** and
its em dashes must survive; if the title says `windows-1252` the detector is wrong. **It reported
UTF-8 and all 47,101 em dashes survived.**

The owner has nominated **two real logs currently in daily use** as the first dogfood targets. The
build does not have to be good; it has to open these two files and follow them. Treat this as the
acceptance gate for "a first version that can actually run".

Actual paths are **deliberately not recorded in this repo** — they sit inside an employer source
tree and their content contains customer names. They are held in the private project memory
(the project memory directory, `~/.claude/projects/<project-key>/memory/`, where `<project-key>` is
the repo path with separators replaced by `-`). See the identity note below.

| | Corpus A | Corpus B |
|---|---|---|
| Size | ~450 KB | ~3.3 MB |
| State when sampled | idle — batch writer had exited | **live, actively appended** |
| Encoding | ASCII / UTF-8, no BOM | **UTF-8, no BOM, with non-ASCII** (U+2014 em dash) |
| Format | single, log4net-shaped | **heterogeneous — three formats in one file** |

**What each one actually tests:**

*Corpus A* — log4net layout, `yyyy-MM-dd HH:mm:ss,fff LEVEL␠␠Logger.Name message`. Note the
**comma** as the millisecond separator and the padded level field. Straight columnisation exercise
plus open-a-completed-file.

*Corpus B* is the valuable one, and it breaks several assumptions:

1. **BOM-less UTF-8 with non-ASCII content.** Windows PowerShell 5.1's `Get-Content` renders the em
   dashes as `â€”` — i.e. a CP1252 default gets this file wrong. This is a ready-made regression
   test for encoding detection; the correct answer is already known.
2. **Three formats in one file.** A banner line, then a **tab-delimited section with its own header
   row** (`TIME⇥INSTANCE⇥STAGE`) carrying **time-only timestamps** (`17:00:02`, no date), then later
   a different layout entirely (`yyyy-MM-dd HH:mm:ss␠␠[tag]␠␠message`). This confirms `format_id` and
   `parse_state` must be **per-record, not per-source** — already the model in `SPEC.md` §4, now with
   a real specimen behind it.
3. **Time-only timestamps.** `SPEC.md` has **no rule** for a record whose timestamp carries no date.
   Inherit the date from the last fully-dated record? Leave `timestamp` unset and rely on
   `observed_timestamp`? **Open — needs a decision.**
4. **Non-monotonic timestamps.** An `07:14:14` line appears *after* an `09:14:14` line. Nothing may
   assume within-source ordering. This matters most for the v2 merged timeline.

**What they do not test:** both are single-digit MB. Neither exercises the multi-GB path, the
block-sparse index, rotation, or UNC latency. A large file and a rolling set are still needed
separately.

**Identity flag — resolve before the first public commit.** Corpus A lives in an employer repo and
its log lines contain customer names; Corpus B names customer instances. This runs straight into the
locked-in "no employer identity anywhere in the repo" decision and into open question 2 below
(employment IP). Dogfooding against them privately is fine. **Never** paste sample lines, paths, or
screenshots from either into the repo, an issue, or a release note without scrubbing.

---

## Traps — do not rediscover these

Recorded because each cost real effort to find, and two of them were caught only by adversarial review.

| Trap | Where |
|---|---|
| **Look at the window. Four defects so far were invisible to every test in the suite** — (1) placeholder boxes with no text, because nothing requested frame 2 and an `InvalidateRect` inside `paint` is wiped by the `ValidateRect` after it; (2) a pure-black background from frame 2, a blend state left bound by the glyph pass; (3) selection tint never reaching the screen, because the painter was handed `&doc.rows` instead of `&*doc`; (4) the title's set description frozen at the inference made at open, so a window showing three files said two. **The common shape: the data was right and the presentation was not, and a test that asks the data cannot see it.** Screenshot the real window after any change to what is drawn or what the chrome says. | sessions 15–16 |
| **The index depends on encoding.** Decode *must* precede index — chunk boundaries need code-unit alignment. The first plan drafted them 13 weeks apart and would have thrown away tested work. *(This row also carried "and `0x0A` is a legal DBCS trail byte" until session 15, three rows above the entry refuting it. A reference table is read one row at a time, so each row has to stand alone.)* | `PLAN.md` §4 |
| **`RESEARCH.md` §5.3 is GPL-contaminated.** The line index must be **re-derived from published docs only**, with `CLEANROOM.md` populated *before* writing code. Do not implement from that section. | `RESEARCH.md` §5.3 |
| **Serilog and NLog roll to *new filenames* by default.** Neither copy-truncate nor rename-and-recreate describes it. Rolling sets (§5.5b) are v1 for this reason. | `SPEC.md` §5.5b |
| **Every ripgrep throughput figure in circulation is fabricated.** The cited article publishes timings only. The search performance target is deliberately unset. | `RESEARCH.md` §11 |
| **"Beat BareTail's 2 MB" is unwinnable** and retired — D3D11+DXGI+DirectWrite costs 30–60 MB before reading a byte. The claim is *flatness*, not absolute size. | `SPEC.md` §11.2 |
| **`FILE_SHARE_DELETE` is mandatory.** Without it we block the writer's own rotation — a real, reported bug in klogg. | `SPEC.md` §5.1 |
| **Never poll file size by path** — NTFS only replicates size to the directory entry on last-handle-close, so a path stat on a live log returns a frozen size forever. | `SPEC.md` §5.4 |
| **Filtering is not O(viewport).** Every filter change is a full-file pass. Debounce, stream, and require Enter on network paths. | `SPEC.md` §7.3 |
| **Continuations collapsed by default**, and no `--wrap` in v1 — both destroy the O(1) `u64` scroll model, and MEL Simple logs are multi-line *by default*. | `SPEC.md` §6.4 |
| **Accessibility is the only automated UI-test surface** for a custom-drawn grid. The minimal chrome provider is v1 for that reason, not for compliance. | `SPEC.md` §14.1 |
| **A `u64` scroll position is not enough on its own.** G7 found the row-*layout* conversion `row * row_h - offset_px` reproduces egui #1391 in full even when the scroll position is exact. One plausible line, whole bug back. | `SPEC.md` §6.4, `experiments/g7-egui-scroll/RESULTS.md` |
| **`GetData` returns `S_FALSE` when a D3D11 query isn't ready — and `S_FALSE` is a *success* HRESULT.** `Result<()>` is `Ok`, so `is_err()` is useless as a readiness test. G4's first GPU timings were all 0.000 ms because of it. Detect readiness with a sentinel the driver must overwrite. | `experiments/g4-glyph-atlas/RESULTS.md` |
| **`DWRITE_GLYPH_RUN::fontFace` is `ManuallyDrop<Option<..>>`.** Writing into it is safe; copying it *out* into an owned interface releases a reference never added, and the underflow surfaces later as a use-after-free. Borrow the face out of a `DWRITE_COLOR_GLYPH_RUN`. | `experiments/g4-glyph-atlas/src/text.rs` |
| **`measure.ps1 -OutFile` must match the filename the subject hardcodes** — `g3-d2d-first-pixel.txt`, `g3-d3d11-<mode>.txt`. Any other name and the script polls for a file nobody writes, warns eleven times and throws, having measured nothing. | `experiments/measure.ps1` |
| **A `windows_subsystem = "windows"` exe does not block PowerShell.** `& .\g4-glyph-atlas.exe` returns in 0.1 s while the process runs on for ~100 s. Poll for its report file, then kill it — it holds a D3D device and is otherwise a leaked subject. | `experiments/g4-glyph-atlas` |
| **A quiet-machine set is how you tell a floor from a queue.** Session 5 derived a "~50–60 ms window-presentation floor" from loaded runs; the quiet re-take put first pixel at 13.1 ms. Conversely G4's rasterisation came in at the *top* of its range when quiet, so the load explanation was wrong in both directions. | `experiments/g3-d3d11/RESULTS.md` |
| **⚠ DirectWrite's glyph cache is cross-process, so "run it again" is a *warm* measurement.** The first version of G4b reported 2.5 µs/glyph and concluded G4 was wrong by 150x; it was reading cache hits left by its own previous run. Only the first process to touch a (glyph, size) pair since the cache lost it measures rasterisation. | `experiments/g4b-batched-raster/RESULTS.md` |
| **An in-process cache probe cannot see a cross-process cache.** Five identical passes gave pass0/pass4 = 0.99x — a confident, worthless "no cache". Vary **em** to get a cold population without a reboot: the cache key includes size. | `experiments/g4b-batched-raster/RESULTS.md` |
| **A reboot is not a neutral cold start for anything that draws text** — it empties the font cache service, so post-reboot text rendering is several times slower than a quiet warm machine. This is what session 6 misread as "a cold run does not bound the cost favourably". | `experiments/g4b-batched-raster/RESULTS.md` |
| **Derive an atlas cell from measured glyph bounds, never from em size.** Guessing 20×26 for em 14 clipped 1,086 of 1,500 glyphs — and the clipped cells then compared *equal* between arms, which reads as a correctness pass. Assert `overflow == 0` before believing any bitmap comparison. | `experiments/g4b-batched-raster/RESULTS.md` |
| **Never time a frame with `Present` inside the measured region.** The flip model blocks on the back-buffer queue, which pinned every G4 frame to exactly 16.669 ms and hid the real cost entirely. | `experiments/g4-glyph-atlas/RESULTS.md` |
| **Test a streaming decoder at *every* split, not a plausible one.** Feeding each fixture at every chunk size from 1 byte upwards found three bugs a single-gulp read cannot: UTF-32 at 1 byte/read decoded to nothing at all; a pending CR resolved against a chunk that decoded to *no characters*, giving every CRLF a phantom empty line; and a UTF-32 unit left at EOF was dropped instead of becoming U+FFFD. All three pass at chunk sizes that happen to land on boundaries. | `crates/tailhawk-core/src/lines.rs` |
| **A decoded chunk can be empty.** UTF-16LE fed one byte at a time produces text on only every second read. Any per-chunk state machine that assumes "a chunk has content" — the pending-CR carry is one — resolves against a character that has not arrived yet. | `crates/tailhawk-core/src/lines.rs` |
| **`0x0A` is *not* a legal DBCS trail byte**, contrary to what `SPEC.md` §5.3 said until session 8. It is always a line terminator, in every byte-oriented encoding — so parallel indexing needs no encoding-specific exception at all, only code-unit alignment. | `crates/tailhawk-core/src/encoding.rs` |
| **Measure the decoder, not the encoder, when the question is "what can appear in a file".** The encoder-side test — every code point through every encoder — was persuasive and would have been an insufficient basis for a normative spec change: decoders accept sequences encoders never emit. The test that actually settles it drives all 65,536 two-byte prefixes *into* each decoder. Both pass; only one of them is evidence. | `crates/tailhawk-core/src/encoding.rs` |
| **A predicate that can only return one value is a fact, not an API.** `allows_parallel_indexing()` became unconditionally `true` once the exception was withdrawn, so it was deleted and `code_unit()` carries the rule. What replaced it — `is_random_access_decodable()`, false for `ISO-2022-JP` alone — is non-vacuous and about a different question. | `crates/tailhawk-core/src/encoding.rs` |
| **A tail encoding sample is meaningless without its absolute offset.** NUL-position parity is relative to the start of the *file*; probing a sample that begins at an odd offset reports UTF-16**BE** for a UTF-16**LE** file — a confident, exactly-wrong answer. | `crates/tailhawk-core/src/encoding.rs` |
| **A share-mode test that only checks the happy path proves nothing.** `writer_safety_…` passes whether or not `FILE_SHARE_DELETE` is set, unless it is paired with a negative control that opens read-shared only and asserts the writer *is* blocked. A share mode is exactly the kind of constant that gets tidied by someone who does not know what it costs. | `crates/tailhawk-core/src/file.rs` |
| **`cargo deny` looks for `deny.toml`, not `cargo-deny.toml`.** The config was misnamed from session 2 to session 8. The first time anything ran it, it logged one `[WARN] unable to find a config path` and then rejected all 26 crates including MIT and Apache-2.0, because the default allow-list is empty. A misnamed config silently stops being the policy, and only running the tool reveals it. | `deny.toml`, `CLEANROOM.md` §7 |
| **A config file nothing executes is documentation.** `deny.toml` and `CLEANROOM.md` §7 described a licence gate for two sessions while no CI job invoked it. Whenever a policy file is added, add the thing that runs it in the same commit. | `.github/workflows/ci.yml` |
| **LTO makes an unreferenced dependency free, which is a misleading way to measure one.** Adding `encoding_rs` + `chardetng` left the exe byte-identical at 249,344 while nothing called them; it went to 502,784 the moment the shell did. Measure a dependency's size *after* wiring it in, never before. | `crates/tailhawk/src/main.rs` |
| **Never check a live file's reported size against a separately-stat'd one.** Corpus A read 5,587,678 bytes against a stat of 2,773,726 taken a minute earlier and looked like a 2.01x bug — a suspiciously exact ratio, which is what made it convincing. The file had simply grown. **To verify counts, snapshot the file first and run against the frozen copy;** use the live file only to exercise sharing and rotation. | session 9 |
| **The obvious verification tools cannot open a live log at all.** `[IO.File]::ReadAllBytes` and anything else opening share-`Read` fails with a sharing violation against a real writer — the very case Tailhawk exists to handle. Take ground truth through `[IO.File]::Open($p,'Open','Read','ReadWrite,Delete')`. Reaching for the naive reader first reads as "the file is locked", not "my reader is wrong". | session 9 |
| **`cargo fmt --all` reformats the frozen experiment sources**, which are the record of how each measurement was taken and must not be tidied — session 14 already had to put them back once (`3ede15b`), and session 15 did it again the same way. Format the shipped crates by name: `cargo fmt -p tailhawk-core -p tailhawk`. | `experiments/` |
| **A bounded axis does not excuse §6.4's rule 3.** `x + offset_px` looks safe once the offset is provably exact and provably small enough — but at 96,624 px one ULP is 0.008 px, and a click 0.0012 px inside a column rounds up onto the next boundary and hit-tests to the right. Two exact terms do not make an exact sum. Resolve to the leftmost visible column first, then measure in viewport-sized numbers. | `crates/tailhawk-core/src/hgrid.rs` |
| **Snap the drawn offset, never the stored one.** Whole-pixel column positions are required — a fractional x resamples ClearType's horizontal subpixel coverage — but rounding the offset *in the state* makes a 0.4 px/frame trackpad round to zero every frame and the view never moves. Store exact, round once at layout, shared by every column so the spacing cannot drift. | `crates/tailhawk-core/src/hgrid.rs` |
| **A regression guard must assert a ratio, never a duration.** The first cost guard for the text pass asserted "under a second", measured 0.21 s alone, 0.60 s under full-suite load and 2.76–7.49 s in repeats — a flake that also made my "two orders of magnitude of headroom" claim wrong. This machine's ~40% background load is the thing `experiments/g3-d3d11` spent two sessions establishing; a duration threshold encodes the idle machine that never exists. Compare two arms **interleaved** and assert their ratio: `the_layout_cost_does_not_scale_…` runs 240 vs 24 columns five times alternating, compares medians, and its control fires at **12.9x** against a 3.0 threshold — a margin load cannot manufacture. | `crates/tailhawk-core/src/paint.rs` |
| **D3D11's context is global, and a pass that does not set a state inherits the last one's.** `TextPipeline::draw` binds a dual-source blend state and does not restore it; `draw_background` set none at all. Frame 1 therefore got the default and was correct, and **every frame after it drew nothing at all** — the background shader emits one output, so against an undefined second source the pass leaves the target untouched. On screen that reads as pure black, because a fresh flip-model back buffer is zeroed; the symptom therefore depends on what was already in the buffer, which is exactly how the first version of the regression test came to pass without firing. At RGB(18,20,23) against RGB(0,0,0) it is invisible to the eye, and it was M0 code that had been right for fifteen sessions because nothing else had ever bound a blend state. **Found by dogfooding a real file, not by any test.** Now guarded by `the_background_survives_a_frame_of_text`. Every pass must declare the states it depends on. | `crates/tailhawk-core/src/gpu.rs` |
| **Verifier agents share the working tree, deliberately — the fix is discipline, not isolation.** Worktree isolation (`isolation: 'worktree'`) would remove the hazard below outright, and was considered and **rejected on cost**: a worktree gets its own `target/`, so eleven verifiers each running `cargo test` against the `windows` crate is eleven cold builds. A 20-minute review would become far worse, to fix something a naming convention fixes for free. **Escalate to worktrees if this bites a second time** — owner's call, 2026-08-12, delegated. | workflows |
| **`git checkout -- <file>` is not "undo my last edit" — it is "discard everything since the last commit".** Used to back out a one-line probe during a negative control on 2026-08-13, it destroyed an hour of *uncommitted* selection work in the same file: the whole shell half of a feature, because the feature had not been committed yet. The undo that was wanted was reverting the probe alone. **Commit before applying a negative control** — the control is a deliberate temporary break, and there is nothing to lose only if the good state is already saved. Where that is impractical, back the file up first, as the `paint.rs` controls did earlier in the session. | git |
| **Never `git add -A` while a review workflow is running.** Verifier agents are told to settle a disagreement by writing a temporary test, running it and deleting it, so the working tree gains and loses files under you for the whole run. On 2026-08-11 a `git add -A` for an unrelated docs change swept up two half-finished probes from a verifier mid-flight and committed them under a commit message about something else. Commit **named paths** while a workflow is in flight, or wait for it. (The probes turned out to be worth keeping — see `every_ascii_character_is_one_cell_in_both_models` — but that was luck, not process.) | workflows |
| **An offscreen target keeps the previous frame; a swapchain does not.** A read-back test that does not clear first cannot tell "drawn correctly" from "not drawn at all", because last frame's pixels are still there — and a fresh flip-model buffer is zeroed, so the shipped code has no such safety net. The first version of the background regression test passed with its fix removed for exactly this reason. **Clear to a colour no pass can produce** (magenta) before each frame; the control then fires *and* names the mechanism, since a magenta corner says "nothing was drawn" where a black one would have said "drawn wrong". | `crates/tailhawk-core/src/gpu.rs` |
| **`InvalidateRect` during `WM_PAINT` does nothing** — the handler calls `ValidateRect` after `paint` returns, which clears exactly the region just invalidated. A frame that needs a successor has to raise a flag and let the handler invalidate *after* validating. The symptom is a window that renders its cold first frame and then freezes: with §3.2's rasterise-after-present, that is a full screen of placeholder boxes and no text, which reads as a layout bug rather than a missing repaint. | `crates/tailhawk/src/main.rs` |
| **The ASCII fast path in `cell.rs` is all-or-nothing per line, and one character decides it.** A single `—` at the start of a 19.4 KB record puts the whole line back on the cluster walk that `ColumnAnchors` then has to amortise. **Anyone measuring the horizontal cost must use a non-ASCII fixture** — an ASCII one takes an O(1) path and reports that everything is fine. The fixtures are `wide.log` and `widenonascii.log`, identical but for the first character. The same trap bit a *test*: `the_layout_cost_does_not_scale_…` had an ASCII fixture, so once the fast path landed both arms of its ratio fell into the tens of microseconds and it began failing at random on scheduler noise. Its fixture now starts with `—` for that reason. | `crates/tailhawk-core/src/cell.rs` |
| **Building an index can cost more than the lookup it replaces.** `ColumnAnchors::build` walks every cluster in a row; `byte_span`'s early exit walks only as far as the viewport shows. At column 0 that is 19,400 clusters against ~150, so building anchors unconditionally *regressed* column-0 paging from 16 ms to 39 ms — a measured loss in the common case, buying a win in a rarer one. Anchors are therefore gated on the view actually being scrolled right, via the `anchored` flag in the fetch cache key. Before adding any precomputation to a per-row path, measure the case that does **not** need it. | `crates/tailhawk-core/src/rows.rs` |
| **A `navigate` that moves nothing shows up as ~0.07 ms a frame, not as a fast frame.** A benchmark that page-downs past the end of its fixture measures the repaint suppression working, not the renderer — a 2,000-line fixture with 60 page-downs of 47 rows exhausts itself a third of the way in and reports absurdly good numbers. Size the fixture to the sweep. | `crates/tailhawk/src/main.rs` |
| **A correctly-painted window and a dead one are indistinguishable at this palette.** `BACKGROUND` is RGB(18, 20, 23), so an M1 window that is working perfectly looks like an empty frame — the owner reasonably reported seeing nothing. **Do not debug this by looking.** Screenshot the client area and assert the pixels equal `background_rgb8()`; that distinguishes "painting correctly" from "not painting" in one step. | `crates/tailhawk-core/src/lib.rs` |

---

## Still owed, not blocking

- **Fold `LOKI.md` into `SPEC.md` §4 and `PLAN.md` §2.3b.** The decisions are settled; the write-up
  is not yet integrated. The cost reconciliation between the two research rounds is owed to
  `PLAN.md` but gates nothing.
- **The highest-value Loki unknown:** is the Loki HTTP API reachable **directly** from a workstation,
  or only via Grafana's datasource proxy? Every measurement in `LOKI.md` went through the proxy. If
  proxy-only, that is a different URL shape, a different auth model and roughly +1 PW.
- **Namespace claims** — GitHub org, crates.io, scoop, winget. Deferred with the rest of the
  publication work, but unlike the rest **this one decays**: nothing holds `tailhawk` for us.

## Open questions the owner still needs to answer

1. **Certum eligibility** for a South African individual — needs a direct question to Certum.
2. **Employment IP position.** If any of this gets built on work equipment or work time, many contracts assign IP. Worth establishing **before the first public commit**, not after the repo has contributors.
3. **Reference perf machine** — must be fixed before any `[TBM]` target in `SPEC.md` §11.3 becomes a number.
4. **Hoo WinTail hands-on (G6)** — the owner's installed copy is the only reliable source for which of its features are actually used in a week, and for how its encoding detection really behaves.
5. ~~**Re-scope `SPEC.md` §3.2's rasterisation requirement?**~~ — **done, owner-approved, session 7.** §3.2 now states the cost as a first-run one (86–108 µs/glyph cold, ~3 µs warm, cache capacity 8,000–16,000 distinct glyphs), keeps placeholders as a v1 requirement, forbids deriving a §11.3 steady-state budget from the cold figure, and specifies batching at 4–64 glyphs per analysis. **The same edit also withdrew the refuted "~50–60 ms window-presentation floor"** from the first-paint bullet, which session 6 disproved but which was still stated as fact in the spec.
6. ~~**`SPEC.md` §5.3's DBCS rationale is factually wrong**~~ — **done, owner-approved, session 8.**
   The exception was deleted rather than narrowed, because the decoder-side measurement showed the
   property is universal across every byte-oriented encoding. §5.3 now states code-unit alignment as
   the *only* constraint on chunking, records both measurements, and keeps one much narrower rule:
   `ISO-2022-JP` cannot be decoded from an arbitrary line, because it is stateful. **M2 may index in
   parallel for every encoding.** See the section at the top.
7. ~~**How to fuzz, for M1's last criterion.**~~ — **done, session 9.** An ordinary `#[test]` with a
   deterministic seed and no dependency, not `cargo-fuzz`. The deciding argument was ARM64, not MSVC.
   **M1 is complete.** See the M1 section at the top.
8. ~~**The provenance questions above.**~~ — **resolved, session 9.** The index was re-derived clean
   and `SPEC.md` §5.3 replaced; `CLEANROOM.md` §4 reads RE-DERIVED CLEAN. **M2 is unblocked.** Three
   of the four were not decisions at all — see the section at the top.

---

## Session artefacts

Workflow transcripts and scripts, if any reasoning needs re-checking:

```
%USERPROFILE%\.claude\projects\<project-key>\<session-id>\
  workflows\scripts\      ← re-runnable workflow scripts
  subagents\workflows\    ← per-agent transcripts and journal.jsonl
```

Session 1 was `c8d47c59-98d0-40f0-86b8-bcd47f6558a3`; session 2 (the Loki run) was
`c2757c25-1785-4b18-9f85-7283f401aaf1`. These are machine-local and are not part of the repo.

Seven workflows ran this session: competitor/tech/format research, cross-platform, the agentic-native-UI
thesis, naming (two rounds), OpenTelemetry, the four-artifact adversarial review, and the stopped Loki run.
