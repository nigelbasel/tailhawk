//! Selection — V3's third piece, over [`crate::grid`]'s rows and [`crate::cell`]'s columns.
//!
//! Portable, device-free and headless like both of them. Nothing here draws and nothing here reads a
//! file: it answers *what is selected*, per row, in cell columns, and resolves that to a byte range
//! once the caller supplies the line's text.
//!
//! ## Why a row is a `u64` and a column is a `usize`
//!
//! The two axes are not symmetric and must not be modelled as though they were. A document has up to
//! 2⁶⁴ rows and `SPEC.md` §6.4's whole scroll model exists because of it, so a row index is a `u64`
//! and **no expression here forms a row count as a pixel or a length** — [`Selection::row_span`] is
//! O(1) in the row asked about, precisely so a selection spanning 100M rows costs nothing to paint.
//! A *line*, by contrast, is bounded by §10.3, so a column is an ordinary `usize`.
//!
//! **The corollary is that this module never iterates its own rows.** A caller paints by asking
//! [`crate::grid::Grid::visible`] which rows are on screen and then asking `row_span` about each —
//! at most a viewport's worth. Selecting a whole 10 GB file and pressing Ctrl+C is §10.2's confirm
//! path, not a loop here.
//!
//! ## Two modes, because §13 of `UI-DESIGN.md` specifies both
//!
//! [`SelectionMode::Stream`] is the ordinary one: from a point on one row, through every intervening
//! row in full, to a point on another. [`SelectionMode::Block`] is Alt+drag — the same column band on
//! every row in range, which is how a column of timestamps gets lifted out of a log.
//!
//! **The column band is the reason block mode is in cells rather than bytes.** A block over rows
//! whose content is a mix of ASCII and CJK covers a different number of *bytes* on each row and the
//! same number of *columns*, and columns are what the user drew the rectangle over. §3.3's cell model
//! is the authority for that mapping, and this module defers to it entirely.
//!
//! ## What this deliberately does not do
//!
//! `UI-DESIGN.md` §12 also specifies shift-click extension, double/triple-click granularity and
//! autoscroll-on-drag. Extension is [`Selection::set_focus`] and belongs to the caller's input
//! handling; granularity is [`crate::cell::CellModel::word_at_cell`] plus a constructor here, and a
//! *sticky* granularity — where dragging after a double-click keeps snapping to whole words — is
//! **not** modelled and is a real gap. Autoscroll is the grid's and the input loop's.

use crate::cell::CellModel;
use core::ops::Range;

/// A point in the document: a row, and a cell column within it.
///
/// **`Ord` is derived, and the field order is load-bearing** — row before column is exactly the
/// document order two points have to be normalised into, so `min`/`max` are the whole of
/// [`Selection::start`] and [`Selection::end`] in stream mode.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Position {
    /// The document row.
    pub row: u64,
    /// The cell column, in [`crate::cell`]'s cells — not bytes, and not characters.
    pub cell: usize,
}

impl Position {
    pub const fn new(row: u64, cell: usize) -> Self {
        Self { row, cell }
    }
}

/// Stream or rectangular.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum SelectionMode {
    /// Point to point through the intervening rows in full.
    #[default]
    Stream,
    /// The same column band on every row in range — Alt+drag.
    Block,
}

/// Where a row's selected span ends.
///
/// **`ToLineEnd` is not a column and cannot be replaced by one.** A stream selection covering a whole
/// interior row runs past the last character to include the line terminator, and this module does not
/// know how long any line is — that depends on the row's text, which the caller has and this does
/// not. Painting draws it to the right edge of the viewport, which is the usual signal that the break
/// is in the selection.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum RowEnd {
    /// Ends at this cell column, exclusive.
    Cell(usize),
    /// Runs to the end of the line's content.
    ToLineEnd,
}

/// The part of one row a selection covers.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct RowSpan {
    /// First selected cell column.
    pub start_cell: usize,
    /// Where it ends.
    pub end: RowEnd,
    /// Whether the row's own line terminator is part of the selection.
    ///
    /// True for every stream row except the last, and **never** for a block selection: a block copy
    /// joins its rows with a newline the caller supplies, and does not carry the original file's
    /// terminators — which may not even agree with each other within one file.
    pub line_break: bool,
}

/// A selection: two points, and how to read the region between them.
///
/// The **anchor** is where the drag started and the **focus** is where the pointer is now, so the two
/// are kept as given rather than pre-sorted — the caller needs to know which end moves when the user
/// shift-clicks, and normalising on construction throws that away.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Selection {
    anchor: Position,
    focus: Position,
    mode: SelectionMode,
}

impl Selection {
    /// An empty selection — a caret, at the point a click landed.
    pub const fn at(anchor: Position) -> Self {
        Self {
            anchor,
            focus: anchor,
            mode: SelectionMode::Stream,
        }
    }

    pub const fn stream(anchor: Position, focus: Position) -> Self {
        Self {
            anchor,
            focus,
            mode: SelectionMode::Stream,
        }
    }

    pub const fn block(anchor: Position, focus: Position) -> Self {
        Self {
            anchor,
            focus,
            mode: SelectionMode::Block,
        }
    }

    /// The word around a column, as a stream selection — a double-click.
    ///
    /// Empty where there is no word, which is what a double-click past the end of a line means. See
    /// [`CellModel::word_at_cell`] for what counts as one.
    pub fn word(model: &CellModel, row: u64, line: &str, cell: usize) -> Self {
        let word = model.word_at_cell(line, cell);
        Self::stream(Position::new(row, word.start), Position::new(row, word.end))
    }

    /// A whole row including its line terminator — a triple-click.
    ///
    /// Expressed as running to the *start* of the next row rather than to a column on this one, so it
    /// needs no knowledge of the line's length and picks up the break for free.
    ///
    /// **The last representable row has no next row, and saturating there is a silent bug.**
    /// `row + 1` saturates to the same row, which makes anchor and focus equal — so a triple-click on
    /// `u64::MAX` would select *nothing at all* rather than the line. It runs to the far end of the
    /// row's own content instead, and takes no break, because there is no following row for a break
    /// to separate it from.
    pub fn line(row: u64) -> Self {
        let focus = match row.checked_add(1) {
            Some(next) => Position::new(next, 0),
            None => Position::new(row, usize::MAX),
        };
        Self::stream(Position::new(row, 0), focus)
    }

    pub fn anchor(&self) -> Position {
        self.anchor
    }

    pub fn focus(&self) -> Position {
        self.focus
    }

    pub fn mode(&self) -> SelectionMode {
        self.mode
    }

    /// Moves the focus — a drag, or a shift-click. The anchor stays put.
    pub fn set_focus(&mut self, focus: Position) {
        self.focus = focus;
    }

    pub fn set_mode(&mut self, mode: SelectionMode) {
        self.mode = mode;
    }

    /// The first point of the selected region in document order.
    ///
    /// **Mode-aware, and it has to be.** `anchor.min(focus)` is right for a stream selection, but for
    /// a block dragged from `(2, 9)` to `(5, 3)` it answers `(2, 9)` — a corner of the rectangle that
    /// is neither its top-left nor anywhere the user pointed. A caller placing a caret or scrolling to
    /// the selection would land off to the right of the block it is meant to be showing. In block
    /// mode the region's first point is the top-left: earliest row, leftmost column.
    pub fn start(&self) -> Position {
        match self.mode {
            SelectionMode::Stream => self.anchor.min(self.focus),
            SelectionMode::Block => Position::new(self.first_row(), self.column_band().start),
        }
    }

    /// The last point of the selected region in document order, exclusive. See [`Selection::start`]
    /// for why this is mode-aware.
    pub fn end(&self) -> Position {
        match self.mode {
            SelectionMode::Stream => self.anchor.max(self.focus),
            SelectionMode::Block => Position::new(self.last_row(), self.column_band().end),
        }
    }

    /// The first row the selection touches.
    pub fn first_row(&self) -> u64 {
        self.anchor.row.min(self.focus.row)
    }

    /// The last row the selection touches — inclusive, and it may be the same as the first.
    ///
    /// **Inclusive on purpose.** An exclusive end would be `last + 1`, which overflows at
    /// `u64::MAX`, and a selection reaching the last row of a document that large is reachable with
    /// one Ctrl+A.
    pub fn last_row(&self) -> u64 {
        self.anchor.row.max(self.focus.row)
    }

    /// The column band a block selection covers. Meaningless in stream mode.
    fn column_band(&self) -> Range<usize> {
        let lo = self.anchor.cell.min(self.focus.cell);
        let hi = self.anchor.cell.max(self.focus.cell);
        lo..hi
    }

    /// Whether anything at all is selected.
    ///
    /// **A block selection is empty when its column band is**, however many rows it spans — dragging
    /// straight down with Alt held selects nothing, and painting a zero-width rectangle on 400 rows
    /// is a visible artefact rather than a no-op.
    pub fn is_empty(&self) -> bool {
        match self.mode {
            SelectionMode::Stream => self.anchor == self.focus,
            SelectionMode::Block => self.anchor.cell == self.focus.cell,
        }
    }

    /// What the selection covers on one row, or `None` if it does not reach it.
    ///
    /// **O(1), and never iterates rows.** This is the API the painter calls once per *visible* row;
    /// a selection spanning the whole document costs a viewport's worth of these, not 50 million.
    pub fn row_span(&self, row: u64) -> Option<RowSpan> {
        if self.is_empty() || row < self.first_row() || row > self.last_row() {
            return None;
        }
        match self.mode {
            SelectionMode::Block => {
                let band = self.column_band();
                Some(RowSpan {
                    start_cell: band.start,
                    end: RowEnd::Cell(band.end),
                    line_break: false,
                })
            }
            SelectionMode::Stream => {
                let (start, end) = (self.start(), self.end());
                let start_cell = if row == start.row { start.cell } else { 0 };
                let (row_end, line_break) = if row == end.row {
                    (RowEnd::Cell(end.cell), false)
                } else {
                    (RowEnd::ToLineEnd, true)
                };
                Some(RowSpan {
                    start_cell,
                    end: row_end,
                    line_break,
                })
            }
        }
    }

    /// The bytes selected on one row, given that row's decoded text.
    ///
    /// `None` where the row is not in the selection. The returned range does **not** include the line
    /// terminator even when [`RowSpan::line_break`] is set — the terminator is not part of `line`,
    /// and §10.2's "preserving original bytes" means the caller re-emits the file's own break rather
    /// than one this module invents.
    ///
    /// Column-to-byte rounding, including what happens to zero-width clusters on a boundary, is
    /// [`CellModel::byte_span`]'s and is specified there.
    pub fn byte_range(&self, model: &CellModel, row: u64, line: &str) -> Option<Range<usize>> {
        let span = self.row_span(row)?;
        let end = match span.end {
            RowEnd::ToLineEnd => model.cell_count(line),
            RowEnd::Cell(cell) => cell,
        };
        Some(model.byte_span(line, span.start_cell..end))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model() -> CellModel {
        CellModel::new()
    }

    #[test]
    fn a_caret_selects_nothing_and_covers_no_row() {
        let sel = Selection::at(Position::new(7, 3));
        assert!(sel.is_empty());
        assert_eq!(sel.row_span(7), None);
        assert_eq!(sel.byte_range(&model(), 7, "hello"), None);
    }

    #[test]
    fn a_selection_normalises_whichever_way_it_was_dragged() {
        let down = Selection::stream(Position::new(2, 4), Position::new(9, 1));
        let up = Selection::stream(Position::new(9, 1), Position::new(2, 4));
        assert_eq!(down.start(), up.start());
        assert_eq!(down.end(), up.end());
        for row in [2, 5, 9] {
            assert_eq!(down.row_span(row), up.row_span(row));
        }
    }

    #[test]
    fn the_anchor_is_not_normalised_away() {
        let mut sel = Selection::stream(Position::new(9, 1), Position::new(2, 4));
        assert_eq!(sel.anchor(), Position::new(9, 1));
        sel.set_focus(Position::new(4, 0));
        assert_eq!(sel.anchor(), Position::new(9, 1));
        assert_eq!(sel.start(), Position::new(4, 0));
    }

    #[test]
    fn a_stream_selection_covers_first_partly_middle_wholly_and_last_partly() {
        let sel = Selection::stream(Position::new(2, 4), Position::new(5, 3));
        assert_eq!(sel.row_span(1), None);
        assert_eq!(sel.row_span(6), None);
        assert_eq!(
            sel.row_span(2),
            Some(RowSpan {
                start_cell: 4,
                end: RowEnd::ToLineEnd,
                line_break: true
            })
        );
        for middle in [3, 4] {
            assert_eq!(
                sel.row_span(middle),
                Some(RowSpan {
                    start_cell: 0,
                    end: RowEnd::ToLineEnd,
                    line_break: true
                })
            );
        }
        assert_eq!(
            sel.row_span(5),
            Some(RowSpan {
                start_cell: 0,
                end: RowEnd::Cell(3),
                line_break: false
            })
        );
    }

    #[test]
    fn a_single_row_stream_selection_takes_no_line_break() {
        let sel = Selection::stream(Position::new(4, 2), Position::new(4, 6));
        assert_eq!(
            sel.row_span(4),
            Some(RowSpan {
                start_cell: 2,
                end: RowEnd::Cell(6),
                line_break: false
            })
        );
    }

    #[test]
    fn a_block_selection_takes_the_same_band_on_every_row_and_no_break() {
        let sel = Selection::block(Position::new(10, 7), Position::new(13, 2));
        for row in 10..=13 {
            assert_eq!(
                sel.row_span(row),
                Some(RowSpan {
                    start_cell: 2,
                    end: RowEnd::Cell(7),
                    line_break: false
                })
            );
        }
        assert_eq!(sel.row_span(9), None);
        assert_eq!(sel.row_span(14), None);
    }

    #[test]
    fn a_block_with_no_width_selects_nothing_however_many_rows_it_spans() {
        let sel = Selection::block(Position::new(0, 5), Position::new(400, 5));
        assert!(sel.is_empty());
        assert_eq!(sel.row_span(200), None);
    }

    /// A selection over the whole of a 100M-row document must cost a viewport, not a document.
    #[test]
    fn a_selection_spanning_the_document_answers_per_row_in_constant_time() {
        let sel = Selection::stream(Position::new(0, 0), Position::new(100_000_000, 0));
        assert!(sel.row_span(50_000_000).is_some());
        assert_eq!(sel.last_row(), 100_000_000);
        assert_eq!(
            sel.row_span(99_999_999).unwrap().end,
            RowEnd::ToLineEnd,
            "an interior row runs to the line end regardless of how far from either end it is"
        );
    }

    /// `last_row` is inclusive because the exclusive form overflows on a document one Ctrl+A wide.
    #[test]
    fn a_selection_reaching_the_last_representable_row_does_not_overflow() {
        let sel = Selection::stream(Position::new(0, 0), Position::new(u64::MAX, 4));
        assert_eq!(sel.last_row(), u64::MAX);
        assert!(sel.row_span(u64::MAX).is_some());
    }

    /// Saturating `row + 1` here collapses anchor onto focus, and the row silently selects nothing.
    #[test]
    fn a_triple_click_on_the_last_representable_row_still_selects_it() {
        let sel = Selection::line(u64::MAX);
        assert_eq!(sel.last_row(), u64::MAX);
        assert!(!sel.is_empty(), "the last row is selectable like any other");
        assert_eq!(sel.byte_range(&model(), u64::MAX, "hello"), Some(0..5));
        assert!(
            !sel.row_span(u64::MAX).unwrap().line_break,
            "there is no following row for a break to separate it from"
        );
    }

    /// A caret sitting on a zero-width cluster must not copy it — the outward rounding of
    /// [`CellModel::byte_span`] makes that the live risk, not a theoretical one.
    #[test]
    fn a_caret_next_to_an_invisible_copies_nothing() {
        let m = model();
        let line = "ab\u{200B}cd";
        for cell in 0..=m.cell_count(line) {
            let span = m.byte_span(line, cell..cell);
            assert!(span.is_empty(), "cell {cell} gave {span:?}");
        }
    }

    /// A block's origin is its top-left corner, not whichever corner the drag happened to start at.
    #[test]
    fn a_block_reports_its_top_left_however_it_was_dragged() {
        let corners = [
            (Position::new(2, 9), Position::new(5, 3)),
            (Position::new(5, 3), Position::new(2, 9)),
            (Position::new(2, 3), Position::new(5, 9)),
            (Position::new(5, 9), Position::new(2, 3)),
        ];
        for (anchor, focus) in corners {
            let sel = Selection::block(anchor, focus);
            assert_eq!(sel.start(), Position::new(2, 3), "from {anchor:?}");
            assert_eq!(sel.end(), Position::new(5, 9), "from {anchor:?}");
        }
    }

    /// The same drag read as a stream keeps the stream meaning of its two ends.
    #[test]
    fn a_stream_selection_reports_its_own_ends_not_a_bounding_box() {
        let sel = Selection::stream(Position::new(2, 9), Position::new(5, 3));
        assert_eq!(sel.start(), Position::new(2, 9));
        assert_eq!(sel.end(), Position::new(5, 3));
    }

    #[test]
    fn a_triple_click_takes_the_row_and_its_break() {
        let sel = Selection::line(3);
        assert_eq!(
            sel.row_span(3),
            Some(RowSpan {
                start_cell: 0,
                end: RowEnd::ToLineEnd,
                line_break: true
            })
        );
        assert_eq!(
            sel.byte_range(&model(), 3, "hello"),
            Some(0..5),
            "the byte range stops at the content; the break is not in `line`"
        );
    }

    #[test]
    fn byte_range_resolves_columns_through_the_cell_model() {
        let sel = Selection::stream(Position::new(0, 1), Position::new(0, 3));
        assert_eq!(sel.byte_range(&model(), 0, "abcdef"), Some(1..3));
    }

    /// The mapping is by column, so the same band is a different number of bytes on each row.
    #[test]
    fn a_block_over_mixed_width_content_is_columns_not_bytes() {
        let sel = Selection::block(Position::new(0, 0), Position::new(1, 4));
        assert_eq!(sel.byte_range(&model(), 0, "abcdef"), Some(0..4));
        // Two CJK clusters fill the same four columns, in six bytes.
        assert_eq!(sel.byte_range(&model(), 1, "日本語"), Some(0..6));
    }

    /// A column band wider than the line clamps to the line rather than running off it.
    #[test]
    fn a_block_wider_than_a_row_clamps_to_that_row() {
        let sel = Selection::block(Position::new(0, 2), Position::new(0, 40));
        assert_eq!(sel.byte_range(&model(), 0, "abcde"), Some(2..5));
        assert_eq!(sel.byte_range(&model(), 0, "a"), Some(1..1));
    }

    /// §5.6 — the copied bytes must not launder a bidi override out of the line.
    #[test]
    fn selecting_a_line_carries_its_zero_width_content() {
        let line = "\u{202E}abc";
        let sel = Selection::line(0);
        assert_eq!(sel.byte_range(&model(), 0, line), Some(0..line.len()));
        let copied = &line[sel.byte_range(&model(), 0, line).unwrap()];
        assert!(copied.contains('\u{202E}'), "copied {copied:?}");
    }

    #[test]
    fn a_double_click_takes_the_word_under_the_column() {
        let m = model();
        let line = "GET /health 200";
        assert_eq!(
            Selection::word(&m, 0, line, 1).byte_range(&m, 0, line),
            Some(0..3)
        );
        assert_eq!(
            Selection::word(&m, 0, line, 13).byte_range(&m, 0, line),
            Some(12..15)
        );
        assert!(Selection::word(&m, 0, line, 99).is_empty());
    }

    #[test]
    fn a_word_selection_on_one_row_stays_on_that_row() {
        let m = model();
        let sel = Selection::word(&m, 42, "alpha beta", 7);
        assert_eq!(sel.first_row(), 42);
        assert_eq!(sel.last_row(), 42);
        assert_eq!(sel.byte_range(&m, 42, "alpha beta"), Some(6..10));
    }

    /// Dragging back over the anchor collapses the selection rather than inverting it.
    #[test]
    fn dragging_the_focus_back_onto_the_anchor_empties_the_selection() {
        let mut sel = Selection::stream(Position::new(3, 2), Position::new(6, 9));
        assert!(!sel.is_empty());
        sel.set_focus(Position::new(3, 2));
        assert!(sel.is_empty());
        assert_eq!(sel.row_span(4), None);
    }

    /// Switching an existing drag to Alt+drag re-reads the same two points as a rectangle.
    #[test]
    fn a_stream_selection_switched_to_block_keeps_its_points() {
        let mut sel = Selection::stream(Position::new(2, 9), Position::new(5, 3));
        sel.set_mode(SelectionMode::Block);
        assert_eq!(sel.anchor(), Position::new(2, 9));
        assert_eq!(
            sel.row_span(3),
            Some(RowSpan {
                start_cell: 3,
                end: RowEnd::Cell(9),
                line_break: false
            })
        );
    }
}
