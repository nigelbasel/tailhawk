//! G7 — reproduce egui issue #1391 (`ScrollArea::show_rows` jitters at large row counts).
//!
//! Dependency-free replication of the scroll arithmetic in egui 0.17.0
//! (`egui/src/containers/scroll_area.rs`), the release current when the issue was filed.
//! Every function below mirrors one expression from that file; the egui source is quoted in the
//! doc comment above each so the correspondence can be checked without re-reading egui.
//!
//! The question G7 asks is *which* f32 breaks: the thumb mapping, the accumulated scroll offset,
//! or row-height accumulation. Each is isolated in its own section.

use std::fmt::Write as _;

/// egui default `Style::spacing.item_spacing.y`.
const SPACING_Y: f32 = 3.0;
/// Viewport height in points — `inner_rect.size().y`.
const INNER_H: f32 = 800.0;
/// `inner_rect.min.y` — a plausible non-zero window-content origin.
const INNER_TOP: f32 = 24.0;
/// One trackpad/wheel notch. egui multiplies raw wheel ticks up to roughly this.
const WHEEL_NOTCH: f32 = 53.0;

/// Row heights to test: `TextStyle::Body` height and `interact_size.y`, the two the demo uses.
const ROW_HEIGHTS: [f32; 2] = [16.0, 18.0];

/// Row counts spanning the thresholds the reporter named (2M jitter, 100M "very broken").
const ROW_COUNTS: [u64; 8] = [
    10_000,
    100_000,
    1_000_000,
    2_000_000,
    4_000_000,
    16_000_000,
    100_000_000,
    160_000_000,
];

// ---------------------------------------------------------------------------------------------
// egui 0.17.0 arithmetic, replicated
// ---------------------------------------------------------------------------------------------

/// `ui.set_height((row_height_with_spacing * total_rows as f32 - spacing.y).at_least(0.0))`
fn content_height(total_rows: u64, row_h_ws: f32) -> f32 {
    (row_h_ws * total_rows as f32 - SPACING_Y).max(0.0)
}

/// `let min_row = (viewport.min.y / row_height_with_spacing).floor().at_least(0.0) as usize`
///
/// `viewport` is `Rect::from_min_size(Pos2::ZERO + state.offset, inner_size)`, so
/// `viewport.min.y` *is* `state.offset.y`.
fn min_row(offset: f32, row_h_ws: f32) -> u64 {
    (offset / row_h_ws).floor().max(0.0) as u64
}

/// `let y_min = ui.max_rect().top() + min_row as f32 * row_height_with_spacing`
///
/// Inside `show_rows`, `ui` is the *content* Ui, whose `max_rect().top()` is
/// `content_max_rect.min.y` = `inner_rect.min - state.offset` from `begin`. So the screen y of the
/// first drawn row is `(INNER_TOP - offset) + min_row * row_h_ws` — two numbers of content
/// magnitude subtracted to produce a sub-row result.
fn drawn_first_row_top(offset: f32, row_h_ws: f32) -> f32 {
    let content_top = INNER_TOP - offset;
    content_top + min_row(offset, row_h_ws) as f32 * row_h_ws
}

/// Where the first drawn row sits relative to the top of the viewport.
///
/// Correct behaviour: always in `(-row_h_ws, 0]`. The row partially scrolled off the top peeks
/// above the viewport by the sub-row remainder of the offset, and never by more.
fn residual(offset: f32, row_h_ws: f32) -> f32 {
    drawn_first_row_top(offset, row_h_ws) - INNER_TOP
}

/// `emath::lerp` — `(1.0 - t) * range.start() + t * range.end()`
fn lerp(a: f32, b: f32, t: f32) -> f32 {
    (1.0 - t) * a + t * b
}

/// `emath::remap` — `lerp(to, (x - from.start()) / (from.end() - from.start()))`
fn remap(x: f32, f0: f32, f1: f32, t0: f32, t1: f32) -> f32 {
    lerp(t0, t1, (x - f0) / (f1 - f0))
}

/// `emath::remap_clamp`, used as `from_content` in `Prepared::end`:
/// `remap_clamp(content, 0.0..=content_size[d], min_main..=max_main)`
fn remap_clamp(x: f32, f0: f32, f1: f32, t0: f32, t1: f32) -> f32 {
    if x <= f0 {
        t0
    } else if x >= f1 {
        t1
    } else {
        remap(x, f0, f1, t0, t1)
    }
}

// ---------------------------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------------------------

/// Distance to the next representable f32 above `x`.
fn ulp(x: f32) -> f32 {
    let x = x.abs();
    f32::from_bits(x.to_bits() + 1) - x
}

/// Smallest `d > 0` such that `x - d` is a different f32 from `x`.
///
/// Any drag or wheel delta below this is silently discarded: `state.offset[d] -= delta` is a no-op.
fn smallest_effective_delta(x: f32) -> f32 {
    let mut d = ulp(x) / 4.0;
    while x - d == x {
        d = f32::from_bits(d.to_bits() + 1);
        if d > x {
            return f32::INFINITY;
        }
    }
    d
}

fn max_offset(total_rows: u64, row_h_ws: f32) -> f32 {
    (content_height(total_rows, row_h_ws) - INNER_H).max(0.0)
}

fn mid_offset(total_rows: u64, row_h_ws: f32) -> f32 {
    max_offset(total_rows, row_h_ws) * 0.5
}

/// Sample points through the file. Precision degrades with absolute offset, so a single sample
/// point understates the problem near the end of the file and overstates it near the start.
const PROBES: [(f32, &str); 3] = [(0.1, "10%"), (0.5, "50%"), (0.9, "90%")];

// ---------------------------------------------------------------------------------------------
// T1 — the precision budget
// ---------------------------------------------------------------------------------------------

fn t1_precision_budget(out: &mut String, row_h_ws: f32) {
    let _ = writeln!(
        out,
        "\n## T1 — f32 precision budget of `state.offset.y` (row_h_ws = {row_h_ws})\n"
    );
    let _ = writeln!(
        out,
        "| total_rows | content height (px) | ULP at 50% (px) | ULP at 90% (px) | ULP at 90% in rows | smallest effective delta at 50% (px) |"
    );
    let _ = writeln!(out, "|---|---|---|---|---|---|");
    for &n in &ROW_COUNTS {
        let h = content_height(n, row_h_ws);
        let mid = mid_offset(n, row_h_ws);
        let late = max_offset(n, row_h_ws) * 0.9;
        let _ = writeln!(
            out,
            "| {n} | {h:.0} | {} | {} | {:.2} | {} |",
            ulp(mid),
            ulp(late),
            ulp(late) / row_h_ws,
            smallest_effective_delta(mid)
        );
    }
}

// ---------------------------------------------------------------------------------------------
// T2 — input accumulation: `state.offset[d] -= delta`
// ---------------------------------------------------------------------------------------------

fn t2_input_accumulation(out: &mut String, row_h_ws: f32) {
    let _ = writeln!(
        out,
        "\n## T2 — offset accumulation, `state.offset[d] -= delta` (row_h_ws = {row_h_ws})\n"
    );
    let _ = writeln!(
        out,
        "60 frames of a 2 px/frame drag (expect 120 px), and 20 wheel notches of {WHEEL_NOTCH} px \
         (expect {:.0} px), both starting at the midpoint of the file.\n",
        20.0 * WHEEL_NOTCH
    );
    let _ = writeln!(
        out,
        "| total_rows | at | drag: moved (px) | drag: error | wheel: moved (px) | wheel: error | wheel: actual step sizes (px) |"
    );
    let _ = writeln!(out, "|---|---|---|---|---|---|---|");

    for &n in &ROW_COUNTS {
        for (frac, label) in PROBES {
            let start = max_offset(n, row_h_ws) * frac;

            let mut offset = start;
            for _ in 0..60 {
                offset += 2.0;
            }
            let drag_moved = offset - start;

            let mut offset = start;
            let mut steps = Vec::new();
            for _ in 0..20 {
                let before = offset;
                offset += WHEEL_NOTCH;
                steps.push(offset - before);
            }
            let wheel_moved = offset - start;
            steps.sort_by(|a, b| a.partial_cmp(b).unwrap());
            steps.dedup();
            let step_list = steps
                .iter()
                .map(|s| format!("{s:.0}"))
                .collect::<Vec<_>>()
                .join(", ");

            let _ = writeln!(
                out,
                "| {n} | {label} | {drag_moved:.1} | {:+.0}% | {wheel_moved:.1} | {:+.0}% | {step_list} |",
                100.0 * (drag_moved / 120.0 - 1.0),
                100.0 * (wheel_moved / (20.0 * WHEEL_NOTCH) - 1.0),
            );
        }
    }
}

// ---------------------------------------------------------------------------------------------
// T3 — row positioning, with accumulation removed
// ---------------------------------------------------------------------------------------------

fn t3_row_positioning(out: &mut String, row_h_ws: f32) {
    let _ = writeln!(
        out,
        "\n## T3 — row positioning alone (row_h_ws = {row_h_ws})\n"
    );
    let _ = writeln!(
        out,
        "Accumulation is removed: each offset is computed in f64 from the midpoint and rounded to \
         f32 once, so this measures only `(INNER_TOP - offset) + min_row * row_h_ws`. 4096 samples \
         stepping one row at a time. A correct implementation keeps the first drawn row in \
         `(-{row_h_ws}, 0]` at every sample.\n"
    );
    let _ = writeln!(
        out,
        "| total_rows | at | residual min (px) | residual max (px) | spread (px) | spread in rows | samples out of band |"
    );
    let _ = writeln!(out, "|---|---|---|---|---|---|---|");

    for &n in &ROW_COUNTS {
        for (frac, label) in PROBES {
            let s = residual_scan(n, row_h_ws, frac, 4096);
            let _ = writeln!(
                out,
                "| {n} | {label} | {:.1} | {:.1} | {:.1} | {:.2} | {} / 4096 |",
                s.lo,
                s.hi,
                s.hi - s.lo,
                (s.hi - s.lo) / row_h_ws,
                s.out_of_band
            );
        }
    }
}

// ---------------------------------------------------------------------------------------------
// T4 — the scrollbar thumb mapping
// ---------------------------------------------------------------------------------------------

fn t4_thumb_mapping(out: &mut String, row_h_ws: f32) {
    let _ = writeln!(
        out,
        "\n## T4 — thumb mapping `from_content` and its inverse (row_h_ws = {row_h_ws})\n"
    );
    let _ = writeln!(
        out,
        "`from_content` is `remap_clamp(offset, 0..=content_size, min_main..=max_main)`. \
         Two columns matter and they are different things: the **inherent** content px per screen \
         px, which any scrollbar over a tall document has, and the **f32 error** of the remap \
         itself, measured against the same expression in f64.\n"
    );
    let _ = writeln!(
        out,
        "| total_rows | inherent content px / screen px | inherent rows / screen px | remap f32 error (screen px) | inverse f32 error (content px) | inverse error in rows |"
    );
    let _ = writeln!(out, "|---|---|---|---|---|---|");

    let min_main = INNER_TOP;
    let max_main = INNER_TOP + INNER_H;

    for &n in &ROW_COUNTS {
        let h = content_height(n, row_h_ws);
        let offset = mid_offset(n, row_h_ws);

        let inherent = h / INNER_H;

        let got = remap_clamp(offset, 0.0, h, min_main, max_main);
        let want = {
            let t = (offset as f64 - 0.0) / (h as f64 - 0.0);
            (1.0 - t) * min_main as f64 + t * max_main as f64
        };
        let remap_err = (got as f64 - want).abs();

        let handle_top = got;
        let back = remap(handle_top, min_main, max_main, 0.0, h);
        let back_want = {
            let t = (handle_top as f64 - min_main as f64) / (max_main as f64 - min_main as f64);
            (1.0 - t) * 0.0 + t * h as f64
        };
        let inv_err = (back as f64 - back_want).abs();

        let _ = writeln!(
            out,
            "| {n} | {inherent:.0} | {:.0} | {remap_err:.4} | {inv_err:.1} | {:.2} |",
            inherent / row_h_ws,
            inv_err / row_h_ws as f64
        );
    }
}

// ---------------------------------------------------------------------------------------------
// T5 — threshold prediction, checked against the reported thresholds
// ---------------------------------------------------------------------------------------------

/// First `total_rows` at which `pred` holds, found by a geometric scan rather than a binary search.
///
/// A bisection would be wrong here: only the ULP-derived predicates are monotonic in `n`, and the
/// row-position ones are not — a particular `n` can land on an arithmetically lucky alignment. The
/// scan reports the first crossing it actually observes and is honest about its resolution.
fn first_row_count_where(row_h_ws: f32, pred: impl Fn(u64, f32) -> bool) -> Option<u64> {
    let mut n = 10_000f64;
    while n < 1e10 {
        if pred(n as u64, row_h_ws) {
            return Some(n as u64);
        }
        n *= 1.01;
    }
    None
}

struct Scan {
    lo: f32,
    hi: f32,
    out_of_band: u32,
}

fn residual_scan(total_rows: u64, row_h_ws: f32, frac: f32, samples: u32) -> Scan {
    let start = (max_offset(total_rows, row_h_ws) * frac) as f64;
    let (mut lo, mut hi) = (f32::INFINITY, f32::NEG_INFINITY);
    let mut out_of_band = 0u32;
    for k in 0..samples {
        let offset = (start + k as f64 * row_h_ws as f64) as f32;
        let r = residual(offset, row_h_ws);
        lo = lo.min(r);
        hi = hi.max(r);
        if r > 0.0 || r <= -row_h_ws {
            out_of_band += 1;
        }
    }
    Scan {
        lo,
        hi,
        out_of_band,
    }
}

/// Worst-case residual spread across all three probe points.
fn residual_spread(total_rows: u64, row_h_ws: f32) -> f32 {
    PROBES
        .iter()
        .map(|&(frac, _)| {
            let s = residual_scan(total_rows, row_h_ws, frac, 1024);
            s.hi - s.lo
        })
        .fold(0.0f32, f32::max)
}

fn t5_thresholds(out: &mut String, row_h_ws: f32) {
    let _ = writeln!(
        out,
        "\n## T5 — predicted onset thresholds (row_h_ws = {row_h_ws})\n"
    );
    let _ = writeln!(
        out,
        "Geometric scan in 1% steps of `total_rows`, worst case over the three probe points. \
         Resolution is therefore ±1%.\n"
    );
    let _ = writeln!(
        out,
        "| symptom | predicted first total_rows |"
    );
    let _ = writeln!(out, "|---|---|");

    let worst_probe = |n: u64, rh: f32, delta: f32| {
        PROBES.iter().any(|&(frac, _)| {
            let o = max_offset(n, rh) * frac;
            o - delta == o
        })
    };

    let rows = [
        (
            "a 1 px drag delta is entirely discarded",
            Box::new(move |n: u64, rh: f32| worst_probe(n, rh, 1.0))
                as Box<dyn Fn(u64, f32) -> bool>,
        ),
        (
            "a 2 px drag delta is entirely discarded",
            Box::new(move |n: u64, rh: f32| worst_probe(n, rh, 2.0)),
        ),
        (
            "row-position error exceeds 1 px",
            Box::new(|n: u64, rh: f32| residual_spread(n, rh) > 1.0),
        ),
        (
            "row-position error exceeds a full row",
            Box::new(|n: u64, rh: f32| residual_spread(n, rh) > rh),
        ),
        (
            "row-position error exceeds a full screen",
            Box::new(|n: u64, rh: f32| residual_spread(n, rh) > INNER_H),
        ),
        (
            "a whole wheel notch is discarded",
            Box::new(move |n: u64, rh: f32| worst_probe(n, rh, WHEEL_NOTCH)),
        ),
    ];

    for (name, pred) in rows {
        match first_row_count_where(row_h_ws, pred) {
            Some(n) => {
                let _ = writeln!(out, "| {name} | {n} |");
            }
            None => {
                let _ = writeln!(out, "| {name} | not reached below 1e10 |");
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// T6 — controls: does the SPEC rule actually fix it?
// ---------------------------------------------------------------------------------------------

/// The model `SPEC.md` mandates: scroll position is a `u64` row index plus a sub-row pixel
/// remainder that never grows beyond one row. No f32 ever holds a content-space coordinate.
struct RowScroll {
    top_row: u64,
    sub_px: f32,
}

impl RowScroll {
    fn scroll_by(&mut self, dy_px: f32, row_h_ws: f32, total_rows: u64) {
        let mut sub = self.sub_px + dy_px;
        while sub >= row_h_ws {
            sub -= row_h_ws;
            self.top_row = (self.top_row + 1).min(total_rows.saturating_sub(1));
        }
        while sub < 0.0 {
            sub += row_h_ws;
            self.top_row = self.top_row.saturating_sub(1);
        }
        self.sub_px = sub;
    }
}

fn t6_controls(out: &mut String, row_h_ws: f32) {
    let _ = writeln!(out, "\n## T6 — controls (row_h_ws = {row_h_ws})\n");
    let _ = writeln!(
        out,
        "The same 60-frame 2 px/frame drag from T2, at 160,000,000 rows, under three position \
         representations.\n"
    );

    let n = 160_000_000u64;
    let expected = 120.0f32;

    let start = mid_offset(n, row_h_ws);
    let mut offset = start;
    for _ in 0..60 {
        offset += 2.0;
    }
    let f32_moved = offset - start;

    let start64 = mid_offset(n, row_h_ws) as f64;
    let mut offset64 = start64;
    for _ in 0..60 {
        offset64 += 2.0;
    }
    let f64_moved = offset64 - start64;

    let mut rs = RowScroll {
        top_row: n / 2,
        sub_px: 0.0,
    };
    let start_px = rs.top_row as f64 * row_h_ws as f64 + rs.sub_px as f64;
    for _ in 0..60 {
        rs.scroll_by(2.0, row_h_ws, n);
    }
    let rows_moved = rs.top_row as f64 * row_h_ws as f64 + rs.sub_px as f64 - start_px;

    let _ = writeln!(out, "| representation | moved (px) | error vs {expected:.0} px |");
    let _ = writeln!(out, "|---|---|---|");
    let _ = writeln!(
        out,
        "| f32 content-pixel offset (egui 0.17.0) | {f32_moved:.1} | {:.1} |",
        f32_moved - expected
    );
    let _ = writeln!(
        out,
        "| f64 content-pixel offset | {f64_moved:.1} | {:.1} |",
        f64_moved - expected as f64
    );
    let _ = writeln!(
        out,
        "| u64 row index + sub-row f32 (SPEC rule) | {rows_moved:.1} | {:.1} |",
        rows_moved - expected as f64
    );

    let mut worst = 0.0f32;
    let mut rs = RowScroll {
        top_row: n / 2,
        sub_px: 0.0,
    };
    for _ in 0..4096 {
        rs.scroll_by(row_h_ws, row_h_ws, n);
        worst = worst.max(rs.sub_px.abs());
    }
    let _ = writeln!(
        out,
        "\nT3's row-position error under the same u64 model, 4096 one-row steps at 160M rows: \
         worst sub-row remainder {worst} px — the residual cannot leave `(-{row_h_ws}, 0]` by \
         construction, because no content-magnitude number is ever formed."
    );
}

// ---------------------------------------------------------------------------------------------
// T7 — is the screen-space origin even retained?
// ---------------------------------------------------------------------------------------------

/// `content_max_rect.min.y = inner_rect.min - state.offset` mixes a screen-space coordinate
/// (tens of px) with a content-space one (up to billions). Once `ulp(offset) > 2 * INNER_TOP`,
/// the window's own y origin cannot survive the subtraction.
fn t7_origin_loss(out: &mut String, row_h_ws: f32) {
    let _ = writeln!(
        out,
        "\n## T7 — survival of the screen origin in `inner_rect.min - state.offset` (row_h_ws = {row_h_ws})\n"
    );
    let _ = writeln!(
        out,
        "`INNER_TOP` is {INNER_TOP} px. The column is what is left of it after the subtraction: \
         `(INNER_TOP - offset) - (0.0 - offset)`.\n"
    );
    let _ = writeln!(out, "| total_rows | retained at 50% (px) | retained at 90% (px) |");
    let _ = writeln!(out, "|---|---|---|");
    for &n in &ROW_COUNTS {
        let retained = |frac: f32| {
            let o = max_offset(n, row_h_ws) * frac;
            (INNER_TOP - o) - (0.0 - o)
        };
        let _ = writeln!(
            out,
            "| {n} | {} | {} |",
            retained(0.5),
            retained(0.9)
        );
    }
}

// ---------------------------------------------------------------------------------------------

fn main() {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "# G7 — egui #1391 reproduction, egui 0.17.0 arithmetic\n\n\
         Viewport {INNER_H} px at y={INNER_TOP}, item_spacing.y={SPACING_Y}, wheel notch \
         {WHEEL_NOTCH} px."
    );

    for rh_sans in ROW_HEIGHTS {
        let row_h_ws = rh_sans + SPACING_Y;
        let _ = writeln!(
            out,
            "\n\n# row_height_sans_spacing = {rh_sans} (row_h_ws = {row_h_ws})"
        );
        t1_precision_budget(&mut out, row_h_ws);
        t2_input_accumulation(&mut out, row_h_ws);
        t3_row_positioning(&mut out, row_h_ws);
        t4_thumb_mapping(&mut out, row_h_ws);
        t5_thresholds(&mut out, row_h_ws);
        t6_controls(&mut out, row_h_ws);
        t7_origin_loss(&mut out, row_h_ws);
    }

    print!("{out}");
}
