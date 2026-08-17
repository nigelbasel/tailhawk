//! The record detail pane — `UI-DESIGN.md` §8, composed into lines.
//!
//! "For very long lines, structured payloads and stack traces, a detail pane opens": the record's
//! number, its fields one under another, a rule, the body wrapped to the pane's width, and the
//! continuation lines under the body. Everything here is text in cells — the painter draws each
//! line with one `lay_out_at`, and the pane needs nothing the rows do not.
//!
//! ## Raw is always right; Pretty is a courtesy
//!
//! §8: "**Raw** shows the original bytes exactly as they appear in the file — always available…
//! **Pretty** JSON-formats a structured body." [`pretty_json`] is a **re-indenter, not a parser**:
//! it walks the text once, respects the string grammar of RFC 8259 (a `{` inside `"…"` is text, a
//! `\"` does not end the string) and puts a newline and an indent after every `{`, `[` and `,` and
//! before every `}` and `]`. Malformed JSON is re-indented as far as it goes rather than refused,
//! because a body the viewer cannot parse is still a body the user wants to read.

use crate::cell::CellModel;
use unicode_segmentation::UnicodeSegmentation;

/// One record, ready to compose.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Detail<'a> {
    /// The physical line number, one-based.
    pub line: u64,
    /// The format's fields, in column order, `(name, value)`. Empty without a format.
    pub fields: Vec<(&'a str, &'a str)>,
    /// The record's first line — the raw text when there is no format, the body field otherwise.
    pub body: &'a str,
    /// The continuation lines that belong to it, in file order.
    pub tail: Vec<&'a str>,
}

/// The lines the pane shows, top to bottom. `width` is the pane's width in cells; every line fits
/// it. `pretty` asks for a JSON body to be re-indented.
pub fn compose(detail: &Detail<'_>, width: usize, pretty: bool, cells: &CellModel) -> Vec<String> {
    let width = width.max(8);
    let mut out = Vec::new();
    out.push(format!("Record {}", with_separators(detail.line)));
    let name_width = detail
        .fields
        .iter()
        .map(|(n, _)| cells.cell_count(n))
        .max()
        .unwrap_or(0)
        .max(4);
    let indent = name_width + 2;
    for (name, value) in &detail.fields {
        push_labelled(&mut out, name, value, indent, width, cells);
    }
    if !detail.fields.is_empty() {
        out.push("─".repeat(width));
    }
    let body = if pretty {
        pretty_json(detail.body)
    } else {
        None
    };
    match body {
        Some(pretty) => {
            let mut first = true;
            for line in pretty.lines() {
                push_labelled(&mut out, if first { "Body" } else { "" }, line, indent, width, cells);
                first = false;
            }
        }
        None => push_labelled(&mut out, "Body", detail.body, indent, width, cells),
    }
    for line in &detail.tail {
        push_labelled(&mut out, "", line, indent, width, cells);
    }
    out
}

/// A `Name  value` row, the value wrapped to the width with a hanging indent under itself.
fn push_labelled(
    out: &mut Vec<String>,
    name: &str,
    value: &str,
    indent: usize,
    width: usize,
    cells: &CellModel,
) {
    let avail = width.saturating_sub(indent).max(1);
    let mut first = true;
    for piece in wrap(value, avail, cells) {
        let mut line = String::new();
        if first {
            line.push_str(name);
            let pad = indent.saturating_sub(cells.cell_count(name));
            line.extend(std::iter::repeat_n(' ', pad));
        } else {
            line.extend(std::iter::repeat_n(' ', indent));
        }
        line.push_str(&piece);
        out.push(line);
        first = false;
    }
}

/// Hard-wraps `text` into pieces of at most `width` cells, on grapheme boundaries. An empty text
/// is one empty piece, so a field with no value still gets its row.
pub fn wrap(text: &str, width: usize, cells: &CellModel) -> Vec<String> {
    let width = width.max(1);
    let mut pieces = Vec::new();
    let mut current = String::new();
    let mut used = 0usize;
    for g in text.graphemes(true) {
        let w = cells.cell_count(g);
        if used + w > width && !current.is_empty() {
            pieces.push(std::mem::take(&mut current));
            used = 0;
        }
        current.push_str(g);
        used += w;
    }
    pieces.push(current);
    pieces
}

/// §8's *Pretty*: the text re-indented as JSON, if it looks like a JSON object or array. `None`
/// otherwise — the caller shows the raw body. See the module note for what this is and is not.
pub fn pretty_json(text: &str) -> Option<String> {
    let trimmed = text.trim();
    if !(trimmed.starts_with('{') || trimmed.starts_with('[')) {
        return None;
    }
    let mut out = String::with_capacity(trimmed.len() * 2);
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    let newline = |out: &mut String, depth: usize| {
        out.push('\n');
        out.extend(std::iter::repeat_n(' ', depth * 2));
    };
    let mut chars = trimmed.chars().peekable();
    while let Some(c) = chars.next() {
        if in_string {
            out.push(c);
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
            }
            continue;
        }
        match c {
            '"' => {
                in_string = true;
                out.push(c);
            }
            '{' | '[' => {
                out.push(c);
                // An empty `{}` or `[]` stays on its line.
                let closes = matches!((c, chars.peek()), ('{', Some('}')) | ('[', Some(']')));
                if closes {
                    out.push(chars.next().expect("peeked"));
                } else {
                    depth += 1;
                    newline(&mut out, depth);
                }
            }
            '}' | ']' => {
                depth = depth.saturating_sub(1);
                newline(&mut out, depth);
                out.push(c);
            }
            ',' => {
                out.push(c);
                newline(&mut out, depth);
            }
            ':' => out.push_str(": "),
            c if c.is_whitespace() => {}
            c => out.push(c),
        }
    }
    Some(out)
}

/// `4182995` → `4,182,995`, as §8's mock-up writes the record number.
fn with_separators(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_record_composes_into_fields_a_rule_the_body_and_its_tail() {
        let cells = CellModel::default();
        let detail = Detail {
            line: 4_182_995,
            fields: vec![("Timestamp", "2026-07-28T09:14:03"), ("Level", "ERROR")],
            body: "Failed to dispatch job 41982",
            tail: vec!["   at Dispatcher.Dispatch(Job job)", "   at Worker.Run()"],
        };
        let lines = compose(&detail, 60, false, &cells);
        assert_eq!(lines[0], "Record 4,182,995");
        assert_eq!(lines[1], "Timestamp  2026-07-28T09:14:03");
        assert_eq!(lines[2], "Level      ERROR");
        assert_eq!(lines[3], "─".repeat(60));
        assert_eq!(lines[4], "Body       Failed to dispatch job 41982");
        assert_eq!(lines[5], "              at Dispatcher.Dispatch(Job job)");
        assert_eq!(lines[6], "              at Worker.Run()");
        assert_eq!(lines.len(), 7);
    }

    #[test]
    fn a_long_body_wraps_under_itself_and_no_line_exceeds_the_width() {
        let cells = CellModel::default();
        let detail = Detail {
            line: 1,
            fields: Vec::new(),
            body: &"x".repeat(100),
            tail: Vec::new(),
        };
        let lines = compose(&detail, 40, false, &cells);
        assert_eq!(lines[0], "Record 1");
        assert!(lines.len() > 3, "wrapped");
        assert!(lines.iter().all(|l| cells.cell_count(l) <= 40), "{lines:?}");
        assert!(lines[1].starts_with("Body  "));
        assert!(lines[2].starts_with("      x"), "hanging indent: {:?}", lines[2]);
        let joined: String = lines[1..].iter().map(|l| l.trim_start()).collect();
        assert_eq!(joined.trim_start_matches("Body").trim_start(), "x".repeat(100));
    }

    #[test]
    fn wrapping_is_by_cells_on_grapheme_boundaries() {
        let cells = CellModel::default();
        // A wide character takes two cells; the flag is one cluster of two scalars.
        let pieces = wrap("ab日本cd🇬🇧e", 4, &cells);
        assert_eq!(pieces, ["ab日", "本cd", "🇬🇧e"]);
        assert_eq!(wrap("", 10, &cells), [""]);
    }

    #[test]
    fn pretty_json_re_indents_and_leaves_strings_alone() {
        let text = r#"{"a":1,"b":[1,2,{"c":"x{y}"}],"d":{},"e":"q\"z"}"#;
        let pretty = pretty_json(text).expect("looks like json");
        assert_eq!(
            pretty,
            "{\n  \"a\": 1,\n  \"b\": [\n    1,\n    2,\n    {\n      \"c\": \"x{y}\"\n    }\n  ],\n  \"d\": {},\n  \"e\": \"q\\\"z\"\n}"
        );
        assert_eq!(pretty_json("plain text"), None);
        assert_eq!(pretty_json("  {\"unterminated\": [1, 2"), Some("{\n  \"unterminated\": [\n    1,\n    2".to_owned()), "as far as it goes");
    }

    #[test]
    fn a_pretty_body_takes_one_row_per_line_labelled_once() {
        let cells = CellModel::default();
        let detail = Detail {
            line: 12,
            fields: vec![("Level", "INFO")],
            body: r#"{"k":1}"#,
            tail: Vec::new(),
        };
        let lines = compose(&detail, 40, true, &cells);
        assert_eq!(&lines[3..], ["Body   {", "         \"k\": 1", "       }"]);
        let raw = compose(&detail, 40, false, &cells);
        assert_eq!(&raw[3..], [r#"Body   {"k":1}"#]);
    }

    #[test]
    fn record_numbers_get_thousands_separators() {
        assert_eq!(with_separators(0), "0");
        assert_eq!(with_separators(999), "999");
        assert_eq!(with_separators(1_000), "1,000");
        assert_eq!(with_separators(4_182_995), "4,182,995");
    }
}
