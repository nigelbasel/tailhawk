//! Persisted state — `SPEC.md` §12.4, E28, brought forward: window placement and per-file state.
//!
//! §12.4: "**One resolution scheme for all persisted state.** Read order is always: exe-adjacent →
//! `%APPDATA%\Tailhawk\` → built-in defaults, **merged**, with the earlier tier winning per key.
//! Write target is the first writable tier, tested by an actual write probe at startup." And
//! "`--stateless` … suppresses all *writes*, and leaves *reads* untouched."
//!
//! ## The file, and why it is read by hand
//!
//! `tailhawk.settings.toml`. There is no TOML crate in the tree and this needs one table, one
//! array of tables, and four value kinds — a reader for that subset is shorter than the argument
//! for a dependency, and it is tested round-trip against its own writer. **It reads leniently**: an
//! unknown key is ignored, a malformed line is skipped, and a file that is not TOML at all yields
//! the defaults. A settings file must never be the reason the viewer does not open.
//!
//! ## What is kept
//!
//! - `[appearance]` — `theme`: `dark`, `light` or `system`, when chosen. Also the preferences that
//!   are not strictly appearance but share its table rather than earning a heading each:
//!   `font`, `font_size`, `toolbar`, `toolbar_large`, and `remote_minutes` — how far back a remote
//!   source fetches, which `File ▸ Open remote ▸ Fetch window` sets and a hand-edited value
//!   overrides with any duration, not only one the menu offers.
//! - `[window]` — `x`, `y`, `width`, `height`, `maximized`: where the window was.
//! - `[[file]]` — `path`, `chips` (each `+text` or `-text`, with a leading `#` when the chip is
//!   switched off), `filters` (`"any"` when the include chips OR together; absent means all),
//!   `collapse`, `bookmarks` (file rows),
//!   `labels` (each `n:text`), `columns` (widths in cells, zero meaning hidden) and `column_order`
//!   (display order before the message): what a file was being
//!   looked at through, so opening it again shows the same view. Keyed by **path**; §12.4 says
//!   file identity, which survives a rename where a path does not, and that upgrade is recorded
//!   rather than done.

use std::path::{Path, PathBuf};

/// The settings file's name, per §12.4.
pub const FILE_NAME: &str = "tailhawk.settings.toml";

/// Where the window was.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Window {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub maximized: bool,
}

/// How one file was being looked at.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FileState {
    /// What this state belongs to: a **file's path**, or, for a remote source, the same
    /// `loki://source?apps=a,b` string the recent list holds it under.
    ///
    /// **A remote window is keyed on its source, not on its file**, because its file is a part of
    /// a spill in `%TEMP%` whose directory is named after the process and gone on the next run.
    /// Keyed on that, the owner's chosen columns were written faithfully every time and found
    /// never — his report of 2026-10-08: *"on the loki tail I selected the app and the severity,
    /// and then on restart, they are no longer showing"*, with his own answer, *"we should really
    /// persist these with the mru data"*. The key is the recent entry's, so the thing the list
    /// reopens and the thing whose layout is remembered are named identically.
    pub path: String,
    /// Chip texts with their polarity as the first character: `+error`, `-retry`.
    pub chips: Vec<String>,
    /// Whether the include chips combine under *any* rather than *all* — written as
    /// `filters = "any"` and absent otherwise, so every file written before the rule existed
    /// reads as it did.
    pub any: bool,
    pub collapse: bool,
    /// E20's bookmarks, as physical (zero-based) file rows.
    pub bookmarks: Vec<u64>,
    /// The colour labels, each `n:text` — the digit key and the literal it marks.
    pub labels: Vec<String>,
    /// Column widths in cells after the user resized them; empty means the measured widths.
    /// **A width of zero is a hidden column**, which is what the chooser writes for an unticked
    /// one.
    pub columns: Vec<u64>,
    /// The display order of the columns before the message, after the user reordered them — a
    /// header drag or the column chooser. Empty means the natural order.
    ///
    /// **Separate from the widths, and written even when the widths are the measured ones.** A
    /// visit to the chooser that only moves a column changed nothing about width, so a state that
    /// carried order inside `columns` would have thrown the move away — which is exactly what
    /// happened to every header drag before this field existed.
    pub column_order: Vec<u64>,
}

/// A remote log source — `LOKI.md`, and the owner's requirement of 2026-09-02 that dev, QA and live
/// can all be tailed at once in separate windows.
///
/// **The secret is not here and must never be.** This is `tailhawk.settings.toml`, which §12.4
/// describes as a file a user opens and edits by hand; a credential in it would be a credential in
/// every backup, screen share and "here is my config" paste. The secret is keyed out to Windows
/// Credential Manager under [`name`](Source::name) — see the shell's `secrets` module — and this
/// carries only what is ordinary configuration.
///
/// **Client credentials is the only flow built**, because it is the only one anything asks for.
/// `token_url`, `client_id` and `scope` are named for the OAuth2 fields rather than for
/// IdentityServer, so a different provider is a different value rather than different code; what a
/// second provider would additionally need — HTTP Basic client authentication, or Entra's
/// `.default` scope convention, or Auth0's `audience` — is a field added when there is one to test
/// against, not now.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Source {
    /// What the user calls it, and the key its secret is stored under. Unique within the file.
    pub name: String,
    /// The base URL, including any mount prefix — `LOKI.md` §9 records that ours is served under
    /// `/loki`, so the endpoint path follows the prefix and the doubled segment is real.
    pub url: String,
    /// The OAuth2 token endpoint. Empty means the source needs no token.
    pub token_url: String,
    /// The OAuth2 **authorization** endpoint. **Set, and the reader signs in as themselves in a
    /// browser; empty, and the source uses the client secret in Credential Manager as it always
    /// has.**
    ///
    /// This is the switch between the two, and it is a URL rather than a flag deliberately: the
    /// endpoint has to be configured for the flow to work at all, so a separate `auth = "browser"`
    /// would be a second thing to set that could disagree with the first.
    ///
    /// **It named the device-authorization endpoint until 2026-10-02.** The device grant was built
    /// and works against this estate's endpoints, but its *verification page* is not part of
    /// Duende and this identity server does not have one — so the reader signed in and the code had
    /// nowhere to be approved. The field was renamed rather than kept beside a second one, because
    /// a configuration file carrying an endpoint nothing can use is a trap for whoever reads it next.
    pub auth_url: String,
    /// The OAuth2 client id.
    pub client_id: String,
    /// The scope to request.
    pub scope: String,
}

/// One named view of one source's logs — a name, and how the source is filtered to make it.
///
/// **A source and a tail are different things, and this product spent two months conflating them.**
/// The owner, 2026-10-21: *"we possibly need to separate the actual remote source with what is
/// being tailed. So we might have a single source, the nurtur loki server, but three separate
/// tails, say one looking at ndc, one at identity server and one at job dispatcher"*. And then the
/// words the screens use: *"They can add a source, and then can create new tail with a name and
/// choose how the source gets filtered to be displayed"*.
///
/// The fields that used to sit on [`Source`] said so themselves. `query`'s doc comment read *"This
/// is what makes one Loki into several sources"* and `apps`' read *"Two sources can share a URL and
/// a selector and differ only here"* — both describing a workaround for a model that had no word
/// for a view.
///
/// **What it fixes is not cosmetic.** A credential is filed under a source's name, so one Loki with
/// three entries meant three credential entries for the same server, the same client and the same
/// person — which is why `Tailhawk/loki-tokens/nurtur-loki-live` sits orphaned in the owner's store
/// after a rename, and why renaming an entry destroyed its sign-in. One source, one credential,
/// however many tails.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Tail {
    /// What the user calls it, and what its tab is titled. Unique within the file.
    pub name: String,
    /// The [`Source::name`] this tail reads from.
    pub source: String,
    /// The LogQL selector this tail starts from — `{environment="dev"}` and the like.
    ///
    /// **Empty is tolerated rather than legitimate**, and the distinction is the difference between
    /// three gates. A file migrated from the old shape can carry one — the old editor let a source
    /// have no selector and drew it as *"(everything)"* — so the set-level gate
    /// [`crate::sourceset::Editor::fault`] has to accept it or such a file could not be closed. But
    /// it cannot be *opened*: it reaches Loki as an empty `query` field and comes back 400. So the
    /// row says so ([`crate::sourceset::tail_row_fault`]) and the editor refuses to make another
    /// ([`crate::sourceset::tail_being_edited`]).
    ///
    /// This said "so it is never a fault" until a review pointed out that one of the three
    /// disagreed with it.
    pub query: String,
    /// The applications this tail is for, remembered from the last time they were chosen.
    ///
    /// **Empty means "ask me".** Set, and the tail opens straight into these without a picker — the
    /// owner's ask of 2026-10-02: *"it would be good to remember which apps were selected the
    /// previous time so that a user doesnt have to reselect a particular set of apps"*.
    ///
    /// Kept *beside* the query rather than narrowed into it, so the tail still follows the selector
    /// if that is edited later — [`apps::with_apps`](crate::apps::with_apps) narrows at the moment
    /// of opening.
    pub apps: Vec<String>,
    /// The attribute keys the reader chose as grid columns, in the order chosen — the Starberry
    /// request of 2026-10-27: *"persist the chosen columns per tail … next to the tail's selector."*
    ///
    /// **Empty means "the defaults"** — [`chosen::default_columns`](crate::chosen::default_columns)
    /// over whatever the window carries — and is what a tail nobody has customised writes, so a
    /// later change to those defaults still reaches it. Names rather than widths by position,
    /// because the same tail's windows differ by application set and a name survives a reorder;
    /// the widths stay on the window's own `[[file]]` entry as before.
    pub columns: Vec<String>,
}

/// One `[[source]]` section as every settings file written before 2026-10-21 holds it: a
/// connection and a view in the same entry.
///
/// **It exists only to be read and split.** Nothing writes this shape any more, and the only thing
/// that constructs one is [`Settings::from_toml`] meeting a `[[source]]` section.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LegacySource {
    /// Everything about the connection, which is what a [`Source`] still is.
    pub source: Source,
    /// The selector, which is now a [`Tail`]'s.
    pub query: String,
    /// The applications, likewise.
    pub apps: Vec<String>,
}

/// Splits the old one-entry-per-view shape into the sources and tails it was really describing.
///
/// **Entries that agree about everything but the view are one source.** The connection is the URL,
/// the two endpoints, the client and the scope; `query` and `apps` are the view. Three entries over
/// one Loki differing only in those two were always one source and three tails, and the file said
/// so by repeating the other five fields three times.
///
/// **The source is named after its host**, because the old names belong to the views and the views
/// keep them: `nurtur-loki-jobdispatcher` stays the name of a *tail*, and the source it reads from
/// is `telemetry.nurtur.tech`. A host is predictable, stable, and already what a reader would call
/// the thing. Characters a credential key may not hold are replaced, so a port or an address
/// literal cannot produce a name the store refuses.
///
/// **The credential has to follow, and this says where from.** A migrated source is a *new name*,
/// and the store is keyed on the name — so the secret and the refresh token sitting under
/// `nurtur-loki-live` are invisible to a source now called `telemetry.nurtur.tech`, and the reader
/// is silently signed out of a server they signed into. That is the orphaned-credential defect this
/// product already has one of, reproduced by its own upgrade. The third return is therefore every
/// new source's old names, for the shell to move the credential across; see [`Renamed`], and
/// `secrets::follow_rename` in the shell, which is the half that may touch the store.
///
/// An earlier version of this comment claimed *"nothing is moved in Credential Manager — the shell
/// looks there as well as under its own, see the `secrets_for` fallback"*. There was no
/// `secrets_for`, and nothing looked anywhere but the source's own name.
pub fn split_sources(entries: &[LegacySource]) -> (Vec<Source>, Vec<Tail>, Vec<Renamed>) {
    let mut servers: Vec<Source> = Vec::new();
    let mut tails: Vec<Tail> = Vec::new();
    let mut renamed: Vec<Renamed> = Vec::new();
    // **Compared with the URLs normalised, because a trailing slash is not a different server.**
    // `https://telemetry.nurtur.tech/loki` and `…/loki/` are the same Loki typed two ways, and a
    // raw string comparison made them two sources — which is two credentials for one server, the
    // exact duplication this whole split exists to end, reintroduced by the thing meant to end it.
    let settled = |source: &Source| Source {
        name: String::new(),
        url: source.url.trim().trim_end_matches('/').to_owned(),
        token_url: source.token_url.trim().to_owned(),
        auth_url: source.auth_url.trim().to_owned(),
        client_id: source.client_id.trim().to_owned(),
        scope: source.scope.trim().to_owned(),
    };
    for entry in entries {
        let source = &entry.source;
        let connection = settled(source);
        let at = servers.iter().position(|s| settled(s) == connection);
        let name = match at {
            Some(at) => servers[at].name.clone(),
            None => {
                let wanted = host_name(&source.url);
                let taken: Vec<String> = servers.iter().map(|s| s.name.clone()).collect();
                let name = crate::apps::saved_entry_name(&wanted, &[], &taken);
                servers.push(Source {
                    name: name.clone(),
                    ..connection
                });
                renamed.push(Renamed {
                    now: name.clone(),
                    was: Vec::new(),
                });
                name
            }
        };
        // **Every entry that folded into this source is a name its credential may be under**, in
        // file order, because the shell has no better way to choose between them than the one the
        // reader wrote.
        if let Some(entry_of) = renamed.iter_mut().find(|r| r.now == name) {
            entry_of.was.push(source.name.clone());
        }
        tails.push(Tail {
            name: source.name.clone(),
            source: name,
            query: entry.query.clone(),
            apps: entry.apps.clone(),
            columns: Vec::new(),
        });
    }
    // A source whose new name is one of its own old ones needs nothing moved.
    renamed.retain(|r| !r.was.contains(&r.now));
    (servers, tails, renamed)
}

/// A source that a migration renamed, and the names it used to be known by.
///
/// **It is not written to the settings file and it does not survive a save.** It is the one-time
/// instruction *"the credential for this server is filed under one of these"*, produced by
/// [`split_sources`] when it reads an old file and consumed by the shell before the new file is
/// written. Once the credential has moved there is nothing left to say.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Renamed {
    /// What the source is called now.
    pub now: String,
    /// What the entries that folded into it were called, in file order.
    pub was: Vec<String>,
}

/// Which old name a credential should be moved from, or `None` if nothing should move.
///
/// **Pure, so the decision is tested and the store is only ever touched on its answer.** `now` is
/// whether the new name already holds one; `was` is each old name and whether it holds one, in the
/// order [`Renamed`] gives.
///
/// - A credential already under the new name wins: a reader who has signed in since the upgrade
///   must not be rolled back to a token from before it.
/// - Otherwise the first old name that has one. Several is the three-entries-over-one-Loki case,
///   where they are tokens for the same server, the same client and the same person.
pub fn secret_to_move(now: bool, was: &[(String, bool)]) -> Option<&str> {
    if now {
        return None;
    }
    was.iter()
        .find(|(_, has)| *has)
        .map(|(name, _)| name.as_str())
}

/// One credential to carry from an old name to a new one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CredentialMove {
    /// The name it is filed under now.
    pub from: String,
    /// The name it belongs under.
    pub to: String,
}

/// Every credential a migration should carry, and the old names to clear afterwards.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CredentialPlan {
    pub moves: Vec<CredentialMove>,
    /// Names to delete once every move has landed.
    pub clear: Vec<String>,
}

/// Plans the whole credential migration **before any of it happens**, which is the only way it is
/// correct.
///
/// `held` is every name the store has something under, read once, before anything moves.
///
/// **Deciding one entry at a time against the live store is wrong, and quietly.** Two things make
/// it so, and both need a legacy entry *named after some other entry's host* — which is the naming
/// convention this very version writes, so it is a shape upgraders will have.
///
/// - **A new name that is also another entry's old name is not "already occupied".** Asking the
///   live store whether `telemetry.nurtur.tech` holds a credential answers yes when that is the
///   name of a *different* legacy source whose own credential is about to move away. Treating that
///   as occupied skips the move, and the secret that belonged there is left orphaned for ever.
/// - **And a move must not land on a name something is still due to be taken from.** Having
///   decided not to skip, writing immediately clobbers the credential the other entry has not read
///   yet — so that entry then carries the *wrong* secret to its own new name. Reading every value
///   first is what makes the order stop mattering.
///
/// The clear list follows from the same fact: a name that is some entry's `to` is never cleared,
/// however many entries took from it, because clearing it would delete what was just written
/// there.
pub fn credential_plan(renamed: &[Renamed], held: &[String]) -> CredentialPlan {
    let has = |name: &str| held.iter().any(|h| h == name);
    let landing: Vec<&str> = renamed.iter().map(|r| r.now.as_str()).collect();
    let leaving: Vec<&str> = renamed
        .iter()
        .flat_map(|r| r.was.iter())
        .map(String::as_str)
        .collect();

    let mut plan = CredentialPlan::default();
    for entry in renamed {
        // Occupied means "holds a credential that is staying there" — so a name something is due
        // to be taken from does not count, however full it looks.
        let occupied = has(&entry.now) && !leaving.contains(&entry.now.as_str());
        let flags: Vec<(String, bool)> = entry
            .was
            .iter()
            .map(|was| (was.clone(), has(was)))
            .collect();
        // **Never onto the name it came from.** `split_sources` drops an entry whose new name is
        // one of its own old ones, so a migration cannot ask for this — but the editor's renames
        // come through here too, and a copy-then-clear of one name onto itself would delete the
        // credential it had just written.
        if let Some(from) = secret_to_move(occupied, &flags).filter(|from| *from != entry.now) {
            plan.moves.push(CredentialMove {
                from: from.to_owned(),
                to: entry.now.clone(),
            });
        }
    }
    for mv in &plan.moves {
        if !landing.contains(&mv.from.as_str()) && !plan.clear.contains(&mv.from) {
            plan.clear.push(mv.from.clone());
        }
    }
    plan
}

/// The longest a source's name may be, because it is also the key its credential is filed under.
///
/// **It lives here rather than in the shell's `secrets` module**, where the same number used to be
/// the only copy. `Source::fault` claimed the store "enforces the same shape" and did not check the
/// length at all, so a 65-character name saved cleanly and then failed silently at every store
/// call: the pasted secret was never written, the row read `(no secret stored)` for ever, and
/// nothing said why. A limit one half of the product enforces and the other half only mentions is
/// not a shared rule, so this is the one definition and `secrets::nameable` reads it.
///
/// `CREDENTIALW::TargetName` is documented as up to 32,767 characters, so this is not the
/// platform's limit — it is a limit on how silly a name can get before it stops being a name.
pub const NAME_MAX: usize = 64;

/// A URL's host, as a name a source may be called and a credential may be filed under.
///
/// Anything outside letters, digits, `-`, `_`, `.` and space becomes `-`, so a port or a bracketed
/// address literal cannot produce a name [`Source::fault`] refuses and the store would not accept.
fn host_name(url: &str) -> String {
    let after = url.split_once("://").map(|(_, rest)| rest).unwrap_or(url);
    let host = after.split('/').next().unwrap_or(after);
    let cleaned: String = host
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ' ') {
                c
            } else {
                '-'
            }
        })
        .collect();
    let trimmed = cleaned.trim_matches(|c| c == '-' || c == ' ').to_owned();
    if trimmed.is_empty() {
        "loki".to_owned()
    } else {
        trimmed
    }
}

impl Source {
    /// Why this source cannot be used yet, in the words the dialog shows — or `None` when it can.
    ///
    /// **A decision rather than a screenful of disabled buttons.** The dialog asks this and shows
    /// the answer beside the fields, so a source that will not work says why while it is being
    /// typed rather than when a query fails.
    pub fn fault(&self) -> Option<&'static str> {
        // The name is the credential's key as well as the label, so it is held to what can be one:
        // no separator, no control characters, nothing that trims away to nothing, and nothing
        // longer than [`NAME_MAX`]. The shell's `secrets::nameable` holds a name to exactly this,
        // reading the same constant; this is the half that can say so in words while the user is
        // still typing.
        if self.name.trim().is_empty() {
            return Some("Give the source a name.");
        }
        if self.name.trim() != self.name {
            return Some("The name cannot start or end with a space.");
        }
        // **The length was the one rule only the store knew.** A longer name saved and then failed
        // silently at every store call — no secret written, no sign-in kept, nothing said.
        if self.name.len() > NAME_MAX {
            return Some("The name is too long to file a credential under.");
        }
        let nameable = |c: char| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ' ');
        if !self.name.chars().all(nameable) {
            return Some("The name can hold letters, digits, spaces, - _ and . only.");
        }
        if self.url.trim().is_empty() {
            return Some("Give the source a URL.");
        }
        if !is_https(&self.url) {
            return Some("The URL must begin with https://.");
        }
        // **Half-configured authentication is the case worth catching.** It looks finished, and it
        // fails at the first query with whatever the server says about a missing token — which
        // sends the reader looking at the server rather than at this dialog.
        match (
            self.token_url.trim().is_empty(),
            self.client_id.trim().is_empty(),
        ) {
            (true, true) => {}
            (false, false) => {
                if !is_https(&self.token_url) {
                    return Some("The token URL must begin with https://.");
                }
            }
            (true, false) => return Some("A client id needs a token URL to send it to."),
            (false, true) => return Some("A token URL needs a client id."),
        }
        // **Signing in needs all three, and saying so beats a poll that 404s.** The device
        // endpoint starts the sign-in and the token endpoint finishes it, so a source with one and
        // not the other is configured for neither flow.
        if !self.auth_url.trim().is_empty() {
            if !is_https(&self.auth_url) {
                return Some("The sign-in URL must begin with https://.");
            }
            if self.token_url.trim().is_empty() || self.client_id.trim().is_empty() {
                return Some("Signing in needs a token URL and a client id as well.");
            }
        }
        None
    }

    /// Whether this source signs the reader in, rather than presenting a stored client secret.
    ///
    /// **The one place that decision is made.** Everything downstream — which credential to load,
    /// which request to build, whether to offer a Sign out — asks this rather than re-deriving it
    /// from an empty string, because two places deriving it is how they come to disagree.
    pub fn signs_in(&self) -> bool {
        !self.auth_url.trim().is_empty()
    }
}

/// The authorize endpoint that conventionally sits beside a token endpoint.
///
/// **This exists because a colleague could not sign in at all.** The sources dialog had no
/// sign-in-URL field until 2026-10-13, so every source anyone created through the app had an empty
/// `auth_url`, [`Source::signs_in`] answered `false` for all of them, and the whole
/// authorization-code path was unreachable — the reader was told to paste a client secret instead.
/// Adding the field fixes that; this function is what stops the field being a question most people
/// cannot answer, because the token URL they already have usually names the authorize URL too.
///
/// **A suggestion, never a correction.** The caller offers it only into an empty field, and the
/// reader can overwrite it. That matters because the convention is not universal: this replaces a
/// final `token` segment with `authorize`, which is right for IdentityServer's
/// `/connect/authorize` and Entra's `/oauth2/v2.0/authorize`, and **wrong for Keycloak**, whose
/// authorize endpoint is `/protocol/openid-connect/auth`. A visible, editable wrong guess in a
/// field the reader is already looking at costs them one correction; silence costs them the
/// feature, which is what it has cost until now.
pub fn authorize_beside(token_url: &str) -> Option<String> {
    let trimmed = token_url.trim();
    if !is_https(trimmed) {
        return None;
    }
    let stem = trimmed.strip_suffix('/').unwrap_or(trimmed);
    let (before, last) = stem.rsplit_once('/')?;
    // **`is_https(before)`, not `!before.is_empty()`, and the difference is a URL this invented.**
    // `https://token` has no path at all, so `rsplit_once` splits it on the *second slash of the
    // scheme* — handing back `https:/` and `token`, and suggesting `https:/authorize`. The dialog
    // would then have written a malformed URL into a box the reader never touched and refused to
    // close over it. Requiring what is left to still be an https URL with a host is the same test
    // the whole string already had to pass.
    if !last.eq_ignore_ascii_case("token") || !is_https(before) {
        return None;
    }
    Some(format!("{before}/authorize"))
}

/// Whether `url` is an absolute https URL with a host.
///
/// **`http` is refused rather than warned about.** A bearer token on a cleartext connection is a
/// bearer token given away, and `LOKI.md` §7 asks for transport that cannot be downgraded — a rule
/// worth nothing if the URL that carries the credential can simply say `http`.
pub(crate) fn is_https(url: &str) -> bool {
    url.strip_prefix("https://")
        .is_some_and(|rest| !rest.is_empty() && !rest.starts_with('/'))
}

/// Everything persisted.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Settings {
    pub window: Option<Window>,
    pub files: Vec<FileState>,
    /// `LOKI.md`'s remote sources, in the order the user put them. Secrets are NOT here.
    pub sources: Vec<Source>,
    /// The tails — the named views, each reading one of the sources above.
    ///
    /// **A source is a connection and a tail is a view**, since 2026-10-21. Before that one
    /// `[[source]]` was both, which is why a file written by an older build migrates on read: see
    /// [`split_sources`].
    pub tails: Vec<Tail>,
    /// V13: `dark`, `light` or `system`, when the user chose one.
    pub theme: Option<String>,
    /// §2.2 Preferences: the grid font family, when the user chose one.
    pub font: Option<String>,
    /// §2.2 Preferences: the grid em size at the 96-DPI baseline, when the user chose one.
    pub font_size: Option<u16>,
    /// §2.3: whether the toolbar row is shown. Absent means shown — the default is the row.
    pub toolbar: Option<bool>,
    /// How far back a remote source fetches, in minutes. Absent means [`DEFAULT_REMOTE_MINUTES`].
    ///
    /// **This was a hard-coded hour with no way to change it**, while the status bar told anyone
    /// whose answer came back cut to *"narrow the time range or the selector"* — naming two
    /// remedies when only the selector existed. Reported 2026-10-05: *"there is no indication of
    /// how they might do that"*, and the honest reading is that for the time range there was no
    /// how. It is remembered rather than per-run because a reader who needs four hours needs four
    /// hours again tomorrow.
    pub remote_minutes: Option<u32>,
    /// §2.3: whether the toolbar draws its icons large. Absent means small, which is what every
    /// shell toolbar starts at. The owner's ask of 2026-09-08: "there should probably be an option
    /// for large and small toolbars, that is a common option of standard windows apps".
    pub toolbar_large: Option<bool>,
    /// File ▸ Open Recent, newest first, at most [`RECENT_MAX`].
    pub recent: Vec<Recent>,
    /// The Find dialog's query history, newest first, at most [`FIND_MAX`].
    pub find_queries: Vec<String>,
    /// Which shape the file said it was in, or `None` for a file that predates [`MODEL`] and is
    /// therefore migrated. Not a setting a reader would ever want to change.
    pub model: Option<u32>,
    /// **Not a setting: the one-time note that reading this file migrated it.** Each entry is a
    /// source that [`split_sources`] gave a new name, and the names its credential may be filed
    /// under. The shell acts on it once, before saving, and nothing writes it to a file — see
    /// [`Renamed`] and [`secret_to_move`].
    pub migrated: Vec<Renamed>,
}

/// One entry in File ▸ Open Recent: a file on disk, or a remote source and what was chosen from it.
///
/// **A remote source is as worth reopening as a file, and until 2026-09-21 only files were kept.**
/// The owner: "I would also like the recently used files to allow supporting the recent remote
/// opens." A remote open is not a path — it is a configured source plus the applications picked
/// from it — so the list holds either, in one sequence, because recency runs across both.
///
/// **Written as `loki://source?apps=a,b`, and that scheme is chosen so it cannot be a path.** A
/// Windows drive is one letter, so no file has ever begun `loki://`; anything that does is a
/// remote entry and anything that does not is a file, with no flag to fall out of step with the
/// value beside it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Recent {
    File(String),
    Remote { tail: String, apps: Vec<String> },
}

/// The prefix that marks a recent entry as a remote source rather than a path.
const REMOTE_SCHEME: &str = "loki://";

/// Which shape the settings file is in — written at the top of every file this version produces.
///
/// **It exists because the two shapes are byte-identical in one case**, and that case is ordinary.
/// Before 2026-10-21 a `[[source]]` was a connection *and* a view; the splitter decides a file is
/// old by finding a `[[source]]` that carries a `query` or an `apps`. But the old writer emitted
/// `query` only when it was non-empty and the old editor let it be empty — it drew the empty one as
/// *"(everything)"* — so a source configured to show everything wrote neither key, and a file full
/// of those is indistinguishable from a new file with sources and no tails.
///
/// Guessing it cost the view. Read as new, the file keeps its sources and has no tails at all, and
/// `File ▸ Open remote` — which lists tails — comes up **empty**: the server is in the
/// configuration and unreachable from the menu. Read as old, a new file whose tails the reader had
/// deliberately deleted would grow them back on every load.
///
/// So the file says. A file carrying no `model` key is from before this and is migrated; one
/// carrying this is read as it stands.
pub const MODEL: u32 = 2;

/// How far back a remote source fetches when nothing has been chosen — one hour, which is what the
/// fixed constant was before there was a choice.
pub const DEFAULT_REMOTE_MINUTES: u32 = 60;

/// The fetch windows offered on the menu, longest label first in reading order.
///
/// **Data rather than a menu, so the choice and the fetch cannot disagree.** The menu is built from
/// this and so is the check mark, and [`remote_minutes_of`] resolves whatever is stored against it —
/// including a hand-edited settings file naming a duration that is not on the list, which is kept
/// rather than silently rounded, because §12.4's file is explicitly something a user edits.
pub const FETCH_WINDOWS: [(&str, u32); 5] = [
    ("Last &15 minutes", 15),
    ("Last &hour", 60),
    ("Last &4 hours", 240),
    ("Last &12 hours", 720),
    ("Last &24 hours", 1440),
];

/// The fetch window in minutes: what was stored, or the default when nothing was.
pub fn remote_minutes_of(stored: Option<u32>) -> u32 {
    stored.filter(|m| *m > 0).unwrap_or(DEFAULT_REMOTE_MINUTES)
}

/// How a fetch window reads in a sentence — "the last 4 hours" — for the status bar and the notice.
///
/// Whole hours read as hours and anything else as minutes, because "the last 240 minutes" is a
/// number a reader has to divide before it means anything.
pub fn fetch_window_said(minutes: u32) -> String {
    match (minutes, minutes % 60) {
        (0, _) => "the last hour".to_owned(),
        (1, _) => "the last minute".to_owned(),
        (60, _) => "the last hour".to_owned(),
        (m, 0) => format!("the last {} hours", m / 60),
        (m, _) => format!("the last {m} minutes"),
    }
}

/// The name Tailhawk gives everything it spills to `%TEMP%` — the single file a piped stream goes
/// to, and the directory of parts a remote tail fills. [`Recent::is_spill`] recognises either from
/// a path alone, and `stdin.rs` builds both from this, so the two cannot drift apart.
///
/// **It lives here, on the portable side, although `stdin.rs` is what does the writing.** `stdin` is
/// `#[cfg(windows)]` and this module is not, so a constant owned by `stdin` and read from here stops
/// `tailhawk-core` compiling for Linux — which is `SPEC.md` §13's portability assertion and a CI job
/// (`portable-core`) rather than a hypothetical. The *name* is portable knowledge; creating the file
/// is the platform's work.
pub(crate) const SPILL_PREFIX: &str = "tailhawk-spill-";

/// Whether this path is one of Tailhawk's own spill files in `%TEMP%` — a piped stream's single
/// file, or one part of a remote tail's rolling set.
///
/// Either the file or the directory above it may carry the name, because the two shapes differ:
/// a pipe spills to `%TEMP%\tailhawk-spill-1234.log` and a remote tail to
/// `%TEMP%\tailhawk-spill-1234-00\part-000001.log`.
pub fn is_spill_path(path: &str) -> bool {
    let named_spill = |part: &str| {
        part.get(..SPILL_PREFIX.len())
            .is_some_and(|head| head.eq_ignore_ascii_case(SPILL_PREFIX))
    };
    path.rsplit(['\\', '/']).take(2).any(named_spill)
}

/// How a remembered-state key is compared: **a path folds case, a source does not.**
///
/// The same rule [`identity_of`] applies to the recent list, and for the same reasons — Windows
/// opens `C:\LOGS\A.LOG` and `C:\logs\a.log` as one file, while a Loki source name is the key its
/// credential is stored under and an `app` value is a server-side string compared exactly. Folding
/// a remote key would quietly merge a reader's `Worker` window with their `worker` one.
///
/// **One function so the two cannot drift.** The state keyed per file and the identity keyed per
/// recent entry are now the same string for a remote source — that is the whole point of keying
/// state on the entry — so a different answer here than there would mean a window whose layout is
/// remembered under a name the recent list cannot produce.
pub fn fold_key(key: &str) -> String {
    if key.starts_with(REMOTE_SCHEME) {
        key.to_owned()
    } else {
        key.to_lowercase()
    }
}

/// What makes two recent entries the same entry rather than two.
///
/// **A path folds case; a source does not, and folding one would lose a window.** Windows opens
/// `C:\LOGS\A.LOG` and `C:\logs\a.log` as one file, so the list must too. A Loki `app` value is a
/// server-side string compared exactly — nothing in [`crate::apps`] folds it — and a source name
/// is the key its credential is stored under, which `Shell::reopen_remote` matches exactly. So a
/// reader who has `live` open on `Worker` and again on `worker` has two real windows, and folding
/// the encoded form would quietly replace one with the other.
///
/// A path cannot begin `loki://`, so a file and a source can never collide here.
fn identity_of(entry: &Recent) -> String {
    fold_key(&entry.encode())
}

/// A source or application name with the two separators — and the escape itself — made safe.
///
/// **`%` goes first, and the order is the whole of why this is a function.** Unescaping walks the
/// text once, so a `%` written after the separators had been encoded would be read as the start of
/// an escape the name never contained.
fn escape(text: &str) -> String {
    text.replace('%', "%25")
        .replace('?', "%3F")
        .replace(',', "%2C")
}

/// The inverse, and it undoes `%` last for the reason [`escape`] does it first.
fn unescape(text: &str) -> String {
    text.replace("%3F", "?")
        .replace("%2C", ",")
        .replace("%25", "%")
}

impl Recent {
    /// Whether this entry is a spill — a file Tailhawk itself wrote under `%TEMP%` to hold a
    /// stream's lines, rather than a file anybody opened.
    ///
    /// **`stdin.rs` writes two shapes and both have to be caught.** A piped stream spills to a
    /// single file, `tailhawk-spill-<pid>-<n>.log`; a remote tail spills into a *directory* of
    /// parts, `tailhawk-spill-<pid>-<n>\part-NNNNNN.log`. So the last component is tested and so is
    /// its parent — the parent because `part-000001.log` alone is a name anyone's log rotation might
    /// use, and the file because the piped form has no spill directory above it.
    ///
    /// Both carry [`SPILL_PREFIX`], which is the one constant the naming comes from, so neither
    /// shape can drift out of recognition without the other going with it.
    ///
    /// **Only that prefix, never "looks temporary".** Somebody tailing a log their own tool wrote to
    /// `%TEMP%` is doing something ordinary and keeps their entry. The comparison folds case, as
    /// [`identity_of`] does for paths, because a path that has been through a short name or a
    /// hand-edited settings file is still the same file to Windows.
    ///
    /// A remote entry is never a spill: it names the source, which is the thing worth reopening.
    pub fn is_spill(&self) -> bool {
        match self {
            Recent::File(path) => is_spill_path(path),
            Recent::Remote { .. } => false,
        }
    }

    /// The entry as a reader should see it on the welcome surface.
    ///
    /// **The owner's report of 2026-10-05**: the list showed *"the temp directory with generated
    /// file names"* and he was right that those were remote tails — a Loki window's document is a
    /// spill part like `tailhawk-spill-52140-00\part-000001.log`, which names nothing anybody would
    /// recognise. A remote entry reads as the source and the applications it was opened for, which
    /// is what he asked to see instead.
    ///
    /// **A file keeps its path**, because for a file the path *is* the recognisable thing — but only
    /// its last two components, since a recent list is for recognising rather than for auditing and
    /// a full path pushes the name off the end of the line.
    pub fn shown(&self) -> String {
        match self {
            Recent::File(path) => {
                let parts: Vec<&str> = path.rsplit(['\\', '/']).take(2).collect();
                match parts.len() {
                    0 => path.clone(),
                    1 => parts[0].to_owned(),
                    _ => format!("{}\\{}", parts[1], parts[0]),
                }
            }
            Recent::Remote { tail, apps } if apps.is_empty() => format!("{tail}  (Loki)"),
            Recent::Remote { tail, apps } => {
                let named: Vec<&str> = apps.iter().map(String::as_str).collect();
                format!("{}  (Loki)", crate::apps::source_label(tail, &named))
            }
        }
    }

    /// The entry as one line of the settings file.
    pub fn encode(&self) -> String {
        match self {
            Recent::File(path) => path.clone(),
            Recent::Remote { tail, apps } if apps.is_empty() => {
                format!("{REMOTE_SCHEME}{}", escape(tail))
            }
            Recent::Remote { tail, apps } => {
                let apps: Vec<String> = apps.iter().map(|a| escape(a)).collect();
                format!("{REMOTE_SCHEME}{}?apps={}", escape(tail), apps.join(","))
            }
        }
    }

    /// The entry back from that line. Anything not carrying the scheme is a path, which is what
    /// makes every settings file written before this reads correctly.
    pub fn decode(text: &str) -> Recent {
        let Some(rest) = text.strip_prefix(REMOTE_SCHEME) else {
            return Recent::File(text.to_owned());
        };
        let (name, apps) = match rest.split_once("?apps=") {
            // **Not filtered for empties.** `encode` writes `?apps=` only when there is at least
            // one, so the marker's presence already says the list is non-empty — and dropping an
            // empty name here would make `decode(encode(x))` differ from `x` for the one a
            // hand-written `{app=""}` selector produces.
            Some((name, apps)) => (name, apps.split(',').map(unescape).collect()),
            None => (rest, Vec::new()),
        };
        Recent::Remote {
            tail: unescape(name),
            apps,
        }
    }

    /// The path, when this is a file — for the menu's compaction and for opening it.
    pub fn path(&self) -> Option<&str> {
        match self {
            Recent::File(path) => Some(path),
            Recent::Remote { .. } => None,
        }
    }
}

/// How many files Open Recent keeps — the platform's customary MRU depth.
pub const RECENT_MAX: usize = 10;
/// How many queries the Find dialog's history holds.
pub const FIND_MAX: usize = 10;

impl Settings {
    /// The state for `key`, if any was kept. A file's key is its path; a remote source's is the
    /// same string the recent list holds it under — see [`FileState::path`].
    pub fn file(&self, key: &str) -> Option<&FileState> {
        let wanted = fold_key(key);
        self.files.iter().find(|f| fold_key(&f.path) == wanted)
    }

    /// Records the state for one window, replacing what was there. A window with nothing to say —
    /// no chips, no collapse — is forgotten rather than kept as an empty entry.
    ///
    /// **A spill path is never recorded.** Until 2026-10-08 a remote tail's state was keyed on the
    /// part file it happened to be reading, in a `%TEMP%` directory named after the process; it
    /// could never be read back, and every remote open left one more entry nothing would ever
    /// match. Those are dropped on load; this is what stops new ones.
    pub fn set_file(&mut self, state: FileState) {
        if is_spill_path(&state.path) {
            return;
        }
        let key = fold_key(&state.path);
        self.files.retain(|f| fold_key(&f.path) != key);
        if !state.chips.is_empty()
            || state.any
            || state.collapse
            || !state.bookmarks.is_empty()
            || !state.labels.is_empty()
            || !state.columns.is_empty()
            // A reordered layout is worth remembering even when every width is the measured one:
            // that is precisely the state a visit to the column chooser leaves behind.
            || !state.column_order.is_empty()
        {
            self.files.push(state);
        }
        // Keep the newest 200: a settings file is not a history.
        if self.files.len() > 200 {
            let drop = self.files.len() - 200;
            self.files.drain(..drop);
        }
    }

    /// Puts `query` at the front of the Find history, moving it there if it is already present —
    /// compared exactly, because case matters in a query — and dropping the oldest past
    /// [`FIND_MAX`].
    pub fn remember_query(&mut self, query: &str) {
        self.find_queries.retain(|q| q != query);
        self.find_queries.insert(0, query.to_owned());
        self.find_queries.truncate(FIND_MAX);
    }

    /// Puts `path` at the front of the recent list, moving it there if it is already present —
    /// compared case-insensitively, as Windows paths are — and dropping the oldest past
    /// [`RECENT_MAX`].
    pub fn remember_recent(&mut self, entry: Recent) {
        let key = identity_of(&entry);
        self.recent.retain(|e| identity_of(e) != key);
        self.recent.insert(0, entry);
        self.recent.truncate(RECENT_MAX);
    }

    /// Drops one entry from the recent list, reporting whether it was there.
    ///
    /// **For an entry that can never work again**, which is the only honest reason to take a row
    /// off a list the reader did not ask to edit: a remote entry naming a source that is no longer
    /// in the configuration. Reported 2026-10-05 — it said so in the status bar and then offered
    /// the same unopenable row every time after. A row that cannot be acted on is worse than no
    /// row, because the reader keeps spending a click to find out.
    ///
    /// Not for an open that merely failed. A server that was down, a token that had expired, a file
    /// on a disconnected share are all reasons to try again later, and forgetting those would make
    /// the list lose exactly the entry the reader most wants back.
    pub fn forget_recent(&mut self, entry: &Recent) -> bool {
        let key = identity_of(entry);
        let before = self.recent.len();
        self.recent.retain(|e| identity_of(e) != key);
        self.recent.len() != before
    }

    /// Merges `over` onto `self`: `over`'s keys win, per §12.4's "earlier tier winning per key",
    /// with the earlier tier passed as `over`.
    pub fn merged_under(mut self, over: Settings) -> Settings {
        if over.window.is_some() {
            self.window = over.window;
        }
        if over.remote_minutes.is_some() {
            self.remote_minutes = over.remote_minutes;
        }
        if over.toolbar.is_some() {
            self.toolbar = over.toolbar;
        }
        if over.toolbar_large.is_some() {
            self.toolbar_large = over.toolbar_large;
        }
        // Sources merge **by name**, so a curated set beside the exe can define the shared ones and
        // a user can still add their own — the same shape §12.4 gives every other tiered artefact.
        for s in over.sources {
            self.sources.retain(|e| e.name != s.name);
            self.sources.push(s);
        }
        // **And the tails the same way, which they were not.** A tail is a named artefact of the
        // same kind as a source, so a curated tail beside the exe and a user's own with the same
        // name is the overriding case `sources` has always handled; two tails sharing a name is a
        // fault the editor refuses, and a merge that appended would have manufactured one.
        for t in over.tails {
            self.tails.retain(|e| e.name != t.name);
            self.tails.push(t);
        }
        // **Every tier's migration note is kept**, because each tier is a file of its own and any
        // of them can be in the old shape. They are instructions to move a credential once, not
        // settings, so the later tier does not replace the earlier one's.
        self.migrated.extend(over.migrated);
        if over.theme.is_some() {
            self.theme = over.theme;
        }
        if !over.recent.is_empty() {
            self.recent = over.recent;
            self.recent.truncate(RECENT_MAX);
        }
        if !over.find_queries.is_empty() {
            self.find_queries = over.find_queries;
            self.find_queries.truncate(FIND_MAX);
        }
        for f in over.files {
            self.set_file(f);
        }
        self
    }

    /// The file's text.
    pub fn to_toml(&self) -> String {
        let mut out =
            String::from("# Tailhawk settings — SPEC.md §12.4. Rewritten whole; edits survive.\n");
        // **First, and before any table.** Its absence is what tells the reader this file predates
        // the source/tail split — see [`MODEL`], which is the whole argument. A bare key must come
        // before the first `[table]` heading or TOML puts it inside that table.
        out.push_str(&format!("model = {MODEL}\n"));
        // One `[appearance]` heading however many of its keys are set — a second heading for the
        // same table is legal TOML but reads as a mistake to anyone editing the file by hand, which
        // §12.4 expects them to do.
        if self.theme.is_some()
            || self.font.is_some()
            || self.font_size.is_some()
            || self.remote_minutes.is_some()
            || self.toolbar.is_some()
            || self.toolbar_large.is_some()
        {
            out.push_str("\n[appearance]\n");
            if let Some(theme) = &self.theme {
                out.push_str(&format!("theme = {}\n", quote(theme)));
            }
            if let Some(font) = &self.font {
                out.push_str(&format!("font = {}\n", quote(font)));
            }
            if let Some(size) = self.font_size {
                out.push_str(&format!("font_size = {size}\n"));
            }
            if let Some(mins) = self.remote_minutes {
                out.push_str(&format!("remote_minutes = {mins}\n"));
            }
            if let Some(shown) = self.toolbar {
                out.push_str(&format!("toolbar = {shown}\n"));
            }
            if let Some(large) = self.toolbar_large {
                out.push_str(&format!("toolbar_large = {large}\n"));
            }
        }
        if !self.recent.is_empty() {
            let files: Vec<String> = self.recent.iter().map(|e| quote(&e.encode())).collect();
            out.push_str(&format!("\n[recent]\nfiles = [{}]\n", files.join(", ")));
        }
        if !self.find_queries.is_empty() {
            let queries: Vec<String> = self.find_queries.iter().map(|q| quote(q)).collect();
            out.push_str(&format!("\n[find]\nqueries = [{}]\n", queries.join(", ")));
        }
        if let Some(w) = &self.window {
            out.push_str("\n[window]\n");
            out.push_str(&format!(
                "x = {}\ny = {}\nwidth = {}\nheight = {}\nmaximized = {}\n",
                w.x, w.y, w.width, w.height, w.maximized
            ));
        }
        for s in &self.sources {
            out.push_str("\n[[source]]\n");
            out.push_str(&format!("name = {}\n", quote(&s.name)));
            out.push_str(&format!("url = {}\n", quote(&s.url)));
            // Written only when set, so an unauthenticated source reads as one rather than as three
            // empty strings a reader has to interpret. **No secret is written, ever** — it lives in
            // Credential Manager under `name`.
            if !s.token_url.is_empty() {
                out.push_str(&format!("token_url = {}\n", quote(&s.token_url)));
            }
            if !s.auth_url.is_empty() {
                out.push_str(&format!("auth_url = {}\n", quote(&s.auth_url)));
            }
            if !s.client_id.is_empty() {
                out.push_str(&format!("client_id = {}\n", quote(&s.client_id)));
            }
            if !s.scope.is_empty() {
                out.push_str(&format!("scope = {}\n", quote(&s.scope)));
            }
        }
        // **A tail is a section of its own**, because it is a thing of its own: a name, the
        // source it reads, and how that source is filtered. Written after the sources, so a
        // hand-edited file reads in the order a person would explain it.
        for t in &self.tails {
            out.push_str("\n[[tail]]\n");
            out.push_str(&format!("name = {}\n", quote(&t.name)));
            out.push_str(&format!("source = {}\n", quote(&t.source)));
            if !t.query.is_empty() {
                out.push_str(&format!("query = {}\n", quote(&t.query)));
            }
            if !t.apps.is_empty() {
                let listed: Vec<String> = t.apps.iter().map(|a| quote(a)).collect();
                out.push_str(&format!("apps = [{}]\n", listed.join(", ")));
            }
            if !t.columns.is_empty() {
                let listed: Vec<String> = t.columns.iter().map(|c| quote(c)).collect();
                out.push_str(&format!("columns = [{}]\n", listed.join(", ")));
            }
        }
        for f in &self.files {
            out.push_str("\n[[file]]\n");
            out.push_str(&format!("path = {}\n", quote(&f.path)));
            if !f.chips.is_empty() {
                let chips: Vec<String> = f.chips.iter().map(|c| quote(c)).collect();
                out.push_str(&format!("chips = [{}]\n", chips.join(", ")));
            }
            if f.any {
                out.push_str("filters = \"any\"\n");
            }
            if f.collapse {
                out.push_str("collapse = true\n");
            }
            if !f.bookmarks.is_empty() {
                let rows: Vec<String> = f.bookmarks.iter().map(u64::to_string).collect();
                out.push_str(&format!("bookmarks = [{}]\n", rows.join(", ")));
            }
            if !f.labels.is_empty() {
                let labels: Vec<String> = f.labels.iter().map(|l| quote(l)).collect();
                out.push_str(&format!("labels = [{}]\n", labels.join(", ")));
            }
            if !f.columns.is_empty() {
                let cols: Vec<String> = f.columns.iter().map(u64::to_string).collect();
                out.push_str(&format!("columns = [{}]\n", cols.join(", ")));
            }
            if !f.column_order.is_empty() {
                let order: Vec<String> = f.column_order.iter().map(u64::to_string).collect();
                out.push_str(&format!("column_order = [{}]\n", order.join(", ")));
            }
        }
        out
    }

    /// Reads the subset this program writes. Lenient: see the module note.
    ///
    /// **A leading byte-order mark is consumed**, because an editor puts one there and the file is
    /// meant to be editable by hand. Without this, `\u{feff}[[source]]` is not a section header,
    /// the file parses to nothing, and every configured source, recent file and preference in it
    /// vanishes with no message — §5.6 already consumes a BOM for a log the user opens, and our own
    /// settings deserve at least that. Found by writing a settings file from PowerShell, whose
    /// `Set-Content -Encoding utf8` writes one: two configured sources became none, and the menu
    /// item for the missing source did nothing at all.
    pub fn from_toml(text: &str) -> Settings {
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        let mut settings = Settings::default();
        let mut section = Section::None;
        let mut window = Window::default();
        let mut have_window = false;
        let mut file: Option<FileState> = None;
        let mut source: Option<LegacySource> = None;
        let mut tail: Option<Tail> = None;
        // **Every `[[source]]` is collected before any of it is believed**, because an entry that
        // carries a `query` or an `apps` is one of the old entry-per-view kind and the whole set
        // has to be split together — two entries are one source only if they agree, which cannot be
        // known one section at a time.
        let mut legacy: Vec<LegacySource> = Vec::new();
        let flush_file = |file: &mut Option<FileState>,
                          source: &mut Option<LegacySource>,
                          tail: &mut Option<Tail>,
                          legacy: &mut Vec<LegacySource>,
                          settings: &mut Settings| {
            if let Some(f) = file.take() {
                if !f.path.is_empty() {
                    settings.set_file(f);
                }
            }
            // A source with no name cannot key its own credential, so it is dropped rather than
            // kept as an entry that can never work.
            if let Some(s) = source.take() {
                if !s.source.name.trim().is_empty() {
                    legacy.retain(|e| e.source.name != s.source.name);
                    legacy.push(s);
                }
            }
            // A tail with no name, or none that says which source it reads, is the same case.
            if let Some(t) = tail.take() {
                if !t.name.trim().is_empty() && !t.source.trim().is_empty() {
                    settings.tails.retain(|e| e.name != t.name);
                    settings.tails.push(t);
                }
            }
        };
        for raw in text.lines() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some(header) = line.strip_prefix("[[").and_then(|l| l.strip_suffix("]]")) {
                flush_file(
                    &mut file,
                    &mut source,
                    &mut tail,
                    &mut legacy,
                    &mut settings,
                );
                section = match header.trim() {
                    "file" => {
                        file = Some(FileState::default());
                        Section::File
                    }
                    "source" => {
                        source = Some(LegacySource::default());
                        Section::Source
                    }
                    "tail" => {
                        tail = Some(Tail::default());
                        Section::Tail
                    }
                    _ => Section::Other,
                };
                continue;
            }
            if let Some(header) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
                flush_file(
                    &mut file,
                    &mut source,
                    &mut tail,
                    &mut legacy,
                    &mut settings,
                );
                section = match header.trim() {
                    "window" => {
                        have_window = true;
                        Section::Window
                    }
                    "appearance" => Section::Appearance,
                    "recent" => Section::Recent,
                    "find" => Section::Find,
                    _ => Section::Other,
                };
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let (key, value) = (key.trim(), value.trim());
            match section {
                Section::Appearance => match key {
                    "theme" => settings.theme = Some(unquote(value)),
                    "font" => settings.font = Some(unquote(value)),
                    "font_size" => settings.font_size = value.parse().ok(),
                    "remote_minutes" => {
                        settings.remote_minutes = value.parse().ok().filter(|m: &u32| *m > 0)
                    }
                    // Only the two spellings TOML has. Anything else leaves the key unset, which
                    // means the default rather than "hidden" — a typo must not silently take the
                    // row away, since the way back to it is the row's own menu item.
                    "toolbar" if value == "true" || value == "false" => {
                        settings.toolbar = Some(value == "true");
                    }
                    "toolbar_large" if value == "true" || value == "false" => {
                        settings.toolbar_large = Some(value == "true");
                    }
                    _ => {}
                },
                Section::Recent => {
                    if key == "files" {
                        settings.recent = array(value)
                            .iter()
                            .map(|s| Recent::decode(s))
                            .filter(|e| !e.is_spill())
                            .collect();
                        settings.recent.truncate(RECENT_MAX);
                    }
                }
                Section::Find => {
                    if key == "queries" {
                        settings.find_queries = array(value);
                        settings.find_queries.truncate(FIND_MAX);
                    }
                }
                Section::Window => match key {
                    "x" => window.x = value.parse().unwrap_or(0),
                    "y" => window.y = value.parse().unwrap_or(0),
                    "width" => window.width = value.parse().unwrap_or(0),
                    "height" => window.height = value.parse().unwrap_or(0),
                    "maximized" => window.maximized = value == "true",
                    _ => {}
                },
                Section::Tail => {
                    if let Some(t) = tail.as_mut() {
                        match key {
                            "name" => t.name = unquote(value),
                            "source" => t.source = unquote(value),
                            "query" => t.query = unquote(value),
                            "apps" => t.apps = array(value),
                            "columns" => t.columns = array(value),
                            _ => {}
                        }
                    }
                }
                // **A `[[source]]` may be either shape**, and which one it is cannot be known until
                // the section ends: a `query` or an `apps` key makes it one of the old
                // entry-per-view kind. So every one is read into a [`LegacySource`] and the
                // decision is taken when it is flushed.
                Section::Source => {
                    if let Some(s) = source.as_mut() {
                        match key {
                            "name" => s.source.name = unquote(value),
                            "url" => s.source.url = unquote(value),
                            "token_url" => s.source.token_url = unquote(value),
                            "auth_url" => s.source.auth_url = unquote(value),
                            "client_id" => s.source.client_id = unquote(value),
                            "scope" => s.source.scope = unquote(value),
                            "apps" => s.apps = array(value),
                            "query" => s.query = unquote(value),
                            // **A `secret` key is read and thrown away.** Somebody will eventually
                            // put one here by hand, and silently ignoring it is better than either
                            // honouring it — which would defeat the whole point of the credential
                            // store — or failing to open, which §12.4 forbids.
                            _ => {}
                        }
                    }
                }
                Section::File => {
                    if let Some(f) = file.as_mut() {
                        match key {
                            "path" => f.path = unquote(value),
                            "chips" => f.chips = array(value),
                            "filters" => f.any = unquote(value) == "any",
                            "collapse" => f.collapse = value == "true",
                            "bookmarks" => {
                                f.bookmarks =
                                    array(value).iter().filter_map(|v| v.parse().ok()).collect()
                            }
                            "labels" => f.labels = array(value),
                            "columns" => {
                                f.columns =
                                    array(value).iter().filter_map(|v| v.parse().ok()).collect()
                            }
                            "column_order" => {
                                f.column_order =
                                    array(value).iter().filter_map(|v| v.parse().ok()).collect()
                            }
                            _ => {}
                        }
                    }
                }
                // **The one key that lives outside every table**, because its absence is what says
                // the file predates the source/tail split — see [`MODEL`]. `Section::Other` is a
                // table this version does not know, and a `model` key inside one of those is not
                // this key.
                Section::None => {
                    if key == "model" {
                        settings.model = value.parse().ok();
                    }
                }
                Section::Other => {}
            }
        }
        flush_file(
            &mut file,
            &mut source,
            &mut tail,
            &mut legacy,
            &mut settings,
        );
        // **Which shape the file was in, decided once, at the end.**
        //
        // A `[[source]]` that carries a view — a query or a list of applications — is one of the old
        // entry-per-view kind, and the whole set is split: entries agreeing about everything but
        // the view become one source with a tail each. A file whose sources carry no view is
        // already in the new shape and its `[[tail]]` sections say the rest.
        //
        // **A file that has both is read as already migrated**, and its `[[source]]` entries are
        // taken as connections. That is the state a migrated file is written in, and it is also
        // what a hand-edited one should mean.
        // **The file says which shape it is in, and only a file that does not say is migrated.**
        // A `[[source]]` carrying a view is still taken as the old shape whatever the key says,
        // because a hand-edited file can be both wrong and sincere — but the key is what rescues
        // the case the view cannot be seen in: sources with no query and no apps, which the old
        // editor drew as *"(everything)"* and which are indistinguishable from new-shape sources
        // with no tails. See [`MODEL`].
        let old_shape = settings.model.is_none();
        let views = legacy
            .iter()
            .any(|e| !e.query.trim().is_empty() || !e.apps.is_empty());
        if (old_shape || views) && settings.tails.is_empty() && !legacy.is_empty() {
            let (sources, tails, renamed) = split_sources(&legacy);
            settings.sources = sources;
            settings.tails = tails;
            settings.migrated = renamed;
        } else {
            settings.sources = legacy.into_iter().map(|e| e.source).collect();
        }
        if have_window && window.width > 0 && window.height > 0 {
            settings.window = Some(window);
        }
        settings
    }
}

#[derive(Copy, Clone, PartialEq, Eq)]
enum Section {
    None,
    Window,
    Appearance,
    Recent,
    Find,
    File,
    Source,
    Tail,
    Other,
}

/// A TOML basic string.
fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04X}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn unquote(value: &str) -> String {
    let Some(inner) = value.strip_prefix('"').and_then(|v| v.strip_suffix('"')) else {
        return value.to_owned();
    };
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('"') => out.push('"'),
            Some('\\') => out.push('\\'),
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('u') => {
                let hex: String = chars.by_ref().take(4).collect();
                if let Some(ch) = u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                    out.push(ch);
                }
            }
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

/// A `["a", "b"]` array of strings.
fn array(value: &str) -> Vec<String> {
    let Some(inner) = value.strip_prefix('[').and_then(|v| v.strip_suffix(']')) else {
        return Vec::new();
    };
    // Split on commas outside quotes.
    let mut items = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut escaped = false;
    for c in inner.chars() {
        match c {
            '\\' if in_quotes && !escaped => {
                escaped = true;
                current.push(c);
                continue;
            }
            '"' if !escaped => in_quotes = !in_quotes,
            ',' if !in_quotes => {
                let item = current.trim();
                if !item.is_empty() {
                    items.push(unquote(item));
                }
                current.clear();
                escaped = false;
                continue;
            }
            _ => {}
        }
        escaped = false;
        current.push(c);
    }
    let item = current.trim();
    if !item.is_empty() {
        items.push(unquote(item));
    }
    items
}

/// §12.4's tiers: exe-adjacent, then `%APPDATA%\Tailhawk\`. `roaming` is the caller's answer for
/// `%APPDATA%` (the shell asks Windows; a test passes a directory).
pub fn tiers(exe_dir: Option<&Path>, roaming: Option<&Path>) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(d) = exe_dir {
        out.push(d.join(FILE_NAME));
    }
    if let Some(d) = roaming {
        out.push(d.join("Tailhawk").join(FILE_NAME));
    }
    out
}

/// Reads and merges every tier that exists — the earlier tier winning per key.
pub fn load(tiers: &[PathBuf]) -> Settings {
    let mut merged = Settings::default();
    // Later tiers first, so an earlier one merges *over* them.
    for path in tiers.iter().rev() {
        if let Ok(text) = std::fs::read_to_string(path) {
            merged = merged.merged_under(Settings::from_toml(&text));
        }
    }
    // **Entries keyed on a spill are dropped, and the next save writes the file without them.**
    // A remote tail's state was keyed on its part file in `%TEMP%` until 2026-10-08, so a settings
    // file written before this holds one dead entry per remote open — unmatchable, and growing
    // without bound. Nothing is lost by forgetting them: they were never found.
    merged.files.retain(|state| !is_spill_path(&state.path));
    merged
}

/// Writes to the first tier that takes a write — §12.4's probe is the write itself, atomic
/// replace-on-write. `None` if no tier was writable, which is what the "settings will not be saved"
/// chip is for. `stateless` writes nothing and says so by returning `None` too.
pub fn save(tiers: &[PathBuf], settings: &Settings, stateless: bool) -> Option<PathBuf> {
    if stateless {
        return None;
    }
    let text = settings.to_toml();
    for path in tiers {
        let Some(dir) = path.parent() else {
            continue;
        };
        if std::fs::create_dir_all(dir).is_err() {
            continue;
        }
        let tmp = path.with_extension("toml.tmp");
        if std::fs::write(&tmp, &text).is_err() {
            continue;
        }
        if std::fs::rename(&tmp, path).is_ok() {
            return Some(path.clone());
        }
        let _ = std::fs::remove_file(&tmp);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// File ▸ Open Recent's list: newest first, re-opening moves to the front, and two spellings
    /// of one Windows path are one entry — paths compare case-insensitively on this platform.
    #[test]
    fn the_recent_list_remembers_in_order_and_dedupes_windows_paths() {
        let file = |p: &str| Recent::File(p.to_owned());
        let mut s = Settings::default();
        s.remember_recent(file(r"C:\logs\a.log"));
        s.remember_recent(file(r"C:\logs\b.log"));
        s.remember_recent(file(r"C:\LOGS\A.LOG"));
        assert_eq!(
            s.recent,
            vec![file(r"C:\LOGS\A.LOG"), file(r"C:\logs\b.log")],
            "re-opening moved a to the front under its new spelling, not a third entry"
        );
        let read = Settings::from_toml(&s.to_toml());
        assert_eq!(read.recent, s.recent, "the list survives the file");
    }

    /// **A fetch window reads as a reader would say it**, and survives the settings file.
    ///
    /// "the last 240 minutes" is a number somebody has to divide before it means anything, so whole
    /// hours read as hours. The round trip matters because the choice is remembered: a reader who
    /// needs four hours needs four hours again tomorrow, which is the whole reason this is a
    /// setting rather than a per-run toggle.
    ///
    /// **The last case is a duration nobody offered.** §12.4's file is explicitly something a user
    /// edits, so a hand-written seven minutes is honoured rather than rounded to the nearest menu
    /// item — and the assertion checks that seven really is absent from the menu, so the case
    /// cannot quietly stop testing what it is named for.
    ///
    /// The round trip also caught a defect in its own right: the `[appearance]` heading is written
    /// only when one of its keys is set, and `remote_minutes` was missing from that condition — so
    /// a reader who changed the fetch window and nothing else had the choice discarded on save.
    #[test]
    fn a_fetch_window_reads_as_a_sentence_and_is_remembered() {
        assert_eq!(fetch_window_said(15), "the last 15 minutes");
        assert_eq!(fetch_window_said(60), "the last hour");
        assert_eq!(fetch_window_said(240), "the last 4 hours");
        assert_eq!(fetch_window_said(1440), "the last 24 hours");
        assert_eq!(
            fetch_window_said(90),
            "the last 90 minutes",
            "not 1.5 hours"
        );

        assert_eq!(
            remote_minutes_of(None),
            DEFAULT_REMOTE_MINUTES,
            "nothing chosen is the hour the constant used to be"
        );
        assert_eq!(
            remote_minutes_of(Some(0)),
            DEFAULT_REMOTE_MINUTES,
            "a zero in a hand-edited file would fetch an empty window"
        );
        assert_eq!(remote_minutes_of(Some(240)), 240);

        let s = Settings {
            remote_minutes: Some(240),
            ..Settings::default()
        };
        assert_eq!(Settings::from_toml(&s.to_toml()).remote_minutes, Some(240));

        let odd = Settings::from_toml("[appearance]\nremote_minutes = 7\n");
        assert_eq!(remote_minutes_of(odd.remote_minutes), 7);
        assert!(
            !FETCH_WINDOWS.iter().any(|(_, m)| *m == 7),
            "and it is genuinely not on the menu"
        );
    }

    /// **A spill part is never a recent file**, however it got into the list.
    ///
    /// The write side has refused to remember one since 2026-09-21, but the stored list still
    /// carries four from before that — and they are the *worst* kind of stale entry, because the
    /// directory is deleted when the tail ends, so the row names nothing and clicking it can only
    /// fail. Dropping them on read is what finally clears them: the next save rewrites the list
    /// without them.
    ///
    /// **Both spill shapes**, because `stdin.rs` writes two: a remote tail spills into a
    /// *directory* of parts, and a piped stream spills to a single *file* beside it. They share
    /// `SPILL_PREFIX` by construction, and only the directory form was caught when this was first
    /// written — a piped document's path is the file form, and nothing else refuses it, so it would
    /// have gone into the list and died there when the process exited.
    ///
    /// A `%TEMP%` path that is *not* a spill is left alone. Somebody tailing a log their own tool
    /// wrote to the temp directory is doing something ordinary, and this must not reach for "looks
    /// like a temporary file" when the shape it actually knows is "a file Tailhawk itself made".
    #[test]
    fn a_stale_spill_part_is_dropped_from_the_recent_list_on_read() {
        let real = r"C:\logs\app.log";
        let temp = r"C:\Users\x\AppData\Local\Temp\my-own.log";
        let spill = r"C:\Users\x\AppData\Local\Temp\tailhawk-spill-52140-00\part-000001.log";
        let piped = r"C:\Users\x\AppData\Local\Temp\tailhawk-spill-52140-00.log";
        let shouted = r"C:\USERS\X\APPDATA\LOCAL\TEMP\TAILHAWK-SPILL-52140-01\PART-000001.LOG";
        let mut s = Settings::default();
        s.recent.extend([
            Recent::File(spill.to_owned()),
            Recent::File(piped.to_owned()),
            Recent::File(real.to_owned()),
            Recent::File(shouted.to_owned()),
            Recent::File(temp.to_owned()),
            Recent::Remote {
                tail: "live".to_owned(),
                apps: Vec::new(),
            },
        ]);
        let read = Settings::from_toml(&s.to_toml());
        assert_eq!(
            read.recent,
            vec![
                Recent::File(real.to_owned()),
                Recent::File(temp.to_owned()),
                Recent::Remote {
                    tail: "live".to_owned(),
                    apps: Vec::new()
                }
            ],
            "both spill shapes went, the ordinary temp file and the remote stayed"
        );
    }

    /// **A remote source sits in the same list as a file, and survives the file it is written to.**
    ///
    /// The owner's ask of 2026-09-21. One list rather than two, because recency runs across both —
    /// and the round trip is through `to_toml`/`from_toml` rather than `encode`/`decode` alone,
    /// since the quoting and the escaping are two layers that have to agree.
    #[test]
    fn a_remote_source_is_remembered_beside_the_files_and_survives_the_file() {
        let mut s = Settings::default();
        s.remember_recent(Recent::File(r"C:\logs\a.log".to_owned()));
        s.remember_recent(Recent::Remote {
            tail: "live".to_owned(),
            apps: vec!["nurtur-gateway".to_owned()],
        });
        // The same source narrowed differently is a different window, so a second entry.
        s.remember_recent(Recent::Remote {
            tail: "live".to_owned(),
            apps: vec!["nurtur-identity".to_owned()],
        });
        assert_eq!(s.recent.len(), 3, "{:?}", s.recent);

        let read = Settings::from_toml(&s.to_toml());
        assert_eq!(read.recent, s.recent, "the list survives the file");
        assert!(
            s.to_toml().contains("loki://live?apps=nurtur-gateway"),
            "and is legible in it: {}",
            s.to_toml()
        );
    }

    /// **A path folds case; a source does not — and folding one loses a window.**
    ///
    /// Windows opens two spellings of a path as one file, so the list treats them as one entry.
    /// A Loki `app` value is compared exactly by the server and by everything in `apps`, so `live`
    /// on `Worker` and `live` on `worker` are two real windows; a dedupe that lower-cased the
    /// encoded form replaced one with the other and the reader lost it from the list.
    #[test]
    fn two_sources_differing_only_in_case_are_two_entries() {
        let of = |app: &str| Recent::Remote {
            tail: "live".to_owned(),
            apps: vec![app.to_owned()],
        };
        let mut s = Settings::default();
        s.remember_recent(of("Worker"));
        s.remember_recent(of("worker"));
        assert_eq!(s.recent.len(), 2, "{:?}", s.recent);

        // While a path still folds, which is the behaviour this must not have cost.
        let mut s = Settings::default();
        s.remember_recent(Recent::File(r"C:\logs\a.log".to_owned()));
        s.remember_recent(Recent::File(r"C:\LOGS\A.LOG".to_owned()));
        assert_eq!(s.recent.len(), 1, "{:?}", s.recent);
    }

    /// An application named as the empty string survives the round trip, which it did not while
    /// `decode` filtered empties out — reachable from a hand-written `{app=""}` selector.
    #[test]
    fn an_empty_application_name_is_not_quietly_dropped() {
        let there = Recent::Remote {
            tail: "live".to_owned(),
            apps: vec![String::new()],
        };
        assert_eq!(Recent::decode(&there.encode()), there);
        // And a source with no applications at all is still distinct from that.
        let none = Recent::Remote {
            tail: "live".to_owned(),
            apps: Vec::new(),
        };
        assert_eq!(Recent::decode(&none.encode()), none);
        assert_ne!(there, none);
    }

    /// **A settings file written before remote entries existed reads back unchanged.**
    ///
    /// The owner has a real file with real recents in it. Every one of them is a bare path, and a
    /// bare path is a file — this is the half of the scheme that cannot be allowed to drift.
    #[test]
    fn an_older_files_recent_list_still_reads_as_files() {
        let older =
            "[recent]\nfiles = [\"C:\\\\logs\\\\a.log\", \"\\\\\\\\server\\\\share\\\\b.log\"]\n";
        let read = Settings::from_toml(older);
        assert_eq!(
            read.recent,
            vec![
                Recent::File(r"C:\logs\a.log".to_owned()),
                Recent::File(r"\\server\share\b.log".to_owned()),
            ]
        );
    }

    /// Ten entries — the platform's customary MRU depth — and the oldest falls off.
    #[test]
    fn the_recent_list_holds_ten_newest() {
        let mut s = Settings::default();
        let file = |p: String| Recent::File(p);
        for i in 0..12 {
            s.remember_recent(file(format!(r"C:\logs\{i}.log")));
        }
        assert_eq!(s.recent.len(), RECENT_MAX);
        assert_eq!(s.recent[0], file(r"C:\logs\11.log".to_owned()));
        assert!(!s.recent.contains(&file(r"C:\logs\0.log".to_owned())));
        assert!(!s.recent.contains(&file(r"C:\logs\1.log".to_owned())));
    }

    /// The Find dialog's history: newest first, deduplicated **exactly** — case matters in a
    /// query in a way it does not in a Windows path — and capped like the recent files are.
    #[test]
    fn the_find_history_round_trips_and_dedupes_exactly() {
        let mut s = Settings::default();
        s.remember_query("Error");
        s.remember_query("timeout");
        s.remember_query("Error");
        assert_eq!(
            s.find_queries,
            vec!["Error".to_owned(), "timeout".to_owned()],
            "re-searching moves the query to the front, once"
        );
        s.remember_query("error");
        assert_eq!(
            s.find_queries.len(),
            3,
            "a different case is a different query"
        );
        let read = Settings::from_toml(&s.to_toml());
        assert_eq!(
            read.find_queries, s.find_queries,
            "the history survives the file"
        );
        for i in 0..12 {
            s.remember_query(&format!("q{i}"));
        }
        assert_eq!(s.find_queries.len(), FIND_MAX);
        assert_eq!(s.find_queries[0], "q11");
    }

    /// §12.4's per-key tier merge covers the list: an earlier tier's list wins whole.
    #[test]
    fn the_recent_list_merges_as_one_key() {
        let file = |p: &str| Recent::File(p.to_owned());
        let mut under = Settings::default();
        under.remember_recent(file(r"C:\old.log"));
        let mut over = Settings::default();
        over.remember_recent(file(r"C:\new.log"));
        let merged = under.clone().merged_under(over);
        assert_eq!(merged.recent, vec![file(r"C:\new.log")]);
        let merged = under.merged_under(Settings::default());
        assert_eq!(
            merged.recent,
            vec![file(r"C:\old.log")],
            "an empty over-tier does not erase the list"
        );
    }

    /// **An entry that can never work again is taken off the list**, and only that kind.
    #[test]
    fn a_recent_entry_can_be_forgotten_without_disturbing_the_rest() {
        let gone = Recent::Remote {
            tail: "nurtur-loki".to_owned(),
            apps: Vec::new(),
        };
        let mut s = Settings::default();
        s.remember_recent(Recent::File(r"C:\logs\a.log".to_owned()));
        s.remember_recent(gone.clone());
        s.remember_recent(Recent::File(r"C:\logs\b.log".to_owned()));

        assert!(s.forget_recent(&gone), "it was there");
        assert_eq!(
            s.recent,
            vec![
                Recent::File(r"C:\logs\b.log".to_owned()),
                Recent::File(r"C:\logs\a.log".to_owned()),
            ],
            "and the order of what is left is untouched"
        );
        assert!(
            !s.forget_recent(&gone),
            "forgetting it again reports that there was nothing to forget"
        );
    }

    /// **A remote source's layout is remembered under its recent-list name**, which is the whole
    /// of the owner's report of 2026-10-08: he chose the `app` and `severity` columns on a Loki
    /// tail and found them gone on the next run. The state was written every time — keyed on the
    /// spill part the tail happened to be reading, in a `%TEMP%` directory named after the
    /// process, so the key could never occur again.
    #[test]
    fn a_remote_sources_layout_is_kept_under_the_name_the_recent_list_uses() {
        let entry = Recent::Remote {
            tail: "nurtur-loki-live".to_owned(),
            apps: vec!["job-dispatcher".to_owned()],
        };
        let mut s = Settings::default();
        s.set_file(FileState {
            path: entry.encode(),
            columns: vec![0, 14, 9, 0],
            column_order: vec![2, 0, 1],
            ..FileState::default()
        });

        let found = s
            .file(&entry.encode())
            .expect("the window reopened from this entry must find its layout");
        assert_eq!(found.columns, vec![0, 14, 9, 0], "including hidden columns");
        assert_eq!(found.column_order, vec![2, 0, 1]);

        let written = s.to_toml();
        assert_eq!(
            Settings::from_toml(&written)
                .file(&entry.encode())
                .map(|f| f.columns.clone()),
            Some(vec![0, 14, 9, 0]),
            "and it survives the round trip through the file"
        );
    }

    /// **A source name's case is not folded, and this is why it matters.** `identity_of` already
    /// argued it for the recent list — a source name is the key a credential is stored under and
    /// an `app` value is compared exactly server-side — and the state key is now the same string,
    /// so the two must answer alike or a window's layout would be remembered under a name its own
    /// recent entry cannot produce.
    #[test]
    fn a_path_folds_case_and_a_source_does_not() {
        let mut s = Settings::default();
        s.set_file(FileState {
            path: r"C:\LOGS\App.LOG".to_owned(),
            collapse: true,
            ..FileState::default()
        });
        assert!(
            s.file(r"c:\logs\app.log").is_some(),
            "one file, however it is spelled"
        );

        s.set_file(FileState {
            path: "loki://live?apps=Worker".to_owned(),
            collapse: true,
            ..FileState::default()
        });
        s.set_file(FileState {
            path: "loki://live?apps=worker".to_owned(),
            bookmarks: vec![7],
            ..FileState::default()
        });
        assert_eq!(
            s.files.len(),
            3,
            "two applications that differ only in case are two windows: {:?}",
            s.files
        );
        assert_eq!(
            s.file("loki://live?apps=Worker").map(|f| f.collapse),
            Some(true),
            "and neither has taken the other's state"
        );
    }

    /// **A spill is never recorded, and the ones already written are forgotten on load.** Every
    /// remote open before 2026-10-08 left one entry keyed on a `%TEMP%` part file that nothing
    /// could ever match, and nothing pruned them.
    #[test]
    fn state_keyed_on_a_spill_is_neither_kept_nor_written() {
        let mut s = Settings::default();
        s.set_file(FileState {
            path: r"C:\Users\x\AppData\Local\Temp\tailhawk-spill-45456-00\part-000001.log"
                .to_owned(),
            collapse: true,
            ..FileState::default()
        });
        assert!(
            s.files.is_empty(),
            "a key that cannot occur twice is not a key: {:?}",
            s.files
        );

        assert!(is_spill_path(
            r"C:\Temp\tailhawk-spill-45456-00\part-000001.log"
        ));
        assert!(is_spill_path(r"C:\Temp\tailhawk-spill-45456.log"));
        assert!(!is_spill_path(r"C:\Users\nigel\.claude\logs\git-22.log"));
    }

    fn sample() -> Settings {
        let mut s = Settings {
            window: Some(Window {
                x: 100,
                y: 80,
                width: 1200,
                height: 800,
                maximized: false,
            }),
            remote_minutes: Some(240),
            files: Vec::new(),
            sources: vec![Source {
                name: "dev".to_owned(),
                url: "https://telemetry-dev.example/loki".to_owned(),
                token_url: "https://identity-dev.example/connect/token".to_owned(),
                client_id: "tailhawk".to_owned(),
                scope: "telemetry:read".to_owned(),
                auth_url: String::new(),
            }],
            tails: vec![Tail {
                name: "dev".to_owned(),
                source: "dev".to_owned(),
                query: "{environment=\"dev\"}".to_owned(),
                apps: Vec::new(),
                columns: Vec::new(),
            }],
            theme: Some("light".to_owned()),
            font: Some("Cascadia Mono".to_owned()),
            font_size: Some(18),
            toolbar: Some(false),
            toolbar_large: Some(true),
            recent: vec![Recent::File(r"C:\logs\app.log".to_owned())],
            find_queries: vec!["ERROR".to_owned(), r"time\d+".to_owned()],
            migrated: Vec::new(),
            model: Some(MODEL),
        };
        s.set_file(FileState {
            path: r"C:\logs\app.log".to_owned(),
            chips: vec!["+error".to_owned(), "-retry \"quoted\"".to_owned()],
            any: false,
            collapse: true,
            bookmarks: vec![0, 42, 1_000_000],
            labels: vec!["1:Exception".to_owned(), "9:a \"quoted\" one".to_owned()],
            columns: vec![19, 5, 0],
            column_order: Vec::new(),
        });
        s.set_file(FileState {
            path: r"C:\logs\other.log".to_owned(),
            chips: vec!["+job".to_owned()],
            any: false,
            collapse: false,
            bookmarks: Vec::new(),
            labels: Vec::new(),
            columns: Vec::new(),
            column_order: Vec::new(),
        });
        s
    }

    /// The owner, 2026-10-29: the rule the include chips combine under rides beside them as
    /// `filters = "any"`, and a file written before the rule existed — or one under *all* —
    /// says nothing and reads as *all*.
    #[test]
    fn the_any_rule_is_written_beside_the_chips_and_absent_means_all() {
        let mut s = Settings::default();
        s.set_file(FileState {
            path: r"C:\logs\app.log".to_owned(),
            chips: vec!["+a".to_owned(), "+b".to_owned()],
            any: true,
            ..Default::default()
        });
        let text = s.to_toml();
        assert!(text.contains("filters = \"any\"\n"), "{text}");
        let back = Settings::from_toml(&text);
        assert!(back.file(r"C:\logs\app.log").expect("the entry").any);

        let old = Settings::from_toml(concat!(
            "[[file]]\n",
            "path = \"C:\\\\logs\\\\app.log\"\n",
            "chips = [\"+a\"]\n",
        ));
        assert!(!old.file(r"C:\logs\app.log").expect("the entry").any);
        assert!(
            !old.to_toml().contains("filters"),
            "all is said by saying nothing"
        );

        // A rule chosen with no chips to apply it to is a preference, kept as `collapse` is.
        let mut alone = Settings::default();
        alone.set_file(FileState {
            path: r"C:\logs\bare.log".to_owned(),
            any: true,
            ..Default::default()
        });
        let back = Settings::from_toml(&alone.to_toml());
        assert!(back.file(r"C:\logs\bare.log").expect("kept").any);
    }

    /// **A key this version no longer knows costs the file nothing.**
    ///
    /// `filters_hidden` was written for every file until 2026-10-16 and is in every settings file
    /// on disk today, three times over in the owner's own. The panel it described is gone, so the
    /// key went with it — and a retired key must read as *nothing was said*, leaving the rest of
    /// the entry intact, rather than taking the file down with it. The next write drops the line.
    #[test]
    fn a_key_this_version_has_retired_leaves_the_rest_of_the_entry_alone() {
        let text = concat!(
            "[[file]]\n",
            "path = \"C:\\\\logs\\\\app.log\"\n",
            "chips = [\"+error\"]\n",
            "filters_hidden = true\n",
            "collapse = true\n",
        );
        let back = Settings::from_toml(text);
        let file = back.file(r"C:\logs\app.log").expect("the entry survives");
        assert_eq!(file.chips, vec!["+error".to_owned()], "{text}");
        assert!(
            file.collapse,
            "the key after the retired one is read: {text}"
        );
        assert!(
            !back.to_toml().contains("filters_hidden"),
            "and writing it back drops the line: {}",
            back.to_toml()
        );
    }

    #[test]
    fn settings_round_trip_through_the_toml_subset() {
        let s = sample();
        let text = s.to_toml();
        assert!(text.contains("[window]"), "{text}");
        assert!(text.contains(r#"path = "C:\\logs\\app.log""#), "{text}");
        assert!(
            text.contains(r#"chips = ["+error", "-retry \"quoted\""]"#),
            "{text}"
        );
        assert_eq!(Settings::from_toml(&text), s);
    }

    /// **A reorder used to be thrown away**, and the shape of the bug is worth pinning: the widths
    /// were written only when they differed from the measured ones, so a visit to the column
    /// chooser that moved a column and resized nothing wrote no state at all. The order is its own
    /// key for that reason, and it survives on its own.
    #[test]
    fn a_column_order_survives_on_its_own_with_the_measured_widths() {
        let mut s = Settings::default();
        s.set_file(FileState {
            path: r"C:\logs\app.log".to_owned(),
            column_order: vec![2, 0, 1],
            ..FileState::default()
        });
        let text = s.to_toml();
        assert!(text.contains("column_order = [2, 0, 1]"), "{text}");
        assert!(!text.contains("columns = ["), "no widths to write: {text}");
        let back = Settings::from_toml(&text);
        assert_eq!(
            back.file(r"C:\logs\app.log")
                .map(|f| f.column_order.clone()),
            Some(vec![2, 0, 1])
        );
    }

    #[test]
    fn a_file_with_nothing_to_say_is_forgotten_and_a_replaced_one_replaced() {
        let mut s = sample();
        s.set_file(FileState {
            path: r"C:\logs\app.log".to_owned(),
            chips: Vec::new(),
            any: false,
            collapse: false,
            bookmarks: Vec::new(),
            labels: Vec::new(),
            columns: Vec::new(),
            column_order: Vec::new(),
        });
        assert!(s.file(r"C:\logs\app.log").is_none());
        s.set_file(FileState {
            path: r"c:\LOGS\other.log".to_owned(),
            chips: vec!["+x".to_owned()],
            any: false,
            collapse: false,
            bookmarks: Vec::new(),
            labels: Vec::new(),
            columns: Vec::new(),
            column_order: Vec::new(),
        });
        assert_eq!(s.files.len(), 1, "case-insensitive path replaces");
        assert_eq!(s.file(r"C:\logs\other.log").unwrap().chips, ["+x"]);
    }

    /// **Sources round-trip, and no secret can reach the file.** The second half is the one that
    /// matters: the whole reason the credential lives in Credential Manager is that this file is
    /// read by hand, copied into pastes and swept into backups.
    #[test]
    fn sources_survive_a_round_trip_and_carry_no_secret() {
        let mut s = Settings::default();
        s.sources.push(Source {
            name: "dev".to_owned(),
            url: "https://telemetry-dev.example/loki".to_owned(),
            token_url: "https://identity-dev.example/connect/token".to_owned(),
            client_id: "tailhawk".to_owned(),
            scope: "telemetry:read".to_owned(),
            auth_url: "https://identity-dev.example/connect/authorize".to_owned(),
        });
        s.tails.push(Tail {
            name: "dev production".to_owned(),
            source: "dev".to_owned(),
            query: "{environment=~\"live|production\"}".to_owned(),
            apps: vec!["ndc".to_owned()],
            columns: Vec::new(),
        });
        s.sources.push(Source {
            name: "open".to_owned(),
            url: "https://logs.example".to_owned(),
            ..Source::default()
        });

        let text = s.to_toml();
        let back = Settings::from_toml(&text);
        assert_eq!(back.sources, s.sources, "round trip");
        assert_eq!(back.tails, s.tails, "and the tails with them");
        // An unauthenticated source writes three fewer keys rather than three empty ones.
        assert!(!text.contains("token_url = \"\""), "no empty keys");

        let secretish = ["secret", "password", "client_secret", "Bearer"];
        for word in secretish {
            assert!(
                !text.to_lowercase().contains(&word.to_lowercase()),
                "the settings file must never carry anything resembling {word:?}"
            );
        }

        // And a `secret` somebody typed in by hand is read and dropped, not honoured.
        let hand_edited = "[[source]]\nname = \"dev\"\nurl = \"https://a.example\"\n\
                           secret = \"hunter2\"\n";
        let read = Settings::from_toml(hand_edited);
        assert_eq!(read.sources.len(), 1);
        assert!(
            !read.to_toml().contains("hunter2"),
            "and it does not survive to be written back"
        );
        // **This fixture carries no `model` key, so it is an old file and reading it migrates.**
        // The source takes its host's name and the entry's name becomes the *tail's* — which is
        // the point of [`MODEL`]: before it, this exact shape (a source with no query and no apps)
        // was read as already-new, the tail was never minted, and `File ▸ Open remote` — which
        // lists tails — came up empty over a server that was still in the configuration.
        assert_eq!(read.sources[0].name, "a.example", "named after its host");
        assert_eq!(read.tails.len(), 1, "**and the view survived**");
        assert_eq!(read.tails[0].name, "dev", "under the name the entry had");
        assert_eq!(read.tails[0].source, "a.example");
    }

    /// A source with no name cannot key its own credential, so it is dropped rather than kept as an
    /// entry that can never work. Sources merge by name, as every other tiered artefact does.
    #[test]
    fn a_nameless_source_is_dropped_and_the_tiers_merge_by_name() {
        let read = Settings::from_toml("[[source]]\nurl = \"https://a.example\"\n");
        assert!(read.sources.is_empty(), "no name, no source");

        let exe_tier = Settings {
            sources: vec![
                Source {
                    name: "live".to_owned(),
                    url: "https://curated.example".to_owned(),
                    ..Source::default()
                },
                Source {
                    name: "mine".to_owned(),
                    url: "https://personal.example".to_owned(),
                    ..Source::default()
                },
            ],
            ..Settings::default()
        };
        let user_tier = Settings {
            sources: vec![Source {
                name: "live".to_owned(),
                url: "https://overridden.example".to_owned(),
                ..Source::default()
            }],
            ..Settings::default()
        };
        let merged = user_tier.merged_under(exe_tier);
        assert_eq!(merged.sources.len(), 2, "one replaced, one added");
        let live = merged.sources.iter().find(|s| s.name == "live").unwrap();
        assert_eq!(live.url, "https://curated.example", "the earlier tier wins");
        assert!(merged.sources.iter().any(|s| s.name == "mine"));
    }

    /// **A remote entry reads as the source it is, not as the spill file it opens.**
    ///
    /// The report of 2026-10-05: the welcome list showed "the temp directory with generated file
    /// names", and they really were remote tails — a Loki window's document is
    /// `tailhawk-spill-52140-00\part-000001.log`-shaped, which names nothing a reader knows.
    ///
    /// Three cases, in the order they are asserted. A source opened with a chosen set of
    /// applications names both, and never in the encoded form that belongs in the settings file. A
    /// source opened whole says so without inventing an application list. And a file keeps only the
    /// part a reader recognises, because a recent list is for recognising and a full path pushes the
    /// name itself off the end of the row — which is also why a spill part still reads as a path,
    /// and why the welcome surface has to take the curated list rather than `files`.
    #[test]
    fn a_recent_entry_reads_as_the_thing_a_reader_recognises() {
        let remote = Recent::Remote {
            tail: "live".to_owned(),
            apps: vec!["Worker".to_owned(), "Api".to_owned()],
        };
        let shown = remote.shown();
        assert!(shown.starts_with("live"), "{shown}");
        assert!(shown.contains("Worker"), "{shown}");
        assert!(
            shown.contains("(Loki)"),
            "it says what kind of thing it is: {shown}"
        );
        assert!(
            !shown.contains("loki://") && !shown.contains('?'),
            "the encoded form is for the file, not the reader: {shown}"
        );

        let whole = Recent::Remote {
            tail: "live".to_owned(),
            apps: Vec::new(),
        };
        assert_eq!(whole.shown(), "live  (Loki)");

        assert_eq!(
            Recent::File("C:\\logs\\api\\today.log".to_owned()).shown(),
            "api\\today.log"
        );
        assert_eq!(Recent::File("today.log".to_owned()).shown(), "today.log");

        assert_eq!(
            Recent::File("C:\\Temp\\tailhawk-spill-52140-00\\part-000001.log".to_owned()).shown(),
            "tailhawk-spill-52140-00\\part-000001.log"
        );
    }

    /// **A remembered application set survives the round trip, and an empty one writes nothing.**
    /// Empty means "ask me", so a tail that has never had a choice made must not come back carrying
    /// an empty list that reads as a deliberate selection of nothing.
    ///
    /// **And it is two tails of one source now, which is the whole point of the split.** This test
    /// used to end by asserting that two *sources* over one Loki shared a URL and a selector and
    /// differed only in name — "the other half of the ask", it said. That shape is what gave one
    /// server three credentials under three names; the ask is still honoured and the model finally
    /// says so.
    #[test]
    fn a_remembered_application_set_comes_back_whole() {
        let mut s = Settings::default();
        s.sources.push(Source {
            name: "live".to_owned(),
            url: "https://telemetry.example/loki".to_owned(),
            token_url: "https://identity.example/connect/token".to_owned(),
            auth_url: "https://identity.example/connect/authorize".to_owned(),
            client_id: "tailhawk".to_owned(),
            scope: "telemetry:read".to_owned(),
        });
        s.tails.push(Tail {
            name: "live-workers".to_owned(),
            source: "live".to_owned(),
            query: "{environment=\"live\"}".to_owned(),
            apps: vec![
                "nurtur-gateway".to_owned(),
                "campaign-editor-api".to_owned(),
            ],
            columns: Vec::new(),
        });
        s.tails.push(Tail {
            name: "live-everything".to_owned(),
            apps: Vec::new(),
            ..s.tails[0].clone()
        });

        let text = s.to_toml();
        assert!(
            text.contains("apps = [\"nurtur-gateway\", \"campaign-editor-api\"]"),
            "{text}"
        );
        let back = Settings::from_toml(&text);
        assert_eq!(back.tails[0].apps, s.tails[0].apps);
        assert!(
            back.tails[1].apps.is_empty(),
            "a tail with nothing remembered must not grow an empty list"
        );
        assert_eq!(
            text.matches("apps = ").count(),
            1,
            "only the entry that remembers something writes the key: {text}"
        );

        // **Two views of one Loki: one server, one credential, two names.**
        assert_eq!(back.sources.len(), 1, "and only one connection for both");
        assert_eq!(back.tails[0].source, back.tails[1].source);
    }

    /// The columns a reader chose for a tail are written beside its selector, as names and in the
    /// order chosen, and only for a tail that chose any — a tail left to the defaults writes
    /// nothing, so a later change to the defaults still reaches it.
    #[test]
    fn a_tails_chosen_columns_come_back_in_order_and_an_unchosen_tail_writes_none() {
        let mut s = Settings::default();
        s.tails.push(Tail {
            name: "starberry".to_owned(),
            source: "telemetry".to_owned(),
            query: "{environment=\"live\"}".to_owned(),
            apps: vec!["sb-edge".to_owned(), "sb-gateway".to_owned()],
            columns: vec![
                "status".to_owned(),
                "path".to_owned(),
                "host_name".to_owned(),
            ],
        });
        s.tails.push(Tail {
            name: "everything".to_owned(),
            columns: Vec::new(),
            ..s.tails[0].clone()
        });

        let text = s.to_toml();
        assert!(
            text.contains("columns = [\"status\", \"path\", \"host_name\"]"),
            "{text}"
        );
        let back = Settings::from_toml(&text);
        assert_eq!(back.tails[0].columns, s.tails[0].columns);
        assert!(back.tails[1].columns.is_empty());
        let tails = text.split("[[tail]]").nth(2).expect("the second tail");
        assert!(
            !tails.contains("columns ="),
            "a tail with no choice must not write the key: {tails}"
        );
        assert_eq!(back.tails[0].query, back.tails[1].query);
        assert_ne!(back.tails[0].name, back.tails[1].name);
    }

    /// **A source says why it cannot be used, while it is being typed.** The alternative is a
    /// query that fails later with whatever the server happened to say, which is a much worse place
    /// to learn that a URL was http or a client id was left blank.
    #[test]
    fn a_source_reports_the_first_thing_wrong_with_it() {
        let good = Source {
            name: "dev".to_owned(),
            url: "https://telemetry-dev.example/loki".to_owned(),
            token_url: "https://identity-dev.example/connect/token".to_owned(),
            client_id: "tailhawk".to_owned(),
            scope: "telemetry:read".to_owned(),
            auth_url: String::new(),
        };
        assert_eq!(good.fault(), None, "a complete source is usable");
        assert!(
            !good.signs_in(),
            "a source with no sign-in URL uses its stored client secret, as it always has"
        );

        // **Signing in needs all three endpoints.** The device endpoint starts the sign-in and the
        // token endpoint finishes it, so one without the other is configured for neither flow —
        // and the poll would 404 at the point a person had already typed their password.
        let signs_in = Source {
            auth_url: "https://identity-dev.example/connect/authorize".to_owned(),
            ..good.clone()
        };
        assert_eq!(signs_in.fault(), None);
        assert!(signs_in.signs_in());

        assert!(
            Source {
                auth_url: "http://identity-dev.example/connect/authorize".to_owned(),
                ..good.clone()
            }
            .fault()
            .is_some(),
            "a sign-in over cleartext is refused like every other endpoint"
        );
        assert!(
            Source {
                token_url: String::new(),
                client_id: String::new(),
                ..signs_in.clone()
            }
            .fault()
            .is_some(),
            "a sign-in URL with nothing to finish the exchange is half-configured"
        );

        let unauthenticated = Source {
            token_url: String::new(),
            client_id: String::new(),
            scope: String::new(),
            ..good.clone()
        };
        assert_eq!(
            unauthenticated.fault(),
            None,
            "a Loki that wants no token is a real deployment, not a broken source"
        );

        let named = |name: &str| Source {
            name: name.to_owned(),
            ..good.clone()
        };
        assert!(named("").fault().is_some(), "a source needs a name");
        assert!(named("  ").fault().is_some(), "and not a blank one");
        assert!(
            named("dev/live").fault().is_some(),
            "a name that cannot key a credential is not a name"
        );

        let with_url = |url: &str| Source {
            url: url.to_owned(),
            ..good.clone()
        };
        assert!(with_url("").fault().is_some(), "a source needs a URL");
        assert!(
            with_url("http://telemetry.example/loki").fault().is_some(),
            "**http is refused**: a bearer token on a cleartext connection is the token given away"
        );
        assert!(with_url("telemetry.example").fault().is_some(), "not a URL");

        // Half-configured auth is the case worth catching: it looks finished and cannot work.
        assert!(
            Source {
                client_id: String::new(),
                ..good.clone()
            }
            .fault()
            .is_some(),
            "a token endpoint with no client id"
        );
        assert!(
            Source {
                token_url: String::new(),
                ..good.clone()
            }
            .fault()
            .is_some(),
            "a client id with no token endpoint"
        );
        assert!(
            Source {
                token_url: "http://identity.example/connect/token".to_owned(),
                ..good.clone()
            }
            .fault()
            .is_some(),
            "the token endpoint is a second origin and gets the same rule"
        );
    }

    /// **The suggestion that makes the new sign-in field answerable**, and the whole reason it
    /// exists: the owner gave the installed app to a colleague on 2026-10-13, who added a Loki
    /// source and was never offered a login. A reader who has a token URL almost always has the
    /// authorize URL too — it is the same path with the last segment changed — so the dialog
    /// offers it rather than asking for a URL most people would have to go and look up.
    #[test]
    fn the_authorize_url_beside_a_token_url_is_offered_but_never_guessed_from_nothing() {
        assert_eq!(
            authorize_beside("https://identity.nurtur.tech/connect/token").as_deref(),
            Some("https://identity.nurtur.tech/connect/authorize"),
            "IdentityServer's spelling, which is this estate's"
        );
        assert_eq!(
            authorize_beside("https://login.example.com/x/oauth2/v2.0/token").as_deref(),
            Some("https://login.example.com/x/oauth2/v2.0/authorize"),
            "and Entra's, which differs only in the path above it"
        );
        assert_eq!(
            authorize_beside("  https://identity.example/connect/token/  ").as_deref(),
            Some("https://identity.example/connect/authorize"),
            "a pasted URL carries whitespace and often a trailing slash"
        );

        assert_eq!(
            authorize_beside("http://identity.example/connect/token"),
            None,
            "never suggest a cleartext sign-in: Source::fault would refuse it anyway"
        );
        assert_eq!(
            authorize_beside("https://identity.example/connect/authorize"),
            None,
            "nothing to derive from a URL that does not end in a token segment"
        );
        assert_eq!(
            authorize_beside("https://identity.example"),
            None,
            "a bare host names no endpoint"
        );
        // **A host that is itself called `token` must not be split on the scheme's own slash.**
        // Found in review before this shipped: `rsplit_once` took `https://token` apart into
        // `https:/` and `token` and suggested `https:/authorize` — a malformed URL written into a
        // field the reader never touched, which `Source::fault` would then refuse to accept.
        for mangled in ["https://token", "https://token/", "https://TOKEN"] {
            assert_eq!(
                authorize_beside(mangled),
                None,
                "{mangled} has no path to replace a segment of"
            );
        }
        assert_eq!(
            authorize_beside("https://mytoken/connect/token").as_deref(),
            Some("https://mytoken/connect/authorize"),
            "but a host merely ending in token is an ordinary host"
        );
        assert_eq!(
            authorize_beside(""),
            None,
            "and an empty field suggests nothing"
        );
    }

    /// **An unreadable `toolbar` value leaves the row alone.** The only way back to a hidden
    /// toolbar is the toolbar's own menu item, so a typo in a hand-edited settings file must not be
    /// able to take it away — absent and unparseable both have to mean "the default", and the
    /// default is shown.
    #[test]
    fn only_a_real_boolean_hides_the_toolbar() {
        let read = |text: &str| Settings::from_toml(text).toolbar;
        assert_eq!(read("[appearance]\ntoolbar = false\n"), Some(false));
        assert_eq!(read("[appearance]\ntoolbar = true\n"), Some(true));
        assert_eq!(read("[appearance]\ntoolbar = \"no\"\n"), None, "a string");
        assert_eq!(read("[appearance]\ntoolbar = 0\n"), None, "a number");
        assert_eq!(read("[appearance]\ntoolbar = False\n"), None, "wrong case");
        assert_eq!(read("[appearance]\ntheme = \"dark\"\n"), None, "absent");
    }

    #[test]
    fn a_hostile_or_foreign_file_yields_defaults_rather_than_an_error() {
        assert_eq!(
            Settings::from_toml("this is not toml\n=\n[[[\n"),
            Settings::default()
        );
        let s = Settings::from_toml("[window]\nx = 10\ny = 10\nwidth = notanumber\nheight = 5\n");
        assert_eq!(s.window, None, "a window with no width is no window");
        let s = Settings::from_toml("[future]\nthing = 1\n[window]\nx=1\ny=2\nwidth=3\nheight=4\n");
        assert_eq!(
            s.window.map(|w| w.width),
            Some(3),
            "unknown tables are skipped"
        );
    }

    #[test]
    fn tiers_merge_with_the_earlier_winning_and_the_first_writable_takes_the_write() {
        let root = std::env::temp_dir().join("tailhawk-settings-test");
        let _ = std::fs::remove_dir_all(&root);
        let exe = root.join("exe");
        let roaming = root.join("roaming");
        std::fs::create_dir_all(&exe).expect("dirs");
        std::fs::create_dir_all(&roaming).expect("dirs");
        let tiers = tiers(Some(&exe), Some(&roaming));
        assert_eq!(tiers.len(), 2);
        assert!(tiers[1].ends_with(Path::new("Tailhawk").join(FILE_NAME)));

        // Only roaming has a file: it is what loads.
        let mut personal = Settings::default();
        personal.set_file(FileState {
            path: "a.log".into(),
            chips: vec!["+a".into()],
            any: false,
            collapse: false,
            bookmarks: Vec::new(),
            labels: Vec::new(),
            columns: Vec::new(),
            column_order: Vec::new(),
        });
        std::fs::create_dir_all(tiers[1].parent().unwrap()).unwrap();
        std::fs::write(&tiers[1], personal.to_toml()).unwrap();
        assert_eq!(load(&tiers).file("a.log").unwrap().chips, ["+a"]);

        // Exe-adjacent appears and disagrees: it wins per key, and roaming's other keys survive.
        let mut curated = Settings::default();
        curated.set_file(FileState {
            path: "a.log".into(),
            chips: vec!["+curated".into()],
            any: false,
            collapse: true,
            bookmarks: Vec::new(),
            labels: Vec::new(),
            columns: Vec::new(),
            column_order: Vec::new(),
        });
        curated.set_file(FileState {
            path: "b.log".into(),
            chips: vec!["+b".into()],
            any: false,
            collapse: false,
            bookmarks: Vec::new(),
            labels: Vec::new(),
            columns: Vec::new(),
            column_order: Vec::new(),
        });
        std::fs::write(&tiers[0], curated.to_toml()).unwrap();
        let merged = load(&tiers);
        assert_eq!(merged.file("a.log").unwrap().chips, ["+curated"]);
        assert_eq!(merged.file("b.log").unwrap().chips, ["+b"]);

        // The write goes to the first writable tier — the exe-adjacent one here.
        let wrote = save(&tiers, &merged, false).expect("a tier is writable");
        assert_eq!(wrote, tiers[0]);
        assert!(
            save(&tiers, &merged, true).is_none(),
            "stateless writes nothing"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// **A settings file with a byte-order mark is still a settings file.** Every editor on this
    /// platform will write one sooner or later — PowerShell's own `Set-Content -Encoding utf8`
    /// does — and before this the whole file parsed to nothing: no sources, no recent files, no
    /// preferences, and not a word to say why. The menu item for a source that had gone simply did
    /// nothing when clicked.
    #[test]
    fn a_settings_file_with_a_byte_order_mark_is_read() {
        let toml = "[[source]]\nname = \"live\"\nurl = \"https://example.com/loki\"\nquery = \"{environment=\\\"live\\\"}\"\n";
        let plain = Settings::from_toml(toml);
        let marked = Settings::from_toml(&format!("\u{feff}{toml}"));

        assert_eq!(plain.sources.len(), 1, "the fixture itself must parse");
        assert_eq!(
            marked.sources.len(),
            1,
            "a leading BOM must not cost the file its contents"
        );
        assert_eq!(marked.sources[0].url, "https://example.com/loki");
        // **The fixture is an *old* file — its `[[source]]` carries a query — so reading it
        // migrates it, and the name `live` is the *tail's*.** This assertion used to be
        // `sources[0].name == "live"` and failed on the day the split landed, which is the
        // migration working rather than the BOM failing. Said here because a test named for one
        // thing failing over another is how an hour goes missing.
        assert_eq!(marked.tails[0].name, "live");
        assert_eq!(marked.sources[0].name, "example.com");
        assert_eq!(marked.tails[0].source, "example.com");
    }

    /// **A path is never mistaken for a source, whatever it contains.**
    ///
    /// This is the whole reason the scheme is `loki://`: a Windows drive is one letter, so no real
    /// path begins with it. A path that merely *mentions* it somewhere is still a path, which a
    /// looser test — "contains" rather than "starts with" — would get wrong.
    #[test]
    fn a_file_entry_survives_the_round_trip_and_is_never_read_as_a_source() {
        for path in [
            r"C:\logs\service.log",
            r"\\server\share\a,b.log",
            r"C:\odd\loki://not-a-source.log",
            "relative/with spaces.log",
        ] {
            let there = Recent::File(path.to_owned());
            let back = Recent::decode(&there.encode());
            assert_eq!(back, there, "{path} did not survive");
            assert_eq!(back.path(), Some(path));
        }
    }

    /// A settings file written before remote entries existed is all bare paths, and must still
    /// read as files rather than as anything else.
    #[test]
    fn an_older_settings_files_bare_paths_are_files() {
        assert_eq!(
            Recent::decode(r"C:\logs\old.log"),
            Recent::File(r"C:\logs\old.log".to_owned())
        );
    }

    #[test]
    fn a_remote_entry_carries_its_source_and_its_applications() {
        let there = Recent::Remote {
            tail: "live".to_owned(),
            apps: vec!["nurtur-gateway".to_owned(), "nurtur-identity".to_owned()],
        };
        assert_eq!(
            there.encode(),
            "loki://live?apps=nurtur-gateway,nurtur-identity"
        );
        assert_eq!(Recent::decode(&there.encode()), there);
        assert_eq!(there.path(), None, "a source is not a path to open");
    }

    /// A source opened with nothing narrowed keeps no application list, and must not grow an
    /// empty one on the way back.
    #[test]
    fn a_remote_entry_with_no_applications_says_nothing_about_them() {
        let there = Recent::Remote {
            tail: "live".to_owned(),
            apps: Vec::new(),
        };
        assert_eq!(there.encode(), "loki://live");
        assert_eq!(Recent::decode(&there.encode()), there);
    }

    /// **The separators are escaped, because a source's name is whatever the reader called it.**
    ///
    /// `?` ends the name and `,` ends an application, so a name containing either would otherwise
    /// come back cut in half — and `%` has to be escaped first or unescaping would undo an escape
    /// the name itself contained.
    #[test]
    fn a_name_containing_the_separators_comes_back_whole() {
        let there = Recent::Remote {
            tail: "live? 50%".to_owned(),
            apps: vec!["a,b".to_owned(), "100%".to_owned()],
        };
        assert_eq!(Recent::decode(&there.encode()), there);
    }

    /// **The migration the owner's insight asked for**, 2026-10-21: *"we might have a single source,
    /// the nurtur loki server, but three separate tails, say one looking at ndc, one at identity
    /// server and one at job dispatcher"*. Three old entries differing only in their selector were
    /// always one server; the file said so by repeating the other five fields three times.
    #[test]
    fn three_old_entries_over_one_loki_become_one_source_and_three_tails() {
        let legacy: Vec<LegacySource> = ["ndc", "identity", "jobdispatcher"]
            .iter()
            .map(|app| LegacySource {
                source: Source {
                    name: format!("nurtur-loki-{app}"),
                    url: "https://telemetry.nurtur.tech/loki".to_owned(),
                    token_url: "https://identity.nurtur.tech/connect/token".to_owned(),
                    auth_url: "https://identity.nurtur.tech/connect/authorize".to_owned(),
                    client_id: "tailhawk".to_owned(),
                    scope: "telemetry:read".to_owned(),
                },
                query: format!("{{application=\"{app}\"}}"),
                apps: vec![(*app).to_owned()],
            })
            .collect();

        let (sources, tails, renamed) = split_sources(&legacy);
        assert_eq!(sources.len(), 1, "one server: {sources:?}");
        assert_eq!(
            sources[0].name, "telemetry.nurtur.tech",
            "named after its host, because the old names belong to the views"
        );
        let names: Vec<&str> = tails.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "nurtur-loki-ndc",
                "nurtur-loki-identity",
                "nurtur-loki-jobdispatcher"
            ],
            "**and the views keep their names**, which is what `Recent::Remote` remembers and what \
             a tab is called — a migration that renamed them would break every recent open"
        );
        assert!(
            tails.iter().all(|t| t.source == sources[0].name),
            "all three read the one server"
        );
        assert_eq!(tails[0].apps, ["ndc"]);
        assert_eq!(tails[0].query, "{application=\"ndc\"}");

        // **And the credential's old names, in file order**, because the shell has to move it and
        // the store is keyed on a name that has just changed.
        assert_eq!(renamed.len(), 1);
        assert_eq!(renamed[0].now, "telemetry.nurtur.tech");
        assert_eq!(renamed[0].was, names, "every entry that folded in");
    }

    /// Entries that differ in the **connection** are different servers, however alike their views.
    #[test]
    fn entries_against_different_servers_stay_apart() {
        let legacy: Vec<LegacySource> = ["dev", "live"]
            .iter()
            .map(|env| LegacySource {
                source: Source {
                    name: format!("loki-{env}"),
                    url: format!("https://telemetry-{env}.example/loki"),
                    ..Source::default()
                },
                query: "{application=\"ndc\"}".to_owned(),
                apps: Vec::new(),
            })
            .collect();
        let (sources, tails, _) = split_sources(&legacy);
        assert_eq!(sources.len(), 2);
        assert_eq!(tails.len(), 2);
        assert_ne!(tails[0].source, tails[1].source);
    }

    /// **Two servers on one host get names of their own.** Dev and live behind one hostname,
    /// differing in a client id or a scope, would otherwise both want the host's name — and two
    /// sources sharing a name is one credential for two servers, which `Editor::fault` refuses and
    /// the store could not tell apart.
    #[test]
    fn two_servers_behind_one_host_do_not_share_a_name() {
        let legacy: Vec<LegacySource> = ["one", "two"]
            .iter()
            .map(|client| LegacySource {
                source: Source {
                    name: format!("loki-{client}"),
                    url: "https://telemetry.example/loki".to_owned(),
                    token_url: "https://identity.example/connect/token".to_owned(),
                    client_id: (*client).to_owned(),
                    scope: "telemetry:read".to_owned(),
                    ..Source::default()
                },
                query: "{application=\"ndc\"}".to_owned(),
                apps: Vec::new(),
            })
            .collect();
        let (sources, _, renamed) = split_sources(&legacy);
        assert_eq!(sources.len(), 2);
        assert_ne!(sources[0].name, sources[1].name, "{sources:?}");
        assert_eq!(renamed.len(), 2, "each has its own credential to move");
    }

    /// A host that cannot be a credential key does not become one: a port, or an address literal.
    #[test]
    fn a_host_with_a_port_is_cleaned_into_a_name_the_store_accepts() {
        let legacy = [LegacySource {
            source: Source {
                name: "local".to_owned(),
                url: "https://[::1]:3100/loki".to_owned(),
                ..Source::default()
            },
            query: "{application=\"ndc\"}".to_owned(),
            apps: Vec::new(),
        }];
        let (sources, _, _) = split_sources(&legacy);
        assert_eq!(sources.len(), 1);
        assert!(
            !sources[0].name.contains(':') && !sources[0].name.contains('/'),
            "{:?} would be refused by Source::fault and by the store",
            sources[0].name
        );
        assert_eq!(sources[0].fault(), None, "{:?}", sources[0]);
    }

    /// **A source whose new name is one it already had needs nothing moved**, and saying it does
    /// would have the shell copy a credential onto itself and then delete it.
    #[test]
    fn a_source_that_kept_its_name_reports_no_rename() {
        let legacy = [LegacySource {
            source: Source {
                name: "telemetry.example".to_owned(),
                url: "https://telemetry.example/loki".to_owned(),
                ..Source::default()
            },
            query: "{application=\"ndc\"}".to_owned(),
            apps: Vec::new(),
        }];
        let (sources, tails, renamed) = split_sources(&legacy);
        assert_eq!(sources[0].name, "telemetry.example");
        assert_eq!(tails[0].name, "telemetry.example");
        assert!(
            renamed.is_empty(),
            "the name did not change, so the credential is already where it belongs"
        );
    }

    /// **Which name a credential moves from, decided without a store.** The store is only ever
    /// touched on this function's answer, which is why it is a function.
    #[test]
    fn a_credential_already_under_the_new_name_is_never_overwritten() {
        let was = [("old-one".to_owned(), true), ("old-two".to_owned(), true)];
        assert_eq!(
            secret_to_move(true, &was),
            None,
            "a reader who signed in since the upgrade is not rolled back to a token from before it"
        );
        assert_eq!(
            secret_to_move(false, &was),
            Some("old-one"),
            "otherwise the first old name that has one, in the order the file put them"
        );
        assert_eq!(
            secret_to_move(
                false,
                &[("old-one".to_owned(), false), ("old-two".to_owned(), true)]
            ),
            Some("old-two"),
            "skipping the ones that hold nothing"
        );
        assert_eq!(secret_to_move(false, &[]), None);
        assert_eq!(
            secret_to_move(false, &[("old-one".to_owned(), false)]),
            None,
            "nothing anywhere is nothing to move, not an empty write"
        );
    }

    /// **A file already in the new shape is not migrated again.** `[[source]]` entries carrying no
    /// view are connections, and the `[[tail]]` sections say the rest — which is exactly what this
    /// product writes from now on, so re-reading its own output must be a no-op.
    #[test]
    fn a_file_in_the_new_shape_is_read_as_it_stands() {
        let text =
            "[[source]]\nname = \"telemetry.example\"\nurl = \"https://telemetry.example/loki\"\n\
                    \n[[tail]]\nname = \"ndc\"\nsource = \"telemetry.example\"\n\
                    query = \"{application=\\\"ndc\\\"}\"\n";
        let read = Settings::from_toml(text);
        assert_eq!(read.sources.len(), 1);
        assert_eq!(read.sources[0].name, "telemetry.example");
        assert_eq!(read.tails.len(), 1);
        assert_eq!(read.tails[0].name, "ndc");
        assert_eq!(read.tails[0].source, "telemetry.example");
        assert!(
            read.migrated.is_empty(),
            "nothing was renamed, so no credential should move"
        );
        // And round-tripping it changes nothing, which is what makes the migration one-way-once.
        let again = Settings::from_toml(&read.to_toml());
        assert_eq!(again.sources, read.sources);
        assert_eq!(again.tails, read.tails);
        assert!(again.migrated.is_empty());
    }

    /// **The migration note never reaches the file.** It is an instruction to move a credential
    /// once, not a setting, and a `migrated` key in a settings file would be read back next launch
    /// and acted on again.
    #[test]
    fn the_migration_note_is_not_written_and_does_not_survive_a_save() {
        let old =
            "[[source]]\nname = \"nurtur-loki-ndc\"\nurl = \"https://telemetry.example/loki\"\n\
                   query = \"{application=\\\"ndc\\\"}\"\n";
        let read = Settings::from_toml(old);
        assert_eq!(read.migrated.len(), 1, "it migrated, so there is a note");

        let text = read.to_toml();
        assert!(!text.contains("migrated"), "{text}");
        // **The old name survives as the tail's and not as the source's**, which is the thing worth
        // asserting and which this could not say for a few hours: it read
        // `!text.contains("nurtur-loki-ndc\n")`, and `to_toml` always writes
        // `name = "nurtur-loki-ndc"` — closing quote, *then* newline — so the searched substring
        // could never appear whatever the code did. An assertion that cannot fail is worse than
        // none, because it is counted.
        let written = Settings::from_toml(&text);
        assert_eq!(written.sources.len(), 1);
        assert_eq!(written.sources[0].name, "telemetry.example");
        assert_eq!(written.tails.len(), 1);
        assert_eq!(written.tails[0].name, "nurtur-loki-ndc");
        assert!(
            Settings::from_toml(&text).migrated.is_empty(),
            "the written file is in the new shape, so reading it migrates nothing"
        );
    }

    /// Tails merge by name across the tiers, as sources do — a curated tail beside the exe and the
    /// user's own of the same name is one tail, not the two that `Editor::fault` refuses.
    #[test]
    fn tails_merge_by_name_across_the_tiers() {
        let curated = Settings {
            tails: vec![
                Tail {
                    name: "ndc".to_owned(),
                    source: "live".to_owned(),
                    query: "{application=\"ndc\"}".to_owned(),
                    apps: Vec::new(),
                    columns: Vec::new(),
                },
                Tail {
                    name: "identity".to_owned(),
                    source: "live".to_owned(),
                    query: "{application=\"identity\"}".to_owned(),
                    apps: Vec::new(),
                    columns: Vec::new(),
                },
            ],
            ..Settings::default()
        };
        let mine = Settings {
            tails: vec![Tail {
                name: "ndc".to_owned(),
                source: "live".to_owned(),
                query: "{application=\"ndc\"} |= `error`".to_owned(),
                apps: Vec::new(),
                columns: Vec::new(),
            }],
            ..Settings::default()
        };
        let merged = curated.merged_under(mine);
        assert_eq!(merged.tails.len(), 2, "{:?}", merged.tails);
        let ndc = merged
            .tails
            .iter()
            .find(|t| t.name == "ndc")
            .expect("still there");
        assert_eq!(
            ndc.query, "{application=\"ndc\"} |= `error`",
            "the user's own wins, as it does for a source"
        );
    }

    fn renamed(now: &str, was: &[&str]) -> Renamed {
        Renamed {
            now: now.to_owned(),
            was: was.iter().map(|w| (*w).to_owned()).collect(),
        }
    }

    fn owned(names: &[&str]) -> Vec<String> {
        names.iter().map(|n| (*n).to_owned()).collect()
    }

    /// The ordinary case: three views of one Loki fold into one server named after its host, and
    /// the credential comes from whichever of the three had one.
    #[test]
    fn the_plan_carries_one_credential_from_the_entries_that_folded_together() {
        let plan = credential_plan(
            &[renamed(
                "telemetry.nurtur.tech",
                &["nurtur-loki-ndc", "nurtur-loki-identity"],
            )],
            &owned(&["nurtur-loki-identity"]),
        );
        assert_eq!(
            plan.moves,
            [CredentialMove {
                from: "nurtur-loki-identity".to_owned(),
                to: "telemetry.nurtur.tech".to_owned(),
            }]
        );
        assert_eq!(plan.clear, ["nurtur-loki-identity"]);
    }

    /// A credential already under the new name wins: a reader who signed in since the upgrade is
    /// not rolled back to a token from before it.
    #[test]
    fn the_plan_leaves_a_credential_that_is_already_in_the_right_place() {
        let plan = credential_plan(
            &[renamed("telemetry.nurtur.tech", &["nurtur-loki-ndc"])],
            &owned(&["telemetry.nurtur.tech", "nurtur-loki-ndc"]),
        );
        assert!(plan.moves.is_empty(), "{:?}", plan.moves);
        assert!(
            plan.clear.is_empty(),
            "and the old one is left alone rather than deleted \u{2014} it is the only copy of \
             something the reader may still want, and nothing has replaced it"
        );
    }

    /// **The case that decides this has to be planned rather than looped: a legacy entry named
    /// after another entry's host.** That is the naming convention this very version writes, so it
    /// is a shape upgraders will have.
    ///
    /// Deciding one entry at a time against the live store gets it wrong twice over. Asking whether
    /// `telemetry.nurtur.tech` is occupied answers yes — it is the name of a *different* source
    /// whose credential is about to leave — so `X`'s secret is never moved and is orphaned for
    /// ever; and had it not been skipped, writing immediately would have clobbered the credential
    /// the other entry had not read yet, carrying the wrong secret to `other.example`.
    #[test]
    fn a_legacy_entry_named_after_another_entrys_host_still_gets_its_own_credential() {
        let renames = [
            renamed("telemetry.nurtur.tech", &["X"]),
            renamed("other.example", &["telemetry.nurtur.tech"]),
        ];
        let plan = credential_plan(&renames, &owned(&["X", "telemetry.nurtur.tech"]));

        assert_eq!(
            plan.moves,
            [
                CredentialMove {
                    from: "X".to_owned(),
                    to: "telemetry.nurtur.tech".to_owned(),
                },
                CredentialMove {
                    from: "telemetry.nurtur.tech".to_owned(),
                    to: "other.example".to_owned(),
                },
            ],
            "both move, and each from its own old name"
        );
        assert_eq!(
            plan.clear,
            ["X"],
            "**and `telemetry.nurtur.tech` is never cleared**, because it is where the first move \
             lands — clearing it would delete what was just written there"
        );
    }

    /// Order in the file does not change the answer, which is the property the planning buys.
    #[test]
    fn the_plan_does_not_depend_on_the_order_of_the_file() {
        let held = owned(&["X", "telemetry.nurtur.tech"]);
        let forward = credential_plan(
            &[
                renamed("telemetry.nurtur.tech", &["X"]),
                renamed("other.example", &["telemetry.nurtur.tech"]),
            ],
            &held,
        );
        let backward = credential_plan(
            &[
                renamed("other.example", &["telemetry.nurtur.tech"]),
                renamed("telemetry.nurtur.tech", &["X"]),
            ],
            &held,
        );
        let mut a: Vec<(String, String)> = forward
            .moves
            .iter()
            .map(|m| (m.from.clone(), m.to.clone()))
            .collect();
        let mut b: Vec<(String, String)> = backward
            .moves
            .iter()
            .map(|m| (m.from.clone(), m.to.clone()))
            .collect();
        a.sort();
        b.sort();
        assert_eq!(a, b);
        assert_eq!(forward.clear, backward.clear);
    }

    /// Nothing anywhere is nothing to move, not an empty write.
    #[test]
    fn the_plan_is_empty_when_nothing_holds_a_credential() {
        let plan = credential_plan(&[renamed("now", &["was"])], &[]);
        assert!(plan.moves.is_empty());
        assert!(plan.clear.is_empty());
        assert_eq!(
            credential_plan(&[], &owned(&["anything"])),
            CredentialPlan::default()
        );
    }

    /// A source that kept its name is not in the list at all — `split_sources` drops it — but a
    /// hand-made plan that said so must not copy a credential onto itself and then delete it.
    #[test]
    fn a_move_onto_the_name_it_came_from_is_never_planned() {
        let plan = credential_plan(&[renamed("same", &["same"])], &owned(&["same"]));
        assert!(
            plan.moves.is_empty(),
            "the name is occupied by the very thing that would move: {:?}",
            plan.moves
        );
        assert!(plan.clear.is_empty());
    }
}
