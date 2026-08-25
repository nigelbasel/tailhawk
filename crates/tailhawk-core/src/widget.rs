//! The text field — V14's core, `SPEC.md` §3.1's seam kept: the model is here, the window is not.
//!
//! Every field the UI has been typing into the title bar — the find query, a filter chip — becomes
//! a [`TextField`]: text, a caret, an anchor for the selection, an undo stack, and an IME
//! composition span. The shell feeds it keys and characters and asks it what to draw; the clipboard
//! is the shell's, so cut and paste hand strings across rather than touching Win32 from here.
//!
//! ## The caret lands on grapheme boundaries and nowhere else
//!
//! A caret between the two halves of `é` written as `e` + U+0301, or between the flag's two
//! regional indicators, is a caret that deletes half a character. Every move and every delete
//! here steps by **grapheme cluster** (UAX #29, `unicode-segmentation`), which is also what
//! `cell.rs` measures in — so a caret drawn at cell *n* is at the boundary the user sees.
//!
//! ## Undo is snapshots, coalesced by run
//!
//! A one-line field does not need an operation log. [`TextField::undo`] restores the `(text,
//! caret)` before the current *run* — a run being consecutive insertions with nothing else between
//! them — so `Ctrl+Z` after typing a word takes the word, not one letter, and after a paste takes
//! the paste. Anything that is not an insertion ends the run.
//!
//! ## Composition is a span, not committed text
//!
//! While an IME is composing, the in-progress string is shown in place with a mark under it and is
//! **not** in [`text`](TextField::text) — a search must not run over half a Japanese word. It is
//! committed by [`commit_composition`](TextField::commit_composition) and cleared by
//! [`clear_composition`](TextField::clear_composition); the shell drives both from `WM_IME_*`.

use unicode_segmentation::UnicodeSegmentation;

/// A single-line text field's state.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TextField {
    text: String,
    /// Byte offset of the caret, always on a grapheme boundary of `text`.
    caret: usize,
    /// The other end of the selection, when there is one; `None` is a plain caret.
    anchor: Option<usize>,
    /// IME text in progress and where it sits, if composing. See the module note.
    composition: Option<Composition>,
    undo: Vec<(String, usize)>,
    redo: Vec<(String, usize)>,
    /// Whether the last edit was an insertion, so the next one joins its undo run.
    typing: bool,
}

/// An IME composition in flight.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Composition {
    /// Where in `text` it will be inserted.
    pub at: usize,
    pub text: String,
    /// The IME's caret within `text`, in bytes.
    pub caret: usize,
}

/// A caret movement.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Move {
    Left,
    Right,
    WordLeft,
    WordRight,
    Home,
    End,
}

impl TextField {
    pub fn new(text: impl Into<String>) -> Self {
        let text = text.into();
        Self {
            caret: text.len(),
            text,
            ..Self::default()
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn caret(&self) -> usize {
        self.caret
    }

    /// The selected byte range, ascending, if anything is selected.
    pub fn selection(&self) -> Option<core::ops::Range<usize>> {
        let anchor = self.anchor?;
        if anchor == self.caret {
            return None;
        }
        Some(anchor.min(self.caret)..anchor.max(self.caret))
    }

    pub fn selected_text(&self) -> Option<&str> {
        self.selection().map(|r| &self.text[r])
    }

    pub fn composition(&self) -> Option<&Composition> {
        self.composition.as_ref()
    }

    /// The text as it should be drawn: with any composition spliced in at its place.
    pub fn display(&self) -> String {
        match &self.composition {
            Some(c) => {
                let mut s = String::with_capacity(self.text.len() + c.text.len());
                s.push_str(&self.text[..c.at]);
                s.push_str(&c.text);
                s.push_str(&self.text[c.at..]);
                s
            }
            None => self.text.clone(),
        }
    }

    /// The caret's byte offset in [`display`](Self::display) — inside the composition while one is
    /// in flight, so the IME's own cursor is where the user sees the caret.
    pub fn display_caret(&self) -> usize {
        match &self.composition {
            Some(c) => c.at + c.caret,
            None => self.caret,
        }
    }

    /// The composition's byte range in [`display`](Self::display), for the mark under it.
    pub fn display_composition(&self) -> Option<core::ops::Range<usize>> {
        self.composition.as_ref().map(|c| c.at..c.at + c.text.len())
    }

    /// Replaces the whole text and puts the caret at the end. Not undoable — this is `set`, not
    /// an edit the user made.
    pub fn set_text(&mut self, text: impl Into<String>) {
        self.text = text.into();
        self.caret = self.text.len();
        self.anchor = None;
        self.composition = None;
        self.typing = false;
    }

    /// Inserts at the caret, replacing any selection. Joins the current typing run for undo.
    pub fn insert(&mut self, s: &str) {
        if s.is_empty() {
            return;
        }
        if !self.typing {
            self.snapshot();
            self.typing = true;
        }
        self.delete_selection_no_snapshot();
        self.text.insert_str(self.caret, s);
        self.caret += s.len();
    }

    /// One typed character.
    pub fn insert_char(&mut self, c: char) {
        let mut buf = [0u8; 4];
        self.insert(c.encode_utf8(&mut buf));
    }

    /// Backspace: the selection, or the grapheme before the caret.
    pub fn backspace(&mut self) {
        self.edit(|f| {
            if f.delete_selection_no_snapshot() {
                return;
            }
            let start = f.prev_boundary(f.caret);
            f.text.replace_range(start..f.caret, "");
            f.caret = start;
        });
    }

    /// Delete: the selection, or the grapheme after the caret.
    pub fn delete(&mut self) {
        self.edit(|f| {
            if f.delete_selection_no_snapshot() {
                return;
            }
            let end = f.next_boundary(f.caret);
            f.text.replace_range(f.caret..end, "");
        });
    }

    /// Moves the caret; with `extend`, keeps or starts a selection from where it was.
    pub fn move_caret(&mut self, m: Move, extend: bool) {
        self.typing = false;
        if extend {
            self.anchor.get_or_insert(self.caret);
        } else if let Some(sel) = self.selection() {
            // A plain move out of a selection collapses to its edge, as every field does.
            self.anchor = None;
            self.caret = match m {
                Move::Left | Move::WordLeft | Move::Home => sel.start,
                Move::Right | Move::WordRight | Move::End => sel.end,
            };
            if matches!(m, Move::Home | Move::End) {
                self.caret = if m == Move::Home { 0 } else { self.text.len() };
            }
            return;
        } else {
            self.anchor = None;
        }
        self.caret = match m {
            Move::Left => self.prev_boundary(self.caret),
            Move::Right => self.next_boundary(self.caret),
            Move::WordLeft => self.prev_word(self.caret),
            Move::WordRight => self.next_word(self.caret),
            Move::Home => 0,
            Move::End => self.text.len(),
        };
        if self.anchor == Some(self.caret) {
            self.anchor = None;
        }
    }

    /// Puts the caret at a byte offset (snapped to a boundary), as a click does; with `extend`,
    /// as a shift-click or a drag does.
    pub fn place(&mut self, at: usize, extend: bool) {
        self.typing = false;
        let at = self.snap(at.min(self.text.len()));
        if extend {
            self.anchor.get_or_insert(self.caret);
        } else {
            self.anchor = None;
        }
        self.caret = at;
        if self.anchor == Some(self.caret) {
            self.anchor = None;
        }
    }

    pub fn select_all(&mut self) {
        self.typing = false;
        if self.text.is_empty() {
            return;
        }
        self.anchor = Some(0);
        self.caret = self.text.len();
    }

    /// Cut: returns the selected text for the clipboard, having removed it. `None` if nothing was
    /// selected, in which case nothing changed.
    pub fn cut(&mut self) -> Option<String> {
        let selected = self.selected_text()?.to_owned();
        self.edit(|f| {
            f.delete_selection_no_snapshot();
        });
        Some(selected)
    }

    /// Paste: inserts, as its own undo step.
    pub fn paste(&mut self, s: &str) {
        self.typing = false;
        self.edit(|f| {
            f.delete_selection_no_snapshot();
            f.text.insert_str(f.caret, s);
            f.caret += s.len();
        });
    }

    pub fn undo(&mut self) -> bool {
        self.typing = false;
        let Some((text, caret)) = self.undo.pop() else {
            return false;
        };
        self.redo.push((std::mem::take(&mut self.text), self.caret));
        self.text = text;
        self.caret = caret.min(self.text.len());
        self.anchor = None;
        true
    }

    pub fn redo(&mut self) -> bool {
        self.typing = false;
        let Some((text, caret)) = self.redo.pop() else {
            return false;
        };
        self.undo.push((std::mem::take(&mut self.text), self.caret));
        self.text = text;
        self.caret = caret.min(self.text.len());
        self.anchor = None;
        true
    }

    /// The IME's in-progress string, shown at the caret and not yet part of the text.
    pub fn set_composition(&mut self, text: &str, caret: usize) {
        self.typing = false;
        let at = match &self.composition {
            Some(c) => c.at,
            None => {
                // A composition replaces the selection, as typing would.
                self.edit(|f| {
                    f.delete_selection_no_snapshot();
                });
                self.caret
            }
        };
        self.composition = Some(Composition {
            at,
            text: text.to_owned(),
            caret: caret.min(text.len()),
        });
    }

    /// The IME finished: `text` is what it settled on, inserted where the composition sat.
    pub fn commit_composition(&mut self, text: &str) {
        let at = self.composition.take().map_or(self.caret, |c| c.at);
        self.caret = at;
        self.anchor = None;
        if !text.is_empty() {
            self.edit(|f| {
                f.text.insert_str(f.caret, text);
                f.caret += text.len();
            });
        }
    }

    /// The IME cancelled: nothing was typed.
    pub fn clear_composition(&mut self) {
        self.composition = None;
    }

    // ---- internals ----

    /// A non-typing edit: its own undo step.
    fn edit(&mut self, f: impl FnOnce(&mut Self)) {
        self.snapshot();
        self.typing = false;
        f(self);
        self.anchor = None;
    }

    fn snapshot(&mut self) {
        self.undo.push((self.text.clone(), self.caret));
        self.redo.clear();
        if self.undo.len() > 200 {
            self.undo.remove(0);
        }
    }

    fn delete_selection_no_snapshot(&mut self) -> bool {
        let Some(sel) = self.selection() else {
            self.anchor = None;
            return false;
        };
        self.text.replace_range(sel.clone(), "");
        self.caret = sel.start;
        self.anchor = None;
        true
    }

    fn prev_boundary(&self, at: usize) -> usize {
        self.text[..at]
            .grapheme_indices(true)
            .next_back()
            .map_or(0, |(i, _)| i)
    }

    fn next_boundary(&self, at: usize) -> usize {
        self.text[at..]
            .graphemes(true)
            .next()
            .map_or(at, |g| at + g.len())
    }

    fn prev_word(&self, at: usize) -> usize {
        let before = &self.text[..at];
        let trimmed = before.trim_end();
        match trimmed.unicode_word_indices().next_back() {
            Some((i, _)) => i,
            None => 0,
        }
    }

    fn next_word(&self, at: usize) -> usize {
        let after = &self.text[at..];
        let mut words = after.unicode_word_indices();
        match words.next() {
            Some((0, w)) => {
                // In a word: go to its end, then over the space to the next word's start.
                let end = at + w.len();
                let rest = &self.text[end..];
                let skip = rest.len() - rest.trim_start().len();
                end + skip
            }
            Some((i, _)) => at + i,
            None => self.text.len(),
        }
    }

    /// The greatest grapheme boundary at or before `at`.
    fn snap(&self, at: usize) -> usize {
        let mut last = 0;
        for (i, g) in self.text.grapheme_indices(true) {
            if i > at {
                break;
            }
            last = i;
            if i + g.len() <= at {
                last = i + g.len();
            }
        }
        last
    }
}

/// The tail of `text` that fits in `width` cells, on a cluster boundary, and how many bytes were cut
/// from the front. A one-line field cuts from the left so its caret end stays on screen.
pub fn fit_from_left<'a>(
    cells: &crate::cell::CellModel,
    text: &'a str,
    width: usize,
) -> (&'a str, usize) {
    if cells.cell_count(text) <= width {
        return (text, 0);
    }
    let mut start = text.len();
    for (i, _) in text.grapheme_indices(true) {
        if cells.cell_count(&text[i..]) <= width {
            start = i;
            break;
        }
    }
    (&text[start..], start)
}

/// The head of `text` that fits in `width` cells, on a cluster boundary — a status line cut from
/// the right, where the least-changing part is.
pub fn fit_from_right<'a>(cells: &crate::cell::CellModel, text: &'a str, width: usize) -> &'a str {
    if cells.cell_count(text) <= width {
        return text;
    }
    let mut end = 0;
    for (i, g) in text.grapheme_indices(true) {
        if cells.cell_count(&text[..i + g.len()]) > width {
            break;
        }
        end = i + g.len();
    }
    &text[..end]
}

/// Which surface has the keyboard. The grid is the default; a field takes it while it is edited.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum Focus {
    #[default]
    Grid,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typing_moves_the_caret_and_backspace_takes_a_whole_grapheme() {
        let mut f = TextField::default();
        for c in "ae\u{301}b".chars() {
            f.insert_char(c);
        }
        assert_eq!(f.text(), "ae\u{301}b");
        assert_eq!(f.caret(), f.text().len());
        f.backspace();
        assert_eq!(f.text(), "ae\u{301}");
        f.backspace();
        assert_eq!(
            f.text(),
            "a",
            "e + combining acute is one grapheme, deleted whole"
        );
    }

    #[test]
    fn the_caret_never_lands_inside_a_cluster() {
        let mut f = TextField::new("x\u{1F1EC}\u{1F1E7}y"); // x 🇬🇧 y
        f.move_caret(Move::Home, false);
        f.move_caret(Move::Right, false);
        assert_eq!(f.caret(), 1);
        f.move_caret(Move::Right, false);
        assert_eq!(f.caret(), 1 + 8, "over the flag in one step");
        f.place(3, false);
        assert!(f.text().is_char_boundary(f.caret()));
        assert!(
            f.caret() == 1 || f.caret() == 9,
            "snapped to a boundary: {}",
            f.caret()
        );
    }

    #[test]
    fn selection_extends_collapses_and_is_replaced_by_typing() {
        let mut f = TextField::new("hello world");
        f.move_caret(Move::Home, false);
        f.move_caret(Move::WordRight, true);
        assert_eq!(f.selected_text(), Some("hello "));
        f.move_caret(Move::Right, false);
        assert_eq!(f.selection(), None, "a plain move collapses");
        assert_eq!(f.caret(), 6, "to the selection's right edge");
        f.select_all();
        f.insert("bye");
        assert_eq!(f.text(), "bye");
        f.move_caret(Move::Left, true);
        f.move_caret(Move::Left, true);
        assert_eq!(f.cut(), Some("ye".to_owned()));
        assert_eq!(f.text(), "b");
        f.paste("ye!");
        assert_eq!(f.text(), "bye!");
    }

    #[test]
    fn undo_takes_a_typing_run_and_a_paste_as_one_step_each() {
        let mut f = TextField::default();
        for c in "abc".chars() {
            f.insert_char(c);
        }
        f.paste(" def");
        assert_eq!(f.text(), "abc def");
        assert!(f.undo());
        assert_eq!(f.text(), "abc", "the paste");
        assert!(f.undo());
        assert_eq!(f.text(), "", "the whole typing run");
        assert!(f.redo());
        assert_eq!(f.text(), "abc");
        assert!(f.redo());
        assert_eq!(f.text(), "abc def");
        assert!(!f.redo());
        f.insert_char('!');
        assert!(!f.redo(), "an edit clears the redo stack");
    }

    #[test]
    fn a_composition_is_shown_but_not_in_the_text_until_committed() {
        let mut f = TextField::new("ab");
        f.move_caret(Move::Left, false);
        f.set_composition("にほ", 3);
        assert_eq!(f.text(), "ab", "not committed");
        assert_eq!(f.display(), "aにほb");
        assert_eq!(f.display_caret(), 1 + 3);
        assert_eq!(f.display_composition(), Some(1..7));
        f.set_composition("日本", 6);
        f.commit_composition("日本");
        assert_eq!(f.text(), "a日本b");
        assert_eq!(f.caret(), 1 + 6);
        assert_eq!(f.composition(), None);
        assert!(f.undo());
        assert_eq!(f.text(), "ab", "a committed composition is one undo step");

        f.set_composition("x", 1);
        f.clear_composition();
        assert_eq!(f.display(), "ab");
    }

    #[test]
    fn word_moves_step_over_words_and_the_space_after_them() {
        let mut f = TextField::new("foo bar  baz");
        f.move_caret(Move::Home, false);
        f.move_caret(Move::WordRight, false);
        assert_eq!(f.caret(), 4);
        f.move_caret(Move::WordRight, false);
        assert_eq!(f.caret(), 9);
        f.move_caret(Move::WordRight, false);
        assert_eq!(f.caret(), 12);
        f.move_caret(Move::WordLeft, false);
        assert_eq!(f.caret(), 9);
        f.move_caret(Move::WordLeft, false);
        assert_eq!(f.caret(), 4);
    }
}
