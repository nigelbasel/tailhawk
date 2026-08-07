//! The horizontal half of the viewport — V3's last piece.
//!
//! Portable and device-free like [`crate::grid`], and deliberately **not** the same model. `SPEC.md`
//! §6.4 cut `--wrap` from v1 in favour of scrolling long lines sideways, and §3.3 captured the
//! extent this needs during the index scan. This module turns that extent into: which columns are on
//! screen, where each one is drawn, and which column a click landed on.
//!
//! ## The two axes are not symmetric, and the difference is load-bearing
//!
//! [`crate::grid::Scroll`] cannot represent a content-pixel offset, because a document has up to 2⁶⁴
//! rows and G7 measured what an `f32` content coordinate does at 10⁸ of them. **The horizontal axis
//! is bounded**, so the same rule does not apply and a plain pixel offset is the honest model here.
//!
//! What bounds it is `SPEC.md` §10.3's **32 KB inline render cap**. No encoding produces more cells
//! than bytes (§3.3), so a rendered line is at most [`RENDER_CAP_CELLS`] cells wide however long the
//! line in the file is — a 41 MB single line is truncated with an `expand · open in viewer`
//! affordance rather than scrolled to its end. [`HGrid::set_columns`] enforces that cap, and
//! [`MAX_CELL_WIDTH_PX`] bounds the other factor, so the widest content this module can form is
//! under 2²⁴ px and **every column position is an exactly-representable `f32` integer**.
//!
//! **That is a dependency, not a coincidence.** If the render cap is ever raised to "unlimited", the
//! content width leaves `f32`'s exact range and §6.4's `(index, remainder)` model has to come back on
//! this axis too. `the_render_cap_is_what_keeps_every_column_exact` is the assertion that fails.
//!
//! ## The offset is stored exactly and drawn snapped
//!
//! Column advances are whole device pixels (`SPEC.md` §3.1), and glyphs are drawn from an atlas
//! rasterised at a fixed pixel phase. Placing that bitmap at a fractional x resamples ClearType's
//! **horizontal** subpixel coverage, which is colour fringing rather than blur — so layout rounds the
//! offset to a whole pixel. It rounds it **once**, shared by every column, so the spacing between
//! columns stays exactly one cell width.
//!
//! Rounding the *stored* offset instead would be the other, worse bug: a trackpad delivering 0.4 px
//! per frame would round to zero every frame and the view would never move at all. So the exact
//! offset accumulates and only [`HGrid::x_of_column`] and its inverse see the snapped one.

use core::ops::Range;

/// The widest a rendered line can be, in cells.
///
/// `SPEC.md` §10.3 caps inline rendering at **32 KB**, and §3.3 establishes that no encoding produces
/// more cells than bytes — so this is the byte cap's exact implication for the cell grid. Content
/// past it is reached through the record detail pane, not by scrolling.
pub const RENDER_CAP_CELLS: usize = 32 * 1024;

/// The widest a cell may be, in device pixels.
///
/// Not a style limit: `RENDER_CAP_CELLS * MAX_CELL_WIDTH_PX` is the largest content width this module
/// can form, and at 512 px it is 2²⁴ — the last integer `f32` represents exactly. A cell twice that
/// wide would make column positions drift by a pixel at the right-hand end of a capped line.
pub const MAX_CELL_WIDTH_PX: f32 = 512.0;

/// The horizontal viewport over a document's widest line.
///
/// Columns are [`crate::cell`]'s cells and are `usize`, matching [`crate::selection::Position`]: a
/// row is a `u64` because a document is unbounded, a column is a `usize` because a line is not.
#[derive(Clone, Debug)]
pub struct HGrid {
    cell_width: f32,
    viewport_px: f32,
    columns: usize,
    /// Exact, unsnapped. See the module note — this accumulates sub-pixel deltas that the drawn
    /// position rounds away.
    offset_px: f32,
}

impl HGrid {
    /// `cell_width` is the advance of one cell in whole device pixels — `SPEC.md` §3.1 requires it be
    /// an integer at the current scale and re-derived on any scale change, because fractional
    /// per-glyph rounding accumulates drift and visibly misaligns columns across a wide window.
    pub fn new(cell_width: f32) -> Self {
        Self {
            cell_width: sane_cell_width(cell_width),
            viewport_px: 0.0,
            columns: 0,
            offset_px: 0.0,
        }
    }

    pub fn cell_width(&self) -> f32 {
        self.cell_width
    }

    pub fn viewport_px(&self) -> f32 {
        self.viewport_px
    }

    /// The widest line, in cells — capped at [`RENDER_CAP_CELLS`].
    pub fn columns(&self) -> usize {
        self.columns
    }

    /// How far the content is scrolled left, in pixels. Exact; the drawn offset is `-x_of_column(0)`.
    pub fn offset_px(&self) -> f32 {
        self.offset_px
    }

    /// The body viewport's width. **The gutter and the line-number column are not part of it** — a
    /// caller subtracts those first, exactly as [`crate::grid::Grid::row_at_y`] takes a
    /// viewport-relative y rather than a window one.
    pub fn set_viewport_px(&mut self, viewport_px: f32) {
        self.viewport_px = finite_or(viewport_px, 0.0).max(0.0);
        self.clamp();
    }

    /// Sets the extent, in cells, from `SPEC.md` §3.3's index-time measurement.
    ///
    /// The caller derives it from the index: `extent.exact_cells(charset)` where that is `Some`, and
    /// [`Extent::max_line_bytes`](crate::index::Extent::max_line_bytes) — an upper bound, since no
    /// encoding produces more cells than bytes — where it is not. It takes a `u64` for that reason:
    /// a byte count off a 10 GB file is not a column count until it has been capped.
    ///
    /// **⚠ Never derive this from the lines currently on screen.** §3.3 says so outright, and the
    /// symptom is the horizontal-thumb jitter every other viewer has: the trough resizes on every
    /// vertical scroll. Refining it downwards as blocks are laid out is fine and is what §3.3
    /// intends — the thumb may grow slightly during a long scroll, which nobody notices.
    pub fn set_columns(&mut self, columns: u64) {
        self.columns = usize::try_from(columns)
            .unwrap_or(usize::MAX)
            .min(RENDER_CAP_CELLS);
        self.clamp();
    }

    /// Changes the cell width — a DPI change, or a font-size change.
    ///
    /// **The leftmost column is kept, not the pixel offset.** Keeping pixels moves the view by a
    /// different number of columns at each scale, which is precisely the column drift §3.3's
    /// acceptance test drags a window between a 100% and a 150% monitor to catch.
    pub fn set_cell_width(&mut self, cell_width: f32) {
        let cell_width = sane_cell_width(cell_width);
        let columns_in = self.offset_px / self.cell_width;
        self.cell_width = cell_width;
        self.offset_px = columns_in * cell_width;
        self.clamp();
    }

    /// Scrolls by a pixel delta — `Shift+wheel`, or a horizontal trackpad gesture. Positive moves the
    /// view *right* along the line.
    pub fn scroll_by_px(&mut self, delta_px: f32) {
        if !delta_px.is_finite() {
            return;
        }
        self.offset_px += delta_px;
        self.clamp();
    }

    /// Scrolls by whole columns — the arrow keys.
    pub fn scroll_by_columns(&mut self, delta_columns: i64) {
        self.offset_px += delta_columns as f32 * self.cell_width;
        self.clamp();
    }

    /// Puts `column` against the left edge.
    pub fn scroll_to_column(&mut self, column: usize) {
        self.offset_px = column as f32 * self.cell_width;
        self.clamp();
    }

    /// `Home`.
    pub fn scroll_to_start(&mut self) {
        self.offset_px = 0.0;
        self.clamp();
    }

    /// `End`, for the document. `End` on a *line* is [`reveal`](Self::reveal) of that line's own last
    /// column — this module knows the extent, not any particular line's length.
    pub fn scroll_to_end(&mut self) {
        self.offset_px = self.max_offset_px();
    }

    /// Scrolls the least that brings `columns` into view — a search hit, or a caret leaving the edge.
    ///
    /// Already-visible columns do not move the view at all, which is what makes this safe to call on
    /// every keystroke. A range wider than the viewport aligns to its **start**: that is where the
    /// match begins, and it is the end a reader needs first.
    ///
    /// An empty range is a caret, and reveals the column it sits at.
    pub fn reveal(&mut self, columns: Range<usize>) {
        let start_px = columns.start as f32 * self.cell_width;
        let end_px = columns.end.max(columns.start + 1) as f32 * self.cell_width;
        let offset = self.snapped_offset();

        // `end_px - offset` before comparing to the viewport, never `offset + viewport` — the
        // difference of two content-magnitude values is exact where their sum is not.
        if end_px - start_px >= self.viewport_px || start_px < offset {
            self.offset_px = start_px;
        } else if end_px - offset > self.viewport_px {
            self.offset_px = end_px - self.viewport_px;
        } else {
            return;
        }
        self.clamp();
    }

    /// Every column touching the viewport, left to right.
    ///
    /// **⚠ A wide cluster straddling the left edge starts *before* `start`**, and painting from
    /// `start` would cut it in half. [`crate::cell::CellModel::byte_span`] is built for this — it
    /// rounds both ends outwards, so it returns the straddling cluster whole — and the painter then
    /// draws from that cluster's own column, which [`x_of_column`](Self::x_of_column) will happily
    /// place at a negative x.
    pub fn visible_columns(&self) -> Range<usize> {
        if self.viewport_px <= 0.0 || self.columns == 0 {
            return 0..0;
        }
        let (first, first_x) = self.first_visible();
        let across = ((self.viewport_px - first_x) / self.cell_width).ceil() as usize;
        first.min(self.columns)..first.saturating_add(across).min(self.columns)
    }

    /// A column's left edge, **relative to the body viewport's left**, in pixels.
    ///
    /// Negative for a column scrolled off the left edge, and deliberately unclamped: the painter of a
    /// straddling wide cluster needs exactly that. Always a whole pixel — see the module note.
    pub fn x_of_column(&self, column: usize) -> f32 {
        column as f32 * self.cell_width - self.snapped_offset()
    }

    /// The column a viewport-relative x lands on, or `None` outside the drawn columns.
    ///
    /// The exact inverse of [`x_of_column`](Self::x_of_column), and it must read the *same* snapped
    /// offset that laid the columns out: hit-testing against the unsnapped one puts a click within a
    /// pixel of a boundary on the wrong side of it.
    ///
    /// **⚠ `x + offset` is not the inverse, and this axis being bounded does not save it.** §6.4's
    /// rule 3 — never mix screen-space and content-space in one expression — is the one rule of the
    /// three that still applies here, and it bites at ordinary sizes: at a 96,624 px offset one ULP
    /// is 0.0078 px, so a click at `x = 231.99881` inside a column drawn at 232 rounds *up* to the
    /// boundary and hit-tests one column to the right. Both terms being exact is not enough when
    /// their sum is not. So the offset is resolved to the first visible column once, and the click is
    /// measured from there in small numbers.
    ///
    /// A drag that leaves the left or right edge hits nothing here, as it does vertically. Autoscroll
    /// is the input loop's, not the grid's.
    pub fn column_at_x(&self, x: f32) -> Option<usize> {
        if x.is_nan() || x >= self.viewport_px || self.columns == 0 {
            return None;
        }
        let (first, first_x) = self.first_visible();
        let across = (x - first_x) / self.cell_width;
        if across < 0.0 {
            return None;
        }
        let column = first.checked_add(across.floor() as usize)?;
        (column < self.columns).then_some(column)
    }

    /// The full content width in pixels — bounded, and exact, by the module note's argument.
    pub fn content_px(&self) -> f32 {
        self.columns as f32 * self.cell_width
    }

    /// The furthest left the content can be pulled. Zero when it fits.
    pub fn max_offset_px(&self) -> f32 {
        (self.content_px() - self.viewport_px).max(0.0)
    }

    /// Whether a horizontal scrollbar should be shown at all — `UI-DESIGN.md` §11 has it appear only
    /// when the content exceeds the viewport, rather than sitting there greyed out.
    pub fn overflows(&self) -> bool {
        self.max_offset_px() > 0.0
    }

    /// Where the thumb sits, in `0.0..=1.0`.
    ///
    /// **No `f64` here, unlike [`crate::grid::Grid::thumb_fraction`].** That one divides row counts
    /// which exceed `f32`'s integer precision long before they exceed the document sizes §11 cares
    /// about; these are pixel counts under 2²⁴, so the division is exact in the width it is done in.
    pub fn thumb_fraction(&self) -> f32 {
        let max = self.max_offset_px();
        if max <= 0.0 {
            return 0.0;
        }
        (self.offset_px / max).clamp(0.0, 1.0)
    }

    /// Drags the thumb to a fraction of the content.
    ///
    /// A non-finite fraction is ignored, for the reason [`crate::grid::Grid::scroll_to_fraction`]
    /// records: a handler computing `x / track_width` produces a `NaN` the first time it runs before
    /// layout has given the track a width, and `NaN.clamp(0.0, 1.0)` is `NaN` rather than a bound.
    pub fn scroll_to_fraction(&mut self, fraction: f32) {
        if !fraction.is_finite() {
            return;
        }
        self.offset_px = self.max_offset_px() * fraction.clamp(0.0, 1.0);
        self.clamp();
    }

    /// The offset the columns are actually drawn at: whole device pixels, rounded once.
    fn snapped_offset(&self) -> f32 {
        self.offset_px.round()
    }

    /// The leftmost column touching the viewport, and its viewport-relative x in `(-cell_width, 0]`.
    ///
    /// **This is where the content magnitude stops.** Everything downstream measures from here in
    /// numbers no larger than the viewport, which is §6.4's rule 3 — see
    /// [`column_at_x`](Self::column_at_x) for the click it was getting wrong.
    fn first_visible(&self) -> (usize, f32) {
        let offset = self.snapped_offset();
        let first = (offset / self.cell_width).floor();
        (first as usize, first * self.cell_width - offset)
    }

    fn clamp(&mut self) {
        if !self.offset_px.is_finite() {
            self.offset_px = 0.0;
        }
        self.offset_px = self.offset_px.clamp(0.0, self.max_offset_px());
    }
}

/// A cell width that keeps every column position exact — see [`MAX_CELL_WIDTH_PX`].
fn sane_cell_width(cell_width: f32) -> f32 {
    finite_or(cell_width, 1.0).clamp(1.0, MAX_CELL_WIDTH_PX)
}

/// A finite value, or `fallback`. `f32::max` neutralises `NaN` but passes `INFINITY` straight
/// through, and an infinite cell width makes the whole layout `NaN` one multiplication later.
fn finite_or(value: f32, fallback: f32) -> f32 {
    if value.is_finite() {
        value
    } else {
        fallback
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CELL_W: f32 = 8.0;

    fn hgrid(columns: u64, viewport_px: f32) -> HGrid {
        let mut h = HGrid::new(CELL_W);
        h.set_columns(columns);
        h.set_viewport_px(viewport_px);
        h
    }

    /// **The assertion the pixel-offset model rests on.** §6.4 forbids a content-pixel offset
    /// vertically because a document is unbounded; this axis may use one only because §10.3's render
    /// cap bounds it. If the cap is ever lifted, this fails and the `(index, remainder)` model has to
    /// come back here too.
    #[test]
    fn the_render_cap_is_what_keeps_every_column_exact() {
        let mut h = HGrid::new(MAX_CELL_WIDTH_PX);
        h.set_columns(u64::MAX);
        h.set_viewport_px(1920.0);

        assert_eq!(h.columns(), RENDER_CAP_CELLS, "the cap was not applied");
        assert_eq!(
            h.content_px(),
            16_777_216.0,
            "the widest content is 2^24 px exactly"
        );

        // Every column boundary in the capped content is an exactly-representable integer: adding
        // one cell width lands on a different f32, at both ends of the range.
        for column in [0usize, 1, RENDER_CAP_CELLS / 2, RENDER_CAP_CELLS - 1] {
            let x = column as f32 * MAX_CELL_WIDTH_PX;
            assert_eq!(x, x.trunc(), "column {column} is not on a whole pixel");
            assert_ne!(
                x + MAX_CELL_WIDTH_PX,
                x,
                "column {column} and the next one are the same f32"
            );
        }

        // And what the cap is holding back, stated without reference to this module: one pixel past
        // 2²⁴ is not an `f32`. Uncapped, a line wide enough to reach there has adjacent columns
        // sharing a position, and every click in that region lands on whichever one rounding picked.
        assert_eq!(16_777_217.0f32, 16_777_216.0f32);
    }

    /// Layout and hit-test are inverses across a sweep, at the widest content the cap allows and at a
    /// fractional offset — which is where a snapped layout and an unsnapped hit-test disagree.
    #[test]
    fn hit_test_is_exactly_the_inverse_of_layout() {
        let mut h = hgrid(RENDER_CAP_CELLS as u64, 1000.0);
        h.scroll_to_fraction(0.37);
        h.scroll_by_px(0.6);

        let visible = h.visible_columns();
        for column in visible.clone() {
            let left = h.x_of_column(column);
            for probe in [0.0f32, 0.5, CELL_W - 0.5] {
                let x = left + probe;
                if !(0.0..1000.0).contains(&x) {
                    continue;
                }
                assert_eq!(
                    h.column_at_x(x),
                    Some(column),
                    "x={x} is inside column {column}, drawn at {left}"
                );
            }
        }

        // And in the other direction: every x inside the viewport hits a column that was laid out
        // there, and nothing outside the content hits anything.
        let mut x = -2.0 * CELL_W;
        while x < 1000.0 + 2.0 * CELL_W {
            match h.column_at_x(x) {
                Some(column) => {
                    let left = h.x_of_column(column);
                    assert!(
                        x >= left && x < left + CELL_W,
                        "x={x} hit column {column}, drawn at {left}..{}",
                        left + CELL_W
                    );
                }
                None => assert!(
                    !(0.0..1000.0).contains(&x) || h.columns() == 0,
                    "x={x} is inside the viewport but hit nothing"
                ),
            }
            x += 0.31;
        }
    }

    /// **Every drawn column lands on a whole device pixel**, whatever sub-pixel offset a trackpad
    /// left behind. A fractional x resamples ClearType's horizontal subpixel coverage, which shows up
    /// as colour fringing on the glyph edges rather than as blur.
    #[test]
    fn every_drawn_column_lands_on_a_whole_device_pixel() {
        let mut h = hgrid(4_000, 1000.0);
        for nudge in [0.1f32, 0.4, 0.5, 0.9, 1.3, 7.7, 0.05] {
            h.scroll_by_px(nudge);
            for column in h.visible_columns() {
                let x = h.x_of_column(column);
                assert_eq!(
                    x,
                    x.trunc(),
                    "column {column} drawn at {x} after a {nudge} px nudge"
                );
            }
        }
    }

    /// And the spacing survives the snap: rounding **once**, shared by every column, is what keeps
    /// adjacent columns exactly one cell apart. Rounding each column's own x would put 7 px between
    /// some pairs and 8 between others.
    #[test]
    fn snapping_does_not_change_the_spacing_between_columns() {
        let mut h = hgrid(4_000, 1000.0);
        h.scroll_by_px(123.45);
        let columns: Vec<usize> = h.visible_columns().collect();
        for pair in columns.windows(2) {
            let gap = h.x_of_column(pair[1]) - h.x_of_column(pair[0]);
            assert_eq!(
                gap, CELL_W,
                "columns {} and {} are {gap} apart",
                pair[0], pair[1]
            );
        }
    }

    /// **A slow drag must still move.** Rounding the *stored* offset — the obvious way to get whole
    /// pixels — makes a trackpad delivering 0.4 px per frame round to zero every frame, for ever.
    #[test]
    fn a_sub_pixel_drag_accumulates_instead_of_vanishing() {
        let mut h = hgrid(4_000, 1000.0);
        for _ in 0..10 {
            h.scroll_by_px(0.4);
        }
        assert!(
            (h.offset_px() - 4.0).abs() < 0.001,
            "ten 0.4 px frames moved {} px",
            h.offset_px()
        );
        assert_eq!(h.x_of_column(0), -4.0, "and the drawn position followed");
    }

    /// A DPI or font-size change keeps the same **column** against the left edge, not the same pixel.
    /// Keeping pixels is §3.3's column drift, one axis over.
    #[test]
    fn a_dpi_change_keeps_the_leftmost_column() {
        for cell_width in [4.0f32, 8.0, 12.0, 13.0, 24.0] {
            let mut h = hgrid(4_000, 1000.0);
            h.scroll_to_column(500);
            h.set_cell_width(cell_width);
            assert_eq!(
                h.visible_columns().start,
                500,
                "a cell width of {cell_width} moved the leftmost column"
            );
        }
    }

    #[test]
    fn scrolling_stops_at_the_start_and_at_the_end() {
        let mut h = hgrid(4_000, 1000.0);
        h.scroll_by_px(-5000.0);
        assert_eq!(h.offset_px(), 0.0);
        assert_eq!(h.visible_columns().start, 0);
        assert_eq!(h.x_of_column(0), 0.0);

        h.scroll_by_px(1e9);
        assert_eq!(h.offset_px(), 4_000.0 * CELL_W - 1000.0);
        assert_eq!(
            h.visible_columns().end,
            4_000,
            "the last column is not against the right edge"
        );

        // By whole columns, which takes the same path.
        h.scroll_by_columns(-100_000);
        assert_eq!(h.offset_px(), 0.0);
    }

    #[test]
    fn content_narrower_than_the_viewport_does_not_scroll() {
        let mut h = hgrid(10, 1000.0);
        assert!(!h.overflows());
        h.scroll_by_px(500.0);
        assert_eq!(h.offset_px(), 0.0);
        assert_eq!(h.visible_columns(), 0..10);
        assert_eq!(h.thumb_fraction(), 0.0);
    }

    #[test]
    fn an_empty_document_has_no_columns_and_no_panic() {
        let h = hgrid(0, 1000.0);
        assert_eq!(h.visible_columns(), 0..0);
        assert_eq!(h.column_at_x(0.0), None);
        assert_eq!(h.thumb_fraction(), 0.0);
        assert!(!h.overflows());
    }

    #[test]
    fn a_zero_width_viewport_shows_nothing_and_does_not_divide_by_zero() {
        let mut h = hgrid(4_000, 0.0);
        assert_eq!(h.visible_columns(), 0..0);
        assert_eq!(h.column_at_x(0.0), None);
        h.scroll_by_px(100.0);
        assert!(h.offset_px().is_finite());
    }

    /// **A refined extent must pull the view back.** §3.3 has the bound start as an upper one and
    /// tighten as blocks are laid out, so the content genuinely narrows underneath a view that is
    /// already scrolled — and a clamp that only guards the near edge leaves it out in empty space,
    /// with the last column off the left of the window and nothing drawn at all.
    #[test]
    fn a_refined_extent_pulls_the_viewport_back() {
        let mut h = hgrid(4_000, 1000.0);
        h.scroll_to_end();
        assert!(h.offset_px() > 0.0);

        h.set_columns(400);
        assert_eq!(
            h.offset_px(),
            400.0 * CELL_W - 1000.0,
            "the view is past the end of the content"
        );
        assert_eq!(h.visible_columns().end, 400);

        // And down to something that fits entirely.
        h.set_columns(10);
        assert_eq!(h.offset_px(), 0.0);
        assert_eq!(h.visible_columns(), 0..10);
    }

    /// The same for a widening viewport, which moves the far edge for the same reason.
    #[test]
    fn a_window_resize_keeps_the_content_against_the_right_edge() {
        let mut h = hgrid(4_000, 1000.0);
        h.scroll_to_end();
        for viewport in [1200.0f32, 2000.0, 8000.0, 40_000.0] {
            h.set_viewport_px(viewport);
            assert_eq!(
                h.offset_px(),
                h.max_offset_px(),
                "resizing to {viewport} px left a gap past the last column"
            );
        }
        assert!(!h.overflows(), "a 40,000 px viewport holds the whole line");
    }

    /// Revealing is the minimum scroll that works — a column already on screen must not move the
    /// view, or every keystroke would jerk it.
    #[test]
    fn revealing_a_column_already_on_screen_does_not_move_the_view() {
        let mut h = hgrid(4_000, 1000.0);
        h.scroll_to_column(200);
        let before = h.offset_px();

        for column in h.visible_columns() {
            h.reveal(column..column + 1);
            assert_eq!(
                h.offset_px(),
                before,
                "revealing the already-visible column {column} moved the view"
            );
        }
    }

    #[test]
    fn revealing_scrolls_the_least_that_brings_a_column_into_view() {
        let mut h = hgrid(4_000, 1000.0);
        h.scroll_to_column(200);

        // Off the right: the column ends up against the right edge, not centred and not at the left.
        h.reveal(400..401);
        assert_eq!(h.x_of_column(401), 1000.0);
        assert!(h.visible_columns().contains(&400));

        // Off the left: against the left edge.
        h.reveal(50..51);
        assert_eq!(h.x_of_column(50), 0.0);

        // An empty range is a caret, and is revealed like the column it sits at.
        h.scroll_to_column(0);
        h.reveal(300..300);
        assert!(
            h.visible_columns().contains(&300),
            "an empty range revealed nothing"
        );
    }

    /// A match wider than the window aligns to its start — the end a reader needs first.
    #[test]
    fn revealing_a_range_wider_than_the_viewport_shows_its_start() {
        let mut h = hgrid(4_000, 1000.0);
        h.reveal(600..1_400);
        assert_eq!(h.x_of_column(600), 0.0);
    }

    #[test]
    fn the_thumb_runs_from_zero_to_one_and_round_trips() {
        let mut h = hgrid(4_000, 1000.0);
        h.scroll_to_start();
        assert_eq!(h.thumb_fraction(), 0.0);
        h.scroll_to_end();
        assert_eq!(h.thumb_fraction(), 1.0);

        let mut previous = 0.0;
        for percent in 0..=100 {
            let asked = percent as f32 / 100.0;
            h.scroll_to_fraction(asked);
            let thumb = h.thumb_fraction();
            assert!(
                (thumb - asked).abs() < 0.001,
                "asked for {asked}, the thumb reads {thumb}"
            );
            assert!(thumb >= previous - 0.001, "the thumb went backwards");
            previous = thumb;
        }
    }

    /// The horizontal counterpart of the vertical grid's poisoning test. A `NaN` reaching the offset
    /// would blank every column permanently, since no later delta can clear it.
    #[test]
    fn a_non_finite_input_cannot_poison_the_offset() {
        for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let mut h = hgrid(4_000, 1000.0);
            h.scroll_to_column(500);

            h.scroll_by_px(bad);
            assert!(h.offset_px().is_finite(), "scroll_by_px({bad})");
            h.scroll_to_fraction(bad);
            assert!(h.offset_px().is_finite(), "scroll_to_fraction({bad})");
            h.set_cell_width(bad);
            assert!(h.cell_width().is_finite(), "set_cell_width({bad})");
            assert!(h.offset_px().is_finite(), "set_cell_width({bad})");
            h.set_viewport_px(bad);
            assert!(h.viewport_px().is_finite(), "set_viewport_px({bad})");

            // And it still works afterwards.
            h.set_cell_width(CELL_W);
            h.set_viewport_px(1000.0);
            h.scroll_by_px(53.0);
            assert!(
                !h.visible_columns().is_empty(),
                "after {bad} the grid drew nothing at all"
            );
            assert_eq!(h.column_at_x(0.0), Some(h.visible_columns().start));
        }
    }

    /// The viewport is covered left to right — no blank strip at either edge, at any offset.
    #[test]
    fn the_visible_columns_cover_the_whole_viewport() {
        let mut h = hgrid(4_000, 1000.0);
        for offset in [0.0f32, 13.5, 700.0, 9_000.0, 30_000.0] {
            h.scroll_to_start();
            h.scroll_by_px(offset);
            let visible = h.visible_columns();
            assert!(h.x_of_column(visible.start) <= 0.0, "a gap on the left");
            assert!(
                h.x_of_column(visible.end - 1) + CELL_W >= 1000.0,
                "a gap on the right at offset {offset}"
            );
        }
    }

    /// A cell width outside the range that keeps positions exact is clamped rather than accepted —
    /// the alternative is a silently drifting column at the right-hand end of a capped line.
    #[test]
    fn an_absurd_cell_width_is_clamped_to_the_exact_range() {
        let mut h = HGrid::new(1e9);
        assert_eq!(h.cell_width(), MAX_CELL_WIDTH_PX);
        h.set_cell_width(0.0);
        assert_eq!(h.cell_width(), 1.0);
        h.set_cell_width(-4.0);
        assert_eq!(h.cell_width(), 1.0);
    }
}
