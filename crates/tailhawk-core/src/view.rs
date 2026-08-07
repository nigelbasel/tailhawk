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

use crate::cell::CellModel;
use crate::grid::Grid;
use crate::hgrid::HGrid;
use crate::selection::Position;

/// The part of one row's line that the viewport shows, and where to draw it.
#[derive(Clone, Debug, PartialEq)]
pub struct RowSlice {
    /// Bytes of the line to shape and draw — **rounded outwards to whole clusters**, so this may
    /// begin left of the first visible column and end right of the last.
    pub bytes: Range<usize>,
    /// Viewport-relative x for the first byte's column. **Negative when a cluster straddles the
    /// left edge**, which is the case this type exists for.
    pub x: f32,
}

/// The two axes and the cell model, as one viewport.
#[derive(Clone, Debug)]
pub struct View {
    grid: Grid,
    hgrid: HGrid,
    cells: CellModel,
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
        }
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
        self.grid.set_viewport_px(height_px);
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
        let visible = self.hgrid.visible_columns();
        let bytes = self.cells.byte_span(line, visible.clone());
        if bytes.is_empty() {
            return RowSlice {
                x: self.hgrid.x_of_column(visible.start),
                bytes,
            };
        }
        // **Its own column, not the one that was asked for.** `byte_span` rounds outwards, so the
        // first byte may belong to a cluster that starts left of `visible.start`; drawing it at
        // `visible.start` would shift the whole line right by the part that is off screen.
        let first_column = self.cells.cell_at_byte(line, bytes.start);
        RowSlice {
            x: self.hgrid.x_of_column(first_column),
            bytes,
        }
    }

    /// The document position a viewport-relative point lands on, or `None` outside the drawn cells.
    ///
    /// Both axes must hit: a click below the last row or right of the widest line is not a position,
    /// and clamping it to one here would silently invent a selection endpoint the user did not
    /// point at. A drag that leaves the viewport is the input loop's autoscroll, not this.
    pub fn position_at(&self, x: f32, y: f32) -> Option<Position> {
        let row = self.grid.row_at_y(y)?;
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
