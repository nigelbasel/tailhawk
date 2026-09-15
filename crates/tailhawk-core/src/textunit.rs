//! UI Automation's text units over the view's rows — the pure half of the grid's text provider,
//! `SPEC.md` §14.1's second part and `UI-DESIGN.md` §13.
//!
//! **A screen reader reads a document by asking for ranges of it**: the line the caret is on, the
//! word after it, the next page. Microsoft's Text pattern puts those questions as two positions and
//! seven units, and every rule about where a unit begins and ends is here, over a trait that hands out
//! one line at a time — so the rules are tested over a vector of strings, and the provider in
//! `main.rs` only resolves them against a live document.
//!
//! **The text is the rows joined by line breaks.** Between one row and the next is one break, which
//! counts as one character: `(row, len)` is the position before it and `(row + 1, 0)` the one after.
//! The last row has no break after it.
//!
//! **A position is a row and a byte offset on a grapheme boundary**, the coordinates the cell model
//! and the selection already speak, rather than a UTF-16 index that would be a third to keep in step.

use std::borrow::Cow;

use unicode_segmentation::UnicodeSegmentation;

/// The rows a text range is resolved against.
///
/// `line` may read the file for a row off the screen, so every function here asks for as few rows
/// as its answer needs.
pub trait Lines {
    /// How many rows there are.
    fn rows(&self) -> u64;
    /// Row `row`'s text, without its line break — `None` past the end, or where it cannot be read,
    /// which every function here treats as an empty row.
    fn line(&self, row: u64) -> Option<Cow<'_, str>>;
}

/// A point between two characters of the view's text.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Pos {
    pub row: u64,
    pub byte: usize,
}

impl Pos {
    pub const fn new(row: u64, byte: usize) -> Self {
        Self { row, byte }
    }
}

/// The first position of any text.
pub const START: Pos = Pos::new(0, 0);

/// UI Automation's text units. `Page` carries the rows a page holds — a screenful, which only the
/// caller knows.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Unit {
    Character,
    Format,
    Word,
    Line,
    Paragraph,
    Page(u64),
    Document,
}

/// The last position of the text: the end of the last row, or [`START`] when there are no rows.
pub fn end<L: Lines + ?Sized>(lines: &L) -> Pos {
    match lines.rows().checked_sub(1) {
        Some(last) => Pos::new(last, row_text(lines, last).len()),
        None => START,
    }
}

/// `pos` moved inside the text: a row past the end to the text's end, a byte past its row's end to
/// that end, and a byte inside a grapheme cluster back to the cluster's start.
///
/// Every public function clamps what it is given first, because a client can hand back any
/// position a range once had and the rows may have changed since.
pub fn clamp<L: Lines + ?Sized>(lines: &L, pos: Pos) -> Pos {
    clamp_in(lines, end(lines), pos)
}

/// [`clamp`] against an end the caller has already read.
fn clamp_in<L: Lines + ?Sized>(lines: &L, last: Pos, pos: Pos) -> Pos {
    if pos.row > last.row {
        return last;
    }
    let line = row_text(lines, pos.row);
    if pos.byte >= line.len() {
        return Pos::new(pos.row, line.len());
    }
    let start = line
        .grapheme_indices(true)
        .map(|(at, _)| at)
        .take_while(|at| *at <= pos.byte)
        .last()
        .unwrap_or(0);
    Pos::new(pos.row, start)
}

/// The unit enclosing `start` — `ExpandToEnclosingUnit`.
///
/// **Normalised by the start alone**, which is Microsoft's rule: the start moves back to the
/// beginning of its unit and the end goes to that unit's far boundary, however long the range was.
/// A start already on a boundary takes the unit that follows it — except at the very end of the
/// text, where nothing follows and the unit before is the only one there is.
///
/// **Unless the text ends with an empty row.** A row's start always starts a line, a paragraph and
/// a word, and a page when one falls there, so an empty last row is one of each — empty, and the
/// answer at the end of the text — rather than the tail of the row above it.
pub fn enclosing<L: Lines + ?Sized>(lines: &L, start: Pos, unit: Unit) -> (Pos, Pos) {
    let last = end(lines);
    let start = clamp_in(lines, last, start);
    let empty_last_unit = start == last
        && last.row > 0
        && last.byte == 0
        && match unit {
            Unit::Word | Unit::Line | Unit::Paragraph => true,
            Unit::Page(rows) => last.row.is_multiple_of(rows.max(1)),
            Unit::Character | Unit::Format | Unit::Document => false,
        };
    if empty_last_unit {
        return (last, last);
    }
    let first = if start == last || !is_boundary(lines, last, start, unit) {
        prev_boundary(lines, start, unit).unwrap_or(START)
    } else {
        start
    };
    (
        first,
        next_boundary(lines, last, first, unit).unwrap_or(first),
    )
}

/// `pos` moved across `count` unit boundaries, forward when `count` is positive —
/// `MoveEndpointByUnit`. Answers where it landed and how many boundaries it crossed, signed, which
/// is fewer than asked when it reached either end of the text.
pub fn move_endpoint<L: Lines + ?Sized>(lines: &L, pos: Pos, unit: Unit, count: i32) -> (Pos, i32) {
    let last = end(lines);
    let mut at = clamp_in(lines, last, pos);
    let mut moved = 0;
    while moved != count {
        let step = if count > 0 {
            next_boundary(lines, last, at, unit)
        } else {
            prev_boundary(lines, at, unit)
        };
        let Some(next) = step else {
            break;
        };
        at = next;
        moved += count.signum();
    }
    (at, moved)
}

/// A range moved by `count` units — `Move`.
///
/// **Normalised to the unit first, then moved a whole unit at a time**, and never past the last
/// unit, so the range still holds one. A degenerate range is the caret and stays degenerate, moving
/// as an endpoint does; a count of zero moves nothing.
pub fn move_range<L: Lines + ?Sized>(
    lines: &L,
    start: Pos,
    finish: Pos,
    unit: Unit,
    count: i32,
) -> (Pos, Pos, i32) {
    let last = end(lines);
    let (start, finish) = (clamp_in(lines, last, start), clamp_in(lines, last, finish));
    if count == 0 {
        return (start, finish, 0);
    }
    if start == finish {
        let (at, moved) = move_endpoint(lines, start, unit, count);
        return (at, at, moved);
    }
    let (mut first, _) = enclosing(lines, start, unit);
    let mut moved = 0;
    while moved != count {
        let step = if count > 0 {
            next_boundary(lines, last, first, unit).filter(|next| *next < last)
        } else {
            prev_boundary(lines, first, unit)
        };
        let Some(next) = step else {
            break;
        };
        first = next;
        moved += count.signum();
    }
    (
        first,
        next_boundary(lines, last, first, unit).unwrap_or(first),
        moved,
    )
}

/// The text from `start` to `finish`, rows joined by `\n`, at most `max` UTF-16 code units long.
///
/// **The limit is in UTF-16 code units** because that is what the caller's string holds, and a
/// character that would not fit whole is left out rather than cut. It also bounds the work: a
/// document range over millions of rows reads only as many rows as the limit lets through.
pub fn text<L: Lines + ?Sized>(lines: &L, start: Pos, finish: Pos, max: usize) -> String {
    let (start, finish) = (clamp(lines, start), clamp(lines, finish));
    let (start, finish) = if start <= finish {
        (start, finish)
    } else {
        (finish, start)
    };
    let mut out = String::new();
    let mut units = 0usize;
    for row in start.row..=finish.row {
        let line = row_text(lines, row);
        let to = if row == finish.row {
            floor_char(&line, finish.byte)
        } else {
            line.len()
        };
        let from = if row == start.row {
            floor_char(&line, start.byte).min(to)
        } else {
            0
        };
        let brk = (row < finish.row).then_some("\n");
        for cluster in line[from..to].graphemes(true).chain(brk) {
            let width = cluster.encode_utf16().count();
            if units.saturating_add(width) > max {
                return out;
            }
            units += width;
            out.push_str(cluster);
        }
    }
    out
}

/// `byte` moved back onto a character boundary of `line`, and no further than its end.
///
/// **The row may not be the one the position was checked against.** A followed file can answer a
/// second read differently from the first, and a byte that was a boundary then can be the middle
/// of a character now; slicing there would panic.
fn floor_char(line: &str, byte: usize) -> usize {
    let mut at = byte.min(line.len());
    while !line.is_char_boundary(at) {
        at -= 1;
    }
    at
}

/// Row `row`'s text, or an empty row where there is none to read.
fn row_text<L: Lines + ?Sized>(lines: &L, row: u64) -> Cow<'_, str> {
    lines.line(row).unwrap_or(Cow::Borrowed(""))
}

/// Where the words of a line start: the line's own start, and every UAX #29 segment that is not
/// whitespace — so the whitespace after a word belongs to the word, as `TextUnit_Word` asks.
fn word_starts(line: &str) -> Vec<usize> {
    let mut starts = vec![0];
    starts.extend(
        line.split_word_bound_indices()
            .filter(|(at, word)| *at > 0 && !word.trim().is_empty())
            .map(|(at, _)| at),
    );
    starts
}

/// Whether `pos` is where a unit begins.
fn is_boundary<L: Lines + ?Sized>(lines: &L, last: Pos, pos: Pos, unit: Unit) -> bool {
    pos == START
        || prev_boundary(lines, pos, unit).and_then(|prev| next_boundary(lines, last, prev, unit))
            == Some(pos)
}

/// The first unit boundary after `pos`, which is the end of the text at the latest — `None` from
/// the end itself.
///
/// **`last` is the text's end, read once by the caller**, and a row is read here only for a
/// character or a word, whose boundaries are inside it. Every other unit's boundaries are row
/// numbers, so a screen reader's move of a million lines is a million additions and no reads.
fn next_boundary<L: Lines + ?Sized>(lines: &L, last: Pos, pos: Pos, unit: Unit) -> Option<Pos> {
    if pos >= last {
        return None;
    }
    let after_row = || {
        if pos.row < last.row {
            Pos::new(pos.row + 1, 0)
        } else {
            last
        }
    };
    Some(match unit {
        Unit::Character => row_text(lines, pos.row)
            .grapheme_indices(true)
            .map(|(at, cluster)| at + cluster.len())
            .find(|stop| *stop > pos.byte)
            .map_or_else(after_row, |stop| Pos::new(pos.row, stop)),
        Unit::Word => word_starts(&row_text(lines, pos.row))
            .into_iter()
            .find(|start| *start > pos.byte)
            .map_or_else(after_row, |start| Pos::new(pos.row, start)),
        Unit::Line | Unit::Paragraph => after_row(),
        Unit::Page(rows) => {
            let rows = rows.max(1);
            let next = pos.row - pos.row % rows + rows;
            if next <= last.row {
                Pos::new(next, 0)
            } else {
                last
            }
        }
        Unit::Format | Unit::Document => last,
    })
}

/// The last unit boundary before `pos`, which is the start of the text at the earliest — `None`
/// from the start itself.
///
/// Back across a line break, a word's boundary is the start of the last word of the row above,
/// because the break belongs to that word. Rows are read only for a character or a word, as in
/// [`next_boundary`].
fn prev_boundary<L: Lines + ?Sized>(lines: &L, pos: Pos, unit: Unit) -> Option<Pos> {
    if pos <= START {
        return None;
    }
    Some(match unit {
        Unit::Character if pos.byte > 0 => Pos::new(
            pos.row,
            row_text(lines, pos.row)
                .grapheme_indices(true)
                .map(|(at, _)| at)
                .take_while(|at| *at < pos.byte)
                .last()
                .unwrap_or(0),
        ),
        Unit::Character => Pos::new(pos.row - 1, row_text(lines, pos.row - 1).len()),
        Unit::Word if pos.byte > 0 => Pos::new(
            pos.row,
            word_starts(&row_text(lines, pos.row))
                .into_iter()
                .take_while(|start| *start < pos.byte)
                .last()
                .unwrap_or(0),
        ),
        Unit::Word => Pos::new(
            pos.row - 1,
            word_starts(&row_text(lines, pos.row - 1))
                .last()
                .copied()
                .unwrap_or(0),
        ),
        Unit::Line | Unit::Paragraph if pos.byte > 0 => Pos::new(pos.row, 0),
        Unit::Line | Unit::Paragraph => Pos::new(pos.row - 1, 0),
        Unit::Page(rows) => {
            let rows = rows.max(1);
            let page = pos.row - pos.row % rows;
            if pos.byte > 0 || pos.row > page {
                Pos::new(page, 0)
            } else {
                Pos::new(page - rows, 0)
            }
        }
        Unit::Format | Unit::Document => START,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Text(Vec<&'static str>);

    impl Lines for Text {
        fn rows(&self) -> u64 {
            self.0.len() as u64
        }

        fn line(&self, row: u64) -> Option<Cow<'_, str>> {
            self.0.get(row as usize).map(|l| Cow::Borrowed(*l))
        }
    }

    fn at(row: u64, byte: usize) -> Pos {
        Pos::new(row, byte)
    }

    /// **A character is a grapheme cluster, and a line break is one character.** `e` and a
    /// combining acute are one character a screen reader says once; the break between two rows is
    /// a position of its own on each side, so moving by characters past a row's end lands after the
    /// break rather than skipping it.
    #[test]
    fn a_character_is_a_grapheme_cluster_and_a_line_break_is_one() {
        let text = Text(vec!["e\u{301}x", "y"]);
        assert_eq!(end(&text), at(1, 1));
        assert_eq!(
            move_endpoint(&text, START, Unit::Character, 1),
            (at(0, 3), 1)
        );
        assert_eq!(
            move_endpoint(&text, at(0, 3), Unit::Character, 2),
            (at(1, 0), 2),
            "the break between the rows is a character"
        );
        assert_eq!(
            move_endpoint(&text, at(1, 0), Unit::Character, -1),
            (at(0, 4), -1)
        );
        assert_eq!(
            move_endpoint(&text, START, Unit::Character, 10),
            (at(1, 1), 4),
            "stops at the end and says how far it got"
        );
        assert_eq!(
            move_endpoint(&text, at(0, 3), Unit::Character, -5),
            (START, -1)
        );
        assert_eq!(
            enclosing(&text, at(0, 1), Unit::Character),
            (START, at(0, 3)),
            "inside a cluster is inside its character"
        );
        assert_eq!(
            enclosing(&text, at(0, 4), Unit::Character),
            (at(0, 4), at(1, 0))
        );
        assert_eq!(
            enclosing(&text, at(1, 1), Unit::Character),
            (at(1, 0), at(1, 1)),
            "at the very end there is no following unit, so it is the one before"
        );
    }

    /// **A word carries the break characters after it** — Microsoft's rule for `TextUnit_Word` —
    /// and the last word of a row carries the row's break, so no position is outside every word.
    /// Boundaries are UAX #29's, the cell model's own, so a date splits at its hyphens here exactly
    /// as a double-click splits it.
    #[test]
    fn a_word_carries_the_spaces_after_it_and_the_last_one_its_break() {
        let text = Text(vec!["2026-09-15 ERROR  disk full", "next"]);
        assert_eq!(
            enclosing(&text, at(0, 13), Unit::Word),
            (at(0, 11), at(0, 18))
        );
        assert_eq!(text_of(&text, at(0, 11), at(0, 18)), "ERROR  ");
        assert_eq!(
            enclosing(&text, at(0, 24), Unit::Word),
            (at(0, 23), at(1, 0)),
            "the row's last word runs to the start of the next row"
        );
        assert_eq!(
            move_endpoint(&text, at(0, 11), Unit::Word, 1),
            (at(0, 18), 1)
        );
        assert_eq!(
            move_endpoint(&text, at(0, 18), Unit::Word, 2),
            (at(1, 0), 2)
        );
        assert_eq!(
            move_endpoint(&text, at(1, 2), Unit::Word, -1),
            (at(1, 0), -1)
        );
        assert_eq!(
            move_endpoint(&text, at(1, 0), Unit::Word, -1),
            (at(0, 23), -1),
            "back across a break is the last word of the row before"
        );
        assert_eq!(enclosing(&text, at(1, 0), Unit::Word), (at(1, 0), at(1, 4)));
    }

    /// **A line is a row and its break; a paragraph is a line.** A record's continuation lines are
    /// already rows of their own, and collapsing them is the command that joins them.
    #[test]
    fn a_line_is_a_row_and_its_break_and_a_paragraph_is_a_line() {
        let text = Text(vec!["one", "two", "three"]);
        assert_eq!(enclosing(&text, at(1, 2), Unit::Line), (at(1, 0), at(2, 0)));
        assert_eq!(
            enclosing(&text, at(1, 2), Unit::Paragraph),
            (at(1, 0), at(2, 0))
        );
        assert_eq!(
            enclosing(&text, at(2, 1), Unit::Line),
            (at(2, 0), at(2, 5)),
            "the last row has no break"
        );
        assert_eq!(move_endpoint(&text, at(0, 1), Unit::Line, 1), (at(1, 0), 1));
        assert_eq!(
            move_endpoint(&text, at(0, 1), Unit::Line, 5),
            (at(2, 5), 3),
            "the end of the text is the last boundary"
        );
        assert_eq!(
            move_endpoint(&text, at(2, 3), Unit::Line, -1),
            (at(2, 0), -1)
        );
        assert_eq!(
            move_endpoint(&text, at(2, 3), Unit::Line, -2),
            (at(1, 0), -2)
        );
    }

    /// **A page is the screenful the caller names**, counted from the first row so a page is the
    /// same rows whichever row asks; **the document and a format run are everything** — the
    /// provider exposes no text attributes, so all of the text shares them.
    #[test]
    fn a_page_is_the_callers_screenful_and_a_format_run_is_the_document() {
        let text = Text(vec!["a", "b", "c", "d", "e"]);
        assert_eq!(
            enclosing(&text, at(3, 0), Unit::Page(2)),
            (at(2, 0), at(4, 0))
        );
        assert_eq!(
            enclosing(&text, at(4, 0), Unit::Page(2)),
            (at(4, 0), at(4, 1))
        );
        assert_eq!(move_endpoint(&text, START, Unit::Page(2), 1), (at(2, 0), 1));
        assert_eq!(
            enclosing(&text, at(1, 1), Unit::Document),
            (START, at(4, 1))
        );
        assert_eq!(enclosing(&text, at(1, 1), Unit::Format), (START, at(4, 1)));
        assert_eq!(
            move_endpoint(&text, at(2, 0), Unit::Document, -3),
            (START, -1),
            "one step reaches the start, and one is all it counts"
        );
    }

    /// **`Move` normalises the range to the unit, then moves it**, and it cannot move past the last
    /// unit: a line range asked to go five lines on from the first of three goes two. A degenerate
    /// range — the caret — stays degenerate, and a count of zero moves nothing.
    #[test]
    fn a_range_moves_by_whole_units_and_the_caret_stays_a_caret() {
        let text = Text(vec!["one", "two", "three"]);
        assert_eq!(
            move_range(&text, at(0, 1), at(0, 2), Unit::Line, 1),
            (at(1, 0), at(2, 0), 1)
        );
        assert_eq!(
            move_range(&text, at(1, 0), at(1, 0), Unit::Line, 1),
            (at(2, 0), at(2, 0), 1)
        );
        assert_eq!(
            move_range(&text, START, at(1, 0), Unit::Line, 5),
            (at(2, 0), at(2, 5), 2)
        );
        assert_eq!(
            move_range(&text, at(2, 0), at(2, 5), Unit::Line, -1),
            (at(1, 0), at(2, 0), -1)
        );
        assert_eq!(
            move_range(&text, at(0, 1), at(0, 2), Unit::Line, 0),
            (at(0, 1), at(0, 2), 0)
        );
    }

    /// **The text joins rows with `\n` and stops at `max`**, counted in UTF-16 code units because
    /// that is what the caller's string holds — and never cuts a character in two to meet it.
    #[test]
    fn the_text_joins_rows_with_breaks_and_stops_at_the_limit() {
        let text = Text(vec!["one", "two", "three"]);
        assert_eq!(text_of(&text, at(0, 1), at(2, 2)), "ne\ntwo\nth");
        assert_eq!(super::text(&text, at(0, 1), at(2, 2), 4), "ne\nt");
        let wide = Text(vec!["a😀b"]);
        assert_eq!(
            super::text(&wide, START, end(&wide), 2),
            "a",
            "the emoji is two code units and does not fit in the one left"
        );
    }

    /// **Positions are kept inside the text.** A client can hand back any position a range once
    /// had, and the text may have changed since.
    #[test]
    fn a_position_is_clamped_into_the_text_and_onto_a_cluster() {
        let text = Text(vec!["e\u{301}x"]);
        assert_eq!(clamp(&text, at(0, 2)), START, "inside the combining mark");
        assert_eq!(clamp(&text, at(0, 9)), at(0, 4));
        assert_eq!(clamp(&text, at(7, 0)), at(0, 4));
    }

    /// **An empty document has one position and no units**, and every question about it answers
    /// without panicking — a pipe that has not written anything yet is exactly this.
    #[test]
    fn an_empty_document_has_one_position_and_nothing_to_move_across() {
        let text = Text(vec![]);
        assert_eq!(end(&text), START);
        assert_eq!(enclosing(&text, START, Unit::Line), (START, START));
        assert_eq!(move_endpoint(&text, START, Unit::Character, 3), (START, 0));
        assert_eq!(
            move_range(&text, START, START, Unit::Word, -2),
            (START, START, 0)
        );
        assert_eq!(text_of(&text, START, START), "");
    }

    /// **An empty last row is a line of its own**, not the tail of the row before it — a log that
    /// ends with a blank line has a place for the caret there, and "read this line" must read
    /// nothing rather than the line above. It holds no character, so the character at the very end
    /// is still the break before it.
    #[test]
    fn an_empty_last_row_is_a_line_of_its_own() {
        let text = Text(vec!["one", ""]);
        assert_eq!(end(&text), at(1, 0));
        assert_eq!(enclosing(&text, at(1, 0), Unit::Line), (at(1, 0), at(1, 0)));
        assert_eq!(enclosing(&text, at(1, 0), Unit::Word), (at(1, 0), at(1, 0)));
        assert_eq!(enclosing(&text, at(0, 1), Unit::Line), (START, at(1, 0)));
        assert_eq!(
            enclosing(&text, at(1, 0), Unit::Character),
            (at(0, 3), at(1, 0)),
            "no character on it, so the break before it"
        );
    }

    /// **The limit never cuts a grapheme cluster in two**, which is the module's character: an `e`
    /// sent without its combining accent is a different letter to whoever hears it.
    #[test]
    fn the_limit_leaves_out_a_cluster_that_does_not_fit_whole() {
        let text = Text(vec!["e\u{301}x"]);
        assert_eq!(super::text(&text, START, end(&text), 1), "");
        assert_eq!(super::text(&text, START, end(&text), 2), "e\u{301}");
    }

    /// The edges a first review found untested: a range cannot move past the last line, a page
    /// steps back through a short last page, a word steps back into an empty row, and a row that
    /// starts with spaces has them as a word of their own, because a row's start is always a word's.
    #[test]
    fn the_edges_of_moving_by_line_page_and_word() {
        let three = Text(vec!["one", "two", "three"]);
        assert_eq!(
            move_range(&three, at(2, 0), at(2, 5), Unit::Line, 1),
            (at(2, 0), at(2, 5), 0)
        );
        let five = Text(vec!["a", "b", "c", "d", "e"]);
        assert_eq!(
            move_endpoint(&five, at(4, 1), Unit::Page(2), -1),
            (at(4, 0), -1)
        );
        assert_eq!(
            move_endpoint(&five, at(4, 1), Unit::Page(2), -2),
            (at(2, 0), -2)
        );
        let gap = Text(vec!["a", "", "b"]);
        assert_eq!(
            move_endpoint(&gap, at(2, 0), Unit::Word, -1),
            (at(1, 0), -1)
        );
        assert_eq!(move_endpoint(&gap, at(2, 0), Unit::Word, -2), (START, -2));
        let indented = Text(vec!["  ERROR"]);
        assert_eq!(enclosing(&indented, START, Unit::Word), (START, at(0, 2)));
    }

    /// Hands out every row as `row` and counts the reads.
    struct Counted {
        rows: u64,
        reads: std::cell::Cell<u64>,
    }

    impl Lines for Counted {
        fn rows(&self) -> u64 {
            self.rows
        }

        fn line(&self, _row: u64) -> Option<Cow<'_, str>> {
            self.reads.set(self.reads.get() + 1);
            Some(Cow::Borrowed("row"))
        }
    }

    /// **Moving by lines, pages or the document reads almost no rows.** In the provider a row off
    /// the screen is a file read, and a screen reader's "go to the end" is a line move of millions:
    /// the boundaries of those units are arithmetic on row numbers, and must stay so.
    #[test]
    fn moving_by_rows_is_arithmetic_and_reads_almost_nothing() {
        let lines = Counted {
            rows: 10_000,
            reads: std::cell::Cell::new(0),
        };
        assert_eq!(
            move_endpoint(&lines, at(0, 1), Unit::Line, 5_000),
            (at(5_000, 0), 5_000)
        );
        assert!(
            lines.reads.get() <= 4,
            "{} reads for a line move",
            lines.reads.get()
        );
        lines.reads.set(0);
        assert_eq!(
            enclosing(&lines, at(7_000, 1), Unit::Page(50)),
            (at(7_000, 0), at(7_050, 0))
        );
        assert!(
            lines.reads.get() <= 4,
            "{} reads for a page",
            lines.reads.get()
        );
        lines.reads.set(0);
        assert_eq!(
            enclosing(&lines, at(7_000, 1), Unit::Document),
            (START, at(9_999, 3))
        );
        assert!(
            lines.reads.get() <= 4,
            "{} reads for the document",
            lines.reads.get()
        );
    }

    /// Answers `ab` and `é` by turns, as a followed file can answer differently from one read to
    /// the next.
    struct Flicker(std::cell::Cell<bool>);

    impl Lines for Flicker {
        fn rows(&self) -> u64 {
            1
        }

        fn line(&self, _row: u64) -> Option<Cow<'_, str>> {
            let wide = self.0.replace(!self.0.get());
            Some(Cow::Borrowed(if wide { "é" } else { "ab" }))
        }
    }

    /// **A row that changes between two reads cannot make the text panic.** A position checked
    /// against one read of a followed row is sliced against the next, and a byte that was a boundary
    /// in `ab` is the middle of `é`.
    #[test]
    fn a_row_that_changes_between_reads_cannot_panic() {
        for first_wide in [false, true] {
            let lines = Flicker(std::cell::Cell::new(first_wide));
            let got = super::text(&lines, START, at(0, 1), usize::MAX);
            assert!(["", "a", "é", "ab"].contains(&got.as_str()), "{got:?}");
        }
    }

    fn text_of(lines: &Text, start: Pos, end: Pos) -> String {
        super::text(lines, start, end, usize::MAX)
    }
}
