//! The virtualised grid — V3's scroll model, row layout and hit-test.
//!
//! Portable, device-free and headless, like [`crate::cell`]. Nothing here draws: it answers *which*
//! rows are on screen and *where*, and turns a click into a row. `SPEC.md` §6.4 specifies the whole
//! of it in three rules, and this module exists to enforce them structurally rather than by
//! discipline — because `experiments/g7-egui-scroll` measured what happens when they are not.
//!
//! ## The three rules, and why two of them do not follow from the first
//!
//! G7 reproduced egui #1391 from arithmetic alone and predicted both of the reporter's thresholds —
//! jitter "above 2 million rows" and "very broken above 100 million" — from the precision budget of
//! a single `f32`. The cause is `ScrollArea::State::offset`: **an `f32` holding an absolute
//! content-pixel coordinate.** At 160M rows one ULP of it is 13 rows.
//!
//! 1. **Scroll position is `(u64 row, f32 sub_row_px)`**, never an absolute content-pixel
//!    coordinate in any float width. Deltas are applied to the *remainder* and carried into the row
//!    index, so a small delta is never added to a large number. Measured: 60 frames of a deliberate
//!    2 px/frame drag moved **0 px** at 100M rows under the f32 model, and the wheel stopped
//!    responding entirely — 53 px is less than half of a 128 px ULP.
//! 2. **Row layout is computed from `(row − top_row)`, a small integer** — never
//!    `row * row_height − scroll_offset_px`. **This is the rule that does not follow from rule 1**,
//!    and G7 calls it the finding worth having spent the gate on: it survives an exact scroll
//!    position, because the fault is in converting an exact position *into* a content-pixel `f32`
//!    to lay rows out. One plausible-looking line reproduces the whole bug inside an otherwise
//!    correct grid — at 160M rows it flings the first drawn row across a **512 px band** while the
//!    scroll position advances one exact row at a time.
//! 3. **Never mix screen-space and content-space coordinates in one expression.** Resolve to
//!    viewport-relative first, then add the window origin. Above ~16M rows egui's
//!    `inner_rect.min − state.offset` corrupts the window's own y origin; above ~100M it destroys
//!    it outright.
//!
//! **`f64` is not the fix and was rejected**, though it passes the drag test: it moves the failure
//! out to a larger file rather than removing it, and it silently reintroduces rule 2's fault the
//! moment an `f64` position is cast to `f32` for layout — which it must be, because the GPU takes
//! `f32`.
//!
//! ## What enforces it here
//!
//! [`Scroll`] cannot represent a content-pixel offset, and [`Grid::visible`] hands out a
//! viewport-relative `y` computed from a loop counter. **No content-magnitude number is formed in
//! any layout or scroll-position expression** — the largest float in either is one row height, or
//! the caller's own delta.
//!
//! That qualifier is exact rather than decorative, and two places sit outside it deliberately.
//! [`Grid::thumb_fraction`] and [`Grid::scroll_to_fraction`] *do* form content-magnitude numbers, in
//! `f64`: the scrollbar has to divide the position by the document length, there is no way around
//! it, and G7 measured that mapping's error at **exactly zero** because it divides two large numbers
//! rather than differencing them. And a caller who passes `scroll_by_px` a delta of tens of millions
//! of pixels gets `f32` precision on their own delta — [`Grid::scroll_by_rows`] is the way to move
//! that far.

/// Where the viewport sits in the document.
///
/// **Two fields, deliberately.** A single pixel offset is the defect `SPEC.md` §6.4 exists to
/// prevent; a single row index cannot express a partially-scrolled row, which is what makes smooth
/// scrolling smooth. `sub_row_px` is how far the top row is scrolled *up* out of the viewport, so
/// the top row is always drawn at `-sub_row_px`.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Scroll {
    /// The first row touching the viewport.
    pub row: u64,
    /// How far that row is scrolled above the viewport's top edge, in `[0, row_height)`.
    pub sub_row_px: f32,
}

impl Scroll {
    pub const TOP: Self = Self {
        row: 0,
        sub_row_px: 0.0,
    };
}

/// One row placed in the viewport.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct PlacedRow {
    /// The document row.
    pub row: u64,
    /// Its top edge, **relative to the viewport's top**, in pixels. The first row's is always in
    /// `(-row_height, 0]`.
    pub y: f32,
}

/// The virtualised grid: a document of `total_rows` uniform rows seen through a viewport.
///
/// **Uniform row height is load-bearing, not a simplification.** `SPEC.md` §6.4 gives O(1)
/// row→pixel mapping *only* under it, which is why continuations are collapsed by default and
/// `--wrap` is cut from v1 — wrapping makes document height a function of window width and would
/// need recomputation across 50M rows on every resize and every `WM_DPICHANGED`.
#[derive(Clone, Debug)]
pub struct Grid {
    total_rows: u64,
    row_height: f32,
    viewport_px: f32,
    scroll: Scroll,
}

impl Grid {
    /// `row_height` is in whole device pixels — the same integer advance `SPEC.md` §3.2 requires of
    /// the glyph cell, so a row boundary always lands on a pixel boundary.
    pub fn new(row_height: f32) -> Self {
        Self {
            total_rows: 0,
            row_height: row_height.max(1.0),
            viewport_px: 0.0,
            scroll: Scroll::TOP,
        }
    }

    pub fn row_height(&self) -> f32 {
        self.row_height
    }

    pub fn total_rows(&self) -> u64 {
        self.total_rows
    }

    pub fn viewport_px(&self) -> f32 {
        self.viewport_px
    }

    pub fn scroll(&self) -> Scroll {
        self.scroll
    }

    /// The document grew or shrank.
    ///
    /// **A viewport that was against the end stays against it**, which is what makes following a
    /// growing file work; one that was not keeps its top row, so a user who has scrolled up to read
    /// something is not yanked to the bottom every time a line arrives. Shrinking pulls a stranded
    /// viewport back, which is what a rotated or truncated file (§5.5) needs.
    pub fn set_total_rows(&mut self, total_rows: u64) {
        let following = self.is_following();
        self.total_rows = total_rows;
        self.settle(following);
    }

    pub fn set_viewport_px(&mut self, viewport_px: f32) {
        let following = self.is_following();
        self.viewport_px = finite_or(viewport_px, 0.0).max(0.0);
        self.settle(following);
    }

    /// Changes the row height — a DPI change, or a font-size change.
    ///
    /// **The top row is kept, and the sub-row remainder is scaled with it.** Keeping the pixel
    /// offset instead would move the view by a different number of rows at each DPI, which is the
    /// column-drift §3.3's acceptance test watches for, one axis over. A viewport that was following
    /// the end keeps following it instead.
    pub fn set_row_height(&mut self, row_height: f32) {
        let following = self.is_following();
        let row_height = finite_or(row_height, 1.0).max(1.0);
        let fraction = self.scroll.sub_row_px / self.row_height;
        self.row_height = row_height;
        self.scroll.sub_row_px = fraction * row_height;
        self.settle(following);
    }

    /// Scrolls by a pixel delta — a wheel notch, or a drag. Positive moves *down* the document.
    ///
    /// **The delta lands on the remainder and is carried into the row index**, which is rule 1. The
    /// arithmetic never sees a number larger than one row height plus the delta itself, so a 2 px
    /// drag moves 2 px at row zero and at row 10⁸ alike.
    ///
    /// A non-finite delta is ignored rather than propagated — see [`Scroll`].
    pub fn scroll_by_px(&mut self, delta_px: f32) {
        if !delta_px.is_finite() || self.viewport_px <= 0.0 {
            return;
        }
        let (rows, sub_row_px) = carry(self.scroll.sub_row_px, delta_px, self.row_height);
        self.scroll = step(self.scroll.row, rows, sub_row_px);
        self.clamp();
    }

    /// Scrolls by whole rows — a key press, or a page.
    ///
    /// A delta against a viewport with no height is dropped, like [`scroll_by_px`](Self::scroll_by_px):
    /// there is nothing on screen to move relative to, and moving the position anyway would be
    /// indistinguishable from the minimise defect `max_scroll` records.
    pub fn scroll_by_rows(&mut self, delta_rows: i64) {
        if self.viewport_px <= 0.0 {
            return;
        }
        self.scroll = step(self.scroll.row, delta_rows, self.scroll.sub_row_px);
        self.clamp();
    }

    /// Puts `row` at the top of the viewport, with no partial row above it.
    pub fn scroll_to_row(&mut self, row: u64) {
        self.scroll = Scroll {
            row,
            sub_row_px: 0.0,
        };
        self.clamp();
    }

    /// Scrolls so the last row sits on the viewport's bottom edge — what following a file does.
    pub fn scroll_to_bottom(&mut self) {
        self.scroll = self.max_scroll();
        self.clamp();
    }

    /// True when the viewport is against the end, which is the condition for staying in follow mode.
    pub fn is_at_bottom(&self) -> bool {
        self.scroll == self.max_scroll()
    }

    /// Whole rows that fit in the viewport — a page, for `PageUp`/`PageDown`.
    ///
    /// One less than fits, so a page keeps a row of context; never zero.
    pub fn page_rows(&self) -> u64 {
        ((self.viewport_px / self.row_height).floor() as u64)
            .saturating_sub(1)
            .max(1)
    }

    /// Every row touching the viewport, top to bottom, with its viewport-relative y.
    ///
    /// **Rule 2 lives here.** `y` comes from the loop counter — the small integer `row − top_row` —
    /// and never from `row * row_height` minus an offset. The first row's `y` is `-sub_row_px`,
    /// which is in `(-row_height, 0]` by [`Scroll`]'s own invariant, so the property §6.4 asks CI to
    /// assert holds by construction.
    pub fn visible(&self) -> impl Iterator<Item = PlacedRow> + '_ {
        let remaining = self.total_rows.saturating_sub(self.scroll.row);
        let fit = if self.viewport_px <= 0.0 {
            0
        } else {
            ((self.viewport_px + self.scroll.sub_row_px) / self.row_height).ceil() as u64
        };
        (0..remaining.min(fit)).map(move |i| PlacedRow {
            row: self.scroll.row + i,
            y: i as f32 * self.row_height - self.scroll.sub_row_px,
        })
    }

    /// The row a viewport-relative y lands on, or `None` outside the drawn rows.
    ///
    /// **`y` is viewport-relative, not screen-absolute** — rule 3. A caller holding a window-space
    /// point subtracts the viewport origin *first*, in whatever space that is, and passes the small
    /// number here.
    ///
    /// **Bounded at both edges, and the lower one was missing.** Only the top was guarded at first,
    /// so a `y` below the viewport happily returned a row that had never been drawn — `row_at_y(900)`
    /// on an 800 px viewport answered row 47 when the last drawn row was 42. A drag-select that
    /// leaves the bottom of the grid is exactly how a caller reaches that, and it would select rows
    /// nobody can see.
    pub fn row_at_y(&self, y: f32) -> Option<u64> {
        // `is_nan` spelled out rather than relying on a negated comparison: a NaN y must hit
        // nothing, and `!(y < viewport)` says so only by accident of IEEE semantics.
        if y.is_nan() || y >= self.viewport_px {
            return None;
        }
        let offset = (y + self.scroll.sub_row_px) / self.row_height;
        if offset < 0.0 {
            return None;
        }
        let row = self.scroll.row.checked_add(offset.floor() as u64)?;
        (row < self.total_rows).then_some(row)
    }

    /// Where the scrollbar thumb sits, in `0.0..=1.0`.
    ///
    /// **This one is allowed to divide large numbers, and G7 exonerated it.** The forward mapping's
    /// `f32` error measured *exactly zero* at every row count tested up to 160M, because it divides
    /// two large numbers into a small range rather than differencing them. It is done in `f64`
    /// anyway, since `u64` row counts exceed `f32`'s integer precision long before they exceed the
    /// document sizes §11 cares about.
    ///
    /// The thumb has a real problem at that scale — at 160M rows one screen pixel addresses 200,000
    /// rows — but it is a *UI* problem, not a precision one, and it is not what #1391 reports.
    pub fn thumb_fraction(&self) -> f32 {
        let max = self.max_scroll();
        if max.row == 0 && max.sub_row_px == 0.0 {
            return 0.0;
        }
        let at = self.scroll.row as f64 + (self.scroll.sub_row_px / self.row_height) as f64;
        let full = max.row as f64 + (max.sub_row_px / self.row_height) as f64;
        (at / full).clamp(0.0, 1.0) as f32
    }

    /// Drags the thumb to a fraction of the document.
    ///
    /// A non-finite fraction is ignored. `NaN` is not exotic here: a scrollbar handler computing
    /// `y / track_height` produces one the first time it is asked before layout has given the track
    /// a height, and `NaN.clamp(0.0, 1.0)` is `NaN` rather than a bound.
    pub fn scroll_to_fraction(&mut self, fraction: f32) {
        if !fraction.is_finite() {
            return;
        }
        let max = self.max_scroll();
        let full = max.row as f64 + (max.sub_row_px / self.row_height) as f64;
        let at = full * fraction.clamp(0.0, 1.0) as f64;
        self.scroll = Scroll {
            row: at.floor() as u64,
            sub_row_px: (at.fract() as f32) * self.row_height,
        };
        self.clamp();
    }

    /// The furthest the viewport can go: the last row resting on the bottom edge.
    ///
    /// **Computed without ever forming the document height.** `total_rows * row_height` is the
    /// content-magnitude number this whole module exists to avoid, so the bottom is derived from
    /// how many rows fit in the viewport — a small number — and subtracted from the row count in
    /// integer space.
    /// **⚠ A zero-height viewport must not answer `TOP` here, and answering it was a real defect.**
    /// `TOP` is the *lower* bound, so [`clamp`](Self::clamp) read it as a hard cap and rewrote the
    /// position: minimising a window reports a 0×0 client area, and a reader tailing a 50M-line log
    /// came back at row 0 with follow silently off. A viewport with no height has no bottom to
    /// rest a row on, so the honest answer is that nothing constrains the position — the far end of
    /// the document — and the real bound returns with the window.
    fn max_scroll(&self) -> Scroll {
        if self.row_height <= 0.0 {
            return Scroll::TOP;
        }
        if self.viewport_px <= 0.0 {
            return Scroll {
                row: self.total_rows,
                sub_row_px: 0.0,
            };
        }
        let whole = (self.viewport_px / self.row_height).floor();
        let leftover_px = self.viewport_px - whole * self.row_height;
        let whole = whole as u64;

        if leftover_px <= 0.0 {
            return Scroll {
                row: self.total_rows.saturating_sub(whole),
                sub_row_px: 0.0,
            };
        }
        match self.total_rows.checked_sub(whole + 1) {
            Some(row) => Scroll {
                row,
                sub_row_px: self.row_height - leftover_px,
            },
            // The document is shorter than the viewport; there is nowhere to scroll to.
            None => Scroll::TOP,
        }
    }

    /// Whether the viewport is following the end *and* that means anything.
    ///
    /// **A grid with no rows or no viewport is vacuously at the bottom**, and treating that as
    /// "following" is wrong: a freshly built grid is at the top with nothing in it, so the first
    /// `set_total_rows` / `set_viewport_px` would pin it to the end and it would never be at the top
    /// again. Following is only a meaningful state once there is something to follow.
    /// **The viewport guard that used to be here threw the follow intent away on restore.** With
    /// [`max_scroll`](Self::max_scroll) now meaningful at zero height, `is_at_bottom` is a real
    /// question while minimised, and `total_rows > 0` is what actually protects the fresh-grid case
    /// this guard was written for.
    /// **Derived, never stored** — which is what makes "scrolling up auto-pauses follow"
    /// (`UI-DESIGN.md` §12) fall out of the scroll model rather than needing to be implemented. The
    /// shell reads it to drive the `⬤ Following` chip and to decide whether new rows should pull the
    /// view along.
    pub fn is_following(&self) -> bool {
        self.total_rows > 0 && self.is_at_bottom()
    }

    /// Re-clamps after a geometry change, re-pinning to the end if that is where we were.
    ///
    /// **A one-sided clamp is not enough, and that was a real defect.** Clamping only pulls the
    /// position *back* when it is past the end. Growing the viewport moves the end further down, so
    /// the old position is now past it and the clamp fires; **shrinking** the viewport moves the end
    /// *up*, nothing fires, and a viewport that was following the tail is left stranded above it —
    /// silently, with `is_at_bottom` flipping to false. Dragging a window shorter while tailing a
    /// file stopped showing the tail, which is §5.4's whole feature.
    fn settle(&mut self, following: bool) {
        if following {
            self.scroll = self.max_scroll();
        }
        self.clamp();
    }

    fn clamp(&mut self) {
        // A remainder that is not a number cannot be compared into range, and it poisons every
        // expression it reaches: `visible` computes an empty row count from it and the grid draws
        // nothing at all, permanently, because no later scroll can clear it either.
        if !self.scroll.sub_row_px.is_finite() {
            self.scroll.sub_row_px = 0.0;
        }
        let max = self.max_scroll();
        if (self.scroll.row, self.scroll.sub_row_px) > (max.row, max.sub_row_px) {
            self.scroll = max;
        }
        if self.scroll.sub_row_px < 0.0 {
            self.scroll.sub_row_px = 0.0;
        }
    }
}

/// A finite value, or `fallback` if it is `NaN` or infinite.
///
/// `f32::max` is not enough on its own: it neutralises `NaN` (`NaN.max(1.0) == 1.0`) but passes
/// `INFINITY` straight through, and an infinite row height makes `viewport − 0.0 * inf` a `NaN` that
/// ends up in the scroll position.
fn finite_or(value: f32, fallback: f32) -> f32 {
    if value.is_finite() {
        value
    } else {
        fallback
    }
}

/// Splits a pixel delta into whole rows and a remainder in `[0, row_height)`.
///
/// **A free function so the raw result can be tested.** The remainder is mathematically in band by
/// construction, but `f32` rounding can put it a hair outside, and the grid's `clamp` would then
/// silently rewrite it — which means a test reading `sub_row_px` off the grid cannot see whether
/// this arithmetic was ever out of band. It reads the band invariant off a value nothing has had a
/// chance to correct.
fn carry(sub_row_px: f32, delta_px: f32, row_height: f32) -> (i64, f32) {
    let carried = sub_row_px + delta_px;
    let mut rows = (carried / row_height).floor();
    let mut sub = carried - rows * row_height;
    // Rounding repair only. Both branches move the position by less than one ULP of `carried`.
    if sub < 0.0 {
        sub = 0.0;
    } else if sub >= row_height {
        sub -= row_height;
        rows += 1.0;
    }
    (rows as i64, sub)
}

/// Moves a scroll position by a signed row delta, saturating at row zero.
///
/// **The row and the remainder are clamped as a pair, and that is the whole reason this is a
/// function.** Saturating the row on its own leaves the remainder behind: scrolling 5,000 px up from
/// the top decomposes to −264 rows and +16 px, and clamping only the row lands on `(0, 16 px)` — a
/// *lower* position than the top of the document, with the first row drawn 16 px too high. Going
/// past the top has to snap to [`Scroll::TOP`] outright.
fn step(row: u64, delta_rows: i64, sub_row_px: f32) -> Scroll {
    match (row as i128) + (delta_rows as i128) {
        moved if moved < 0 => Scroll::TOP,
        moved => Scroll {
            row: moved.min(u64::MAX as i128) as u64,
            sub_row_px,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROW_H: f32 = 19.0;

    fn grid(total_rows: u64, viewport_px: f32) -> Grid {
        let mut g = Grid::new(ROW_H);
        g.set_total_rows(total_rows);
        g.set_viewport_px(viewport_px);
        g
    }

    /// **`SPEC.md` §6.4's required test, and it belongs in CI — which is where it now is.**
    ///
    /// "Assert that the first drawn row's viewport-relative y stays in `(−row_height, 0]` across a
    /// scroll sweep at 10⁸ rows. That one assertion catches all three." G7 measured egui's
    /// equivalent landing anywhere in a 512 px band at 160M rows while the scroll position advanced
    /// one exact row at a time.
    ///
    /// **⚠ The first-row assertion on its own does *not* catch rule 2 in this grid, and the spec's
    /// "that one assertion catches all three" is true of egui's architecture rather than of ours.**
    /// Reintroducing rule 2 faithfully — reconstructing a content-magnitude `content_top` and adding
    /// `row * row_height` back to it — leaves the **first** row exact, because with an exact
    /// `(u64, f32)` position the two content-magnitude terms are the *same expression* and cancel
    /// perfectly at `i = 0`. The damage lands on every row after it. egui's first row is misplaced
    /// only because its `offset` is an independently-rounded `f32`, which is rule 1's violation as
    /// well as rule 2's.
    ///
    /// So the sweep asserts **every visible row**, not just the first. Verified by mutation: the
    /// first-row-only form passed the faithful rule-2 reintroduction; the all-rows form fails it.
    #[test]
    fn the_first_row_stays_in_band_at_a_hundred_million_rows() {
        let mut g = grid(100_000_000, 800.0);
        g.scroll_to_fraction(0.9);

        for step in 0..4096 {
            g.scroll_by_px(ROW_H);
            let first = g.visible().next().expect("the viewport is not empty");
            assert!(
                first.y > -ROW_H && first.y <= 0.0,
                "step {step}: first row {} placed at y={}, outside (-{ROW_H}, 0]",
                first.row,
                first.y
            );

            // **Every** row, not only the first — see the note above.
            for (i, placed) in g.visible().enumerate() {
                let expected = i as f32 * ROW_H - first.y.abs();
                assert!(
                    (placed.y - expected).abs() < 0.001,
                    "step {step}: row {} (the {i}th drawn) is at y={}, expected {expected}",
                    placed.row,
                    placed.y
                );
            }
        }
    }

    /// The same sweep by single pixels rather than whole rows, which is what a drag does — and the
    /// arrangement in which egui discarded the motion entirely.
    #[test]
    fn a_two_pixel_drag_still_moves_at_a_hundred_million_rows() {
        let mut g = grid(100_000_000, 800.0);
        g.scroll_to_fraction(0.9);
        let before = g.scroll();

        // Sixty frames of a deliberate 2 px/frame drag: one second of it, expecting 120 px.
        for _ in 0..60 {
            g.scroll_by_px(2.0);
        }
        let after = g.scroll();

        let moved_rows = after.row - before.row;
        let moved_px = moved_rows as f32 * ROW_H + (after.sub_row_px - before.sub_row_px);
        assert!(
            (moved_px - 120.0).abs() < 0.01,
            "a 120 px drag moved {moved_px} px at 100M rows (egui moved 0.0)"
        );
    }

    /// A wheel notch must be the same size wherever you are in the document. G7 measured egui's
    /// changing gear — 53, then 48, then 64 px per notch — and then stopping altogether.
    #[test]
    fn a_wheel_notch_is_the_same_size_everywhere_in_the_document() {
        const NOTCH: f32 = 53.0;
        for total_rows in [1_000u64, 1_000_000, 16_000_000, 100_000_000, 160_000_000] {
            let mut g = grid(total_rows, 800.0);
            for fraction in [0.1f32, 0.5, 0.9] {
                g.scroll_to_fraction(fraction);
                let before = g.scroll();
                g.scroll_by_px(NOTCH);
                let after = g.scroll();

                let moved = (after.row - before.row) as f32 * ROW_H
                    + (after.sub_row_px - before.sub_row_px);
                assert!(
                    (moved - NOTCH).abs() < 0.01,
                    "{total_rows} rows at {fraction}: a {NOTCH} px notch moved {moved} px"
                );
            }
        }
    }

    /// Rows are laid out consecutively with no gap and no overlap, at any document size. This is
    /// rule 2's observable consequence: the spacing comes from the loop counter, so it cannot drift.
    #[test]
    fn visible_rows_are_consecutive_and_evenly_spaced() {
        for total_rows in [10u64, 1_000_000, 160_000_000] {
            let mut g = grid(total_rows, 800.0);
            g.scroll_to_fraction(0.5);
            let rows: Vec<PlacedRow> = g.visible().collect();
            assert!(rows.len() > 1);

            for pair in rows.windows(2) {
                assert_eq!(
                    pair[1].row,
                    pair[0].row + 1,
                    "{total_rows}: a row was skipped"
                );
                assert!(
                    (pair[1].y - pair[0].y - ROW_H).abs() < 0.001,
                    "{total_rows}: rows {} and {} are {} apart, not {ROW_H}",
                    pair[0].row,
                    pair[1].row,
                    pair[1].y - pair[0].y
                );
            }
        }
    }

    /// The viewport is covered top to bottom — no blank strip at either edge.
    #[test]
    fn the_visible_rows_cover_the_whole_viewport() {
        let mut g = grid(1_000_000, 800.0);
        g.scroll_to_fraction(0.5);
        let rows: Vec<PlacedRow> = g.visible().collect();

        let first = rows.first().expect("non-empty");
        let last = rows.last().expect("non-empty");
        assert!(first.y <= 0.0, "a gap above the first row: {}", first.y);
        assert!(
            last.y + ROW_H >= 800.0,
            "a gap below the last row: it ends at {}",
            last.y + ROW_H
        );
    }

    /// Hit-test is the inverse of layout. Clicking anywhere inside a drawn row must return that row.
    #[test]
    fn a_click_lands_on_the_row_that_was_drawn_there() {
        let mut g = grid(160_000_000, 800.0);
        g.scroll_to_fraction(0.73);
        g.scroll_by_px(7.0);

        for placed in g.visible() {
            for probe in [0.0f32, 1.0, ROW_H / 2.0, ROW_H - 0.5] {
                let y = placed.y + probe;
                if !(0.0..800.0).contains(&y) {
                    continue;
                }
                assert_eq!(
                    g.row_at_y(y),
                    Some(placed.row),
                    "y={y} is inside row {} but hit-tested elsewhere",
                    placed.row
                );
            }
        }
    }

    #[test]
    fn a_click_above_the_viewport_or_past_the_last_row_hits_nothing() {
        let g = grid(3, 800.0);
        assert_eq!(g.row_at_y(-1.0), None);
        assert_eq!(g.row_at_y(0.0), Some(0));
        assert_eq!(g.row_at_y(2.0 * ROW_H), Some(2));
        assert_eq!(g.row_at_y(3.0 * ROW_H), None, "past the last row");
        assert_eq!(g.row_at_y(700.0), None);
    }

    /// **The row and the remainder have to be clamped together.** 5,000 px up from the top
    /// decomposes to −264 rows and +16 px at this row height; saturating only the row lands on
    /// `(0, 16 px)`, which draws row 0 sixteen pixels above where it belongs and is a *lower*
    /// position than the top of the document. The first version of this module did exactly that.
    #[test]
    fn scrolling_stops_at_the_top() {
        for overshoot in [-5000.0f32, -19.0, -0.5, -1e9] {
            let mut g = grid(1000, 800.0);
            g.scroll_by_px(overshoot);
            assert_eq!(g.scroll(), Scroll::TOP, "overshooting by {overshoot} px");
            assert_eq!(g.visible().next().expect("rows").row, 0);
            assert_eq!(g.visible().next().expect("rows").y, 0.0);
        }

        // And by whole rows, which takes the same path.
        let mut g = grid(1000, 800.0);
        g.scroll_to_row(10);
        g.scroll_by_rows(-500);
        assert_eq!(g.scroll(), Scroll::TOP);
    }

    /// Scrolling down and back up by the same amount returns to where it started, at any size.
    /// A clamp that fired when it should not would show up here as drift.
    #[test]
    fn scrolling_down_and_back_up_returns_to_the_same_place() {
        let mut g = grid(160_000_000, 800.0);
        g.scroll_to_fraction(0.5);
        let before = g.scroll();

        for delta in [53.0f32, 2.0, 19.0, 0.25, 800.0] {
            g.scroll_by_px(delta);
            g.scroll_by_px(-delta);
            assert_eq!(
                g.scroll().row,
                before.row,
                "a {delta} px round trip moved the row"
            );
            assert!(
                (g.scroll().sub_row_px - before.sub_row_px).abs() < 0.001,
                "a {delta} px round trip left the remainder at {}",
                g.scroll().sub_row_px
            );
        }
    }

    /// The bottom clamp puts the last row on the bottom edge — not past it, and not short of it.
    #[test]
    fn scrolling_stops_with_the_last_row_on_the_bottom_edge() {
        let mut g = grid(1000, 800.0);
        g.scroll_by_px(1e9);
        let last = g.visible().last().expect("rows");
        assert_eq!(last.row, 999);
        assert!(
            (last.y + ROW_H - 800.0).abs() < 0.001,
            "the last row ends at {} rather than the viewport bottom",
            last.y + ROW_H
        );
    }

    /// A document shorter than the viewport has nowhere to scroll, and must not be draggable into
    /// empty space above its own first row.
    #[test]
    fn a_document_shorter_than_the_viewport_does_not_scroll() {
        let mut g = grid(3, 800.0);
        g.scroll_by_px(500.0);
        assert_eq!(g.scroll(), Scroll::TOP);
        assert_eq!(g.visible().count(), 3);
        assert!(g.is_at_bottom());
    }

    #[test]
    fn an_empty_document_has_no_rows_and_no_panic() {
        let g = grid(0, 800.0);
        assert_eq!(g.visible().count(), 0);
        assert_eq!(g.row_at_y(0.0), None);
        assert_eq!(g.thumb_fraction(), 0.0);
        assert!(g.is_at_bottom());
    }

    #[test]
    fn a_zero_height_viewport_shows_nothing_and_does_not_divide_by_zero() {
        let mut g = grid(1000, 0.0);
        assert_eq!(g.visible().count(), 0);
        g.scroll_by_px(100.0);
        assert_eq!(g.scroll(), Scroll::TOP);
    }

    /// Following a file: the view is pinned to the bottom, and stays pinned as rows arrive.
    #[test]
    fn following_stays_at_the_bottom_as_rows_arrive() {
        let mut g = grid(1000, 800.0);
        g.scroll_to_bottom();
        assert!(g.is_at_bottom());

        for total in 1001..1100 {
            g.set_total_rows(total);
            g.scroll_to_bottom();
            assert!(g.is_at_bottom());
            assert_eq!(
                g.visible().last().expect("rows").row,
                total - 1,
                "the newest row should be the one on screen"
            );
        }
    }

    /// **Shrinking the window must not silently stop the tail.** A one-sided clamp only pulls back
    /// when the position is *past* the end; shrinking moves the end up instead, so nothing fired and
    /// a viewport that was following was left stranded above it. Dragging an 800 px window to 600 px
    /// while tailing left the newest 10 rows off screen, with `is_at_bottom` quietly false.
    #[test]
    fn a_window_resize_does_not_break_following() {
        for viewport in [1000.0f32, 900.0, 800.0, 700.0, 600.0, 400.0, 137.0] {
            let mut g = grid(1000, 800.0);
            g.scroll_to_bottom();
            assert!(g.is_at_bottom());

            g.set_viewport_px(viewport);
            assert!(
                g.is_at_bottom(),
                "resizing 800 -> {viewport} px stopped following"
            );
            assert_eq!(
                g.visible().last().expect("rows").row,
                999,
                "resizing 800 -> {viewport} px lost the newest row"
            );
        }
    }

    /// **Minimising the window must not lose the reading position, or follow.** A `WM_SIZE` on
    /// minimise reports a 0×0 client area, so this sequence happens to every user of every Windows
    /// app. It used to destroy both: `max_scroll` answered `Scroll::TOP` at zero height, `clamp`
    /// read that lower bound as a cap and rewrote the position to row 0, and on restore
    /// `is_following` had already been false-d out by its own viewport guard, so nothing re-pinned.
    /// Tailing a 50M-line log, minimise, restore, and you were at the top of the file with follow
    /// silently off.
    #[test]
    fn minimising_and_restoring_keeps_the_position_and_the_follow_state() {
        // Following the tail.
        let mut g = grid(1_000_000, 800.0);
        g.scroll_to_bottom();
        let tail = g.scroll();

        g.set_viewport_px(0.0);
        assert_eq!(g.visible().count(), 0, "a minimised window draws nothing");
        g.set_viewport_px(800.0);

        assert!(g.is_at_bottom(), "restoring stopped following the tail");
        assert_eq!(g.scroll(), tail, "restoring moved the view");
        assert_eq!(g.visible().last().expect("rows").row, 999_999);

        // And a reader who had scrolled up keeps their place rather than being taken anywhere.
        let mut g = grid(1_000_000, 800.0);
        g.scroll_to_row(400_000);
        let reading = g.scroll();
        g.set_viewport_px(0.0);
        g.set_viewport_px(800.0);
        assert_eq!(g.scroll(), reading, "restoring lost the reading position");
        assert!(!g.is_at_bottom());
    }

    /// **And the log keeps being written while the window is minimised**, which is the whole point
    /// of a tail tool: the interesting case is precisely the one where you are not looking.
    ///
    /// This is the sequence that makes the `is_following` half of the minimise fix load-bearing.
    /// Without it, `is_following` is false while the viewport is zero, so rows arriving during the
    /// minimise do not move the pinned position; the view then restores to wherever the tail *was*
    /// when the window went away, with follow off, and the newest lines are off screen. The
    /// position-only fix does not catch it — the clamp has nothing to pull back, because the
    /// position is short of the end rather than past it.
    #[test]
    fn a_file_that_grows_while_minimised_is_still_followed_on_restore() {
        let mut g = grid(1_000_000, 800.0);
        g.scroll_to_bottom();

        g.set_viewport_px(0.0);
        for total in [1_000_100u64, 1_000_400, 1_002_000] {
            g.set_total_rows(total);
        }
        g.set_viewport_px(800.0);

        assert!(
            g.is_at_bottom(),
            "the tail was lost while the window was down"
        );
        assert_eq!(
            g.visible().last().expect("rows").row,
            1_001_999,
            "restored short of the newest row"
        );
    }

    /// The same for a font-size or DPI change, which moves the end for the same reason.
    #[test]
    fn a_row_height_change_does_not_break_following() {
        for row_height in [10.0f32, 19.0, 24.0, 28.5, 38.0] {
            let mut g = grid(1000, 800.0);
            g.scroll_to_bottom();

            g.set_row_height(row_height);
            assert!(
                g.is_at_bottom(),
                "a row height of {row_height} stopped following"
            );
            assert_eq!(g.visible().last().expect("rows").row, 999);
        }
    }

    /// But a reader who has scrolled up is **not** dragged to the end by an arriving line, or by a
    /// resize. Sticky-follow that is not conditional is just as broken as no follow at all.
    #[test]
    fn a_reader_who_scrolled_up_is_left_where_they_are() {
        let mut g = grid(1000, 800.0);
        g.scroll_to_row(400);
        let before = g.scroll();

        g.set_total_rows(1200);
        assert_eq!(g.scroll(), before, "an arriving line yanked the view");
        g.set_viewport_px(600.0);
        assert_eq!(g.scroll(), before, "a resize yanked the view");
        assert!(!g.is_at_bottom());
    }

    /// A freshly built grid is at the **top**, not the bottom. It is vacuously "at the bottom" while
    /// it has no rows and no viewport, and an unconditional sticky-follow read that as intent and
    /// pinned every new grid to the end of its document.
    #[test]
    fn a_new_grid_starts_at_the_top() {
        let g = grid(1000, 800.0);
        assert_eq!(g.scroll(), Scroll::TOP);
        assert_eq!(g.visible().next().expect("rows").row, 0);

        // In either assignment order.
        let mut g = Grid::new(ROW_H);
        g.set_viewport_px(800.0);
        g.set_total_rows(1000);
        assert_eq!(g.scroll(), Scroll::TOP);
    }

    /// **A single `NaN` must not be able to blank the grid for good.** `NaN.clamp(0.0, 1.0)` is
    /// `NaN`, so it reached `sub_row_px`; from there `visible()` computed a row count of zero and
    /// drew nothing, `is_at_bottom` was false for ever, and no later scroll could clear it because
    /// `NaN + delta` is `NaN`. A scrollbar handler dividing by a not-yet-laid-out track height
    /// produces one on the first frame.
    #[test]
    fn a_non_finite_input_cannot_poison_the_scroll_position() {
        for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let mut g = grid(1000, 800.0);
            g.scroll_to_row(500);

            g.scroll_to_fraction(bad);
            assert!(
                g.scroll().sub_row_px.is_finite(),
                "scroll_to_fraction({bad})"
            );
            g.scroll_by_px(bad);
            assert!(g.scroll().sub_row_px.is_finite(), "scroll_by_px({bad})");
            g.set_row_height(bad);
            assert!(g.row_height().is_finite(), "set_row_height({bad})");
            assert!(g.scroll().sub_row_px.is_finite(), "set_row_height({bad})");
            g.set_viewport_px(bad);
            assert!(g.viewport_px().is_finite(), "set_viewport_px({bad})");

            // And the grid still works afterwards.
            g.set_viewport_px(800.0);
            g.set_row_height(ROW_H);
            g.scroll_by_px(53.0);
            assert!(
                g.visible().next().is_some(),
                "after {bad} the grid drew nothing at all"
            );
            assert!(g.thumb_fraction().is_finite());
        }
    }

    /// Hit-test is bounded at the **bottom** as well as the top. Only the top was guarded at first,
    /// so a y below the viewport returned a row that had never been drawn — which a drag-select
    /// leaving the bottom of the grid produces immediately.
    #[test]
    fn a_click_below_the_viewport_hits_nothing() {
        let g = grid(1_000_000, 800.0);
        let last = g.visible().last().expect("rows");
        assert!(g.row_at_y(799.0).is_some());
        assert_eq!(g.row_at_y(800.0), None, "the row below the viewport edge");
        assert_eq!(g.row_at_y(900.0), None);
        assert_eq!(g.row_at_y(5000.0), None);
        assert!(
            last.row <= 42,
            "the fixture assumes a 43-row viewport, got {}",
            last.row
        );
    }

    /// Every y outside the drawn rows must hit nothing, and every y inside must hit the row that
    /// was drawn there — the round trip in both directions, not just outwards from `visible`.
    #[test]
    fn hit_test_is_exactly_the_inverse_of_layout() {
        let mut g = grid(160_000_000, 800.0);
        g.scroll_to_fraction(0.61);
        g.scroll_by_px(3.0);

        let drawn: std::collections::BTreeMap<u64, f32> =
            g.visible().map(|p| (p.row, p.y)).collect();
        let mut y = -2.0f32 * ROW_H;
        while y < 800.0 + 2.0 * ROW_H {
            match g.row_at_y(y) {
                Some(row) => {
                    let top = *drawn
                        .get(&row)
                        .unwrap_or_else(|| panic!("y={y} hit row {row}, which was never drawn"));
                    assert!(
                        y >= top && y < top + ROW_H,
                        "y={y} hit row {row}, drawn at {top}..{}",
                        top + ROW_H
                    );
                }
                None => assert!(
                    !(0.0..800.0).contains(&y),
                    "y={y} is inside the viewport but hit nothing"
                ),
            }
            y += 0.37;
        }
    }

    /// **The band invariant, read off the raw arithmetic.** Reading `sub_row_px` back off the grid
    /// cannot see this: `clamp` rewrites an out-of-band remainder before any test can look at it, so
    /// the assertion was guarding a value that had already been repaired. `carry` returns the
    /// unrepaired result.
    #[test]
    fn the_carry_never_leaves_the_band() {
        let deltas = [
            0.5f32, -0.5, 19.0, -19.0, 1e6, -1e6, 0.001, -0.001, 53.0, -53.0, 1e9, -1e9, 3.8e8,
            -3.8e8, 1e-7, 18.999998,
        ];
        for row_height in [1.0f32, 16.0, 19.0, 28.5] {
            for sub in [0.0f32, 0.5, row_height * 0.5, row_height - 0.001] {
                for delta in deltas {
                    let (_, out) = carry(sub, delta, row_height);
                    assert!(
                        (0.0..row_height).contains(&out),
                        "carry({sub}, {delta}, {row_height}) gave {out}, outside [0, {row_height})"
                    );
                }
            }
        }
    }

    /// A file that was truncated (§5.5) must not leave the viewport stranded past the new end.
    #[test]
    fn a_document_that_shrank_pulls_the_viewport_back() {
        let mut g = grid(1_000_000, 800.0);
        g.scroll_to_bottom();
        g.set_total_rows(50);

        assert!(
            g.scroll().row < 50,
            "the viewport is past the end of the file"
        );
        assert_eq!(g.visible().last().expect("rows").row, 49);
    }

    /// The thumb is monotonic and reaches both ends. G7 exonerated the forward mapping as a
    /// precision fault, so what is worth asserting is that it is *usable*: 0 at the top, 1 at the
    /// bottom, and never going backwards on the way.
    #[test]
    fn the_thumb_runs_from_zero_to_one_without_going_backwards() {
        for total_rows in [1_000u64, 160_000_000] {
            let mut g = grid(total_rows, 800.0);
            g.scroll_to_row(0);
            assert_eq!(g.thumb_fraction(), 0.0, "{total_rows}: not zero at the top");

            let mut previous = 0.0;
            for i in 0..=100 {
                g.scroll_to_fraction(i as f32 / 100.0);
                let thumb = g.thumb_fraction();
                assert!(
                    thumb >= previous - 0.001,
                    "{total_rows}: the thumb went backwards, {previous} then {thumb}"
                );
                previous = thumb;
            }

            g.scroll_to_bottom();
            assert!(
                (g.thumb_fraction() - 1.0).abs() < 0.001,
                "{total_rows}: the thumb is {} at the bottom",
                g.thumb_fraction()
            );
        }
    }

    /// Dragging the thumb and reading it back must agree, or the thumb jumps under the cursor.
    #[test]
    fn the_thumb_round_trips() {
        let mut g = grid(160_000_000, 800.0);
        for percent in 0..=100 {
            let asked = percent as f32 / 100.0;
            g.scroll_to_fraction(asked);
            assert!(
                (g.thumb_fraction() - asked).abs() < 0.001,
                "asked for {asked}, the thumb reads {}",
                g.thumb_fraction()
            );
        }
    }

    /// A DPI change keeps the view on the same row rather than the same pixel — the vertical
    /// counterpart of §3.3's "no column drift between a 100% and a 150% monitor".
    #[test]
    fn a_dpi_change_keeps_the_top_row() {
        let mut g = grid(1_000_000, 800.0);
        g.scroll_to_row(500_000);
        g.scroll_by_px(ROW_H / 2.0);
        let before = g.scroll();

        g.set_row_height(ROW_H * 1.5);
        assert_eq!(
            g.scroll().row,
            before.row,
            "the top row moved on a DPI change"
        );
        assert!(
            (g.scroll().sub_row_px / g.row_height() - before.sub_row_px / ROW_H).abs() < 0.001,
            "the part-scrolled row was not scaled with the row height"
        );
    }

    #[test]
    fn a_page_leaves_a_row_of_context_and_is_never_zero() {
        assert_eq!(grid(1000, 800.0).page_rows(), 41);
        assert_eq!(
            grid(1000, 0.0).page_rows(),
            1,
            "a page must never be zero rows"
        );
        assert_eq!(grid(1000, ROW_H).page_rows(), 1);
    }

    /// The invariant every other test rests on: `sub_row_px` never leaves `[0, row_height)`, however
    /// it was arrived at. If it ever did, the first row would be drawn outside its band.
    #[test]
    fn the_sub_row_remainder_never_leaves_its_band() {
        let mut g = grid(160_000_000, 800.0);
        let deltas = [
            0.5f32, -0.5, 19.0, -19.0, 1e6, -1e6, 0.001, -0.001, 53.0, -53.0, 1e9, -1e9,
        ];
        for (i, delta) in deltas.iter().cycle().take(2000).enumerate() {
            g.scroll_by_px(*delta);
            let sub = g.scroll().sub_row_px;
            assert!(
                (0.0..ROW_H).contains(&sub),
                "after {i} deltas the remainder is {sub}, outside [0, {ROW_H})"
            );
        }
    }
}

#[cfg(test)]
mod follow_frame_tests {
    use super::*;

    /// The shell re-measures the face and re-sets the geometry **every frame** — `Shell::paint`
    /// does that deliberately, so a DPI change between frames cannot be missed. Following has to
    /// survive being told the same numbers again.
    #[test]
    fn following_survives_the_frame_loop_setting_the_same_geometry_again() {
        let mut g = Grid::new(19.0);
        g.set_viewport_px(1466.0);
        g.set_total_rows(5000);
        g.scroll_to_bottom();
        assert!(
            g.is_at_bottom(),
            "scroll_to_bottom must arrive at the bottom"
        );

        for frame in 0..5 {
            g.set_row_height(19.0);
            g.set_viewport_px(1466.0);
            g.set_total_rows(5000);
            assert!(
                g.is_at_bottom(),
                "stopped following on frame {frame} with nothing changed"
            );
        }
    }

    /// The same, with a viewport height whose division by the row height does not land on a bit
    /// boundary — which is the ordinary case, not a contrived one.
    #[test]
    fn following_survives_a_viewport_that_does_not_divide_evenly() {
        for (row_h, viewport) in [
            (19.0, 1466.0),
            (18.7, 1439.0),
            (21.3, 1073.5),
            (16.0, 900.0),
        ] {
            let mut g = Grid::new(row_h);
            g.set_viewport_px(viewport);
            g.set_total_rows(5000);
            g.scroll_to_bottom();
            assert!(
                g.is_at_bottom(),
                "{row_h}/{viewport}: not at bottom after scroll_to_bottom"
            );
            g.set_row_height(row_h);
            g.set_viewport_px(viewport);
            assert!(
                g.is_at_bottom(),
                "{row_h}/{viewport}: lost the bottom when the frame re-set the same geometry"
            );
        }
    }
}
