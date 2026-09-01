//! Reading a Loki `query_range` response, without believing it.
//!
//! `loki.rs` decides what Tailhawk says; this reads what comes back. It is pure — a `&str` in,
//! records out — and it is the half where a hostile or merely broken server gets to choose the
//! input, so every loop in it is bounded by a [`Limits`] and every bound has a test that drives it.
//! The bounds are tested against the *work avoided* and not only against the error returned, which
//! is a distinction that cost this module a real defect: the length caps were once checked after
//! each run of the string reader rather than during it, so a 32 MB line was fully allocated before
//! being refused for passing a 512 KB limit. A cap that allocates the input before refusing it is
//! a report, not a cap.
//!
//! **The shape of the response is measured, not documented, and the two disagree.** Grafana's HTTP
//! API reference documents `values` entries as three-element `[ts, line, {metadata}]` tuples.
//! `LOKI.md` §3 records that on 3.5.2 this is the *push* format: `query_range` **responses** merge
//! structured metadata into the `stream` object and carry **two**-element entries, and because that
//! metadata differs per record, each stream object holds exactly one entry. So the arity is read
//! per entry rather than assumed — a parser that believes either source exclusively is one that
//! breaks on a version change.
//!
//! **Labels are most of the payload.** §3 measured 491,250 bytes of label JSON against 90,290 bytes
//! of actual log lines at `limit=1000`, a 5.4x amplification. So this interns keys and values and
//! never builds a document: a map per record would allocate the amplification rather than collapse
//! it, and `SPEC.md` §11.2's memory-flatness claim would go with it on the first page.
//!
//! **Record text is never stripped or rewritten.** §7's C0/ANSI/bidi stripping belongs where server
//! text is rendered, copied or exported — `ansi.rs` owns that — and doing it here would quietly
//! alter the `raw` that §2 already qualifies as "as received from Loki". The one exception is
//! [`printable`], for the scraps of server text a [`WireFault`] repeats back: a fault is not record
//! text, it lands in the status bar or a dialog, and it never passes through `ansi.rs`.

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use crate::loki::Nanos;

/// A label key or value, interned.
///
/// Ids are indices into the [`Interner`] that produced them and mean nothing without it.
///
/// **Two ids being equal means the same string only while the table is not saturated.** Past
/// [`Limits::max_interned`] the table stops recognising what it has already seen, so the same
/// string minted afterwards gets a fresh id each time — [`Interner::is_saturated`] is how a caller
/// asks. Compare text rather than ids anywhere that matters, which is what [`Entry::label`] does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LabelId(pub u32);

/// The label text of one response, stored once per distinct string.
///
/// **Degrades rather than fails.** Past [`Limits::max_interned`] a string is stored without being
/// remembered, so a server sending a million unique keys costs memory linearly and then trips the
/// response's own caps, instead of growing a hash map without bound. The total is bounded by
/// [`Limits::max_bytes`] rather than by `max_interned`, which bounds only the lookup table.
///
/// The text is held once and shared between the table and the lookup, not copied into both — at
/// §3's measured 5.4x label amplification, storing each string twice would double the very thing
/// this type exists to collapse.
#[derive(Debug, Clone, Default)]
pub struct Interner {
    texts: Vec<Arc<str>>,
    seen: HashMap<Arc<str>, u32>,
    full: bool,
}

impl Interner {
    /// An empty table.
    pub fn new() -> Interner {
        Interner::default()
    }

    /// The text behind an id.
    ///
    /// `None` means the id is out of range for *this* table. An id minted by a different table is
    /// not detected and will resolve to whatever text sits at that index — ids are meaningful only
    /// alongside the [`Batch`] that produced them.
    pub fn text(&self, id: LabelId) -> Option<&str> {
        self.texts.get(id.0 as usize).map(Arc::as_ref)
    }

    /// How many ids have been minted. Equal to the number of distinct strings until the table
    /// saturates, and larger afterwards.
    pub fn len(&self) -> usize {
        self.texts.len()
    }

    /// Is anything held at all?
    pub fn is_empty(&self) -> bool {
        self.texts.is_empty()
    }

    /// Has the table stopped remembering what it has seen?
    pub fn is_saturated(&self) -> bool {
        self.full
    }

    fn intern(&mut self, text: &str, ceiling: usize) -> LabelId {
        if !self.full {
            if let Some(id) = self.seen.get(text) {
                return LabelId(*id);
            }
            if self.seen.len() >= ceiling {
                self.full = true;
                self.seen.clear();
                self.seen.shrink_to_fit();
            }
        }
        let id = self.texts.len() as u32;
        let shared: Arc<str> = Arc::from(text);
        self.texts.push(Arc::clone(&shared));
        if !self.full {
            self.seen.insert(shared, id);
        }
        LabelId(id)
    }
}

/// One record: when it happened, what was logged, and the labels it arrived under.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// The emitter's timestamp, in nanoseconds. `LOKI.md` §2 calls this the best timestamp input
    /// the product will ever get, and it is the reason none of §8.3's normalisation applies here.
    pub timestamp: Nanos,
    /// The log line as received from Loki. §2 is explicit that this is not "as emitted" — Loki's
    /// `max_line_size` truncates upstream, undetectably.
    pub line: String,
    /// Label keys and values, interned, in the order the response gave them.
    pub labels: Vec<(LabelId, LabelId)>,
}

impl Entry {
    /// The value of one label, if the record carries it.
    ///
    /// **The last occurrence wins**, and that is the rule rather than an accident of iteration. A
    /// three-element entry carries per-record structured metadata, which is appended after the
    /// stream's own labels; where the two name the same key the per-record value is the more
    /// specific and must be the answer. It also makes the two wire shapes agree: the server-merged
    /// form Loki 3.5.2 actually sends has already resolved the collision the same way, so a parser
    /// that took the first occurrence would give two different answers for one record depending on
    /// which shape it arrived in.
    pub fn label<'a>(&self, interner: &'a Interner, key: &str) -> Option<&'a str> {
        self.labels
            .iter()
            .rev()
            .find(|(k, _)| interner.text(*k) == Some(key))
            .and_then(|(_, v)| interner.text(*v))
    }
}

/// What one response yielded.
#[derive(Debug, Clone, Default)]
pub struct Batch {
    /// The records, in the order the response gave them. Loki orders within a stream; ordering
    /// *across* streams is the caller's business, not the parser's.
    pub entries: Vec<Entry>,
    /// The label text every [`LabelId`] above refers to.
    pub interner: Interner,
    /// Records the response held beyond [`Limits::max_entries`] and this did not keep.
    ///
    /// Never silently zero when something was dropped — `LOKI.md` §6's rule is that silent
    /// truncation is the worst failure this feature can have, so a caller that ignores this is
    /// making a choice rather than missing a field.
    pub dropped: usize,
}

/// `LOKI.md` §7's response-parse caps.
///
/// §7 asks for these to be "hard non-configurable caps", and they are: nothing reads them from a
/// TOML file, which is what that clause defends against. They are a value rather than constants so
/// that a test can shrink them, because a cap nobody has driven is a cap nobody knows works.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// The most response text that will be looked at.
    pub max_bytes: usize,
    /// The most records one response may yield.
    pub max_entries: usize,
    /// The most label pairs one stream object may carry.
    pub max_labels: usize,
    /// The longest a label key or value may be.
    pub max_label_len: usize,
    /// The longest a log line may be. Loki's own `max_line_size` defaults to 256 KB.
    pub max_line_len: usize,
    /// How deeply arrays and objects may nest before the response is refused.
    pub max_depth: usize,
    /// How many distinct strings the intern table remembers before it stops remembering.
    pub max_interned: usize,
}

impl Default for Limits {
    fn default() -> Limits {
        Limits {
            max_bytes: 64 * 1024 * 1024,
            max_entries: 50_000,
            max_labels: 128,
            max_label_len: 4 * 1024,
            max_line_len: 512 * 1024,
            max_depth: 32,
            max_interned: 100_000,
        }
    }
}

/// Why a response could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WireFault {
    /// The response is longer than [`Limits::max_bytes`].
    TooLarge {
        /// How long it was.
        bytes: usize,
    },
    /// Not JSON, or not the JSON this expects. The offset is a byte index into the response.
    Malformed {
        /// Where the reader gave up.
        at: usize,
        /// What it wanted there.
        wanted: &'static str,
    },
    /// The response nests deeper than [`Limits::max_depth`].
    TooDeep {
        /// Where the limit was passed.
        at: usize,
    },
    /// A label key or value longer than [`Limits::max_label_len`].
    LabelTooLong {
        /// How long it was.
        len: usize,
    },
    /// A log line longer than [`Limits::max_line_len`].
    LineTooLong {
        /// How long it was.
        len: usize,
    },
    /// One stream object carrying more than [`Limits::max_labels`] labels.
    TooManyLabels {
        /// How many it carried before the reader stopped.
        count: usize,
    },
    /// Loki answered, and said no. The `status` field was not `success`.
    ServerSaidNo {
        /// Whatever it said instead, put through [`printable`].
        status: String,
    },
    /// A `resultType` this cannot read — a metric query answered where a log query was asked.
    NotStreams {
        /// What it said the result was.
        kind: String,
    },
    /// A timestamp that is not a nanosecond integer.
    BadTimestamp {
        /// What was there instead.
        text: String,
    },
}

impl fmt::Display for WireFault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WireFault::TooLarge { bytes } => write!(
                f,
                "the response is {bytes} bytes, which is more than will be read"
            ),
            WireFault::Malformed { at, wanted } => write!(f, "expected {wanted} at byte {at}"),
            WireFault::TooDeep { at } => write!(f, "the response nests too deeply, at byte {at}"),
            WireFault::LabelTooLong { len } => {
                write!(f, "a label of {len} bytes is longer than a label should be")
            }
            WireFault::LineTooLong { len } => write!(
                f,
                "a log line of {len} bytes is longer than a line should be"
            ),
            WireFault::TooManyLabels { count } => {
                write!(f, "a stream carrying {count} labels carries too many")
            }
            WireFault::ServerSaidNo { status } => write!(f, "Loki answered \"{status}\""),
            WireFault::NotStreams { kind } => {
                write!(f, "the answer is {kind}, which is not log lines")
            }
            WireFault::BadTimestamp { text } => {
                write!(f, "\"{text}\" is not a nanosecond timestamp")
            }
        }
    }
}

/// One record as a **CLEF NDJSON** line, without its newline.
///
/// `LOKI.md` §4 spills a materialised window to CLEF so that the line index, the format detector
/// and the grid read a Loki source with no new document type behind them. This is that conversion.
///
/// **The field order is dictated by `format.rs`'s own `ndjson` pattern, which matches `@t`, then
/// `@l`, then `@m` sequentially.** A spill written in any other order would be a file Tailhawk
/// could not recognise as the format it had just produced — so the order here is a contract with
/// the detector rather than a preference, and the test that guards it runs the real detector
/// instead of comparing against an expected string.
///
/// `@m` and not `@mt`: Loki hands over a rendered line, and calling it a message *template* would
/// be a small untruth in a field other tools read. Labels follow, except the `__`-prefixed ones §2
/// suppresses — Loki's own sharding bookkeeping, which the owner's deployment really does send.
pub fn clef_line(entry: &Entry, interner: &Interner) -> String {
    let mut out = String::with_capacity(entry.line.len() + 96);
    out.push_str("{\"@t\":\"");
    out.push_str(&rfc3339_nanos(entry.timestamp));
    out.push('"');
    if let Some(level) = entry.level(interner) {
        out.push_str(",\"@l\":\"");
        json_escape_into(&mut out, level);
        out.push('"');
    }
    out.push_str(",\"@m\":\"");
    json_escape_into(&mut out, &entry.line);
    out.push('"');
    for (key, value) in &entry.labels {
        let (Some(key), Some(value)) = (interner.text(*key), interner.text(*value)) else {
            continue;
        };
        if key.starts_with("__") || shadows_a_clef_field(key) {
            continue;
        }
        out.push_str(",\"");
        json_escape_into(&mut out, key);
        out.push_str("\":\"");
        json_escape_into(&mut out, value);
        out.push('"');
    }
    out.push('}');
    out
}

/// Label names that carry a level, ordered measured-first, documented-second, inferred-last.
///
/// **The last two are here because `format.rs`'s `ndjson` reader accepts them**, not because Loki
/// emits them. Its pattern reads `level|lvl|severity|@l`, and it scans a line for a timestamp, then
/// a level, then a message, *in that order*. So a stream label spelled `severity` sitting after the
/// message is read as the level, the message group then has nothing left to match, and the reader's
/// fallback makes the body the entire raw JSON line — every row of that stream showing its own
/// JSON. Naming a label the reader treats as a level and then not treating it as one is what
/// produced that.
const LEVEL_LABELS: [&str; 5] = [
    "level",
    "severity_text",
    "detected_level",
    "severity",
    "lvl",
];

/// Label names the spill must not repeat after the message, because the reader would find them
/// first and lose the message behind them.
///
/// A label named `msg` or `message` duplicates `@m`; one named `severity` or `lvl` is already
/// written as `@l` by [`Entry::level`]. Emitting either again is at best redundant and at worst the
/// bug above, so the tail carries neither.
fn shadows_a_clef_field(key: &str) -> bool {
    LEVEL_LABELS.contains(&key) || matches!(key, "msg" | "message" | "@m" | "@mt" | "@t" | "@l")
}

/// A whole batch as CLEF NDJSON, one record per line, newline-terminated.
pub fn clef_spill(batch: &Batch) -> String {
    let mut out = String::new();
    for entry in &batch.entries {
        out.push_str(&clef_line(entry, &batch.interner));
        out.push('\n');
    }
    out
}

impl Entry {
    /// The record's level word, from whichever label carries it.
    ///
    /// `level` is what the owner's deployment indexes; `severity_text` is what `LOKI.md` §2 says an
    /// OTLP-native Loki puts in structured metadata; `detected_level` is Loki's own guess. They are
    /// tried in that order — measured first, documented second, inferred last. `severity` and `lvl`
    /// follow because [`format`](crate::format)'s own reader accepts those spellings, and a name
    /// the reader treats as a level has to be treated as one here too — see [`LEVEL_LABELS`].
    pub fn level<'a>(&self, interner: &'a Interner) -> Option<&'a str> {
        LEVEL_LABELS
            .into_iter()
            .find_map(|key| self.label(interner, key))
    }
}

fn json_escape_into(out: &mut String, text: &str) {
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
}

/// A nanosecond instant as RFC 3339 in UTC, to the nanosecond.
///
/// **The precision is kept deliberately.** §2 calls Loki's timestamp the best input the product
/// will ever get — nanosecond, UTC, from the emitting process — and rounding it to milliseconds on
/// the way through the spill would throw away the one thing a remote source has over a local file.
fn rfc3339_nanos(ns: Nanos) -> String {
    let (secs, sub) = (ns.div_euclid(1_000_000_000), ns.rem_euclid(1_000_000_000));
    let days = secs.div_euclid(86_400);
    let rest = secs.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{sub:09}Z",
        rest / 3600,
        (rest % 3600) / 60,
        rest % 60
    )
}

/// Days since the Unix epoch back to a civil date — the inverse of `filter.rs`'s
/// `days_from_civil`, and tested by round-tripping against it rather than against copied answers.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_shifted = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * month_shifted + 2) / 5 + 1) as u32;
    let month = if month_shifted < 10 {
        month_shifted + 3
    } else {
        month_shifted - 9
    } as u32;
    (if month <= 2 { year + 1 } else { year }, month, day)
}

/// Make a scrap of server text safe to put in front of a person, and short enough to fit.
///
/// **This is the one place in this module where server text is sanitised, and the exception proves
/// the rule.** `Entry::line` is deliberately left exactly as it arrived — `LOKI.md` §2 qualifies it
/// as "as received from Loki", and `ansi.rs` owns stripping at the point of render, copy or export.
/// A [`WireFault`], though, is not record text: it becomes a status-bar line or a dialog, and it
/// will never pass through `ansi.rs`. §7 asks for C0, ANSI and bidi sequences to be stripped from
/// all server text before rendering, and a fault carrying a clear-screen sequence and a
/// right-to-left override is a server writing its own message into Tailhawk's chrome.
///
/// C0 and DEL go, the bidi overrides and isolates go, and what is left is cut to `MAX_SAID`.
pub fn printable(text: &str) -> String {
    let mut out = String::with_capacity(text.len().min(MAX_SAID));
    for c in text.chars() {
        if out.chars().count() >= MAX_SAID {
            out.push('…');
            break;
        }
        let banned = c.is_control()
            || matches!(c, '\u{200e}' | '\u{200f}' | '\u{061c}')
            || ('\u{202a}'..='\u{202e}').contains(&c)
            || ('\u{2066}'..='\u{2069}').contains(&c);
        if !banned {
            out.push(c);
        }
    }
    out
}

/// The most server-authored text a fault will repeat back.
const MAX_SAID: usize = 120;

/// Read a `query_range` response.
///
/// The shape read is `{"status":"success","data":{"resultType":"streams","result":[…]}}`, where
/// each element of `result` is `{"stream":{…labels…},"values":[[ts,line],…]}`. A `values` entry of
/// three elements is accepted too, its third member merged as further labels — see this module's
/// note on why the arity is probed rather than assumed.
pub fn parse_query_range(text: &str, limits: &Limits) -> Result<Batch, WireFault> {
    if text.len() > limits.max_bytes.min(RESPONSE_CEILING) {
        return Err(WireFault::TooLarge { bytes: text.len() });
    }
    let mut reader = Reader {
        bytes: text.as_bytes(),
        at: 0,
        depth: 0,
        limits,
    };
    let mut batch = Batch::default();
    reader.read_response(&mut batch)?;
    Ok(batch)
}

/// Which kind of text is being read, so the fault says which cap was passed.
///
/// This used to be inferred by comparing the cap against `max_line_len`, which reported a label as
/// a line whenever a caller set the two limits to the same number, and reported an over-long
/// timestamp as a label. A parser that cannot say what it refused is a parser nobody can act on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Text {
    Line,
    Label,
    Timestamp,
}

impl Text {
    fn too_long(self, len: usize, at: usize) -> WireFault {
        match self {
            Text::Line => WireFault::LineTooLong { len },
            Text::Label => WireFault::LabelTooLong { len },
            Text::Timestamp => WireFault::Malformed {
                at,
                wanted: "a nanosecond timestamp",
            },
        }
    }
}

/// The deepest nesting this will descend, whatever a caller asks for.
///
/// `Limits::max_depth` is a public field and every other cap's violation is a `WireFault` a caller
/// can handle. This one's is a stack overflow, which on Windows aborts the process and cannot be
/// caught — so it is the one cap that must not be settable past a ceiling. Measured: 200,000
/// nested arrays under a raised `max_depth` killed the test harness with `0xC00000FD`.
const DEPTH_CEILING: usize = 256;

/// The most response text that will be looked at, whatever a caller asks for.
///
/// Bounds `Interner`'s id space as a side effect: ids are `u32`, and four billion labels cannot
/// arrive in a gigabyte.
const RESPONSE_CEILING: usize = 1 << 30;

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
    depth: usize,
    limits: &'a Limits,
}

impl<'a> Reader<'a> {
    fn fault(&self, wanted: &'static str) -> WireFault {
        WireFault::Malformed {
            at: self.at,
            wanted,
        }
    }

    fn skip_space(&mut self) {
        while let Some(b) = self.bytes.get(self.at) {
            if matches!(b, b' ' | b'\t' | b'\n' | b'\r') {
                self.at += 1;
            } else {
                break;
            }
        }
    }

    fn peek(&mut self) -> Option<u8> {
        self.skip_space();
        self.bytes.get(self.at).copied()
    }

    fn eat(&mut self, byte: u8, wanted: &'static str) -> Result<(), WireFault> {
        if self.peek() == Some(byte) {
            self.at += 1;
            Ok(())
        } else {
            Err(self.fault(wanted))
        }
    }

    fn enter(&mut self) -> Result<(), WireFault> {
        self.depth += 1;
        if self.depth > self.limits.max_depth.min(DEPTH_CEILING) {
            return Err(WireFault::TooDeep { at: self.at });
        }
        Ok(())
    }

    fn leave(&mut self) {
        self.depth -= 1;
    }

    /// Read a JSON string, resolving escapes.
    ///
    /// **The cap is tested as the string is built, not after it is built.** Testing at the end of
    /// each run made the cap a *report* rather than a bound: a run is everything up to the next
    /// quote or backslash, so a hostile server sending one 32 MB line got 32 MB allocated before
    /// being told the 512 KB limit had been passed. §7 lists these among the hard caps precisely
    /// because a hostile server is an out-of-memory lever, and a limit that allocates the whole
    /// input before refusing it is not a limit. Measured at 64x the cap before this changed.
    fn string(&mut self, cap: usize, kind: Text) -> Result<String, WireFault> {
        self.eat(b'"', "a string")?;
        let mut out = String::new();
        loop {
            let byte = *self
                .bytes
                .get(self.at)
                .ok_or(self.fault("a closing quote"))?;
            match byte {
                b'"' => {
                    self.at += 1;
                    return Ok(out);
                }
                b'\\' => {
                    self.at += 1;
                    let escape = *self.bytes.get(self.at).ok_or(self.fault("an escape"))?;
                    self.at += 1;
                    match escape {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => out.push(self.unicode_escape()?),
                        _ => return Err(self.fault("a known escape")),
                    }
                }
                _ => {
                    let start = self.at;
                    while let Some(b) = self.bytes.get(self.at) {
                        if *b == b'"' || *b == b'\\' {
                            break;
                        }
                        if out.len() + (self.at - start) > cap {
                            return Err(kind.too_long(cap + 1, self.at));
                        }
                        self.at += 1;
                    }
                    match std::str::from_utf8(&self.bytes[start..self.at]) {
                        Ok(run) => out.push_str(run),
                        Err(_) => return Err(self.fault("valid UTF-8")),
                    }
                }
            }
            if out.len() > cap {
                return Err(kind.too_long(out.len(), self.at));
            }
        }
    }

    /// The `\uXXXX` form, including a surrogate pair.
    fn unicode_escape(&mut self) -> Result<char, WireFault> {
        let first = self.hex4()?;
        if (0xd800..0xdc00).contains(&first) {
            if self.bytes.get(self.at) != Some(&b'\\') || self.bytes.get(self.at + 1) != Some(&b'u')
            {
                return Err(self.fault("the second half of a surrogate pair"));
            }
            self.at += 2;
            let second = self.hex4()?;
            if !(0xdc00..0xe000).contains(&second) {
                return Err(self.fault("a low surrogate"));
            }
            let combined = 0x10000 + (((first as u32) - 0xd800) << 10) + ((second as u32) - 0xdc00);
            return char::from_u32(combined).ok_or(self.fault("a character"));
        }
        char::from_u32(first as u32).ok_or(self.fault("a character"))
    }

    fn hex4(&mut self) -> Result<u16, WireFault> {
        let end = self.at + 4;
        let digits = self
            .bytes
            .get(self.at..end)
            .ok_or(self.fault("four hex digits"))?;
        let mut value: u16 = 0;
        for digit in digits {
            let nibble = match digit {
                b'0'..=b'9' => digit - b'0',
                b'a'..=b'f' => digit - b'a' + 10,
                b'A'..=b'F' => digit - b'A' + 10,
                _ => return Err(self.fault("four hex digits")),
            };
            value = (value << 4) | nibble as u16;
        }
        self.at = end;
        Ok(value)
    }

    /// Step over any value without keeping it.
    fn skip_value(&mut self) -> Result<(), WireFault> {
        match self.peek().ok_or(self.fault("a value"))? {
            b'"' => {
                self.string(self.limits.max_line_len, Text::Line)?;
            }
            b'{' | b'[' => {
                let open = self.peek().unwrap();
                let close = if open == b'{' { b'}' } else { b']' };
                self.at += 1;
                self.enter()?;
                if self.peek() == Some(close) {
                    self.at += 1;
                    self.leave();
                    return Ok(());
                }
                loop {
                    if open == b'{' {
                        self.string(self.limits.max_label_len, Text::Label)?;
                        self.eat(b':', "a colon")?;
                    }
                    self.skip_value()?;
                    match self.peek() {
                        Some(b',') => self.at += 1,
                        Some(b) if b == close => {
                            self.at += 1;
                            self.leave();
                            return Ok(());
                        }
                        _ => return Err(self.fault("a comma or a close")),
                    }
                }
            }
            _ => {
                let start = self.at;
                while let Some(b) = self.bytes.get(self.at) {
                    if matches!(b, b',' | b'}' | b']' | b' ' | b'\t' | b'\n' | b'\r') {
                        break;
                    }
                    self.at += 1;
                }
                if self.at == start {
                    return Err(self.fault("a value"));
                }
            }
        }
        Ok(())
    }

    fn read_response(&mut self, batch: &mut Batch) -> Result<(), WireFault> {
        self.eat(b'{', "an object")?;
        self.enter()?;
        let mut saw_data = false;
        loop {
            if self.peek() == Some(b'}') {
                self.at += 1;
                self.leave();
                break;
            }
            let key = self.string(self.limits.max_label_len, Text::Label)?;
            self.eat(b':', "a colon")?;
            match key.as_str() {
                "status" => {
                    let status = self.string(self.limits.max_label_len, Text::Label)?;
                    if status != "success" {
                        return Err(WireFault::ServerSaidNo {
                            status: printable(&status),
                        });
                    }
                }
                "data" => {
                    self.read_data(batch)?;
                    saw_data = true;
                }
                _ => self.skip_value()?,
            }
            match self.peek() {
                Some(b',') => self.at += 1,
                Some(b'}') => {}
                _ => return Err(self.fault("a comma or a close")),
            }
        }
        if !saw_data {
            return Err(self.fault("a data object"));
        }
        Ok(())
    }

    fn read_data(&mut self, batch: &mut Batch) -> Result<(), WireFault> {
        self.eat(b'{', "the data object")?;
        self.enter()?;
        loop {
            if self.peek() == Some(b'}') {
                self.at += 1;
                self.leave();
                return Ok(());
            }
            let key = self.string(self.limits.max_label_len, Text::Label)?;
            self.eat(b':', "a colon")?;
            match key.as_str() {
                "resultType" => {
                    let kind = self.string(self.limits.max_label_len, Text::Label)?;
                    if kind != "streams" {
                        return Err(WireFault::NotStreams {
                            kind: printable(&kind),
                        });
                    }
                }
                "result" => self.read_result(batch)?,
                _ => self.skip_value()?,
            }
            match self.peek() {
                Some(b',') => self.at += 1,
                Some(b'}') => {}
                _ => return Err(self.fault("a comma or a close")),
            }
        }
    }

    fn read_result(&mut self, batch: &mut Batch) -> Result<(), WireFault> {
        self.eat(b'[', "the result array")?;
        self.enter()?;
        if self.peek() == Some(b']') {
            self.at += 1;
            self.leave();
            return Ok(());
        }
        loop {
            self.read_stream(batch)?;
            match self.peek() {
                Some(b',') => self.at += 1,
                Some(b']') => {
                    self.at += 1;
                    self.leave();
                    return Ok(());
                }
                _ => return Err(self.fault("a comma or the end of the result")),
            }
        }
    }

    /// One `{"stream":{…},"values":[…]}` object.
    ///
    /// `values` is remembered as a span and read after the whole object has been walked, because a
    /// stream object may present its values before its labels and every entry needs the labels
    /// attached. A **second** `values` key is refused rather than allowed to replace the first:
    /// overwriting the span would discard the first array's records with nothing set to say so, and
    /// `LOKI.md` §6's rule is that silent truncation is the worst failure this feature can have.
    fn read_stream(&mut self, batch: &mut Batch) -> Result<(), WireFault> {
        self.eat(b'{', "a stream object")?;
        self.enter()?;
        let mut labels: Vec<(LabelId, LabelId)> = Vec::new();
        let mut values: Option<(usize, usize)> = None;
        loop {
            if self.peek() == Some(b'}') {
                self.at += 1;
                self.leave();
                break;
            }
            let key = self.string(self.limits.max_label_len, Text::Label)?;
            self.eat(b':', "a colon")?;
            match key.as_str() {
                "stream" => self.read_labels(batch, &mut labels)?,
                "values" => {
                    if values.is_some() {
                        return Err(self.fault("one values array, not two"));
                    }
                    let start = self.at;
                    self.skip_value()?;
                    values = Some((start, self.at));
                }
                _ => self.skip_value()?,
            }
            match self.peek() {
                Some(b',') => self.at += 1,
                Some(b'}') => {}
                _ => return Err(self.fault("a comma or a close")),
            }
        }

        // `values` is read after the whole object, because a stream object may present it before
        // its labels and every entry needs the labels attached.
        if let Some((start, end)) = values {
            let mut inner = Reader {
                bytes: &self.bytes[..end],
                at: start,
                depth: self.depth,
                limits: self.limits,
            };
            inner.read_values(batch, &labels)?;
        }
        Ok(())
    }

    fn read_labels(
        &mut self,
        batch: &mut Batch,
        labels: &mut Vec<(LabelId, LabelId)>,
    ) -> Result<(), WireFault> {
        self.eat(b'{', "the label object")?;
        self.enter()?;
        if self.peek() == Some(b'}') {
            self.at += 1;
            self.leave();
            return Ok(());
        }
        loop {
            let key = self.string(self.limits.max_label_len, Text::Label)?;
            self.eat(b':', "a colon")?;
            let value = self.string(self.limits.max_label_len, Text::Label)?;
            if labels.len() >= self.limits.max_labels {
                return Err(WireFault::TooManyLabels {
                    count: labels.len() + 1,
                });
            }
            let key = batch.interner.intern(&key, self.limits.max_interned);
            let value = batch.interner.intern(&value, self.limits.max_interned);
            labels.push((key, value));
            match self.peek() {
                Some(b',') => self.at += 1,
                Some(b'}') => {
                    self.at += 1;
                    self.leave();
                    return Ok(());
                }
                _ => return Err(self.fault("a comma or a close")),
            }
        }
    }

    fn read_values(
        &mut self,
        batch: &mut Batch,
        labels: &[(LabelId, LabelId)],
    ) -> Result<(), WireFault> {
        self.eat(b'[', "the values array")?;
        self.enter()?;
        if self.peek() == Some(b']') {
            self.at += 1;
            self.leave();
            return Ok(());
        }
        loop {
            self.read_entry(batch, labels)?;
            match self.peek() {
                Some(b',') => self.at += 1,
                Some(b']') => {
                    self.at += 1;
                    self.leave();
                    return Ok(());
                }
                _ => return Err(self.fault("a comma or the end of the values")),
            }
        }
    }

    /// One `[ts, line]` — or `[ts, line, {metadata}]`, whose arity is read rather than assumed.
    ///
    /// Past [`Limits::max_entries`] the entry is stepped over rather than built and discarded: the
    /// line, the timestamp and the label vector are the whole cost of a record, and there is no
    /// reason to pay it for one that will not be kept. [`Batch::dropped`] still counts it.
    fn read_entry(
        &mut self,
        batch: &mut Batch,
        labels: &[(LabelId, LabelId)],
    ) -> Result<(), WireFault> {
        if batch.entries.len() >= self.limits.max_entries {
            self.skip_value()?;
            batch.dropped += 1;
            return Ok(());
        }
        self.eat(b'[', "an entry")?;
        self.enter()?;
        let stamp = self.string(64, Text::Timestamp)?;
        let timestamp = stamp
            .parse::<Nanos>()
            .map_err(|_| WireFault::BadTimestamp {
                text: printable(&stamp),
            })?;
        self.eat(b',', "a comma")?;
        let line = self.string(self.limits.max_line_len, Text::Line)?;

        let mut labels = labels.to_vec();
        if self.peek() == Some(b',') {
            self.at += 1;
            if self.peek() == Some(b'{') {
                self.read_labels(batch, &mut labels)?;
            } else {
                self.skip_value()?;
            }
        }
        self.eat(b']', "the end of the entry")?;
        self.leave();

        batch.entries.push(Entry {
            timestamp,
            line,
            labels,
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(text: &str) -> Batch {
        parse_query_range(text, &Limits::default()).expect("should parse")
    }

    fn fault(text: &str) -> WireFault {
        parse_query_range(text, &Limits::default()).expect_err("should be refused")
    }

    /// The shape `LOKI.md` §3 measured: two-element entries, metadata merged into `stream`, one
    /// entry per stream object.
    const MEASURED: &str = r#"{
      "status": "success",
      "data": {
        "resultType": "streams",
        "result": [
          {
            "stream": {"app": "checkout", "severity_text": "Error", "trace_id": "abc"},
            "values": [["1724750000000000001", "order failed"]]
          },
          {
            "stream": {"app": "checkout", "severity_text": "Information", "trace_id": "def"},
            "values": [["1724750000000000002", "order placed"]]
          }
        ]
      }
    }"#;

    /// What the published reference describes: three-element entries with their own metadata.
    const DOCUMENTED: &str = r#"{
      "status": "success",
      "data": {
        "resultType": "streams",
        "result": [
          {
            "stream": {"app": "checkout"},
            "values": [
              ["1724750000000000001", "order failed", {"severity_text": "Error"}],
              ["1724750000000000002", "order placed", {"severity_text": "Information"}]
            ]
          }
        ]
      }
    }"#;

    #[test]
    fn the_shape_the_deployment_actually_sends_is_read() {
        let batch = read(MEASURED);
        assert_eq!(batch.entries.len(), 2);
        assert_eq!(batch.entries[0].timestamp, 1_724_750_000_000_000_001);
        assert_eq!(batch.entries[0].line, "order failed");
        assert_eq!(
            batch.entries[0].label(&batch.interner, "app"),
            Some("checkout")
        );
        assert_eq!(
            batch.entries[0].label(&batch.interner, "severity_text"),
            Some("Error")
        );
        assert_eq!(
            batch.entries[1].label(&batch.interner, "severity_text"),
            Some("Information")
        );
        assert_eq!(batch.dropped, 0);
    }

    #[test]
    fn the_shape_the_documentation_describes_is_read_too() {
        let batch = read(DOCUMENTED);
        assert_eq!(batch.entries.len(), 2);
        assert_eq!(
            batch.entries[0].label(&batch.interner, "severity_text"),
            Some("Error")
        );
        assert_eq!(
            batch.entries[1].label(&batch.interner, "severity_text"),
            Some("Information")
        );
        assert_eq!(
            batch.entries[1].label(&batch.interner, "app"),
            Some("checkout"),
            "the stream's own labels reach a three-element entry as well"
        );
    }

    #[test]
    fn both_shapes_give_the_same_answer_which_is_the_reason_for_probing() {
        let measured = read(MEASURED);
        let documented = read(DOCUMENTED);
        let flatten = |b: &Batch| -> Vec<(Nanos, String, Option<String>)> {
            b.entries
                .iter()
                .map(|e| {
                    (
                        e.timestamp,
                        e.line.clone(),
                        e.label(&b.interner, "severity_text").map(str::to_owned),
                    )
                })
                .collect()
        };
        assert_eq!(flatten(&measured), flatten(&documented));
    }

    #[test]
    fn labels_repeated_across_records_are_stored_once() {
        let batch = read(MEASURED);
        let distinct = batch.interner.len();
        assert!(
            distinct <= 8,
            "app, checkout, severity_text, Error, Information, trace_id, abc, def — {distinct} held"
        );
        let app_ids: Vec<LabelId> = batch
            .entries
            .iter()
            .map(|e| {
                e.labels
                    .iter()
                    .find(|(k, _)| batch.interner.text(*k) == Some("app"))
                    .unwrap()
                    .1
            })
            .collect();
        assert_eq!(app_ids[0], app_ids[1], "one string, one id, both records");
    }

    #[test]
    fn a_stream_that_puts_its_values_before_its_labels_still_gets_them() {
        let batch = read(
            r#"{"status":"success","data":{"resultType":"streams","result":[
                 {"values":[["1","a"]],"stream":{"app":"x"}}]}}"#,
        );
        assert_eq!(batch.entries.len(), 1);
        assert_eq!(batch.entries[0].label(&batch.interner, "app"), Some("x"));
    }

    #[test]
    fn fields_this_does_not_care_about_are_stepped_over() {
        let batch = read(
            r#"{"status":"success","warnings":["a"],"data":{"resultType":"streams",
                 "stats":{"summary":{"totalBytesProcessed":112300000},"ingester":{}},
                 "result":[{"stream":{"app":"x"},"values":[["1","a"]]}]},"extra":null}"#,
        );
        assert_eq!(batch.entries.len(), 1);
        assert_eq!(batch.entries[0].line, "a");
    }

    #[test]
    fn an_empty_result_is_an_answer_rather_than_a_fault() {
        let batch = read(r#"{"status":"success","data":{"resultType":"streams","result":[]}}"#);
        assert!(batch.entries.is_empty());
        assert_eq!(batch.dropped, 0);
    }

    #[test]
    fn escapes_in_a_log_line_survive() {
        let batch = read(
            r#"{"status":"success","data":{"resultType":"streams","result":[
                 {"stream":{"app":"x"},"values":[["1","a\"b\\c\nd\teéf😀"]]}]}}"#,
        );
        assert_eq!(batch.entries[0].line, "a\"b\\c\nd\te\u{e9}f\u{1f600}");
    }

    #[test]
    fn a_server_that_says_it_failed_is_believed() {
        assert_eq!(
            fault(r#"{"status":"error","errorType":"bad_data","error":"parse error"}"#),
            WireFault::ServerSaidNo {
                status: "error".to_owned()
            }
        );
    }

    #[test]
    fn a_metric_answer_to_a_log_question_is_refused_rather_than_read_as_empty() {
        assert_eq!(
            fault(r#"{"status":"success","data":{"resultType":"matrix","result":[]}}"#),
            WireFault::NotStreams {
                kind: "matrix".to_owned()
            }
        );
    }

    #[test]
    fn a_timestamp_that_is_not_a_number_is_refused() {
        assert_eq!(
            fault(
                r#"{"status":"success","data":{"resultType":"streams","result":[
                     {"stream":{},"values":[["not-a-time","a"]]}]}}"#
            ),
            WireFault::BadTimestamp {
                text: "not-a-time".to_owned()
            }
        );
    }

    #[test]
    fn rubbish_is_refused_and_says_where() {
        for text in [
            "",
            "{",
            "[]",
            "null",
            r#"{"status":}"#,
            r#"{"status":"success""#,
        ] {
            let got = parse_query_range(text, &Limits::default());
            assert!(got.is_err(), "{text:?} parsed as {got:?}");
        }
        assert!(matches!(fault("{}"), WireFault::Malformed { .. }));
    }

    #[test]
    fn a_response_larger_than_the_cap_is_not_looked_at() {
        let limits = Limits {
            max_bytes: 16,
            ..Limits::default()
        };
        assert_eq!(
            parse_query_range(MEASURED, &limits).unwrap_err(),
            WireFault::TooLarge {
                bytes: MEASURED.len()
            }
        );
    }

    #[test]
    fn records_past_the_cap_are_dropped_and_counted_rather_than_lost_quietly() {
        let limits = Limits {
            max_entries: 1,
            ..Limits::default()
        };
        let batch = parse_query_range(MEASURED, &limits).expect("should still parse");
        assert_eq!(batch.entries.len(), 1);
        assert_eq!(batch.dropped, 1, "the caller has to be able to see this");
    }

    #[test]
    fn a_stream_carrying_too_many_labels_is_refused() {
        let mut labels = String::new();
        for i in 0..40 {
            if i > 0 {
                labels.push(',');
            }
            labels.push_str(&format!(r#""k{i}":"v""#));
        }
        let text = format!(
            r#"{{"status":"success","data":{{"resultType":"streams","result":[
                 {{"stream":{{{labels}}},"values":[["1","a"]]}}]}}}}"#
        );
        let limits = Limits {
            max_labels: 8,
            ..Limits::default()
        };
        assert_eq!(
            parse_query_range(&text, &limits).unwrap_err(),
            WireFault::TooManyLabels { count: 9 }
        );
    }

    #[test]
    fn a_line_longer_than_the_cap_is_refused() {
        let text = format!(
            r#"{{"status":"success","data":{{"resultType":"streams","result":[
                 {{"stream":{{}},"values":[["1","{}"]]}}]}}}}"#,
            "x".repeat(500)
        );
        let limits = Limits {
            max_line_len: 100,
            ..Limits::default()
        };
        assert!(matches!(
            parse_query_range(&text, &limits),
            Err(WireFault::LineTooLong { .. })
        ));
    }

    #[test]
    fn a_label_longer_than_the_cap_is_refused() {
        let text = format!(
            r#"{{"status":"success","data":{{"resultType":"streams","result":[
                 {{"stream":{{"k":"{}"}},"values":[["1","a"]]}}]}}}}"#,
            "x".repeat(500)
        );
        let limits = Limits {
            max_label_len: 100,
            ..Limits::default()
        };
        assert!(matches!(
            parse_query_range(&text, &limits),
            Err(WireFault::LabelTooLong { .. })
        ));
    }

    #[test]
    fn a_response_nested_to_exhaust_the_stack_is_refused_before_it_does() {
        let deep = format!(
            r#"{{"status":"success","junk":{}{},"data":{{"resultType":"streams","result":[]}}}}"#,
            "[".repeat(200),
            "]".repeat(200)
        );
        assert!(matches!(
            parse_query_range(&deep, &Limits::default()),
            Err(WireFault::TooDeep { .. })
        ));
    }

    #[test]
    fn the_intern_table_stops_growing_and_keeps_working() {
        let mut streams = String::new();
        for i in 0..50 {
            if i > 0 {
                streams.push(',');
            }
            streams.push_str(&format!(
                r#"{{"stream":{{"k{i}":"v{i}"}},"values":[["{i}","line {i}"]]}}"#
            ));
        }
        let text = format!(
            r#"{{"status":"success","data":{{"resultType":"streams","result":[{streams}]}}}}"#
        );
        let limits = Limits {
            max_interned: 8,
            ..Limits::default()
        };
        let batch = parse_query_range(&text, &limits).expect("should still parse");
        assert_eq!(batch.entries.len(), 50, "every record still arrives");
        assert!(batch.interner.is_saturated());
        assert_eq!(batch.entries[49].label(&batch.interner, "k49"), Some("v49"));
        assert_eq!(batch.entries[0].label(&batch.interner, "k0"), Some("v0"));
    }

    /// The defect this catches is not the error — the old code returned `LineTooLong` too. It is
    /// that the error arrived *after* the whole line had been allocated. `len` is the proxy: the
    /// old reader reported the true length of the run it had already built, so it could only
    /// answer 4,194,304 here, and a reader that stops at the cap can only answer 65.
    #[test]
    fn a_long_line_is_refused_without_being_built_first() {
        let text = format!(
            r#"{{"status":"success","data":{{"resultType":"streams","result":[
                 {{"stream":{{}},"values":[["1","{}"]]}}]}}}}"#,
            "x".repeat(4 * 1024 * 1024)
        );
        let limits = Limits {
            max_line_len: 64,
            ..Limits::default()
        };
        assert_eq!(
            parse_query_range(&text, &limits).unwrap_err(),
            WireFault::LineTooLong { len: 65 },
            "the reader read the whole line before deciding it was too long"
        );
    }

    #[test]
    fn a_long_label_is_refused_without_being_built_first() {
        let text = format!(
            r#"{{"status":"success","data":{{"resultType":"streams","result":[
                 {{"stream":{{"k":"{}"}},"values":[["1","a"]]}}]}}}}"#,
            "x".repeat(1024 * 1024)
        );
        let limits = Limits {
            max_label_len: 32,
            ..Limits::default()
        };
        assert_eq!(
            parse_query_range(&text, &limits).unwrap_err(),
            WireFault::LabelTooLong { len: 33 }
        );
    }

    #[test]
    fn a_label_is_reported_as_a_label_even_when_the_two_caps_are_equal() {
        let text = format!(
            r#"{{"status":"success","data":{{"resultType":"streams","result":[
                 {{"stream":{{"k":"{}"}},"values":[["1","a"]]}}]}}}}"#,
            "x".repeat(200)
        );
        let limits = Limits {
            max_label_len: 64,
            max_line_len: 64,
            ..Limits::default()
        };
        assert_eq!(
            parse_query_range(&text, &limits).unwrap_err(),
            WireFault::LabelTooLong { len: 65 },
            "which cap was passed must not be guessed from the number"
        );
    }

    #[test]
    fn an_absurd_timestamp_is_not_reported_as_a_label() {
        let text = format!(
            r#"{{"status":"success","data":{{"resultType":"streams","result":[
                 {{"stream":{{}},"values":[["{}","a"]]}}]}}}}"#,
            "9".repeat(200)
        );
        assert!(matches!(
            parse_query_range(&text, &Limits::default()),
            Err(WireFault::Malformed {
                wanted: "a nanosecond timestamp",
                ..
            })
        ));
    }

    #[test]
    fn a_caller_cannot_raise_the_nesting_limit_into_a_stack_overflow() {
        let deep = format!(
            r#"{{"status":"success","junk":{}{},"data":{{"resultType":"streams","result":[]}}}}"#,
            "[".repeat(20_000),
            "]".repeat(20_000)
        );
        let limits = Limits {
            max_depth: usize::MAX,
            ..Limits::default()
        };
        assert!(
            matches!(
                parse_query_range(&deep, &limits),
                Err(WireFault::TooDeep { .. })
            ),
            "the one cap whose breach cannot be caught must not be caller-settable"
        );
    }

    #[test]
    fn per_record_metadata_beats_the_streams_label_of_the_same_name() {
        let batch = read(
            r#"{"status":"success","data":{"resultType":"streams","result":[
                 {"stream":{"app":"stream-said"},
                  "values":[["1","a",{"app":"record-said"}]]}]}}"#,
        );
        assert_eq!(
            batch.entries[0].label(&batch.interner, "app"),
            Some("record-said"),
            "the server-merged form resolves this collision the same way"
        );
    }

    #[test]
    fn a_second_values_array_is_refused_rather_than_replacing_the_first() {
        let got = parse_query_range(
            r#"{"status":"success","data":{"resultType":"streams","result":[
                 {"stream":{"app":"x"},"values":[["1","first"],["2","second"]],
                  "values":[["3","third"]]}]}}"#,
            &Limits::default(),
        );
        assert!(
            got.is_err(),
            "two records vanished with dropped == 0: {got:?}"
        );
    }

    #[test]
    fn a_fault_cannot_write_the_servers_own_message_into_the_status_bar() {
        let hostile = r#"{"status":"\u001b[2J\u001b[31mLOKI: run tailhawk --reset\r\n\u202e"}"#;
        let WireFault::ServerSaidNo { status } =
            parse_query_range(hostile, &Limits::default()).unwrap_err()
        else {
            panic!("expected the server's own refusal");
        };
        assert!(!status.contains('\u{1b}'), "an escape survived: {status:?}");
        assert!(!status.contains('\r') && !status.contains('\n'));
        assert!(!status.contains('\u{202e}'), "a bidi override survived");
        assert_eq!(status, "[2J[31mLOKI: run tailhawk --reset");
    }

    #[test]
    fn a_fault_repeats_back_only_so_much_of_what_the_server_said() {
        let long = "z".repeat(4_000);
        let text = format!(r#"{{"status":"{long}"}}"#);
        let WireFault::ServerSaidNo { status } =
            parse_query_range(&text, &Limits::default()).unwrap_err()
        else {
            panic!("expected the server's own refusal");
        };
        assert!(status.chars().count() <= MAX_SAID + 1, "{}", status.len());
        assert!(status.ends_with('…'));
    }

    /// **The spill is read back by the detector that will actually read it.**
    ///
    /// `format.rs`'s `ndjson` pattern matches `@t`, then `@l`, then `@m` *sequentially*, so a spill
    /// with the fields in any other order would be a file Tailhawk could not recognise as the
    /// format it had just written — and an assertion against an expected string would pass while
    /// that was true. So the oracle is the real format.
    #[test]
    fn a_spilled_record_is_read_back_by_the_real_detector() {
        let batch = read(MEASURED);
        let line = clef_line(&batch.entries[0], &batch.interner);
        let ndjson = crate::format::by_id("ndjson").expect("the format the spill targets");

        assert!(
            ndjson.is_first_line(&line),
            "the detector does not claim {line}"
        );
        let record = ndjson.parse(&line).expect("and cannot read it");
        assert_eq!(
            record.severity_number.map(crate::record::Severity::get),
            Some(17),
            "Error came back through the spill"
        );

        // The contract, stated directly as well as exercised: the pattern matches these three in
        // this order, so their positions in the text are the thing that must not drift.
        let at = |needle: &str| {
            line.find(needle)
                .unwrap_or_else(|| panic!("{needle} in {line}"))
        };
        assert!(
            at("\"@t\"") < at("\"@l\"") && at("\"@l\"") < at("\"@m\""),
            "@t, then @l, then @m — the detector reads them sequentially: {line}"
        );
    }

    /// **A label whose name the detector also reads as a level breaks the whole line.**
    ///
    /// The round-trip test above uses a fixture that always carries `severity_text`, so `@l` is
    /// always written and this branch was never exercised — the claim was only ever tested on the
    /// safe half. With no level and a stream label called `severity`, `@l` is omitted, and
    /// `format.rs`'s pattern — which matches ts, then level, then message *in that order* — finds
    /// its level in the trailing label, leaving nothing after it for the message group. The
    /// fallback then makes the body the **entire raw JSON line**, on every row of that stream.
    #[test]
    fn a_label_the_detector_would_read_as_a_level_does_not_swallow_the_message() {
        let batch = read(
            r#"{"status":"success","data":{"resultType":"streams","result":[
                 {"stream":{"app":"checkout","severity":"page-2"},
                  "values":[["1724750000000000001","the message itself"]]}]}}"#,
        );
        let line = clef_line(&batch.entries[0], &batch.interner);
        let ndjson = crate::format::by_id("ndjson").expect("the format the spill targets");
        let record = ndjson
            .parse(&line)
            .expect("the detector must still read it");
        assert_eq!(
            record.body, "the message itself",
            "the message came back as something else — {line}"
        );
    }

    #[test]
    fn a_spilled_record_carries_the_timestamp_to_the_nanosecond() {
        let batch = read(MEASURED);
        let line = clef_line(&batch.entries[0], &batch.interner);
        // 1_724_750_000_000_000_001 ns — the trailing 1 ns is the point.
        assert!(
            line.contains("\"@t\":\"2024-08-27T09:13:20.000000001Z\""),
            "{line}"
        );
    }

    #[test]
    fn a_spill_drops_lokis_own_bookkeeping_and_keeps_the_rest() {
        let batch = read(
            r#"{"status":"success","data":{"resultType":"streams","result":[
                 {"stream":{"app":"checkout","__stream_shard__":"3","level":"error"},
                  "values":[["1","went wrong"]]}]}}"#,
        );
        let line = clef_line(&batch.entries[0], &batch.interner);
        assert!(
            !line.contains("__stream_shard__"),
            "§2 suppresses these: {line}"
        );
        assert!(line.contains("\"app\":\"checkout\""), "{line}");
        assert!(line.contains("\"@l\":\"error\""), "{line}");
        assert_eq!(
            line.matches("level").count(),
            0,
            "the level is @l and is not repeated as a label: {line}"
        );
    }

    #[test]
    fn a_message_that_would_break_the_json_is_escaped() {
        let batch = read(
            r#"{"status":"success","data":{"resultType":"streams","result":[
                 {"stream":{"app":"x"},"values":[["1","he said \"no\" and\nleft\ttoday"]]}]}}"#,
        );
        let line = clef_line(&batch.entries[0], &batch.interner);
        let ndjson = crate::format::by_id("ndjson").expect("ndjson");
        assert!(
            ndjson.is_first_line(&line),
            "an escaped message still parses as one line: {line}"
        );
        assert!(
            !line.contains('\n'),
            "a raw newline would split the record in two"
        );
    }

    #[test]
    fn a_batch_spills_one_line_per_record() {
        let batch = read(MEASURED);
        let spill = clef_spill(&batch);
        assert_eq!(spill.lines().count(), batch.entries.len());
        assert!(
            spill.ends_with('\n'),
            "every record is terminated, including the last"
        );
    }

    /// The inverse of `filter.rs`'s `days_from_civil`, checked against it rather than against a
    /// table of answers copied from somewhere — a copied table can be wrong in the same way twice.
    #[test]
    fn a_day_number_and_a_civil_date_agree_in_both_directions() {
        for day in [
            -25_567_i64,
            -1,
            0,
            1,
            11_016,
            11_017,
            20_000,
            30_000,
            40_000,
        ] {
            let (y, m, d) = civil_from_days(day);
            assert_eq!(
                crate::filter::days_from_civil(y, i64::from(m), i64::from(d)),
                Some(day),
                "{day} became {y:04}-{m:02}-{d:02}, which is a different day"
            );
        }
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(11_016), (2000, 2, 29), "a leap day");
    }

    /// An instant before the epoch is not a Loki timestamp, but the arithmetic must not produce
    /// nonsense if one arrives — `div_euclid` rather than `/` is the whole reason.
    #[test]
    fn an_instant_before_the_epoch_still_formats_as_a_real_time() {
        let text = rfc3339_nanos(-1);
        assert_eq!(text, "1969-12-31T23:59:59.999999999Z");
    }

    #[test]
    fn nothing_in_the_response_text_is_stripped_or_rewritten() {
        let batch = read(
            r#"{"status":"success","data":{"resultType":"streams","result":[
                 {"stream":{"app":"x"},"values":[["1","\u001b[31mred\u001b[0m\u0007"]]}]}}"#,
        );
        assert_eq!(
            batch.entries[0].line, "\u{1b}[31mred\u{1b}[0m\u{7}",
            "stripping belongs where it is rendered, not where it is read"
        );
    }
}
