//! Choosing which applications a remote source shows — the owner's ask of 2026-08-31.
//!
//! > "Choose multiple apps such as `nurtur-identity-server` and either interleave them or run each
//! > in a separate log window."
//!
//! Both halves of that already worked in the sense that mattered least: a source's `query` is raw
//! LogQL, so a person who knows LogQL could type `{environment="live", app=~"a|b"}` into the source
//! editor and get exactly it. What was missing is the part a person should not have to do — knowing
//! which values exist, spelling them correctly, and writing the alternation by hand for seventy
//! services whose names differ by a hyphen.
//!
//! ## The selector is rewritten, not replaced
//!
//! **A source's query belongs to the user.** It carries the environment they chose, whatever other
//! labels they narrowed by, and any pipeline stages after the selector. So an app choice is *merged*
//! into it: [`with_apps`] finds the **first** `{...}` -- a metric query with two of them gets the
//! app matcher in the first -- splits its matchers, drops whatever `app` matcher was there, and
//! writes one in its place. Every other matcher survives, as does whatever pipeline follows the
//! selector; only the spacing between matchers is renormalised.
//!
//! That also makes the two things the owner asked for the same function called differently:
//! *interleaved* is one call with every chosen name, and *one window each* is one call per name.
//! There is no second code path for the second behaviour, which is why neither can drift from the
//! other.
//!
//! ## What this deliberately does not do
//!
//! - **It does not parse LogQL.** It finds one balanced `{…}` and splits its contents on commas
//!   that are outside quotes. A selector that is not well-formed is refused rather than guessed at,
//!   because a half-understood selector sent to a server is how a user ends up reading the wrong
//!   log without knowing it.
//! - **It does not know which apps exist.** The values come from the server through
//!   [`crate::loki::label_values_from_json`]; nothing here invents, caches or filters them by what
//!   has already been seen in a spill.

/// The most label values a picker will hold.
///
/// The estate this was written against has about seventy `app` values and Loki will answer with
/// every value it has seen in the window, which on a shared deployment is a number nobody controls.
/// A dialog that has to render ten thousand rows is a dialog that stops answering, so the list is
/// cut here and [`AppList::cut`] says it was — §6's rule that what is missing is said.
pub const MAX_APPS: usize = 512;

/// The most documents *Separate windows* will open at once.
///
/// **Each one is a tail**: its own five-second poll against a shared server, its own worker, and its
/// own bounded spill of up to `stdin::SPILL_BYTES`. Ticking every application in an estate of
/// seventy and asking for a window each is therefore not a big request — it is seventy tails, some
/// gigabytes of `%TEMP%` and fourteen queries a second on somebody else's Loki, from one keystroke.
/// Eight is what a person can actually watch at once.
pub const MAX_SEPARATE: usize = 8;

/// Why *Separate windows* will not open this many, or `None` when it will.
///
/// **A refusal with the remedy in it, rather than opening the first eight.** Silently truncating a
/// choice is the failure this project keeps naming: the user would get eight windows, believe they
/// had asked for eight, and never learn which sixty-two were missing.
pub fn too_many_windows(chosen: usize) -> Option<String> {
    (chosen > MAX_SEPARATE).then(|| {
        format!(
            "{chosen} applications is more than Tailhawk will open in separate windows at once \
             ({MAX_SEPARATE}), because each one is a tail of its own. Tick fewer, or use Interleave \
             to see them in one window."
        )
    })
}

/// Why a selector could not be rewritten.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectorFault {
    /// There is no `{` in the query at all. A Loki query needs a stream selector; a source whose
    /// query is empty gets one written for it, but a source whose query is `| json` is malformed
    /// and this is not the place to decide what it meant.
    NoSelector,
    /// A `{` with no matching `}`, or a quote that never closes.
    Unbalanced,
    /// A chosen name has a character that cannot appear inside a quoted LogQL label value without
    /// escaping — a quote or a backslash. Values come from the server, so this is a server that
    /// answered with something strange rather than a user who typed it, and it is refused rather
    /// than escaped: a value nobody can type is a value the picker should not offer.
    UnquotableName,
}

/// The selector `query` becomes when it is asked for exactly `apps`.
///
/// An empty `apps` leaves the query alone — "no app chosen" means the source's own selector, not an
/// empty one. One app becomes `app="that"`, several become `app=~"a|b"`, because an exact matcher is
/// what a reader expects to see for a single choice and Loki indexes it better.
///
/// A query with no selector at all is refused unless it is *empty*, which is a legitimate source
/// meaning "everything this credential can see" — that gets a selector written for it.
pub fn with_apps(query: &str, apps: &[&str]) -> Result<String, SelectorFault> {
    if apps.is_empty() {
        return Ok(query.to_owned());
    }
    for app in apps {
        if app.contains('"') || app.contains('\\') {
            return Err(SelectorFault::UnquotableName);
        }
    }
    let matcher = app_matcher(apps);

    if query.trim().is_empty() {
        return Ok(format!("{{{matcher}}}"));
    }
    let (open, close) = selector_span(query)?;
    let inside = &query[open + 1..close];
    let mut kept: Vec<&str> = Vec::new();
    for part in split_matchers(inside)? {
        let part = part.trim();
        if part.is_empty() || names_app(part) {
            continue;
        }
        kept.push(part);
    }
    kept.push(&matcher);
    Ok(format!(
        "{}{{{}}}{}",
        &query[..open],
        kept.join(", "),
        &query[close + 1..]
    ))
}

/// `app="one"` for a single choice, `app=~"one|two"` for several.
fn app_matcher(apps: &[&str]) -> String {
    if let [only] = apps {
        return format!("app=\"{only}\"");
    }
    // **The names are joined, not escaped into a regex.** A Loki label value is matched by RE2 and
    // a name carrying `.` or `+` would match more than itself — but these names come from the
    // server's own label values, and anchoring each alternative is what keeps `api` from matching
    // `api-v2`. Loki anchors the whole match, so alternation of literals is exact as long as no
    // alternative carries a metacharacter; the ones that could are quoted here.
    let joined = apps
        .iter()
        .map(|app| quote_meta(app))
        .collect::<Vec<_>>()
        .join("|");
    format!("app=~\"{joined}\"")
}

/// The characters that mean something to RE2 and must be escaped to match themselves.
///
/// `|` is here because it separates alternatives: a name carrying one would otherwise become two.
fn is_meta(c: char) -> bool {
    matches!(
        c,
        '.' | '+' | '*' | '?' | '(' | ')' | '|' | '[' | ']' | '{' | '}' | '^' | '$' | '\\'
    )
}

/// Escapes a name so it matches itself, **in the query text** rather than in the value.
///
/// **Two backslashes, and the reason is a layer nobody sees until Loki answers 400.** A matcher's
/// value is a Go-quoted string: the parser unquotes it *before* the result is compiled as a regex,
/// and `\.` is not a valid Go escape — a query carrying it is refused with a parse error, which
/// reaches the user as "Loki answered 400" with nothing to say that Tailhawk wrote the query. So
/// the text needs `\\.`, which unquotes to `\.`, which is the regex for a literal dot.
///
/// Missed in review-by-reading and caught by a reviewer who knew the lexer; the test that named the
/// property could not catch it, because it asserted the string this produces rather than that a
/// server would accept it.
fn quote_meta(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        if is_meta(c) {
            out.push_str("\\\\");
        }
        out.push(c);
    }
    out
}

/// The literal names an alternation stands for, or `None` if it is a pattern rather than a list.
///
/// The exact inverse of [`quote_meta`], and it has to be: what it cannot read back as a list of
/// names is a regex the user wrote, and a picker that ticked boxes for `api.*` would be claiming
/// the user chose two applications when they chose a pattern.
fn alternatives(value: &str) -> Option<Vec<String>> {
    let mut names = Vec::new();
    let mut name = String::new();
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                // Written by `quote_meta` as two backslashes and one metacharacter. Anything else
                // is an escape this did not write, so the value is not a list of names.
                if chars.next()? != '\\' {
                    return None;
                }
                let escaped = chars.next()?;
                if !is_meta(escaped) {
                    return None;
                }
                name.push(escaped);
            }
            '|' => names.push(std::mem::take(&mut name)),
            c if is_meta(c) => return None,
            c => name.push(c),
        }
    }
    names.push(name);
    names.retain(|name| !name.is_empty());
    Some(names)
}

/// Sets each row's tick from `ticks`, positionally, and reports whether anything changed.
///
/// **The decision behind the dialog's read-back**, so that the shell only carries ticks out of a
/// list view and hands them here. A `ticks` shorter than the list leaves the rest alone, which is
/// the case that matters: a list view part-way through being filled has fewer items than the model
/// has rows, and reading a missing row as "unticked" is how a pre-ticked selection disappears
/// before the user ever sees it.
pub fn sync(list: &mut AppList, ticks: &[bool]) -> bool {
    let mut changed = false;
    for (row, ticked) in list.rows.iter_mut().zip(ticks) {
        if row.chosen != *ticked {
            row.chosen = *ticked;
            changed = true;
        }
    }
    changed
}

/// Whether one matcher is about the `app` label — `app="x"`, `app =~ "y"`, `app!="z"`.
fn names_app(matcher: &str) -> bool {
    let name: String = matcher
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect();
    name == "app"
}

/// The byte positions of the outermost `{` and its matching `}`.
fn selector_span(query: &str) -> Result<(usize, usize), SelectorFault> {
    let bytes = query.as_bytes();
    let open = bytes
        .iter()
        .position(|b| *b == b'{')
        .ok_or(SelectorFault::NoSelector)?;
    let mut quoted = false;
    let mut escaped = false;
    for (at, b) in bytes.iter().enumerate().skip(open + 1) {
        match b {
            _ if escaped => escaped = false,
            b'\\' if quoted => escaped = true,
            b'"' => quoted = !quoted,
            b'}' if !quoted => return Ok((open, at)),
            _ => {}
        }
    }
    Err(SelectorFault::Unbalanced)
}

/// The matchers inside a selector, split on the commas that are not inside a quoted value.
fn split_matchers(inside: &str) -> Result<Vec<&str>, SelectorFault> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut quoted = false;
    let mut escaped = false;
    for (at, c) in inside.char_indices() {
        match c {
            _ if escaped => escaped = false,
            '\\' if quoted => escaped = true,
            '"' => quoted = !quoted,
            ',' if !quoted => {
                parts.push(&inside[start..at]);
                start = at + 1;
            }
            _ => {}
        }
    }
    if quoted {
        return Err(SelectorFault::Unbalanced);
    }
    parts.push(&inside[start..]);
    Ok(parts)
}

/// One offered application, and whether the user has chosen it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppRow {
    pub name: String,
    pub chosen: bool,
}

/// The picker as one frame should draw it — `UI-DESIGN.md`'s view-model shape, and the reason none
/// of this needs a window to test.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AppList {
    rows: Vec<AppRow>,
    /// Whether the server offered more values than [`MAX_APPS`], so the dialog can say so.
    cut: bool,
}

impl AppList {
    /// Builds the list from the values the server gave, in the order it gave them, with any that
    /// the source's query already names pre-chosen.
    ///
    /// **Duplicates are dropped and blanks refused.** Loki returns the values of one label and has
    /// no reason to repeat itself, but a picker is a place where a repeated row reads as two
    /// different services, and an empty name reads as a service with no name.
    pub fn new(values: &[String], already: &[String]) -> AppList {
        let mut rows: Vec<AppRow> = Vec::new();
        let mut cut = false;
        for value in values {
            let name = value.trim();
            if name.is_empty() || rows.iter().any(|row| row.name == name) {
                continue;
            }
            if rows.len() == MAX_APPS {
                cut = true;
                break;
            }
            rows.push(AppRow {
                name: name.to_owned(),
                chosen: already.iter().any(|a| a == name),
            });
        }
        AppList { rows, cut }
    }

    pub fn rows(&self) -> &[AppRow] {
        &self.rows
    }

    /// Whether the server had more to offer than the list holds. §6: what is missing is said.
    pub fn cut(&self) -> bool {
        self.cut
    }

    /// Turns the choice at `at` on or off. Out of range is ignored rather than panicking: a list
    /// redrawn while a click was in flight is a real sequence, not a bug in the caller.
    pub fn toggle(&mut self, at: usize) {
        if let Some(row) = self.rows.get_mut(at) {
            row.chosen = !row.chosen;
        }
    }

    /// Every chosen name, in the order the server offered them.
    pub fn chosen(&self) -> Vec<&str> {
        self.rows
            .iter()
            .filter(|row| row.chosen)
            .map(|row| row.name.as_str())
            .collect()
    }

    /// Whether anything is chosen. What the dialog's buttons are enabled by.
    pub fn any(&self) -> bool {
        self.rows.iter().any(|row| row.chosen)
    }

    /// Chooses everything, or nothing — the "all" box a seventy-row list needs.
    pub fn choose_all(&mut self, chosen: bool) {
        for row in &mut self.rows {
            row.chosen = chosen;
        }
    }
}

/// The apps a query already names, so reopening a picker shows what was chosen last time.
///
/// Only an exact `app="x"` or an alternation `app=~"a|b"` is read back; anything else — a negation,
/// a pattern with real regex in it — is left alone and reported as nothing chosen, because showing
/// a tick beside `api` for a query that says `app!="api"` would be a lie about the user's own
/// selector.
pub fn apps_in(query: &str) -> Vec<String> {
    let Ok((open, close)) = selector_span(query) else {
        return Vec::new();
    };
    let Ok(parts) = split_matchers(&query[open + 1..close]) else {
        return Vec::new();
    };
    for part in parts {
        let part = part.trim();
        if !names_app(part) {
            continue;
        }
        let rest = part[3..].trim_start();
        let (exact, value) = if let Some(rest) = rest.strip_prefix("=~") {
            (false, rest.trim())
        } else if let Some(rest) = rest.strip_prefix('=') {
            (true, rest.trim())
        } else {
            continue;
        };
        let Some(value) = value
            .strip_prefix('"')
            .and_then(|value| value.strip_suffix('"'))
        else {
            continue;
        };
        if exact {
            return vec![value.to_owned()];
        }
        // **A pattern is the user's and is not read back as a set of ticks.** `alternatives` is
        // the exact inverse of what `quote_meta` writes, so anything it cannot read is a regex
        // somebody typed rather than a list this produced.
        return alternatives(value).unwrap_or_default();
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **The user's own selector survives.** A source's query carries the environment they chose
    /// and whatever else they narrowed by; an app choice that replaced it would silently widen the
    /// window to every environment at once, which is the failure this project keeps calling the
    /// worst kind — more data than asked for, with nothing to say so.
    #[test]
    fn every_other_matcher_survives_the_rewrite() {
        let out =
            with_apps(r#"{environment="live", cluster="ukwest"}"#, &["identity"]).expect("ok");
        assert_eq!(
            out,
            r#"{environment="live", cluster="ukwest", app="identity"}"#
        );
    }

    #[test]
    fn one_app_is_an_exact_matcher_and_several_are_an_alternation() {
        assert_eq!(
            with_apps(r#"{environment="live"}"#, &["identity"]).expect("ok"),
            r#"{environment="live", app="identity"}"#
        );
        assert_eq!(
            with_apps(r#"{environment="live"}"#, &["identity", "gateway"]).expect("ok"),
            r#"{environment="live", app=~"identity|gateway"}"#
        );
    }

    /// Reopening the picker and choosing again must not leave two `app` matchers, which Loki reads
    /// as "both must hold" — an intersection that matches nothing and looks like an empty source.
    #[test]
    fn choosing_again_replaces_the_app_matcher_rather_than_adding_one() {
        let once = with_apps(r#"{environment="live"}"#, &["identity"]).expect("ok");
        let twice = with_apps(&once, &["gateway", "media"]).expect("ok");
        assert_eq!(twice, r#"{environment="live", app=~"gateway|media"}"#);
        assert_eq!(twice.matches("app").count(), 1);
    }

    /// The pipeline after the selector is the user's too, and it is not the picker's business.
    #[test]
    fn the_pipeline_after_the_selector_is_untouched() {
        let out =
            with_apps(r#"{environment="live"} | json | level = "error""#, &["api"]).expect("ok");
        assert_eq!(
            out,
            r#"{environment="live", app="api"} | json | level = "error""#
        );
    }

    /// An empty query is a legitimate source meaning "everything this credential can see", so it
    /// gets a selector written for it rather than being refused.
    #[test]
    fn an_empty_query_gains_a_selector() {
        assert_eq!(with_apps("", &["api"]).expect("ok"), r#"{app="api"}"#);
        assert_eq!(with_apps("   ", &["api"]).expect("ok"), r#"{app="api"}"#);
    }

    #[test]
    fn no_choice_leaves_the_query_exactly_as_it_was() {
        let query = r#"{environment="live", app="identity"} | json"#;
        assert_eq!(with_apps(query, &[]).expect("ok"), query);
    }

    /// A comma inside a quoted value is not a matcher boundary, and a selector that never closes
    /// is refused rather than guessed at.
    #[test]
    fn a_malformed_or_quoted_selector_is_refused_rather_than_guessed_at() {
        let out = with_apps(r#"{message="a, b", environment="live"}"#, &["api"]).expect("ok");
        assert_eq!(out, r#"{message="a, b", environment="live", app="api"}"#);

        assert_eq!(
            with_apps(r#"{environment="live""#, &["api"]),
            Err(SelectorFault::Unbalanced)
        );
        assert_eq!(
            with_apps(r#"| json"#, &["api"]),
            Err(SelectorFault::NoSelector)
        );
        assert_eq!(
            with_apps(r#"{environment="live"}"#, &["a\"b"]),
            Err(SelectorFault::UnquotableName)
        );
    }

    /// **`api` must not match `api-v2`.** Loki anchors a label regex, so alternation of literals is
    /// exact — but a name carrying a metacharacter would not be a literal, and this estate has none
    /// today and cannot promise to have none tomorrow.
    ///
    /// **And the escape needs two backslashes, not one**, which is the layer this got wrong until a
    /// reviewer named it: a matcher's value is Go-unquoted before it is compiled as a regex, so the
    /// text `"dw\.api"` is a parse error rather than a pattern. What must be on the wire is
    /// `"dw\\.api"`, and the assertion below is written against the bytes for that reason.
    #[test]
    fn a_name_with_a_metacharacter_matches_only_itself() {
        let out = with_apps("{}", &["property-data-api-v2", "dw.api"]).expect("ok");
        assert_eq!(
            out, "{app=~\"property-data-api-v2|dw\\\\.api\"}",
            "the query text carries two backslashes, which unquote to the regex escape"
        );
        assert!(
            out.contains("dw\\\\.api"),
            "spelled out: two backslash characters, then the dot: {out}"
        );

        // And the round trip, over the name that would have broken it. Written through `with_apps`
        // rather than as a literal, so the two halves cannot drift apart.
        assert_eq!(apps_in(&out), ["property-data-api-v2", "dw.api"]);
    }

    /// Every metacharacter survives the round trip, not only the dot. The old reader bailed on some
    /// of them and dropped the *whole* selection — including the names that were fine.
    #[test]
    fn every_escapable_name_survives_the_round_trip() {
        for name in [
            "a.b", "a+b", "a*b", "a?b", "a(b)", "a[b]", "a{b}", "a^b", "a$b", "a|b",
        ] {
            let query = with_apps("{environment=\"live\"}", &[name, "gateway"])
                .unwrap_or_else(|e| panic!("{name}: {e:?}"));
            assert_eq!(
                apps_in(&query),
                [name, "gateway"],
                "{name} did not survive: {query}"
            );
        }
    }

    /// **A refusal, not a truncation.** Opening the first eight of seventy would give the user
    /// eight windows and no way to learn which sixty-two were missing.
    #[test]
    fn separate_windows_refuses_a_choice_it_cannot_honour() {
        assert_eq!(too_many_windows(0), None);
        assert_eq!(
            too_many_windows(MAX_SEPARATE),
            None,
            "the limit itself is allowed"
        );
        let refusal = too_many_windows(MAX_SEPARATE + 1).expect("a refusal");
        assert!(
            refusal.contains(&format!("{}", MAX_SEPARATE + 1)),
            "it says how many were asked for: {refusal}"
        );
        assert!(
            refusal.contains("Interleave"),
            "and what to do instead: {refusal}"
        );
    }

    #[test]
    fn a_list_drops_blanks_and_repeats_and_marks_what_the_query_already_asks_for() {
        let values = vec![
            "gateway".to_owned(),
            "identity".to_owned(),
            "gateway".to_owned(),
            "   ".to_owned(),
        ];
        let list = AppList::new(&values, &["identity".to_owned()]);
        assert_eq!(list.rows().len(), 2);
        assert_eq!(list.rows()[0].name, "gateway");
        assert!(!list.rows()[0].chosen);
        assert!(
            list.rows()[1].chosen,
            "what the query names is already ticked"
        );
        assert!(!list.cut());
    }

    #[test]
    fn a_list_longer_than_the_cap_is_cut_and_says_so() {
        let values: Vec<String> = (0..MAX_APPS + 10).map(|n| format!("app-{n}")).collect();
        let list = AppList::new(&values, &[]);
        assert_eq!(list.rows().len(), MAX_APPS);
        assert!(
            list.cut(),
            "a picker that quietly drops values is a picker that lies"
        );
    }

    #[test]
    fn choosing_and_unchoosing_is_what_the_dialog_acts_on() {
        let values = vec!["a".to_owned(), "b".to_owned(), "c".to_owned()];
        let mut list = AppList::new(&values, &[]);
        assert!(!list.any());
        list.toggle(0);
        list.toggle(2);
        assert_eq!(list.chosen(), ["a", "c"]);
        assert!(list.any());
        list.toggle(0);
        assert_eq!(list.chosen(), ["c"]);
        list.choose_all(true);
        assert_eq!(list.chosen(), ["a", "b", "c"]);
        list.choose_all(false);
        assert!(!list.any());
        list.toggle(99);
        assert!(
            !list.any(),
            "a click past the end of a redrawn list does nothing"
        );
    }

    /// Reopening the picker shows what was chosen last time, and **only when the query really says
    /// so** — a tick beside a name a negation excludes would be a lie about the user's own query.
    #[test]
    fn the_query_says_which_apps_are_already_chosen() {
        assert_eq!(
            apps_in(r#"{environment="live", app="identity"}"#),
            ["identity"]
        );
        assert_eq!(
            apps_in(r#"{app=~"identity|gateway", environment="live"}"#),
            ["identity", "gateway"]
        );
        assert_eq!(
            apps_in("{app=~\"property-data-api-v2|dw\\\\.api\"}"),
            ["property-data-api-v2", "dw.api"],
            "the escaping with_apps wrote is read back as the name it stood for"
        );
        assert!(
            apps_in("{app=~\"dw\\.api\"}").is_empty(),
            "a single backslash is not an escape this wrote, so it is not read back as a name"
        );
        assert!(apps_in(r#"{environment="live"}"#).is_empty());
        assert!(
            apps_in(r#"{app!="identity"}"#).is_empty(),
            "a negation names no chosen app"
        );
        assert!(
            apps_in(r#"{app=~"api.*"}"#).is_empty(),
            "a real pattern is the user's and is not read back as a set of ticks"
        );
        assert!(apps_in("| json").is_empty());
    }

    /// The round trip the dialog actually performs: read what the source asks for, offer the
    /// server's values with those ticked, change the choice, write it back.
    #[test]
    fn a_choice_survives_being_written_and_read_again() {
        let query = r#"{environment="live"} | json"#;
        let values = vec![
            "gateway".to_owned(),
            "identity".to_owned(),
            "media".to_owned(),
        ];

        let mut list = AppList::new(&values, &apps_in(query));
        assert!(!list.any());
        list.toggle(1);
        list.toggle(2);
        let asked = with_apps(query, &list.chosen()).expect("ok");

        let again = AppList::new(&values, &apps_in(&asked));
        assert_eq!(again.chosen(), ["identity", "media"]);
    }

    /// **The bug this exists for: a list view part-way through being filled.** Each row the dialog
    /// ticks raises a notification that reads every row back, and a row the list view has not
    /// reached yet answers "not ticked". Reading that as a choice turned off every pre-ticked
    /// application below the one being filled, so a source already narrowed to two applications
    /// opened with nothing ticked. Ticks that do not reach a row leave it alone.
    #[test]
    fn ticks_that_do_not_cover_a_row_leave_it_alone() {
        let values = vec!["a".to_owned(), "b".to_owned(), "c".to_owned()];
        let mut list = AppList::new(&values, &["b".to_owned(), "c".to_owned()]);
        assert_eq!(list.chosen(), ["b", "c"]);

        // What a half-filled list view answers: row zero exists and matches its model; the rows
        // below it do not exist yet and answer nothing.
        assert!(!sync(&mut list, &[false]));
        assert_eq!(
            list.chosen(),
            ["b", "c"],
            "the rows the list view has not reached keep their choice"
        );

        assert!(sync(&mut list, &[true, false, true]));
        assert_eq!(list.chosen(), ["a", "c"], "and a full read is obeyed");
        assert!(
            !sync(&mut list, &[true, false, true]),
            "an unchanged read is reported as unchanged, so the dialog can skip its redraw"
        );
    }
}
