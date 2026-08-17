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

/// Column widths for one format, measured on a sample.
#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    pub format: &'static Format,
    /// One per [`Format::columns`]; the last is 0, meaning "the rest of the row".
    pub widths: Vec<usize>,
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
        Self { format, widths }
    }

    /// The header line: each column's name padded to its width, in the row's own cells, so it sits
    /// over the values. Built once; the layout does not change while a file is open.
    pub fn header(&self) -> String {
        let mut text = String::new();
        let last = self.widths.len().saturating_sub(1);
        for (i, name) in self.format.columns.iter().enumerate() {
            let name = column_title(name);
            if i == last {
                text.push_str(name);
                break;
            }
            if self.widths[i] == 0 {
                continue;
            }
            let take = cut_to_cells(&CellModel::new(), name, self.widths[i]);
            text.push_str(take);
            for _ in CellModel::new().cell_count(take)..self.widths[i] + GAP {
                text.push(' ');
            }
        }
        text
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
                for (i, field) in fields.iter().enumerate() {
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

/// A column's title from its capture name: the three the format model understands get their
/// long names, anything else is shown as written.
fn column_title(name: &str) -> &str {
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
}
