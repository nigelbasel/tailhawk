//! Columns — V5, the visible half of `SPEC.md` §6: a detected format's fields laid out in aligned
//! columns, and continuations indented under the message.
//!
//! ## A presentation per visible row, not a second copy of the file
//!
//! A columnised row is the raw line's fields **rearranged and padded**, and the painter draws that
//! string instead of the raw one. It is built per *visible* row per frame — the same rule §7.1
//! gives highlighting — so a 10 GB file costs fifty parses a frame and no memory. `raw` is untouched
//! and still what a search runs over, what a filter sees, and what `Ctrl+C` copies.
//!
//! [`Presentation::segments`] is what makes that consistent: every field records where its bytes
//! came from in the raw line, so a search match — a byte range in the raw — is carried into the
//! presentation by [`Presentation::map`], and a match that spans two fields is drawn as two pieces.
//! A continuation line is one segment, indented under the message column.
//!
//! ## Widths come from the head sample
//!
//! [`Layout::from_sample`] measures each column over the lines detection already read, in cells,
//! and caps every column but the last at [`MAX_CELLS`]: a logger name of sixty characters would
//! otherwise push every message off screen for the sake of one row. The last column — the message
//! — is never padded and never capped. There is no header row yet, no resize, no reorder and no
//! hide: those are `UI-DESIGN.md` §6's controls and want M7's widget layer; the *layout* they will
//! act on is this one.

use crate::cell::CellModel;
use crate::format::Format;
use crate::highlight::Span;

/// The most cells a column other than the last may take.
pub const MAX_CELLS: usize = 48;

/// Cells between columns.
pub const GAP: usize = 2;

/// The natural column order, for a layout whose `order` is not usable; long enough for any format.
const NATURAL: [usize; 32] = {
    let mut a = [0usize; 32];
    let mut i = 0;
    while i < 32 {
        a[i] = i;
        i += 1;
    }
    a
};

/// Column widths for one format, measured on a sample.
#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    pub format: &'static Format,
    /// One per [`Format::columns`]; the last is 0, meaning "the rest of the row".
    pub widths: Vec<usize>,
    /// The display order of the columns before the last — indices into `widths` — after the user
    /// has dragged one; the message column stays last. `UI-DESIGN.md` §2.1's reorderable columns.
    pub order: Vec<usize>,
    /// The column the rows are sorted by and whether descending — E22 — for the header's mark.
    pub sort: Option<(usize, bool)>,
}

impl Layout {
    /// Measures `lines` — the detection sample — and sizes each column to its widest value.
    pub fn from_sample(format: &'static Format, lines: &[String]) -> Self {
        let cells = CellModel::new();
        let n = format.columns.len();
        // At least the title's width, so the header is never cut to fit its own column.
        let mut widths: Vec<usize> = format
            .columns
            .iter()
            .map(|name| cells.cell_count(column_title(name)).max(1))
            .collect();
        // A column no sampled line populated is left out entirely — MEL Simple without its
        // timestamp option would otherwise reserve eleven blank cells on every row.
        let mut seen = vec![false; n];
        for line in lines {
            let Some(fields) = format.fields(line) else {
                continue;
            };
            for (i, field) in fields.iter().enumerate() {
                if let Some(range) = field {
                    seen[i] = true;
                    let w = cells.cell_count(&line[range.clone()]);
                    widths[i] = widths[i].max(w);
                }
            }
        }
        for (i, w) in widths.iter_mut().enumerate().take(n.saturating_sub(1)) {
            *w = if seen[i] { (*w).min(MAX_CELLS) } else { 0 };
        }
        if let Some(last) = widths.last_mut() {
            *last = 0;
        }
        let order = (0..n.saturating_sub(1)).collect();
        Self {
            format,
            widths,
            order,
            sort: None,
        }
    }

    /// The header line: each column's name padded to its width, in the row's own cells, so it sits
    /// over the values. The sorted column, if any, carries `▲` or `▼` in the gap after its title —
    /// E22's affordance, on the column that has it and nowhere else, taking no room from the
    /// title. Rebuilt whenever a width, the order or the sort changes.
    pub fn header(&self) -> String {
        let mut text = String::new();
        let last = self.widths.len().saturating_sub(1);
        let title = |i: usize| self.title(i);
        let mark = |i: usize| match self.sort {
            Some((c, false)) if c == i => "▲",
            Some((c, true)) if c == i => "▼",
            _ => "",
        };
        let model = CellModel::new();
        for &i in self.shown_order() {
            if self.widths[i] == 0 {
                continue;
            }
            let take = cut_to_cells(&model, title(i), self.widths[i]);
            let mark = mark(i);
            text.push_str(take);
            text.push_str(mark);
            for _ in model.cell_count(take) + model.cell_count(mark)..self.widths[i] + GAP {
                text.push(' ');
            }
        }
        if !self.format.columns.is_empty() {
            text.push_str(title(last));
            text.push_str(mark(last));
        }
        text
    }

    /// Column `i`'s title as the header shows it: the format's own title when it declares one, the
    /// capture name spelt out otherwise (`ts` → `timestamp`).
    pub fn title(&self, i: usize) -> &'static str {
        match self.format.titles {
            Some(titles) => titles
                .get(i)
                .copied()
                .unwrap_or_else(|| self.format.columns.get(i).copied().unwrap_or("")),
            None => column_title(self.format.columns.get(i).copied().unwrap_or("")),
        }
    }

    /// Column `i`'s heading as a person reads it — see [`display_title`].
    ///
    /// **A key the format *understood* gets its readable name; one it merely found stays
    /// verbatim.** The owner, 2026-09-22, seeing `@t`, `@l` and `@m` across his Loki columns: "why
    /// is this?" Because Tailhawk wrote them. `lokiwire::clef_line` spills a window as Serilog
    /// CLEF (`LOKI.md` §4), whose fields *are* `@t`/`@l`/`@m`, and the detector then read that
    /// spill back and showed each key exactly as written — so the program was showing a reader its
    /// own wire shorthand, having already decided in `understood_key` that `@t` means a timestamp.
    ///
    /// Verbatim remains right for every other key, and that is the half worth keeping: `app`,
    /// `environment` and `pod` are the service's own words, a filter is written against them, and
    /// renaming them would be Tailhawk second-guessing a file it did not write.
    ///
    /// The capture name is what decides it, not the spelling: `json_columns` binds an understood
    /// key to the role's name — `ts`, `level`, `msg` — and leaves an invented one sanitised, so
    /// "did this format understand column `i`" is already answered before this is asked.
    pub fn display_title(&self, i: usize) -> String {
        let name = self.format.columns.get(i).copied().unwrap_or("");
        let understood = matches!(
            name,
            crate::format::TS | crate::format::LEVEL | crate::format::MSG
        );
        if self.format.titles.is_some() && understood {
            return display_title(column_title(name), false).into_owned();
        }
        display_title(self.title(i), self.format.titles.is_some()).into_owned()
    }

    /// The columns before the last, in display order — `order` when it names them all, the
    /// natural order otherwise (a layout built by hand, or an order from an older settings file).
    pub fn shown_order(&self) -> &[usize] {
        let n = self.widths.len().saturating_sub(1);
        if self.order.len() == n && (0..n).all(|i| self.order.contains(&i)) {
            &self.order
        } else {
            &NATURAL[..n.min(NATURAL.len())]
        }
    }

    /// Moves column `from` so it is drawn at display position `to` — a header drag. Reports a
    /// change. The message column cannot move.
    pub fn move_column(&mut self, from: usize, to: usize) -> bool {
        let n = self.widths.len().saturating_sub(1);
        if self.order.len() != n {
            self.order = (0..n).collect();
        }
        let Some(at) = self.order.iter().position(|&c| c == from) else {
            return false;
        };
        let to = to.min(n.saturating_sub(1));
        if at == to {
            return false;
        }
        let col = self.order.remove(at);
        self.order.insert(to, col);
        true
    }

    /// The cell at which the message column starts — where a continuation is indented to.
    pub fn message_indent(&self) -> usize {
        self.widths[..self.widths.len().saturating_sub(1)]
            .iter()
            .filter(|&&w| w > 0)
            .map(|w| w + GAP)
            .sum()
    }

    /// Cells the padding adds to a row at most, for the horizontal extent.
    pub fn extra_cells(&self) -> usize {
        self.message_indent()
    }

    /// [`present`](Self::present), with the record's body pulled in from `next` when the format
    /// keeps its message on the line after the first (MEL Simple, [`Format::body_next_line`]) and
    /// `next` is that line. §6.4's "assemble correctly", for a view that has collapsed the
    /// continuation the body would otherwise sit on. The body's segment points at *another* row's
    /// bytes, so it carries no span: a search hit in the message is on the hidden line.
    pub fn present_record(&self, raw: &str, next: Option<&str>) -> Presentation {
        let mut p = self.present(raw);
        if !self.format.body_next_line || p.continuation {
            return p;
        }
        let Some(body) = next.filter(|n| self.format.is_continuation(n)) else {
            return p;
        };
        p.text.push_str(body.trim_start());
        p
    }

    /// The presentation of one raw line: its fields in columns if it is a first line, indented
    /// under the message column if it is not.
    pub fn present(&self, raw: &str) -> Presentation {
        let cells = CellModel::new();
        let mut text = String::with_capacity(raw.len() + self.extra_cells());
        let mut segments = Vec::new();
        let mut continuation = false;
        match self.format.fields(raw) {
            Some(fields) => {
                let last = fields.len().saturating_sub(1);
                let order: Vec<usize> = self
                    .shown_order()
                    .iter()
                    .copied()
                    .chain(std::iter::once(last))
                    .collect();
                for i in order {
                    let field = &fields[i];
                    if i != last && self.widths[i] == 0 {
                        continue;
                    }
                    let start = text.len();
                    let mut used = 0usize;
                    if let Some(range) = field {
                        let value = &raw[range.clone()];
                        // A wide value in a capped column is cut to the column, per cell, so it
                        // cannot push the next column out of line.
                        let take = if i == last {
                            value
                        } else {
                            cut_to_cells(&cells, value, self.widths[i])
                        };
                        text.push_str(take);
                        used = cells.cell_count(take);
                        segments.push(Segment {
                            raw: range.start..range.start + take.len(),
                            at: start,
                        });
                    }
                    if i != last {
                        for _ in used..self.widths[i] + GAP {
                            text.push(' ');
                        }
                    }
                }
            }
            None => {
                continuation = true;
                let indent = self.message_indent();
                for _ in 0..indent {
                    text.push(' ');
                }
                let at = text.len();
                text.push_str(raw);
                segments.push(Segment {
                    raw: 0..raw.len(),
                    at,
                });
            }
        }
        Presentation {
            text,
            continuation,
            segments,
        }
    }
}

/// A heading as a person reads it: the first letter capitalised, unless the heading is `verbatim`.
///
/// **UX-REVIEW finding 14, settled by the owner on 2026-09-15.** *List Views* asks for
/// sentence-style capitalisation, and a heading from a format Tailhawk recognises gets it. A heading
/// that is the file's own spelling — a JSON key, a W3C `#Fields:` entry — is `verbatim` and is shown
/// exactly as written, because that spelling is what the file says and a filter is written against
/// it.
///
/// **Display only.** Nothing that matches a field — a filter's scope, a sort, copy-as-TSV — reads
/// this; they read [`Layout::title`], which is unchanged.
pub fn display_title(title: &str, verbatim: bool) -> std::borrow::Cow<'_, str> {
    let mut chars = title.chars();
    match chars.next() {
        Some(first) if !verbatim && !first.is_uppercase() => {
            std::borrow::Cow::Owned(first.to_uppercase().chain(chars).collect())
        }
        _ => std::borrow::Cow::Borrowed(title),
    }
}

/// A column's title from its capture name: the three the format model understands get their
/// long names, anything else is shown as written.
pub fn column_title(name: &str) -> &str {
    match name {
        "ts" => "timestamp",
        "level" => "level",
        "msg" => "message",
        other => other,
    }
}

/// The longest prefix of `value` that fits in `cells` cells, on a cluster boundary.
fn cut_to_cells<'a>(model: &CellModel, value: &'a str, cells: usize) -> &'a str {
    if model.cell_count(value) <= cells {
        return value;
    }
    let mut end = 0;
    let mut used = 0;
    for (i, cluster) in unicode_segmentation::UnicodeSegmentation::grapheme_indices(value, true) {
        let w = model.cluster_width(cluster);
        if used + w > cells {
            break;
        }
        used += w;
        end = i + cluster.len();
    }
    &value[..end]
}

/// Where a run of the presentation came from in the raw line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Segment {
    /// Bytes of the raw line.
    pub raw: core::ops::Range<usize>,
    /// Byte offset in the presentation where `raw.start` landed.
    pub at: usize,
}

/// One row as the painter draws it under a [`Layout`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Presentation {
    pub text: String,
    /// Whether this row is a continuation — not a first line — and so drawn dimmed (§6.4).
    pub continuation: bool,
    /// Ascending by `at`; the raw ranges are disjoint but need not be ascending — a format may
    /// present its columns in another order than the line writes them.
    pub segments: Vec<Segment>,
}

impl Presentation {
    /// Carries spans over raw bytes into spans over the presentation. A span crossing a field
    /// boundary becomes one piece per field; bytes the presentation dropped (a cut value, the
    /// separators a pattern skipped) are not drawn.
    pub fn map(&self, spans: &[Span], out: &mut Vec<Span>) {
        out.clear();
        for span in spans {
            for seg in &self.segments {
                let start = span.start.max(seg.raw.start);
                let end = span.end.min(seg.raw.end);
                if start < end {
                    out.push(Span {
                        start: seg.at + (start - seg.raw.start),
                        end: seg.at + (end - seg.raw.start),
                        fg: span.fg,
                        bg: span.bg,
                    });
                }
            }
        }
        out.sort_by_key(|s| s.start);
    }
}

/// The least a column may be squeezed to before it is hidden instead.
///
/// Six cells holds a level word and the ellipsis that says a value was cut. Below that a column is
/// noise occupying the room the message needs.
pub const MIN_CELLS: usize = 6;

/// The least the message column is left with, in cells.
///
/// The message is the reason the program exists, and a row showing every label and none of the
/// message has failed at its job.
pub const MIN_MESSAGE_CELLS: usize = 40;

impl Layout {
    /// Hides the columns a derived format merely found, keeping the ones it understood.
    ///
    /// A catalogue format's columns were chosen by hand and each earns its place. A format derived
    /// from the file's own keys has no such warrant: Loki telemetry wraps one message in a dozen
    /// identifier and label fields, and showing all of them is how the message ends up at the back
    /// of a queue nobody can see the end of. Timestamp and level are kept, the message is last and
    /// untouched, and everything else starts hidden — the Columns dialog is where a reader ticks
    /// back the ones they actually want.
    ///
    /// **This is not [`Layout::fit`] and does not replace it.** `fit` answers "does this row fit
    /// the window", which is a question about width; this answers "is this column worth a reader's
    /// attention by default", which is a question about the format. A wide monitor makes the first
    /// question go away and leaves the second exactly where it was.
    ///
    /// Reports whether anything changed. A format that did not invent its columns is left alone.
    pub fn only_understood(&mut self) -> bool {
        if !self.format.has_invented_columns() {
            return false;
        }
        let before = self.widths.clone();
        let last = self.widths.len().saturating_sub(1);
        for i in 0..last {
            let name = self.format.columns.get(i).copied().unwrap_or_default();
            if name != crate::format::TS && name != crate::format::LEVEL {
                self.widths[i] = 0;
            }
        }
        self.widths != before
    }

    /// Fits the columns before the message into `budget` cells, so the message always has room.
    ///
    /// **One algorithm doing two jobs, because they are the same job.** Wide columns are capped
    /// until the row fits; if it still will not fit with every column at [`MIN_CELLS`], trailing
    /// columns are hidden and the cap is recomputed over what remains. Trimming and hiding are the
    /// same decision taken at two depths, which is what lets a Loki telemetry line carrying four
    /// sixty-cell label columns show its labels *and* its message rather than neither.
    ///
    /// The cap is max-min fair — it takes from the widest first — so a four-cell level column is
    /// never trimmed to pay for a sixty-cell trace id. Columns already hidden stay hidden and cost
    /// nothing, and the message column is never given a width.
    ///
    /// **Hiding walks the display order, not the column order**, and takes from the far end: the
    /// column a reader's eye reaches last is the one they miss least. A layout whose columns have
    /// been dragged into a new order therefore loses a different column than an untouched one,
    /// which is the intended behaviour and is what `order` is for.
    ///
    /// The two searches are linear in the widest column, which is bounded by [`MAX_CELLS`] for any
    /// layout that came from [`Layout::from_sample`]. A width set from somewhere else — a drag —
    /// only makes the walk proportionally longer, never wrong.
    ///
    /// Reports whether anything changed.
    pub fn fit(&mut self, budget: usize) -> bool {
        let last = self.widths.len().saturating_sub(1);
        if last == 0 {
            return false;
        }
        let before = self.widths.clone();
        let available = budget.saturating_sub(MIN_MESSAGE_CELLS);
        let mut shown: Vec<usize> = self
            .shown_order()
            .iter()
            .copied()
            .filter(|i| *i < last && self.widths[*i] > 0)
            .collect();

        while let Some(&drop) = shown.last() {
            let floor: usize = shown
                .iter()
                .map(|i| MIN_CELLS.min(self.widths[*i]) + GAP)
                .sum();
            if floor <= available {
                break;
            }
            self.widths[drop] = 0;
            shown.pop();
        }

        if !shown.is_empty() {
            let room = available.saturating_sub(shown.len() * GAP);
            let mut cap = shown.iter().map(|i| self.widths[*i]).max().unwrap_or(0);
            while cap > 0 {
                let spent: usize = shown.iter().map(|i| self.widths[*i].min(cap)).sum();
                if spent <= room {
                    break;
                }
                cap -= 1;
            }
            for i in &shown {
                self.widths[*i] = self.widths[*i].min(cap);
            }
        }

        self.widths != before
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::by_id;

    fn serilog() -> Layout {
        let f = by_id("serilog-file").expect("catalogue");
        let sample: Vec<String> = f.samples.iter().map(|(l, _)| l.to_string()).collect();
        Layout::from_sample(f, &sample)
    }

    #[test]
    fn widths_come_from_the_sample_and_the_last_column_is_the_rest_of_the_row() {
        let layout = serilog();
        // "2026-08-16 09:14:02.117 +02:00" is 30 cells; the level column is its title's 5, wider
        // than the 3-letter values; the message is open.
        assert_eq!(layout.widths, [30, 5, 0]);
        assert_eq!(layout.message_indent(), 30 + GAP + 5 + GAP);
    }

    #[test]
    fn a_first_line_is_its_fields_in_columns_and_a_match_follows_its_field() {
        let layout = serilog();
        let raw = "2026-08-16 09:14:03.884 +02:00 [ERR] Failed to dispatch job 41982";
        let p = layout.present(raw);
        assert_eq!(
            p.text,
            "2026-08-16 09:14:03.884 +02:00  ERR    Failed to dispatch job 41982"
        );
        // A search for "ERR" hit raw bytes 32..35; on screen that is 32..35 too here, but a search
        // for "job 4" hit raw 55..60 and lands where the message column put it.
        let hit = raw.find("job 4").unwrap();
        let mut out = Vec::new();
        p.map(
            &[Span {
                start: hit,
                end: hit + 5,
                fg: None,
                bg: None,
            }],
            &mut out,
        );
        assert_eq!(out.len(), 1);
        assert_eq!(&p.text[out[0].start..out[0].end], "job 4");
        // A match across the "] " the pattern skipped is two pieces, neither drawing the skipped bytes.
        let hit = raw.find("ERR] Fail").unwrap();
        p.map(
            &[Span {
                start: hit,
                end: hit + 9,
                fg: None,
                bg: None,
            }],
            &mut out,
        );
        assert_eq!(out.len(), 2);
        assert_eq!(&p.text[out[0].start..out[0].end], "ERR");
        assert_eq!(&p.text[out[1].start..out[1].end], "Fail");
    }

    #[test]
    fn a_continuation_is_indented_under_the_message() {
        let layout = serilog();
        let p = layout.present("   at Api.Dispatch.Run()");
        assert_eq!(
            p.text,
            format!(
                "{}   at Api.Dispatch.Run()",
                " ".repeat(layout.message_indent())
            )
        );
        assert_eq!(p.segments.len(), 1);
        assert_eq!(p.segments[0].at, layout.message_indent());
    }

    #[test]
    fn a_value_wider_than_its_column_is_cut_and_the_next_column_stays_in_line() {
        let f = by_id("log4net").expect("catalogue");
        let sample = vec!["2026-08-16 09:14:02,117 [12] INFO  Api.Controller - Started".to_owned()];
        let layout = Layout::from_sample(f, &sample);
        let long_thread = format!(
            "2026-08-16 09:14:02,117 [{}] INFO  Api.Controller - Started",
            "t".repeat(20)
        );
        let p = layout.present(&long_thread);
        let level_at = p.text.find("INFO").unwrap();
        let expected = layout.widths[0] + GAP + layout.widths[1] + GAP;
        assert_eq!(level_at, expected, "{:?}", p.text);
    }

    /// **UX-REVIEW finding 14, as the owner settled it on 2026-09-15.** A heading from a format
    /// Tailhawk recognises is shown in sentence case — *List Views*: "Use sentence-style
    /// capitalization" — and a heading that is the file's own spelling is shown exactly as the file
    /// spells it, because a JSON key or a W3C field *is* what the file says and renaming it hides
    /// that.
    #[test]
    fn a_recognised_heading_is_capitalised_and_the_files_own_is_not() {
        assert_eq!(display_title("timestamp", false), "Timestamp");
        assert_eq!(display_title("level", false), "Level");
        assert_eq!(display_title("Level", false), "Level", "already right");
        assert_eq!(
            display_title("état", false),
            "État",
            "the first letter, not the first byte"
        );
        assert_eq!(
            display_title("ßcode", false),
            "SScode",
            "a letter whose capital is two letters — which is why the chooser sizes a column from \
             the heading it draws, not from the name"
        );
        assert_eq!(display_title("", false), "");
        assert_eq!(display_title("trace_id", true), "trace_id");
        assert_eq!(display_title("cs(User-Agent)", true), "cs(User-Agent)");
    }

    /// The layout knows which kind a heading is, from the format: a format carries `titles` exactly
    /// when its names are the file's own spelling — W3C's `#Fields:` and a JSON file's keys — and
    /// the catalogue's formats carry none.
    #[test]
    fn the_layout_shows_the_catalogues_names_capitalised_and_a_files_verbatim() {
        let nlog = by_id("nlog").expect("catalogue");
        let catalogue = Layout {
            format: nlog,
            widths: vec![10, 5, 8, 0],
            order: vec![0, 1, 2],
            sort: None,
        };
        assert_eq!(catalogue.display_title(0), "Timestamp");
        assert_eq!(catalogue.display_title(3), "Message");
        assert_eq!(
            catalogue.title(0),
            "timestamp",
            "the name a filter uses is not changed by how it is shown"
        );

        let fields: Vec<String> = ["date", "cs-uri-stem", "sc-status", "cs(User-Agent)"]
            .iter()
            .map(|f| (*f).to_owned())
            .collect();
        let w3c = crate::format::w3c(&fields);
        let file = Layout {
            format: w3c,
            widths: vec![10, 12, 3, 0],
            order: vec![0, 1, 2],
            sort: None,
        };
        assert_eq!(file.display_title(0), "date");
        assert_eq!(file.display_title(3), "cs(User-Agent)");
    }

    /// **A key the format understood is named; a key it merely found is left alone.**
    ///
    /// The owner asked on 2026-09-22 why his Loki columns were headed `@t`, `@l` and `@m`. Because
    /// Tailhawk wrote them: `lokiwire::clef_line` spills a window as Serilog CLEF, whose fields are
    /// exactly those, and the detector then read its own shorthand back and printed it. It had
    /// already mapped `@t` to a timestamp in `understood_key` — the name was there all along.
    ///
    /// The other half must not move with it. `app` and `pod` are the service's words, a filter is
    /// written against them, and Tailhawk renaming a key it does not understand would be guessing
    /// about a file it did not write.
    #[test]
    fn a_clef_key_is_named_and_a_services_own_key_is_not() {
        use crate::detect::{JsonKey, JsonValue};
        let keys: Vec<JsonKey> = ["@t", "@l", "app", "pod", "@m"]
            .iter()
            .map(|name| JsonKey {
                name: (*name).to_owned(),
                value: JsonValue::Text,
            })
            .collect();
        let format = crate::format::json_lines(&keys);
        let layout = Layout {
            format,
            widths: vec![0; format.columns.len()],
            order: (0..format.columns.len().saturating_sub(1)).collect(),
            sort: None,
        };
        let shown: Vec<String> = (0..format.columns.len())
            .map(|i| layout.display_title(i))
            .collect();
        assert!(shown.contains(&"Timestamp".to_owned()), "{shown:?}");
        assert!(shown.contains(&"Level".to_owned()), "{shown:?}");
        assert!(shown.contains(&"Message".to_owned()), "{shown:?}");
        assert!(shown.contains(&"app".to_owned()), "verbatim: {shown:?}");
        assert!(shown.contains(&"pod".to_owned()), "verbatim: {shown:?}");
        assert!(
            !shown.iter().any(|t| t.starts_with('@')),
            "no CLEF shorthand reaches a header: {shown:?}"
        );

        // **And the name a filter is written against is untouched**, which is the whole reason
        // this is display-only: `title` still answers the file's own spelling.
        let raw: Vec<&str> = (0..format.columns.len()).map(|i| layout.title(i)).collect();
        assert!(raw.contains(&"@t"), "{raw:?}");
    }

    /// A layout whose widths are dictated rather than measured, for the fitting tests.
    fn widths_of(widths: Vec<usize>) -> Layout {
        let n = widths.len();
        Layout {
            format: by_id("serilog-file").expect("catalogue"),
            widths,
            order: (0..n.saturating_sub(1)).collect(),
            sort: None,
        }
    }

    /// What the columns before the message cost the row, gaps included, as the row pays it.
    fn spent(layout: &Layout) -> usize {
        let last = layout.widths.len().saturating_sub(1);
        layout.widths[..last]
            .iter()
            .filter(|w| **w > 0)
            .map(|w| w + GAP)
            .sum()
    }

    #[test]
    fn a_row_of_wide_labels_leaves_the_message_its_minimum() {
        let mut layout = widths_of(vec![20, 30, 48, 48, 0]);
        assert!(layout.fit(100));
        // **The exact widths, not the budget bound.** "Spent no more than was available" is the
        // algorithm's own invariant and holds for any implementation that respects it — including
        // one that hides every label outright instead of capping them fairly. Four columns share
        // the 52 cells left after 40 for the message and 8 of gap, which is 13 each.
        assert_eq!(layout.widths, vec![13, 13, 13, 13, 0]);
        assert!(
            spent(&layout) + MIN_MESSAGE_CELLS <= 100,
            "spent {} of 100 with {MIN_MESSAGE_CELLS} reserved: {:?}",
            spent(&layout),
            layout.widths
        );
    }

    /// Hiding takes from the far end of the **display** order, which is not the column order once
    /// a column has been dragged. With the order reversed the columns that go are 0 and 1, where
    /// an implementation walking raw indices would have taken 3 and 2.
    #[test]
    fn hiding_follows_the_display_order_rather_than_the_column_order() {
        let mut layout = widths_of(vec![30, 30, 30, 30, 0]);
        layout.order = vec![3, 2, 1, 0];
        assert!(layout.fit(60));
        assert_eq!(layout.widths, vec![0, 0, 8, 8, 0]);
    }

    #[test]
    fn the_cap_takes_from_the_widest_and_leaves_narrow_columns_alone() {
        let mut layout = widths_of(vec![4, 6, 48, 48, 0]);
        assert!(layout.fit(100));
        assert_eq!(layout.widths, vec![4, 6, 21, 21, 0]);
    }

    #[test]
    fn columns_that_cannot_fit_even_squeezed_are_hidden_from_the_end() {
        let mut layout = widths_of(vec![30, 30, 30, 30, 30, 30, 0]);
        assert!(layout.fit(60));
        assert_eq!(layout.widths, vec![8, 8, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn a_row_that_already_fits_is_left_exactly_as_it_was() {
        let mut layout = widths_of(vec![10, 8, 0]);
        let before = layout.widths.clone();
        assert!(!layout.fit(100));
        assert_eq!(layout.widths, before);
    }

    #[test]
    fn a_hidden_column_stays_hidden_and_costs_the_row_nothing() {
        let mut layout = widths_of(vec![48, 0, 48, 0]);
        assert!(layout.fit(100));
        // **The exact widths, not a bound.** A hidden column stays at zero however the row is
        // accounted for, so asserting only that catches nothing; what it must not do is charge the
        // row a gap it never draws. Two shown columns pay 4 cells of gap and share the remaining
        // 56, which is 28 each — counting the hidden one would leave 54 and give 27.
        assert_eq!(layout.widths, vec![28, 0, 28, 0]);
        assert!(spent(&layout) + MIN_MESSAGE_CELLS <= 100);
    }

    #[test]
    fn the_message_column_is_never_given_a_width() {
        let mut layout = widths_of(vec![48, 48, 48, 0]);
        layout.fit(100);
        assert_eq!(
            layout.widths.last(),
            Some(&0),
            "the message column must stay on the rest of the row"
        );
    }

    /// A derived JSON format over the keys a telemetry record actually carries.
    fn telemetry() -> Layout {
        use crate::detect::{JsonKey, JsonValue};
        let keys: Vec<JsonKey> = [
            "timestamp",
            "level",
            "message",
            "traceId",
            "spanId",
            "service",
            "namespace",
            "pod",
        ]
        .iter()
        .map(|name| JsonKey {
            name: (*name).to_owned(),
            value: JsonValue::Text,
        })
        .collect();
        let f = crate::format::json_lines(&keys);
        // A sample is not optional here: `from_sample` leaves a column no sampled line populated
        // at zero, so an empty sample hides everything and the test would pass against an
        // implementation that does nothing at all.
        let sample = [concat!(
            r#"{"timestamp":"2026-09-21T08:14:02.117Z","level":"INFO","#,
            r#""message":"Dispatched job 41982","traceId":"4bf92f3577b34da6a3ce929d0e0e4736","#,
            r#""spanId":"00f067aa0ba902b7","service":"dispatch-api","namespace":"production","#,
            r#""pod":"dispatch-api-7d9f4c8b6d-x2k9v"}"#
        )
        .to_owned()];
        Layout::from_sample(f, &sample)
    }

    #[test]
    fn a_derived_format_starts_with_only_the_columns_it_understood() {
        let mut layout = telemetry();
        assert!(
            layout.widths[..layout.widths.len() - 1]
                .iter()
                .filter(|w| **w > 0)
                .count()
                > 2,
            "the fixture must start with invented columns showing, or it proves nothing"
        );
        assert!(layout.only_understood());
        let last = layout.widths.len() - 1;
        for (i, w) in layout.widths[..last].iter().enumerate() {
            let name = layout.format.columns[i];
            let understood = name == crate::format::TS || name == crate::format::LEVEL;
            assert_eq!(
                *w > 0,
                understood,
                "column {i} ({name}) shown={} but understood={understood}",
                *w > 0
            );
        }
        assert_eq!(
            layout.widths[last], 0,
            "the message keeps the rest of the row"
        );
    }

    #[test]
    fn a_catalogue_format_keeps_every_column_it_was_given() {
        // **`log4net`, not `serilog`.** Serilog's columns are `ts, level, msg` and nothing else,
        // so "keep only the understood ones" keeps all three and the test passes against a rule
        // applied to every format in the catalogue — which is precisely the mistake it is here to
        // catch. `log4net` carries `thread` and `logger`, which the rule would hide.
        let f = by_id("log4net").expect("catalogue");
        let sample: Vec<String> = f.samples.iter().map(|(l, _)| l.to_string()).collect();
        let mut layout = Layout::from_sample(f, &sample);
        let last = layout.widths.len() - 1;
        assert!(
            layout.widths[..last].iter().all(|w| *w > 0),
            "the fixture must start with every column showing: {:?}",
            layout.widths
        );
        let before = layout.widths.clone();
        assert!(!layout.only_understood());
        assert_eq!(layout.widths, before);
    }

    #[test]
    fn a_window_too_narrow_for_the_message_alone_hides_every_label() {
        let mut layout = widths_of(vec![10, 10, 0]);
        assert!(layout.fit(20));
        assert_eq!(layout.widths, vec![0, 0, 0]);
    }
}
