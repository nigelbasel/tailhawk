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

/// Widths for the list's six columns — name, URL, query, applications, secret, problem —
/// apportioned to the width the list actually has.
///
/// **The widths were fixed, and did not fit.** 90 + 250 + 140 + 60 + 180 is 720px inside a list
/// 340 dialog units wide, which is somewhere around 510 to 595px depending on the shell font — so
/// `Problem` hung off the right-hand edge. That is the only column with anything in it for a row
/// that has just been added, which is why adding one looked like adding nothing.
///
/// Each column is floored at something a word fits in, and the rest is shared out — so a list
/// narrower than six such columns overflows and scrolls rather than producing a column too narrow
/// to read or, as a first draft of this did, one of negative width. Zero is the width the shell
/// reports when it cannot measure the list at all, so it is the one value that has to be safe.
pub fn column_widths(total: i32) -> [i32; 6] {
    const SHARE: [i32; 6] = [14, 24, 18, 18, 8, 18];
    const FLOOR: i32 = 36;
    let total = total.max(FLOOR * SHARE.len() as i32);
    let spare = i64::from(total - FLOOR * SHARE.len() as i32);
    let mut out = [FLOOR; 6];
    for (slot, share) in out.iter_mut().zip(SHARE) {
        *slot += (spare * i64::from(share) / 100) as i32;
    }
    // The division's remainder goes on the widest column, so the row fills the list exactly rather
    // than leaving a few pixels of dead ground beyond the last divider.
    out[1] += total - out.iter().sum::<i32>();
    out
}

/// One row as the list should draw it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row<'a> {
    pub name: &'a str,
    pub url: &'a str,
    /// The selector this source starts from, or that it takes everything.
    ///
    /// **Shown in the list, and it has to be.** The estate runs one Loki, so the sources a person
    /// actually wants — dev, QA, live — share a URL, a client and a credential and differ *only*
    /// here. A list without this column is three rows that look identical.
    pub query: &'a str,
    /// The applications this entry opens straight into, or that it asks.
    ///
    /// **Shown for the same reason the query is.** Two entries over one Loki may differ only in
    /// which applications they carry — that is what makes several entries over one server worth
    /// having — and a list that does not show them is two rows a reader cannot tell apart. Held as
    /// a `String` rather than a slice because the list needs the composed text and the row is where
    /// the composing belongs.
    pub apps: String,
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

    /// Opens on the first row, making one if the set is empty.
    ///
    /// **A dialog that shows editable fields must have a row behind them.** Without the row the
    /// fields were a form that discarded every keystroke: [`Self::edit`] writes into the selected
    /// source and an empty set has none, so the owner filled the dialog in twice on a fresh install
    /// and twice watched OK close it having saved nothing. Pressing **Add** afterwards only cleared
    /// the boxes, because the shell repaints them from the new blank row.
    ///
    /// Offering the row is not a change to the set, so it deliberately leaves the editor clean —
    /// and [`Self::discard_blank`] takes it back out again if nothing was ever typed into it.
    pub fn open(&mut self) {
        self.open = true;
        self.selected = 0;
        self.pending = None;
        if self.sources.is_empty() {
            self.sources.push(Source::default());
            self.stored.push(false);
        }
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
                // **A row with no name yet still has to look like a row.** Every field of a new
                // source is empty, so drawn as stored it is a blank line — and the owner, having
                // pressed Add, reported that it "added nothing at all".
                name: if s.name.is_empty() {
                    "(unnamed)"
                } else {
                    &s.name
                },
                url: &s.url,
                query: if s.query.is_empty() {
                    "(everything)"
                } else {
                    &s.query
                },
                // **`(ask)`, not blank.** An empty `apps` means the picker opens — `Source::apps`
                // says so — which is a behaviour rather than an absence, and the commonest entry
                // there is. Blank would read as "no applications", which is the one thing it is not.
                apps: if s.apps.is_empty() {
                    "(ask)".to_owned()
                } else {
                    apps_said(&s.apps)
                },
                has_secret: self.stored.get(i).copied().unwrap_or(false),
                // **A row's own fault, including a blank row's — and deliberately not the same
                // question [`Self::fault`] answers.** That one decides whether the set can be
                // saved and so forgives a row nobody typed into; this one describes the row, and
                // "Give the source a name." is the most useful thing the list can say about the
                // blank row it just offered. The two disagreeing about the blank row is the point,
                // not an oversight to be tidied away.
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

    /// Whether row `at` holds nothing at all — no field typed, and no secret typed against it.
    ///
    /// A source indistinguishable from a fresh blank one is the row [`Self::open`] offers before
    /// anybody has used it. The secret is part of the question because a credential typed against
    /// a nameless row has nowhere to be stored: that row must keep saying so rather than be
    /// quietly dropped, which would be this whole defect over again in miniature.
    fn is_blank_row(&self, at: usize) -> bool {
        self.sources
            .get(at)
            .is_some_and(|s| *s == Source::default())
            && !(at == self.selected && self.pending.as_deref().is_some_and(|s| !s.is_empty()))
    }

    /// Drops every row nobody typed anything into, so the offered row is not saved as a source.
    ///
    /// **The selection follows the row it was on.** A blank row removed from *below* the selection
    /// shifts every row above it down one, and leaving the index alone would silently move the
    /// selection to the next source along — which is how [`Self::take_secret`] would come to store
    /// a credential against the wrong source, the mistake [`Self::select`] exists to prevent.
    /// `self.selected` itself is left untouched until the end, because [`Self::is_blank_row`] asks
    /// which row is selected and must get the same answer throughout.
    pub fn discard_blank(&mut self) {
        let mut selected = self.selected;
        for at in (0..self.sources.len()).rev() {
            if self.is_blank_row(at) {
                self.sources.remove(at);
                self.stored.remove(at);
                if at < selected {
                    selected -= 1;
                }
            }
        }
        self.selected = selected.min(self.sources.len().saturating_sub(1));
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

    /// Copies the selected source under a name of its own and selects the copy.
    ///
    /// **Everything but the name and the credential.** The owner asked for this to edit one field
    /// of a configuration that is otherwise identical — the estate runs one Loki, so dev, QA and
    /// live share a URL, a client and a scope and differ in a selector or a list of applications.
    ///
    /// **The copy inherits no secret, and that is correctness rather than caution.** A credential
    /// is filed under the source's *name*, so a copy reported as having one would be pointing at
    /// the original's — and renaming it later would orphan that one, which is a thing that has
    /// already happened in this product's store. A pending secret is left with the row it was typed
    /// for, exactly as [`Self::select`] leaves it.
    ///
    /// The name comes from [`crate::apps::saved_entry_name`], which is where the "characters a
    /// credential key may hold, numeric suffix on a collision" rule already lives.
    /// **A row nobody has typed into is not worth copying**, and copying it would be worse than
    /// nothing: the copy has a generated *name*, so [`Self::is_blank_row`] no longer recognises it,
    /// [`Self::discard_blank`] leaves it behind, and the reader is held at OK by a complaint about
    /// a source they never meant to make.
    pub fn clone_selected(&mut self) -> bool {
        if self.sources.len() >= MAX_SOURCES || self.is_blank_row(self.selected) {
            return false;
        }
        let Some(source) = self.sources.get(self.selected) else {
            return false;
        };
        let taken: Vec<String> = self.sources.iter().map(|s| s.name.clone()).collect();
        let copy = Source {
            name: crate::apps::saved_entry_name(&format!("{} copy", source.name), &[], &taken),
            ..source.clone()
        };
        self.sources.push(copy);
        self.stored.push(false);
        self.selected = self.sources.len() - 1;
        self.pending = None;
        self.dirty = true;
        true
    }

    /// Edits a field of the selected source.
    pub fn edit(&mut self, field: Field, value: &str) {
        let Some(source) = self.sources.get_mut(self.selected) else {
            return;
        };
        // **The applications are a list, so they are not one of the string slots below.** Compared
        // *after* parsing rather than as text, because the dialog reads every field on every
        // keystroke and `gateway` and ` gateway ` are the same choice — a field that reported a
        // change whenever the spacing differed would hold the set dirty for ever.
        if field == Field::Apps {
            let apps = apps_typed(value);
            if source.apps != apps {
                source.apps = apps;
                self.dirty = true;
            }
            return;
        }
        let slot = match field {
            Field::Name => &mut source.name,
            Field::Url => &mut source.url,
            Field::TokenUrl => &mut source.token_url,
            Field::AuthUrl => &mut source.auth_url,
            Field::ClientId => &mut source.client_id,
            Field::Scope => &mut source.scope,
            Field::Query => &mut source.query,
            Field::Apps => unreachable!("answered above, where the list is parsed"),
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
    /// A row nobody has typed into is not judged at all — see [`Self::is_blank_row`]. A reader who
    /// opens the dialog to look, touches nothing and presses OK must not be held there by a
    /// complaint about the blank row they were offered.
    pub fn fault(&self) -> Option<&'static str> {
        let used = || {
            (0..self.sources.len())
                .filter(|at| !self.is_blank_row(*at))
                .map(|at| &self.sources[at])
        };
        if let Some(fault) = used().find_map(|s| s.fault()) {
            return Some(fault);
        }
        let mut seen: Vec<&str> = Vec::with_capacity(self.sources.len());
        for source in used() {
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
///
/// **`AuthUrl` was missing until 2026-10-13, and its absence was a whole feature unreachable.**
/// [`Source::signs_in`](crate::settings::Source::signs_in) is `!auth_url.is_empty()`, so a field
/// that could not be set meant every source anyone created in the app answered "does not sign
/// in" — and the authorization-code path, loopback socket, PKCE and sign-in dialog all built and
/// all working, could never be entered. The owner's own sources had it only because his settings
/// file was written by hand. The first person to be handed the app was told to paste a client
/// secret instead, and had no login screen to find.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Name,
    Url,
    TokenUrl,
    AuthUrl,
    ClientId,
    Scope,
    Query,
    /// The applications the entry opens into. A list rather than a string, so [`Editor::edit`]
    /// answers it before the others.
    Apps,
}

/// The applications as a reader types them: a comma-separated list, trimmed, empties dropped.
///
/// **A comma, because that is what the list shows and what `apps=a,b` already means** in the
/// `loki://source?apps=…` key a remote tab is remembered under. An application name cannot hold a
/// comma — `LOKI.md` §2 has them as label values, and the picker only ever offers what the server
/// returned — so nothing is lost by splitting on one.
pub fn apps_typed(text: &str) -> Vec<String> {
    text.split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(str::to_owned)
        .collect()
}

/// The inverse, for the box and for the list.
pub fn apps_said(apps: &[String]) -> String {
    apps.join(", ")
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
            apps: Vec::new(),
            scope: "telemetry:read".to_owned(),
            query: format!("{{environment=\"{name}\"}}"),
            auth_url: String::new(),
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
    ///
    /// Adding to a set that was empty lands on row **1**, not row 0: since 2026-10-13 [`open`]
    /// offers a row to type into, so Add on a fresh dialog is a *second* row. The extra blank one
    /// costs nothing — [`discard_blank`] takes every untyped row back out on save.
    ///
    /// [`open`]: Editor::open
    /// [`discard_blank`]: Editor::discard_blank
    #[test]
    fn adding_selects_the_new_row_and_the_list_is_bounded() {
        let mut editor = Editor::new(Vec::new(), Vec::new());
        editor.open();
        assert!(editor.add());
        assert_eq!(editor.selected(), 1);
        assert_eq!(editor.sources().len(), 2);
        editor.edit(Field::Name, "dev");
        assert_eq!(editor.current().map(|s| s.name.as_str()), Some("dev"));
        assert!(editor.is_dirty());

        while editor.sources().len() < MAX_SOURCES {
            assert!(editor.add());
        }
        assert!(!editor.add(), "the list is bounded");
        assert_eq!(editor.sources().len(), MAX_SOURCES);
    }

    /// **An editor opened with no sources had nowhere to put what you typed.** The owner filled the
    /// fields in twice on a fresh install and hit OK, and both times the dialog shut having added
    /// nothing: [`Editor::edit`] writes into the *selected* source, an empty set has none, so every
    /// keystroke was discarded — and [`Editor::fault`] then found nothing wrong with an empty set,
    /// so OK was a perfectly valid save of it. Pressing **Add** was no escape, because the shell
    /// repaints the fields from the new blank row and so clears what had been typed.
    ///
    /// A dialog that shows seven editable boxes must have a row behind them. Offering the row is
    /// not a change to the set, though, so it does not make the editor dirty.
    #[test]
    fn an_editor_opened_with_no_sources_has_a_row_to_type_into() {
        let mut editor = Editor::new(Vec::new(), Vec::new());
        editor.open();
        assert_eq!(
            editor.sources().len(),
            1,
            "an empty set opens on a row, or there is nowhere for a keystroke to land"
        );
        assert!(
            editor.current().is_some(),
            "and that row is the selected one"
        );
        assert!(
            !editor.is_dirty(),
            "a row offered is not a change made; Cancel must still discard cleanly"
        );

        editor.edit(Field::Name, "live");
        assert_eq!(
            editor.current().map(|s| s.name.as_str()),
            Some("live"),
            "this is the keystroke that used to go nowhere"
        );
        assert!(editor.is_dirty());
    }

    /// **A row nobody typed into is not a source.** It is the other half of opening on a row: a
    /// reader who opens the dialog to look, touches nothing and presses OK must not be held there
    /// by "Give the source a name." for a row they never asked for, and must not have a nameless
    /// source written to their settings file either.
    ///
    /// A secret typed against it is the exception, and it has to be: dropping that row would
    /// discard a credential silently, which is the very fault this whole change exists to fix. So
    /// a row with a secret and no name keeps faulting until it has one.
    #[test]
    fn a_row_nobody_typed_into_is_not_a_source_and_is_not_saved() {
        let mut editor = Editor::new(vec![Source::default()], vec![false]);
        editor.open();
        assert_eq!(editor.sources().len(), 1, "the row is there to be judged");
        assert_eq!(
            editor.fault(),
            None,
            "an untouched row is not a half-finished one"
        );
        editor.discard_blank();
        assert!(
            editor.sources().is_empty(),
            "and it is not written to the settings file"
        );

        let mut typed = Editor::new(vec![Source::default()], vec![false]);
        typed.open();
        typed.set_secret("hunter2");
        assert_eq!(
            typed.fault(),
            Some("Give the source a name."),
            "a secret has nowhere to be stored without a name, so say so rather than drop it"
        );
        typed.discard_blank();
        assert_eq!(
            typed.sources().len(),
            1,
            "and the row survives, because discarding it would discard the secret with it"
        );

        let mut real = Editor::new(vec![source("live"), Source::default()], vec![true, false]);
        real.open();
        assert_eq!(
            real.fault(),
            None,
            "a blank row beside a real one faults nobody"
        );
        real.discard_blank();
        assert_eq!(real.sources().len(), 1, "and only the real one is saved");
        assert_eq!(real.sources()[0].name, "live");

        // End to end from the state the owner was actually in: nothing configured, the dialog
        // offers a row, he fills it in, and that one source is what gets saved.
        let mut fresh = Editor::new(Vec::new(), Vec::new());
        fresh.open();
        fresh.edit(Field::Name, "live");
        fresh.edit(Field::Url, "https://telemetry.example/loki");
        assert_eq!(fresh.fault(), None);
        fresh.discard_blank();
        assert_eq!(
            fresh.sources().len(),
            1,
            "the row he typed into is a source"
        );
        assert_eq!(fresh.sources()[0].name, "live");
    }

    /// **The selection follows the row it was on, rather than the index it had.** Dropping a blank
    /// row from below the selection shifts everything above it down one, so leaving the index alone
    /// moves the selection silently to the *next* source — and [`Editor::take_secret`] reads the
    /// selection for the name to store a credential against. That is how a secret typed for one
    /// source comes to be written against another, which is the mistake [`Editor::select`] was
    /// given its own doc comment to prevent.
    #[test]
    fn discarding_blank_rows_leaves_the_selection_on_the_source_it_was_on() {
        let mut editor = Editor::new(
            vec![Source::default(), source("dev"), source("live")],
            vec![false, true, true],
        );
        editor.select(2);
        assert_eq!(editor.current().map(|s| s.name.as_str()), Some("live"));

        editor.discard_blank();
        assert_eq!(
            editor.current().map(|s| s.name.as_str()),
            Some("live"),
            "the blank row below it went, so the index has to come down with it"
        );

        // The shape that costs a credential: the blank row carrying the typed secret is the one
        // kept, and it must still be the selected one afterwards or `take_secret` names the wrong
        // source.
        let mut carrying = Editor::new(
            vec![Source::default(), Source::default(), source("live")],
            vec![false, false, true],
        );
        carrying.select(1);
        carrying.set_secret("hunter2");
        carrying.discard_blank();
        assert_eq!(
            carrying.take_secret(),
            Some((String::new(), "hunter2".to_owned())),
            "the secret belongs to the nameless row it was typed into, not to live"
        );
    }

    /// **A new row has to look like a row.** Every field of one is empty, so with the name shown
    /// as it is stored the list draws a blank line — and the owner, having pressed Add, reported
    /// that it "added nothing at all". The query column already substitutes a word for empty for
    /// exactly this reason; the name is the column that says a row is there.
    #[test]
    fn a_row_with_no_name_yet_still_reads_as_a_row() {
        let mut editor = Editor::new(vec![Source::default()], vec![false]);
        editor.open();
        let rows = editor.rows();
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0].name, "(unnamed)",
            "a blank line in the list reads as nothing having been added"
        );
        assert_eq!(rows[0].query, "(everything)");

        editor.edit(Field::Name, "live");
        assert_eq!(
            editor.rows()[0].name,
            "live",
            "and a name, once given, is shown instead of standing in for one"
        );
    }

    /// **The columns are apportioned to the width there is, not to a width that was hoped for.**
    /// They were fixed at 90 + 250 + 140 + 60 + 180 = 720px inside a list 340 dialog units wide —
    /// around 510 to 595px depending on the shell font — so `Problem` fell off the right-hand edge.
    /// That is the one column with any text in a new row, which is why an added row looked like no
    /// row at all.
    #[test]
    fn the_list_columns_fit_the_width_the_list_actually_has() {
        for total in [320, 420, 510, 595, 720, 1200] {
            let widths = column_widths(total);
            let sum: i32 = widths.iter().sum();
            assert_eq!(
                sum, total,
                "{widths:?} sums to {sum} and so does not fill {total}px exactly"
            );
            assert!(
                widths.iter().all(|w| *w >= 36),
                "{widths:?} has a column too narrow to read a word in"
            );
            assert!(
                widths[1] > widths[0],
                "{widths:?}: a URL is longer than a name"
            );
        }

        // **Zero is what the shell reports when it cannot measure the list**, and a width shared
        // out from nothing used to come back negative — the remainder was pushed onto the URL
        // column after the floor had already overspent the total, so `LVM_INSERTCOLUMNW` was handed
        // a `cx` of -4. A list too narrow for six readable columns overflows and scrolls instead.
        for total in [i32::MIN, -20, 0, 1, 4, 5, 179] {
            let widths = column_widths(total);
            assert!(
                widths.iter().all(|w| *w >= 36),
                "{widths:?} from a total of {total} is not a width a column can have"
            );
        }
        assert_eq!(column_widths(0), [36; 6]);
    }

    /// **A source created in the app can sign the reader in**, which it could not do before
    /// 2026-10-13: there was no [`Field`] for `auth_url`, so the editor had no way to set the one
    /// value [`Source::signs_in`] reads, and the colleague the owner gave the app to got no login
    /// screen — just an instruction to paste a client secret he had no reason to possess.
    #[test]
    fn a_source_typed_in_the_dialog_can_be_made_to_sign_in() {
        let mut editor = Editor::new(vec![source("live")], vec![true]);
        assert!(
            !editor.sources()[0].signs_in(),
            "a source with no sign-in URL presents a client secret instead"
        );

        editor.edit(Field::AuthUrl, "https://identity.example/connect/authorize");
        assert!(
            editor.sources()[0].signs_in(),
            "and setting the field is what moves it onto the authorization-code path"
        );
        assert_eq!(
            editor.sources()[0].auth_url,
            "https://identity.example/connect/authorize",
            "stored where pull::begin_sign_in reads it, not merely remembered by the dialog"
        );
        assert_eq!(
            editor.fault(),
            None,
            "a complete sign-in source is not a half-configured one"
        );
    }

    /// **Three sources against one Loki differ only by their selector**, which is the shape the
    /// owner chose on 2026-09-03 once it was established that `environment` is a label and not a
    /// host. The list has to show it, or those three rows are indistinguishable — and an empty
    /// selector has to say something rather than nothing, or it reads as a missing value.
    #[test]
    fn the_selector_is_what_tells_three_sources_of_one_loki_apart() {
        let one = |name: &str, query: &str| Source {
            name: name.to_owned(),
            url: "https://telemetry.example/loki".to_owned(),
            token_url: "https://identity.example/connect/token".to_owned(),
            client_id: "tailhawk".to_owned(),
            apps: Vec::new(),
            scope: "telemetry:read".to_owned(),
            query: query.to_owned(),
            auth_url: String::new(),
        };
        let editor = Editor::new(
            vec![
                one("dev", "{environment=\"dev\"}"),
                one("live", "{environment=~\"live|production\"}"),
                one("all", ""),
            ],
            vec![true, true, true],
        );
        let rows = editor.rows();
        assert!(
            rows.iter()
                .map(|r| r.url)
                .collect::<Vec<_>>()
                .windows(2)
                .all(|p| p[0] == p[1]),
            "they share a host, which is the whole point"
        );
        let queries: Vec<&str> = rows.iter().map(|r| r.query).collect();
        assert_eq!(
            queries,
            [
                "{environment=\"dev\"}",
                "{environment=~\"live|production\"}",
                "(everything)"
            ],
            "and are told apart only by this"
        );
        assert_eq!(
            queries
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            3,
            "no two rows read the same"
        );

        // An empty selector is legitimate — never a fault.
        assert!(editor.fault().is_none());
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

    /// **The applications are editable and visible, which they had never been.** The owner,
    /// 2026-10-20: *"there is no place where we can edit or view the list of apps being tailed"* —
    /// `Source::apps` was written only by the picker, so an entry that opened straight into seven
    /// applications could not be told from one that asks, and the list showed neither.
    #[test]
    fn the_applications_are_a_field_and_a_column() {
        let mut editor = Editor::new(vec![source("live")], vec![false]);
        editor.open();
        assert_eq!(
            apps_said(&[]),
            "",
            "no applications is an empty box, not a word"
        );
        assert_eq!(editor.rows()[0].apps, "(ask)", "and the list says it asks");

        editor.edit(Field::Apps, "gateway, job-dispatcher ,, identity ");
        assert_eq!(
            editor.current().expect("a row").apps,
            ["gateway", "job-dispatcher", "identity"],
            "split on commas, trimmed, and an empty one is not an application"
        );
        assert_eq!(
            apps_said(&editor.current().expect("a row").apps),
            "gateway, job-dispatcher, identity",
            "and it reads back as what a reader would type"
        );
        assert_eq!(editor.rows()[0].apps, "gateway, job-dispatcher, identity");
        assert!(editor.is_dirty());

        // Typing the same thing again is not an edit — the dialog reads every field on every
        // keystroke, so a field that always reports a change would mark the set dirty for ever.
        let mut again = Editor::new(vec![source("live")], vec![false]);
        again.open();
        again.edit(Field::Apps, "gateway");
        again.saved();
        again.edit(Field::Apps, " gateway ");
        assert!(!again.is_dirty(), "the same list, differently spaced");
    }

    /// **A clone carries everything but the name and the sign-in.** The owner, 2026-10-20: *"it
    /// would also be nice to be able to clone a setup and then edit one field"*.
    ///
    /// **It must inherit no credential**, and that is not caution — a secret is filed under the
    /// source's *name*, so a clone that reported one as stored would be pointing at the original's
    /// credential, and renaming it afterwards would orphan that one. The `Secret` column saying
    /// `—` on a fresh clone is the truth, and it is also the prompt to sign in.
    #[test]
    fn a_clone_is_the_same_source_under_a_new_name_with_no_secret() {
        let mut editor = Editor::new(vec![source("live"), source("dev")], vec![true, true]);
        editor.open();
        editor.edit(Field::Apps, "gateway, identity");
        editor.saved();

        assert!(editor.clone_selected(), "there was room");
        assert_eq!(editor.sources().len(), 3);
        assert_eq!(editor.selected(), 2, "and the copy is the row being edited");
        let copy = editor.current().expect("the copy");
        assert_eq!(copy.name, "live copy", "a name of its own");
        assert_eq!(copy.url, "https://live.example/loki");
        assert_eq!(copy.query, "{environment=\"live\"}");
        assert_eq!(copy.apps, ["gateway", "identity"], "including the apps");
        assert!(!editor.rows()[2].has_secret, "and no credential");
        assert!(editor.rows()[0].has_secret, "the original keeps its own");
        assert!(editor.is_dirty());
        assert!(
            editor.fault().is_none(),
            "a clone is savable at once, which a duplicate name would not be: {:?}",
            editor.fault()
        );

        // A second copy does not collide with the first — two sources sharing a name share a
        // credential silently, which `fault` refuses and this must not create.
        editor.select(0);
        assert!(editor.clone_selected());
        assert_eq!(editor.current().expect("the copy").name, "live copy 2");
        assert!(editor.fault().is_none(), "{:?}", editor.fault());

        // A secret typed for the row being left does not follow the clone, for the same reason
        // moving the selection discards it.
        editor.select(0);
        editor.set_secret("live-secret");
        assert!(editor.clone_selected());
        assert_eq!(
            editor.take_secret(),
            None,
            "the secret stayed with the row it was typed for"
        );
    }

    /// The bound holds for cloning as it does for adding, and a refusal changes nothing.
    #[test]
    fn a_clone_is_refused_when_the_set_is_full() {
        let sources: Vec<Source> = (0..MAX_SOURCES).map(|n| source(&format!("s{n}"))).collect();
        let mut editor = Editor::new(sources, Vec::new());
        editor.open();
        assert!(!editor.clone_selected());
        assert_eq!(editor.sources().len(), MAX_SOURCES);
    }

    /// **The row the dialog offers to type into is not copied.** A copy of it would carry a
    /// generated name, which is enough to stop `discard_blank` recognising it — so pressing Clone
    /// on a fresh install would leave a source called `copy` with no URL, and hold the reader at OK
    /// with a complaint about a row they never meant to create.
    #[test]
    fn a_blank_row_is_not_worth_cloning() {
        let mut editor = Editor::default();
        editor.open();
        assert_eq!(editor.sources().len(), 1, "the offered row");
        assert!(!editor.clone_selected());
        assert_eq!(editor.sources().len(), 1, "and nothing was added");

        // One field is enough to make it a row somebody is working on, and then it copies.
        editor.edit(Field::Name, "live");
        assert!(editor.clone_selected());
        assert_eq!(editor.current().expect("the copy").name, "live copy");
    }

    /// Six columns now, and the widths still fit the list rather than hanging off its edge —
    /// the defect `column_widths` exists for, with one more column to share out.
    #[test]
    fn six_columns_fit_the_width_they_are_given() {
        for total in [0, 100, 300, 510, 595, 1200] {
            let widths = column_widths(total);
            assert!(
                widths.iter().all(|w| *w >= 36),
                "no column is too narrow to read at {total}: {widths:?}"
            );
            if total >= 36 * 6 {
                assert_eq!(
                    widths.iter().sum::<i32>(),
                    total,
                    "and the row fills the list exactly at {total}"
                );
            }
        }
    }
}
