//! Setting up one remote source, step by step — the decision half of the wizard `dialog.rs` draws.
//!
//! **The login mode is a fork, and that is why this exists.** The owner, 2026-10-21, on the flat
//! form that preceded it: *"I dont really understand the screen you have created, the discover
//! button is just sitting there, and all the fields next to it. that is not like the analogy I gave
//! you of setting up email where the user simply adds an email, or has an advanced option."* Then
//! the shape, and the sentence that makes it a state machine rather than four fixed pages: *"the
//! decisions about login modes would modify later screens in this 'Wizard' style dialog."*
//!
//! A source that signs its reader in through a browser needs a client id and **never** a secret.
//! One that uses a client secret needs both. A Loki that wants neither needs no credential page at
//! all. A wizard that asks everybody for everything is the form he rejected, one page at a time.
//!
//! # What this module does not do
//!
//! It opens no socket, reads no store and holds no credential beyond the one being typed. Discovery's
//! answer and a test's verdict are **given** to it, exactly as [`crate::sourceset::Editor`] is *told*
//! whether a secret exists rather than going to look. Everything here runs with no window, which is
//! the point: `CLAUDE.md` records that every dialog defect this project has had lived in a procedure
//! that could not be tested.

use crate::loki::Discovered;
use crate::settings::Source;

/// How a source proves who is asking.
///
/// These are the three shapes [`Source::fault`] already enforces, named. It refuses a client id
/// without a token URL and a sign-in URL without both, so a mode is not a fourth thing to keep in
/// step with the settings file — it is a reading of what is in it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// The reader signs in through a browser. Needs a token URL, a sign-in URL and a client id;
    /// **never** a secret, which is why the secret box has no business on a page this path reaches.
    SignIn,
    /// A client id and a stored secret, exchanged for a token. The only path with a secret page.
    ClientSecret,
    /// Nothing at all — a Loki with `auth_enabled: false`, or one behind a proxy that has already
    /// decided who is asking. No credential page whatsoever.
    Open,
}

impl Mode {
    /// Whether this mode needs a client id, which is the one value nothing can discover: it is
    /// assigned when the client is registered.
    pub fn needs_client_id(self) -> bool {
        !matches!(self, Mode::Open)
    }

    /// Whether this mode needs a secret. Exactly one of them does.
    pub fn needs_secret(self) -> bool {
        matches!(self, Mode::ClientSecret)
    }

    /// How the page offering the choice should name it.
    pub fn said(self) -> &'static str {
        match self {
            Mode::SignIn => "Sign in with a browser",
            Mode::ClientSecret => "Use a client id and secret",
            Mode::Open => "No sign-in needed",
        }
    }
}

/// What a test reached, as the wizard needs to see it.
///
/// **A mirror of the shell's own verdict, carrying no shell types.** `pull::Reached` holds a
/// `NetFault` and a `PullFault`, which are Windows-transport and shell concerns; this is the part
/// a decision can be made from, so the shell maps into it and the core stays portable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reach {
    /// 401 or 403 — there is a Loki here and it wants a credential. The answer a test hopes for.
    WantsToken,
    /// 200 — there is a Loki here and it answered without one.
    Answered,
    /// 404 — the server is there and nothing is mounted at this path.
    NothingMounted,
    /// Any other status, or nothing reached at all.
    Failed,
}

/// One page of the guided setup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// Name it, and say where the server is. The only page everybody sees first.
    Server,
    /// **The decision**, not a result screen: what discovery found, and which way this source will
    /// prove who is asking — proposed, and overridable.
    HowToSignIn,
    /// The client id, and the secret when the mode needs one. Absent entirely for [`Mode::Open`].
    Credential,
    /// Which of the server's own labels to constrain on, and what the selector comes to.
    ///
    /// **The labels are the server's, not the product's.** The owner, 2026-10-21: *"the
    /// applications field is specifically a nurtur feature so I would think we need to offer the
    /// user a set of fields and possible values that could be used to constrain the output based on
    /// an initial fetch from the source once connected. this is why the wizard works well."* `app`
    /// is one label in one estate; another Loki keys on `service_name` or `namespace`, and a form
    /// with an **Applications** box on it had quietly made this a Nurtur tool.
    WhatToShow,
    /// What it amounts to, and the chance to try it before it is saved.
    Ready,
}

impl Step {
    /// The page's heading, which is a question wherever a question is what the page is asking —
    /// `UI-DESIGN.md`'s applications picker set that pattern and the review that praised it asked
    /// for it to be used again.
    pub fn heading(self) -> &'static str {
        match self {
            Step::Server => "Which log server?",
            Step::HowToSignIn => "How should Tailhawk sign in?",
            Step::Credential => "What is this client called?",
            Step::WhatToShow => "What should this source show?",
            Step::Ready => "Ready to use",
        }
    }
}

/// One source being set up.
///
/// **A draft, not a `Source`.** A `Source` is a thing that can be used; this is a thing being made,
/// and the difference is that half its fields are legitimately empty for most of its life. It
/// becomes one through [`Draft::into_source`] once [`Draft::blocking`] has nothing left to say.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Draft {
    pub name: String,
    pub url: String,
    /// The mode in force, once the decision page has been reached. `None` means it has not.
    pub mode: Option<Mode>,
    pub token_url: String,
    pub auth_url: String,
    pub client_id: String,
    pub scope: String,
    /// The secret as typed. `None` means *leave whatever is in the store alone* — the distinction
    /// `sourceset` keeps for the same reason, and for the same credential.
    pub secret: Option<String>,
    /// What asking the server where it signs in came back with, when it has been asked.
    pub found: Option<Discovered>,
    /// What a test reached, when one has been run.
    pub reached: Option<Reach>,
    /// Whether the reader chose the mode themselves, rather than it being proposed.
    ///
    /// **Without this, nothing a server says can ever change the mode.** The dialog writes the
    /// combo's selection into `mode` on every keystroke, so `mode` is `Some` long before any probe
    /// runs — and a check for `mode.is_none()` is then a check that is never true. A Loki with
    /// `auth_enabled: false` would answer *"Reached it, and it answered without a token"* and then
    /// be refused for having no token URL, with the reader left to find **No sign-in needed** in
    /// the advanced half by themselves.
    pub mode_chosen: bool,
}

impl Draft {
    /// A draft of a source that already exists, so **Edit** and **New** are one screen with one
    /// state machine behind them.
    pub fn of(source: &Source) -> Self {
        Self {
            name: source.name.clone(),
            url: source.url.clone(),
            mode: Some(mode_of(source)),
            token_url: source.token_url.clone(),
            auth_url: source.auth_url.clone(),
            client_id: source.client_id.clone(),
            scope: source.scope.clone(),
            secret: None,
            found: None,
            reached: None,
            // **A source that exists is in the mode it is in, not one proposed to it.** Editing
            // `live` and pressing `Set up` must not move it off the browser flow because its
            // server happens to advertise a token endpoint as well.
            mode_chosen: true,
        }
    }

    /// What discovery found, written into the draft and proposing a mode.
    ///
    /// **It proposes rather than decides**, and only into fields the reader has not filled: the
    /// whole point of the page it lands on is that the answer can be overruled. The scope is offered
    /// only into an empty box, which is `settings::authorize_beside`'s rule for the same reason.
    pub fn discovered(&mut self, found: Discovered) {
        self.token_url = found.token_url.clone();
        // Empty is meaningful here — `Source::signs_in` is `!auth_url.is_empty()` — so a server
        // that advertises no authorization endpoint must not blank one the reader typed.
        if !found.auth_url.is_empty() {
            self.auth_url = found.auth_url.clone();
        }
        if self.scope.trim().is_empty() {
            self.scope = crate::loki::scope_from(&found);
        }
        // **Proposes, which is what the doc has always said.** A reader who has opened the advanced
        // half and picked a mode has answered this question already, and a button that goes and
        // asks the server must not overrule them.
        if !self.mode_chosen {
            self.mode = Some(match found.auth_url.is_empty() {
                false => Mode::SignIn,
                true => Mode::ClientSecret,
            });
        }
        self.found = Some(found);
    }

    /// What a test reached. A server that answers without a token is the one verdict that can
    /// *decide* a mode on its own, and only when nothing else has.
    pub fn tested(&mut self, reach: Reach) {
        if reach == Reach::Answered && self.found.is_none() && !self.mode_chosen {
            self.mode = Some(Mode::Open);
        }
        self.reached = Some(reach);
    }

    /// The mode in force, or what would be proposed if the decision page were reached now.
    pub fn mode_now(&self) -> Mode {
        self.mode.unwrap_or(Mode::SignIn)
    }

    /// The pages this draft will actually have, in order.
    ///
    /// **Computed, so the wizard can say "2 of 4" and mean it.** A credential page that is going to
    /// be skipped was never one of the steps, and a reader counting pages is entitled to the truth
    /// about how much is left.
    pub fn steps(&self) -> Vec<Step> {
        let mut steps = vec![Step::Server, Step::HowToSignIn];
        if self.mode_now() != Mode::Open {
            steps.push(Step::Credential);
        }
        steps.push(Step::WhatToShow);
        steps.push(Step::Ready);
        steps
    }

    /// Why **Next** cannot be pressed on `step`, or `None` when it can.
    ///
    /// **Each page asks only about itself.** A reader held on page one by a complaint about a scope
    /// they have not been shown yet is the flat form again, one page at a time.
    pub fn blocking(&self, step: Step) -> Option<&'static str> {
        match step {
            Step::Server => {
                if self.name.trim().is_empty() {
                    return Some("Give this source a name.");
                }
                if self.name.trim() != self.name {
                    return Some("The name cannot start or end with a space.");
                }
                if !nameable(&self.name) {
                    return Some("The name can hold letters, digits, spaces, - _ and . only.");
                }
                if self.url.trim().is_empty() {
                    return Some("Give this source the Loki server's URL.");
                }
                if !self
                    .url
                    .trim_start()
                    .to_ascii_lowercase()
                    .starts_with("https://")
                {
                    return Some("The URL must begin with https://.");
                }
                None
            }
            Step::HowToSignIn => match self.mode_now() {
                Mode::Open => None,
                mode => {
                    if self.token_url.trim().is_empty() {
                        return Some("A sign-in needs the token URL. Set it up, or type it here.");
                    }
                    if mode == Mode::SignIn && self.auth_url.trim().is_empty() {
                        return Some("Signing in with a browser needs the sign-in URL as well.");
                    }
                    None
                }
            },
            Step::Credential => {
                if self.mode_now().needs_client_id() && self.client_id.trim().is_empty() {
                    return Some("The client id is the one thing the server cannot tell us.");
                }
                // **Absent is not empty.** `None` means the store already holds one and nothing was
                // typed, which is the state editing an existing source starts in; `Some("")` means
                // the reader cleared it, which for this mode is a source that cannot authenticate.
                if self.mode_now().needs_secret() && self.secret.as_deref() == Some("") {
                    return Some("This way of signing in needs a secret.");
                }
                None
            }
            Step::WhatToShow | Step::Ready => None,
        }
    }

    /// The first thing wrong with this draft, across every page it has.
    ///
    /// **What the flat editor needs and the wizard must not use.** A wizard page asks about itself
    /// — [`Self::blocking`] — because holding a reader on page one over a client id they have not
    /// been shown is the wall of fields all over again. A form shows every box at once, so it owes
    /// them every answer at once.
    pub fn fault(&self) -> Option<&'static str> {
        self.steps()
            .into_iter()
            .find_map(|step| self.blocking(step))
            // **And then the authority itself.** `blocking` is written per page and had drifted:
            // it checked the Loki URL's scheme and not the token or sign-in URLs', and its check
            // was case-insensitive where `Source::fault`'s is not. So the editor accepted a source
            // the *list* then refused — and the list has no fields, so the reader was held there by
            // a complaint about a box that was not on the screen in front of them, with the editor
            // accepting the same value every time they reopened it. Asking the authority last costs
            // one conversion and makes that disagreement impossible.
            .or_else(|| self.into_source().fault())
    }

    /// The source this draft amounts to.
    ///
    /// **The mode decides what is carried, not what happens to be in the boxes.** A reader who tried
    /// the browser path, went back and chose a client secret would otherwise take a sign-in URL with
    /// them — and `Source::signs_in` reads exactly that field, so the source would use a flow they
    /// had just rejected.
    pub fn into_source(&self) -> Source {
        let mode = self.mode_now();
        Source {
            name: self.name.trim().to_owned(),
            url: self.url.trim().to_owned(),
            token_url: match mode {
                Mode::Open => String::new(),
                _ => self.token_url.trim().to_owned(),
            },
            auth_url: match mode {
                Mode::SignIn => self.auth_url.trim().to_owned(),
                _ => String::new(),
            },
            client_id: match mode.needs_client_id() {
                true => self.client_id.trim().to_owned(),
                false => String::new(),
            },
            scope: match mode {
                Mode::Open => String::new(),
                _ => self.scope.trim().to_owned(),
            },
        }
    }

    /// Whether the server can be asked what labels it holds, which is what [`Step::WhatToShow`]
    /// needs before it can offer anything.
    ///
    /// **This is the wizard's whole advantage over a form.** A label list costs a credential, and on
    /// a flat form the moment the reader fills in what to show is a moment when nothing knows
    /// whether the source can be reached at all — so the box has to be free text and the reader has
    /// to already know the answer. By this page the mode is settled and the credential is in hand,
    /// so there is something to ask. When there is not, the page says so and takes text.
    pub fn can_ask_server(&self) -> bool {
        if self.blocking(Step::Server).is_some() {
            return false;
        }
        match self.mode_now() {
            // Nothing to obtain: a Loki that answers without a token can be asked immediately.
            Mode::Open => true,
            // A secret in the store or in the box, and the client it belongs to.
            Mode::ClientSecret => {
                !self.client_id.trim().is_empty() && self.secret.as_deref() != Some("")
            }
            // **A browser sign-in has to have actually happened**, and nothing on this side can
            // tell whether it has — the shell holds the token. It answers by asking, which is why
            // this is the one mode where the page may still find itself with nothing to show.
            Mode::SignIn => !self.client_id.trim().is_empty() && !self.auth_url.trim().is_empty(),
        }
    }

    /// The secret to store, and the name to store it under — or `None` when nothing was typed or
    /// this mode has no use for one.
    ///
    /// **A mode that does not use a secret hands none over**, so switching a source to the browser
    /// flow cannot leave a client secret behind in the store for a flow that will never send it.
    pub fn secret_to_store(&self) -> Option<(String, String)> {
        let secret = self.secret.clone()?;
        self.mode_now()
            .needs_secret()
            .then(|| (self.name.trim().to_owned(), secret))
    }

    /// The name whose stored secret should be **deleted**, if any.
    ///
    /// **Handing none over is what leaves one behind.** [`Self::secret_to_store`] returning `None`
    /// for a mode with no use for a secret was described as preventing a credential being stranded
    /// in the store; it is what causes it. A source switched from a client secret to a browser
    /// sign-in kept its secret in Credential Manager indefinitely, for a flow that will never send
    /// it — and nothing on screen said so.
    ///
    /// `stored` is the shell's answer to whether there is one, carried in rather than looked up,
    /// for the reason this whole module takes its facts as arguments.
    pub fn secret_to_forget(&self, stored: bool) -> Option<String> {
        (stored && !self.mode_now().needs_secret()).then(|| self.name.trim().to_owned())
    }
}

/// Which of the three shapes a source already in the settings file is in.
pub fn mode_of(source: &Source) -> Mode {
    if !source.auth_url.trim().is_empty() {
        return Mode::SignIn;
    }
    if source.token_url.trim().is_empty() && source.client_id.trim().is_empty() {
        return Mode::Open;
    }
    Mode::ClientSecret
}

/// What the decision page proposes, given what asking the server produced.
///
/// **A server that answers without a token settles it on its own**; everything else is read off the
/// discovery document, and when there is none the proposal is the commonest shape rather than a
/// shrug — the reader is on a page that exists to be overruled.
pub fn mode_proposed(found: Option<&Discovered>, reached: Option<Reach>) -> Mode {
    match (found, reached) {
        (Some(found), _) if !found.auth_url.is_empty() => Mode::SignIn,
        (Some(_), _) => Mode::ClientSecret,
        (None, Some(Reach::Answered)) => Mode::Open,
        _ => Mode::SignIn,
    }
}

/// The characters a source's name may hold — the same set `Source::fault` holds it to, because the
/// name is also the key its credential is filed under.
fn nameable(name: &str) -> bool {
    name.chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ' '))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn discovered(auth: &str) -> Discovered {
        Discovered {
            issuer: "https://identity.example".to_owned(),
            token_url: "https://identity.example/connect/token".to_owned(),
            auth_url: auth.to_owned(),
            // `openid` among them, as his own identity server advertises — `scope_from` leads with
            // it on the browser path and leaves it out of a client-credentials one, so a fixture
            // without it would have exercised neither rule.
            scopes: vec![
                "openid".to_owned(),
                "profile".to_owned(),
                "telemetry:read".to_owned(),
                "offline_access".to_owned(),
            ],
            pkce_s256: true,
        }
    }

    fn named(name: &str) -> Draft {
        Draft {
            name: name.to_owned(),
            url: "https://telemetry.example/loki".to_owned(),
            ..Draft::default()
        }
    }

    /// **The mode changes which pages there are, which is the whole reason this is not a form.**
    /// The owner, 2026-10-21: *"the decisions about login modes would modify later screens in this
    /// 'Wizard' style dialog."*
    #[test]
    fn the_login_mode_decides_which_pages_the_reader_sees() {
        let mut draft = named("live");
        draft.mode = Some(Mode::SignIn);
        assert_eq!(
            draft.steps(),
            [
                Step::Server,
                Step::HowToSignIn,
                Step::Credential,
                Step::WhatToShow,
                Step::Ready
            ]
        );

        draft.mode = Some(Mode::Open);
        assert_eq!(
            draft.steps(),
            [
                Step::Server,
                Step::HowToSignIn,
                Step::WhatToShow,
                Step::Ready
            ],
            "a server that wants nothing is not asked for a client id"
        );
        assert!(
            !draft.steps().contains(&Step::Credential),
            "and the page it would have been is not counted either"
        );
    }

    /// **Each page complains about itself and nothing else.** Being held on page one by a
    /// complaint about a scope you have not been shown is the flat form again, one page at a time.
    #[test]
    fn a_page_refuses_next_only_for_what_that_page_asked() {
        let mut draft = Draft::default();
        assert_eq!(
            draft.blocking(Step::Server),
            Some("Give this source a name.")
        );
        draft.name = "live ".to_owned();
        assert_eq!(
            draft.blocking(Step::Server),
            Some("The name cannot start or end with a space.")
        );
        draft.name = "live/dev".to_owned();
        assert!(draft
            .blocking(Step::Server)
            .is_some_and(|s| s.contains("letters")));
        draft.name = "live".to_owned();
        assert_eq!(
            draft.blocking(Step::Server),
            Some("Give this source the Loki server's URL.")
        );
        draft.url = "http://telemetry.example/loki".to_owned();
        assert_eq!(
            draft.blocking(Step::Server),
            Some("The URL must begin with https://.")
        );
        draft.url = "https://telemetry.example/loki".to_owned();
        assert_eq!(draft.blocking(Step::Server), None, "and page one is done");

        // Page one says nothing about a client id, although the finished source will need one.
        draft.mode = Some(Mode::SignIn);
        assert_eq!(draft.blocking(Step::Server), None);
        assert!(draft.blocking(Step::Credential).is_some());
    }

    /// **A browser sign-in is never asked for a secret, and a client-secret source always is.**
    #[test]
    fn the_credential_page_asks_for_what_the_mode_actually_needs() {
        let mut draft = named("live");
        draft.discovered(discovered("https://identity.example/connect/authorize"));
        assert_eq!(draft.mode, Some(Mode::SignIn));
        assert!(!draft.mode_now().needs_secret());
        draft.client_id = "tailhawk".to_owned();
        assert_eq!(draft.blocking(Step::Credential), None, "no secret wanted");

        // The same server with no authorization endpoint is the other path entirely.
        let mut draft = named("live");
        draft.discovered(discovered(""));
        assert_eq!(draft.mode, Some(Mode::ClientSecret));
        draft.client_id = "tailhawk".to_owned();
        assert_eq!(
            draft.blocking(Step::Credential),
            None,
            "an untyped secret is the store's, not an empty one"
        );
        draft.secret = Some(String::new());
        assert_eq!(
            draft.blocking(Step::Credential),
            Some("This way of signing in needs a secret."),
            "cleared is not the same as untouched"
        );
        draft.secret = Some("shh".to_owned());
        assert_eq!(draft.blocking(Step::Credential), None);
    }

    /// **Every path produces a source the product will accept**, which is the test that says the
    /// wizard cannot deliver something `Source::fault` then refuses.
    #[test]
    fn every_path_ends_in_a_source_with_no_fault() {
        let mut browser = named("live");
        browser.discovered(discovered("https://identity.example/connect/authorize"));
        browser.client_id = "tailhawk".to_owned();
        let source = browser.into_source();
        assert_eq!(source.fault(), None, "{:?}", source.fault());
        assert!(source.signs_in());
        assert_eq!(source.scope, "openid telemetry:read offline_access");

        let mut secret = named("dev");
        secret.discovered(discovered(""));
        secret.client_id = "tailhawk".to_owned();
        secret.secret = Some("shh".to_owned());
        let source = secret.into_source();
        assert_eq!(source.fault(), None, "{:?}", source.fault());
        assert!(!source.signs_in());
        assert_eq!(
            source.scope, "telemetry:read offline_access",
            "and no openid on a client-credentials exchange, where it means nothing"
        );
        assert_eq!(
            secret.secret_to_store(),
            Some(("dev".to_owned(), "shh".to_owned()))
        );

        let mut open = named("lab");
        open.tested(Reach::Answered);
        assert_eq!(open.mode, Some(Mode::Open));
        let source = open.into_source();
        assert_eq!(source.fault(), None, "{:?}", source.fault());
        assert!(source.token_url.is_empty() && source.client_id.is_empty());
        assert_eq!(
            open.secret_to_store(),
            None,
            "nothing to store for a mode with no secret"
        );
    }

    /// **Changing your mind leaves nothing of the path not taken.** `Source::signs_in` is
    /// `!auth_url.is_empty()`, so a sign-in URL carried into a client-secret source would make it
    /// use the flow the reader had just rejected.
    #[test]
    fn overruling_the_proposal_takes_the_other_paths_fields_with_it() {
        let mut draft = named("live");
        draft.discovered(discovered("https://identity.example/connect/authorize"));
        draft.client_id = "tailhawk".to_owned();
        assert!(draft.into_source().signs_in());

        draft.mode = Some(Mode::ClientSecret);
        draft.secret = Some("shh".to_owned());
        let source = draft.into_source();
        assert!(!source.signs_in(), "the sign-in URL did not come along");
        assert!(source.auth_url.is_empty());
        assert_eq!(source.token_url, "https://identity.example/connect/token");

        draft.mode = Some(Mode::Open);
        let source = draft.into_source();
        assert_eq!(source.fault(), None);
        assert!(source.token_url.is_empty(), "nor did the token URL");
        assert!(source.scope.is_empty());
        assert_eq!(
            draft.secret_to_store(),
            None,
            "and a typed secret is not filed against a source that will never send one"
        );
    }

    /// **Discovery proposes; it does not overwrite what the reader has said.**
    #[test]
    fn what_was_found_fills_the_empty_boxes_and_not_the_full_ones() {
        let mut draft = named("live");
        draft.scope = "telemetry:read".to_owned();
        draft.auth_url = "https://typed.example/authorize".to_owned();
        draft.discovered(discovered(""));
        assert_eq!(
            draft.scope, "telemetry:read",
            "a scope that was typed stands"
        );
        assert_eq!(
            draft.auth_url, "https://typed.example/authorize",
            "and a server that advertises no sign-in URL does not blank one"
        );
    }

    /// An existing source opens its editor already in the mode it is in, so **Edit** and **New**
    /// are one screen.
    #[test]
    fn a_source_that_exists_reads_back_as_the_mode_it_is_in() {
        let browser = Source {
            name: "live".to_owned(),
            url: "https://telemetry.example/loki".to_owned(),
            token_url: "https://identity.example/connect/token".to_owned(),
            auth_url: "https://identity.example/connect/authorize".to_owned(),
            client_id: "tailhawk".to_owned(),
            scope: "telemetry:read".to_owned(),
        };
        assert_eq!(mode_of(&browser), Mode::SignIn);
        let draft = Draft::of(&browser);
        assert_eq!(draft.mode, Some(Mode::SignIn));
        assert_eq!(
            draft.secret, None,
            "editing one does not reach into the store"
        );
        assert_eq!(draft.into_source(), browser, "and it round-trips untouched");

        let open = Source {
            token_url: String::new(),
            auth_url: String::new(),
            client_id: String::new(),
            scope: String::new(),
            ..browser.clone()
        };
        assert_eq!(mode_of(&open), Mode::Open);

        let secret = Source {
            auth_url: String::new(),
            ..browser
        };
        assert_eq!(mode_of(&secret), Mode::ClientSecret);
    }

    /// **The page that asks what to show can only offer the server's own labels once there is a
    /// way to ask**, and the wizard is what makes that true by the time the page is reached.
    #[test]
    fn what_to_show_knows_whether_the_server_can_be_asked_yet() {
        let mut draft = Draft::default();
        assert!(!draft.can_ask_server(), "nothing is configured at all");

        draft = named("lab");
        draft.tested(Reach::Answered);
        assert!(
            draft.can_ask_server(),
            "a Loki that answers without a token can be asked straight away"
        );

        let mut secret = named("dev");
        secret.discovered(discovered(""));
        assert!(!secret.can_ask_server(), "no client id yet");
        secret.client_id = "tailhawk".to_owned();
        assert!(
            secret.can_ask_server(),
            "a secret already in the store is a secret"
        );
        secret.secret = Some(String::new());
        assert!(!secret.can_ask_server(), "and one cleared is not");

        let mut browser = named("live");
        browser.discovered(discovered("https://identity.example/connect/authorize"));
        assert!(!browser.can_ask_server());
        browser.client_id = "tailhawk".to_owned();
        assert!(browser.can_ask_server(), "as far as this side can tell");

        // A draft whose own first page is unfinished cannot be asked anything, whatever else is set.
        browser.url = "http://telemetry.example/loki".to_owned();
        assert!(!browser.can_ask_server());
    }

    /// **A server that needs nothing settles it, and a reader who has chosen is not overruled.**
    /// The dialog writes the mode on every keystroke, so a check for "no mode yet" is a check that
    /// is never true — a Loki with `auth_enabled: false` answered *"it answered without a token"*
    /// and was then refused for having no token URL.
    #[test]
    fn what_the_server_said_decides_the_mode_until_the_reader_decides_it() {
        let mut draft = named("lab");
        // As the dialog leaves it: a mode is always set, because the combo always has a selection.
        draft.mode = Some(Mode::SignIn);
        draft.tested(Reach::Answered);
        assert_eq!(
            draft.mode,
            Some(Mode::Open),
            "a server that answers without a token has said what it needs"
        );

        // Once the reader has picked one, nothing overrules it.
        let mut chosen = named("lab");
        chosen.mode = Some(Mode::ClientSecret);
        chosen.mode_chosen = true;
        chosen.tested(Reach::Answered);
        assert_eq!(chosen.mode, Some(Mode::ClientSecret));
        chosen.discovered(discovered("https://identity.example/connect/authorize"));
        assert_eq!(
            chosen.mode,
            Some(Mode::ClientSecret),
            "discovery fills the endpoints in but does not take the decision back"
        );
        assert_eq!(
            chosen.token_url, "https://identity.example/connect/token",
            "and it did fill them in"
        );
    }

    /// **The editor cannot accept what the list will refuse.** `blocking` is written per page and
    /// drifted from `Source::fault`: it checked the Loki URL's scheme and neither of the other two,
    /// and did it case-insensitively where the authority does not. A reader who typed
    /// `http://id/token` into the advanced half was accepted here and then held in the Remote
    /// sources dialog by a complaint about a box that dialog does not have.
    #[test]
    fn the_draft_defers_to_the_authority_on_what_a_usable_source_is() {
        let mut draft = named("live");
        draft.discovered(discovered("https://identity.example/connect/authorize"));
        draft.client_id = "tailhawk".to_owned();
        assert_eq!(draft.fault(), None);

        for (what, set) in [
            ("a token URL", 0),
            ("a sign-in URL", 1),
            ("the server URL", 2),
        ] {
            let mut bad = draft.clone();
            match set {
                0 => bad.token_url = "http://identity.example/connect/token".to_owned(),
                1 => bad.auth_url = "http://identity.example/connect/authorize".to_owned(),
                _ => bad.url = "HTTPS://telemetry.example/loki".to_owned(),
            }
            assert!(
                bad.fault().is_some(),
                "{what} that is not https is refused here, not two dialogs later"
            );
            assert_eq!(
                bad.fault(),
                bad.into_source().fault(),
                "{what}: and in the authority's own words"
            );
        }
    }

    /// What the decision page offers before anybody has touched it.
    #[test]
    fn the_proposal_follows_what_the_server_said() {
        assert_eq!(
            mode_proposed(Some(&discovered("https://x/authorize")), None),
            Mode::SignIn
        );
        assert_eq!(
            mode_proposed(Some(&discovered("")), None),
            Mode::ClientSecret
        );
        assert_eq!(
            mode_proposed(None, Some(Reach::Answered)),
            Mode::Open,
            "a Loki that answered without a token has settled it"
        );
        assert_eq!(
            mode_proposed(None, Some(Reach::WantsToken)),
            Mode::SignIn,
            "and one that wants a token needs a choice, offered at the commonest shape"
        );
        assert_eq!(mode_proposed(None, None), Mode::SignIn);
    }
}
