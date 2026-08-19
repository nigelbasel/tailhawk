//! The format wizard's model — `UI-DESIGN.md` §6.2 and §6.3, `SPEC.md` §6.5.
//!
//! Two doors lead to the same artefact. **Define from example** (§6.2) takes one representative
//! line, proposes a split across it, and lets the user drag the boundaries and name the roles;
//! **import** (§6.3) takes a layout string the user already has in a logging config. Either way the
//! result is a *template* — a string plus the language it is written in — and `template::compile`
//! is what turns that into a [`Format`]. Nothing here parses a log.
//!
//! **This module does not compile as a side effect of editing, and that is the point.**
//! `template::compile` hands back a `&'static Format`, and it gets that by leaking
//! ([`crate::format::custom`]) — a cost that module's own doc-comment sizes at *"one per opened
//! file"*. §6.2 wants the pattern and the preview to follow the boundary handles live. So building
//! the template from the boundaries is pure text ([`Wizard::template`]), free to run on every drag,
//! and compilation happens only where the user asked for it: [`Wizard::test`], or a save. Those
//! memoise on the template string, so testing one pattern twice leaks once and a wizard nobody
//! tests leaks nothing.

use std::ops::Range;
use std::path::{Path, PathBuf};

use regex::Regex;

use crate::format::Format;
use crate::rules::{quote, strip_comment, unquote};
use crate::template::{self, Language, TIMESTAMP_SHAPE};

/// The formats file's name. Resolved through `SPEC.md` §12.4's tiers, as the rules file is.
pub const FILE_NAME: &str = "tailhawk.formats.toml";

/// The most a pasted layout may be. `SPEC.md` §13.1: imported configuration is inert data, and
/// inert data is bounded before it is looked at.
pub const MAX_LAYOUT: usize = 4096;

/// §6.2's preview is "the next 200 lines".
pub const MAX_SAMPLES: usize = 200;

/// More fields than any real log line has columns.
pub const MAX_FIELDS: usize = 32;

/// What a span of the example line is for. §6.2's `Roles` row picks one of these per field.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Role {
    /// The record's time — the DSL's `<ts>`, and what drives [`crate::format::Stamp`].
    Timestamp,
    /// The severity word — the DSL's `<level>`.
    Severity,
    /// The rest of the line — the DSL's `<message>`. At most one, and nothing may follow it.
    Message,
    /// A column of the user's own naming — the DSL's `<name>`.
    Named,
    /// Matched and thrown away — the DSL's `<_>`, which is **one whitespace-free token**, not a
    /// span of them.
    Discard,
}

/// One span of the example line, and what it is for. Text *between* fields is literal and is
/// reproduced verbatim in the template.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Field {
    /// Byte offset into the example, on a character boundary.
    pub start: usize,
    /// Byte offset into the example, on a character boundary, after `start`.
    pub end: usize,
    pub role: Role,
    /// Used only by [`Role::Named`]; carried through a role change so flipping back restores it.
    pub name: String,
}

impl Field {
    /// The DSL token this field becomes — what §6.2's ruler labels the span with.
    pub fn token(&self) -> String {
        match self.role {
            Role::Timestamp => "<ts>".to_owned(),
            Role::Severity => "<level>".to_owned(),
            Role::Message => "<message>".to_owned(),
            Role::Discard => "<_>".to_owned(),
            Role::Named => format!("<{}>", self.name),
        }
    }
}

/// Which end of a field a drag is moving.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Edge {
    Start,
    End,
}

/// Where a wizard's template comes from — §6.2's example line, or §6.3's pasted layout.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    Example {
        example: String,
        fields: Vec<Field>,
    },
    Layout {
        language: Language,
        template: String,
    },
}

/// A saved format definition — one `[[format]]` in `tailhawk.formats.toml`.
///
/// The **original** template and its language are stored, not the pattern they compile to, so an
/// imported NLog layout stays an NLog layout: re-openable in the wizard, and recompiled by whatever
/// the compiler has learned since. `samples` is what §6.5's **Test** re-runs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Definition {
    pub name: String,
    pub language: Option<Language>,
    pub template: String,
    /// §6.5.1's "remembered per path and per glob" — the pattern this definition claims.
    pub glob: Option<String>,
    pub samples: Vec<String>,
}

/// One row of §6.2's preview: the span of each column within the sample line, or `None` for a
/// sample the format did not match.
pub type PreviewRow = Option<Vec<Option<Range<usize>>>>;

/// The result of §6.2's **Test** — the preview grid and the match-rate readout above it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Test {
    pub columns: Vec<String>,
    pub rows: Vec<PreviewRow>,
    /// How many of `rows` matched. The readout is this over `rows.len()`.
    pub matched: usize,
    /// Why the template did not compile at all. When set, `rows` is empty.
    pub error: Option<String>,
}

impl Test {
    /// The match rate §6.2 prints beside the preview, or `None` when there was nothing to try.
    pub fn rate(&self) -> Option<f32> {
        if self.rows.is_empty() {
            None
        } else {
            Some(self.matched as f32 / self.rows.len() as f32)
        }
    }
}

/// §6.2 and §6.3's editable state.
pub struct Wizard {
    source: Source,
    samples: Vec<String>,
    /// What the definition will be called — §6.2's "Save as…".
    pub name: String,
    /// §6.5.1's glob, when the user chose one over "this file".
    pub glob: Option<String>,
    /// Every template this wizard has compiled, and what came back. One entry is one leak; the
    /// cache is what keeps a second **Test** on an unchanged pattern from being a second one.
    cache: Vec<(Language, String, String, Result<&'static Format, String>)>,
    tested: Option<(String, Test)>,
}

impl Wizard {
    /// §6.2, opened on a right-clicked line: the proposed split, ready to be dragged.
    pub fn from_example(line: &str) -> Wizard {
        let example = line.trim_end_matches(['\r', '\n']).to_owned();
        let fields = propose(&example);
        Wizard {
            source: Source::Example { example, fields },
            samples: Vec::new(),
            name: String::new(),
            glob: None,
            cache: Vec::new(),
            tested: None,
        }
    }

    /// §6.3, from a layout already known to be in `language`.
    pub fn from_layout(language: Language, template: &str) -> Wizard {
        Wizard {
            source: Source::Layout {
                language,
                template: template.trim().to_owned(),
            },
            samples: Vec::new(),
            name: String::new(),
            glob: None,
            cache: Vec::new(),
            tested: None,
        }
    }

    /// §6.3's paste box: recognise the layout's language, or say why it was not recognised.
    pub fn paste(text: &str) -> Result<Wizard, String> {
        Ok(Wizard::from_layout(recognise(text)?, text))
    }

    /// §6.3's findings list: the `index`-th of `template::scan`'s results, taken as it stands.
    ///
    /// One config file can hold several layouts, and the name becomes the compiled format's id —
    /// so a second layout from one file is numbered rather than made indistinguishable from the
    /// first.
    pub fn from_found(found: &template::Found, index: usize) -> Wizard {
        let mut w = Wizard::from_layout(found.language, &found.template);
        let file = found
            .source
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("config");
        w.name = if index == 0 {
            file.to_owned()
        } else {
            format!("{file} ({})", index + 1)
        };
        w
    }

    pub fn source(&self) -> &Source {
        &self.source
    }

    /// The example line, for §6.2's ruler. `None` when this wizard came from a layout.
    pub fn example(&self) -> Option<&str> {
        match &self.source {
            Source::Example { example, .. } => Some(example),
            Source::Layout { .. } => None,
        }
    }

    /// The fields across the example, in order. Empty when this wizard came from a layout.
    pub fn fields(&self) -> &[Field] {
        match &self.source {
            Source::Example { fields, .. } => fields,
            Source::Layout { .. } => &[],
        }
    }

    pub fn language(&self) -> Language {
        match &self.source {
            Source::Example { .. } => Language::Dsl,
            Source::Layout { language, .. } => *language,
        }
    }

    /// The template as text — §6.2's `Pattern` field, rebuilt from the boundaries.
    ///
    /// **Allocates a string and nothing else.** Safe to call on every drag; see the module note.
    pub fn template(&self) -> String {
        match &self.source {
            Source::Layout { template, .. } => template.clone(),
            Source::Example { example, fields } => {
                let mut out = String::with_capacity(example.len() + 16);
                let mut at = 0;
                for f in fields {
                    out.push_str(&example[at..f.start]);
                    out.push_str(&f.token());
                    at = f.end;
                }
                out.push_str(&example[at..]);
                out
            }
        }
    }

    /// Why the template cannot be compiled, found by reading it rather than by compiling it — so
    /// §6.2 can put an error under the pattern field on every keystroke without leaking a
    /// [`Format`] to learn of it. `None` does not promise the compiler will agree; it promises the
    /// faults this module is responsible for are absent.
    pub fn error(&self) -> Option<String> {
        let Source::Example { example, fields } = &self.source else {
            let t = self.template();
            return if t.trim().is_empty() {
                Some("nothing to compile".into())
            } else if t.len() > MAX_LAYOUT {
                Some(format!("layout is longer than {MAX_LAYOUT} bytes"))
            } else {
                expression_template(&t)
            };
        };
        if fields.is_empty() {
            return Some("no fields marked — drag a boundary across part of the line".into());
        }
        let mut at = 0;
        let mut seen: Vec<&str> = Vec::new();
        for (i, f) in fields.iter().enumerate() {
            if f.start < at {
                return Some("fields overlap".into());
            }
            if f.start >= f.end {
                return Some("a field is empty".into());
            }
            if !example.is_char_boundary(f.start) || !example.is_char_boundary(f.end) {
                return Some("a boundary falls inside a character".into());
            }
            if let Some(bad) = example[at..f.start]
                .chars()
                .find(|c| *c == '<' || *c == '>')
            {
                return Some(format!(
                    "the text between fields contains {bad}, which the pattern language reserves"
                ));
            }
            if f.role == Role::Message && i + 1 != fields.len() {
                return Some(
                    "the message must be the last field — it takes the rest of the line".into(),
                );
            }
            if i > 0 && f.start == at {
                return Some(
                    "two fields touch — leave a separator between them, or nothing tells the pattern where one ends"
                        .into(),
                );
            }
            if let Some(why) = fits(f, &example[f.start..f.end], i + 1 == fields.len()) {
                return Some(why);
            }
            if f.role != Role::Discard {
                let name = token_name(f);
                if seen.contains(&name) {
                    return Some(format!("two fields are both called {name}"));
                }
                if f.role == Role::Named {
                    if let Some(why) = bad_name(&f.name) {
                        return Some(why);
                    }
                }
                seen.push(name);
            }
            at = f.end;
        }
        if let Some(bad) = example[at..].chars().find(|c| *c == '<' || *c == '>') {
            return Some(format!(
                "the text after the last field contains {bad}, which the pattern language reserves"
            ));
        }
        if !fields
            .iter()
            .any(|f| matches!(f.role, Role::Timestamp | Role::Severity | Role::Message))
        {
            return Some("mark a timestamp, a level or a message — a format of named columns alone cannot be told from any other line".into());
        }
        None
    }

    /// The lines §6.2 previews over and §6.5's **Test** re-runs. Capped at [`MAX_SAMPLES`].
    pub fn samples(&self) -> &[String] {
        &self.samples
    }

    pub fn set_samples<I: IntoIterator<Item = String>>(&mut self, lines: I) {
        self.samples = lines
            .into_iter()
            .map(|l| l.trim_end_matches(['\r', '\n']).to_owned())
            .filter(|l| !l.is_empty())
            .take(MAX_SAMPLES)
            .collect();
        self.tested = None;
    }

    /// §6.2's drag. `to` is a byte offset into the example and must land on a character boundary,
    /// keep the field non-empty, and not cross a neighbour.
    pub fn move_boundary(&mut self, i: usize, edge: Edge, to: usize) -> Result<(), String> {
        let Source::Example { example, fields } = &mut self.source else {
            return Err("this format came from a layout, not an example".into());
        };
        if i >= fields.len() {
            return Err("no such field".into());
        }
        if to > example.len() || !example.is_char_boundary(to) {
            return Err("that is not a character boundary".into());
        }
        let low = if i == 0 { 0 } else { fields[i - 1].end };
        let high = fields.get(i + 1).map_or(example.len(), |f| f.start);
        match edge {
            Edge::Start if to < low => return Err("that crosses the field before it".into()),
            Edge::Start if to >= fields[i].end => return Err("a field cannot be empty".into()),
            Edge::End if to > high => return Err("that crosses the field after it".into()),
            Edge::End if to <= fields[i].start => return Err("a field cannot be empty".into()),
            _ => {}
        }
        match edge {
            Edge::Start => fields[i].start = to,
            Edge::End => fields[i].end = to,
        }
        self.tested = None;
        Ok(())
    }

    /// §6.2's `Roles` dropdown.
    pub fn set_role(&mut self, i: usize, role: Role) -> Result<(), String> {
        let Source::Example { fields, .. } = &mut self.source else {
            return Err("this format came from a layout, not an example".into());
        };
        if i >= fields.len() {
            return Err("no such field".into());
        }
        let free = next_name(fields);
        let f = &mut fields[i];
        if role == Role::Named && f.name.is_empty() {
            f.name = free;
        }
        f.role = role;
        self.tested = None;
        Ok(())
    }

    /// Names a [`Role::Named`] column. The DSL's own role words are refused rather than quietly
    /// turning the column into that role.
    pub fn set_name(&mut self, i: usize, name: &str) -> Result<(), String> {
        if let Some(why) = bad_name(name) {
            return Err(why);
        }
        let Source::Example { fields, .. } = &mut self.source else {
            return Err("this format came from a layout, not an example".into());
        };
        let Some(f) = fields.get_mut(i) else {
            return Err("no such field".into());
        };
        f.name = name.to_owned();
        self.tested = None;
        Ok(())
    }

    /// Adds a field over `span`, in its place in the order. Fails if it would overlap one.
    pub fn add_field(
        &mut self,
        span: Range<usize>,
        role: Role,
        name: &str,
    ) -> Result<usize, String> {
        let Source::Example { example, fields } = &mut self.source else {
            return Err("this format came from a layout, not an example".into());
        };
        if fields.len() >= MAX_FIELDS {
            return Err(format!("more than {MAX_FIELDS} fields"));
        }
        if span.start >= span.end || span.end > example.len() {
            return Err("that is not a span of the example".into());
        }
        if !example.is_char_boundary(span.start) || !example.is_char_boundary(span.end) {
            return Err("that is not a character boundary".into());
        }
        if fields
            .iter()
            .any(|f| span.start < f.end && f.start < span.end)
        {
            return Err("that overlaps a field".into());
        }
        let at = fields.iter().take_while(|f| f.end <= span.start).count();
        fields.insert(
            at,
            Field {
                start: span.start,
                end: span.end,
                role,
                name: name.to_owned(),
            },
        );
        self.tested = None;
        Ok(at)
    }

    /// Removes a field; the span it held becomes literal text again.
    pub fn remove_field(&mut self, i: usize) -> Result<(), String> {
        let Source::Example { fields, .. } = &mut self.source else {
            return Err("this format came from a layout, not an example".into());
        };
        if i >= fields.len() {
            return Err("no such field".into());
        }
        fields.remove(i);
        self.tested = None;
        Ok(())
    }

    /// Splits field `i` at byte offset `at`, which must fall strictly inside it. The new field
    /// takes the tail and is [`Role::Named`].
    pub fn split(&mut self, i: usize, at: usize) -> Result<(), String> {
        let Source::Example { example, fields } = &mut self.source else {
            return Err("this format came from a layout, not an example".into());
        };
        if fields.len() >= MAX_FIELDS {
            return Err(format!("more than {MAX_FIELDS} fields"));
        }
        let Some(f) = fields.get(i).cloned() else {
            return Err("no such field".into());
        };
        if at <= f.start || at >= f.end {
            return Err("that is not inside the field".into());
        }
        if !example.is_char_boundary(at) {
            return Err("that is not a character boundary".into());
        }
        let free = next_name(fields);
        fields[i].end = at;
        fields.insert(
            i + 1,
            Field {
                start: at,
                end: f.end,
                role: Role::Named,
                name: free,
            },
        );
        self.tested = None;
        Ok(())
    }

    /// Merges field `i` with the one after it, swallowing the literal text between them. The
    /// merged field keeps `i`'s role and name.
    pub fn merge(&mut self, i: usize) -> Result<(), String> {
        let Source::Example { fields, .. } = &mut self.source else {
            return Err("this format came from a layout, not an example".into());
        };
        if i + 1 >= fields.len() {
            return Err("there is no field after that one".into());
        }
        let end = fields[i + 1].end;
        fields[i].end = end;
        fields.remove(i + 1);
        self.tested = None;
        Ok(())
    }

    /// Replaces the layout text — §6.3's paste box as it is typed into. Keeps the language it was
    /// recognised as; use [`Wizard::paste`] to recognise afresh.
    pub fn set_layout(&mut self, text: &str) -> Result<(), String> {
        if text.len() > MAX_LAYOUT {
            return Err(format!("layout is longer than {MAX_LAYOUT} bytes"));
        }
        if let Some(why) = expression_template(text) {
            return Err(why);
        }
        let Source::Layout { template, .. } = &mut self.source else {
            return Err("this format came from an example, not a layout".into());
        };
        *template = text.to_owned();
        self.tested = None;
        Ok(())
    }

    /// Compiles the template — the one place in this module that leaks. Memoised, so asking twice
    /// for the same template costs one [`Format`].
    pub fn compile(&mut self) -> Result<&'static Format, String> {
        if let Some(why) = self.error() {
            return Err(why);
        }
        let (language, text, origin) = (self.language(), self.template(), self.origin());
        if let Some((_, _, _, got)) = self
            .cache
            .iter()
            .find(|(l, t, o, _)| *l == language && *t == text && *o == origin)
        {
            return got.clone();
        }
        let got = template::compile(language, &text, &origin);
        self.cache.push((language, text, origin, got.clone()));
        got
    }

    /// What the compiled [`Format`] is named after — and therefore part of the memo's key, because
    /// renaming a definition must not go on showing the old name in §6.1's chip.
    fn origin(&self) -> String {
        if self.name.is_empty() {
            "wizard".to_owned()
        } else {
            self.name.clone()
        }
    }

    /// The last [`Wizard::test`], if it still belongs to the pattern as it now stands — **without
    /// compiling anything**.
    ///
    /// This is the seam that makes the module note's rule a property of the types rather than a
    /// discipline the surface has to remember: a painter is handed a `&Wizard`, [`Wizard::test`]
    /// needs `&mut`, and so a frame *cannot* compile even by mistake.
    ///
    /// `None` means the pattern has changed since the last Test. §6.2's readout should say so —
    /// a match rate belonging to a pattern the user has since edited away is worse than no rate at
    /// all, because the rate is the reassurance §6.2 leans on.
    pub fn last_test(&self) -> Option<&Test> {
        match &self.tested {
            Some((was, test)) if *was == self.origin() => Some(test),
            _ => None,
        }
    }

    /// §6.2's preview and its match-rate readout, and §6.5's **Test** button. Compiles — see the
    /// module note on why nothing else here does.
    pub fn test(&mut self) -> &Test {
        let origin = self.origin();
        let stale = match &self.tested {
            Some((was, _)) => *was != origin,
            None => true,
        };
        if stale {
            let format = self.compile();
            let test = match format {
                Err(error) => Test {
                    error: Some(error),
                    ..Test::default()
                },
                Ok(format) => {
                    let rows: Vec<PreviewRow> =
                        self.samples.iter().map(|l| format.fields(l)).collect();
                    Test {
                        columns: format.columns.iter().map(|c| (*c).to_owned()).collect(),
                        matched: rows.iter().filter(|r| r.is_some()).count(),
                        rows,
                        error: None,
                    }
                }
            };
            self.tested = Some((origin, test));
        }
        &self.tested.as_ref().expect("just set").1
    }

    /// What "Save as…" writes.
    pub fn definition(&self) -> Definition {
        Definition {
            name: self.name.clone(),
            language: Some(self.language()),
            template: self.template(),
            glob: self.glob.clone(),
            samples: self.samples.clone(),
        }
    }
}

/// Why the token `f` becomes cannot match the span `f` covers.
///
/// Without this a drag that widened `<ts>` over its trailing space, or marked a two-word span as a
/// level, would leave §6.2's pattern field showing no error at all and the fault would surface only
/// as a preview that matched nothing.
fn fits(f: &Field, span: &str, last: bool) -> Option<String> {
    match f.role {
        Role::Timestamp if !stamp_shape().is_match(span) => Some(format!(
            "{span:?} is not a timestamp shape the pattern language knows"
        )),
        Role::Severity if !level_word().is_match(span) => Some(format!(
            "{span:?} is not a single word, so it cannot be a level"
        )),
        Role::Discard if span.chars().any(char::is_whitespace) => {
            Some("a discarded field is one token — it cannot span a space".into())
        }
        Role::Named if !last && span.chars().any(char::is_whitespace) => Some(format!(
            "{} spans a space, and only the last field on a line may",
            f.name
        )),
        _ => None,
    }
}

fn stamp_shape() -> &'static Regex {
    static RE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    RE.get_or_init(|| Regex::new(&format!("^(?:{TIMESTAMP_SHAPE})$")).expect("regex"))
}

fn level_word() -> &'static Regex {
    static RE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    RE.get_or_init(|| Regex::new("^[A-Za-z]+$").expect("regex"))
}

/// `SPEC.md` §6.5 excludes Serilog's `ExpressionTemplate`, because its `{#if}` and `{#each}`
/// segments appear and disappear line to line and a confidence score over such a format means
/// nothing. Its built-in `{@t}` / `{@l}` / `{@m}` properties give one away even when it carries no
/// directive at all, which is the common case and the one a generic "not recognised" would
/// misexplain.
fn expression_template(text: &str) -> Option<String> {
    (text.contains("{#") || text.contains("{@")).then(|| {
        "that is a Serilog ExpressionTemplate — its segments come and go line to line, which SPEC.md §6.5 does not support"
            .to_owned()
    })
}

/// The first `fN` no field has taken.
fn next_name(fields: &[Field]) -> String {
    (1..)
        .map(|n| format!("f{n}"))
        .find(|candidate| !fields.iter().any(|f| f.name == *candidate))
        .expect("unbounded")
}

/// The capture name a field claims, for the duplicate check.
fn token_name(f: &Field) -> &str {
    match f.role {
        Role::Timestamp => "ts",
        Role::Severity => "level",
        Role::Message => "message",
        Role::Discard => "_",
        Role::Named => &f.name,
    }
}

/// The DSL's own role words, which a named column may not take.
const RESERVED: &[&str] = &[
    "ts",
    "timestamp",
    "date",
    "time",
    "level",
    "severity",
    "message",
    "msg",
    "body",
    "_",
];

/// Why `name` cannot be a column name — it becomes a regex capture group, so it is not free-form.
fn bad_name(name: &str) -> Option<String> {
    if name.is_empty() {
        return Some("a column needs a name".into());
    }
    if RESERVED.contains(&name) {
        return Some(format!(
            "{name} is a role, not a column name — set the field's role instead"
        ));
    }
    if !name.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
        || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        return Some(format!(
            "{name} must be letters, digits and underscores, starting with a letter"
        ));
    }
    None
}

/// The longest run of literal text that may separate two proposed fields. Past this the run is
/// prose, not a separator: claiming what follows it would bake a sentence of the message body into
/// the format as a mandatory literal, and the result would match the line it came from and nothing
/// else in the file.
const MAX_SEPARATOR: usize = 4;

/// How far into the line the *first* proposed field may start. A log line may open with a short
/// prefix the header proper follows; it does not open with a paragraph.
const MAX_PREFIX: usize = 16;

/// §6.5's "Tailhawk proposes a tokenisation": the timestamp, bracketed groups, the level word, a
/// dotted logger name, and the rest of the line as the message.
///
/// Candidates are taken left to right and only while they stay within [`MAX_SEPARATOR`] of the one
/// before, so a level word or a bracket in the *message* is left in the message rather than
/// promoted into the header. A bracketed group that is itself a level word is the level: `[ERR]` is
/// Serilog's default file template, not a thread id.
///
/// **Advisory only.** It runs once, when the wizard opens, and is never consulted again — §6.2's
/// control is the drag, and a proposer that re-ran would fight the user.
pub fn propose(example: &str) -> Vec<Field> {
    let stamp = Regex::new(&format!("(?:{TIMESTAMP_SHAPE})")).expect("regex");
    let bracket = Regex::new(r"\[([^\]\[]*)\]").expect("regex");
    let level =
        Regex::new(r"(?i)\b(?:TRACE|DEBUG|INFO|INFORMATION|VERBOSE|NOTICE|WARN|WARNING|ERROR|ERR|FATAL|CRITICAL)\b")
            .expect("regex");
    let logger =
        Regex::new(r"\b[A-Za-z_][A-Za-z0-9_]*(?:\.[A-Za-z_][A-Za-z0-9_]*)+\b").expect("regex");

    let mut cands: Vec<(usize, usize, u8, Role, &str)> = Vec::new();
    if let Some(m) = stamp.find(example) {
        cands.push((m.start(), m.end(), 0, Role::Timestamp, "ts"));
    }
    if let Some(c) = bracket.captures(example) {
        let g = c.get(1).expect("group 1");
        if !g.is_empty() {
            if level.find(g.as_str()).is_some_and(|m| m.len() == g.len()) {
                cands.push((g.start(), g.end(), 1, Role::Severity, "level"));
            } else {
                cands.push((g.start(), g.end(), 1, Role::Named, "thread"));
            }
        }
    }
    if let Some(m) = level.find(example) {
        cands.push((m.start(), m.end(), 2, Role::Severity, "level"));
    }
    if let Some(m) = logger.find(example) {
        cands.push((m.start(), m.end(), 3, Role::Named, "logger"));
    }
    cands.sort_by_key(|(start, _, rank, _, _)| (*start, *rank));

    let mut fields: Vec<Field> = Vec::new();
    let mut at = 0;
    for (start, end, _, role, name) in cands {
        let budget = if fields.is_empty() {
            MAX_PREFIX
        } else {
            MAX_SEPARATOR
        };
        if start < at || start - at > budget {
            continue;
        }
        if fields.iter().any(|f| f.role == role && role != Role::Named) {
            continue;
        }
        if fields.iter().any(|f| f.name == name) {
            continue;
        }
        fields.push(Field {
            start,
            end,
            role,
            name: name.to_owned(),
        });
        at = end;
    }
    if let Some(start) = message_start(example, at) {
        fields.push(Field {
            start,
            end: example.len(),
            role: Role::Message,
            name: String::new(),
        });
    }
    fields
}

/// Where the message begins after the last proposed field: past the whitespace, and past one short
/// run of separator punctuation — §6.2's ` - `, and the `|` of a pipe-delimited layout, which
/// carries no space around it and must still be left outside both fields. A separator swallowed
/// into the message would put two DSL tokens flush against each other, and the pattern would have
/// nothing to tell it where the field before ended.
fn message_start(example: &str, after: usize) -> Option<usize> {
    let mut at = after;
    let skip_space = |example: &str, at: &mut usize| {
        while let Some(c) = example[*at..].chars().next() {
            if !c.is_whitespace() {
                break;
            }
            *at += c.len_utf8();
        }
    };
    skip_space(example, &mut at);
    let mut punct = at;
    while punct < example.len()
        && punct - at < MAX_SEPARATOR
        && example[punct..].starts_with(|c: char| c.is_ascii_punctuation())
    {
        punct += 1;
    }
    if punct > at && punct < example.len() {
        at = punct;
        skip_space(example, &mut at);
    }
    (at < example.len()).then_some(at)
}

/// §6.3's "Recognised as": which layout language a pasted string is written in.
///
/// The languages overlap, so this **counts** each one's markers and takes the language with the
/// most rather than the first that matches anything. A log4net pattern may carry a `${env:…}`
/// lookup and a Serilog template may carry a literal `%`; asking which marker appears *more* gets
/// both right, where asking which appears *first* gets both wrong.
///
/// `SPEC.md` §13.1 — the text is inert data, bounded before it is read, and never followed as a
/// path or run.
pub fn recognise(layout: &str) -> Result<Language, String> {
    let text = layout.trim();
    if text.is_empty() {
        return Err("nothing pasted".into());
    }
    if layout.len() > MAX_LAYOUT {
        return Err(format!("layout is longer than {MAX_LAYOUT} bytes"));
    }
    if let Some(why) = expression_template(text) {
        return Err(why);
    }
    let count = |re: &str| Regex::new(re).expect("regex").find_iter(text).count();
    let scores = [
        (Language::NLog, text.matches("${").count()),
        (Language::Serilog, serilog_holes(text)),
        (Language::Log4net, count(r"%-?\d*[A-Za-z]")),
        (Language::Dsl, count(r"<[A-Za-z_][A-Za-z0-9_]*>")),
    ];
    let mut best: Option<(Language, usize)> = None;
    for (language, n) in scores {
        if n > 0 && best.is_none_or(|(_, best)| n > best) {
            best = Some((language, n));
        }
    }
    best.map(|(language, _)| language).ok_or_else(|| {
        "not a Serilog outputTemplate, an NLog layout or a log4net or Logback pattern".to_owned()
    })
}

/// Serilog's `{Property}` and `{Property:format}` holes, not counting NLog's `${…}`.
fn serilog_holes(text: &str) -> usize {
    let hole = Regex::new(r"\{[A-Za-z][A-Za-z0-9]*[:}]").expect("regex");
    hole.find_iter(text)
        .filter(|m| m.start() == 0 || !text[..m.start()].ends_with('$'))
        .count()
}

/// What §6.3 prints after "Recognised as".
pub fn language_label(language: Language) -> &'static str {
    match language {
        Language::Serilog => "Serilog outputTemplate",
        Language::NLog => "NLog layout",
        Language::Log4net => "log4net or Logback pattern",
        Language::Dsl => "pattern",
    }
}

/// The language's key in `tailhawk.formats.toml`.
pub fn language_key(language: Language) -> &'static str {
    match language {
        Language::Serilog => "serilog",
        Language::NLog => "nlog",
        Language::Log4net => "log4net",
        Language::Dsl => "dsl",
    }
}

/// A language from its key, or `None` for one this build does not know.
pub fn language_of(key: &str) -> Option<Language> {
    match key.trim().to_ascii_lowercase().as_str() {
        "serilog" => Some(Language::Serilog),
        "nlog" => Some(Language::NLog),
        "log4net" | "logback" => Some(Language::Log4net),
        "dsl" | "pattern" => Some(Language::Dsl),
        _ => None,
    }
}

/// The definitions in `tailhawk.formats.toml`'s text.
///
/// The same lenient reader `rules::parse` is: this is the user's own file, written by the wizard
/// and edited by hand. A definition that arrives from elsewhere is §13.1's problem and does not
/// come through here.
pub fn parse(text: &str) -> Vec<Definition> {
    let mut out: Vec<Definition> = Vec::new();
    let mut current: Option<Definition> = None;
    let flush = |current: &mut Option<Definition>, out: &mut Vec<Definition>| {
        if let Some(mut def) = current.take() {
            if def.name.is_empty() {
                def.name = format!("format {}", out.len() + 1);
            }
            out.push(def);
        }
    };
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with("[[") {
            flush(&mut current, &mut out);
            if line.trim_start_matches('[').trim_end_matches(']').trim() == "format" {
                current = Some(Definition::default());
            }
            continue;
        }
        if line.starts_with('[') {
            flush(&mut current, &mut out);
            continue;
        }
        let Some(def) = current.as_mut() else {
            continue;
        };
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let (key, value) = (key.trim(), strip_comment(value.trim()));
        match key {
            "name" => def.name = unquote(value),
            "language" => def.language = language_of(&unquote(value)),
            "template" => def.template = unquote(value),
            "glob" => {
                let g = unquote(value);
                def.glob = (!g.is_empty()).then_some(g);
            }
            "sample" => def.samples.push(unquote(value)),
            _ => {}
        }
    }
    flush(&mut current, &mut out);
    out
}

/// The text for `defs` — what "Save as…" writes.
pub fn to_toml(defs: &[Definition]) -> String {
    let mut out = String::from(
        "# Tailhawk format definitions — SPEC.md §6.5. One [[format]] per definition.\n\
         # language is serilog, nlog, log4net or dsl; template is the layout as your logging\n\
         # config has it; glob binds the definition to a path; each sample is a line Test re-runs.\n",
    );
    for def in defs {
        out.push_str("\n[[format]]\n");
        out.push_str(&format!("name = {}\n", quote(&def.name)));
        if let Some(language) = def.language {
            out.push_str(&format!("language = \"{}\"\n", language_key(language)));
        }
        out.push_str(&format!("template = {}\n", quote(&def.template)));
        if let Some(glob) = &def.glob {
            out.push_str(&format!("glob = {}\n", quote(glob)));
        }
        for sample in &def.samples {
            out.push_str(&format!("sample = {}\n", quote(sample)));
        }
    }
    out
}

/// The formats file in each tier, exe-adjacent first — `SPEC.md` §12.4, as `rules::tiers`.
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

/// Every definition from every tier that exists, exe-adjacent first.
///
/// **Reads; does not compile.** A file with forty definitions in it would otherwise cost forty
/// leaked [`Format`]s at startup for the one the opened file needs.
pub fn load(tiers: &[PathBuf]) -> Vec<Definition> {
    let mut out = Vec::new();
    for path in tiers {
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        out.extend(parse(&text));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const LINE: &str =
        "2026-07-28 09:14:02,117 [12] INFO  Zenith.Automation.Runner - Evaluated 412 triggers";

    fn span<'a>(line: &'a str, f: &Field) -> &'a str {
        &line[f.start..f.end]
    }

    /// An example with the proposal cleared, for tests about marking fields by hand.
    fn blank(line: &str) -> Wizard {
        let mut w = Wizard::from_example(line);
        while !w.fields().is_empty() {
            w.remove_field(0).expect("removes");
        }
        w
    }

    #[test]
    fn the_proposal_is_ui_designs_worked_example() {
        let w = Wizard::from_example(LINE);
        let got: Vec<(&str, Role)> = w.fields().iter().map(|f| (span(LINE, f), f.role)).collect();
        assert_eq!(
            got,
            vec![
                ("2026-07-28 09:14:02,117", Role::Timestamp),
                ("12", Role::Named),
                ("INFO", Role::Severity),
                ("Zenith.Automation.Runner", Role::Named),
                ("Evaluated 412 triggers", Role::Message),
            ]
        );
    }

    #[test]
    fn the_pattern_is_the_one_ui_design_prints() {
        let w = Wizard::from_example(LINE);
        assert_eq!(
            w.template(),
            "<ts> [<thread>] <level>  <logger> - <message>"
        );
        assert_eq!(w.error(), None);
    }

    #[test]
    fn the_pattern_compiles_and_matches_the_line_it_came_from() {
        let mut w = Wizard::from_example(LINE);
        w.set_samples([LINE.to_owned()]);
        let test = w.test();
        assert_eq!(test.error, None);
        assert_eq!(test.matched, 1);
        assert_eq!(test.rate(), Some(1.0));
        assert!(test.columns.iter().any(|c| c == "logger"));
    }

    #[test]
    fn the_preview_reports_the_lines_that_did_not_match() {
        let mut w = Wizard::from_example(LINE);
        w.set_samples([LINE.to_owned(), "not a log line at all".to_owned()]);
        let test = w.test();
        assert_eq!(test.matched, 1);
        assert_eq!(test.rows.len(), 2);
        assert!(test.rows[1].is_none());
    }

    #[test]
    fn a_second_test_on_an_unchanged_pattern_does_not_compile_again() {
        let mut w = Wizard::from_example(LINE);
        w.set_samples([LINE.to_owned()]);
        let first = w.compile().expect("compiles");
        let second = w.compile().expect("compiles");
        assert!(std::ptr::eq(first, second), "the compile is memoised");
        assert_eq!(w.cache.len(), 1);
    }

    #[test]
    fn editing_does_not_compile() {
        let mut w = Wizard::from_example(LINE);
        for to in 4..20 {
            if LINE.is_char_boundary(to) {
                let _ = w.move_boundary(0, Edge::End, to);
                let _ = w.template();
                let _ = w.error();
            }
        }
        assert!(w.cache.is_empty(), "no Format was leaked by dragging");
    }

    #[test]
    fn a_boundary_cannot_cross_its_neighbour() {
        let mut w = Wizard::from_example(LINE);
        let next = w.fields()[1].start;
        assert!(w.move_boundary(0, Edge::End, next + 1).is_err());
        assert!(w.move_boundary(0, Edge::End, next).is_ok());
    }

    #[test]
    fn a_boundary_cannot_land_inside_a_character() {
        let line = "héllo world";
        let mut w = Wizard::from_example(line);
        let _ = w.add_field(0..1, Role::Named, "a");
        assert!(w.move_boundary(0, Edge::End, 2).is_err());
    }

    #[test]
    fn a_field_cannot_be_emptied() {
        let mut w = Wizard::from_example(LINE);
        let start = w.fields()[0].start;
        assert!(w.move_boundary(0, Edge::End, start).is_err());
    }

    #[test]
    fn splitting_and_merging_are_inverse() {
        let mut w = Wizard::from_example(LINE);
        let before = w.template();
        let at = w.fields()[0].start + 10;
        w.split(0, at).expect("splits");
        assert_eq!(w.fields().len(), 6);
        w.merge(0).expect("merges");
        assert_eq!(w.template(), before);
    }

    #[test]
    fn a_removed_field_becomes_literal_text_again() {
        let mut w = Wizard::from_example(LINE);
        w.remove_field(1).expect("removes");
        assert_eq!(w.template(), "<ts> [12] <level>  <logger> - <message>");
    }

    #[test]
    fn an_added_field_lands_in_its_place_in_the_order() {
        let mut w = blank("a bb ccc");
        let at = w.add_field(5..8, Role::Named, "third").expect("adds");
        assert_eq!(at, 0);
        let at = w.add_field(0..1, Role::Named, "first").expect("adds");
        assert_eq!(at, 0, "an earlier span takes the earlier index");
        assert_eq!(w.template(), "<first> bb <third>");
    }

    #[test]
    fn an_added_field_may_not_overlap_one() {
        let mut w = Wizard::from_example(LINE);
        assert!(w.add_field(0..5, Role::Named, "x").is_err());
    }

    #[test]
    fn a_column_may_not_take_a_role_word() {
        let mut w = Wizard::from_example(LINE);
        assert!(w.set_name(1, "level").is_err());
        assert!(w.set_name(1, "msg").is_err());
        assert!(w.set_name(1, "worker").is_ok());
    }

    #[test]
    fn a_column_name_must_be_a_capture_name() {
        let mut w = Wizard::from_example(LINE);
        assert!(w.set_name(1, "thread id").is_err());
        assert!(w.set_name(1, "9lives").is_err());
        assert!(w.set_name(1, "").is_err());
        assert!(w.set_name(1, "thread_id").is_ok());
    }

    #[test]
    fn two_columns_may_not_claim_one_name() {
        let mut w = blank("a b");
        w.add_field(0..1, Role::Named, "x").expect("adds");
        w.add_field(2..3, Role::Named, "x").expect("adds");
        assert_eq!(w.error().as_deref(), Some("two fields are both called x"));
    }

    #[test]
    fn the_message_must_be_last() {
        let mut w = blank("a b");
        w.add_field(0..1, Role::Message, "").expect("adds");
        w.add_field(2..3, Role::Named, "x").expect("adds");
        assert!(w.error().expect("error").contains("must be the last field"));
    }

    #[test]
    fn an_angle_bracket_in_the_literal_text_is_named_not_swallowed() {
        let line = "2026-07-28 09:14:02 <boot> INFO ready";
        let mut w = Wizard::from_example(line);
        while w.fields().len() > 1 {
            w.remove_field(1).expect("removes");
        }
        assert!(w.error().expect("error").contains("reserves"));
    }

    #[test]
    fn a_wizard_with_no_fields_says_so_rather_than_compiling_the_line() {
        let mut w = Wizard::from_example(LINE);
        while !w.fields().is_empty() {
            w.remove_field(0).expect("removes");
        }
        assert!(w.error().expect("error").contains("no fields marked"));
        assert!(w.compile().is_err());
        assert!(w.cache.is_empty());
    }

    #[test]
    fn nlog_is_recognised_before_serilog_because_its_braces_are_too() {
        assert_eq!(
            recognise("${longdate}|${level:uppercase=true}|${logger}|${message}"),
            Ok(Language::NLog)
        );
    }

    #[test]
    fn the_four_layout_languages_spec_names_are_recognised() {
        assert_eq!(
            recognise("{Timestamp:yyyy-MM-dd HH:mm:ss.fff} [{Level:u3}] {Message:lj}{NewLine}"),
            Ok(Language::Serilog)
        );
        assert_eq!(
            recognise("%date{ISO8601} [%thread] %-5level %logger - %message%newline"),
            Ok(Language::Log4net)
        );
        assert_eq!(
            recognise("%d{HH:mm:ss.SSS} [%thread] %-5level %logger{36} - %msg%n"),
            Ok(Language::Log4net)
        );
        assert_eq!(recognise("<ts> <level> <message>"), Ok(Language::Dsl));
    }

    #[test]
    fn an_expression_template_is_refused_by_name() {
        let why = recognise("{#if Level = 'Error'}{Message}{#end}").expect_err("refused");
        assert!(why.contains("ExpressionTemplate"), "{why}");
    }

    #[test]
    fn nothing_and_nonsense_are_refused() {
        assert!(recognise("   ").is_err());
        assert!(recognise("just some prose").is_err());
        assert!(recognise(&"x".repeat(MAX_LAYOUT + 1)).is_err());
    }

    #[test]
    fn an_imported_layout_compiles_and_columnises() {
        let mut w = Wizard::paste("${longdate}|${level:uppercase=true}|${logger}|${message}")
            .expect("recognised");
        w.name = "NLog.config".to_owned();
        w.set_samples(["2026-07-28 09:14:02.1170|INFO|Zenith.Runner|off we go".to_owned()]);
        let test = w.test();
        assert_eq!(test.error, None);
        assert_eq!(test.matched, 1);
    }

    #[test]
    fn a_layout_wizard_has_no_example_to_drag() {
        let mut w = Wizard::paste("${message}").expect("recognised");
        assert_eq!(w.example(), None);
        assert!(w.fields().is_empty());
        assert!(w.move_boundary(0, Edge::End, 0).is_err());
        assert!(w.set_role(0, Role::Message).is_err());
    }

    #[test]
    fn changing_the_layout_invalidates_the_test() {
        let mut w = Wizard::paste("${message}").expect("recognised");
        w.set_samples(["hello".to_owned()]);
        assert_eq!(w.test().matched, 1);
        w.set_layout("${longdate}|${message}").expect("sets");
        assert_eq!(w.test().matched, 0);
    }

    #[test]
    fn samples_are_capped_and_blank_lines_dropped() {
        let mut w = Wizard::from_example(LINE);
        w.set_samples((0..MAX_SAMPLES + 50).map(|i| {
            if i % 2 == 0 {
                String::new()
            } else {
                format!("line {i}\n")
            }
        }));
        assert_eq!(w.samples().len(), (MAX_SAMPLES + 50) / 2);
        assert!(w.samples().iter().all(|s| !s.ends_with('\n')));
    }

    #[test]
    fn a_definition_round_trips_through_the_file() {
        let mut w = Wizard::from_example(LINE);
        w.name = "NDC api".to_owned();
        w.glob = Some(r"C:\logs\ndc\*.log".to_owned());
        w.set_samples([LINE.to_owned()]);
        let def = w.definition();
        let back = parse(&to_toml(&[def.clone()]));
        assert_eq!(back, vec![def]);
    }

    #[test]
    fn an_unnamed_definition_is_named_by_its_position() {
        let got = parse("[[format]]\ntemplate = \"<message>\"\n");
        assert_eq!(got[0].name, "format 1");
    }

    #[test]
    fn a_language_this_build_does_not_know_is_none_rather_than_a_wrong_guess() {
        let got = parse("[[format]]\nname = \"x\"\nlanguage = \"logstash\"\ntemplate = \"a\"\n");
        assert_eq!(got[0].language, None);
    }

    #[test]
    fn every_language_key_round_trips() {
        for language in [
            Language::Serilog,
            Language::NLog,
            Language::Log4net,
            Language::Dsl,
        ] {
            assert_eq!(language_of(language_key(language)), Some(language));
            assert!(!language_label(language).is_empty());
        }
    }

    #[test]
    fn loading_reads_every_tier_and_compiles_none_of_them() {
        let dir = std::env::temp_dir().join("tailhawk-wizard-tiers");
        let _ = std::fs::remove_dir_all(&dir);
        let (a, b) = (dir.join("exe"), dir.join("roaming"));
        std::fs::create_dir_all(b.join("Tailhawk")).expect("dirs");
        std::fs::create_dir_all(&a).expect("dirs");
        std::fs::write(
            a.join(FILE_NAME),
            to_toml(&[Definition {
                name: "first".to_owned(),
                language: Some(Language::Dsl),
                template: "<message>".to_owned(),
                ..Definition::default()
            }]),
        )
        .expect("write");
        std::fs::write(
            b.join("Tailhawk").join(FILE_NAME),
            to_toml(&[Definition {
                name: "second".to_owned(),
                language: Some(Language::NLog),
                template: "${message}".to_owned(),
                ..Definition::default()
            }]),
        )
        .expect("write");
        let got = load(&tiers(Some(&a), Some(&b)));
        assert_eq!(
            got.iter().map(|d| d.name.as_str()).collect::<Vec<_>>(),
            vec!["first", "second"]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_tier_is_not_an_error() {
        assert!(load(&tiers(Some(Path::new("no-such-dir")), None)).is_empty());
    }

    #[test]
    fn a_line_with_nothing_but_a_message_still_proposes_one_field() {
        let w = Wizard::from_example("something happened");
        assert_eq!(w.fields().len(), 1);
        assert_eq!(w.fields()[0].role, Role::Message);
        assert_eq!(w.template(), "<message>");
    }

    #[test]
    fn an_empty_example_proposes_nothing_and_says_so() {
        let w = Wizard::from_example("");
        assert!(w.fields().is_empty());
        assert!(w.error().is_some());
    }

    #[test]
    fn a_syslog_line_finds_its_stamp_and_message() {
        let line = "Jul 28 09:14:02 host sshd: accepted publickey";
        let w = Wizard::from_example(line);
        assert_eq!(span(line, &w.fields()[0]), "Jul 28 09:14:02");
        assert_eq!(w.fields()[0].role, Role::Timestamp);
        assert_eq!(w.fields().last().expect("last").role, Role::Message);
    }

    #[test]
    fn a_found_config_becomes_a_wizard_named_for_its_file() {
        let found = template::Found {
            language: Language::Log4net,
            template: "%date [%thread] %-5level %logger - %message".to_owned(),
            source: PathBuf::from(r"C:\dev\ndc\Api\log4net.config"),
        };
        let w = Wizard::from_found(&found, 0);
        assert_eq!(w.name, "log4net.config");
        assert_eq!(w.language(), Language::Log4net);
    }

    #[test]
    fn the_definition_stores_the_layout_it_came_from_not_the_pattern_it_compiles_to() {
        let w = Wizard::paste("${longdate}|${message}").expect("recognised");
        let def = w.definition();
        assert_eq!(def.template, "${longdate}|${message}");
        assert_eq!(def.language, Some(Language::NLog));
    }

    #[test]
    fn a_discard_is_one_token_and_the_doc_says_so() {
        let mut w = blank("a bb ccc");
        w.add_field(0..1, Role::Discard, "").expect("adds");
        w.add_field(2..4, Role::Discard, "").expect("adds");
        w.add_field(5..8, Role::Message, "").expect("adds");
        assert_eq!(w.template(), "<_> <_> <message>");
        assert!(w.error().is_none());
        assert!(w.compile().is_ok());
    }

    #[test]
    fn a_pipe_delimited_line_keeps_its_separators_outside_the_fields() {
        let line = "2026-07-28 09:14:02.1170|INFO|Zenith.Runner|off we go";
        let mut w = Wizard::from_example(line);
        assert_eq!(w.template(), "<ts>|<level>|<logger>|<message>");
        w.set_samples([line.to_owned()]);
        let test = w.test().clone();
        assert_eq!(test.matched, 1);
        let logger = test
            .columns
            .iter()
            .position(|c| c == "logger")
            .expect("column");
        let row = test.rows[0].as_ref().expect("matched");
        let span = row[logger].clone().expect("captured");
        assert_eq!(&line[span], "Zenith.Runner");
    }

    #[test]
    fn two_fields_may_not_touch_because_nothing_would_part_them() {
        let mut w = blank("ab");
        w.add_field(0..1, Role::Severity, "").expect("adds");
        w.add_field(1..2, Role::Message, "").expect("adds");
        assert!(w.error().expect("error").contains("two fields touch"));
    }

    #[test]
    fn serilogs_default_file_template_finds_its_level_and_swallows_the_offset() {
        let line = "2026-08-16 09:14:03.884 +02:00 [ERR] Failed to dispatch job 41982";
        let mut w = Wizard::from_example(line);
        assert_eq!(w.template(), "<ts> [<level>] <message>");
        assert_eq!(span(line, &w.fields()[0]), "2026-08-16 09:14:03.884 +02:00");
        assert_eq!(w.fields()[1].role, Role::Severity);
        w.set_samples([
            line.to_owned(),
            "2026-08-16 10:00:00.000 +01:00 [INF] and again after the clocks change".to_owned(),
        ]);
        assert_eq!(
            w.test().matched,
            2,
            "the offset is inside <ts>, not a literal"
        );
    }

    #[test]
    fn a_level_word_in_the_message_stays_in_the_message() {
        let line = "could not connect (ERROR in Foo.Bar) retrying";
        let w = Wizard::from_example(line);
        assert_eq!(w.template(), "<message>");
    }

    #[test]
    fn a_bracket_far_past_the_header_is_not_claimed() {
        let line = "2026-07-28 09:14:02 INFO C++ parse of a*b (100%) [ok]";
        let mut w = Wizard::from_example(line);
        assert_eq!(w.template(), "<ts> <level> <message>");
        w.set_samples([line.to_owned()]);
        let test = w.test().clone();
        let msg = test
            .columns
            .iter()
            .position(|c| c == "msg")
            .expect("column");
        let span = test.rows[0].as_ref().expect("matched")[msg]
            .clone()
            .expect("captured");
        assert_eq!(&line[span], "C++ parse of a*b (100%) [ok]");
    }

    #[test]
    fn an_expression_template_cannot_be_typed_in_after_the_paste_was_recognised() {
        let mut w = Wizard::paste("{Timestamp:HH:mm:ss} {Message}").expect("recognised");
        let why = w
            .set_layout("{#if Level = 'Error'}{Message}{#end}")
            .expect_err("refused");
        assert!(why.contains("ExpressionTemplate"), "{why}");
        assert!(recognise("{@t:HH:mm:ss} [{@l:u3}] {@m}")
            .expect_err("refused")
            .contains("ExpressionTemplate"));
    }

    #[test]
    fn a_drag_that_makes_a_token_unable_to_match_its_own_span_says_so() {
        let line = "2026-07-28 09:14:02 INFO hello";
        let mut w = Wizard::from_example(line);
        let end = w.fields()[0].end;
        w.move_boundary(0, Edge::End, end + 1).expect("moves");
        assert!(w.error().expect("error").contains("timestamp shape"));

        let mut w = blank("12 34 INFO hi");
        w.add_field(0..5, Role::Discard, "").expect("adds");
        assert!(w.error().expect("error").contains("one token"));

        let mut w = blank("a bb INFO cc");
        w.add_field(0..4, Role::Named, "wide").expect("adds");
        w.add_field(5..9, Role::Severity, "").expect("adds");
        assert!(w.error().expect("error").contains("spans a space"));
    }

    #[test]
    fn merging_a_timestamp_into_its_neighbour_is_an_error_not_a_silent_miss() {
        let mut w = Wizard::from_example(LINE);
        w.merge(0).expect("merges");
        assert!(w.error().expect("error").contains("timestamp shape"));
    }

    #[test]
    fn renaming_recompiles_so_the_chip_cannot_show_the_old_name() {
        let mut w = Wizard::from_example(LINE);
        w.name = "before".to_owned();
        assert_eq!(w.compile().expect("compiles").name, "pattern (before)");
        w.name = "after".to_owned();
        assert_eq!(w.compile().expect("compiles").name, "pattern (after)");
        assert_eq!(w.test().error, None);
    }

    #[test]
    fn a_language_is_recognised_by_which_marker_it_has_most_of() {
        assert_eq!(
            recognise("{Timestamp:yyyy-MM-dd} {Level} {Message} (%s)"),
            Ok(Language::Serilog)
        );
        assert_eq!(
            recognise("{Timestamp:HH:mm:ss} {Level:u3} {Message:lj} 100%done{NewLine}"),
            Ok(Language::Serilog)
        );
        assert_eq!(
            recognise("%date [%thread] %-5level %logger - %message%newline ${env:X}"),
            Ok(Language::Log4net)
        );
    }

    #[test]
    fn a_padded_level_does_not_pin_the_format_to_the_line_it_came_from() {
        // log4net's `%-5level` writes "INFO " and "ERROR", so a template read off an INFO line
        // carries two spaces where an ERROR line has one. Escaping the run verbatim built a format
        // that matched three lines of five — found by previewing a real log, not by a test.
        let line = "2026-07-28 09:14:02,117 [12] INFO  Zenith.Automation.Runner - Evaluated 412";
        let mut w = Wizard::from_example(line);
        w.set_samples([
            "2026-07-28 09:14:03,884 [12] ERROR Zenith.Data.SessionFactory - Could not open"
                .to_owned(),
            "2026-07-28 09:14:04,001 [15] WARN  Zenith.Automation.Runner - Retry 1 of 3".to_owned(),
            "2026-07-28 09:14:06,742 [18] DEBUG Zenith.Data.SessionFactory - Pool size 8"
                .to_owned(),
        ]);
        let test = w.test();
        assert_eq!(test.error, None);
        assert_eq!(
            test.matched, 3,
            "every line matches, whatever the level's width"
        );
    }

    #[test]
    fn a_frame_can_read_the_last_test_but_never_provoke_one() {
        let mut w = Wizard::from_example(LINE);
        w.set_samples([LINE.to_owned()]);
        assert!(w.last_test().is_none(), "nothing has been tested yet");
        assert_eq!(w.test().matched, 1);
        assert_eq!(w.last_test().expect("tested").matched, 1);
        assert!(w.cache.len() == 1);

        let end = w.fields()[0].end;
        w.move_boundary(0, Edge::End, end - 1).expect("moves");
        assert!(
            w.last_test().is_none(),
            "the rate belonged to a pattern that has been edited away"
        );
        assert_eq!(w.cache.len(), 1, "reading it did not compile");

        w.test();
        w.name = "renamed".to_owned();
        assert!(w.last_test().is_none(), "and a rename is a change too");
    }

    #[test]
    fn a_second_layout_from_one_config_file_is_not_the_first_ones_twin() {
        let found = |t: &str| template::Found {
            language: Language::NLog,
            template: t.to_owned(),
            source: PathBuf::from(r"C:\dev\ndc\Api\NLog.config"),
        };
        let mut a = Wizard::from_found(&found("${longdate}|${message}"), 0);
        let mut b = Wizard::from_found(&found("${longdate}|${level}|${message}"), 1);
        assert_eq!(a.name, "NLog.config");
        assert_eq!(b.name, "NLog.config (2)");
        assert_ne!(
            a.compile().expect("compiles").id,
            b.compile().expect("compiles").id
        );
    }
}
