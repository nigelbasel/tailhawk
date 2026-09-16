//! The grid's text as a screen reader reads it — the shell's half of `SPEC.md` §14.1's text
//! provider, over [`tailhawk_core::textunit`]'s positions and units.
//!
//! **Everything a text range needs from a live document is here, and none of it needs a window**:
//! the rows as [`Lines`] — on screen from the painter's window, off screen read on demand — the
//! selection and the caret as positions, where a range's lines are on screen, and which position a
//! point lands on. The provider's COM objects in `main.rs` only call these and hand the answers to
//! UI Automation, so every decision below is tested against a real document with no device.
//!
//! **The text is what the screen shows** — the owner's answer of 2026-09-15: under a column layout a
//! row reads as its aligned fields, so what a screen reader says, what is highlighted and where a
//! click lands all agree.

use std::borrow::Cow;

use tailhawk_core::rows::RowSource;
use tailhawk_core::selection::Position;
use tailhawk_core::textunit::{self, Lines, Pos};

use crate::Document;

/// The view's rows as [`Lines`]: view rows, so a filter or a sort reads in the order it shows.
///
/// **A row comes from wherever it already is**: the presentation `lay_out` built for a visible row,
/// then the painter's window, and only then the file, through [`LogSet::read_row`], which leaves the
/// window alone. A row read from the file under a column layout is presented here exactly as
/// `lay_out` would present it — with the next line's body pulled in where the format keeps its
/// message there and continuations are collapsed — so a row reads the same on the screen or off it.
///
/// [`LogSet::read_row`]: tailhawk_core::set::LogSet::read_row
pub struct DocText<'a>(pub &'a Document);

impl Lines for DocText<'_> {
    fn rows(&self) -> u64 {
        self.0.view_rows()
    }

    fn line(&self, row: u64) -> Option<Cow<'_, str>> {
        let doc = self.0;
        let file_row = doc.filtering.file_row(row)?;
        if let Some(presented) = doc.presentation(file_row) {
            return Some(Cow::Borrowed(presented.text.as_str()));
        }
        let read = |file_row: u64| -> Option<Cow<'_, str>> {
            match doc.set.row_text(file_row) {
                Some(text) => Some(Cow::Borrowed(text)),
                None => doc.set.read_row(file_row).map(Cow::Owned),
            }
        };
        let raw = read(file_row)?;
        let Some(layout) = doc.layout.as_ref() else {
            return Some(raw);
        };
        let next = if layout.format.body_next_line && doc.filtering.records_only {
            read(file_row + 1)
        } else {
            None
        };
        Some(Cow::Owned(
            layout.present_record(&raw, next.as_deref()).text,
        ))
    }
}

/// A selection endpoint, in cells, as a text position in bytes.
pub fn pos_of(doc: &Document, at: Position) -> Pos {
    let byte = DocText(doc)
        .line(at.row)
        .map_or(0, |line| doc.view.cells().byte_at_cell(&line, at.cell));
    Pos::new(at.row, byte)
}

/// A text position as a selection endpoint in cells.
pub fn position_of(doc: &Document, pos: Pos) -> Position {
    let cell = DocText(doc)
        .line(pos.row)
        .map_or(0, |line| doc.view.cells().cell_at_byte(&line, pos.byte));
    Position::new(pos.row, cell)
}

/// The selection as a text range — or, with nothing selected, the caret as a degenerate range at the
/// start of the current row. `None` for a document with no rows.
///
/// **The caret is the current row**, `Document::current_row`, because that is the row every row-wise
/// command acts on and the one the painter rules; a screen reader told a different row would be told
/// about a row the next command will not touch.
pub fn selection_range(doc: &Document) -> Option<(Pos, Pos)> {
    if doc.view_rows() == 0 {
        return None;
    }
    match doc.selection {
        Some(selection) => Some((pos_of(doc, selection.start()), pos_of(doc, selection.end()))),
        None => {
            let caret = Pos::new(doc.current_row()?, 0);
            Some((caret, caret))
        }
    }
}

/// The rows on screen as one range, from the first visible row's start to the last one's end.
pub fn visible_range(doc: &Document) -> Option<(Pos, Pos)> {
    let grid = doc.view.grid();
    let first = grid.visible().next()?.row;
    let last = grid.visible().last()?.row;
    let len = DocText(doc).line(last).map_or(0, |line| line.len());
    Some((Pos::new(first, 0), Pos::new(last, len)))
}

/// The rectangle of each visible line a range covers, pane-relative, in pixels: `(x, y, w, h)`.
///
/// **Where the painter drew it**: after the gutter, below the top inset, from the range's first cell
/// to its last, clipped to the body. A row the range only touches — ending at that row's very
/// start — is not one of its lines, and a row off the screen has no rectangle, which is what
/// Microsoft's `GetBoundingRectangles` asks for: the visible lines only.
pub fn line_rects(doc: &Document, start: Pos, end: Pos) -> Vec<(f32, f32, f32, f32)> {
    let (start, end) = if start <= end {
        (start, end)
    } else {
        (end, start)
    };
    let view = &doc.view;
    let (top, gutter) = (view.top_inset(), view.gutter_px());
    let body_right = gutter + view.hgrid().viewport_px();
    let row_h = view.grid().row_height();
    let text = DocText(doc);
    view.grid()
        .visible()
        .filter(|placed| placed.row >= start.row && placed.row <= end.row)
        .filter(|placed| !(placed.row == end.row && end.byte == 0 && placed.row != start.row))
        .filter_map(|placed| {
            let line = text.line(placed.row)?;
            let cells = view.cells();
            let from = if placed.row == start.row {
                cells.cell_at_byte(&line, start.byte)
            } else {
                0
            };
            let to = if placed.row == end.row {
                cells.cell_at_byte(&line, end.byte)
            } else {
                cells.cell_count(&line)
            };
            let left = (gutter + view.hgrid().x_of_column(from)).max(gutter);
            let right = (gutter + view.hgrid().x_of_column(to)).min(body_right);
            Some((left, top + placed.y, (right - left).max(0.0), row_h))
        })
        .collect()
}

/// The position a pane-relative point lands on, or `None` outside the rows — the position a click
/// there would put the caret at, as `RangeFromPoint` asks.
pub fn pos_at(doc: &Document, x: f32, y: f32) -> Option<Pos> {
    let at = doc.view.position_at(x, y)?;
    Some(pos_of(doc, at))
}

/// The position a pane-relative point lands on, or the nearest one to it — what `RangeFromPoint`
/// answers with, since it must answer with a position.
pub fn pos_at_or_nearest(doc: &Document, x: f32, y: f32) -> Option<Pos> {
    if let Some(at) = pos_at(doc, x, y) {
        return Some(at);
    }
    let (first, last) = visible_range(doc)?;
    let top = doc.view.top_inset();
    Some(match doc.view.grid().row_at_y(y - top) {
        // The row is on screen and the point missed it sideways — the gutter, or past the line's
        // end — so it is that row's start.
        Some(row) => Pos::new(row, 0),
        None if y >= top => last,
        None => first,
    })
}

/// Rows a text search reads at most. A `FindText` over the whole document range of a large file
/// would otherwise read the file on the window's thread; a search that has not found its text in a
/// hundred thousand rows answers "not found" rather than freezing the window.
pub const FIND_ROWS: u64 = 100_000;

/// The first occurrence of `needle` inside `start..end` — the last, when `backward` — for
/// `FindText`.
///
/// **One row at a time**: a log's lines are its records, and a match across a line break is not
/// one a reader of a log is looking for. Case is folded for ASCII only, which keeps every byte offset
/// where it was. A row that changed since the range was clamped is skipped rather than sliced where
/// it no longer has a boundary.
pub fn find(
    doc: &Document,
    start: Pos,
    end: Pos,
    needle: &str,
    backward: bool,
    ignore_case: bool,
) -> Option<(Pos, Pos)> {
    if needle.is_empty() {
        return None;
    }
    let text = DocText(doc);
    let (start, end) = (textunit::clamp(&text, start), textunit::clamp(&text, end));
    let (start, end) = if start <= end {
        (start, end)
    } else {
        (end, start)
    };
    let needle = if ignore_case {
        needle.to_ascii_lowercase()
    } else {
        needle.to_owned()
    };
    let in_row = |row: u64| -> Option<(Pos, Pos)> {
        let line = text.line(row)?;
        let from = if row == start.row { start.byte } else { 0 };
        let to = if row == end.row { end.byte } else { line.len() };
        let hay = line.get(from..to)?;
        let hay: Cow<'_, str> = if ignore_case {
            Cow::Owned(hay.to_ascii_lowercase())
        } else {
            Cow::Borrowed(hay)
        };
        let at = if backward {
            hay.rfind(needle.as_str())
        } else {
            hay.find(needle.as_str())
        }?;
        Some((
            Pos::new(row, from + at),
            Pos::new(row, from + at + needle.len()),
        ))
    };
    if backward {
        let first = start.row.max(end.row.saturating_sub(FIND_ROWS - 1));
        (first..=end.row).rev().find_map(in_row)
    } else {
        let last = end.row.min(start.row.saturating_add(FIND_ROWS - 1));
        (start.row..=last).find_map(in_row)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tailhawk_core::rows::RowSource;
    use tailhawk_core::selection::Selection;
    use tailhawk_core::textunit;

    /// **A search reads the range, off the screen too, in either direction**, folding case only
    /// when asked, and never past the range's end.
    #[test]
    fn find_looks_inside_the_range_in_either_direction() {
        let doc = document("tailhawk_gridtext_find.log", 300);
        let (first, last) = (Pos::new(0, 0), textunit::end(&DocText(&doc)));
        assert_eq!(
            find(&doc, first, last, "line 250", false, false),
            Some((Pos::new(250, 0), Pos::new(250, 8))),
            "off the screen"
        );
        assert_eq!(
            find(&doc, first, last, "LINE 7 ", false, true),
            Some((Pos::new(7, 0), Pos::new(7, 7)))
        );
        assert_eq!(find(&doc, first, last, "LINE 7 ", false, false), None);
        assert_eq!(
            find(&doc, first, last, "record", true, false).map(|(at, _)| at.row),
            Some(299),
            "backward finds the last"
        );
        assert_eq!(
            find(&doc, Pos::new(3, 0), Pos::new(3, 4), "line 3", false, false),
            None,
            "not past the range's end"
        );
    }

    const CELL: (f32, f32) = (8.0, 10.0);

    fn document(name: &str, lines: usize) -> Document {
        let path = std::env::temp_dir().join(name);
        let mut text = String::new();
        for i in 0..lines {
            text.push_str(&format!("line {i} — a log record with some width to it\n"));
        }
        std::fs::write(&path, text).expect("write the fixture");
        let mut doc = Document::open(&path).expect("open");
        doc.lay_out(CELL, (800, 200));
        doc.view.grid_mut().scroll_to_row(0);
        doc.lay_out(CELL, (800, 200));
        doc
    }

    fn row(i: usize) -> String {
        format!("line {i} — a log record with some width to it")
    }

    /// **A point outside the rows lands on the nearest line, not always the first.** A client
    /// asking about the bottom of the pane means the bottom; the gutter beside a row means that
    /// row.
    #[test]
    fn a_point_outside_the_rows_lands_on_the_nearest_line() {
        let doc = document("tailhawk_gridtext_nearest.log", 300);
        let (top, gutter) = (doc.view.top_inset(), doc.view.gutter_px());
        let (first, last) = visible_range(&doc).expect("rows on screen");
        assert_eq!(
            pos_at_or_nearest(&doc, gutter + 1.0, top + 15.0),
            Some(Pos::new(1, 0)),
            "inside the rows it is where the point is"
        );
        assert_eq!(
            pos_at_or_nearest(&doc, 1.0, top + 25.0),
            Some(Pos::new(2, 0)),
            "the gutter beside row 2 is the start of row 2"
        );
        assert_eq!(
            pos_at_or_nearest(&doc, gutter + 1.0, top + 5_000.0),
            Some(last),
            "below every row is the end of the last"
        );
        assert_eq!(
            pos_at_or_nearest(&doc, gutter + 1.0, top - 50.0),
            Some(first),
            "above them is the start of the first"
        );
    }

    fn document_of(name: &str, lines: &[&str]) -> Document {
        let path = std::env::temp_dir().join(name);
        std::fs::write(&path, lines.join("\n") + "\n").expect("write the fixture");
        let mut doc = Document::open(&path).expect("open");
        doc.lay_out(CELL, (800, 200));
        doc
    }

    /// **Backward takes the last match in a row, and folding case folds both sides.** Both of these
    /// were mutations that survived the test above: it had one match per row, so a backward search
    /// reading forward still answered with the right row, and its text was already lower case, so
    /// folding only the needle was enough.
    #[test]
    fn find_takes_the_last_match_backward_and_folds_both_sides() {
        let doc = document_of(
            "tailhawk_gridtext_find_edges.log",
            &["alpha ERROR alpha", "beta"],
        );
        let (first, last) = (Pos::new(0, 0), Pos::new(1, 4));
        assert_eq!(
            find(&doc, first, last, "alpha", true, false),
            Some((Pos::new(0, 12), Pos::new(0, 17))),
            "backward is the second alpha, not the first"
        );
        assert_eq!(
            find(&doc, first, last, "alpha", false, false),
            Some((Pos::new(0, 0), Pos::new(0, 5)))
        );
        assert_eq!(
            find(&doc, first, last, "error", false, true),
            Some((Pos::new(0, 6), Pos::new(0, 11))),
            "the row's own case is folded too"
        );
        assert_eq!(find(&doc, first, last, "error", false, false), None);
    }

    /// **The text is every view row, on the screen or off it, and reading one off the screen does
    /// not move the screen.** A screen reader reading down a document asks for rows the painter
    /// never fetched; the painter's window must still hold the rows it draws afterwards.
    #[test]
    fn the_text_is_every_row_and_reading_off_screen_leaves_the_screen_alone() {
        let doc = document("tailhawk_gridtext_rows.log", 300);
        let text = DocText(&doc);
        assert_eq!(text.rows(), 300);
        assert_eq!(text.line(0).as_deref(), Some(row(0).as_str()));
        assert_eq!(
            text.line(250).as_deref(),
            Some(row(250).as_str()),
            "off the screen, read on demand"
        );
        assert_eq!(text.line(300), None, "past the end");
        assert_eq!(
            doc.row_text(0),
            Some(row(0).as_str()),
            "the painter's window still holds the first row"
        );
        assert_eq!(
            doc.row_text(250),
            None,
            "and was not refilled with the one read"
        );
    }

    /// **A selection's cells are a range's bytes.** The em dash is one cell and three bytes, so a
    /// column past it is not the same number as a byte past it — and the caret, with nothing
    /// selected, is the start of the current row, which is what every row-wise command acts on.
    #[test]
    fn a_selection_in_cells_is_a_range_in_bytes_and_the_caret_is_the_current_row() {
        let mut doc = document("tailhawk_gridtext_selection.log", 40);
        assert_eq!(pos_of(&doc, Position::new(0, 8)), Pos::new(0, 10));
        assert_eq!(position_of(&doc, Pos::new(0, 10)), Position::new(0, 8));
        assert_eq!(
            selection_range(&doc),
            Some((Pos::new(0, 0), Pos::new(0, 0))),
            "no selection: the caret at the current row"
        );
        doc.selection = Some(Selection::stream(Position::new(0, 8), Position::new(0, 5)));
        assert_eq!(
            selection_range(&doc),
            Some((Pos::new(0, 5), Pos::new(0, 10))),
            "in document order, whichever way it was dragged"
        );
        doc.selection = Some(Selection::at(Position::new(3, 2)));
        assert_eq!(
            selection_range(&doc),
            Some((Pos::new(3, 2), Pos::new(3, 2)))
        );
    }

    /// **The visible range is the rows the grid placed**, first to last, whole.
    #[test]
    fn the_visible_range_runs_from_the_first_placed_row_to_the_end_of_the_last() {
        let doc = document("tailhawk_gridtext_visible.log", 300);
        let last = doc
            .view
            .grid()
            .visible()
            .last()
            .expect("rows on screen")
            .row;
        let len = row(last as usize).len();
        assert_eq!(
            visible_range(&doc),
            Some((Pos::new(0, 0), Pos::new(last, len)))
        );
    }

    /// **A line's rectangle is where the painter drew it**: after the gutter, from the range's first
    /// cell to its last, one per visible row — and none for a row the range only touches at its
    /// start, or one off the screen.
    #[test]
    fn a_ranges_rectangles_are_its_visible_lines_where_the_painter_drew_them() {
        let doc = document("tailhawk_gridtext_rects.log", 300);
        let gutter = doc.view.gutter_px();
        let top = doc.view.top_inset();
        assert_eq!(
            line_rects(&doc, Pos::new(1, 0), Pos::new(1, 4)),
            vec![(gutter, top + 10.0, 32.0, 10.0)]
        );
        let across = line_rects(&doc, Pos::new(1, 5), Pos::new(3, 0));
        assert_eq!(
            across.len(),
            2,
            "rows 1 and 2; row 3 is only touched: {across:?}"
        );
        assert_eq!(across[0].0, gutter + 40.0, "row 1 from its fifth cell");
        assert_eq!(across[1].0, gutter, "row 2 whole");
        assert!(line_rects(&doc, Pos::new(250, 0), Pos::new(251, 0)).is_empty());
    }

    /// **A point lands where a click would**: the row under it and the cell's first byte.
    #[test]
    fn a_point_lands_on_the_position_a_click_would() {
        let doc = document("tailhawk_gridtext_point.log", 300);
        let gutter = doc.view.gutter_px();
        let top = doc.view.top_inset();
        assert_eq!(
            pos_at(&doc, gutter + 8.0 * 8.0 + 1.0, top + 20.0 + 1.0),
            Some(Pos::new(2, 10)),
            "the ninth cell of row 2 is the space after the em dash"
        );
        assert_eq!(pos_at(&doc, 1.0, top + 1.0), None, "the gutter is not text");
    }
}
