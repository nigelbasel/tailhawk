//! The sources and their tails under edit — the decision half of the Remote sources dialog.
//!
//! `dialog.rs` draws this and routes clicks back into it; the shell's `secrets` module stores what
//! it produces. Nothing here needs a window, which is the point: the rules editor's own history is
//! three defects that lived in a dialog procedure and became testable the moment the decision moved
//! out of it.
//!
//! **This mirrors [`crate::ruleset::Editor`] on purpose.** Rows, a selection, a fault per row, a
//! dirty flag, open and close. §1.2's memorability rule is usually read as being about the people
//! using the product, but it applies just as much to the people maintaining it: two list dialogs
//! that disagree about what "dirty" means, or about whether the selection survives a delete, are
//! two dialogs to learn instead of one.
//!
//! # A source is a connection; a tail is a view
//!
//! Since 2026-10-21, and it is the owner's framing: *"we might have a single source, the nurtur
//! loki server, but three separate tails, say one looking at ndc, one at identity server and one at
//! job dispatcher"*. The tree draws it — a source is a node, its tails are its children — and the
//! credential follows the **source**, which is what one entry per view had been getting wrong: one
//! Loki with three entries meant three credentials for the same server, the same client and the
//! same person.
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

use crate::settings::{Renamed, Source, Tail};
use crate::sourcesetup::mode_of;

/// How many sources the dialog will **add**. Generous for "one per environment".
///
/// **It bounds adding, not loading**, which its own comment used to get wrong: it said a bound
/// existed "so a hand-edited settings file cannot make the list unusable", and nothing caps
/// `[[source]]` at load. What actually protects the menu is
/// [`crate::menubar`]-side truncation, which says how many it left out; the tree itself draws
/// whatever the file holds, which is the honest behaviour for a configuration a person edited.
pub const MAX_SOURCES: usize = 64;

/// One row of the tree, flattened for drawing.
///
/// **A source is a node and its tails are its children**, which is the owner's shape, 2026-10-21:
/// *"instead of a simple list, we would have a list of sources, each of which might be a node on a
/// tree with the tails underneath them."* Flattened here rather than handed over as a nest, because
/// a `SysTreeView32` is filled one item at a time and the order is the drawing order.
///
/// **No columns.** The list this replaces had six — name, URL, query, applications, secret and
/// problem — which was the old model drawn out: a row carried both halves because a row *was* both
/// halves. A source is named by its host and a tail by what it shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node<'a> {
    pub name: &'a str,
    /// **Which row this is, ready to go into a tree item.** Carried rather than left for the shell
    /// to work out, because a source and an orphaned tail both have `under: None` and the only way
    /// to tell them apart from the outside is to count the sources already drawn — which is the
    /// view re-deriving what the model knows, and the shape of three defects this project has
    /// already fixed.
    pub at: Chosen,
    /// `None` for a source, and for a tail whose source is not in the set; `Some(at)` for a tail of
    /// the source at that index.
    pub under: Option<usize>,
    /// Why this node cannot be used, if it cannot.
    pub fault: Option<&'static str>,
    /// Whether a credential is stored for this source. Meaningless on a tail, which has none —
    /// **one credential per source, however many tails read it**, which is the whole point of the
    /// split.
    pub has_secret: bool,
    /// Whether this source *needs* a client secret — true only in
    /// [`Mode::ClientSecret`](crate::sourcesetup::Mode::ClientSecret). A source that signs the
    /// reader in through a browser has none and never will, and a source that wants no token at all
    /// has nothing to authenticate with; saying either was missing a secret would be a warning on a
    /// correctly-configured server. False on a tail, which holds no credential of any kind.
    pub wants_secret: bool,
}

/// What the tree has selected.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum Chosen {
    /// Nothing, which is the state an empty set opens in.
    #[default]
    Nothing,
    Source(usize),
    Tail(usize),
}

/// The sources and their tails, under edit.
///
/// **This mirrors [`crate::ruleset::Editor`] on purpose**, as it always has: rows, a selection, a
/// fault, a dirty flag, open and close. What changed on 2026-10-21 is that there are two kinds of
/// row and one of them belongs to the other.
#[derive(Debug, Default)]
pub struct Editor {
    sources: Vec<Source>,
    tails: Vec<Tail>,
    /// What the store holds for each source and what the reader has typed for it, parallel to
    /// `sources`. See [`Credential`].
    credentials: Vec<Credential>,
    /// The original names of the sources deleted this sitting, so their credentials can go too.
    gone: Vec<String>,
    chosen: Chosen,
    open: bool,
    dirty: bool,
}

/// One source's credential, as this sitting knows it: where it was filed when the dialog opened,
/// whether anything was there, and what has been typed since.
///
/// **Keyed to the row, not to the name — which is the whole fix.** Until 2026-10-26 the dialog
/// kept three flat lists of *names* — to store, to forget, to move — and the shell reconciled them
/// against the names that survived the sitting. A name is not a source: a source given a secret
/// and renamed before Close had its entry dropped because the name it was typed under did not
/// survive; a rename `a→b→c` moved nothing because `b` did not survive either; and a source whose
/// mode moved away from needing a secret kept it, because the row had been added that sitting and
/// so was not "stored". Each list was right about the event it recorded and wrong about the
/// source it belonged to. Carried on the row, the original name and the typed value cannot come
/// apart from it, and [`Editor::secret_plan`] reads the end state rather than replaying events.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Credential {
    /// The name the source was filed under when the dialog opened. `None` for a source added in
    /// this sitting: nothing is in the store under any name for it yet.
    origin: Option<String>,
    /// Whether the store held a secret under `origin` when the dialog opened — the shell's answer,
    /// carried in rather than looked up, because this module never touches the store.
    stored: bool,
    /// What the reader typed in this sitting, if anything. `Some("")` is a reader saying the
    /// source has none; `None` leaves the store alone.
    typed: Option<String>,
}

impl Credential {
    fn existing(name: &str, stored: bool) -> Self {
        Self {
            origin: Some(name.to_owned()),
            stored,
            typed: None,
        }
    }

    /// Whether a secret would be behind this source once the sitting is written.
    fn has_secret(&self) -> bool {
        match &self.typed {
            Some(typed) => !typed.is_empty(),
            None => self.stored,
        }
    }
}

/// What the shell does to the store when the dialog closes, in the order the fields are listed.
///
/// **Forgets first, then moves, then writes.** A delete of `a` followed by a rename of `b` to `a`
/// has to clear `a` before `b`'s credential lands there; a secret typed for a renamed source has
/// to be written *after* the old name's entry has moved, so that the move carries the sign-in and
/// the write then replaces the secret beside it. The moves are given as one `Renamed` per source,
/// from its original name straight to its current one, and `settings::credential_plan` reads every
/// value before writing any — so `a→b` and `b→a` in one sitting is two reads then two writes, and
/// nothing is cleared that something landed on.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct SecretPlan {
    /// Names whose secret **and** sign-in go: the sources deleted this sitting, under the names
    /// they were filed under.
    pub forget: Vec<String>,
    /// Sources renamed, original name to current name, for `secrets::follow_rename`.
    pub moves: Vec<Renamed>,
    /// `(name, secret)` to write.
    pub store: Vec<(String, String)>,
    /// Names whose secret alone goes: the reader emptied the box, or the source's mode has no use
    /// for the secret the store holds. **Only the secret**, because a browser sign-in files its
    /// refresh token under the same name, and a stale client secret beside it is no reason to sign
    /// the reader out — the first version of this put such a source on `forget`, which would have
    /// done exactly that to any signed-in source carrying an orphan from before 2026-10-21.
    pub clear: Vec<String>,
}

impl SecretPlan {
    pub fn is_empty(&self) -> bool {
        self.forget.is_empty()
            && self.moves.is_empty()
            && self.store.is_empty()
            && self.clear.is_empty()
    }
}

impl Editor {
    /// An editor over `sources` and their `tails`, closed. `stored` says which sources already have
    /// a secret; a short list is padded with `false` rather than rejected, because the answer to
    /// "does this have a secret" for a source the shell did not ask about is "not as far as we
    /// know".
    pub fn new(sources: Vec<Source>, tails: Vec<Tail>, stored: Vec<bool>) -> Self {
        let credentials = sources
            .iter()
            .enumerate()
            .map(|(at, source)| {
                Credential::existing(&source.name, stored.get(at).copied().unwrap_or(false))
            })
            .collect();
        Self {
            sources,
            tails,
            credentials,
            ..Self::default()
        }
    }

    /// Records the secret the reader typed for the chosen source — `""` included, which says the
    /// source has none. A later edit that types nothing leaves an earlier one in place, which is
    /// why this is called only when something was typed.
    pub fn secret_typed(&mut self, secret: String) {
        if let Chosen::Source(at) = self.chosen {
            if let Some(credential) = self.credentials.get_mut(at) {
                credential.typed = Some(secret);
                self.dirty = true;
            }
        }
    }

    /// What the store should do when the sitting is written. Pure: reads the rows as they stand.
    ///
    /// Per surviving source: a renamed source's entry moves to its current name — whether or not
    /// a secret was there, because a browser sign-in's refresh token is filed under the same name
    /// and has to follow too. Then a secret the mode has no use for is cleared under the current
    /// name, the sign-in beside it untouched; for a mode that wants one, a typed value is written
    /// under the current name and an emptied box clears what was there. Every deleted source's
    /// original name is forgotten, secret and sign-in both.
    pub fn secret_plan(&self) -> SecretPlan {
        let mut plan = SecretPlan::default();
        for name in &self.gone {
            plan.forget.push(name.clone());
        }
        for (source, credential) in self.sources.iter().zip(&self.credentials) {
            let now = source.name.as_str();
            let wants = mode_of(source).needs_secret();
            if let Some(from) = credential.origin.as_deref().filter(|origin| *origin != now) {
                plan.moves.push(Renamed {
                    now: now.to_owned(),
                    was: vec![from.to_owned()],
                });
            }
            if !wants {
                if credential.stored {
                    plan.clear.push(now.to_owned());
                }
                continue;
            }
            match credential.typed.as_deref() {
                Some("") if credential.stored => plan.clear.push(now.to_owned()),
                Some("") | None => {}
                Some(typed) => plan.store.push((now.to_owned(), typed.to_owned())),
            }
        }
        plan
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Opens on the first source, if there is one.
    ///
    /// **No blank row is offered any more.** The dialog that needed one showed editable fields
    /// beside the list, and a form with no row behind it discarded every keystroke. `New source…`
    /// is that row now, and it arrives with a screen of its own.
    pub fn open(&mut self) {
        self.open = true;
        self.chosen = match self.sources.is_empty() {
            true => Chosen::Nothing,
            false => Chosen::Source(0),
        };
    }

    pub fn close(&mut self) {
        self.open = false;
    }

    pub fn sources(&self) -> &[Source] {
        &self.sources
    }

    pub fn tails(&self) -> &[Tail] {
        &self.tails
    }

    pub fn chosen(&self) -> Chosen {
        self.chosen
    }

    /// Selects a node. Out of range is a deselection rather than a panic, because the tree reports
    /// what it has and this is told rather than asked.
    pub fn choose(&mut self, chosen: Chosen) {
        self.chosen = match chosen {
            Chosen::Source(at) if at < self.sources.len() => chosen,
            Chosen::Tail(at) if at < self.tails.len() => chosen,
            _ => Chosen::Nothing,
        };
    }

    /// The source the selection is on or under — a source node, or the source a chosen tail reads.
    ///
    /// **`Add tail…` works with either chosen**, because "add a tail to this" means the same thing
    /// whether the reader is pointing at the source or at one of its tails.
    pub fn chosen_source(&self) -> Option<usize> {
        match self.chosen {
            Chosen::Source(at) => Some(at),
            Chosen::Tail(at) => {
                let name = &self.tails.get(at)?.source;
                self.sources.iter().position(|s| &s.name == name)
            }
            Chosen::Nothing => None,
        }
    }

    pub fn source(&self, at: usize) -> Option<&Source> {
        self.sources.get(at)
    }

    pub fn tail(&self, at: usize) -> Option<&Tail> {
        self.tails.get(at)
    }

    /// Whether a secret will be behind source `at` once the sitting is written: one the store
    /// holds or the reader typed, for a mode that uses one. A source whose mode has stopped
    /// wanting its secret answers no, because [`Self::secret_plan`] forgets it at Close.
    pub fn source_has_secret(&self, at: usize) -> bool {
        let wants = self
            .sources
            .get(at)
            .is_some_and(|source| mode_of(source).needs_secret());
        wants && self.credentials.get(at).is_some_and(Credential::has_secret)
    }

    /// The tree, flattened: each source followed by the tails that read it.
    ///
    /// **A tail whose source is not in the set is still shown**, under no parent and carrying its
    /// own fault, because a hand-edited file can produce one and a row that vanishes is worse than
    /// a row that explains itself.
    pub fn nodes(&self) -> Vec<Node<'_>> {
        let mut out = Vec::with_capacity(self.sources.len() + self.tails.len());
        for (at, source) in self.sources.iter().enumerate() {
            out.push(Node {
                name: if source.name.is_empty() {
                    "(unnamed)"
                } else {
                    &source.name
                },
                at: Chosen::Source(at),
                under: None,
                fault: source.fault(),
                has_secret: self.source_has_secret(at),
                wants_secret: crate::sourcesetup::mode_of(source)
                    == crate::sourcesetup::Mode::ClientSecret,
            });
            for (n, tail) in self
                .tails
                .iter()
                .enumerate()
                .filter(|(_, t)| t.source == source.name)
            {
                out.push(Node {
                    name: if tail.name.is_empty() {
                        "(unnamed)"
                    } else {
                        &tail.name
                    },
                    at: Chosen::Tail(n),
                    under: Some(at),
                    fault: tail_row_fault(tail, &self.sources),
                    has_secret: false,
                    wants_secret: false,
                });
            }
        }
        for (n, tail) in self
            .tails
            .iter()
            .enumerate()
            .filter(|(_, t)| !self.sources.iter().any(|s| s.name == t.source))
        {
            out.push(Node {
                name: if tail.name.is_empty() {
                    "(unnamed)"
                } else {
                    &tail.name
                },
                at: Chosen::Tail(n),
                under: None,
                fault: tail_row_fault(tail, &self.sources),
                has_secret: false,
                wants_secret: false,
            });
        }
        out
    }

    /// Adds a source and selects it. Reports whether there was room.
    pub fn add_source(&mut self, source: Source) -> bool {
        if self.sources.len() >= MAX_SOURCES {
            return false;
        }
        self.sources.push(source);
        self.credentials.push(Credential::default());
        self.chosen = Chosen::Source(self.sources.len() - 1);
        self.dirty = true;
        true
    }

    /// Adds a tail and selects it.
    pub fn add_tail(&mut self, tail: Tail) -> bool {
        if self.tails.len() >= MAX_TAILS {
            return false;
        }
        self.tails.push(tail);
        self.chosen = Chosen::Tail(self.tails.len() - 1);
        self.dirty = true;
        true
    }

    /// Puts `source` on the chosen source's row, renaming its tails with it.
    ///
    /// **The tails follow the name.** A tail says which source it reads by *name*, so renaming a
    /// source without carrying its children would orphan every one of them — the same mistake, one
    /// level up, as filing a credential under a view's name. **The credential follows too**: its
    /// row's [`Credential`] keeps the original name, and [`Self::secret_plan`] moves the store's
    /// entry when the sitting is written.
    pub fn replace_source(&mut self, source: Source) {
        let Chosen::Source(at) = self.chosen else {
            return;
        };
        let Some(slot) = self.sources.get_mut(at) else {
            return;
        };
        if slot.name != source.name {
            let was = slot.name.clone();
            for tail in self.tails.iter_mut().filter(|t| t.source == was) {
                tail.source = source.name.clone();
            }
        }
        if *slot != source {
            *slot = source;
            self.dirty = true;
        }
    }

    /// Puts `tail` on the chosen tail's row.
    pub fn replace_tail(&mut self, tail: Tail) {
        let Chosen::Tail(at) = self.chosen else {
            return;
        };
        let Some(slot) = self.tails.get_mut(at) else {
            return;
        };
        if *slot != tail {
            *slot = tail;
            self.dirty = true;
        }
    }

    /// Removes what is chosen. **A source takes its tails with it**, and its credential — filed
    /// under the name it had when the dialog opened — goes when the sitting is written. A tail owns
    /// no credential: one source, one credential, however many tails, so removing a tail never
    /// reaches the store.
    pub fn remove(&mut self) {
        match self.chosen {
            Chosen::Source(at) if at < self.sources.len() => {
                let gone = self.sources.remove(at);
                let credential = self.credentials.remove(at);
                if let Some(origin) = credential.origin {
                    self.gone.push(origin);
                }
                self.tails.retain(|t| t.source != gone.name);
                self.dirty = true;
                self.chosen = match self.sources.is_empty() {
                    true => Chosen::Nothing,
                    false => Chosen::Source(at.min(self.sources.len() - 1)),
                };
            }
            Chosen::Tail(at) if at < self.tails.len() => {
                self.tails.remove(at);
                self.dirty = true;
                self.chosen = Chosen::Nothing;
            }
            _ => {}
        }
    }

    /// How many tails read source `at` — what a confirmation has to say before a delete.
    pub fn tails_of(&self, at: usize) -> usize {
        let Some(source) = self.sources.get(at) else {
            return 0;
        };
        self.tails
            .iter()
            .filter(|t| t.source == source.name)
            .count()
    }

    /// Why the set cannot be saved, or `None`.
    ///
    /// Each source's own fault first, then the two a single row cannot see: two sources sharing a
    /// name, and two tails sharing one.
    pub fn fault(&self) -> Option<&'static str> {
        if let Some(fault) = self.sources.iter().find_map(|s| s.fault()) {
            return Some(fault);
        }
        if let Some(fault) = self.tails.iter().find_map(|t| tail_fault(t, &self.sources)) {
            return Some(fault);
        }
        // **Compared trimmed, as [`tail_being_edited`] compares them.** They did not match: the
        // gate compared exactly, so `ndc` and ` ndc ` could sit in a saveable set although the
        // editor refused to create the second — two rows the store and `already_asked` would then
        // treat as one. Two gates that disagree about what "the same name" means are two rules.
        let mut seen: Vec<&str> = Vec::with_capacity(self.sources.len());
        for source in &self.sources {
            let name = source.name.trim();
            if seen.contains(&name) {
                return Some("Two sources share a name; each needs its own.");
            }
            seen.push(name);
        }
        let mut named: Vec<&str> = Vec::with_capacity(self.tails.len());
        for tail in &self.tails {
            let name = tail.name.trim();
            if named.contains(&name) {
                return Some("Two tails share a name; each needs its own.");
            }
            named.push(name);
        }
        None
    }

    /// Copies what is chosen under a name of its own, and selects the copy.
    ///
    /// **A source brings its tails.** The owner, 2026-10-20: *"it would also be nice to be able to
    /// clone a setup and then edit one field"* — and the field he means is a URL, because the
    /// estate runs one Loki per environment with the same applications on each. A clone that copied
    /// the server alone would hand him a connection showing nothing until he had typed all three
    /// views again, which is not the labour he asked to be saved.
    ///
    /// **A tail brings nothing**: it is a second view of the *same* server, which is precisely what
    /// the old one-entry-per-view shape could not express without a second credential.
    ///
    /// **The copy inherits no secret, and that is correctness rather than caution.** The store is
    /// keyed on the source's *name*, so a copy reported as having one would be pointing at the
    /// original's — and renaming it later would orphan that one, which has already happened in this
    /// product's store.
    ///
    /// **A source whose tails will not fit is refused whole.** Half a clone is a server showing
    /// nothing, marked dirty, which Close would then write.
    ///
    /// Names come from [`crate::apps::saved_entry_name`], where the "characters a credential key
    /// may hold, numeric suffix on a collision" rule already lives.
    pub fn clone_chosen(&mut self) -> bool {
        match self.chosen {
            Chosen::Source(at) => self.clone_source(at),
            Chosen::Tail(at) => self.clone_tail(at),
            Chosen::Nothing => false,
        }
    }

    fn clone_source(&mut self, at: usize) -> bool {
        let Some(source) = self.sources.get(at).cloned() else {
            return false;
        };
        let moving: Vec<Tail> = self
            .tails
            .iter()
            .filter(|t| t.source == source.name)
            .cloned()
            .collect();
        if self.sources.len() >= MAX_SOURCES || self.tails.len() + moving.len() > MAX_TAILS {
            return false;
        }
        let taken: Vec<String> = self.sources.iter().map(|s| s.name.clone()).collect();
        let name = crate::apps::saved_entry_name(&format!("{} copy", source.name), &[], &taken);
        // **Every copied tail is named before any is pushed**, against a list that grows as they
        // are named: two tails called `ndc` and `ndc copy` would otherwise both want `ndc copy`.
        let mut names: Vec<String> = self.tails.iter().map(|t| t.name.clone()).collect();
        let copies: Vec<Tail> = moving
            .into_iter()
            .map(|tail| {
                let named =
                    crate::apps::saved_entry_name(&format!("{} copy", tail.name), &[], &names);
                names.push(named.clone());
                Tail {
                    name: named,
                    source: name.clone(),
                    ..tail
                }
            })
            .collect();
        self.sources.push(Source { name, ..source });
        // **No credential.** The store has nothing under a name that did not exist a moment ago,
        // and a copy must not carry the original's row — renaming the copy would move the
        // original's secret away from it.
        self.credentials.push(Credential::default());
        self.tails.extend(copies);
        self.chosen = Chosen::Source(self.sources.len() - 1);
        self.dirty = true;
        true
    }

    fn clone_tail(&mut self, at: usize) -> bool {
        if self.tails.len() >= MAX_TAILS {
            return false;
        }
        let Some(tail) = self.tails.get(at).cloned() else {
            return false;
        };
        let taken: Vec<String> = self.tails.iter().map(|t| t.name.clone()).collect();
        let name = crate::apps::saved_entry_name(&format!("{} copy", tail.name), &[], &taken);
        self.tails.push(Tail { name, ..tail });
        self.chosen = Chosen::Tail(self.tails.len() - 1);
        self.dirty = true;
        true
    }

    /// Marks the set saved.
    pub fn saved(&mut self) {
        self.dirty = false;
    }
}

/// What a tail's **row** should say is wrong with it, or `None`.
///
/// **Everything [`tail_fault`] refuses, and the one thing it tolerates.** The gate has to let a
/// tail with no selector through, or a file migrated from the old shape could not be closed — but
/// a tail with no selector cannot be opened either: the query reaches Loki as an empty `query`
/// field and comes back 400. Before this the row said nothing and the reader found out on the
/// first open, which is the "fewer lines, no error" shape of failure this project keeps naming as
/// the worst kind.
///
/// This is the third of three faults and each has one job: the **row** warns, the **gate**
/// ([`Editor::fault`]) decides whether the set can be written, and the **editor**
/// ([`tail_being_edited`]) decides whether a new one can be made.
fn tail_row_fault(tail: &Tail, sources: &[Source]) -> Option<&'static str> {
    if let Some(fault) = tail_fault(tail, sources) {
        return Some(fault);
    }
    if nothing_to_ask_for(tail) {
        return Some("Say what this tail shows — a selector such as {application=\"ndc\"}.");
    }
    None
}

/// Whether this tail names nothing for the server to answer.
///
/// **Applications with no selector are a selector.** [`crate::apps::with_label`] writes
/// `{app="job-dispatcher"}` for an empty query rather than refusing it, so a tail carrying only
/// applications opens perfectly well — the owner's own `nurtur-loki-jobdispatcher` is that shape.
/// An empty query alone is therefore not the question; having neither is.
fn nothing_to_ask_for(tail: &Tail) -> bool {
    tail.query.trim().is_empty() && tail.apps.iter().all(|app| app.trim().is_empty())
}

/// Why a tail cannot be used, or `None`.
///
/// **A tail's name is not a credential key**, so it is held to less than a source's: it must be
/// something, and it must say which source it reads.
fn tail_fault(tail: &Tail, sources: &[Source]) -> Option<&'static str> {
    if tail.name.trim().is_empty() {
        return Some("Give the tail a name.");
    }
    if tail.source.trim().is_empty() {
        return Some("Say which source this tail reads.");
    }
    if !sources.iter().any(|s| s.name == tail.source) {
        return Some("This tail reads a source that is not in the list.");
    }
    None
}

/// Why the tail being edited cannot be accepted, or `None`.
///
/// **Stricter than [`tail_fault`], deliberately, and the difference is which gate it guards.**
/// `tail_fault` is the set-level gate that holds the Close button: anything it refuses makes the
/// whole configuration unsaveable. This holds one editor's OK, where the reader is looking at the
/// field in question and can fix it on the spot.
///
/// The query is the difference. A tail with no selector cannot be opened — `query_range` has
/// nothing to ask — so an editor must not accept one. But a settings file migrated from the old
/// shape can contain a `[[source]]` that never had a query, and holding *Close* over that would
/// wedge the dialog on a configuration the reader did not write and may not be able to find: they
/// would have to fix a tail they never made before they could leave a screen they only opened to
/// look. So the set tolerates it, the row says so, and the editor refuses to produce another.
///
/// `taken` is every *other* tail's name. Two tails sharing one is two windows asking one question,
/// where the second looks to `already_asked` like the first already open — so it is refused here as
/// well as in [`Editor::fault`], because here it can be refused next to the box that caused it.
pub fn tail_being_edited(tail: &Tail, taken: &[String]) -> Option<&'static str> {
    if tail.name.trim().is_empty() {
        return Some("Give the tail a name.");
    }
    if taken.iter().any(|name| name.trim() == tail.name.trim()) {
        return Some("A tail of that name already exists.");
    }
    if tail.source.trim().is_empty() {
        return Some("Say which source this tail reads.");
    }
    if nothing_to_ask_for(tail) {
        return Some("Say what this tail shows — a selector such as {application=\"ndc\"}.");
    }
    None
}

/// One server as `File ▸ Open remote` lists it: its name, and the tails under it with the index
/// each one is opened by.
///
/// **The menu mirrors the tree, which is the owner's ask of 2026-10-22**: *"the remote sources menu
/// should reflect the tree by having a multiple level menu"*. A flat list of tails was what the
/// menu had the moment the model gained two levels, and it threw away the one thing the split is
/// for — which server a view reads.
///
/// **It owns its strings** rather than borrowing the settings, because both callers read the shell
/// under a `STATE` borrow and then *drop it* before building the menu — a menu is tracked in a
/// modal loop and holding that borrow across one is the re-entrancy panic this file's deferred
/// dialogs exist to avoid. A menu is built on demand, so the copies cost nothing that matters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuSource {
    pub name: String,
    /// `(index into `tails()`, the tail's name)`, in list order.
    pub tails: Vec<(usize, String)>,
}

/// The sources and their tails, as a menu should nest them.
///
/// **A tail whose source is not in the set comes back under `None`**, the way the tree hangs it at
/// the root: a hand-edited file can produce one, and a menu that silently omitted it would be the
/// configuration quietly disagreeing with the dialog that lists it.
pub fn menu_tree(sources: &[Source], tails: &[Tail]) -> (Vec<MenuSource>, Vec<(usize, String)>) {
    let mut out: Vec<MenuSource> = Vec::with_capacity(sources.len());
    for source in sources {
        out.push(MenuSource {
            name: source.name.clone(),
            tails: tails
                .iter()
                .enumerate()
                .filter(|(_, t)| t.source == source.name)
                .map(|(n, t)| (n, t.name.clone()))
                .collect(),
        });
    }
    let loose: Vec<(usize, String)> = tails
        .iter()
        .enumerate()
        .filter(|(_, t)| !sources.iter().any(|s| s.name == t.source))
        .map(|(n, t)| (n, t.name.clone()))
        .collect();
    (out, loose)
}

/// Which tails `Open` should start, given what is ticked and what is selected.
///
/// **Ticking is the deliberate gesture and it wins.** The owner, 2026-10-22: *"probably the tails
/// should have checkboxes to select multiple tails to start"* — so when any box is ticked, those
/// are the tails, and the highlighted row is only where the caret happens to be. With nothing
/// ticked the selection is the obvious intent, and `Open` on a *source* opens nothing, because a
/// source is a server and has no window of its own.
///
/// **Ordered and deduplicated**, because each one becomes a window: two routes to the same tail
/// must not open it twice, and the order a reader ticked them in is not an order worth preserving
/// over the order they are listed in.
pub fn tails_to_open(ticked: &[usize], chosen: Chosen) -> Vec<usize> {
    if !ticked.is_empty() {
        let mut out = ticked.to_vec();
        out.sort_unstable();
        out.dedup();
        return out;
    }
    match chosen {
        Chosen::Tail(at) => vec![at],
        _ => Vec::new(),
    }
}

/// How many tails the set will **add** — and the bound the two `File ▸ Open remote` id ranges are
/// sized for, which is the one place it is load-bearing rather than a courtesy.
///
/// **It bounds adding, not loading**; see [`MAX_SOURCES`], whose comment made the same wrong claim.
/// `remote_menu_of` truncates the menu here and names what it left out, so a longer hand-edited
/// file cannot run an item's id into the next range.
pub const MAX_TAILS: usize = 512;

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

impl Chosen {
    /// This selection as one number, for a tree item to carry.
    ///
    /// **A `SysTreeView32` item holds an `LPARAM` and nothing else**, and what the dialog needs back
    /// from a click is *which row of which list* — two numbers. Packing them is the shell's only
    /// honest option, so the packing is decided here, beside the type, with a test: the alternative
    /// is a parallel `Vec<Chosen>` indexed by insertion order, which is a second model of the tree
    /// that can disagree with the first.
    ///
    /// `Nothing` is 0, a source is odd, a tail is even and non-zero. That keeps the kind in the low
    /// bit, so a stray or stale `LPARAM` cannot be read as the wrong kind of row — only as a row
    /// that is out of range, which [`Editor::choose`] already treats as a deselection.
    pub fn packed(self) -> isize {
        match self {
            Chosen::Nothing => 0,
            Chosen::Source(at) => 1 + (at as isize) * 2,
            Chosen::Tail(at) => 2 + (at as isize) * 2,
        }
    }

    /// The inverse. Anything that was not produced by [`Self::packed`] reads as `Nothing` rather
    /// than as a row, because a tree item whose data this cannot account for is not a row this
    /// dialog put there.
    pub fn unpack(packed: isize) -> Chosen {
        if packed <= 0 {
            return Chosen::Nothing;
        }
        match packed % 2 {
            1 => Chosen::Source(((packed - 1) / 2) as usize),
            _ => Chosen::Tail(((packed - 2) / 2) as usize),
        }
    }
}

/// How one node of the tree reads, with what is wrong with it and what it is missing.
///
/// **The tree has no columns, so the row has to say it.** The list this replaced had a `Secret`
/// column reading `stored` or `—` and a `Problem` column; both were how a reader found out that a
/// source was configured but unusable, which is the commonest half-finished state there is. A tree
/// item is one string, so they are appended to it.
///
/// **`(no secret stored)` is said only of a source that wants one.** A source that signs the reader
/// in through a browser legitimately has no client secret and never will, so labelling it would be
/// telling every correctly-configured sign-in source that something is missing — which is how a
/// warning stops being read.
pub fn node_label(node: &Node<'_>) -> String {
    let mut out = node.name.to_owned();
    if node.wants_secret && !node.has_secret {
        out.push_str("  (no secret stored)");
    }
    if let Some(fault) = node.fault {
        out.push_str("  — ");
        out.push_str(fault);
    }
    out
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
            auth_url: String::new(),
        }
    }

    fn tail(name: &str, of: &str) -> Tail {
        Tail {
            name: name.to_owned(),
            source: of.to_owned(),
            query: format!("{{application=\"{name}\"}}"),
            apps: Vec::new(),
            columns: Vec::new(),
        }
    }

    /// An editor over two sources, opened, where **both** have a tail.
    ///
    /// **`spare` has one on purpose.** It had none until a review pointed out what that cost: with
    /// every tail reading one source, `remove`'s `retain(|t| t.source != gone.name)` and
    /// `replace_source`'s `filter(|t| t.source == was)` could both be replaced by "do it to all of
    /// them" and the tests named for them would still pass. A fixture where the distinction cannot
    /// be observed is a fixture that cannot catch the bug.
    fn opened() -> Editor {
        let mut editor = Editor::new(
            vec![source("loki"), source("spare")],
            vec![
                tail("ndc", "loki"),
                tail("identity", "loki"),
                tail("spare-view", "spare"),
            ],
            vec![true, false],
        );
        editor.open();
        editor
    }

    /// **The shape the owner asked for, flattened in drawing order.** 2026-10-21: *"instead of a
    /// simple list, we would have a list of sources, each of which might be a node on a tree with
    /// the tails unerneath them."* A tail follows the source it reads, and `under` says which.
    #[test]
    fn the_tree_puts_each_tail_under_the_source_it_reads() {
        let editor = opened();
        let nodes = editor.nodes();
        let shape: Vec<(&str, Option<usize>)> = nodes.iter().map(|n| (n.name, n.under)).collect();
        assert_eq!(
            shape,
            [
                ("loki", None),
                ("ndc", Some(0)),
                ("identity", Some(0)),
                ("spare", None),
                ("spare-view", Some(1)),
            ],
            "sources in order, each followed by its own tails"
        );
        // **And each node says which row it is**, which is what the tree item carries and what a
        // click hands back. A source and an orphaned tail both have `under: None`, so this is the
        // only thing that tells them apart — the shell must never have to count.
        let rows: Vec<Chosen> = nodes.iter().map(|n| n.at).collect();
        assert_eq!(
            rows,
            [
                Chosen::Source(0),
                Chosen::Tail(0),
                Chosen::Tail(1),
                Chosen::Source(1),
                Chosen::Tail(2),
            ]
        );
        for node in &nodes {
            assert_eq!(
                Chosen::unpack(node.at.packed()),
                node.at,
                "and it survives the tree item"
            );
        }
    }

    /// **One credential per source, however many tails read it** — the whole reason the split
    /// exists. A tail must never report a secret of its own: it has none to report, and a tree that
    /// showed one would be inviting the reader to give three credentials to one server, which is
    /// the state this product's own store is in.
    #[test]
    fn only_a_source_carries_a_secret() {
        let editor = opened();
        let nodes = editor.nodes();
        let flags: Vec<(&str, bool)> = nodes.iter().map(|n| (n.name, n.has_secret)).collect();
        assert_eq!(
            flags,
            [
                ("loki", true),
                ("ndc", false),
                ("identity", false),
                ("spare", false),
                ("spare-view", false),
            ]
        );
    }

    /// **A tail whose source has gone is shown, faulted, under nothing.** A hand-edited settings
    /// file produces one, and a row that silently vanishes is how a reader concludes their
    /// configuration was eaten.
    #[test]
    fn a_tail_with_no_source_is_shown_and_says_why() {
        let mut editor = Editor::new(
            vec![source("loki")],
            vec![tail("ndc", "loki"), tail("orphan", "gone")],
            vec![false],
        );
        editor.open();
        let nodes = editor.nodes();
        let orphan = nodes
            .iter()
            .find(|n| n.name == "orphan")
            .expect("it is still listed");
        assert_eq!(
            orphan.under, None,
            "it hangs under nothing, having no parent"
        );
        assert_eq!(
            orphan.at,
            Chosen::Tail(1),
            "**and it is still tail 1**, although it is drawn last: a row drawn out of order that              reported its drawing position would hand Edit the wrong tail"
        );
        assert_eq!(
            orphan.fault,
            Some("This tail reads a source that is not in the list.")
        );
        assert!(
            editor.fault().is_some(),
            "and the set cannot be saved while it is there"
        );
    }

    /// **`Add tail…` works from either kind of row.** Pointing at a source and pointing at one of
    /// its tails both mean "add a tail to this source", and a reader who has just clicked a tail is
    /// exactly the reader who wants another one beside it.
    #[test]
    fn the_chosen_source_is_the_one_underneath_a_chosen_tail() {
        let mut editor = opened();
        editor.choose(Chosen::Source(1));
        assert_eq!(editor.chosen_source(), Some(1));
        editor.choose(Chosen::Tail(0));
        assert_eq!(
            editor.chosen_source(),
            Some(0),
            "the tail reads `loki`, which is source 0"
        );
        editor.choose(Chosen::Nothing);
        assert_eq!(editor.chosen_source(), None);
    }

    /// Out of range is a deselection, not a panic: the tree reports what it has and this is told
    /// rather than asked.
    #[test]
    fn choosing_a_row_that_is_not_there_selects_nothing() {
        let mut editor = opened();
        editor.choose(Chosen::Source(99));
        assert_eq!(editor.chosen(), Chosen::Nothing);
        editor.choose(Chosen::Tail(99));
        assert_eq!(editor.chosen(), Chosen::Nothing);
    }

    /// **A rename carries the children.** A tail names its source by *name*, so renaming the source
    /// without moving its tails orphans every one of them — the same mistake, one level up, as
    /// filing a credential under a view's name.
    #[test]
    fn renaming_a_source_takes_its_tails_with_it() {
        let mut editor = opened();
        editor.choose(Chosen::Source(0));
        editor.replace_source(Source {
            name: "nurtur-loki".to_owned(),
            ..source("loki")
        });
        let reads: Vec<&str> = editor.tails().iter().map(|t| t.source.as_str()).collect();
        assert_eq!(
            reads,
            ["nurtur-loki", "nurtur-loki", "spare"],
            "**its own two tails followed it and the other source's did not.** Without a tail on \
             `spare` this could not tell a carried rename from one that renamed every tail there is"
        );
        assert!(editor.fault().is_none(), "nothing was orphaned: {reads:?}");
        assert!(
            editor.source_has_secret(0),
            "the credential follows the row through a rename; until 2026-10-26 this asserted the \
             opposite, and the row read `(no secret stored)` the moment a name was corrected"
        );
    }

    /// **Deleting a source deletes its tails, and forgets only its own credential.** A tail reading
    /// a server that is gone has nowhere to ask; and a tail owns no credential, so removing one
    /// must never reach the store.
    #[test]
    fn deleting_a_source_takes_its_tails_and_names_one_credential() {
        let mut editor = opened();
        editor.choose(Chosen::Source(0));
        editor.remove();
        let plan = editor.secret_plan();
        assert_eq!(plan.forget, ["loki"], "its own name, and nothing else");
        assert!(plan.moves.is_empty() && plan.store.is_empty() && plan.clear.is_empty());
        assert_eq!(editor.sources().len(), 1);
        let left: Vec<&str> = editor.tails().iter().map(|t| t.name.as_str()).collect();
        assert_eq!(
            left,
            ["spare-view"],
            "**its own two tails went and the other source's stayed.** A fixture where every tail \
             read one source could not tell this from `tails.clear()`"
        );
    }

    #[test]
    fn deleting_a_tail_touches_no_credential() {
        let mut editor = opened();
        let before = editor.tails().len();
        editor.choose(Chosen::Tail(1));
        editor.remove();
        assert!(
            editor.secret_plan().is_empty(),
            "a tail has no credential of its own to forget"
        );
        assert_eq!(editor.tails().len(), before - 1, "exactly the one chosen");
        let left: Vec<&str> = editor.tails().iter().map(|t| t.name.as_str()).collect();
        assert_eq!(left, ["ndc", "spare-view"], "and the others are untouched");
        assert_eq!(editor.sources().len(), 2, "the server it read stays");
    }

    fn renamed(name: &str) -> Renamed {
        Renamed {
            now: name.to_owned(),
            was: Vec::new(),
        }
    }

    fn moved(from: &str, to: &str) -> Renamed {
        Renamed {
            was: vec![from.to_owned()],
            ..renamed(to)
        }
    }

    /// A source that signs the reader in through a browser: no secret, but a refresh token filed
    /// under its name once they have.
    fn browser(name: &str) -> Source {
        Source {
            auth_url: "https://identity.example/connect/authorize".to_owned(),
            ..source(name)
        }
    }

    /// **The defect the review found on 2026-10-22:** a source created with a secret and renamed
    /// before Close lost the secret, because the name it was typed under did not survive the
    /// sitting. The row knows which source the secret belongs to; the name is where it ends up.
    #[test]
    fn a_secret_typed_for_a_new_source_survives_renaming_it_in_the_same_sitting() {
        let mut editor = opened();
        assert!(editor.add_source(source("fresh")));
        editor.secret_typed("s3cret".to_owned());
        assert!(editor.source_has_secret(2));
        editor.replace_source(source("renamed"));
        assert!(editor.source_has_secret(2), "the row still has it");
        let plan = editor.secret_plan();
        assert_eq!(plan.store, [("renamed".to_owned(), "s3cret".to_owned())]);
        assert!(
            plan.moves.is_empty() && plan.forget.is_empty(),
            "nothing is in the store for a source that did not exist when the dialog opened: {plan:?}"
        );
    }

    /// **A rename moves the credential, from the original name straight to the current one** —
    /// however many names the source passed through. `a→b→c` used to move nothing, because the
    /// move `a→b` was reconciled against the surviving names and `b` was not one of them.
    #[test]
    fn a_source_renamed_twice_moves_its_credential_once_from_where_it_started() {
        let mut editor = opened();
        editor.choose(Chosen::Source(0));
        editor.replace_source(source("loki-b"));
        editor.replace_source(source("loki-c"));
        let plan = editor.secret_plan();
        assert_eq!(plan.moves, [moved("loki", "loki-c")]);
        assert!(plan.forget.is_empty() && plan.store.is_empty() && plan.clear.is_empty());
        assert!(editor.source_has_secret(0));
    }

    /// Typing a new secret and then renaming, in one sitting: the old entry moves (it carries the
    /// sign-in half too), and the new secret is written under the new name afterwards. The order of
    /// the plan's fields is the order the shell applies them, so the write lands last.
    #[test]
    fn a_secret_typed_and_then_renamed_is_written_under_the_new_name_after_the_move() {
        let mut editor = opened();
        editor.choose(Chosen::Source(0));
        editor.secret_typed("newer".to_owned());
        editor.replace_source(source("loki-2"));
        let plan = editor.secret_plan();
        assert_eq!(plan.moves, [moved("loki", "loki-2")]);
        assert_eq!(plan.store, [("loki-2".to_owned(), "newer".to_owned())]);
        assert!(plan.forget.is_empty() && plan.clear.is_empty());
    }

    /// **A mode with no use for a secret gives the stored one up — and only the secret.** The
    /// entry still moves with a rename, and the clear is under the name it moved to, because the
    /// sign-in half is filed beside the secret and must survive: a browser source carrying a stale
    /// client secret from before 2026-10-21 would otherwise be signed out by pressing Close.
    #[test]
    fn a_source_that_stops_wanting_a_secret_clears_only_the_secret_under_its_current_name() {
        let mut editor = opened();
        editor.choose(Chosen::Source(0));
        editor.replace_source(browser("loki-browser"));
        let plan = editor.secret_plan();
        assert_eq!(plan.moves, [moved("loki", "loki-browser")]);
        assert_eq!(plan.clear, ["loki-browser"]);
        assert!(plan.forget.is_empty() && plan.store.is_empty(), "{plan:?}");
        assert!(!editor.source_has_secret(0));
    }

    /// The case the review caught before it shipped: a browser source that already had a stale
    /// secret in the store when the dialog opened, touched only by a rename. Its sign-in must
    /// follow the rename and nothing may be forgotten; the stale secret is cleared where it landed.
    #[test]
    fn a_signed_in_source_with_a_stale_secret_is_renamed_without_being_signed_out() {
        let mut editor = Editor::new(vec![browser("live")], Vec::new(), vec![true]);
        editor.open();
        assert!(
            !editor.source_has_secret(0),
            "a browser source has no secret to speak of"
        );
        assert_eq!(
            editor.secret_plan().clear,
            ["live"],
            "untouched, the stale secret is still cleared, and nothing else happens: {:?}",
            editor.secret_plan()
        );
        editor.choose(Chosen::Source(0));
        editor.replace_source(browser("live-2"));
        let plan = editor.secret_plan();
        assert_eq!(plan.moves, [moved("live", "live-2")]);
        assert_eq!(plan.clear, ["live-2"]);
        assert!(
            plan.forget.is_empty(),
            "forgetting would take the refresh token: {plan:?}"
        );
        assert!(plan.store.is_empty());
    }

    /// **A browser sign-in has no secret but it has a refresh token under the same name**, so a
    /// rename of one must still move the entry — this is why the plan moves on the row, not on
    /// whether a secret was stored. And a secret typed into a mode that does not want one is not
    /// written.
    #[test]
    fn a_browser_sign_in_renamed_moves_its_entry_and_a_secret_it_cannot_use_is_not_written() {
        let mut editor = Editor::new(vec![browser("live")], Vec::new(), vec![false]);
        editor.open();
        editor.choose(Chosen::Source(0));
        editor.secret_typed("ignored".to_owned());
        editor.replace_source(browser("live-2"));
        let plan = editor.secret_plan();
        assert_eq!(plan.moves, [moved("live", "live-2")]);
        assert!(plan.store.is_empty(), "{plan:?}");
        assert!(plan.forget.is_empty() && plan.clear.is_empty());
    }

    /// Delete `loki`, then rename `spare` to `loki`: the deleted entry is forgotten **before** the
    /// rename lands on its name, which is what the plan's field order promises. And a source added
    /// under a deleted name is written after the forget for the same reason.
    #[test]
    fn a_deleted_name_is_forgotten_before_anything_lands_on_it() {
        let mut editor = opened();
        editor.choose(Chosen::Source(0));
        editor.remove();
        editor.choose(Chosen::Source(0));
        editor.replace_source(source("loki"));
        let plan = editor.secret_plan();
        assert_eq!(plan.forget, ["loki"]);
        assert_eq!(plan.moves, [moved("spare", "loki")]);

        let mut editor = opened();
        editor.choose(Chosen::Source(0));
        editor.remove();
        assert!(editor.add_source(source("loki")));
        editor.secret_typed("again".to_owned());
        let plan = editor.secret_plan();
        assert_eq!(plan.forget, ["loki"]);
        assert_eq!(plan.store, [("loki".to_owned(), "again".to_owned())]);
        assert!(plan.moves.is_empty());
    }

    /// An emptied box clears the secret and only the secret — and only where there was one; a
    /// source that never had one has nothing to clear.
    #[test]
    fn an_emptied_box_clears_a_stored_secret_and_nothing_else() {
        let mut editor = opened();
        editor.choose(Chosen::Source(0));
        editor.secret_typed(String::new());
        assert!(!editor.source_has_secret(0));
        let plan = editor.secret_plan();
        assert_eq!(plan.clear, ["loki"]);
        assert!(plan.forget.is_empty() && plan.moves.is_empty() && plan.store.is_empty());

        editor.choose(Chosen::Source(1));
        editor.secret_typed(String::new());
        assert_eq!(
            editor.secret_plan().clear,
            ["loki"],
            "`spare` had none to clear"
        );
    }

    /// Nothing touched, nothing planned; and a clone carries no credential into the plan.
    #[test]
    fn an_untouched_sitting_and_a_clone_plan_nothing() {
        let mut editor = opened();
        assert!(editor.secret_plan().is_empty());
        editor.choose(Chosen::Source(0));
        assert!(editor.clone_chosen());
        assert!(
            editor.secret_plan().is_empty(),
            "{:?}",
            editor.secret_plan()
        );
    }

    /// What a confirmation has to be able to say before a delete, because the count is the whole
    /// reason to ask.
    #[test]
    fn a_source_knows_how_many_tails_would_go_with_it() {
        let mut editor = opened();
        assert_eq!(editor.tails_of(0), 2);
        assert_eq!(editor.tails_of(1), 1, "its own, not everyone's");
        assert_eq!(editor.tails_of(99), 0, "and nothing at all for no source");
        // **A source with no tails reports none**, which the fixture used to say by accident and
        // now says on purpose: `spare` was the no-tails case until it was given one to make
        // `remove` and `replace_source` testable, so the case needs a source of its own.
        assert!(editor.add_source(source("untouched")));
        assert_eq!(editor.tails_of(2), 0);
    }

    /// **Two sources sharing a name is the fault no single row can see**, and so is two tails: the
    /// first is a shared credential key, the second a shared question — three windows one of which
    /// would look to `already_asked` like the others already open.
    #[test]
    fn two_rows_of_a_kind_may_not_share_a_name() {
        let mut editor = Editor::new(
            vec![source("loki"), source("loki")],
            Vec::new(),
            vec![false, false],
        );
        editor.open();
        assert_eq!(
            editor.fault(),
            Some("Two sources share a name; each needs its own.")
        );

        let mut tails = Editor::new(
            vec![source("loki")],
            vec![tail("ndc", "loki"), tail("ndc", "loki")],
            vec![false],
        );
        tails.open();
        assert_eq!(
            tails.fault(),
            Some("Two tails share a name; each needs its own.")
        );
    }

    #[test]
    fn a_tail_needs_a_name_and_a_source() {
        let mut editor = Editor::new(
            vec![source("loki")],
            vec![Tail {
                name: "  ".to_owned(),
                ..tail("ndc", "loki")
            }],
            vec![false],
        );
        editor.open();
        assert_eq!(editor.fault(), Some("Give the tail a name."));

        let mut nameless = Editor::new(
            vec![source("loki")],
            vec![Tail {
                source: String::new(),
                ..tail("ndc", "loki")
            }],
            vec![false],
        );
        nameless.open();
        assert_eq!(nameless.fault(), Some("Say which source this tail reads."));
    }

    /// Adding selects what was added, so the reader's next click lands on it.
    #[test]
    fn adding_selects_what_was_added_and_marks_the_set_dirty() {
        let mut editor = opened();
        assert!(!editor.is_dirty());
        assert!(editor.add_source(source("third")));
        assert_eq!(editor.chosen(), Chosen::Source(2));
        assert!(editor.is_dirty());
        editor.saved();

        assert!(editor.add_tail(tail("jobs", "third")));
        assert_eq!(
            editor.chosen(),
            Chosen::Tail(editor.tails().len() - 1),
            "the row just added, wherever the fixture put it"
        );
        assert!(editor.is_dirty());
    }

    /// Both bounds are refusals rather than truncations, and neither is silent — the caller says so.
    #[test]
    fn neither_list_grows_past_its_bound() {
        let mut editor = Editor::new(
            (0..MAX_SOURCES).map(|n| source(&format!("s{n}"))).collect(),
            Vec::new(),
            Vec::new(),
        );
        editor.open();
        assert!(!editor.add_source(source("one too many")));
        assert_eq!(editor.sources().len(), MAX_SOURCES);

        let mut tails = Editor::new(
            vec![source("loki")],
            (0..MAX_TAILS)
                .map(|n| tail(&format!("t{n}"), "loki"))
                .collect(),
            vec![false],
        );
        tails.open();
        assert!(!tails.add_tail(tail("one too many", "loki")));
        assert_eq!(tails.tails().len(), MAX_TAILS);
    }

    /// **A short `stored` is padded, not rejected.** The answer to "does this have a secret" for a
    /// source the shell did not ask about is "not as far as we know" — and a panic here would take
    /// the dialog down over a question nobody needed answered.
    #[test]
    fn a_stored_list_shorter_than_the_sources_is_padded() {
        let editor = Editor::new(vec![source("a"), source("b")], Vec::new(), vec![true]);
        assert!(editor.source_has_secret(0));
        assert!(!editor.source_has_secret(1));
        assert!(!editor.source_has_secret(99));
    }

    /// Typing the same thing again is not an edit — every accepted editor hands a whole value back,
    /// so a replace that always marked the set dirty would mark it dirty for ever.
    #[test]
    fn replacing_a_row_with_itself_is_not_a_change() {
        let mut editor = opened();
        editor.choose(Chosen::Source(0));
        editor.replace_source(source("loki"));
        assert!(!editor.is_dirty(), "the same source, field for field");

        editor.choose(Chosen::Tail(0));
        editor.replace_tail(tail("ndc", "loki"));
        assert!(!editor.is_dirty(), "and the same tail");

        editor.replace_tail(Tail {
            query: "{application=\"ndc\"} |= `error`".to_owned(),
            ..tail("ndc", "loki")
        });
        assert!(editor.is_dirty(), "a different query is a change");
    }

    /// A replace with nothing chosen changes nothing, rather than writing to row zero.
    #[test]
    fn replacing_with_nothing_chosen_writes_nowhere() {
        let mut editor = opened();
        editor.choose(Chosen::Nothing);
        editor.replace_source(source("overwritten"));
        editor.replace_tail(tail("overwritten", "loki"));
        assert_eq!(editor.source(0).map(|s| s.name.as_str()), Some("loki"));
        assert_eq!(editor.tail(0).map(|t| t.name.as_str()), Some("ndc"));
        assert!(!editor.is_dirty());
    }

    /// **`replace_source` on a chosen tail is not a cross-write**, and vice versa. Each takes only
    /// its own kind of row, because the two lists are indexed separately and `Tail(1)` is a
    /// perfectly valid index into `sources`.
    #[test]
    fn a_replace_of_one_kind_ignores_a_chosen_row_of_the_other() {
        let mut editor = opened();
        editor.choose(Chosen::Tail(1));
        editor.replace_source(source("clobbered"));
        assert_eq!(
            editor.source(1).map(|s| s.name.as_str()),
            Some("spare"),
            "source 1 was not written through a chosen tail 1"
        );

        editor.choose(Chosen::Source(0));
        editor.replace_tail(tail("clobbered", "loki"));
        assert_eq!(editor.tail(0).map(|t| t.name.as_str()), Some("ndc"));
    }

    /// **A clone carries everything but the name and the sign-in.** The owner, 2026-10-20: *"it
    /// would also be nice to be able to clone a setup and then edit one field"*.
    ///
    /// **A source's tails come with it**, which is the version of this that is worth having: the
    /// estate runs one Loki per environment and the same three applications on each, so cloning
    /// `live` to make `qa` and then editing one URL is the whole labour saved. Cloning the server
    /// alone would leave a copy that shows nothing until three tails are typed again.
    ///
    /// **The copy inherits no secret, and that is correctness rather than caution.** A credential is
    /// filed under the source's *name*, so a copy reported as having one would be pointing at the
    /// original's — and renaming it later would orphan that one, which is a thing that has already
    /// happened in this product's store.
    #[test]
    fn cloning_a_source_copies_its_tails_and_no_credential() {
        let mut editor = opened();
        editor.choose(Chosen::Source(0));
        assert!(editor.clone_chosen());
        let Chosen::Source(at) = editor.chosen() else {
            panic!("the copy is selected: {:?}", editor.chosen());
        };
        let copy = editor.source(at).expect("it is there").clone();
        assert_ne!(copy.name, "loki", "a name of its own");
        assert_eq!(copy.url, source("loki").url, "and everything else carried");
        assert!(
            !editor.source_has_secret(at),
            "no secret, because the store is keyed on the name"
        );
        assert_eq!(
            editor.tails_of(at),
            2,
            "both tails were copied onto the new server"
        );
        assert_eq!(editor.tails_of(0), 2, "and the original kept its own");
        assert!(
            editor.fault().is_none(),
            "every copied tail got a name of its own: {:?}",
            editor.tails().iter().map(|t| &t.name).collect::<Vec<_>>()
        );
    }

    /// **The collision the clone's own doc comment names, which nothing tested.**
    ///
    /// *"two tails called `ndc` and `ndc copy` would otherwise both want `ndc copy`"* — and with
    /// that pair in the set, dropping the line that grows the taken-names list as each copy is
    /// named produces exactly that, which `Editor::fault` then refuses. The fixture has to contain
    /// the pair for the assertion to mean anything.
    #[test]
    fn cloning_a_source_names_each_copied_tail_against_the_copies_already_named() {
        let mut editor = Editor::new(
            vec![source("loki")],
            vec![tail("ndc", "loki"), tail("ndc copy", "loki")],
            vec![false],
        );
        editor.open();
        editor.choose(Chosen::Source(0));
        assert!(editor.clone_chosen());

        let names: Vec<&str> = editor.tails().iter().map(|t| t.name.as_str()).collect();
        let mut unique: Vec<&str> = names.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(
            unique.len(),
            names.len(),
            "two tails ended up sharing a name: {names:?}"
        );
        assert_eq!(
            editor.fault(),
            None,
            "which is what the set would have refused: {names:?}"
        );
    }

    /// A cloned tail is a second view of the **same** server — no second credential, no second
    /// connection, which is exactly what the old one-entry-per-view model could not express.
    #[test]
    fn cloning_a_tail_adds_a_view_of_the_same_source() {
        let mut editor = opened();
        editor.choose(Chosen::Tail(0));
        assert!(editor.clone_chosen());
        let Chosen::Tail(at) = editor.chosen() else {
            panic!("the copy is selected: {:?}", editor.chosen());
        };
        let copy = editor.tail(at).expect("it is there").clone();
        assert_eq!(copy.source, "loki", "the same server");
        assert_eq!(copy.query, tail("ndc", "loki").query, "and the same query");
        assert_ne!(copy.name, "ndc", "under a name of its own");
        assert_eq!(editor.sources().len(), 2, "no source was added");
        assert!(editor.fault().is_none());
    }

    #[test]
    fn there_is_nothing_to_clone_with_nothing_chosen_and_nothing_to_clone_into_a_full_list() {
        let mut editor = opened();
        editor.choose(Chosen::Nothing);
        assert!(!editor.clone_chosen());
        assert!(!editor.is_dirty());

        let mut full = Editor::new(
            (0..MAX_SOURCES).map(|n| source(&format!("s{n}"))).collect(),
            Vec::new(),
            Vec::new(),
        );
        full.open();
        full.choose(Chosen::Source(0));
        assert!(
            !full.clone_chosen(),
            "and it says no rather than truncating"
        );
        assert_eq!(full.sources().len(), MAX_SOURCES);
    }

    /// **A source whose tails would not fit is not half-cloned.** Copying the server and then
    /// finding there is no room for its views would leave a connection showing nothing, which is
    /// worse than the refusal — and it would be marked dirty, so Close would write it.
    #[test]
    fn a_source_clone_that_cannot_carry_its_tails_is_refused_whole() {
        let mut editor = Editor::new(
            vec![source("loki")],
            (0..MAX_TAILS)
                .map(|n| tail(&format!("t{n}"), "loki"))
                .collect(),
            vec![false],
        );
        editor.open();
        editor.choose(Chosen::Source(0));
        assert!(!editor.clone_chosen());
        assert_eq!(editor.sources().len(), 1, "no half-made server");
        assert_eq!(editor.tails().len(), MAX_TAILS);
        assert!(!editor.is_dirty());
    }

    /// Opening selects the first source, and an empty set selects nothing. **No blank row is
    /// offered**: the dialog that needed one showed editable fields beside the list, and a form with
    /// no row behind it discarded every keystroke. `New source…` is that row now.
    #[test]
    fn opening_lands_on_the_first_source_or_on_nothing() {
        let editor = opened();
        assert_eq!(editor.chosen(), Chosen::Source(0));
        assert!(editor.is_open());

        let mut empty = Editor::new(Vec::new(), Vec::new(), Vec::new());
        empty.open();
        assert_eq!(empty.chosen(), Chosen::Nothing);
        assert!(
            empty.sources().is_empty() && empty.tails().is_empty(),
            "and nothing was invented to fill it"
        );
    }

    /// An unnamed row still draws, because a blank line in a tree is unclickable and unexplainable.
    #[test]
    fn a_row_with_no_name_is_drawn_as_unnamed() {
        let mut editor = Editor::new(
            vec![Source {
                name: String::new(),
                ..source("loki")
            }],
            vec![Tail {
                name: String::new(),
                source: String::new(),
                ..tail("ndc", "loki")
            }],
            vec![false],
        );
        editor.open();
        let names: Vec<&str> = editor.nodes().iter().map(|n| n.name).collect();
        assert_eq!(names, ["(unnamed)", "(unnamed)"]);
    }

    #[test]
    fn the_applications_round_trip_through_the_box() {
        assert_eq!(
            apps_typed(" ndc , identity ,, "),
            ["ndc".to_owned(), "identity".to_owned()]
        );
        assert_eq!(apps_said(&apps_typed("ndc,identity")), "ndc, identity");
        assert!(apps_typed("  ,, ").is_empty());
    }

    /// **The packing is a round trip or it is a bug**, and it is the only thing standing between a
    /// click on one row and the dialog acting on another.
    #[test]
    fn a_selection_survives_being_packed_into_a_tree_item() {
        let every: Vec<Chosen> = std::iter::once(Chosen::Nothing)
            .chain((0..MAX_SOURCES).map(Chosen::Source))
            .chain((0..MAX_TAILS).map(Chosen::Tail))
            .collect();
        for chosen in &every {
            assert_eq!(
                Chosen::unpack(chosen.packed()),
                *chosen,
                "{chosen:?} did not come back"
            );
        }
        // **No two rows pack to the same number**, which is the failure that would hand a click on
        // a tail to a source.
        let mut packed: Vec<isize> = every.iter().map(|c| c.packed()).collect();
        let count = packed.len();
        packed.sort_unstable();
        packed.dedup();
        assert_eq!(packed.len(), count, "two rows share a number");
    }

    /// Anything this did not write reads as nothing, rather than as row zero of a list.
    #[test]
    fn a_tree_item_carrying_nothing_this_wrote_is_not_a_row() {
        assert_eq!(Chosen::unpack(0), Chosen::Nothing);
        assert_eq!(Chosen::unpack(-1), Chosen::Nothing);
        assert_eq!(Chosen::unpack(isize::MIN), Chosen::Nothing);
    }

    /// **What the row says, now that there are no columns to say it in.** The list this replaced
    /// had a `Secret` column and a `Problem` column, and both were how a reader learned that a
    /// server was configured and unusable.
    #[test]
    fn a_row_says_what_is_wrong_with_it_and_what_it_is_missing() {
        let plain = Node {
            name: "loki",
            at: Chosen::Source(0),
            under: None,
            fault: None,
            has_secret: true,
            wants_secret: true,
        };
        assert_eq!(
            node_label(&plain),
            "loki",
            "a ready source says only its name"
        );

        assert_eq!(
            node_label(&Node {
                has_secret: false,
                ..plain.clone()
            }),
            "loki  (no secret stored)"
        );
        // **And a source that wants no secret is never told it is missing one.** A browser sign-in
        // has none until the first sign-in and a server wanting no token never will; labelling
        // either is how a warning stops being read.
        assert_eq!(
            node_label(&Node {
                has_secret: false,
                wants_secret: false,
                ..plain.clone()
            }),
            "loki"
        );
        assert_eq!(
            node_label(&Node {
                fault: Some("Give the source a URL."),
                ..plain.clone()
            }),
            "loki  — Give the source a URL."
        );
        assert_eq!(
            node_label(&Node {
                has_secret: false,
                fault: Some("Give the source a URL."),
                ..plain
            }),
            "loki  (no secret stored)  — Give the source a URL.",
            "both, when both are true"
        );
    }

    /// Which sources the tree marks as wanting a secret, read off the same `mode_of` the editor
    /// uses — so the row and the screen that edits it cannot disagree about what is missing.
    #[test]
    fn only_a_client_secret_source_is_marked_as_wanting_one() {
        let signs_in = Source {
            auth_url: "https://identity.example/connect/authorize".to_owned(),
            ..source("live")
        };
        let open = Source {
            token_url: String::new(),
            client_id: String::new(),
            scope: String::new(),
            ..source("open")
        };
        let mut editor = Editor::new(
            vec![source("secret"), signs_in, open],
            Vec::new(),
            vec![false, false, false],
        );
        editor.open();
        let wants: Vec<(&str, bool)> = editor
            .nodes()
            .iter()
            .map(|n| (n.name, n.wants_secret))
            .collect();
        assert_eq!(wants, [("secret", true), ("live", false), ("open", false)]);
    }

    /// **The editor is stricter than the set, and the query is the whole difference.** A tail with
    /// no selector cannot be opened; but a file migrated from the old shape can hold one, and
    /// refusing *Close* over it would wedge the dialog on a configuration the reader never wrote.
    #[test]
    fn the_tail_editor_insists_on_a_selector_where_the_set_tolerates_one() {
        let blank = Tail {
            query: String::new(),
            ..tail("ndc", "loki")
        };
        assert_eq!(
            tail_being_edited(&blank, &[]),
            Some("Say what this tail shows — a selector such as {application=\"ndc\"}.")
        );
        assert_eq!(
            tail_fault(&blank, &[source("loki")]),
            None,
            "**and the set still saves**, or a migrated file could not be closed"
        );
        assert_eq!(tail_being_edited(&tail("ndc", "loki"), &[]), None);
    }

    /// The editor refuses a duplicate name next to the box that caused it, rather than leaving the
    /// reader to discover it from the fault line after Close.
    #[test]
    fn the_tail_editor_refuses_a_name_another_tail_already_has() {
        let taken = ["identity".to_owned(), "ndc".to_owned()];
        assert_eq!(
            tail_being_edited(&tail("ndc", "loki"), &taken),
            Some("A tail of that name already exists.")
        );
        assert_eq!(
            tail_being_edited(&tail(" ndc ", "loki"), &taken),
            Some("A tail of that name already exists."),
            "spacing is not a different name"
        );
        assert_eq!(tail_being_edited(&tail("jobs", "loki"), &taken), None);
        assert_eq!(
            tail_being_edited(
                &Tail {
                    name: "  ".to_owned(),
                    ..tail("ndc", "loki")
                },
                &taken
            ),
            Some("Give the tail a name."),
            "and a blank name is reported as blank, not as taken"
        );
    }

    /// **Sources come back in index order, every source before its own children.**
    ///
    /// This is what the tree's fill depends on and the reason it can be one pass: it keeps the
    /// handle of each source it inserts, indexed by `Chosen::Source(at)`, and a tail arrives with
    /// `under: Some(at)` naming the parent to hang it from — so the parent must already have been
    /// inserted. Pinned here because a reordering of `nodes()` would otherwise be found only on
    /// screen, as tails hanging at the root.
    ///
    /// **And a tail's `at` is its index in `tails()`, never its position in the drawing.** That is
    /// the half that was got wrong: the shell derived the index from a counter that incremented in
    /// drawing order, and `nodes()` groups tails under their source rather than emitting them in
    /// index order — so with two sources whose tails interleave, a click on one row acted on
    /// another. `Edit…` edited the wrong tail and `Delete` deleted the wrong one.
    ///
    /// **This test could not have caught that, and saying so matters more than the coverage it
    /// claims.** The field was always right; the view ignored it. No pure function can stop a
    /// window procedure re-deriving what it was handed — only a screenshot or a test that owns a
    /// window can, and this project has neither for this dialog. What this pins is the invariant the
    /// fill now leans on, so a change to `nodes()` breaks loudly here instead of quietly there.
    #[test]
    fn the_nodes_come_back_in_an_order_the_tree_can_fill_in_one_pass() {
        // tails[0] reads the *second* source, which is what makes drawing order differ from index
        // order — exactly the arrangement the counting fill got wrong.
        let mut editor = Editor::new(
            vec![source("first"), source("second")],
            vec![tail("b", "second"), tail("a", "first")],
            vec![false, false],
        );
        editor.open();
        let nodes = editor.nodes();

        let mut seen_sources = 0usize;
        for node in &nodes {
            match node.at {
                Chosen::Source(at) => {
                    assert_eq!(at, seen_sources, "sources arrive in index order: {at}");
                    seen_sources += 1;
                }
                Chosen::Tail(_) => {
                    if let Some(under) = node.under {
                        assert!(
                            under < seen_sources,
                            "{:?} hangs under source {under}, which has not been drawn yet",
                            node.name
                        );
                    }
                }
                Chosen::Nothing => panic!("no node is nothing"),
            }
        }

        // And the mapping itself, which is the part a counter cannot reproduce.
        let drawn: Vec<(&str, Chosen)> = nodes.iter().map(|n| (n.name, n.at)).collect();
        assert_eq!(
            drawn,
            [
                ("first", Chosen::Source(0)),
                ("a", Chosen::Tail(1)),
                ("second", Chosen::Source(1)),
                ("b", Chosen::Tail(0)),
            ],
            "the first tail *drawn* is tails[1]; a counter would have called it Tail(0)"
        );
    }

    /// **A tail with applications and no selector is complete, because the applications *are* the
    /// selector.** `apps::with_label` writes `{app="job-dispatcher"}` for an empty query rather than
    /// refusing it, so such a tail opens.
    ///
    /// **Found on the owner's own configuration, before the tree had even been looked at.** His
    /// `nurtur-loki-jobdispatcher` is exactly this shape — `apps = ["job-dispatcher"]`, no query —
    /// and the empty-query rule written the day before would have drawn a fault on it in the tree
    /// and refused to let its editor be accepted unchanged. A warning on a working row is how a
    /// warning stops being read, which this project has a scar about already.
    #[test]
    fn applications_with_no_selector_are_the_selector() {
        let his = Tail {
            name: "nurtur-loki-jobdispatcher".to_owned(),
            source: "telemetry.nurtur.tech".to_owned(),
            query: String::new(),
            apps: vec!["job-dispatcher".to_owned()],
            columns: Vec::new(),
        };
        assert_eq!(
            tail_row_fault(&his, &[source("telemetry.nurtur.tech")]),
            None,
            "the row must not warn about a tail that opens"
        );
        assert_eq!(
            tail_being_edited(&his, &[]),
            None,
            "and the editor must accept it unchanged"
        );

        // With neither, there is nothing to ask the server for and both say so.
        let neither = Tail {
            apps: Vec::new(),
            ..his
        };
        assert_eq!(
            tail_row_fault(&neither, &[source("telemetry.nurtur.tech")]),
            Some("Say what this tail shows — a selector such as {application=\"ndc\"}.")
        );
        assert_eq!(
            tail_being_edited(&neither, &[]),
            Some("Say what this tail shows — a selector such as {application=\"ndc\"}.")
        );
    }

    /// **Ticking several tails and pressing Open starts all of them**, which is the owner's ask of
    /// 2026-10-22. The highlighted row is where the caret is, not what was asked for.
    #[test]
    fn ticked_tails_are_what_open_starts() {
        assert_eq!(
            tails_to_open(&[2, 0], Chosen::Tail(1)),
            [0, 2],
            "the ticks win over the selection, in list order"
        );
        assert_eq!(
            tails_to_open(&[1, 1, 1], Chosen::Nothing),
            [1],
            "and a tail reached twice is still one window"
        );
    }

    /// With nothing ticked the selection is the intent — and a **source** is not a window.
    #[test]
    fn with_nothing_ticked_open_takes_the_selected_tail_and_only_a_tail() {
        assert_eq!(tails_to_open(&[], Chosen::Tail(3)), [3]);
        assert!(
            tails_to_open(&[], Chosen::Source(0)).is_empty(),
            "a source is a server; it has no window of its own"
        );
        assert!(tails_to_open(&[], Chosen::Nothing).is_empty());
    }

    /// **The menu nests the way the tree does**, and each tail keeps the index it is opened by —
    /// which is its position in `tails()`, not its position in the menu. Those differ as soon as
    /// two sources' tails interleave, and handing the menu a position would open the wrong one.
    #[test]
    fn the_menu_nests_tails_under_their_source_and_keeps_their_real_index() {
        let editor = opened();
        let (nested, loose) = menu_tree(editor.sources(), editor.tails());
        let shape: Vec<(&str, Vec<(usize, &str)>)> = nested
            .iter()
            .map(|m| {
                (
                    m.name.as_str(),
                    m.tails.iter().map(|(n, t)| (*n, t.as_str())).collect(),
                )
            })
            .collect();
        assert_eq!(
            shape,
            [
                ("loki", vec![(0, "ndc"), (1, "identity")]),
                ("spare", vec![(2, "spare-view")]),
            ]
        );
        assert!(loose.is_empty());
    }

    /// A tail whose source is missing is listed, not dropped — as the tree hangs it at the root.
    #[test]
    fn a_tail_with_no_source_is_still_on_the_menu() {
        let mut editor = Editor::new(
            vec![source("loki")],
            vec![tail("ndc", "loki"), tail("orphan", "gone")],
            vec![false],
        );
        editor.open();
        let (nested, loose) = menu_tree(editor.sources(), editor.tails());
        assert_eq!(nested.len(), 1);
        assert_eq!(nested[0].tails, [(0, "ndc".to_owned())]);
        assert_eq!(
            loose,
            [(1, "orphan".to_owned())],
            "under no server, carrying the index that opens it"
        );
    }

    /// A source with no tails is still a row, because it is still a server the reader configured.
    #[test]
    fn a_source_with_no_tails_is_on_the_menu_with_nothing_under_it() {
        let mut editor = Editor::new(vec![source("empty")], Vec::new(), vec![false]);
        editor.open();
        let (nested, _) = menu_tree(editor.sources(), editor.tails());
        assert_eq!(nested.len(), 1);
        assert!(nested[0].tails.is_empty());
    }
}
