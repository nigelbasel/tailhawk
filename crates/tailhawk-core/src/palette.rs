//! The command palette — `UI-DESIGN.md` §9: "the single discovery surface, which is why the chrome
//! can stay thin."
//!
//! Every command the shell has is listed here **by name, with its key beside it**, so the palette
//! teaches the keys rather than replacing them. A query narrows the list; `Up`/`Down` move the
//! selection; `Enter` is the caller's to act on. The model is here and the window is not, as with
//! [`crate::widget`]: the shell owns the `Ctrl+K`, the overlay's pixels and what each command does.
//!
//! ## The match is a subsequence, ranked by how tightly it fits
//!
//! `tb` should find "Toggle bookmark" and `gtl` "Go to line". Each query character is looked for
//! after the previous one, case-folded; a label that has them all is a hit, and the **span** from
//! the first to the last hit ranks it — a query whose letters sit close together beats one whose
//! letters are strewn across the label. Ties keep the caller's order, which is the order the
//! palette is defined in and the order a user learns.
//!
//! ## A number is a line
//!
//! `UI-DESIGN.md` §12 gives `Ctrl+G` to "go to line". Rather than a second field, a query that is
//! only digits offers **Go to line *N*** first — the palette is where a typed number means a place.

use crate::widget::TextField;

/// A named command with the key that reaches it without the palette. `id` is the caller's — the
/// palette hands it back untouched when the entry is chosen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub id: usize,
    pub label: String,
    pub key: String,
}

impl Entry {
    pub fn new(id: usize, label: &str, key: &str) -> Self {
        Self {
            id,
            label: label.to_owned(),
            key: key.to_owned(),
        }
    }
}

/// What a chosen row means.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice {
    /// The [`Entry::id`] of a listed command.
    Command(usize),
    /// A one-based line, from a numeric query.
    GoToLine(u64),
}

/// A row as the painter should draw it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row<'a> {
    pub label: std::borrow::Cow<'a, str>,
    pub key: &'a str,
    pub selected: bool,
}

/// The palette's state: the query, the entries, what the query keeps of them, and the selection.
#[derive(Clone, Debug, Default)]
pub struct Palette {
    pub field: TextField,
    entries: Vec<Entry>,
    open: bool,
    /// The choices the current query offers, best first. Rebuilt by [`Palette::refresh`].
    shown: Vec<Choice>,
    selected: usize,
    /// The query the `shown` list was built from, so a repaint does not rebuild it.
    built_from: String,
}

/// The most rows a query shows; the rest are reachable by narrowing.
pub const MAX_ROWS: usize = 12;

impl Palette {
    pub fn new(entries: Vec<Entry>) -> Self {
        Self {
            entries,
            ..Self::default()
        }
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Opens with an empty query and the first command selected.
    pub fn open(&mut self) {
        self.open = true;
        self.field.set_text("");
        self.selected = 0;
        self.built_from.clear();
        self.built_from.push('\u{0}');
        self.refresh();
    }

    pub fn close(&mut self) {
        self.open = false;
    }

    /// Rebuilds the shown list from the query if the query changed since the last build. Call
    /// after the field was edited; cheap to call every frame.
    pub fn refresh(&mut self) {
        let query = self.field.text();
        if query == self.built_from {
            return;
        }
        self.built_from = query.to_owned();
        self.shown.clear();
        let trimmed = query.trim();
        if !trimmed.is_empty() && trimmed.bytes().all(|b| b.is_ascii_digit()) {
            if let Ok(n) = trimmed.parse::<u64>() {
                if n > 0 {
                    self.shown.push(Choice::GoToLine(n));
                }
            }
        }
        let mut ranked: Vec<(u32, usize)> = self
            .entries
            .iter()
            .enumerate()
            .filter_map(|(i, e)| score(trimmed, &e.label).map(|s| (s, i)))
            .collect();
        ranked.sort_by_key(|&(s, i)| (s, i));
        self.shown.extend(
            ranked
                .into_iter()
                .map(|(_, i)| Choice::Command(self.entries[i].id)),
        );
        self.shown.truncate(MAX_ROWS);
        self.selected = self.selected.min(self.shown.len().saturating_sub(1));
    }

    /// The rows to draw, best first, with the selection marked.
    pub fn rows(&self) -> Vec<Row<'_>> {
        self.shown
            .iter()
            .enumerate()
            .map(|(i, choice)| match choice {
                Choice::GoToLine(n) => Row {
                    label: format!("Go to line {n}").into(),
                    key: "Ctrl+G",
                    selected: i == self.selected,
                },
                Choice::Command(id) => {
                    let entry = self
                        .entries
                        .iter()
                        .find(|e| e.id == *id)
                        .expect("a shown id is an entry's");
                    Row {
                        label: entry.label.as_str().into(),
                        key: entry.key.as_str(),
                        selected: i == self.selected,
                    }
                }
            })
            .collect()
    }

    /// `Up` / `Down`: moves the selection, clamped to the list.
    pub fn move_selection(&mut self, delta: i32) {
        if self.shown.is_empty() {
            self.selected = 0;
            return;
        }
        let last = self.shown.len() - 1;
        self.selected = (self.selected as i64 + delta as i64).clamp(0, last as i64) as usize;
    }

    /// Selects a row directly — a click.
    pub fn select(&mut self, row: usize) {
        self.selected = row.min(self.shown.len().saturating_sub(1));
    }

    pub fn selected(&self) -> usize {
        self.selected
    }

    /// The choice `Enter` means, if the list has one.
    pub fn choice(&self) -> Option<Choice> {
        self.shown.get(self.selected).copied()
    }
}

/// Whether `query` is a case-folded subsequence of `label`, and how tightly: the span from the
/// first matched character to the last, plus the offset of the first — smaller is better. An
/// empty query matches everything at the label's position, so the defined order holds.
pub fn score(query: &str, label: &str) -> Option<u32> {
    let query: Vec<char> = query.chars().flat_map(char::to_lowercase).collect();
    if query.is_empty() {
        return Some(0);
    }
    let label: Vec<char> = label.chars().flat_map(char::to_lowercase).collect();
    let mut at = 0usize;
    let mut first = None;
    let mut last = 0usize;
    for &q in &query {
        let found = label[at..].iter().position(|&c| c == q)? + at;
        first.get_or_insert(found);
        last = found;
        at = found + 1;
    }
    let first = first?;
    Some(((last - first + 1) * 4 + first) as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn palette() -> Palette {
        Palette::new(vec![
            Entry::new(1, "Open file…", "Ctrl+O"),
            Entry::new(2, "Toggle bookmark", "Ctrl+D"),
            Entry::new(3, "Table of columns", ""),
            Entry::new(4, "Go to line…", "Ctrl+G"),
            Entry::new(5, "Toggle follow", "F"),
        ])
    }

    #[test]
    fn an_empty_query_lists_everything_in_the_defined_order() {
        let mut p = palette();
        p.open();
        let labels: Vec<_> = p.rows().iter().map(|r| r.label.to_string()).collect();
        assert_eq!(
            labels,
            [
                "Open file…",
                "Toggle bookmark",
                "Table of columns",
                "Go to line…",
                "Toggle follow"
            ]
        );
        assert!(p.rows()[0].selected);
        assert_eq!(p.choice(), Some(Choice::Command(1)));
    }

    #[test]
    fn a_subsequence_matches_and_the_tighter_fit_ranks_first() {
        let mut p = palette();
        p.open();
        p.field.set_text("tb");
        p.refresh();
        let labels: Vec<_> = p.rows().iter().map(|r| r.label.to_string()).collect();
        assert_eq!(
            labels,
            ["Table of columns", "Toggle bookmark"],
            "t·a·b vs t·oggle·b"
        );
        p.field.set_text("tobo");
        p.refresh();
        let labels: Vec<_> = p.rows().iter().map(|r| r.label.to_string()).collect();
        assert_eq!(labels, ["Toggle bookmark"]);
        p.field.set_text("xyz");
        p.refresh();
        assert!(p.rows().is_empty());
        assert_eq!(p.choice(), None);
    }

    #[test]
    fn the_match_folds_case() {
        assert!(score("OPEN", "open file").is_some());
        assert!(score("öf", "Öffnen Fenster").is_some());
        assert_eq!(score("", "anything"), Some(0));
        assert_eq!(score("ab", "ba"), None, "order matters");
    }

    #[test]
    fn digits_offer_go_to_line_first() {
        let mut p = palette();
        p.open();
        p.field.set_text("120");
        p.refresh();
        let rows = p.rows();
        assert_eq!(rows[0].label, "Go to line 120");
        assert_eq!(rows[0].key, "Ctrl+G");
        assert_eq!(p.choice(), Some(Choice::GoToLine(120)));
        p.field.set_text("0");
        p.refresh();
        assert!(p.rows().is_empty(), "there is no line 0");
    }

    #[test]
    fn the_selection_moves_within_the_list_and_survives_a_narrowing() {
        let mut p = palette();
        p.open();
        p.move_selection(-1);
        assert_eq!(p.selected(), 0, "clamped at the top");
        p.move_selection(3);
        assert_eq!(p.selected(), 3);
        p.move_selection(10);
        assert_eq!(p.selected(), 4, "clamped at the bottom");
        p.field.set_text("toggle");
        p.refresh();
        assert_eq!(p.rows().len(), 2);
        assert_eq!(p.selected(), 1, "pulled back into the shorter list");
        p.select(0);
        assert_eq!(p.choice(), Some(Choice::Command(2)));
    }

    #[test]
    fn opening_resets_the_query_and_the_selection() {
        let mut p = palette();
        p.open();
        p.field.set_text("toggle");
        p.refresh();
        p.move_selection(1);
        p.close();
        assert!(!p.is_open());
        p.open();
        assert!(p.is_open());
        assert_eq!(p.field.text(), "");
        assert_eq!(p.selected(), 0);
        assert_eq!(p.rows().len(), 5);
    }
}
