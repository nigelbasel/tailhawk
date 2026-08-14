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
    /// The needle is stored folded so evaluation does not refold it per row.
    Text {
        folded: String,
        source: String,
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
            Predicate::Text { folded, .. } => {
                Truth::of(record.raw.to_lowercase().contains(folded.as_str()))
            }
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
    /// The text the user typed, kept so §7.2's "there is no hidden second representation" holds —
    /// a chip made by the UI's *"filter for this value"* is editable as the text it round-trips to.
    pub source: String,
}

impl Chip {
    pub fn parse(text: &str, polarity: Polarity) -> Result<Self, ParseError> {
        Ok(Self {
            predicate: parse(text)?,
            polarity,
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
        for chip in &self.chips {
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
fn parse_instant(token: &str) -> Option<Timestamp> {
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
fn days_from_civil(year: i64, month: i64, day: i64) -> Option<i64> {
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
}
