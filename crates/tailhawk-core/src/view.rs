//! The viewport — where V3's four pieces meet.
//!
//! [`crate::grid`] answers which rows are on screen, [`crate::hgrid`] which columns,
//! [`crate::cell`] how a line maps to those columns, and [`crate::selection`] what is selected.
//! Each was built and reviewed on its own, and **each was unwired until this module existed** — so
//! their defects were latent rather than live. This is the thing that makes them live, and it is
//! deliberately still portable and device-free: nothing here draws, and it holds no font, no device
//! and no window. What draws it is `Renderer`, which owns the resources a lost device invalidates.
//!
//! ## Why the composition is a function rather than a note in the painter
//!
//! Two of the joins are not obvious, and getting either wrong is silent.
//!
//! **A wide cluster straddling the left edge starts before the first visible column.** A painter
//! that shapes from `visible_columns().start` cuts a CJK character in half and draws the remains.
//! [`CellModel::byte_span`](crate::cell::CellModel::byte_span) already rounds both ends outwards —
//! it was built that way for §5.6, so a copy cannot launder a zero-width override out of a
//! selection — and the same rounding returns the straddling cluster whole. What is then required is
//! that the painter draw it at **its own** column rather than at the one it asked for, which is why
//! [`HGrid::x_of_column`](crate::hgrid::HGrid::x_of_column) is unclamped and returns a negative x.
//! [`View::slice`] is those three calls in the one order that is correct.
//!
//! **A click is two independent hit-tests that must agree about the same frame.**
//! [`View::position_at`] pairs them into the [`Position`] [`crate::selection`] takes, so a caller
//! cannot pass a row from one frame and a column from another.

use core::ops::Range;

use crate::cell::{CellModel, ColumnAnchors};
use crate::grid::Grid;
use crate::hgrid::HGrid;
use crate::selection::Position;

/// The most bytes of one row handed to the shaper in a frame.
///
/// Not a line-length limit — §10.3's 32 KB render cap is that, and it is enforced on the *extent*
/// in [`crate::hgrid`]. This bounds the one case the column window cannot: a single grapheme
/// cluster that is itself megabytes long, which occupies one cell and shapes to one glyph per
/// combining mark. Generous enough that no ordinary line reaches it.
const MAX_SLICE_BYTES: usize = 8 * 1024;

/// The part of one row's line that the viewport shows, and where to draw it.
#[derive(Clone, Debug, PartialEq)]
pub struct RowSlice {
    /// Bytes of the line to shape and draw — **rounded outwards to whole clusters**, so this may
    /// begin left of the first visible column and end right of the last.
    pub bytes: Range<usize>,
    /// Viewport-relative x for the first byte's column. **Negative when a cluster straddles the
    /// left edge**, which is the case this type exists for.
    pub x: f32,
    /// The cell column [`bytes`](Self::bytes) starts at.
    ///
    /// **Carried rather than left to the caller to re-derive**, because re-deriving it is
    /// `cell_at_byte`, which walks the line's graphemes from byte zero. A painter needs a column
    /// per cluster, and asking per cluster measured 16.4 s for one frame of 32 KB lines. This
    /// value is already computed here to produce `x`; handing it over costs nothing.
    pub column: usize,
    /// **This row's visible bytes hit `MAX_SLICE_BYTES` and the tail was dropped.**
    ///
    /// The cap is deliberate and the comment in [`View::slice`] says why, but "an ordinary line
    /// never reaches it" is not the same as "no on-screen line reaches it". A cell costs `1 + 2n`
    /// bytes with `n` combining marks, so around twenty marks per base character puts a *200-column*
    /// row past 8 KB — narrower than any real viewport, and §13.4 makes hostile text an explicit
    /// threat model rather than a curiosity. When that happens the right-hand part of the row draws
    /// nothing at all and the row looks like it simply ends.
    ///
    /// Silently is the part that is not acceptable: §5.6 governs the clipboard, not the screen, but
    /// a viewer that shows two thirds of a line and looks complete is the same failure one surface
    /// over. So the cap stays and it reports itself.
    pub truncated: bool,
}

/// The two axes and the cell model, as one viewport.
#[derive(Clone, Debug)]
pub struct View {
    grid: Grid,
    hgrid: HGrid,
    cells: CellModel,
    /// A band at the top of the viewport that is not rows — the column header, when a format has
    /// columns. The grid never sees it: it gets the height below the band, every placed row is
    /// drawn lower by the whole inset, and a hit-test subtracts it first. Zero for a plain file.
    header_px: f32,
    /// The command bar's band, above the header — V14's chrome: the find field, the chip row.
    /// Same rule as the header; the two add up to [`View::top_inset`].
    chrome_px: f32,
    /// The whole viewport height, so a header change can re-derive the grid's share.
    height_px: f32,
}

impl View {
    /// `row_height` and `cell_width` are device pixels, and both come from the **measured face**
    /// rather than from a constant — `SPEC.md` §3.1 requires integer cell advances re-derived on
    /// every scale change, and the only thing that knows the advance is the rasterised font.
    pub fn new(cell_width: f32, row_height: f32) -> Self {
        Self {
            grid: Grid::new(row_height),
            hgrid: HGrid::new(cell_width),
            cells: CellModel::new(),
            header_px: 0.0,
            chrome_px: 0.0,
            height_px: 0.0,
        }
    }

    /// The header band's height: one row for a column header, zero for none. See [`View::header_px`].
    pub fn set_header_px(&mut self, header_px: f32) {
        self.header_px = header_px.max(0.0);
        self.grid
            .set_viewport_px((self.height_px - self.top_inset()).max(0.0));
    }

    /// The command bar's height, above the header. See [`View::chrome_px`].
    pub fn set_chrome_px(&mut self, chrome_px: f32) {
        self.chrome_px = chrome_px.max(0.0);
        self.grid
            .set_viewport_px((self.height_px - self.top_inset()).max(0.0));
    }

    /// The column header's height. It is drawn in `chrome_px..chrome_px + header_px`.
    pub fn header_px(&self) -> f32 {
        self.header_px
    }

    /// The command bar's height. It is drawn in `0..chrome_px`.
    pub fn chrome_px(&self) -> f32 {
        self.chrome_px
    }

    /// Where the rows start: the chrome and the header together. A painter adds this to every
    /// [`PlacedRow`](crate::grid::PlacedRow)'s `y`.
    pub fn top_inset(&self) -> f32 {
        self.chrome_px + self.header_px
    }

    pub fn grid(&self) -> &Grid {
        &self.grid
    }

    pub fn grid_mut(&mut self) -> &mut Grid {
        &mut self.grid
    }

    pub fn hgrid(&self) -> &HGrid {
        &self.hgrid
    }

    pub fn hgrid_mut(&mut self) -> &mut HGrid {
        &mut self.hgrid
    }

    pub fn cells(&self) -> &CellModel {
        &self.cells
    }

    pub fn cells_mut(&mut self) -> &mut CellModel {
        &mut self.cells
    }

    /// The body's size in device pixels — the gutter and line-number column already subtracted.
    pub fn set_viewport(&mut self, width_px: f32, height_px: f32) {
        self.hgrid.set_viewport_px(width_px);
        self.height_px = height_px;
        self.grid
            .set_viewport_px((height_px - self.top_inset()).max(0.0));
    }

    /// A DPI or font-size change, both axes at once.
    ///
    /// **Both keep their leading unit rather than their pixel offset** — the top row stays the top
    /// row and the leftmost column stays the leftmost column. Each half of that was argued and
    /// tested where it lives; setting them together is what stops a caller doing one and not the
    /// other, which would drift one axis against the other on every monitor change.
    pub fn set_metrics(&mut self, cell_width: f32, row_height: f32) {
        self.hgrid.set_cell_width(cell_width);
        self.grid.set_row_height(row_height);
    }

    /// The bytes of `line` to draw, and where to put them. See the module note.
    ///
    /// A line that ends left of the viewport yields an empty range, and the painter draws nothing
    /// for it — which is the ordinary case for a short line in a horizontally scrolled file, not an
    /// error.
    pub fn slice(&self, line: &str) -> RowSlice {
        self.slice_anchored(line, &ColumnAnchors::none())
    }

    /// [`slice`](Self::slice), using a row's prebuilt column anchors.
    ///
    /// **This is where the horizontal frame budget is spent.** Both calls below mapped columns to
    /// bytes by walking the line from byte zero, once per row per frame; at the end of a 19.4 KB
    /// line that measured 76 ms a frame. The anchors turn each into a binary search plus a bounded
    /// walk, and passing [`ColumnAnchors::none`] is always correct and only slower.
    pub fn slice_anchored(&self, line: &str, anchors: &ColumnAnchors) -> RowSlice {
        let visible = self.hgrid.visible_columns();
        let mut bytes = self
            .cells
            .byte_span_anchored(line, visible.clone(), anchors);

        // **A slice is bounded in columns but not in bytes, and one cluster can be the whole line.**
        // `"a" + "\u{0301}".repeat(16000)` is 32 KB — exactly §10.3's supported inline size — and it
        // is *one* grapheme cluster occupying *one* cell. The column window cannot narrow it, so the
        // whole 32 KB reaches the shaper, comes back as 16,001 glyphs, and emits 16,001 cell-sized
        // quads stacked on one column. Nothing downstream bounds that: the atlas is per glyph, and
        // the instance buffer is per quad.
        //
        // So the cap goes here, where "which bytes of this row are drawn" is already decided, and it
        // bounds shaping, glyph count and instances together. The end snaps **inward** to a `char`
        // boundary rather than outward to a cluster one — outward would restore the entire cluster
        // and defeat the cap. It is the same truncation §10.3 already sanctions, and an ordinary
        // line never reaches it: a 32 KB ASCII line still slices to its visible columns.
        let truncated = bytes.len() > MAX_SLICE_BYTES;
        if truncated {
            let mut end = bytes.start + MAX_SLICE_BYTES;
            while end > bytes.start && !line.is_char_boundary(end) {
                end -= 1;
            }
            bytes.end = end;
        }

        if bytes.is_empty() {
            return RowSlice {
                x: self.hgrid.x_of_column(visible.start),
                column: visible.start,
                truncated,
                bytes,
            };
        }
        // **Its own column, not the one that was asked for.** `byte_span` rounds outwards, so the
        // first byte may belong to a cluster that starts left of `visible.start`; drawing it at
        // `visible.start` would shift the whole line right by the part that is off screen.
        let first_column = self.cells.cell_at_byte_anchored(line, bytes.start, anchors);
        RowSlice {
            x: self.hgrid.x_of_column(first_column),
            column: first_column,
            truncated,
            bytes,
        }
    }

    /// The document position a viewport-relative point lands on, or `None` outside the drawn cells.
    ///
    /// Both axes must hit: a click below the last row or right of the widest line is not a position,
    /// and clamping it to one here would silently invent a selection endpoint the user did not
    /// point at. A drag that leaves the viewport is the input loop's autoscroll, not this.
    pub fn position_at(&self, x: f32, y: f32) -> Option<Position> {
        // The chrome and the header are not rows: a click in them is nowhere here.
        let row = self.grid.row_at_y(y - self.top_inset())?;
        let cell = self.hgrid.column_at_x(x)?;
        Some(Position::new(row, cell))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CELL_W: f32 = 8.0;
    const ROW_H: f32 = 19.0;

    fn view(rows: u64, columns: u64, width: f32, height: f32) -> View {
        let mut v = View::new(CELL_W, ROW_H);
        v.grid_mut().set_total_rows(rows);
        v.hgrid_mut().set_columns(columns);
        v.set_viewport(width, height);
        v
    }

    #[test]
    fn an_unscrolled_line_slices_from_its_first_byte_at_x_zero() {
        let v = view(100, 4_000, 80.0, 190.0);
        let slice = v.slice("hello world, this is a log line");
        assert_eq!(slice.bytes.start, 0);
        assert_eq!(slice.x, 0.0);
        // 80 px of an 8 px cell is ten columns, and the painter gets whole clusters for them.
        assert_eq!(slice.bytes.end, 10);
    }

    /// **The case this module exists for.** A CJK cluster is two cells wide, so scrolling by an odd
    /// number of columns puts one straddling the left edge — and it must be drawn whole, from its
    /// own column, at a negative x.
    #[test]
    fn a_wide_cluster_straddling_the_left_edge_is_drawn_whole_from_its_own_column() {
        let line = "日本語のログ行";
        let mut v = view(100, 4_000, 80.0, 190.0);

        // Column 3 is the second half of the second cluster: clusters occupy 0-1, 2-3, 4-5, ...
        v.hgrid_mut().scroll_to_column(3);
        let slice = v.slice(line);

        let first = v.cells().cell_at_byte(line, slice.bytes.start);
        assert_eq!(first, 2, "the straddling cluster starts at column 2");
        assert_eq!(
            slice.x, -CELL_W,
            "it must be drawn one cell left of the viewport, not at zero"
        );
        assert!(
            line[slice.bytes.clone()].starts_with('本'),
            "the cluster was cut in half: {:?}",
            &line[slice.bytes.clone()]
        );
    }

    /// Composing the two ends of `byte_at_cell` instead would drop a zero-width cluster from what
    /// gets drawn — the §5.6 defect `byte_span` was built to prevent, arriving through the painter
    /// rather than through a copy. `reveal_invisibles` makes it visible on screen; the point here is
    /// that the bytes reach the shaper either way, since a bidi override changes how the rest of the
    /// line is laid out even when it draws as nothing.
    #[test]
    fn a_zero_width_cluster_at_the_left_edge_reaches_the_painter() {
        let line = "\u{202E}abc";
        let v = view(100, 4_000, 80.0, 190.0);
        let slice = v.slice(line);
        assert_eq!(
            slice.bytes.start, 0,
            "the override was dropped from the drawn bytes"
        );
        assert!(line[slice.bytes].starts_with('\u{202E}'));
    }

    /// **One cluster can be the whole line, and the column window cannot narrow it.** 16,000
    /// combining acutes on one base is 32 KB — §10.3's supported inline size — as a single cluster
    /// in a single cell, which shapes to 16,001 glyphs and would emit 16,001 quads stacked on
    /// column zero. The byte cap is the only thing between that line and the frame.
    #[test]
    fn a_single_cluster_the_size_of_the_whole_line_is_capped() {
        let line = format!("a{}", "\u{0301}".repeat(16_000));
        assert_eq!(
            CellModel::new().cell_count(&line),
            1,
            "the fixture is meant to be one cell"
        );

        let v = view(100, 4_000, 80.0, 190.0);
        let slice = v.slice(&line);
        assert!(
            slice.bytes.len() <= MAX_SLICE_BYTES,
            "{} bytes reached the shaper",
            slice.bytes.len()
        );
        assert!(
            line.is_char_boundary(slice.bytes.end),
            "the cap cut a character in half"
        );
        // And an ordinary wide line is untouched by it.
        let ordinary = "x".repeat(32_000);
        assert!(v.slice(&ordinary).bytes.len() < 200);
    }

    /// The cap can bite a row that is *on screen and within §10.3's render cap*, and when it does the
    /// row must not look complete.
    ///
    /// "An ordinary line never reaches it" is true and is not the claim that matters. A cell costs
    /// `1 + 2n` bytes with `n` combining marks, so twenty marks per base character — Zalgo, which
    /// §13.4 puts squarely in scope — puts a 200-column row past 8 KB. 200 columns is roughly 1,600
    /// px: narrower than any real viewport, so the dropped tail is text the user is looking at.
    #[test]
    fn a_row_whose_visible_bytes_are_capped_says_so() {
        let cell = format!("a{}", "\u{0301}".repeat(20));
        let line = cell.repeat(300);
        assert_eq!(
            CellModel::new().cell_count(&line),
            300,
            "the fixture is meant to be 300 ordinary-width cells"
        );

        // A viewport showing all 300 of them — no horizontal scrolling involved.
        let v = view(100, 4_000, 300.0 * CELL_W, 190.0);
        let slice = v.slice(&line);
        assert!(
            slice.truncated,
            "{} bytes of a 300-column row fitted under the cap after all — \
             re-derive the fixture rather than deleting the test",
            slice.bytes.len()
        );
        assert!(slice.bytes.len() <= MAX_SLICE_BYTES);

        // The dropped tail is the point: the row draws short of its last visible column.
        let drawn = CellModel::new().cell_count(&line[slice.bytes.clone()]);
        assert!(
            drawn < 300,
            "the cap reported truncation without dropping anything"
        );

        // An ordinary line that fills the same viewport is not flagged.
        assert!(!v.slice(&"x".repeat(32_000)).truncated);
    }

    #[test]
    fn a_line_ending_left_of_the_viewport_draws_nothing() {
        let mut v = view(100, 4_000, 80.0, 190.0);
        v.hgrid_mut().scroll_to_column(200);
        let slice = v.slice("short");
        assert!(slice.bytes.is_empty(), "{:?}", slice.bytes);
    }

    /// A click is the inverse of layout on both axes at once, at an offset on each.
    #[test]
    fn a_click_resolves_to_the_row_and_column_drawn_there() {
        let mut v = view(1_000_000, 4_000, 800.0, 400.0);
        v.grid_mut().scroll_to_row(500_000);
        v.grid_mut().scroll_by_px(5.0);
        v.hgrid_mut().scroll_to_column(120);
        v.hgrid_mut().scroll_by_px(3.0);

        for placed in v.grid().visible() {
            for column in v.hgrid().visible_columns().take(4) {
                let x = v.hgrid().x_of_column(column) + CELL_W / 2.0;
                let y = placed.y + ROW_H / 2.0;
                if !(0.0..800.0).contains(&x) || !(0.0..400.0).contains(&y) {
                    continue;
                }
                assert_eq!(
                    v.position_at(x, y),
                    Some(Position::new(placed.row, column)),
                    "the point inside row {} column {column} resolved elsewhere",
                    placed.row
                );
            }
        }
    }

    #[test]
    fn a_click_outside_either_axis_is_not_a_position() {
        let v = view(3, 10, 800.0, 400.0);
        assert!(v.position_at(0.0, 0.0).is_some());
        assert_eq!(v.position_at(0.0, 400.0), None, "below the last row");
        assert_eq!(v.position_at(800.0, 0.0), None, "right of the viewport");
        assert_eq!(
            v.position_at(90.0, 0.0),
            None,
            "right of the widest line, inside the viewport"
        );
        assert_eq!(v.position_at(0.0, 3.0 * ROW_H), None, "past the last row");
    }

    /// A monitor change keeps the top row and the leftmost column — §3.3's acceptance test on both
    /// axes. Doing one and not the other is the failure this single call exists to prevent.
    #[test]
    fn a_dpi_change_keeps_the_leading_row_and_column_on_both_axes() {
        let mut v = view(1_000_000, 4_000, 800.0, 400.0);
        v.grid_mut().scroll_to_row(400_000);
        v.hgrid_mut().scroll_to_column(300);

        v.set_metrics(CELL_W * 1.5, ROW_H * 1.5);

        assert_eq!(v.grid().scroll().row, 400_000, "the top row moved");
        assert_eq!(
            v.hgrid().visible_columns().start,
            300,
            "the leftmost column moved"
        );
    }
}
