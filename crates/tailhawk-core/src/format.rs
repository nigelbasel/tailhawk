//! The format catalogue — `SPEC.md` §6.3's stage-4 formats and §6.4's first-line rule, E10.
//!
//! A [`Format`] is a **`^`-anchored first-line pattern** with named captures, a rule for reading
//! its timestamp, the level words it declares, a continuation predicate for its family, a
//! specificity from §6.3's table, and **sample lines with their expected levels**. The samples are
//! not decoration: §6.3 says "detector ordering is a build-time unit test, not a runtime
//! heuristic", and [`tests::no_generic_format_outscores_a_specific_one_on_its_own_samples`] is that
//! test — every format's samples against every format's pattern.
//!
//! ## What a format produces
//!
//! [`Format::parse`] turns one line into `record.rs`'s [`Record`]: timestamp, severity (through
//! the OTel banding, §6.2), body, and the remaining named captures as **attributes** — which is what
//! becomes columns. `raw` is always the whole line, per §6.1: lossless.
//!
//! ## Written from what each framework documents it emits
//!
//! Serilog's default file and console templates, MEL's `SimpleConsoleFormatter`, log4net's
//! `PatternLayout`, NLog's default layout, RFC 5424 and RFC 3164, the Apache log formats, Python's
//! `logging` defaults, W3C Extended, Serilog CLEF, logfmt. **No log viewer's catalogue was read** —
//! `CLEANROOM.md`, 2026-08-17. The patterns are tested against the samples here and against the
//! owner's own logs.
//!
//! ## Rows stay lines
//!
//! §6.4 makes a record start "if and only if the line matches the format's first-line anchor"; the
//! rest are continuations. That is a *view* over lines, and it is built as one — see the decision
//! recorded in `CLEANROOM.md`: a sorted list of continuation lines, identity when empty, the shape
//! the filter's survivor list already has. Nothing here renumbers anything.

use std::sync::OnceLock;

use regex::Regex;

use crate::filter::parse_instant;
use crate::record::{AttributeValue, FormatId, ParseState, Record, Severity, SeverityBand};

/// How a format's `ts` capture becomes a [`Timestamp`](crate::record::Timestamp).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Stamp {
    /// ISO-8601 / RFC 3339, with `T` or a space, `.` or `,` before the fraction, optional zone.
    Iso,
    /// `HH:mm:ss[.fff]` alone — Serilog's console template. **Valid but yields no instant.**
    TimeOnly,
    /// RFC 3164 `Mmm dd HH:mm:ss` — no year, no zone. Valid but yields no instant.
    Bsd,
    /// Apache CLF `dd/Mmm/yyyy:HH:mm:ss +zzzz`.
    Clf,
    /// No timestamp capture at all.
    None,
}

/// How the `level` capture becomes a severity.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Level {
    /// A level word — `record.rs`'s `from_level_text` decides.
    Word,
    /// A syslog PRI value: `severity = pri % 8`.
    Pri,
    /// An HTTP status code.
    HttpStatus,
    /// The format carries no level.
    None,
}

/// One built-in format.
pub struct Format {
    pub id: &'static str,
    pub name: &'static str,
    /// §6.3's stage-4 ordering. Ties broken by catalogue order.
    pub specificity: f32,
    first_line: Regex,
    pub stamp: Stamp,
    pub level: Level,
    /// The level words this format emits, when it declares a closed set — what makes
    /// `field_validity` bite. Empty means "any word `from_level_text` accepts".
    pub levels: &'static [&'static str],
    continuation: Option<Regex>,
    /// The named captures shown as columns, in order. `ts`, `level` and `msg` are understood.
    pub columns: &'static [&'static str],
    /// First lines this format must parse, with the band each carries.
    pub samples: &'static [(&'static str, Option<SeverityBand>)],
    /// The message is the *next* line, not part of the first — MEL Simple's `info: Logger[0]`
    /// followed by six spaces and the text. Under §6.4 that line is a continuation; a collapsed
    /// view assembles it into the record's message column.
    pub body_next_line: bool,
    /// Column titles, when they differ from the capture names — W3C's `cs(User-Agent)` cannot be a
    /// capture name. Parallel to [`columns`](Self::columns); `None` means the names are the titles.
    pub titles: Option<&'static [&'static str]>,
}

impl Format {
    /// Each of [`columns`](Self::columns) as the header names it: the declared title where there is
    /// one, the capture name otherwise. The same choice [`Layout::title`](crate::columns::Layout::title)
    /// makes, without needing a layout to ask it.
    pub fn column_titles(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.columns.iter().enumerate().map(|(i, name)| {
            self.titles
                .and_then(|titles| titles.get(i).copied())
                .unwrap_or(name)
        })
    }

    /// Marks the message as living on the line after the first — MEL Simple. See [`Format::body_next_line`].
    fn with_body_on_next_line(mut self) -> Self {
        self.body_next_line = true;
        self
    }
}

impl std::fmt::Debug for Format {
    /// The id and the specificity — the compiled patterns are pages of state nobody wants.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Format({} {})", self.id, self.specificity)
    }
}

impl PartialEq for Format {
    /// Two formats are the same format if they have the same id; the catalogue holds one of each.
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

/// A first-line pattern's names, so a caller can ask a format for its columns without a match.
pub const TS: &str = "ts";
pub const LEVEL: &str = "level";
pub const MSG: &str = "msg";

impl Format {
    /// The first-line pattern's source — for a template compiler's tests and a wizard's display.
    pub fn pattern(&self) -> &str {
        self.first_line.as_str()
    }

    /// Whether `line` starts a record under this format — §6.4's rule.
    ///
    /// **The one-byte dispatch is `regex`'s own literal prefilter**: every pattern here is
    /// `^`-anchored and begins with a literal or a small class, and the crate turns that into a
    /// first-byte check before the automaton runs. §6.4's "single largest parser performance win"
    /// is had without a second table to keep in step with the patterns.
    pub fn is_first_line(&self, line: &str) -> bool {
        self.first_line.is_match(line)
    }

    /// Whether `line` is a continuation of this format's family — a stack-trace line, an indented
    /// message. `false` for a format with no predicate, in which case any non-first line is one.
    pub fn is_continuation(&self, line: &str) -> bool {
        self.continuation.as_ref().is_some_and(|c| c.is_match(line))
    }

    /// Parses a first line, or `None` if it is not one. `raw` is the whole line, always.
    pub fn parse(&self, line: &str) -> Option<Record> {
        let caps = self.first_line.captures(line)?;
        let mut record = Record {
            raw: line.to_owned(),
            parse_state: ParseState::Parsed,
            format_id: Some(FormatId::new(self.id)),
            ..Record::default()
        };
        for name in self.first_line.capture_names().flatten() {
            let Some(m) = caps.name(name) else {
                continue;
            };
            let text = m.as_str();
            match name {
                TS => record.timestamp = self.stamp.parse(text),
                LEVEL => {
                    record.severity_text = Some(text.to_owned());
                    record.severity_number = self.level.parse(text);
                }
                MSG => record.body = text.to_owned(),
                _ => record
                    .attributes
                    .push((name.to_owned(), AttributeValue::String(text.to_owned()))),
            }
        }
        if record.body.is_empty() && caps.name(MSG).is_none() {
            record.body = line.to_owned();
        }
        Some(record)
    }

    /// The byte range of each of [`columns`](Self::columns) in a first line, in column order —
    /// `None` for a column whose capture did not participate, and `None` overall when the line is
    /// not a first line. What a columnised presentation of the row is built from: byte ranges into
    /// the raw line, so a search match can be carried across.
    pub fn fields(&self, line: &str) -> Option<Vec<Option<core::ops::Range<usize>>>> {
        let caps = self.first_line.captures(line)?;
        Some(
            self.columns
                .iter()
                .map(|name| caps.name(name).map(|m| m.range()))
                .collect(),
        )
    }

    /// The parts of §6.3's `field_validity` a single line can answer: does the timestamp read as
    /// one, and is the level a word this format declares. `None` when the line is not a first
    /// line.
    pub fn validity(&self, line: &str) -> Option<bool> {
        let caps = self.first_line.captures(line)?;
        let ts_ok = match (self.stamp, caps.name(TS)) {
            (Stamp::None, _) => true,
            (Stamp::TimeOnly | Stamp::Bsd, Some(_)) => true,
            (stamp, Some(m)) => stamp.parse(m.as_str()).is_some(),
            // An optional group that did not participate: a mandatory one is always captured on a
            // match, so absence here is the format allowing it, as MEL Simple does.
            (_, None) => true,
        };
        let level_ok = match (self.level, caps.name(LEVEL)) {
            (Level::None, _) => true,
            (Level::Word, Some(m)) if !self.levels.is_empty() => self
                .levels
                .iter()
                .any(|l| l.eq_ignore_ascii_case(m.as_str())),
            (level, Some(m)) => level.parse(m.as_str()).is_some(),
            (_, None) => self.levels.is_empty(),
        };
        Some(ts_ok && level_ok)
    }
}

impl Stamp {
    /// The instant, when the shape carries one.
    pub fn parse(self, text: &str) -> Option<crate::record::Timestamp> {
        match self {
            Stamp::Iso => {
                // log4net and Python write the fraction with a comma; NLog with four digits.
                let normalised = text
                    .replacen(',', ".", 1)
                    .replacen(" +", "+", 1)
                    .replacen(" -", "-", 1);
                parse_instant(normalised.trim())
            }
            Stamp::Clf => {
                // `16/Aug/2026:09:14:02 +0200`
                let (date, zone) = text.split_once(' ')?;
                let mut parts = date.splitn(3, '/');
                let day = parts.next()?;
                let mon = month(parts.next()?)?;
                let (year, time) = parts.next()?.split_once(':')?;
                let zone = format!("{}:{}", &zone[..3], &zone[3..]);
                parse_instant(&format!("{year}-{mon:02}-{day:0>2}T{time}{zone}"))
            }
            Stamp::TimeOnly | Stamp::Bsd | Stamp::None => None,
        }
    }
}

fn month(name: &str) -> Option<u32> {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    MONTHS
        .iter()
        .position(|m| m.eq_ignore_ascii_case(name))
        .map(|i| i as u32 + 1)
}

impl Level {
    pub fn parse(self, text: &str) -> Option<Severity> {
        match self {
            Level::Word => Severity::from_level_text(text),
            Level::Pri => text
                .parse::<u16>()
                .ok()
                .and_then(|pri| Severity::from_syslog((pri % 8) as u8)),
            Level::HttpStatus => text
                .parse::<u16>()
                .ok()
                .and_then(Severity::from_http_status),
            Level::None => None,
        }
    }
}

const ISO: &str = r"\d{4}-\d{2}-\d{2}[T ]\d{2}:\d{2}:\d{2}(?:[.,]\d+)?(?:Z|[+-]\d{2}:?\d{2})?";
const SERILOG_LEVELS: &[&str] = &["VRB", "DBG", "INF", "WRN", "ERR", "FTL"];
const MEL_LEVELS: &[&str] = &["trce", "dbug", "info", "warn", "fail", "crit"];
const WORD_LEVELS: &[&str] = &[
    "TRACE", "DEBUG", "INFO", "WARN", "WARNING", "ERROR", "FATAL", "CRITICAL",
];
const PY_LEVELS: &[&str] = &["DEBUG", "INFO", "WARNING", "ERROR", "CRITICAL"];
const DOTNET_CONTINUATION: &str = r"^\s*(at\s|--->\s|--- End of inner exception|\S+Exception\b)";
const JAVA_CONTINUATION: &str = r"^\s*(at\s|Caused by:|\.\.\.\s\d+\smore|Suppressed:)";
const PY_CONTINUATION: &str = r#"^(Traceback \(most recent call last\):|\s{2,}File ")"#;

fn re(source: &str) -> Regex {
    Regex::new(source).unwrap_or_else(|e| panic!("catalogue pattern {source:?}: {e}"))
}

macro_rules! fmt {
    ($id:literal, $name:literal, $spec:literal, $first:expr, $stamp:expr, $level:expr, $levels:expr, $cont:expr, $cols:expr, $samples:expr $(,)?) => {
        Format {
            id: $id,
            name: $name,
            specificity: $spec,
            first_line: re($first),
            stamp: $stamp,
            level: $level,
            levels: $levels,
            continuation: $cont.map(re),
            columns: $cols,
            samples: $samples,
            body_next_line: false,
            titles: None,
        }
    };
}

/// What a compiled template needs to become a [`Format`] — `template.rs` fills one of these.
pub struct Custom {
    pub id: String,
    pub name: String,
    pub specificity: f32,
    /// The `^`-anchored first-line pattern with named captures. Compiled here, and a pattern the
    /// engine refuses is an error, not a panic — this one comes from a user's config.
    pub pattern: String,
    pub stamp: Stamp,
    pub level: Level,
    pub levels: Vec<String>,
    pub continuation: Option<String>,
    pub columns: Vec<String>,
}

/// A format built at run time — from a template (E11) or a directive — leaked once, for the same
/// reason [`w3c`] gives.
pub fn custom(spec: Custom) -> Result<&'static Format, String> {
    let first_line = Regex::new(&spec.pattern).map_err(|e| e.to_string())?;
    let continuation = match &spec.continuation {
        Some(c) => Some(Regex::new(c).map_err(|e| e.to_string())?),
        None => None,
    };
    let leak = |s: &str| -> &'static str { Box::leak(s.to_owned().into_boxed_str()) };
    let leak_all = |v: &[String]| -> &'static [&'static str] {
        Box::leak(
            v.iter()
                .map(|s| leak(s))
                .collect::<Vec<_>>()
                .into_boxed_slice(),
        )
    };
    Ok(Box::leak(Box::new(Format {
        id: leak(&spec.id),
        name: leak(&spec.name),
        specificity: spec.specificity,
        first_line,
        stamp: spec.stamp,
        level: spec.level,
        levels: leak_all(&spec.levels),
        continuation,
        columns: leak_all(&spec.columns),
        samples: &[],
        body_next_line: false,
        titles: None,
    })))
}

/// A format for one W3C Extended file, from its `#Fields:` directive — §6.3's short-circuit,
/// "take columns verbatim". Fields are whitespace-separated; `sc-status` is the level; a `#`
/// directive line is not a record.
///
/// **Leaked, deliberately.** The catalogue is `'static` and everything downstream holds a
/// `&'static Format`; a W3C format is one per opened file and a few hundred bytes, and a
/// `Box::leak` is the honest cost of not making every format own its strings for the one that
/// is not a constant. Recorded in `HANDOFF.md`.
pub fn w3c(fields: &[String]) -> &'static Format {
    let names: Vec<String> = fields
        .iter()
        .map(|f| {
            if f == "sc-status" {
                LEVEL.to_owned()
            } else {
                let mut name: String = f
                    .chars()
                    .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
                    .collect();
                if name.starts_with(|c: char| c.is_ascii_digit()) {
                    name.insert(0, 'f');
                }
                name
            }
        })
        .collect();
    let mut pattern = String::from("^");
    for (i, name) in names.iter().enumerate() {
        if i > 0 {
            pattern.push(' ');
        }
        // The first field cannot start with `#`: a directive line has enough tokens to match.
        if i == 0 {
            pattern.push_str(&format!(r"(?P<{name}>[^#\s]\S*)"));
        } else if i + 1 == names.len() {
            pattern.push_str(&format!("(?P<{name}>.*)"));
        } else {
            pattern.push_str(&format!(r"(?P<{name}>\S+)"));
        }
    }
    pattern.push('$');
    let leak = |s: &str| -> &'static str { Box::leak(s.to_owned().into_boxed_str()) };
    let columns: &'static [&'static str] = Box::leak(
        names
            .iter()
            .map(|n| leak(n))
            .collect::<Vec<_>>()
            .into_boxed_slice(),
    );
    let titles: &'static [&'static str] = Box::leak(
        fields
            .iter()
            .map(|f| leak(f))
            .collect::<Vec<_>>()
            .into_boxed_slice(),
    );
    let has_status = names.iter().any(|n| n == LEVEL);
    Box::leak(Box::new(Format {
        id: "w3c",
        name: "W3C Extended",
        specificity: 1.0,
        first_line: re(&pattern),
        stamp: Stamp::None,
        level: if has_status {
            Level::HttpStatus
        } else {
            Level::None
        },
        levels: &[],
        continuation: Some(re("^#")),
        columns,
        samples: &[],
        body_next_line: false,
        titles: Some(titles),
    }))
}

/// The capture name a JSON key plays, when it is one of the three the grid understands.
///
/// The spellings are the `ndjson` catalogue entry's own, deliberately: a key that reader would
/// have taken as the timestamp must not arrive here as an ordinary column, or the same file would
/// have a `ts` column under one reader and a `@t` column under the other.
pub(crate) fn understood_key(key: &str) -> Option<&'static str> {
    match key {
        "@t" | "time" | "timestamp" | "ts" => Some(TS),
        "@l" | "level" | "lvl" | "severity" => Some(LEVEL),
        "@m" | "@mt" | "msg" | "message" => Some(MSG),
        _ => None,
    }
}

/// The columns a JSON template yields: the capture name, and the raw key as its title.
///
/// A key is dropped when its value is not a string (see [`JsonKey`](crate::detect::JsonKey)), when
/// its sanitised name collides with one already taken — two keys spelled `a.b` and `a-b` would
/// compile to one capture name, which is an error rather than a column — or when the extras are
/// already at [`MAX_JSON_COLUMNS`](crate::detect::MAX_JSON_COLUMNS).
/// **Two passes, and the order of them is the point.** The understood roles are bound first, from
/// the *raw* key, so that a key spelled `msg ` or `@level` — which sanitises onto a role's own
/// name — cannot take a role's place merely by appearing earlier in the record. A single pass did
/// exactly that: `{"msg ":"…","@m":"the real message"}` gave the `msg` column to the decoy and
/// dropped `@m` from the grid entirely, message and all.
///
/// Extras are then sanitised into capture names and **disambiguated with a suffix** rather than
/// dropped. Two keys spelled `a.b` and `a-b` compile to one capture name, which `regex` refuses,
/// and any two non-ASCII keys sanitise to nothing at all — silently losing the second is a column
/// of real data gone with no sign of it.
fn json_columns(keys: &[crate::detect::JsonKey]) -> Vec<(String, String)> {
    let text: Vec<&crate::detect::JsonKey> = keys.iter().filter(|k| k.is_text()).collect();
    let mut roles: Vec<(String, String)> = Vec::new();
    for key in &text {
        if let Some(role) = understood_key(&key.name) {
            if !roles.iter().any(|(taken, _)| taken == role) {
                roles.push((role.to_owned(), key.name.clone()));
            }
        }
    }
    let mut out: Vec<(String, String)> = Vec::new();
    let mut extras = 0usize;
    for key in &text {
        if let Some(found) = roles.iter().find(|(_, raw)| *raw == key.name) {
            out.push(found.clone());
            continue;
        }
        if extras == crate::detect::MAX_JSON_COLUMNS {
            continue;
        }
        extras += 1;
        let sanitised: String = key
            .name
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect();
        let mut stem = sanitised.trim_matches('_').to_owned();
        if stem.is_empty() || stem.starts_with(|c: char| c.is_ascii_digit()) {
            stem.insert(0, 'k');
        }
        let mut name = stem.clone();
        let mut suffix = 2usize;
        while roles
            .iter()
            .chain(out.iter())
            .any(|(taken, _)| *taken == name)
        {
            name = format!("{stem}{suffix}");
            suffix += 1;
        }
        out.push((name, key.name.clone()));
    }
    out
}

/// Whether a JSON template is worth a format of its own.
///
/// Two conditions, both necessary. It must carry **something beyond** the three columns the
/// catalogue's `ndjson` reader already shows, or there is nothing to gain by leaving it. And it
/// must have a **message column**, because §2.5 gives the grid's last column the free remainder of
/// the width and [`json_lines`] puts the message there — with no message key at all, a label would
/// inherit the whole window.
pub fn json_lines_adds_columns(keys: &[crate::detect::JsonKey]) -> bool {
    let columns = json_columns(keys);
    let has_message = columns.iter().any(|(name, _)| name == MSG);
    let extras = columns.iter().any(|(_, raw)| understood_key(raw).is_none());
    has_message && extras
}

/// A format for JSON lines that share one key order — `SPEC.md` §6.3 stage 2, the self-describing
/// branch a `#Fields:` line takes for W3C. See [`detect::json_template`](crate::detect::json_template).
///
/// Every group is **optional**, so a line missing a key the template has still matches and simply
/// leaves that cell empty; what a line may not do is write its keys in a different order, which is
/// what `json_template` refuses up front.
///
/// **Leaked, for the reason [`w3c`] is**: the catalogue is `'static`, everything downstream holds a
/// `&'static Format`, and this is one small allocation per file opened.
pub fn json_lines(keys: &[crate::detect::JsonKey]) -> &'static Format {
    let columns = json_columns(keys);
    // **The pattern follows the file's key order; the columns do not have to.** A regex must
    // consume its groups in the order the text writes them, but `Format::fields` looks each column
    // up *by name*, so display order is free — which is what lets the message go last whatever the
    // file does with it.
    let mut pattern = String::from(r"^\s*\{");
    for (name, raw) in &columns {
        pattern.push_str(&format!(
            r#"(?:.*?"{}"\s*:\s*"(?P<{name}>(?:[^"\\]|\\.)*)")?"#,
            regex::escape(raw)
        ));
    }
    pattern.push_str(r".*\}\s*$");
    // §2.5 gives the last column the free remainder of the width, and that has to be the message.
    // Bunyan writes `name, hostname, pid, level, msg, time, v`; left in file order the message
    // would be capped at `columns::MAX_CELLS` and a twenty-character timestamp would be handed the
    // rest of the window.
    let mut shown = columns.clone();
    if let Some(at) = shown.iter().position(|(name, _)| name == MSG) {
        let message = shown.remove(at);
        shown.push(message);
    }
    let leak = |s: &str| -> &'static str { Box::leak(s.to_owned().into_boxed_str()) };
    let names: &'static [&'static str] = Box::leak(
        shown
            .iter()
            .map(|(name, _)| leak(name))
            .collect::<Vec<_>>()
            .into_boxed_slice(),
    );
    let titles: &'static [&'static str] = Box::leak(
        shown
            .iter()
            .map(|(_, raw)| leak(raw))
            .collect::<Vec<_>>()
            .into_boxed_slice(),
    );
    Box::leak(Box::new(Format {
        id: "json-lines",
        name: "JSON lines",
        specificity: 0.95,
        first_line: re(&pattern),
        stamp: Stamp::Iso,
        level: Level::Word,
        levels: &[],
        continuation: None,
        columns: names,
        samples: &[],
        body_next_line: false,
        titles: Some(titles),
    }))
}

/// The built-in catalogue, in §6.3's specificity order. Built once.
pub fn catalogue() -> &'static [Format] {
    static CATALOGUE: OnceLock<Vec<Format>> = OnceLock::new();
    CATALOGUE.get_or_init(build)
}

/// A format by id.
pub fn by_id(id: &str) -> Option<&'static Format> {
    catalogue().iter().find(|f| f.id == id)
}

fn build() -> Vec<Format> {
    use SeverityBand::*;
    vec![
        fmt!(
            "rfc5424", "syslog (RFC 5424)", 0.95,
            r"^<(?P<level>\d{1,3})>1 (?P<ts>\S+) (?P<host>\S+) (?P<app>\S+) (?P<procid>\S+) (?P<msgid>\S+) (?P<sd>-|\[.*?\])(?: (?P<msg>.*))?$",
            Stamp::Iso, Level::Pri, &[], None::<&str>,
            &["ts", "host", "app", "procid", "msgid", "msg"],
            &[("<165>1 2026-08-16T09:14:02.117Z host01 api 4321 ID47 - dispatch failed", Some(Info)),
              ("<11>1 2026-08-16T09:14:02Z host01 sshd - - [meta seq=\"1\"] Failed password", Some(Error))],
        ),
        fmt!(
            "nlog", "NLog", 0.90,
            r"^(?P<ts>\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}\.\d{4})\|(?P<level>[A-Za-z]+)\|(?P<logger>[^|]*)\|(?P<msg>.*)$",
            Stamp::Iso, Level::Word, WORD_LEVELS, Some(DOTNET_CONTINUATION),
            &["ts", "level", "logger", "msg"],
            &[("2026-08-16 09:14:02.1170|INFO|Api.Controller|Started", Some(Info)),
              ("2026-08-16 09:14:03.8840|ERROR|Api.Dispatch|Failed to dispatch job 41982", Some(Error))],
        ),
        fmt!(
            "mel-simple", "MEL Simple", 0.90,
            r"^(?:(?P<ts>\d{4}-\d{2}-\d{2}[T ]\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:\d{2})?) )?(?P<level>trce|dbug|info|warn|fail|crit): (?P<logger>\S+)\[(?P<event>\d+)\]$",
            Stamp::Iso, Level::Word, MEL_LEVELS, Some(r"^ {6}\S|^\s*(at\s|--->\s|\S+Exception\b)"),
            &["ts", "level", "logger", "event", "msg"],
            &[("info: Microsoft.Hosting.Lifetime[14]", Some(Info)),
              ("fail: Api.Dispatch[0]", Some(Error)),
              ("2026-08-16 09:14:02 warn: Api.Sql[0]", Some(Warn))],
        ).with_body_on_next_line(),
        fmt!(
            "serilog-file", "Serilog (file)", 0.85,
            r"^(?P<ts>\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}\.\d{3} [+-]\d{2}:\d{2}) \[(?P<level>VRB|DBG|INF|WRN|ERR|FTL)\] (?P<msg>.*)$",
            Stamp::Iso, Level::Word, SERILOG_LEVELS, Some(DOTNET_CONTINUATION),
            &["ts", "level", "msg"],
            &[("2026-08-16 09:14:02.117 +02:00 [INF] Started HTTP GET /api/contacts", Some(Info)),
              ("2026-08-16 09:14:03.884 +02:00 [ERR] Failed to dispatch job 41982", Some(Error))],
        ),
        fmt!(
            "clf", "Apache / nginx access", 0.85,
            r#"^(?P<host>\S+) (?P<ident>\S+) (?P<user>\S+) \[(?P<ts>[^\]]+)\] "(?P<request>[^"]*)" (?P<level>\d{3}) (?P<bytes>\S+)(?: "(?P<referer>[^"]*)" "(?P<agent>[^"]*)")?"#,
            Stamp::Clf, Level::HttpStatus, &[], None::<&str>,
            &["ts", "host", "request", "level", "bytes", "referer", "agent"],
            &[(r#"10.0.0.1 - - [16/Aug/2026:09:14:02 +0200] "GET /api/contacts HTTP/1.1" 200 512"#, Some(Info)),
              (r#"10.0.0.1 - frank [16/Aug/2026:09:14:02 +0200] "POST /login HTTP/1.1" 503 12 "-" "curl/8.0""#, Some(Error))],
        ),
        fmt!(
            "log4net", "log4net", 0.80,
            r"^(?P<ts>\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2},\d{3}) \[(?P<thread>[^\]]+)\] (?P<level>[A-Z]+)\s+(?P<logger>\S+) - (?P<msg>.*)$",
            Stamp::Iso, Level::Word, WORD_LEVELS, Some(DOTNET_CONTINUATION),
            &["ts", "thread", "level", "logger", "msg"],
            &[("2026-08-16 09:14:02,117 [12] INFO  Api.Controller - Started", Some(Info)),
              ("2026-08-16 09:14:03,884 [main] ERROR Api.Dispatch - Failed to dispatch job 41982", Some(Error))],
        ),
        fmt!(
            "log4net-compact", "log4net (compact)", 0.75,
            r"^(?P<ts>\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2},\d{3}) (?P<level>[A-Z]+)\s+(?P<logger>[A-Za-z_][\w.]*)\s+(?P<msg>.*)$",
            Stamp::Iso, Level::Word, WORD_LEVELS, Some(DOTNET_CONTINUATION),
            &["ts", "level", "logger", "msg"],
            &[("2026-08-07 07:50:45,075 INFO  Compiler.Program Template Set Compiler Version 7.0.2277.0", Some(Info)),
              ("2026-08-07 07:50:46,102 ERROR Design.TemplateSet \tCould not open template", Some(Error))],
        ),
        fmt!(
            "rfc3164", "syslog (RFC 3164)", 0.70,
            r"^(?:<(?P<pri>\d{1,3})>)?(?P<ts>[A-Z][a-z]{2} [ \d]\d \d{2}:\d{2}:\d{2}) (?P<host>\S+) (?P<app>[^\s:\[]+)(?:\[(?P<procid>\d+)\])?: (?P<msg>.*)$",
            Stamp::Bsd, Level::None, &[], None::<&str>,
            &["ts", "host", "app", "procid", "msg"],
            &[("Aug 16 09:14:02 host01 sshd[4321]: Accepted publickey for nigel", None),
              ("<13>Aug  6 09:14:02 host01 cron: session opened", None)],
        ),
        fmt!(
            "serilog-console", "Serilog (console)", 0.65,
            r"^\[(?P<ts>\d{2}:\d{2}:\d{2}) (?P<level>VRB|DBG|INF|WRN|ERR|FTL)\] (?P<msg>.*)$",
            Stamp::TimeOnly, Level::Word, SERILOG_LEVELS, Some(DOTNET_CONTINUATION),
            &["ts", "level", "msg"],
            &[("[09:14:02 INF] Started HTTP GET /api/contacts", Some(Info)),
              ("[09:14:03 ERR] Failed to dispatch job 41982", Some(Error))],
        ),
        fmt!(
            "python", "Python logging", 0.60,
            r"^(?P<ts>\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2},\d{3}) - (?P<logger>\S+) - (?P<level>DEBUG|INFO|WARNING|ERROR|CRITICAL) - (?P<msg>.*)$",
            Stamp::Iso, Level::Word, PY_LEVELS, Some(PY_CONTINUATION),
            &["ts", "level", "logger", "msg"],
            &[("2026-08-16 09:14:02,117 - app.api - INFO - started", Some(Info)),
              ("2026-08-16 09:14:03,884 - app.api - ERROR - failed", Some(Error))],
        ),
        fmt!(
            "python-basic", "Python logging (basicConfig)", 0.58,
            r"^(?P<level>DEBUG|INFO|WARNING|ERROR|CRITICAL):(?P<logger>[^:\s]+):(?P<msg>.*)$",
            Stamp::None, Level::Word, PY_LEVELS, Some(PY_CONTINUATION),
            &["level", "logger", "msg"],
            &[("ERROR:root:something failed", Some(Error)),
              ("INFO:app.api:started", Some(Info))],
        ),
        fmt!(
            "ndjson", "JSON lines", 0.55,
            r#"^\{(?:.*?"(?:@t|time|timestamp|ts)"\s*:\s*"(?P<ts>[^"]+)")?(?:.*?"(?:level|lvl|severity|@l)"\s*:\s*"(?P<level>[^"]+)")?(?:.*?"(?:msg|message|@m|@mt)"\s*:\s*"(?P<msg>(?:[^"\\]|\\.)*)")?.*\}$"#,
            Stamp::Iso, Level::Word, &[], None::<&str>,
            &["ts", "level", "msg"],
            &[(r#"{"time":"2026-08-16T09:14:02Z","level":"info","msg":"started"}"#, Some(Info)),
              (r#"{"@t":"2026-08-16T09:14:03.884Z","@l":"Error","@mt":"Failed to dispatch job {Job}","Job":41982}"#, Some(Error))],
        ),
        fmt!(
            "logfmt", "logfmt", 0.45,
            r#"^(?:[A-Za-z_][\w.]*=(?:"(?:[^"\\]|\\.)*"|\S*)\s*)+$"#,
            Stamp::None, Level::None, &[], None::<&str>,
            &["msg"],
            &[(r#"ts=2026-08-16T09:14:02Z level=info msg="started" port=8080"#, None)],
        ),
        fmt!(
            "generic", "timestamped text", 0.20,
            &(String::from(r"^(?P<ts>") + ISO + r")\s+(?:\[?(?P<level>[A-Za-z]{3,9})\]?:?\s+)?(?P<msg>.*)$"),
            Stamp::Iso, Level::Word, &[], Some(JAVA_CONTINUATION),
            &["ts", "level", "msg"],
            &[("2026-08-16T09:14:02.117Z INFO  task starting E13", Some(Info)),
              ("2026-08-16 09:14:03.884 ERROR Api.Dispatch failed", Some(Error))],
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_format_parses_its_own_samples_to_the_expected_band() {
        for f in catalogue() {
            for (line, band) in f.samples {
                let record = f
                    .parse(line)
                    .unwrap_or_else(|| panic!("{}: does not parse {line:?}", f.id));
                assert_eq!(
                    record.severity_number.map(|s| s.band()),
                    *band,
                    "{}: {line:?}",
                    f.id
                );
                assert_eq!(f.validity(line), Some(true), "{}: {line:?}", f.id);
                assert_eq!(record.raw, *line, "raw is the whole line");
                assert_eq!(record.parse_state, ParseState::Parsed);
            }
        }
    }

    /// §6.3: "the build cross-matches every format's samples against every other format's pattern
    /// and fails if a generic format outscores a specific one on the specific one's samples." A
    /// less specific format may *match* a specific format's line — `generic` matches most things —
    /// but a **more** specific one must not claim it, and nothing must claim it as valid at a
    /// higher specificity than its owner.
    #[test]
    fn no_generic_format_outscores_a_specific_one_on_its_own_samples() {
        for owner in catalogue() {
            for (line, _) in owner.samples {
                for other in catalogue() {
                    if other.id == owner.id {
                        continue;
                    }
                    if other.validity(line) == Some(true) && other.specificity > owner.specificity {
                        panic!(
                            "{} ({}) claims {}'s sample as valid at higher specificity: {line:?}",
                            other.id, other.specificity, owner.id
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn timestamps_and_levels_land_in_the_record() {
        let f = by_id("serilog-file").expect("catalogue");
        let r = f
            .parse("2026-08-16 09:14:03.884 +02:00 [ERR] Failed to dispatch job 41982")
            .expect("parse");
        assert_eq!(r.body, "Failed to dispatch job 41982");
        assert_eq!(r.severity_text.as_deref(), Some("ERR"));
        assert!(r.is_error());
        let ts = r.timestamp.expect("timestamp");
        assert_eq!(ts.utc_offset_minutes, 120);
        assert_eq!(ts.unix_nanos % 1_000_000_000, 884_000_000);

        let f = by_id("log4net").expect("catalogue");
        let r = f
            .parse("2026-08-16 09:14:02,117 [12] INFO  Api.Controller - Started")
            .expect("parse");
        assert_eq!(
            r.attributes,
            [
                ("thread".to_owned(), AttributeValue::String("12".into())),
                (
                    "logger".to_owned(),
                    AttributeValue::String("Api.Controller".into())
                ),
            ]
        );
        assert!(r.timestamp.is_some(), "comma fraction is read");

        let f = by_id("clf").expect("catalogue");
        let r = f
            .parse(r#"10.0.0.1 - - [16/Aug/2026:09:14:02 +0200] "GET /x HTTP/1.1" 404 512"#)
            .expect("parse");
        assert_eq!(
            r.severity_number.map(|s| s.band()),
            Some(SeverityBand::Warn)
        );
        assert!(r.timestamp.is_some(), "CLF date is read");
    }

    #[test]
    fn continuations_are_recognised_by_family_and_never_start_a_record() {
        let f = by_id("serilog-file").expect("catalogue");
        assert!(f.is_continuation("   at Api.Dispatch.Run() in Dispatch.cs:line 42"));
        assert!(f.is_continuation("System.InvalidOperationException: boom"));
        assert!(!f.is_first_line("   at Api.Dispatch.Run()"));
        let m = by_id("mel-simple").expect("catalogue");
        assert!(m.is_continuation("      Started HTTP GET /api/contacts"));
        assert!(!m.is_first_line("      Started HTTP GET /api/contacts"));
    }

    #[test]
    fn a_first_line_check_is_the_anchored_pattern() {
        let f = by_id("generic").expect("catalogue");
        assert!(f.is_first_line("2026-08-16 09:14:02 hello"));
        assert!(!f.is_first_line("at 2026-08-16 09:14:02 a date mid-line does not start a record"));
    }
}
