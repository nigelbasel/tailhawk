//! The filter expression language — `SPEC.md` §7.2, implemented as written.
//!
//! §7.2 exists because `level >= Warning` appeared in three UI mockups, `severity_number >= 17` was
//! called the universal error predicate, and `--filter=EXPR` shipped on the CLI **while no document
//! defined the language**. It does now, in full, so this module is an implementation rather than a
//! design — and where it goes beyond §7.2 it says so.
//!
//! ## A chip, not a query box
//!
//! §7.2's design constraint: three of the owner's five daily-use features are *include filters*,
//! *exclude filters* and *multiple composing text filters*, and "a single text box cannot express
//! them". So the surface is a row of **chips**, each one predicate with its own polarity, composing
//! with implicit AND. What is parsed here is the contents of one chip; [`Chips`] composes them.
//!
//! ## Unknown is a third answer, and it is the subtle part
//!
//! §7.2: "a predicate naming a field the current format does not produce evaluates to **unknown**,
//! not false." An unknown predicate:
//!
//! - **excludes** the row when the chip is an *include* chip,
//! - **does not exclude** it when the chip is an *exclude* chip,
//! - and renders the chip in a **warning state** naming the missing field.
//!
//! Those two rules look asymmetric and are the same rule: **an unknown never causes an action.** An
//! include chip acts by keeping and needs a definite yes; an exclude chip acts by dropping and needs
//! a definite yes. [`Truth::acts`] is that sentence.
//!
//! §7.2 says why it matters, and it is not fussiness: "in the merged view, where sources have
//! different column sets — an exclude chip scoped to one source's column must not silently delete
//! every row from the others."
//!
//! **The Kleene table for `and` and `or` is ours**, because §7.2 states the unknown rule for a
//! predicate and then permits predicates to compose. It is forced rather than chosen: `unknown and
//! false` is **false** — the conjunction fails whatever the unknown turns out to be — while `unknown
//! and true` is still unknown. `or` is the mirror. Anything else would let an unknown field on one
//! side of an `or` swallow a definite match on the other.
//!
//! ## What is deliberately not here
//!
//! - **`source`** parses and always evaluates to unknown. §6.1 puts `resource` on the *pane*, not
//!   the row, and `Record` accordingly has no source field — the same reason `record.rs` gives for
//!   `ByteSpan` carrying no source id. When §8.3's merged view forces a pane model this resolves;
//!   until then the honest answer is the one §7.2 already defines for an unresolvable field.
//! - **The two-engine policy** of §7.4 — `fancy-regex` for lookaround, the 8 KB per-line cap, the
//!   backtrack limit. `/pattern/` here compiles through the `regex` crate and a pattern it rejects is
//!   a parse error. That is E15 and it is not smuggled in.
//! - **`not`.** §7.2's production list does not have it, and adding an operator to a language whose
//!   whole point was that nobody had written it down would be exactly the wrong move.

use std::fmt;

use crate::record::{AttributeValue, Record, Severity, SeverityBand, Timestamp};

/// Three-valued logic. See the module note on why there are three.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Truth {
    True,
    False,
    /// The predicate could not be evaluated — §7.2's field "the current format does not produce".
    Unknown,
}

impl Truth {
    /// Whether this answer is definite enough to act on. **The whole of §7.2's asymmetry.**
    ///
    /// An include chip acts by keeping and an exclude chip acts by dropping; both need a definite
    /// yes, and an unknown never causes either.
    pub fn acts(self) -> bool {
        self == Truth::True
    }

    fn of(yes: bool) -> Self {
        if yes {
            Truth::True
        } else {
            Truth::False
        }
    }

    /// Kleene conjunction. `unknown and false` is **false**: the conjunction fails whichever way the
    /// unknown resolves, so claiming not to know would be claiming less than is known.
    fn and(self, other: Self) -> Self {
        match (self, other) {
            (Truth::False, _) | (_, Truth::False) => Truth::False,
            (Truth::Unknown, _) | (_, Truth::Unknown) => Truth::Unknown,
            _ => Truth::True,
        }
    }

    /// Kleene disjunction, the mirror. `unknown or true` is **true**.
    fn or(self, other: Self) -> Self {
        match (self, other) {
            (Truth::True, _) | (_, Truth::True) => Truth::True,
            (Truth::Unknown, _) | (_, Truth::Unknown) => Truth::Unknown,
            _ => Truth::False,
        }
    }
}

/// A field a predicate can name. §7.2's `field` production.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Field {
    /// `level` and `severity`, both aliases for `severity_number` — §7.2, and the whole point of
    /// §6.2's banding.
    Severity,
    Timestamp,
    Body,
    /// Parses, never resolves. See the module note.
    Source,
    /// `trace` — the W3C trace id, compared as lower-case hex.
    Trace,
    /// `span` — the W3C span id. **Not** `Record::span`, which is a byte range; §7.2 named this
    /// field and the collision is in the spec, not introduced here.
    Span,
    /// `attributes.<key>`, or a bare column name from the detected format.
    Attribute(String),
}

impl Field {
    fn parse(name: &str) -> Self {
        match name.to_ascii_lowercase().as_str() {
            "level" | "severity" | "severity_number" => Field::Severity,
            "timestamp" => Field::Timestamp,
            "body" => Field::Body,
            "source" => Field::Source,
            "trace" => Field::Trace,
            "span" => Field::Span,
            _ => Field::Attribute(name.trim_start_matches("attributes.").to_string()),
        }
    }

    /// What to show in §7.2's warning state.
    pub fn name(&self) -> &str {
        match self {
            Field::Severity => "level",
            Field::Timestamp => "timestamp",
            Field::Body => "body",
            Field::Source => "source",
            Field::Trace => "trace",
            Field::Span => "span",
            Field::Attribute(key) => key,
        }
    }
}

/// §7.2's comparison operators.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Op {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    /// `like` — case-insensitive substring, which is what a log user means by it. §7.2 lists the
    /// operator and not its semantics; SQL wildcards would need an escape story for a language whose
    /// values are routinely paths and GUIDs.
    Like,
}

/// §7.2's `function` production.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Func {
    StartsWith,
    Contains,
    EndsWith,
}

/// §7.2's `value` production, resolved at parse time so a comparison knows what kind it is.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    /// A number, **and the text it was written as**.
    ///
    /// Both, because a token can be a number *and* an identifier: `span = 0101010101010101` parses
    /// as a perfectly good `f64` and comes back out as `101010101010101` with the leading zero gone,
    /// so a span id compared as text never matches. Log fields are full of all-digit identifiers —
    /// span ids, order numbers, correlation ids — and the one that is a number depends on the field,
    /// which the value does not know. Keeping both means the comparison decides, not the lexer.
    Number {
        value: f64,
        text: String,
    },
    Text(String),
    /// A severity *name*, resolved through the OTel banding — §7.2: "`level >= Warning` means
    /// `severity_number >= 13` and works identically across Serilog, log4net, NLog and syslog."
    Severity(Severity),
    Instant(Timestamp),
}

impl Value {
    /// Parses one value token. Ordering matters: a bare token that names a severity is a severity,
    /// and one that looks like an instant is an instant, before either can become text.
    fn parse(token: &str, quoted: bool) -> Self {
        if quoted {
            // §7.2: a quoted value "forces literal, even if it parses as an expression".
            return Value::Text(token.to_string());
        }
        if let Some(instant) = parse_instant(token) {
            return Value::Instant(instant);
        }
        if let Ok(number) = token.parse::<f64>() {
            return Value::Number {
                value: number,
                text: token.to_string(),
            };
        }
        if let Some(band) = SeverityBand::parse(token) {
            return Value::Severity(band.first());
        }
        Value::Text(token.to_string())
    }
}

/// One chip's contents. §7.2's `predicate`, with the composition it permits.
#[derive(Clone, Debug)]
pub enum Predicate {
    /// `bare_text` and `quoted` alike — §7.2: "case-insensitive substring over the whole record".
    ///
    /// **`finder` is a case-insensitive literal search compiled once**, because §7.3 runs a chip
    /// over every line of the file: the first version lower-cased the whole row per evaluation, an
    /// allocation and a full-row fold per line per chip, which is the cost of the whole pass. A
    /// literal under `(?i)` is a fixed-string search that touches no memory but the row. `folded`
    /// stays as the identity two chips are compared by.
    Text {
        folded: String,
        source: String,
        finder: regex::Regex,
    },
    Regex {
        source: String,
        re: regex::Regex,
    },
    Compare {
        field: Field,
        op: Op,
        value: Value,
    },
    In {
        field: Field,
        values: Vec<Value>,
    },
    Call {
        func: Func,
        field: Field,
        value: String,
    },
    And(Box<Predicate>, Box<Predicate>),
    Or(Box<Predicate>, Box<Predicate>),
}

impl PartialEq for Predicate {
    /// Compares regexes by their source, because `regex::Regex` is not `PartialEq` — two chips
    /// holding the same pattern are the same chip, which is what a UI comparing them means.
    fn eq(&self, other: &Self) -> bool {
        use Predicate::*;
        match (self, other) {
            (Text { folded: a, .. }, Text { folded: b, .. }) => a == b,
            (Regex { source: a, .. }, Regex { source: b, .. }) => a == b,
            (
                Compare {
                    field: fa,
                    op: oa,
                    value: va,
                },
                Compare {
                    field: fb,
                    op: ob,
                    value: vb,
                },
            ) => fa == fb && oa == ob && va == vb,
            (
                In {
                    field: fa,
                    values: a,
                },
                In {
                    field: fb,
                    values: b,
                },
            ) => fa == fb && a == b,
            (
                Call {
                    func: na,
                    field: fa,
                    value: va,
                },
                Call {
                    func: nb,
                    field: fb,
                    value: vb,
                },
            ) => na == nb && fa == fb && va == vb,
            (And(a1, a2), And(b1, b2)) | (Or(a1, a2), Or(b1, b2)) => a1 == b1 && a2 == b2,
            _ => false,
        }
    }
}

impl Predicate {
    /// Answers this predicate for one record. See the module note on the third answer.
    pub fn eval(&self, record: &Record) -> Truth {
        match self {
            // §7.2: "case-insensitive substring **over the whole record**", so this searches `raw`
            // rather than `body` — a user filtering for a correlation id does not care which field a
            // parser happened to put it in, and an unparsed line has only `raw` anyway.
            Predicate::Text { finder, .. } => Truth::of(finder.is_match(&record.raw)),
            Predicate::Regex { re, .. } => Truth::of(re.is_match(&record.raw)),
            Predicate::Compare { field, op, value } => compare(record, field, *op, value),
            Predicate::In { field, values } => values
                .iter()
                .map(|v| compare(record, field, Op::Eq, v))
                .fold(Truth::False, Truth::or),
            Predicate::Call { func, field, value } => {
                let Some(text) = resolve_text(record, field) else {
                    return Truth::Unknown;
                };
                let (text, needle) = (text.to_lowercase(), value.to_lowercase());
                Truth::of(match func {
                    Func::StartsWith => text.starts_with(&needle),
                    Func::Contains => text.contains(&needle),
                    Func::EndsWith => text.ends_with(&needle),
                })
            }
            Predicate::And(a, b) => a.eval(record).and(b.eval(record)),
            Predicate::Or(a, b) => a.eval(record).or(b.eval(record)),
        }
    }

    /// Every field this predicate names, for §7.2's warning state.
    ///
    /// Reported from the *expression*, not from an evaluation, so a chip can show its warning before
    /// a single row has been read — which is when the user is still typing and can fix it.
    pub fn fields(&self) -> Vec<&Field> {
        let mut out = Vec::new();
        self.collect_fields(&mut out);
        out
    }

    fn collect_fields<'a>(&'a self, out: &mut Vec<&'a Field>) {
        match self {
            Predicate::Text { .. } | Predicate::Regex { .. } => {}
            Predicate::Compare { field, .. }
            | Predicate::In { field, .. }
            | Predicate::Call { field, .. } => out.push(field),
            Predicate::And(a, b) | Predicate::Or(a, b) => {
                a.collect_fields(out);
                b.collect_fields(out);
            }
        }
    }
}

/// Resolves a field to text, or `None` for §7.2's unknown.
fn resolve_text(record: &Record, field: &Field) -> Option<String> {
    match field {
        Field::Body => Some(record.body.clone()),
        Field::Severity => record.severity_text.clone(),
        Field::Trace => record.trace.map(|t| hex(&t.trace_id)),
        Field::Span => record.trace.map(|t| hex(&t.span_id)),
        Field::Source => None,
        Field::Timestamp => None,
        Field::Attribute(key) => record.attribute(key).map(|v| match v {
            AttributeValue::String(s) => s.clone(),
            AttributeValue::Int(n) => n.to_string(),
            AttributeValue::Float(f) => f.to_string(),
            AttributeValue::Bool(b) => b.to_string(),
        }),
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// One comparison, with §7.2's typing rules.
///
/// **The unresolvable cases all return [`Truth::Unknown`] and none of them returns false.** That is
/// §7.2's rule and it is the one thing here that a plausible implementation gets wrong: a missing
/// field compared for inequality is *not* "different, therefore true".
fn compare(record: &Record, field: &Field, op: Op, value: &Value) -> Truth {
    // Severity is ordered, so it is compared as a number however the value was written.
    if *field == Field::Severity {
        let Some(actual) = record.severity_number else {
            return Truth::Unknown;
        };
        let wanted = match value {
            Value::Severity(s) => s.get() as f64,
            Value::Number { value, .. } => *value,
            // A severity compared against a *string* falls back to the text the writer used, which
            // is what `severity = "WRN"` means.
            Value::Text(_) | Value::Instant(_) => {
                return compare_text(record.severity_text.as_deref(), op, value)
            }
        };
        return compare_numbers(actual.get() as f64, op, wanted);
    }

    if *field == Field::Timestamp {
        let Some(actual) = record.timestamp else {
            return Truth::Unknown;
        };
        let Value::Instant(wanted) = value else {
            return Truth::Unknown;
        };
        return compare_numbers(actual.unix_nanos as f64, op, wanted.unix_nanos as f64);
    }

    // A numeric attribute compared against a number is compared as a number — `record.rs` types
    // `AttributeValue` for exactly this, so `duration_ms > 500` is not a string comparison.
    if let (Field::Attribute(key), Value::Number { value: wanted, .. }) = (field, value) {
        match record.attribute(key) {
            Some(AttributeValue::Int(n)) => return compare_numbers(*n as f64, op, *wanted),
            Some(AttributeValue::Float(f)) => return compare_numbers(*f, op, *wanted),
            // A Loki spill writes every attribute as text, so `status` arrives as `"444"`; text
            // that reads as a number, asked which side of a number it falls, answers as the
            // number. Equality stays textual: an all-digit identifier keeps the digits it was
            // written with, and `0088213` is not `88213`.
            Some(AttributeValue::String(text))
                if matches!(op, Op::Lt | Op::Le | Op::Gt | Op::Ge) =>
            {
                if let Ok(actual) = text.trim().parse::<f64>() {
                    return compare_numbers(actual, op, *wanted);
                }
            }
            None => return Truth::Unknown,
            _ => {}
        }
    }

    compare_text(resolve_text(record, field).as_deref(), op, value)
}

fn compare_numbers(actual: f64, op: Op, wanted: f64) -> Truth {
    Truth::of(match op {
        Op::Eq => actual == wanted,
        Op::Ne => actual != wanted,
        Op::Lt => actual < wanted,
        Op::Le => actual <= wanted,
        Op::Gt => actual > wanted,
        Op::Ge => actual >= wanted,
        // `like` on a number is a substring of how it is written, which is what a user filtering
        // `status like 50` means.
        Op::Like => format!("{actual}").contains(&format!("{wanted}")),
    })
}

fn compare_text(actual: Option<&str>, op: Op, value: &Value) -> Truth {
    let Some(actual) = actual else {
        return Truth::Unknown;
    };
    let wanted = match value {
        Value::Text(s) => s.clone(),
        // **The text, not the number.** See `Value::Number` -- reformatting loses a leading zero and
        // with it every all-digit identifier.
        Value::Number { text, .. } => text.clone(),
        Value::Severity(s) => s.band().name().to_string(),
        Value::Instant(_) => return Truth::Unknown,
    };
    // Case-insensitive throughout: §6.2 records that Loki's filed bugs came from case-sensitive
    // level matching, and a log user typing `error` does not mean `error` exactly.
    let (actual, wanted) = (actual.to_lowercase(), wanted.to_lowercase());
    Truth::of(match op {
        Op::Eq => actual == wanted,
        Op::Ne => actual != wanted,
        Op::Lt => actual < wanted,
        Op::Le => actual <= wanted,
        Op::Gt => actual > wanted,
        Op::Ge => actual >= wanted,
        Op::Like => actual.contains(&wanted),
    })
}

/// Whether a chip keeps rows or drops them. §7.2's per-chip polarity.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Polarity {
    Include,
    Exclude,
}

/// One filter chip: a predicate and what it does with a match.
#[derive(Clone, Debug, PartialEq)]
pub struct Chip {
    pub predicate: Predicate,
    pub polarity: Polarity,
    /// §7.1's per-rule enable/disable, for chips: a disabled chip keeps its place and its text and
    /// takes no part in [`Chips::keeps`] — `UI-DESIGN.md` §5's checkbox.
    pub enabled: bool,
    /// The text the user typed, kept so §7.2's "there is no hidden second representation" holds —
    /// a chip made by the UI's *"filter for this value"* is editable as the text it round-trips to.
    pub source: String,
}

impl Chip {
    pub fn parse(text: &str, polarity: Polarity) -> Result<Self, ParseError> {
        Ok(Self {
            predicate: parse(text)?,
            polarity,
            enabled: true,
            source: text.to_string(),
        })
    }
}

/// A row of chips, composed as §7.2 specifies.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Chips {
    pub chips: Vec<Chip>,
}

impl Chips {
    /// Whether a record survives. §7.2: "include chips AND together, then exclude chips are
    /// subtracted", and "order of chips is display order and does not affect the result".
    ///
    /// **No chips means everything survives**, which is the unfiltered view and not an empty one.
    pub fn keeps(&self, record: &Record) -> bool {
        let mut any_include = false;
        for chip in self.chips.iter().filter(|c| c.enabled) {
            match chip.polarity {
                Polarity::Include => {
                    any_include = true;
                    if !chip.predicate.eval(record).acts() {
                        return false;
                    }
                }
                Polarity::Exclude => {
                    if chip.predicate.eval(record).acts() {
                        return false;
                    }
                }
            }
        }
        let _ = any_include;
        true
    }
}

/// The filters as a dialog edits them: the rows, which is selected, and what has not been applied.
///
/// **A dialog, not the bar along the bottom.** The owner, 2026-10-06, twice: *"the filters should be
/// defined in a dialog, which will allow editing, adding and removing filters. The little bar at
/// the bottom is confusing and hard to notice"*, and then — on the Visual Studio comparison the old
/// design note leaned on — *"even if filters are a docked panel, the little line at the bottom is
/// not that. In visual studio, these are non-modal, dockable dialog boxes."* A non-modal dockable
/// editor is the eventual shape and waits on docking infrastructure this program does not have; a
/// dialog is what it gets meanwhile.
///
/// **Edits a copy and commits on demand**, which is what gives the dialog an OK, a Cancel and an
/// Apply. Apply is the owner's: *"if the filter dialog has an apply button, then it is easy for the
/// user to see the effect of the filter changes"* — so a reader can watch a filter take effect
/// without first losing the dialog that defined it.
///
/// Shaped after [`crate::ruleset::Editor`] deliberately, so the two dialogs behave the same way.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Editor {
    rows: Vec<Chip>,
    selected: Option<usize>,
    /// Whether the rows differ from what was last committed, which is what makes `Apply` live.
    dirty: bool,
    /// The row that is only an offer: it exists so the field has somewhere to write, and it is not
    /// a filter until something is typed into it. [`Self::commit`] leaves it out.
    offered: Option<usize>,
    /// A dialog opened to add, before anything is typed: the polarity the row typing makes will
    /// carry. Not a row, so nothing is listed and nothing reaches [`Self::commit`].
    pending: Option<Polarity>,
}

/// What an offered row reads until it is typed into. **A word the grammar accepts**, because the
/// dialog's first act cannot be an error message about a row the dialog itself just made.
const OFFERED: &str = "new";

impl Editor {
    /// Opens the editor over the filters in force.
    pub fn open(chips: &Chips) -> Self {
        Self {
            rows: chips.chips.clone(),
            selected: (!chips.chips.is_empty()).then_some(0),
            dirty: false,
            offered: None,
            pending: None,
        }
    }

    /// Opens it ready for a filter to be typed, carrying `polarity` for the row that typing makes.
    ///
    /// **This is what `Filter: include` opens.** The owner asked for a filter and was given a form
    /// holding that one filter and nothing else: *"the filter dialog does not show me a list of
    /// filters, and no way to delete, edit or activate/deactivate them"*. There is one filter
    /// surface, and asking to add arrives at the same list as asking to manage — the only
    /// difference being that the field is live with nothing selected, and which way round the
    /// row it makes will be. **No row is listed until something is typed**: this used to offer a
    /// row reading `new`, and the owner, 2026-10-28: *"every time I open it it appends an entry
    /// called new on the bottom, we really dont need to do that."*
    pub fn opened_to_add(chips: &Chips, polarity: Polarity) -> Self {
        let mut editor = Self::open(chips);
        editor.selected = None;
        editor.pending = Some(polarity);
        editor
    }

    /// The polarity a row typed into the empty field would carry, while nothing is selected and
    /// the dialog was opened to add.
    pub fn pending(&self) -> Option<Polarity> {
        self.pending
    }

    /// Offers a row to type into, selects it, and says where it went.
    ///
    /// **Not dirty, and not committed**, which is the whole difference from [`Self::add`]: a row
    /// nobody typed into is not a filter, so OK over an untouched offer puts nothing in force. The
    /// sources dialog learned the same lesson the hard way — an empty set had no row for the fields
    /// to write into, and every keystroke went nowhere.
    ///
    /// `None` only if [`OFFERED`] ever stops parsing, which a test holds it to.
    pub fn offer(&mut self, polarity: Polarity) -> Option<usize> {
        // **A row asked for by the button is the row the pending polarity was waiting to make**,
        // so nothing is left waiting beside it.
        self.pending = None;
        // **One offer at a time.** The dialog a `Ctrl+L` opens has an Add button in it, and a
        // second offer would displace the first — which would stop being an offer at that moment
        // and go into force on OK as a filter reading `new`. Asking again answers with the row
        // already waiting, turned the way round that was asked for.
        if let Some(at) = self.offered {
            self.selected = Some(at);
            if let Some(row) = self.rows.get_mut(at) {
                row.polarity = polarity;
            }
            return Some(at);
        }
        let chip = Chip::parse(OFFERED, polarity).ok()?;
        self.rows.push(chip);
        let at = self.rows.len() - 1;
        self.selected = Some(at);
        self.offered = Some(at);
        Some(at)
    }

    pub fn rows(&self) -> &[Chip] {
        &self.rows
    }

    pub fn selected(&self) -> Option<usize> {
        self.selected
    }

    /// The row that is only an offer, if there is one.
    ///
    /// The dialog asks because the caret belongs in the text field when a row is waiting to be
    /// typed into — and because [`OFFERED`] is already in that field, so it has to be selected or
    /// the first thing typed lands beside it rather than over it.
    pub fn offered(&self) -> Option<usize> {
        self.offered
    }

    pub fn dirty(&self) -> bool {
        self.dirty
    }

    /// Selects a row, or nothing. An index past the end selects nothing rather than panicking: the
    /// list control's notifications and this model can disagree for one message after a removal.
    pub fn select(&mut self, at: Option<usize>) {
        self.selected = at.filter(|n| *n < self.rows.len());
        if self.selected.is_some() {
            self.pending = None;
        }
    }

    /// Adds a filter, selects it, and reports where it went — or says why the text is not one.
    ///
    /// **The new row goes last and is selected**, because adding and then editing is one gesture
    /// and a reader should not have to find what they just made.
    pub fn add(&mut self, text: &str, polarity: Polarity) -> Result<usize, ParseError> {
        let chip = Chip::parse(text, polarity)?;
        self.rows.push(chip);
        self.selected = Some(self.rows.len() - 1);
        self.dirty = true;
        Ok(self.rows.len() - 1)
    }

    /// Replaces the selected filter's text, keeping its polarity and enabled state.
    ///
    /// A text that does not parse leaves the row exactly as it was and returns the fault, so a
    /// half-typed expression never destroys the one it is replacing.
    pub fn retext(&mut self, text: &str) -> Result<(), ParseError> {
        let Some(at) = self.selected else {
            // **Typing into the empty field of a dialog opened to add makes the row**, with the
            // polarity the command named; until then there is nothing in the list. A text that
            // does not parse makes nothing and keeps the polarity waiting.
            let Some(polarity) = self.pending else {
                return Ok(());
            };
            if text.trim().is_empty() {
                return Ok(());
            }
            self.add(text, polarity)?;
            self.pending = None;
            return Ok(());
        };
        let old = &self.rows[at];
        let mut chip = Chip::parse(text, old.polarity)?;
        chip.enabled = old.enabled;
        if self.rows[at] != chip {
            self.rows[at] = chip;
            self.dirty = true;
            // Typed into, so no longer merely offered. **Only when the text actually differs:**
            // the dialog fills this field programmatically whenever the selection moves, and that
            // write comes back as a keystroke the control cannot distinguish from a typist's.
            self.by_hand(at);
        }
        Ok(())
    }

    /// Sets the selected filter's polarity, or its enabled mark.
    ///
    /// **Either one retires an offer**, for the reason [`Self::retext`] does: three verbs can
    /// change a row and a row changed by hand is no longer merely on offer. Without this, turning
    /// a waiting row round and pressing OK produced no filter, and unticking `Enabled` made it
    /// vanish rather than becoming a disabled one — which §7.1 says keeps its place and its text.
    /// **Only when the value actually differs**, because the dialog asks for the value a row
    /// already has every time it refills its controls.
    pub fn set_polarity(&mut self, polarity: Polarity) {
        let Some(at) = self.selected else {
            if self.pending.is_some() {
                self.pending = Some(polarity);
            }
            return;
        };
        if let Some(row) = self.rows.get_mut(at) {
            if row.polarity != polarity {
                row.polarity = polarity;
                self.dirty = true;
                self.by_hand(at);
            }
        }
    }

    pub fn set_enabled(&mut self, at: usize, enabled: bool) {
        if let Some(row) = self.rows.get_mut(at) {
            if row.enabled != enabled {
                row.enabled = enabled;
                self.dirty = true;
                self.by_hand(at);
            }
        }
    }

    /// Notes that row `at` was changed by hand, so it is a filter rather than an offer.
    fn by_hand(&mut self, at: usize) {
        if self.offered == Some(at) {
            self.offered = None;
        }
    }

    /// Moves the selected filter one place, and keeps the selection on the row that moved.
    ///
    /// **Because dragging must not be the only way to reorder** — the accessibility rule that put
    /// `Move up` and `Move down` on the old panel's row menu, now the dialog's two buttons. Order
    /// is display order under §7.2, so nothing is filtered again by this; what has to be right is
    /// that the selection follows, or a second press acts on a different filter.
    ///
    /// The offer travels with its own row for the reason [`Self::remove`] shifts it: `offered`
    /// names an index, and a swap that steps over it would otherwise leave it naming a filter
    /// somebody typed.
    pub fn move_selected(&mut self, down: bool) -> bool {
        let Some(at) = self.selected else {
            return false;
        };
        let to = if down { at + 1 } else { at.wrapping_sub(1) };
        if to >= self.rows.len() {
            return false;
        }
        self.rows.swap(at, to);
        self.selected = Some(to);
        self.offered = match self.offered {
            Some(o) if o == at => Some(to),
            Some(o) if o == to => Some(at),
            keep => keep,
        };
        self.dirty = true;
        true
    }

    /// Enables or disables every filter of one polarity, leaving the other polarity alone.
    ///
    /// **A reader turns off every excluding filter to see what they were hiding**, and wants them
    /// back afterwards — which is §7.1's disabled filter, keeping its place and its text, done to
    /// a whole polarity at once.
    ///
    /// **An offered row takes no part**, because it is not a filter yet. Without that, `Disable
    /// all include` over a freshly offered row would count as changing it by hand and OK would put
    /// a filter reading [`OFFERED`] in force.
    pub fn set_all(&mut self, polarity: Polarity, enabled: bool) {
        for at in 0..self.rows.len() {
            if self.offered == Some(at) || self.rows[at].polarity != polarity {
                continue;
            }
            if self.rows[at].enabled != enabled {
                self.rows[at].enabled = enabled;
                self.dirty = true;
            }
        }
    }

    /// Drops every row, the offered one included: there is nothing left to select.
    pub fn remove_all(&mut self) {
        self.rows.clear();
        self.selected = None;
        self.offered = None;
        self.dirty = true;
    }

    /// Removes the selected filter and selects its neighbour.
    ///
    /// **The row after it, or the last one** — the same rule a list control follows, so removing
    /// several in a row is a repeated press rather than a press and a re-aim.
    pub fn remove(&mut self) -> bool {
        let Some(at) = self.selected else {
            return false;
        };
        self.rows.remove(at);
        self.selected = if self.rows.is_empty() {
            None
        } else {
            Some(at.min(self.rows.len() - 1))
        };
        self.offered = match self.offered {
            Some(o) if o == at => None,
            Some(o) if o > at => Some(o - 1),
            keep => keep,
        };
        self.dirty = true;
        true
    }

    /// The filters to put in force, and a note that they now are.
    ///
    /// **An offered row nobody typed into is left out**, so OK over the row the dialog itself put
    /// there does not add a filter reading `new`. The row stays in the list — this is Apply as well
    /// as OK, and a row vanishing from under the cursor on Apply would be its own defect.
    pub fn commit(&mut self) -> Chips {
        self.dirty = false;
        Chips {
            chips: self
                .rows
                .iter()
                .enumerate()
                .filter(|(at, _)| self.offered != Some(*at))
                .map(|(_, chip)| chip.clone())
                .collect(),
        }
    }
}

/// What a filter's own text says about itself — the phrase, and the two things the boxes beside it
/// describe.
///
/// **The text is the filter, and these are readings of it.** §7.2 allows no hidden second
/// representation: what the dialog shows is what runs, so `Regular expression` and `Match case` are
/// not flags stored alongside the expression but answers *about* it. Toggling one rewrites the
/// expression, and the next reading agrees with what was typed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Phrase {
    /// The words themselves, with the regex delimiters or the quotes taken off.
    pub text: String,
    pub regex: bool,
    pub match_case: bool,
    /// The expression is more than a phrase — a comparison, a call, an `and` — so neither box
    /// describes it and neither may be offered. Wrapping `level >= Warning` in slashes because a
    /// box was ticked would turn a comparison into a search for its own spelling.
    pub structured: bool,
}

/// `/` inside a regex, written so the grammar's delimiter does not end the pattern early.
///
/// **The lexer has taken `\/` since it was written** — *"otherwise a path pattern cannot be written
/// at all, which for a log tool is most of them"* — and the composer did not, so every phrase with
/// a slash in it composed to something that would not parse. `GET /api` became `/GET /api/`: an
/// empty regex, then `a`, `p` and `i` read as flags, then a stray delimiter.
fn escaped_delimiters(text: &str) -> String {
    text.replace('/', "\\/")
}

/// And back: the lexer unescapes `\/` into a literal slash, so a reading of the written text has
/// to do the same or the next write would escape the escape.
fn unescaped_delimiters(text: &str) -> String {
    text.replace("\\/", "/")
}

/// Reads a filter's text. See [`Phrase`].
pub fn phrase_of(source: &str) -> Phrase {
    let trimmed = source.trim();
    // `/…/` and `/…/i` are the grammar's regex forms, and the `i` is its only way of saying that
    // case does not matter.
    //
    // **The grammar decides, not the first and last characters.** `/a/ and /b/i` begins with a
    // delimiter and ends with one and is not a regex at all — it is an `and` of two — and reading
    // it as one claimed its text was `a/ and /b`, offered both boxes over it, and would have
    // rewritten the pair with only the second half's case changed. So the shape is taken from the
    // ends and then *checked* against what the expression actually parses to.
    let single_regex = matches!(
        Chip::parse(trimmed, Polarity::Include),
        Ok(Chip {
            predicate: Predicate::Regex { .. },
            ..
        })
    );
    if single_regex {
        if let Some(body) = trimmed.strip_prefix('/') {
            if let Some(inner) = body.strip_suffix("/i") {
                return Phrase {
                    text: unescaped_delimiters(inner),
                    regex: true,
                    ..Phrase::default()
                };
            }
            if let Some(inner) = body.strip_suffix('/') {
                return Phrase {
                    text: unescaped_delimiters(inner),
                    regex: true,
                    match_case: true,
                    ..Phrase::default()
                };
            }
        }
    }
    // **The grammar answers, not a second parser.** `Predicate::Text` carries the words with the
    // quotes already off, which is exactly the phrase — and anything that parses as more than text
    // is not a phrase at all. Text that does not parse is shown as typed: it is being written.
    match Chip::parse(trimmed, Polarity::Include) {
        Ok(Chip {
            predicate: Predicate::Text { source, .. },
            ..
        }) => Phrase {
            text: source,
            ..Phrase::default()
        },
        Ok(_) => Phrase {
            text: trimmed.to_owned(),
            structured: true,
            ..Phrase::default()
        },
        Err(_) => Phrase {
            text: trimmed.to_owned(),
            ..Phrase::default()
        },
    }
}

/// Writes a phrase and its two answers back as a filter.
///
/// **Ticking `Match case` over plain text changes the kind of filter**, because the grammar's only
/// case-sensitive form is a regex without the `i` flag. The text becomes one, the escaping keeps it
/// meaning its own letters, and reading it back says `Regular expression` — which is the truth and
/// is visible in the phrase box, rather than a box that quietly does nothing.
///
/// **The phrase itself is never edited to make it fit.** Three things used to be quietly changed
/// on the way through, each of which alters what the filter matches: a `/` ended the pattern early
/// and left something that would not parse, the words were trimmed, and a `"` was **deleted**.
/// A slash is escaped, the spacing is kept, and a phrase carrying a quotation mark — which the
/// grammar's quoted token has no escape for — is written as a case-insensitive regex over its own
/// letters, which is the same filter.
pub fn phrase_source(text: &str, regex: bool, match_case: bool) -> String {
    let literal = || escaped_delimiters(&crate::search::Pattern::escape(text));
    if regex {
        let body = escaped_delimiters(text);
        return if match_case {
            format!("/{body}/")
        } else {
            format!("/{body}/i")
        };
    }
    if match_case {
        return format!("/{}/", literal());
    }
    // Verbatim when the parser reads it back as exactly these words — `timeout`. Quoted when it
    // would otherwise read as an expression, or when the spacing would not survive, because
    // `level >= Warning` and `ERROR ` must both mean their own letters.
    if let Ok(Chip {
        predicate: Predicate::Text { source, .. },
        ..
    }) = Chip::parse(text, Polarity::Include)
    {
        if source == text {
            return text.to_owned();
        }
    }
    if text.contains('"') {
        return format!("/{}/i", literal());
    }
    format!("\"{text}\"")
}

/// Widths for the filters list's four columns — the filter, match case, regular expression, type —
/// apportioned to the width the list actually has.
///
/// The sources editor's `column_widths` is the same decision and carries the argument: fixed widths
/// overflowed a list nobody had measured, and a first draft of the arithmetic produced a column of
/// width -4 from the zero the shell reports when it cannot measure at all. Each column is floored
/// at something a word fits in and the rest is shared out.
pub fn column_widths(total: i32) -> [i32; 4] {
    const SHARE: [i32; 4] = [55, 12, 18, 15];
    const FLOOR: i32 = 36;
    let total = total.max(FLOOR * SHARE.len() as i32);
    let spare = i64::from(total - FLOOR * SHARE.len() as i32);
    let mut out = [FLOOR; 4];
    for (slot, share) in out.iter_mut().zip(SHARE) {
        *slot += (spare * i64::from(share) / 100) as i32;
    }
    // The remainder goes on the filter's own text, which is the column that wants it.
    out[0] += total - out.iter().sum::<i32>();
    out
}

/// Where a chip's text stopped making sense, and what was expected there.
///
/// **The offset is the point.** A chip is a small text field the user is typing into, and "expected
/// a value" without a position is a message that makes them re-read the whole thing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseError {
    pub at: usize,
    pub message: String,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} (at {})", self.message, self.at)
    }
}

impl std::error::Error for ParseError {}

/// Parses one chip's contents. §7.2's grammar, with its precedence.
pub fn parse(text: &str) -> Result<Predicate, ParseError> {
    let tokens = tokenise(text)?;
    let mut parser = Parser {
        tokens,
        at: 0,
        // The end of the *input*, not of the last token: a message about something missing belongs
        // where the caret is, which is after the trailing space the user has just typed.
        end: text.chars().count(),
    };
    let predicate = parser.expression()?;
    if let Some(token) = parser.peek() {
        return Err(ParseError {
            at: token.at,
            message: format!("unexpected `{}`", token.text),
        });
    }
    Ok(predicate)
}

#[derive(Clone, Debug, PartialEq)]
struct Token {
    text: String,
    at: usize,
    kind: Kind,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Kind {
    /// An unquoted run: a field name, an operator word, a value, or bare text.
    Word,
    /// `"…"` — §7.2's forced literal.
    Quoted,
    /// `/…/flags`.
    Regex,
    Symbol,
}

/// Splits a chip's text into tokens.
///
/// **Quoting and regex delimiters are recognised here and nowhere else**, so §7.2's "forces literal,
/// even if it parses as an expression" is a property of the lexer rather than a special case sitting
/// in the parser where it could be forgotten on one path.
///
/// ## `/` is ambiguous, and §7.2 does not resolve it
///
/// §7.2 gives `/pattern/` as a predicate and also permits a bare value — and in a log tool most
/// values are **paths**. `startsWith(path, /api)` reads as a function whose argument is `/api`, and
/// as a function whose argument opens an unterminated regex. Both are honest readings of the
/// grammar as written.
///
/// It is resolved the way every language with this problem resolves it: **`/` opens a regex only
/// where a predicate can begin** — at the start, or after `and`, `or` or `(`. Anywhere else it is an
/// ordinary character in a word. The alternative would be forcing every path value to be quoted,
/// which pushes the ambiguity onto the user in the case that is most common rather than least.
fn tokenise(text: &str) -> Result<Vec<Token>, ParseError> {
    let bytes: Vec<char> = text.chars().collect();
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let start = i;
        // True where §7.2 permits a predicate to start, which is the only place a `/` is a regex.
        let predicate_may_start = match tokens.last() {
            None => true,
            Some(Token { kind, text, .. }) => {
                (*kind == Kind::Symbol && text == "(")
                    || (*kind == Kind::Word
                        && (text.eq_ignore_ascii_case("and") || text.eq_ignore_ascii_case("or")))
            }
        };
        match bytes[i] {
            '/' if !predicate_may_start => {
                while i < bytes.len()
                    && !bytes[i].is_whitespace()
                    && !matches!(
                        bytes[i],
                        '(' | ')' | '[' | ']' | ',' | '=' | '!' | '<' | '>'
                    )
                {
                    i += 1;
                }
                tokens.push(Token {
                    text: bytes[start..i].iter().collect(),
                    at: start,
                    kind: Kind::Word,
                });
            }
            c if c.is_whitespace() => {
                i += 1;
            }
            '"' => {
                i += 1;
                let mut value = String::new();
                let mut closed = false;
                while i < bytes.len() {
                    if bytes[i] == '"' {
                        closed = true;
                        i += 1;
                        break;
                    }
                    value.push(bytes[i]);
                    i += 1;
                }
                if !closed {
                    return Err(ParseError {
                        at: start,
                        message: "unterminated quoted value".into(),
                    });
                }
                tokens.push(Token {
                    text: value,
                    at: start,
                    kind: Kind::Quoted,
                });
            }
            '/' => {
                i += 1;
                let mut pattern = String::new();
                let mut closed = false;
                while i < bytes.len() {
                    // `\/` is an escaped delimiter and stays in the pattern as a literal slash —
                    // otherwise a path pattern cannot be written at all, which for a log tool is
                    // most of them.
                    if bytes[i] == '\\' && i + 1 < bytes.len() && bytes[i + 1] == '/' {
                        pattern.push('/');
                        i += 2;
                        continue;
                    }
                    if bytes[i] == '/' {
                        closed = true;
                        i += 1;
                        break;
                    }
                    pattern.push(bytes[i]);
                    i += 1;
                }
                if !closed {
                    return Err(ParseError {
                        at: start,
                        message: "unterminated regex — expected a closing `/`".into(),
                    });
                }
                while i < bytes.len() && bytes[i].is_ascii_alphabetic() {
                    pattern.push('\u{0}');
                    pattern.push(bytes[i]);
                    i += 1;
                }
                tokens.push(Token {
                    text: pattern,
                    at: start,
                    kind: Kind::Regex,
                });
            }
            '(' | ')' | '[' | ']' | ',' => {
                i += 1;
                tokens.push(Token {
                    text: bytes[start].to_string(),
                    at: start,
                    kind: Kind::Symbol,
                });
            }
            '=' | '!' | '<' | '>' => {
                i += 1;
                if i < bytes.len() && bytes[i] == '=' {
                    i += 1;
                }
                tokens.push(Token {
                    text: bytes[start..i].iter().collect(),
                    at: start,
                    kind: Kind::Symbol,
                });
            }
            _ => {
                while i < bytes.len()
                    && !bytes[i].is_whitespace()
                    && !matches!(
                        bytes[i],
                        '(' | ')' | '[' | ']' | ',' | '=' | '!' | '<' | '>'
                    )
                {
                    i += 1;
                }
                tokens.push(Token {
                    text: bytes[start..i].iter().collect(),
                    at: start,
                    kind: Kind::Word,
                });
            }
        }
    }
    Ok(tokens)
}

struct Parser {
    tokens: Vec<Token>,
    at: usize,
    end: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.at)
    }

    fn peek_word(&self) -> Option<String> {
        self.tokens
            .get(self.at)
            .filter(|t| t.kind == Kind::Word)
            .map(|t| t.text.to_ascii_lowercase())
    }

    fn next(&mut self) -> Option<Token> {
        let token = self.tokens.get(self.at).cloned();
        if token.is_some() {
            self.at += 1;
        }
        token
    }

    fn end(&self) -> usize {
        self.end
    }

    fn expect_symbol(&mut self, symbol: &str) -> Result<(), ParseError> {
        match self.next() {
            Some(t) if t.kind == Kind::Symbol && t.text == symbol => Ok(()),
            Some(t) => Err(ParseError {
                at: t.at,
                message: format!("expected `{symbol}`, found `{}`", t.text),
            }),
            None => Err(ParseError {
                at: self.end(),
                message: format!("expected `{symbol}`"),
            }),
        }
    }

    /// §7.2's precedence, lowest binding first: `or`, then `and`, then everything else.
    fn expression(&mut self) -> Result<Predicate, ParseError> {
        let mut left = self.conjunction()?;
        while self.peek_word().as_deref() == Some("or") {
            self.at += 1;
            let right = self.conjunction()?;
            left = Predicate::Or(Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn conjunction(&mut self) -> Result<Predicate, ParseError> {
        let mut left = self.primary()?;
        while self.peek_word().as_deref() == Some("and") {
            self.at += 1;
            let right = self.primary()?;
            left = Predicate::And(Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn primary(&mut self) -> Result<Predicate, ParseError> {
        let Some(token) = self.next() else {
            return Err(ParseError {
                at: self.end(),
                message: "expected a predicate".into(),
            });
        };

        if token.kind == Kind::Symbol && token.text == "(" {
            let inner = self.expression()?;
            self.expect_symbol(")")?;
            return Ok(inner);
        }
        if token.kind == Kind::Quoted {
            return Ok(text_predicate(&token.text));
        }
        if token.kind == Kind::Regex {
            return build_regex(&token);
        }
        if token.kind == Kind::Symbol {
            return Err(ParseError {
                at: token.at,
                message: format!("unexpected `{}`", token.text),
            });
        }

        // A function call: `startsWith(field, value)`.
        if let Some(func) = function_named(&token.text) {
            if self.peek().is_some_and(|t| t.text == "(") {
                self.at += 1;
                let field = self.field()?;
                self.expect_symbol(",")?;
                let value = self.value_token()?;
                self.expect_symbol(")")?;
                return Ok(Predicate::Call {
                    func,
                    field,
                    value: value.text,
                });
            }
        }

        // `field in [a, b, c]`.
        if self.peek_word().as_deref() == Some("in") {
            self.at += 1;
            self.expect_symbol("[")?;
            let mut values = Vec::new();
            loop {
                let token = self.value_token()?;
                values.push(Value::parse(&token.text, token.kind == Kind::Quoted));
                match self.peek() {
                    Some(t) if t.text == "," => self.at += 1,
                    _ => break,
                }
            }
            self.expect_symbol("]")?;
            return Ok(Predicate::In {
                field: Field::parse(&token.text),
                values,
            });
        }

        // `field op value`.
        if let Some(op) = self.peek_operator() {
            self.at += 1;
            let value = self.value_token()?;
            return Ok(Predicate::Compare {
                field: Field::parse(&token.text),
                op,
                value: Value::parse(&value.text, value.kind == Kind::Quoted),
            });
        }

        // **Anything else is bare text**, which is §7.2's first production and the one a user reaches
        // for without knowing there is a grammar at all. Getting here means the run named no field
        // and carried no operator, so treating it as a search term is the reading that matches what
        // they typed.
        Ok(text_predicate(&token.text))
    }

    fn peek_operator(&self) -> Option<Op> {
        let token = self.tokens.get(self.at)?;
        match (token.kind, token.text.as_str()) {
            (Kind::Symbol, "=") => Some(Op::Eq),
            (Kind::Symbol, "!=") => Some(Op::Ne),
            (Kind::Symbol, "<") => Some(Op::Lt),
            (Kind::Symbol, "<=") => Some(Op::Le),
            (Kind::Symbol, ">") => Some(Op::Gt),
            (Kind::Symbol, ">=") => Some(Op::Ge),
            (Kind::Word, w) if w.eq_ignore_ascii_case("like") => Some(Op::Like),
            _ => None,
        }
    }

    fn field(&mut self) -> Result<Field, ParseError> {
        match self.next() {
            Some(t) if t.kind == Kind::Word || t.kind == Kind::Quoted => Ok(Field::parse(&t.text)),
            Some(t) => Err(ParseError {
                at: t.at,
                message: format!("expected a field name, found `{}`", t.text),
            }),
            None => Err(ParseError {
                at: self.end(),
                message: "expected a field name".into(),
            }),
        }
    }

    fn value_token(&mut self) -> Result<Token, ParseError> {
        match self.next() {
            Some(t) if t.kind == Kind::Word || t.kind == Kind::Quoted => Ok(t),
            Some(t) => Err(ParseError {
                at: t.at,
                message: format!("expected a value, found `{}`", t.text),
            }),
            None => Err(ParseError {
                at: self.end(),
                message: "expected a value".into(),
            }),
        }
    }
}

fn function_named(word: &str) -> Option<Func> {
    match word.to_ascii_lowercase().as_str() {
        "startswith" => Some(Func::StartsWith),
        "contains" => Some(Func::Contains),
        "endswith" => Some(Func::EndsWith),
        _ => None,
    }
}

fn text_predicate(text: &str) -> Predicate {
    Predicate::Text {
        folded: text.to_lowercase(),
        source: text.to_string(),
        finder: regex::RegexBuilder::new(&regex::escape(text))
            .case_insensitive(true)
            .build()
            .expect("an escaped literal always compiles"),
    }
}

/// Builds a regex node, translating a compile failure into a positioned parse error.
///
/// The lexer packs flags after a NUL so the token stays one string; unpacking here keeps the
/// knowledge of that in one place. §7.2 lists `i` and nothing else, and an unknown flag is an error
/// rather than being ignored — a filter that silently drops a flag is a filter that quietly does
/// something other than what it says.
fn build_regex(token: &Token) -> Result<Predicate, ParseError> {
    let mut parts = token.text.split('\u{0}');
    let pattern = parts.next().unwrap_or_default().to_string();
    let mut case_insensitive = false;
    for flag in parts {
        match flag {
            "i" => case_insensitive = true,
            other => {
                return Err(ParseError {
                    at: token.at,
                    message: format!("unknown regex flag `{other}` — only `i` is supported"),
                })
            }
        }
    }
    let built = regex::RegexBuilder::new(&pattern)
        .case_insensitive(case_insensitive)
        .build()
        .map_err(|e| ParseError {
            at: token.at,
            // The crate's own message names the offending construct and its position within the
            // pattern, which is more than this could say by paraphrasing it.
            message: format!("{e}"),
        })?;
    Ok(Predicate::Regex {
        source: pattern,
        re: built,
    })
}

/// Parses a strict ISO-8601 instant: `YYYY-MM-DD` with an optional `THH:MM[:SS[.fff…]]` and an
/// optional `Z` or `±HH:MM`.
///
/// **Ours, and deliberately narrow.** §7.2's `value` production admits an ISO-8601 instant and
/// `record.rs` carries no date library on purpose. Accepting only the shape above means no ambiguity
/// about what `2026-08` or `14/08/2026` mean — both are rejected and fall through to text, which is
/// the same answer §7.2 gives any other unrecognised token.
pub(crate) fn parse_instant(token: &str) -> Option<Timestamp> {
    let bytes = token.as_bytes();
    if bytes.len() < 10 || bytes[4] != b'-' || bytes[7] != b'-' {
        return None;
    }
    let year: i64 = token.get(0..4)?.parse().ok()?;
    let month: i64 = token.get(5..7)?.parse().ok()?;
    let day: i64 = token.get(8..10)?.parse().ok()?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }

    let mut rest = &token[10..];
    let (mut hour, mut minute, mut second, mut nanos) = (0i64, 0i64, 0i64, 0i64);
    if let Some(time) = rest.strip_prefix(['T', 't', ' ']) {
        rest = time;
        hour = take_two(&mut rest)?;
        rest = rest.strip_prefix(':')?;
        minute = take_two(&mut rest)?;
        if let Some(next) = rest.strip_prefix(':') {
            rest = next;
            second = take_two(&mut rest)?;
            if let Some(next) = rest.strip_prefix('.') {
                rest = next;
                let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
                if digits.is_empty() {
                    return None;
                }
                rest = &rest[digits.len()..];
                // Padded to nanoseconds so `.5` is half a second and not five.
                let padded = format!("{digits:0<9}");
                nanos = padded.get(0..9)?.parse().ok()?;
            }
        }
        if !(0..24).contains(&hour) || !(0..60).contains(&minute) || !(0..=60).contains(&second) {
            return None;
        }
    } else if !rest.is_empty() {
        return None;
    }

    let mut offset_minutes = 0i64;
    if !rest.is_empty() {
        match rest.as_bytes()[0] {
            b'Z' | b'z' if rest.len() == 1 => {}
            b'+' | b'-' if rest.len() == 6 && rest.as_bytes()[3] == b':' => {
                let sign = if rest.starts_with('-') { -1 } else { 1 };
                let h: i64 = rest.get(1..3)?.parse().ok()?;
                let m: i64 = rest.get(4..6)?.parse().ok()?;
                offset_minutes = sign * (h * 60 + m);
            }
            _ => return None,
        }
    }

    let secs = days_from_civil(year, month, day)? * 86_400 + hour * 3600 + minute * 60 + second
        - offset_minutes * 60;
    Some(Timestamp::new(
        secs.checked_mul(1_000_000_000)?.checked_add(nanos)?,
        i16::try_from(offset_minutes).ok()?,
    ))
}

fn take_two(rest: &mut &str) -> Option<i64> {
    let value = rest.get(0..2)?.parse().ok()?;
    *rest = &rest[2..];
    Some(value)
}

/// Days since 1970-01-01 for a proleptic Gregorian date.
///
/// **Derived, not transcribed.** The Gregorian rules are that a year is a leap year when it is
/// divisible by 4 and not by 100, or divisible by 400; shifting the year to start in March puts the
/// leap day last, which is what makes the month-length run expressible in closed form. `days_and_the_
/// epoch_agree_on_known_dates` pins it against dates whose day numbers are independently known.
pub(crate) fn days_from_civil(year: i64, month: i64, day: i64) -> Option<i64> {
    if day > days_in_month(year, month) {
        return None;
    }
    // March-based year: January and February belong to the previous one, so the leap day is at the
    // end and no month before it shifts.
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let year_of_era = y - era * 400;
    let month_shifted = if month > 2 { month - 3 } else { month + 9 };
    let day_of_year = (153 * month_shifted + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    Some(era * 146_097 + day_of_era - 719_468)
}

fn days_in_month(year: i64, month: i64) -> i64 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 => 29,
        2 => 28,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::{ParseState, TraceContext};

    fn chips_of(texts: &[(&str, Polarity)]) -> Chips {
        Chips {
            chips: texts
                .iter()
                .map(|(t, p)| Chip::parse(t, *p).expect("a plain word parses"))
                .collect(),
        }
    }

    /// **The editor adds, edits and removes, and nothing takes effect until it is committed.**
    ///
    /// Which is what makes the dialog's three buttons mean different things: Cancel is simply never
    /// committing, and Apply is committing without closing — the owner's, 2026-10-06, so the effect
    /// of a change can be seen while the dialog that made it is still open.
    #[test]
    fn the_editor_holds_its_changes_until_they_are_committed() {
        let live = chips_of(&[("error", Polarity::Include)]);
        let mut editor = Editor::open(&live);
        assert_eq!(editor.rows().len(), 1);
        assert_eq!(editor.selected(), Some(0), "the first row is selected");
        assert!(!editor.dirty(), "and nothing has changed yet");

        let at = editor.add("retry", Polarity::Exclude).expect("parses");
        assert_eq!(at, 1, "added last");
        assert_eq!(editor.selected(), Some(1), "and selected, ready to edit");
        assert!(editor.dirty());
        assert_eq!(live.chips.len(), 1, "the filters in force are untouched");

        let applied = editor.commit();
        assert_eq!(applied.chips.len(), 2);
        assert!(!editor.dirty(), "committing settles it");
    }

    /// **A text that does not parse leaves the row it would have replaced alone.**
    ///
    /// Half-typed expressions are the normal state of a text field, and one of them destroying the
    /// filter being edited would make the dialog hostile to use.
    #[test]
    fn a_filter_that_does_not_parse_changes_nothing() {
        let mut editor = Editor::open(&chips_of(&[("error", Polarity::Include)]));
        let before = editor.rows().to_vec();

        assert!(editor.retext("(unclosed").is_err(), "it is refused");
        assert_eq!(editor.rows(), before, "and the row stands");
        assert!(!editor.dirty(), "with nothing to apply");

        assert!(editor.add("(also unclosed", Polarity::Include).is_err());
        assert_eq!(editor.rows().len(), 1, "and nothing was added");

        editor.retext("warning").expect("this one parses");
        assert_eq!(editor.rows()[0].source, "warning");
        assert_eq!(
            editor.rows()[0].polarity,
            Polarity::Include,
            "retext keeps the polarity it had"
        );
        assert!(editor.dirty());
    }

    /// **Removing selects the neighbour**, the rule a list control follows, so removing several is
    /// a repeated press rather than a press and a re-aim.
    #[test]
    fn removing_a_filter_selects_the_next_one() {
        let mut editor = Editor::open(&chips_of(&[
            ("a", Polarity::Include),
            ("b", Polarity::Include),
            ("c", Polarity::Include),
        ]));
        editor.select(Some(1));
        assert!(editor.remove());
        assert_eq!(editor.rows().len(), 2);
        assert_eq!(editor.selected(), Some(1), "what was after it, now at 1");
        assert_eq!(editor.rows()[1].source, "c");

        editor.select(Some(1));
        assert!(editor.remove());
        assert_eq!(
            editor.selected(),
            Some(0),
            "the last row, once there is no after"
        );

        assert!(editor.remove());
        assert_eq!(editor.selected(), None, "and nothing, once there are none");
        assert!(!editor.remove(), "removing nothing is not a change");
    }

    /// **An enabled mark and a polarity are edits like any other**, and §7.1 keeps a disabled chip's
    /// place and text rather than dropping it — so the round trip through the editor must too.
    #[test]
    fn the_enabled_mark_and_the_polarity_survive_a_commit() {
        let mut editor = Editor::open(&chips_of(&[
            ("a", Polarity::Include),
            ("b", Polarity::Include),
        ]));
        editor.set_enabled(0, false);
        editor.select(Some(1));
        editor.set_polarity(Polarity::Exclude);

        let out = editor.commit();
        assert!(!out.chips[0].enabled, "still there, still off");
        assert_eq!(out.chips[0].source, "a", "and still readable");
        assert_eq!(out.chips[1].polarity, Polarity::Exclude);
    }

    /// A selection past the end selects nothing rather than panicking: a list control's
    /// notification and this model can disagree for one message after a removal.
    #[test]
    fn a_selection_past_the_end_is_no_selection() {
        let mut editor = Editor::open(&chips_of(&[("a", Polarity::Include)]));
        editor.select(Some(9));
        assert_eq!(editor.selected(), None);
        editor.retext("b").expect("parses");
        assert_eq!(editor.rows()[0].source, "a", "and edits nothing");
    }

    fn record(raw: &str) -> Record {
        Record::unparsed(raw)
    }

    fn parsed(body: &str, severity: u8, attrs: &[(&str, AttributeValue)]) -> Record {
        Record {
            body: body.to_string(),
            raw: body.to_string(),
            severity_number: Severity::new(severity),
            severity_text: Some(
                SeverityBand::parse("warn")
                    .expect("band")
                    .name()
                    .to_string(),
            ),
            attributes: attrs
                .iter()
                .map(|(k, v)| (k.to_string(), v.clone()))
                .collect(),
            parse_state: ParseState::Parsed,
            ..Record::default()
        }
    }

    fn truth(expr: &str, record: &Record) -> Truth {
        parse(expr).expect("parse").eval(record)
    }

    /// §7.2's first production, and the one a user reaches for without knowing there is a grammar.
    #[test]
    fn bare_text_is_a_case_insensitive_substring_of_the_whole_record() {
        let r = record("2026-08-14 WRN  Retry 3 of 5 for order 88213");
        assert_eq!(truth("retry", &r), Truth::True);
        assert_eq!(truth("RETRY", &r), Truth::True);
        assert_eq!(truth("refund", &r), Truth::False);
    }

    /// §7.2: a quoted value "forces literal, even if it parses as an expression".
    #[test]
    fn quoting_defeats_the_grammar() {
        let r = record("the config said level >= Warning and nobody noticed");
        assert_eq!(truth("\"level >= Warning\"", &r), Truth::True);
        // The same characters unquoted are a comparison, and an unparsed record has no severity.
        assert_eq!(truth("level >= Warning", &r), Truth::Unknown);
    }

    /// §7.2, and the whole point of §6.2: a severity *name* resolves through the OTel banding, so
    /// this works identically across Serilog, log4net, NLog and syslog.
    #[test]
    fn a_severity_name_resolves_through_the_banding() {
        let warn = parsed("something", 13, &[]);
        assert_eq!(truth("level >= Warning", &warn), Truth::True);
        assert_eq!(truth("level >= Error", &warn), Truth::False);
        // §6.2's universal error predicate, written the other way.
        assert_eq!(truth("severity_number >= 17", &warn), Truth::False);
        let err = parsed("something", 17, &[]);
        assert_eq!(truth("severity_number >= 17", &err), Truth::True);
        assert_eq!(truth("level >= warning", &err), Truth::True);
    }

    /// **§7.2's unknown rule, and the assertion that it is not false.** A plausible implementation
    /// returns false here and every test above still passes.
    #[test]
    fn a_field_the_format_does_not_produce_is_unknown_and_not_false() {
        let r = record("a line nothing parsed");
        assert_eq!(truth("level >= Warning", &r), Truth::Unknown);
        assert_eq!(truth("duration_ms > 500", &r), Truth::Unknown);
        // Inequality is the tempting one: "the field is missing, so it is *not* 500" is wrong.
        assert_eq!(truth("duration_ms != 500", &r), Truth::Unknown);
        assert_eq!(truth("source = api", &r), Truth::Unknown);
    }

    /// §7.2's two consequences of unknown, which look asymmetric and are one rule: **an unknown
    /// never causes an action.**
    ///
    /// §7.2 says why it matters: "in the merged view, where sources have different column sets — an
    /// exclude chip scoped to one source's column must not silently delete every row from the
    /// others."
    #[test]
    fn an_unknown_never_causes_an_action() {
        let r = record("a line nothing parsed");
        let include = Chips {
            chips: vec![Chip::parse("level >= Warning", Polarity::Include).expect("parse")],
        };
        assert!(!include.keeps(&r), "an include chip needs a definite yes");

        let exclude = Chips {
            chips: vec![Chip::parse("level >= Warning", Polarity::Exclude).expect("parse")],
        };
        assert!(
            exclude.keeps(&r),
            "an exclude chip needs a definite yes too"
        );
    }

    /// The Kleene table, which §7.2 forces the moment predicates compose. `unknown and false` is
    /// **false** — the conjunction fails whichever way the unknown resolves.
    #[test]
    fn an_unknown_does_not_swallow_a_definite_answer() {
        let r = record("nothing parsed here");
        assert_eq!(truth("level >= Warning and zzz", &r), Truth::False);
        assert_eq!(truth("level >= Warning and nothing", &r), Truth::Unknown);
        assert_eq!(truth("level >= Warning or nothing", &r), Truth::True);
        assert_eq!(truth("level >= Warning or zzz", &r), Truth::Unknown);
    }

    /// §7.2: "functions and comparisons bind tighter than `and`, which binds tighter than `or`;
    /// parentheses permitted."
    #[test]
    fn and_binds_tighter_than_or() {
        let r = parsed("timeout contacting billing", 17, &[]);
        // Parsed as `(a and b) or c`. Without the precedence this is `a and (b or c)`, which is true
        // for a record that has neither `a` nor `b` — so the two disagree and the fixture can tell.
        assert_eq!(truth("zzz and yyy or timeout", &r), Truth::True);
        assert_eq!(truth("zzz and (yyy or timeout)", &r), Truth::False);
    }

    /// §7.2's `membership` production.
    #[test]
    fn membership_is_an_or_over_equality() {
        let r = parsed(
            "a request",
            13,
            &[("service", AttributeValue::String("billing".into()))],
        );
        assert_eq!(truth("service in [orders, billing]", &r), Truth::True);
        assert_eq!(truth("service in [orders, shipping]", &r), Truth::False);
        assert_eq!(truth("level in [Warning, Error]", &r), Truth::True);
    }

    /// §7.2's `function` production, case-insensitively named because a user typing it in a chip is
    /// not consulting a reference.
    #[test]
    fn the_functions_work_however_they_are_capitalised() {
        let r = parsed(
            "a request",
            13,
            &[("path", AttributeValue::String("/api/orders".into()))],
        );
        assert_eq!(truth("startsWith(path, /api)", &r), Truth::True);
        assert_eq!(truth("STARTSWITH(path, /api)", &r), Truth::True);
        assert_eq!(truth("endsWith(path, orders)", &r), Truth::True);
        assert_eq!(truth("contains(path, api)", &r), Truth::True);
        assert_eq!(truth("startsWith(path, /web)", &r), Truth::False);
    }

    /// `record.rs` types `AttributeValue` for exactly this: `duration_ms > 500` is a different
    /// question from `"duration_ms" > "500"`, and a string comparison would say 90 > 500.
    #[test]
    fn a_numeric_attribute_compares_as_a_number() {
        let slow = parsed("req", 9, &[("duration_ms", AttributeValue::Int(900))]);
        let quick = parsed("req", 9, &[("duration_ms", AttributeValue::Int(90))]);
        assert_eq!(truth("duration_ms > 500", &slow), Truth::True);
        assert_eq!(truth("duration_ms > 500", &quick), Truth::False);
        let mut byte_sorted = ["900", "90", "500"];
        byte_sorted.sort();
        assert_eq!(
            byte_sorted,
            ["500", "90", "900"],
            "if a byte sort ordered these correctly the fixture would prove nothing"
        );
    }

    /// A Loki spill writes every attribute as a string, so `status` arrives as `"444"` and `bytes`
    /// as `"717"` — and `bytes >= 1000` compared as text says `"717" >= "1000"`, because `7` sorts
    /// after `1`. A number typed against an attribute that reads as one is a numeric question.
    #[test]
    fn a_numeric_attribute_stored_as_text_compares_as_a_number() {
        let small = parsed(
            "req",
            9,
            &[("bytes", AttributeValue::String("717".to_owned()))],
        );
        let large = parsed(
            "req",
            9,
            &[("bytes", AttributeValue::String("5794".to_owned()))],
        );
        let word = parsed(
            "req",
            9,
            &[("bytes", AttributeValue::String("-".to_owned()))],
        );
        assert_eq!(truth("bytes >= 1000", &small), Truth::False);
        assert_eq!(truth("bytes >= 1000", &large), Truth::True);
        assert_eq!(
            truth(
                "status >= 500",
                &parsed(
                    "req",
                    9,
                    &[("status", AttributeValue::String("502".to_owned()))]
                )
            ),
            Truth::True
        );
        assert_eq!(
            truth("bytes >= 1000", &word),
            Truth::False,
            "a value that is not a number is still compared as the text it is"
        );
        let order = parsed(
            "req",
            9,
            &[("order", AttributeValue::String("0088213".to_owned()))],
        );
        assert_eq!(
            truth("order = 88213", &order),
            Truth::False,
            "equality is still the text: an identifier keeps its leading zero"
        );
    }

    /// §7.2's `regex` production, and the escaped delimiter without which no path pattern is
    /// writable — which for a log tool is most of them.
    #[test]
    fn a_regex_predicate_compiles_and_matches() {
        let r = record("GET /api/orders/88213 200");
        assert_eq!(truth("/orders\\/\\d+/", &r), Truth::True);
        assert_eq!(truth("/ORDERS/", &r), Truth::False);
        assert_eq!(truth("/ORDERS/i", &r), Truth::True);
    }

    /// A pattern the engine rejects is a parse error at the chip, not a filter that quietly matches
    /// nothing.
    #[test]
    fn a_broken_pattern_is_reported_rather_than_matching_nothing() {
        let err = parse("/[unclosed/").expect_err("should not compile");
        assert_eq!(err.at, 0);
        assert!(!err.message.is_empty());
        let flag = parse("/x/q").expect_err("unknown flag");
        assert!(flag.message.contains('q'), "{}", flag.message);
    }

    /// The offset is what makes the message usable in a small text field.
    #[test]
    fn a_parse_error_says_where() {
        let err = parse("level >= ").expect_err("no value");
        assert_eq!(err.message, "expected a value");
        assert_eq!(err.at, 9);

        let err = parse("(level >= Warning").expect_err("unclosed");
        assert_eq!(err.message, "expected `)`");

        let err = parse("\"unterminated").expect_err("unterminated");
        assert_eq!(err.at, 0);
    }

    /// §7.2: "include chips AND together, then exclude chips are subtracted", and "order of chips is
    /// display order and does not affect the result".
    #[test]
    fn chips_compose_the_same_way_whatever_order_they_are_in() {
        let r = parsed("timeout contacting billing", 17, &[]);
        let include = Chip::parse("timeout", Polarity::Include).expect("parse");
        let exclude = Chip::parse("billing", Polarity::Exclude).expect("parse");

        let one = Chips {
            chips: vec![include.clone(), exclude.clone()],
        };
        let other = Chips {
            chips: vec![exclude, include],
        };
        assert!(!one.keeps(&r));
        assert_eq!(one.keeps(&r), other.keeps(&r));
    }

    /// No chips is the unfiltered view, not an empty one.
    #[test]
    fn no_chips_keeps_everything() {
        assert!(Chips::default().keeps(&record("anything at all")));
    }

    /// §7.2's warning state needs the fields named **before** a row has been read, because that is
    /// when the user is still typing and can fix it.
    #[test]
    fn a_predicate_reports_the_fields_it_names() {
        let p =
            parse("level >= Warning and startsWith(path, /api) or service in [a]").expect("parse");
        let named: Vec<&str> = p.fields().iter().map(|f| f.name()).collect();
        assert_eq!(named, ["level", "path", "service"]);
        // Bare text and regexes name no field, so a chip of either never warns.
        assert!(parse("timeout").expect("parse").fields().is_empty());
        assert!(parse("/time.?out/").expect("parse").fields().is_empty());
    }

    /// The trace fields are compared as lower-case hex, which is how they appear everywhere else.
    #[test]
    fn trace_and_span_compare_as_hex() {
        let r = Record {
            trace: Some(TraceContext {
                trace_id: [0xab; 16],
                span_id: [0x01; 8],
                trace_flags: 1,
            }),
            ..Record::unparsed("x")
        };
        assert_eq!(
            truth("trace = abababababababababababababababab", &r),
            Truth::True
        );
        assert_eq!(truth("span = 0101010101010101", &r), Truth::True);
        assert_eq!(truth("span = ffffffffffffffff", &r), Truth::False);
    }

    /// The instant parser, against dates whose day numbers are independently known: the epoch
    /// itself, the day after, and a leap day.
    #[test]
    fn days_and_the_epoch_agree_on_known_dates() {
        assert_eq!(days_from_civil(1970, 1, 1), Some(0));
        assert_eq!(days_from_civil(1970, 1, 2), Some(1));
        assert_eq!(days_from_civil(1969, 12, 31), Some(-1));
        assert_eq!(days_from_civil(2000, 3, 1), Some(11017));
        // 2000 is a leap year (divisible by 400); 1900 is not (divisible by 100, not 400).
        assert_eq!(days_from_civil(2000, 2, 29), Some(11016));
        assert_eq!(days_from_civil(1900, 2, 29), None);
        assert_eq!(days_from_civil(2026, 2, 29), None);
        assert_eq!(days_from_civil(2024, 2, 29), Some(19782));
    }

    #[test]
    fn an_instant_parses_with_or_without_a_time_and_an_offset() {
        let midnight = parse_instant("2026-08-14").expect("date only");
        assert_eq!(midnight.unix_nanos, 1_786_665_600 * 1_000_000_000);

        let zulu = parse_instant("2026-08-14T00:00:00Z").expect("zulu");
        assert_eq!(zulu.unix_nanos, midnight.unix_nanos);

        // An offset is subtracted to reach UTC and kept beside the instant, which is what
        // `Timestamp` exists to preserve.
        let offset = parse_instant("2026-08-14T01:00:00+01:00").expect("offset");
        assert_eq!(offset.unix_nanos, midnight.unix_nanos);
        assert_eq!(offset.utc_offset_minutes, 60);

        // Fractions pad to nanoseconds, so `.5` is half a second and not five of something.
        let half = parse_instant("2026-08-14T00:00:00.5Z").expect("fraction");
        assert_eq!(half.unix_nanos - midnight.unix_nanos, 500_000_000);
    }

    /// Anything not exactly the shape above falls through to text — which is the same answer §7.2
    /// gives any other unrecognised token, and is better than guessing at a date format.
    #[test]
    fn an_ambiguous_date_is_text_rather_than_a_guess() {
        for token in [
            "2026-08",
            "14/08/2026",
            "2026-13-01",
            "2026-08-32",
            "yesterday",
        ] {
            assert_eq!(parse_instant(token), None, "{token}");
        }
    }

    #[test]
    fn a_timestamp_comparison_orders_by_instant() {
        let r = Record {
            timestamp: Some(parse_instant("2026-08-14T12:00:00Z").expect("stamp")),
            ..Record::unparsed("x")
        };
        assert_eq!(truth("timestamp > 2026-08-14T11:00:00Z", &r), Truth::True);
        assert_eq!(truth("timestamp > 2026-08-14T13:00:00Z", &r), Truth::False);
        assert_eq!(truth("timestamp >= 2026-08-14", &r), Truth::True);
        // A record with no parsed timestamp is unknown, not false.
        assert_eq!(
            truth("timestamp > 2026-08-14", &record("unparsed")),
            Truth::Unknown
        );
    }

    /// §7.2: "A chip created by the per-field *filter for this value* is a normal comparison chip
    /// and is editable as text — there is no hidden second representation."
    #[test]
    fn a_chip_round_trips_through_its_own_text() {
        let chip = Chip::parse("service = billing", Polarity::Include).expect("parse");
        let again = Chip::parse(&chip.source, Polarity::Include).expect("reparse");
        assert_eq!(chip, again);
    }

    /// **`/` is ambiguous in §7.2 and this is the resolution.** A regex opens only where a predicate
    /// can begin; everywhere else `/` is an ordinary character, because in a log tool most values are
    /// paths. The alternative — quote every path — pushes the ambiguity onto the user in the common
    /// case rather than the rare one.
    #[test]
    fn a_path_value_is_not_a_regex() {
        let r = parsed(
            "req",
            9,
            &[("path", AttributeValue::String("/api/v2/orders".into()))],
        );
        assert_eq!(truth("path = /api/v2/orders", &r), Truth::True);
        assert_eq!(truth("startsWith(path, /api/v2)", &r), Truth::True);
        assert_eq!(truth("path in [/api/v2/orders, /health]", &r), Truth::True);
        // …and where a predicate *can* begin, it is still a regex.
        assert_eq!(
            truth("/api.v2/", &Record::unparsed("GET /api/v2")),
            Truth::True
        );
        assert_eq!(
            truth("zzz or /api.v2/", &Record::unparsed("GET /api/v2")),
            Truth::True
        );
    }

    /// **An all-digit identifier is not a number to be reformatted.** `0101010101010101` parses as a
    /// perfectly good `f64` and comes back as `101010101010101`, so a span id compared as text never
    /// matches. Log fields are full of these — span ids, order numbers, correlation ids — and which
    /// ones are numbers depends on the field, not on the token.
    #[test]
    fn an_all_digit_identifier_keeps_the_digits_it_was_written_with() {
        let r = parsed(
            "req",
            9,
            &[
                ("order", AttributeValue::String("0088213".into())),
                ("attempt", AttributeValue::Int(3)),
            ],
        );
        assert_eq!(truth("order = 0088213", &r), Truth::True);
        assert_eq!(truth("order = 88213", &r), Truth::False);
        // And a field that really is numeric still compares numerically.
        assert_eq!(truth("attempt > 2", &r), Truth::True);
    }

    /// Whitespace and case are how a person types, not how a grammar is written.
    #[test]
    fn spacing_and_keyword_case_do_not_change_the_meaning() {
        let r = parsed("timeout", 17, &[]);
        for expr in [
            "level>=Error",
            "level >= Error",
            "level  >=   error",
            "LEVEL >= ERROR",
        ] {
            assert_eq!(truth(expr, &r), Truth::True, "{expr}");
        }
        for expr in ["timeout AND level >= Error", "timeout and level >= Error"] {
            assert_eq!(truth(expr, &r), Truth::True, "{expr}");
        }
    }

    /// **Asking for a filter opens the list the filter is going into, with nothing listed until
    /// something is typed.**
    ///
    /// The owner added one and got a form with a single filter in it: *"the filter dialog does not
    /// show me a list of filters, and no way to delete, edit or activate/deactivate them"*. There is
    /// one filter surface, and `Filter: include` arrives at it the same way `Filters…` does — the
    /// difference being that the field is live with nothing selected, and the row it makes
    /// carries the polarity the command named. **And no row until then**, the owner again on
    /// 2026-10-28: *"every time I open it it appends an entry called new on the bottom, we really
    /// dont need to do that."*
    #[test]
    fn asking_to_add_a_filter_opens_the_whole_list_and_lists_nothing_until_typed() {
        let live = chips_of(&[("error", Polarity::Include)]);
        let mut editor = Editor::opened_to_add(&live, Polarity::Exclude);

        assert_eq!(
            editor.rows().len(),
            1,
            "the filters already in force are shown, and nothing called `new` under them"
        );
        assert_eq!(
            editor.selected(),
            None,
            "nothing selected: the field is for the new one"
        );
        assert_eq!(
            editor.pending(),
            Some(Polarity::Exclude),
            "the polarity the command asked for waits for the text"
        );
        assert!(!editor.dirty(), "nothing has happened yet");

        editor.retext("retry").expect("parses");
        assert_eq!(editor.rows().len(), 2, "the first text typed makes the row");
        assert_eq!(
            editor.selected(),
            Some(1),
            "and it is the selected one, so typing goes on"
        );
        assert_eq!(editor.rows()[1].polarity, Polarity::Exclude);
        assert!(
            editor.rows()[1].enabled,
            "a filter you just asked for is on"
        );
        assert_eq!(editor.pending(), None);
        assert!(editor.dirty());
        editor.retext("retrying").expect("parses");
        assert_eq!(
            editor.rows().len(),
            2,
            "the next keystroke edits that row, not another"
        );
        assert_eq!(editor.rows()[1].source, "retrying");

        // An empty set: still no row until typed, and typing makes the first.
        let mut first = Editor::opened_to_add(&Chips::default(), Polarity::Include);
        assert!(first.rows().is_empty());
        first.retext("x").expect("parses");
        assert_eq!(first.rows().len(), 1);
        assert_eq!(first.selected(), Some(0));
    }

    /// **Nothing typed, nothing in force**: OK over the empty field puts exactly what was there.
    /// A text that does not parse makes no row and keeps the polarity waiting; a blank one is not
    /// a text.
    #[test]
    fn a_row_nobody_typed_into_is_not_a_filter() {
        let live = chips_of(&[("error", Polarity::Include)]);

        let mut untouched = Editor::opened_to_add(&live, Polarity::Exclude);
        assert!(!untouched.dirty(), "opening to add is not a change");
        assert_eq!(
            untouched.commit().chips,
            live.chips,
            "OK over the empty field puts exactly what was already in force"
        );
        assert_eq!(
            untouched.rows().len(),
            1,
            "and lists nothing it did not have"
        );
        assert_eq!(untouched.offered(), None, "no offered row either");

        let mut blank = Editor::opened_to_add(&live, Polarity::Exclude);
        blank
            .retext("   ")
            .expect("a blank is nothing, not an error");
        assert_eq!(blank.rows().len(), 1);
        assert_eq!(blank.pending(), Some(Polarity::Exclude));
        assert!(
            blank.retext("/(/").is_err(),
            "a text that does not parse is reported"
        );
        assert_eq!(blank.rows().len(), 1, "and makes no row");
        assert_eq!(
            blank.pending(),
            Some(Polarity::Exclude),
            "the polarity goes on waiting for a text that does"
        );

        let mut typed = Editor::opened_to_add(&live, Polarity::Exclude);
        typed.retext("retry").expect("parses");
        let applied = typed.commit();
        assert_eq!(applied.chips.len(), 2, "typed, it is a filter");
        assert_eq!(applied.chips[1].source, "retry");
        assert_eq!(applied.chips[1].polarity, Polarity::Exclude);

        // Picking a row is editing that row: nothing is left waiting beside it, and typing goes
        // into the row picked rather than making another.
        let mut picked = Editor::opened_to_add(&live, Polarity::Exclude);
        picked.select(Some(0));
        assert_eq!(picked.pending(), None, "a row picked, nothing pending");
        picked.retext("errors").expect("parses");
        assert_eq!(picked.rows().len(), 1, "edited, not added");
        assert_eq!(picked.rows()[0].source, "errors");
    }

    /// **The Add button still offers a row, and asking twice leaves one row, not two.**
    ///
    /// A button pressed is a row asked for, so `Add` lists one reading `new` to type into; a
    /// second press would displace the first — which would stop being an offer at that moment and
    /// go into force on OK as a filter reading `new`. There is one offer at a time, and pressing
    /// `Add` in a dialog opened to add is the row the pending polarity was waiting to make.
    #[test]
    fn asking_twice_for_a_row_to_type_into_leaves_one() {
        let live = chips_of(&[("error", Polarity::Include)]);
        let mut editor = Editor::opened_to_add(&live, Polarity::Exclude);
        let first = editor.offer(Polarity::Exclude).expect("a row is offered");
        assert_eq!(editor.rows().len(), 2);
        assert_eq!(
            editor.rows()[first].polarity,
            Polarity::Exclude,
            "the button's row is the one the pending polarity was waiting to make"
        );
        assert_eq!(
            editor.pending(),
            None,
            "so nothing is left waiting beside it"
        );
        editor.retext("x").expect("parses");
        assert_eq!(
            editor.rows().len(),
            2,
            "typing edits the offered row, not a pending one"
        );

        let mut editor = Editor::opened_to_add(&live, Polarity::Include);
        let first = editor.offer(Polarity::Include).expect("a row is offered");
        assert_eq!(
            editor.offer(Polarity::Exclude),
            Some(first),
            "the row already waiting is the answer, not a second one"
        );
        assert_eq!(editor.rows().len(), 2, "so no row was added");
        assert_eq!(editor.selected(), Some(first), "and it is the selected row");
        assert_eq!(
            editor.rows()[first].polarity,
            Polarity::Exclude,
            "asking again may change which way round it is"
        );
        assert_eq!(
            editor.commit().chips,
            live.chips,
            "nothing reading `new` reached the view"
        );

        // Typed into, it is a filter — and *then* Add offers a fresh row.
        editor.retext("retry").expect("parses");
        let second = editor.offer(Polarity::Include).expect("a new offer");
        assert_ne!(second, first, "the typed row is not reused");
        assert_eq!(editor.rows().len(), 3);
        assert_eq!(
            editor.commit().chips.len(),
            2,
            "two filters: the one in force and the one typed"
        );
    }

    /// **Turning the pair round before typing turns the row that typing makes**; turning an
    /// offered row round, or off, is typing into it.
    ///
    /// Three verbs can change an offered row and only one of them retired the offer, so clicking
    /// `Exclude matching lines` on a waiting row and pressing OK produced no filter at all, and
    /// unticking `Enabled` made the row vanish instead of becoming a disabled filter — which §7.1
    /// says keeps its place *and* its text. The dialog acknowledged the press and OK did nothing.
    #[test]
    fn turning_the_offered_row_round_or_off_makes_it_a_filter() {
        let turned_before_typing = {
            let mut editor = Editor::opened_to_add(&Chips::default(), Polarity::Include);
            editor.set_polarity(Polarity::Exclude);
            assert!(
                editor.rows().is_empty(),
                "turning the pair makes no row by itself"
            );
            assert_eq!(editor.pending(), Some(Polarity::Exclude));
            editor.retext("x").expect("parses");
            editor.commit()
        };
        assert_eq!(turned_before_typing.chips.len(), 1);
        assert_eq!(turned_before_typing.chips[0].polarity, Polarity::Exclude);

        let turned = {
            let mut editor = Editor::open(&Chips::default());
            editor.offer(Polarity::Include);
            editor.set_polarity(Polarity::Exclude);
            assert_eq!(
                editor.offered(),
                None,
                "changed by hand, so no longer an offer"
            );
            editor.commit()
        };
        assert_eq!(turned.chips.len(), 1, "the row he turned round is a filter");
        assert_eq!(turned.chips[0].polarity, Polarity::Exclude);

        let switched_off = {
            let mut editor = Editor::open(&Chips::default());
            editor.offer(Polarity::Include);
            editor.set_enabled(0, false);
            assert_eq!(editor.offered(), None);
            editor.commit()
        };
        assert_eq!(
            switched_off.chips.len(),
            1,
            "§7.1: a disabled filter keeps its place and its text"
        );
        assert!(!switched_off.chips[0].enabled);

        // The programmatic refills are what the dialog's quiet flag is for, and they ask for the
        // value the row already has — which must leave the offer alone.
        let mut untouched = Editor::open(&Chips::default());
        untouched.offer(Polarity::Include);
        untouched.set_polarity(Polarity::Include);
        untouched.set_enabled(0, true);
        assert_eq!(
            untouched.offered(),
            Some(0),
            "asking for what it already is changes nothing"
        );
        assert!(untouched.commit().chips.is_empty());
    }

    /// The word an offered row carries has to be one §7.2 accepts, or the dialog's first act is an
    /// error message about a row the dialog itself made. [`Editor::offer`] can only fail if this
    /// stops being true.
    #[test]
    fn the_offered_row_is_a_filter_the_grammar_accepts() {
        assert!(
            Chip::parse(OFFERED, Polarity::Include).is_ok(),
            "`{OFFERED}` has to parse, or `offer` answers None and no row appears at all"
        );
        let mut editor = Editor::default();
        assert_eq!(editor.offer(Polarity::Include), Some(0));
        assert_eq!(
            editor.rows()[0].source,
            OFFERED,
            "and it reads as that word"
        );
    }

    /// Removing a row below the offered one keeps the offer pointing at the row it was made for —
    /// the drift the sources editor was caught by, where a secret typed for one source would have
    /// been stored against the next one along.
    #[test]
    fn removing_a_row_below_the_offer_does_not_move_the_offer() {
        let live = chips_of(&[("a", Polarity::Include), ("b", Polarity::Include)]);
        let mut editor = Editor::open(&live);
        editor.offer(Polarity::Include);
        assert_eq!(editor.selected(), Some(2));

        editor.select(Some(0));
        assert!(editor.remove(), "the first real filter goes");
        assert_eq!(editor.rows().len(), 2);

        assert_eq!(
            editor.commit().chips.len(),
            1,
            "what is left is `b` alone: the offer is still an offer, not a filter"
        );
        assert_eq!(editor.commit().chips[0].source, "b");
    }

    /// **What a filter's own text says about itself**, which is what the two boxes beside the
    /// phrase are reading. The grammar has exactly three shapes to tell apart, and the fourth
    /// case — an expression that is not a phrase at all — must say so rather than answer `No`
    /// twice and invite a click that would wrap `level >= Warning` in slashes.
    #[test]
    fn a_filter_says_whether_it_is_a_regular_expression_and_whether_case_matters() {
        let read = |source: &str| {
            let p = phrase_of(source);
            (p.text, p.regex, p.match_case, p.structured)
        };
        assert_eq!(
            read("timeout"),
            ("timeout".to_owned(), false, false, false),
            "plain text is §7.2's case-insensitive substring"
        );
        assert_eq!(
            read("/err.*/i"),
            ("err.*".to_owned(), true, false, false),
            "the i flag is the grammar's only way to say case does not matter in a regex"
        );
        assert_eq!(
            read("/err.*/"),
            ("err.*".to_owned(), true, true, false),
            "and without it, case matters"
        );
        assert!(
            read("level >= Warning").3,
            "a comparison is not a phrase, and neither box describes it"
        );
        assert!(read("contains(message, \"timeout\")").3, "nor is a call");
        // A lone slash is not a regex, and must not be read as one — `strip_prefix` alone would.
        assert_eq!(read("/"), ("/".to_owned(), false, false, false));
    }

    /// **The two boxes write back through the same reading**, so a filter the dialog composed is
    /// one the dialog can read again. Ticking `Match case` over plain text is the one case that
    /// changes the *kind* of filter — the grammar's only case-sensitive form is a regex, so the
    /// text becomes one and the `Regular expression` box ticks itself. That is surfaced rather
    /// than hidden: the phrase box shows the slashes.
    #[test]
    fn what_the_boxes_write_is_what_the_boxes_read() {
        for (text, regex, case) in [
            ("timeout", false, false),
            ("err.*", true, false),
            ("err.*", true, true),
        ] {
            let source = phrase_source(text, regex, case);
            let back = phrase_of(&source);
            assert_eq!(
                (back.text.as_str(), back.regex, back.match_case),
                (text, regex, case),
                "{source}"
            );
        }
        assert_eq!(
            phrase_source("timeout", false, true),
            "/timeout/",
            "case-sensitive plain text has to become a regex: the grammar has no other form"
        );
        assert!(
            phrase_of(&phrase_source("timeout", false, true)).regex,
            "and it says so when read back, rather than claiming to be plain text"
        );
        // Text that would parse as something else is quoted, or `level >= Warning` typed into the
        // phrase box would silently become a comparison rather than the words themselves.
        assert_eq!(
            phrase_source("level >= Warning", false, false),
            "\"level >= Warning\""
        );
    }

    /// **Up and Down, because dragging must not be the only way** — the rule that put `Move up`
    /// on the panel's row menu before the panel went. Order is display order under §7.2, so
    /// nothing is filtered again; what matters is that the selection follows the row it moved,
    /// or the second press acts on a different filter.
    #[test]
    fn a_filter_moves_up_and_down_and_the_selection_goes_with_it() {
        let live = chips_of(&[
            ("a", Polarity::Include),
            ("b", Polarity::Include),
            ("c", Polarity::Include),
        ]);
        let mut editor = Editor::open(&live);
        editor.select(Some(2));
        assert!(editor.move_selected(false), "c moves up");
        assert_eq!(editor.selected(), Some(1));
        let order: Vec<&str> = editor.rows().iter().map(|c| c.source.as_str()).collect();
        assert_eq!(order, ["a", "c", "b"]);

        assert!(editor.move_selected(true), "and back down");
        assert_eq!(editor.selected(), Some(2));

        assert!(!editor.move_selected(true), "nothing below the last");
        editor.select(Some(0));
        assert!(!editor.move_selected(false), "nor above the first");
    }

    /// An offered row is not a filter, so a move that steps over it carries the offer with it —
    /// the same bookkeeping `remove` does, for the same reason: `offered` names an index.
    #[test]
    fn moving_a_row_past_the_offer_keeps_the_offer_on_its_own_row() {
        let live = chips_of(&[("a", Polarity::Include), ("b", Polarity::Include)]);
        let mut editor = Editor::open(&live);
        editor.offer(Polarity::Include);
        assert_eq!(editor.offered(), Some(2));
        editor.select(Some(1));
        assert!(editor.move_selected(true), "b swaps with the offered row");
        assert_eq!(editor.offered(), Some(1), "the offer moved with its row");
        assert_eq!(
            editor.commit().chips.len(),
            2,
            "and is still not a filter: a and b"
        );
    }

    /// **The four bulk verbs act on a polarity**, which is what makes them worth having: a reader
    /// turns off every excluding filter to see what they were hiding, without losing them.
    ///
    /// **A row nobody typed into is left out of all four**, because it is not a filter yet — and
    /// `Disable all include` over a freshly offered row would otherwise mark it changed by hand
    /// and put a filter reading `new` in force.
    #[test]
    fn enabling_or_disabling_every_filter_of_one_polarity_leaves_the_others_alone() {
        let live = chips_of(&[
            ("a", Polarity::Include),
            ("b", Polarity::Exclude),
            ("c", Polarity::Include),
        ]);
        let mut editor = Editor::open(&live);
        editor.offer(Polarity::Include);
        assert_eq!(editor.offered(), Some(3));

        editor.set_all(Polarity::Include, false);
        let state: Vec<bool> = editor.rows().iter().map(|c| c.enabled).collect();
        assert_eq!(
            state,
            [false, true, false, true],
            "both includes off, the exclude untouched, and the offered row untouched"
        );
        assert_eq!(editor.offered(), Some(3), "still only an offer");
        assert_eq!(
            editor.commit().chips.len(),
            3,
            "so OK adds no fourth filter"
        );

        editor.set_all(Polarity::Include, true);
        assert!(editor.rows().iter().all(|c| c.enabled));
    }

    /// Remove all empties the list and leaves nothing selected — including the offered row, which
    /// is on the list even though it is not a filter.
    #[test]
    fn remove_all_empties_the_list() {
        let live = chips_of(&[("a", Polarity::Include), ("b", Polarity::Exclude)]);
        let mut editor = Editor::open(&live);
        editor.offer(Polarity::Include);
        editor.remove_all();
        assert!(editor.rows().is_empty());
        assert_eq!(editor.selected(), None);
        assert_eq!(editor.offered(), None);
        assert!(editor.dirty(), "there is something to apply");
        assert!(editor.commit().chips.is_empty());
    }

    /// The four columns fit the list they are in, and the narrowest case the shell can hand over
    /// — zero, which is what `GetClientRect` reports when it cannot measure — still produces four
    /// columns a word fits in. `sourceset::column_widths` was caught giving one a width of -4.
    #[test]
    fn the_filter_lists_columns_fit_the_width_they_are_given() {
        for total in [0, 1, 120, 520, 1400] {
            let widths = column_widths(total);
            assert!(
                widths.iter().all(|w| *w >= 36),
                "{total}: {widths:?} has a column too narrow to read"
            );
            if total >= 36 * 4 {
                assert_eq!(
                    widths.iter().sum::<i32>(),
                    total,
                    "{total}: the row fills the list exactly"
                );
            }
        }
        assert!(
            column_widths(520)[0] > column_widths(520)[3],
            "the filter's own text gets the room"
        );
    }

    /// **A phrase with a slash in it is the commonest phrase a log tool has**, and both option
    /// boxes used to make one unparsable: `/api` composed as `//api/`, which the lexer reads as an
    /// empty regex followed by the flags `a`, `p`, `i` and then a stray delimiter. The grammar has
    /// had `\/` since it was written — its own comment says *"otherwise a path pattern cannot be
    /// written at all, which for a log tool is most of them"* — and this is the composer finally
    /// using it.
    #[test]
    fn a_phrase_with_a_slash_in_it_survives_both_boxes() {
        for (text, regex, case) in [
            ("GET /api/v2", false, false),
            ("GET /api/v2", false, true),
            ("GET /api/v2", true, true),
            ("GET /api/v2", true, false),
        ] {
            let source = phrase_source(text, regex, case);
            assert!(
                Chip::parse(&source, Polarity::Include).is_ok(),
                "{source:?} does not parse"
            );
            let back = phrase_of(&source);
            assert_eq!(back.text, text, "{source:?} reads back as something else");
            // Case matters either way round; whether it is *called* a regular expression
            // afterwards is `what_the_boxes_write_is_what_the_boxes_read`'s subject, because
            // ticking `Match case` over plain text is how the grammar says case-sensitive.
            assert_eq!(back.match_case, case, "{source:?}");
            if regex {
                assert!(back.regex, "{source:?} stopped being a regular expression");
            }
        }
    }

    /// **Whitespace inside a phrase is part of the phrase.** `/ERROR /i` and `/^\s+at /i` are both
    /// filters somebody wrote on purpose — the trailing space is what keeps `ERROR ` off
    /// `ERRORS`, and a stack-trace filter is nothing without it. Composing used to trim, so
    /// clicking either box widened the filter with nothing on screen saying so.
    #[test]
    fn whitespace_inside_a_phrase_is_part_of_it() {
        for (text, regex, case) in [
            ("ERROR ", true, false),
            ("ERROR ", false, false),
            (" at ", false, false),
        ] {
            let source = phrase_source(text, regex, case);
            assert!(
                Chip::parse(&source, Polarity::Include).is_ok(),
                "{source:?} does not parse"
            );
            assert_eq!(
                phrase_of(&source).text,
                text,
                "{source:?} lost the space it was written with"
            );
        }
    }

    /// **A quotation mark cannot be written inside a quoted value**, because the lexer's quoted
    /// token has no escape — so the composer used to *delete* it, and `say "hi"` quietly became
    /// `say hi`, which matches different lines. The grammar's regex token takes a `"` happily, and
    /// a case-insensitive regex over the escaped letters is the same filter, so that is what it
    /// composes instead of mangling the words.
    #[test]
    fn a_phrase_with_a_quotation_mark_keeps_it() {
        let source = phrase_source("say \"hi\"", false, false);
        let chip = Chip::parse(&source, Polarity::Include).expect("composes to something legal");
        let said = record("he did say \"hi\" again");
        assert!(
            chip.predicate.eval(&said).acts(),
            "{source:?} no longer matches the line it was written for"
        );
        let other = record("he did say hi again");
        assert!(
            !chip.predicate.eval(&other).acts(),
            "{source:?} matches a line without the quotes, so the quotes were dropped"
        );
    }

    /// **Two regexes joined by `and` are not one regex**, however the whole expression begins and
    /// ends. Reading the delimiters alone said `/a/ and /b/i` was a case-insensitive regex whose
    /// text is `a/ and /b` — so the list claimed Yes and No about it, both boxes were offered, and
    /// ticking one would have rewritten the pair with only the second half's case changed. The
    /// grammar decides instead.
    #[test]
    fn an_expression_that_merely_starts_and_ends_with_a_slash_is_not_a_regex() {
        let joined = phrase_of("/a/ and /b/i");
        assert!(
            joined.structured,
            "an and of two regexes is an expression, not a phrase: {joined:?}"
        );
        assert!(!joined.regex, "{joined:?}");
        assert_eq!(joined.text, "/a/ and /b/i", "shown as written");

        // And one that really is a single regex still reads as one.
        let single = phrase_of("/a and b/i");
        assert!(!single.structured && single.regex);
        assert_eq!(single.text, "a and b");
    }
}
