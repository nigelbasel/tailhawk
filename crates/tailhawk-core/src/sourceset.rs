//! The remote sources under edit — the decision half of the credentials dialog.
//!
//! `dialog.rs` draws this and routes clicks back into it; the shell's `secrets` module stores what
//! it produces. Nothing here needs a window, which is the point: the rules editor's own history is
//! three defects that lived in a dialog procedure and became testable the moment the decision moved
//! out of it.
//!
//! **This mirrors [`crate::ruleset::Editor`] on purpose.** Rows, a selection, a fault per row, a
//! dirty flag, open and close. §1.2's memorability rule is usually read as being about the people
//! using the product, but it applies just as much to the people maintaining it: two list-and-fields
//! dialogs that disagree about what "dirty" means, or about whether the selection survives a
//! delete, are two dialogs to learn instead of one.
//!
//! # The secret is handled by not being here
//!
//! A [`Source`] carries no credential — `settings.rs` says why, and this type is the second half of
//! that argument. The editor holds at most a **pending** secret for the row being edited, and hands
//! it to the store on save.
//!
//! - **`None` is not `Some("")`.** `None` means *leave whatever is in the store alone*; `Some("")`
//!   means *this source has no secret*. Editing a source's URL must not silently wipe a credential
//!   the user never touched, and an editor that could not tell those apart would do precisely that
//!   the first time somebody fixed a typo in a hostname.
//! - **Nothing here reads the store.** The editor is *told* whether a secret exists, as a `bool`.
//!   It never sees the value, so a credential cannot escape through a `Debug` line, a test fixture
//!   or a panic message by way of this type.

use crate::settings::Source;

/// How many sources the dialog will hold. Generous for "one per environment", and a bound rather
/// than none at all so a hand-edited settings file cannot make the list unusable.
pub const MAX_SOURCES: usize = 64;

/// One row as the list should draw it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row<'a> {
    pub name: &'a str,
    pub url: &'a str,
    /// What the Authentication column says: the client id, or that the source uses none.
    pub auth: &'a str,
    /// Whether a secret is stored for this source — the shell answers this, the editor only carries
    /// it. Shown so a source that is configured but has never been given its secret is visible as
    /// such rather than failing at the first query.
    pub has_secret: bool,
    /// Why this row cannot be used, if it cannot.
    pub fault: Option<&'static str>,
}

/// The set of sources under edit.
#[derive(Debug, Default)]
pub struct Editor {
    sources: Vec<Source>,
    /// Whether the shell found a stored secret for each source, parallel to `sources`.
    stored: Vec<bool>,
    /// The secret typed into the box for the selected row, if the user has typed one.
    pending: Option<String>,
    selected: usize,
    open: bool,
    dirty: bool,
}

impl Editor {
    /// An editor over `sources`, closed. `stored` says which of them already have a secret; a short
    /// list is padded with `false` rather than rejected, because the answer to "does this have a
    /// secret" for a source the shell did not ask about is "not as far as we know".
    pub fn new(sources: Vec<Source>, stored: Vec<bool>) -> Self {
        let mut stored = stored;
        stored.resize(sources.len(), false);
        Self {
            sources,
            stored,
            ..Self::default()
        }
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Opens on the first row.
    pub fn open(&mut self) {
        self.open = true;
        self.selected = 0;
        self.pending = None;
    }

    /// Closes, forgetting any secret typed but not saved.
    ///
    /// **The pending secret is dropped here and this is the only place it can be.** A cancelled
    /// dialog that left a credential in memory would be a credential in the next crash dump.
    pub fn close(&mut self) {
        self.open = false;
        self.pending = None;
    }

    pub fn sources(&self) -> &[Source] {
        &self.sources
    }

    pub fn selected(&self) -> usize {
        self.selected
    }

    /// The selected source, if there is one.
    pub fn current(&self) -> Option<&Source> {
        self.sources.get(self.selected)
    }

    /// Points the editor at row `at`, forgetting a secret typed against the row being left.
    ///
    /// **Forgetting it is the whole reason this is not a plain assignment.** A secret typed for
    /// `dev` and then carried to `live` by a click on the list would be stored against the wrong
    /// source — a mistake nothing downstream could detect, because both are valid strings.
    pub fn select(&mut self, at: usize) {
        if at < self.sources.len() && at != self.selected {
            self.pending = None;
        }
        self.selected = at.min(self.sources.len().saturating_sub(1));
    }

    /// The rows, for the list.
    pub fn rows(&self) -> Vec<Row<'_>> {
        self.sources
            .iter()
            .enumerate()
            .map(|(i, s)| Row {
                name: &s.name,
                url: &s.url,
                auth: if s.client_id.is_empty() {
                    "none"
                } else {
                    &s.client_id
                },
                has_secret: self.stored.get(i).copied().unwrap_or(false),
                fault: s.fault(),
            })
            .collect()
    }

    /// Adds an empty source and selects it. Reports whether there was room.
    pub fn add(&mut self) -> bool {
        if self.sources.len() >= MAX_SOURCES {
            return false;
        }
        self.sources.push(Source::default());
        self.stored.push(false);
        self.selected = self.sources.len() - 1;
        self.pending = None;
        self.dirty = true;
        true
    }

    /// Removes the selected source. Reports the name it had, so the shell can forget its secret.
    ///
    /// **The name comes back rather than the shell reading it first**, because after this call the
    /// row is gone and the only other way to know which credential to delete would be to remember
    /// it beforehand — which is the sort of thing that gets forgotten when a second caller appears.
    pub fn remove(&mut self) -> Option<String> {
        if self.selected >= self.sources.len() {
            return None;
        }
        let gone = self.sources.remove(self.selected);
        self.stored.remove(self.selected);
        self.selected = self.selected.min(self.sources.len().saturating_sub(1));
        self.pending = None;
        self.dirty = true;
        Some(gone.name)
    }

    /// Edits a field of the selected source.
    pub fn edit(&mut self, field: Field, value: &str) {
        let Some(source) = self.sources.get_mut(self.selected) else {
            return;
        };
        let slot = match field {
            Field::Name => &mut source.name,
            Field::Url => &mut source.url,
            Field::TokenUrl => &mut source.token_url,
            Field::ClientId => &mut source.client_id,
            Field::Scope => &mut source.scope,
        };
        if slot != value {
            slot.clear();
            slot.push_str(value);
            self.dirty = true;
        }
    }

    /// Records the secret typed for the selected row. An empty string means *there is none*, which
    /// is not the same as never having typed one.
    pub fn set_secret(&mut self, secret: &str) {
        self.pending = Some(secret.to_owned());
        self.dirty = true;
    }

    /// Whether a secret has been typed for the selected row this time round.
    pub fn has_pending_secret(&self) -> bool {
        self.pending.is_some()
    }

    /// Takes the pending secret and the name to store it against, clearing it.
    ///
    /// `None` when nothing was typed — which the caller must read as *leave the store alone*, not
    /// as *store nothing*.
    pub fn take_secret(&mut self) -> Option<(String, String)> {
        let secret = self.pending.take()?;
        let name = self.current()?.name.clone();
        Some((name, secret))
    }

    /// Why the set cannot be saved, or `None`. The first fault of any row, and duplicate names —
    /// which the rows themselves cannot see, because a name is only a duplicate next to another.
    pub fn fault(&self) -> Option<&'static str> {
        if let Some(fault) = self.sources.iter().find_map(|s| s.fault()) {
            return Some(fault);
        }
        let mut seen: Vec<&str> = Vec::with_capacity(self.sources.len());
        for source in &self.sources {
            if seen.contains(&source.name.as_str()) {
                return Some("Two sources share a name; each needs its own.");
            }
            seen.push(&source.name);
        }
        None
    }

    /// Marks the set saved. The pending secret is **not** cleared here: the shell takes it with
    /// [`take_secret`](Self::take_secret) and this only records that the list itself is clean.
    pub fn saved(&mut self) {
        self.dirty = false;
    }
}

/// The editable fields of a source, so the dialog names one rather than passing an index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Name,
    Url,
    TokenUrl,
    ClientId,
    Scope,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(name: &str) -> Source {
        Source {
            name: name.to_owned(),
            url: format!("https://{name}.example/loki"),
            token_url: "https://identity.example/connect/token".to_owned(),
            client_id: "tailhawk".to_owned(),
            scope: "telemetry:read".to_owned(),
        }
    }

    /// **A secret typed for one source must not be stored against another.** Both are valid
    /// strings, so nothing downstream could ever detect the mistake — the credential would simply
    /// be wrong, and the source it was meant for would go on failing to authenticate.
    #[test]
    fn moving_the_selection_forgets_a_secret_typed_for_the_row_being_left() {
        let mut editor = Editor::new(vec![source("dev"), source("live")], vec![false, false]);
        editor.open();
        editor.set_secret("dev-secret");
        assert!(editor.has_pending_secret());

        editor.select(1);
        assert!(
            !editor.has_pending_secret(),
            "the secret did not follow the click to the other source"
        );
        assert_eq!(editor.take_secret(), None);

        // Selecting the row already selected is not a move, so it does not discard a secret being
        // typed — a dialog that re-selects on every keystroke would otherwise lose it.
        editor.set_secret("live-secret");
        editor.select(1);
        assert_eq!(
            editor.take_secret(),
            Some(("live".to_owned(), "live-secret".to_owned()))
        );
    }

    /// **`None` and `Some("")` are different answers**, and conflating them wipes a credential the
    /// user never touched the first time somebody fixes a typo in a hostname.
    #[test]
    fn not_typing_a_secret_is_not_the_same_as_clearing_one() {
        let mut editor = Editor::new(vec![source("dev")], vec![true]);
        editor.open();

        editor.edit(Field::Url, "https://corrected.example/loki");
        assert_eq!(
            editor.take_secret(),
            None,
            "editing another field leaves the stored secret alone"
        );
        assert!(
            editor.rows()[0].has_secret,
            "and it is still recorded as there"
        );

        editor.set_secret("");
        assert_eq!(
            editor.take_secret(),
            Some(("dev".to_owned(), String::new())),
            "an empty box is a deliberate 'there is no secret'"
        );
    }

    /// Closing forgets a secret that was typed but not saved — the only place it can be dropped,
    /// and the difference between a cancelled dialog and a credential in the next crash dump.
    #[test]
    fn cancelling_forgets_what_was_typed() {
        let mut editor = Editor::new(vec![source("dev")], vec![false]);
        editor.open();
        editor.set_secret("typed-then-cancelled");
        editor.close();
        assert!(!editor.is_open());
        assert!(!editor.has_pending_secret());
        assert_eq!(editor.take_secret(), None);
    }

    /// Removing reports the name, because after the call there is no other way to know which
    /// credential to forget.
    #[test]
    fn removing_reports_the_name_whose_secret_should_go() {
        let mut editor = Editor::new(vec![source("dev"), source("live")], vec![true, true]);
        editor.open();
        editor.select(1);
        assert_eq!(editor.remove().as_deref(), Some("live"));
        assert_eq!(editor.sources().len(), 1);
        assert_eq!(
            editor.selected(),
            0,
            "the selection lands on a row that exists"
        );
        assert!(editor.rows()[0].has_secret, "the survivor keeps its own");

        assert_eq!(editor.remove().as_deref(), Some("dev"));
        assert!(editor.sources().is_empty());
        assert_eq!(
            editor.remove(),
            None,
            "removing from nothing is not a panic"
        );
    }

    /// **A duplicate name is the one fault a row cannot see**, because a name is only a duplicate
    /// beside another one — and two sources sharing a name share a credential, silently.
    #[test]
    fn the_set_refuses_two_sources_with_one_name() {
        let mut editor = Editor::new(vec![source("dev"), source("dev")], vec![false, false]);
        assert!(editor.fault().is_some(), "two rows, one name");
        assert!(
            editor.rows().iter().all(|r| r.fault.is_none()),
            "and neither row is wrong by itself, which is why the set has to check"
        );

        editor.select(1);
        editor.edit(Field::Name, "live");
        assert_eq!(editor.fault(), None);

        // A row's own fault is reported too, and before the duplicate check.
        editor.edit(Field::Url, "http://insecure.example");
        assert!(editor.fault().is_some(), "http is a fault of the row");
    }

    /// The list is bounded, and adding selects what was added so typing goes to the new row.
    #[test]
    fn adding_selects_the_new_row_and_the_list_is_bounded() {
        let mut editor = Editor::new(Vec::new(), Vec::new());
        editor.open();
        assert!(editor.add());
        assert_eq!(editor.selected(), 0);
        editor.edit(Field::Name, "dev");
        assert_eq!(editor.current().map(|s| s.name.as_str()), Some("dev"));
        assert!(editor.is_dirty());

        while editor.sources().len() < MAX_SOURCES {
            assert!(editor.add());
        }
        assert!(!editor.add(), "the list is bounded");
        assert_eq!(editor.sources().len(), MAX_SOURCES);
    }

    /// A short `stored` list is padded rather than refused: "we did not ask" reads as "no secret",
    /// which shows the source as needing one rather than hiding it.
    #[test]
    fn a_short_stored_list_is_padded() {
        let editor = Editor::new(vec![source("a"), source("b")], vec![true]);
        let rows = editor.rows();
        assert!(rows[0].has_secret);
        assert!(!rows[1].has_secret);
    }
}
