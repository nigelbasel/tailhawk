# G7 — reproducing egui #1391

`PLAN.md` §3 G7. **Pass criterion: "the actual cause is identified." Met.**

Measured 2026-07-29.

## The question

`RESEARCH.md` §3.4 marks the diagnosis **[L]**, not [V]. egui issue #1391 reports that
`ScrollArea::show_rows` jitters above ~2M rows and "gets very broken" above ~100M; the reporter
(parasyte, 2022-03-21) writes *"It looks like an issue with `f32` precision, but I haven't been able
to track it down yet."* The whole Tailhawk grid design rests on the rule that follows from it, so
G7 asks **which** f32: the scrollbar thumb mapping, the accumulated scroll offset, or row-height
accumulation.

## Method

Pure arithmetic replication — no GPU, no windowing, no dependencies. Every function in
`src/main.rs` mirrors one expression from **egui 0.17.0** `egui/src/containers/scroll_area.rs`, the
release current when the issue was filed, with the original quoted in the doc comment above it. The
functions replicated are `ScrollArea::begin`, `show_rows`, the offset mutations in `Prepared::end`,
and `emath::remap` / `remap_clamp` / `lerp`.

Fixed parameters: viewport 800 px at screen y=24, `item_spacing.y` = 3.0 (egui default), wheel notch
53 px. Row height is run at both 16.0 (`TextStyle::Body`) and 18.0 (`interact_size.y`); the two
differ only in where thresholds land, by well under 2x, so the tables below quote row height 16.0
(`row_h_ws` = 19.0) and the conclusions hold for both.

Each test samples at 10%, 50% and 90% through the file. **This matters** — precision degrades with
absolute offset, so a single sample point at the midpoint understates the problem by roughly a
factor of two.

## Answer in one line

**It is `ScrollArea::State::offset` — an `f32` holding an absolute content-pixel coordinate.** Not
the thumb mapping, and not row-height accumulation. It breaks in **two independent ways**, and
fixing either one leaves the other intact.

## The precision budget

`state.offset.y` must span the whole document height, so its ULP grows with the file.

| total_rows | content height (px) | ULP at 90% (px) | in rows | smallest delta that moves at all, at 50% (px) |
|---|---|---|---|---|
| 1,000,000 | 18,999,996 | 2 | 0.11 | 0.5 |
| 2,000,000 | 37,999,996 | 4 | 0.21 | 1.0 |
| 4,000,000 | 76,000,000 | 8 | 0.42 | 2.0 |
| 16,000,000 | 304,000,000 | 32 | 1.68 | 8.0 |
| 100,000,000 | 1,900,000,000 | 128 | 6.74 | 32.0 |
| 160,000,000 | 3,040,000,000 | 256 | 13.47 | 64.0 |

At 160M rows a single ULP of the scroll offset is **13 rows**.

## Failure 1 — input accumulation

`state.offset[d] -= scroll_delta[d]` (wheel) and `state.offset[d] -= ui.input().pointer.delta()[d]`
(drag). Any delta below half a ULP rounds away to nothing: **the assignment is a no-op and the
motion is silently discarded.**

60 frames of a 2 px/frame drag — a slow, deliberate drag, one second of it. Expected 120 px:

| total_rows | at 10% | at 50% | at 90% |
|---|---|---|---|
| 1,000,000 | 120.0 | 120.0 | 120.0 |
| 2,000,000 | 120.0 | 120.0 | **4.0** |
| 4,000,000 | 120.0 | **0.0** | **0.0** |
| 16,000,000 | 120.0 | **0.0** | **0.0** |
| 100,000,000 | **0.0** | **0.0** | **0.0** |

Twenty wheel notches of 53 px, expected 1060 px, showing the actual per-notch step:

| total_rows | at 50%: moved / step | at 90%: moved / step |
|---|---|---|
| 1,000,000 | 1060 / 53 px | 1040 / 52 px |
| 2,000,000 | 1042 / 52, 54 px | 1040 / 52 px |
| 16,000,000 | 960 / 48 px | 1280 / **64 px** |
| 100,000,000 | 1280 / **64 px** | **0 / 0 px** |
| 160,000,000 | **0 / 0 px** | **0 / 0 px** |

Two distinct symptoms, both visible to a user: the wheel silently changes gear (53 → 48 → 64 px per
notch, depending on where in the file you are), and past ~100M rows at the end of the file the wheel
**stops working entirely** — 53 px is less than half of the 128 px ULP, so every event rounds to
zero.

## Failure 2 — row positioning, which survives an exact scroll position

`show_rows` computes the screen y of the first drawn row as

```rust
let y_min = ui.max_rect().top() + min_row as f32 * row_height_with_spacing;
```

where, from `begin`, `ui.max_rect().top()` is `inner_rect.min.y - state.offset.y`. Expanded:

```
y_min = (inner_top - offset) + min_row * row_h_ws
```

Both terms are of *content* magnitude — up to 3×10⁹ — and each is independently rounded to its own
ULP. Their difference is supposed to be a **sub-row** quantity. This is textbook catastrophic
cancellation.

The test removes accumulation entirely: each offset is computed in f64 and rounded to f32 exactly
once, so nothing here is inherited from Failure 1. 4096 samples, stepping one row at a time. A
correct implementation keeps the first drawn row in `(-19, 0]` at every sample.

| total_rows | at | residual range (px) | spread in rows | samples out of band |
|---|---|---|---|---|
| 1,000,000 | 90% | −18.0 … −16.0 | 0.11 | 0 / 4096 |
| 2,000,000 | 90% | −16.0 … 0.0 | 0.84 | 0 / 4096 |
| 4,000,000 | 50% | −16.0 … 0.0 | 0.84 | 0 / 4096 |
| 16,000,000 | 50% | −24.0 … 8.0 | 1.68 | **2048 / 4096** |
| 16,000,000 | 90% | 8.0 … 8.0 | 0.00 | **4096 / 4096** |
| 100,000,000 | 90% | −152.0 … 104.0 | 6.74 | **4096 / 4096** |
| 160,000,000 | 90% | −280.0 … 232.0 | **26.95** | **4096 / 4096** |

At 160M rows the first row lands anywhere in a **512 px band** — most of the 800 px viewport — while
the scroll position advances one exact row at a time. That is the jitter.

The 16M/90% row is worth its own line: spread 0.0, yet every sample out of band. The rows are
*stably* misplaced 8 px below where they belong, leaving a permanent gap at the top of the viewport.
**Zero jitter is not the same as correct.**

### Why this one is the dangerous finding

Failure 2 does not depend on Failure 1. Making the scroll position exact — u64, f64, rational,
anything — does not fix it, because the fault is in converting an exact position *into* a content-
pixel f32 to lay rows out. `RESEARCH.md` §3.4 anticipated the accumulation trap and warned that
adopting "u64 thumb" and stopping there would miss it. This is a **second** trap of the same family
and the sneakier of the two, because it survives the fix for the first.

## Not the cause — the thumb mapping

`from_content` is `remap_clamp(offset, 0.0..=content_size, min_main..=max_main)`.

| total_rows | inherent rows per screen px | f32 error of the forward remap (screen px) | f32 error of the inverse (rows) |
|---|---|---|---|
| 16,000,000 | 20,000 | 0.0000 | 0.30 |
| 100,000,000 | 125,000 | 0.0000 | 0.69 |
| 160,000,000 | 200,000 | 0.0000 | 1.90 |

The forward mapping's f32 error is **exactly zero** at every row count tested — it divides two large
numbers and lerps into a 800 px range, which is numerically benign. The inverse (dragging the thumb)
carries up to 1.9 rows of f32 error at 160M, which is nothing beside the **200,000 rows per screen
pixel** the mapping inherently has at that size. **The thumb is exonerated as a precision fault.**

That inherent ratio is a real problem, but it is a *UI* problem — a thumb one pixel wide addressing
200,000 rows per pixel is unusable for fine positioning at any precision — and it is not what #1391
reports.

## Not the cause — row-height accumulation

There is none to blame. egui does not accumulate row heights; it multiplies once
(`min_row as f32 * row_height_with_spacing`). The multiply itself is correctly rounded. What breaks
is that its **product** is of content magnitude, which is Failure 2. Row-height accumulation is
therefore ruled out as a distinct cause — but only because egui never does it. Any implementation
that *did* accumulate would add a third, independent error term on top of these two.

## Threshold prediction vs. the report

Geometric scan in 1% steps of `total_rows`, worst case over the three probe points, ±1% resolution:

| symptom | predicted first total_rows | reporter |
|---|---|---|
| a 1 px drag delta is entirely discarded | 1,001,834 | — |
| row-position error exceeds 1 px | 991,915 | *"up to about 2 million… relatively smooth"* |
| a 2 px drag delta is entirely discarded | 2,030,549 | **"above 2 million rows… obvious jitter"** |
| row-position error exceeds a full row | 2,380,979 | **"above 2 million rows… obvious jitter"** |
| a whole wheel notch is discarded | 62,876,383 | *"above 100 million… gets very broken"* |
| row-position error exceeds a full screen | 299,877,056 | *"above 100 million… gets very broken"* |

**Both reported thresholds are predicted from the arithmetic alone.** The 2M onset is where a
deliberate drag starts being discarded and where row misplacement first exceeds a whole row, within
2% and 19% of the reported figure respectively. The 100M "very broken" point is bracketed by the two
catastrophic symptoms — the wheel ceasing to function (63M) and rows being flung a full screen out of
place (300M).

This is the strongest evidence available short of building the demo: the mechanism reproduces the
symptom *and* independently predicts where it starts, at two separate thresholds, from arithmetic
that was never fitted to them.

## The screen origin does not survive either

`content_max_rect.min = inner_rect.min - state.offset` subtracts a content-space coordinate from a
screen-space one. What is left of the 24 px window origin afterwards:

| total_rows | retained at 50% | retained at 90% |
|---|---|---|
| 4,000,000 | 24 px | 24 px |
| 16,000,000 | **16 px** | **32 px** |
| 100,000,000 | **0 px** | **0 px** |

Above ~16M rows the window's own y origin is corrupted; above ~100M it is **completely destroyed**.
Mixing screen-space and content-space coordinates in one f32 expression is a third face of the same
mistake.

## Controls — does the SPEC rule actually work?

The same 60-frame 2 px/frame drag at 160,000,000 rows:

| representation | moved | error |
|---|---|---|
| f32 content-pixel offset (egui 0.17.0) | 0.0 px | −120.0 |
| f64 content-pixel offset | 120.0 px | 0.0 |
| **u64 row index + sub-row f32 remainder** | **120.0 px** | **0.0** |

Under the u64 model, 4096 one-row steps at 160M rows give a worst sub-row remainder of **0 px** — the
residual *cannot* leave `(-19, 0]`, because no content-magnitude number is ever formed.

f64 also passes this test. It is the wrong fix regardless: it moves the failure out to a larger file
rather than removing it, and it silently reintroduces Failure 2 the moment an f64 position is cast to
f32 for layout — which it must be, since the GPU takes f32.

## Consequences for Tailhawk

`SPEC.md` §6.4 already mandates a `u64` scroll model. G7 confirms that is correct and adds three
rules that do **not** follow from it, each corresponding to a failure above:

1. **Scroll position is `(u64 row, f32 sub_row_px)` with `sub_row_px ∈ [0, row_height)`.** Never an
   absolute content-pixel coordinate in any float width. Wheel and drag deltas are applied to the
   sub-row remainder and carried into the row index, so no delta is ever added to a large number.
2. **Row layout is computed from `(row − top_row)`, a small integer.** Never `row * row_height −
   scroll_offset_px`. This is the rule that Failure 2 shows does not follow from rule 1, and the one
   most likely to be reintroduced by accident by someone who believes the u64 rule alone is
   sufficient.
3. **Never mix screen-space and content-space coordinates in one expression.** Resolve to
   viewport-relative first, then add the window origin.

Rule 2 is the finding worth having spent the gate on. Rules 1 and 3 are corollaries of the general
principle; rule 2 is a specific, plausible-looking line of code that reproduces the whole bug on its
own inside an otherwise correct grid.

**Suggested test to carry into the grid work:** assert that the first drawn row's viewport-relative y
stays in `(-row_height, 0]` across a sweep at 10⁸ rows. That single assertion catches every failure
mode above, and it is cheap enough to run in CI.

## Caveats

- **Arithmetic, not the running demo.** This establishes that egui 0.17.0's scroll arithmetic
  produces the reported symptom at the reported thresholds. It does not prove no *other* defect also
  contributes — only that none needs to be invoked to explain the report.
- **egui 0.17.0 specifically.** Later versions may have changed these expressions. The issue remains
  open and untouched since 2022-04-04, so no fix is expected to have landed, but this was not checked
  against current egui.
- Row height, viewport height and window origin are plausible constants, not measured from the demo.
  The row-height sensitivity was tested (16.0 and 18.0); viewport and origin were not swept.
- egui is MIT OR Apache-2.0 and is on the `CLEANROOM.md` §3 allow-list. The consultation is logged in
  `CLEANROOM.md` §5.

## Reproducing

No dependencies; the `.cargo/config.toml` `LIB` override is still needed on this machine to link.

```
cargo run --release -p g7-egui-scroll
```

Writes the full report — both row heights, all seven tests — to stdout as markdown.
