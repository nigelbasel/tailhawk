//! The record model — E8, to `SPEC.md` §6.1 and §6.2.
//!
//! One normalised record that every parsed format maps into: the OpenTelemetry log data model,
//! extended with `raw`, `format_id` and `parse_state`. Nothing here parses anything — the detection
//! pipeline (§6.3, E9) and the format catalogue (E10) fill these in. This is the shape they fill,
//! and the severity tables they resolve through.
//!
//! **Severity is banded, not levelled** (§6.2). Six bands of four let syslog's NOTICE, log4net's
//! extra levels and Zap's DPanic/Panic coexist with Serilog's levels in one sortable cross-format
//! ordering, which is what makes `level >= Warning` mean the same thing in every file.
//!
//! **Absent severity stays absent.** W3C, IIS, nginx and logfmt rows genuinely have no severity, and
//! both the OTel spec and `UI-DESIGN.md` §11.2 require rendering that blank rather than inventing
//! INFO. `Option<Severity>` is that rule in the type system.

use std::sync::Arc;

/// An OTel severity number, 1–24.
///
/// Zero is deliberately not representable. The OTel spec says `SeverityNumber=0` MAY mean
/// "unspecified" and that a source MAY omit the field entirely; carrying both a zero and a `None`
/// would be two spellings of one state, and the one that survives is the one the UI already has a
/// rule for.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Severity(u8);

/// The six bands of four that §6.2 is built on.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SeverityBand {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
    Fatal,
}

impl SeverityBand {
    /// The first severity in the band — what a band *name* resolves to on the right-hand side of a
    /// filter comparison (`SPEC.md` §7.2), so `level >= Warning` is `severity_number >= 13`.
    pub const fn first(self) -> Severity {
        Severity(match self {
            SeverityBand::Trace => 1,
            SeverityBand::Debug => 5,
            SeverityBand::Info => 9,
            SeverityBand::Warn => 13,
            SeverityBand::Error => 17,
            SeverityBand::Fatal => 21,
        })
    }

    pub const fn name(self) -> &'static str {
        match self {
            SeverityBand::Trace => "TRACE",
            SeverityBand::Debug => "DEBUG",
            SeverityBand::Info => "INFO",
            SeverityBand::Warn => "WARN",
            SeverityBand::Error => "ERROR",
            SeverityBand::Fatal => "FATAL",
        }
    }

    /// Resolves a band name for the filter grammar. Case-insensitive, and accepts the long forms a
    /// user is at least as likely to type as the OTel short ones.
    ///
    /// **Every name here must land in the same band that [`Severity::from_level_text`] gives it**,
    /// or `level >= Critical` filters out the very rows whose level reads `Critical`. That is the
    /// one way this function can be wrong, so
    /// `a_name_means_the_same_thing_however_it_is_resolved` asserts it for every name both accept.
    ///
    /// Two names are genuinely ambiguous across frameworks and are resolved against the *normative*
    /// table rather than intuition: **`verbose`** is Serilog's lowest level but Windows Event Log's
    /// `DEBUG (5)`, and **`critical`** is `FATAL (21)` in .NET but `ERROR2 (18)` in Windows Event
    /// Log. Appendix B has a Windows Event Log table and no .NET one, so Windows wins both.
    pub fn parse(name: &str) -> Option<Self> {
        Some(match name.trim().to_ascii_lowercase().as_str() {
            "trace" => SeverityBand::Trace,
            "debug" | "verbose" => SeverityBand::Debug,
            "info" | "information" | "informational" => SeverityBand::Info,
            "warn" | "warning" => SeverityBand::Warn,
            "error" | "err" | "critical" => SeverityBand::Error,
            "fatal" => SeverityBand::Fatal,
            _ => return None,
        })
    }
}

/// `SPEC.md` §6.2's universal error predicate.
pub const ERROR_THRESHOLD: Severity = Severity(17);

impl Severity {
    /// Returns `None` outside 1–24, which includes the spec's "unspecified" zero.
    pub const fn new(number: u8) -> Option<Self> {
        if number >= 1 && number <= 24 {
            Some(Severity(number))
        } else {
            None
        }
    }

    pub const fn get(self) -> u8 {
        self.0
    }

    pub const fn band(self) -> SeverityBand {
        match self.0 {
            1..=4 => SeverityBand::Trace,
            5..=8 => SeverityBand::Debug,
            9..=12 => SeverityBand::Info,
            13..=16 => SeverityBand::Warn,
            17..=20 => SeverityBand::Error,
            _ => SeverityBand::Fatal,
        }
    }

    /// The OTel short name — `WARN`, `WARN2`, `WARN3`, `WARN4`.
    pub fn short_name(self) -> String {
        let band = self.band();
        match self.0 - band.first().0 {
            0 => band.name().to_string(),
            n => format!("{}{}", band.name(), n + 1),
        }
    }

    /// `SPEC.md` §6.2's universal error predicate: ERROR (17) or higher, which the OTel spec states
    /// in the same terms.
    pub const fn is_error(self) -> bool {
        self.0 >= ERROR_THRESHOLD.0
    }
}

impl std::fmt::Display for Severity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.short_name())
    }
}

/// Normative mappings, reproduced from the OpenTelemetry logs data model **Appendix B**
/// ("SeverityNumber example mappings"), which `CLEANROOM.md` §3 permits and whose §5 entry is dated
/// 2026-08-05.
///
/// These are transcribed rather than derived, and three of them are counter-intuitive enough to be
/// worth naming: **syslog Emergency is FATAL (21), not the top of the scale**, with Alert and
/// Critical landing in the *ERROR* band at 19 and 18; **Zap's DPanic and Panic are also ERROR2 and
/// ERROR3**, not FATAL; and **`java.util.logging`'s FINER is DEBUG (5)** while FINE is DEBUG2 (6),
/// which is finer-is-lower inverted from what the names suggest. Anyone "correcting" these to a more
/// intuitive ordering is reintroducing a cross-format inconsistency that §6.2 exists to remove.
impl Severity {
    /// RFC 5424 numeric severity, 0–7.
    pub const fn from_syslog(severity: u8) -> Option<Self> {
        Some(Severity(match severity {
            0 => 21, // Emergency  → FATAL
            1 => 19, // Alert      → ERROR3
            2 => 18, // Critical   → ERROR2
            3 => 17, // Error      → ERROR
            4 => 13, // Warning    → WARN
            5 => 10, // Notice     → INFO2
            6 => 9,  // Informational → INFO
            7 => 5,  // Debug      → DEBUG
            _ => return None,
        }))
    }

    /// log4j / log4net / NLog level names. Case-insensitive per §6.2's Loki-derived rule.
    pub fn from_log4j(level: &str) -> Option<Self> {
        Some(Severity(match level.trim().to_ascii_lowercase().as_str() {
            "trace" => 1,
            "debug" => 5,
            "info" => 9,
            "warn" => 13,
            "error" => 17,
            "fatal" => 21,
            _ => return None,
        }))
    }

    /// Go `zap` level names.
    pub fn from_zap(level: &str) -> Option<Self> {
        Some(Severity(match level.trim().to_ascii_lowercase().as_str() {
            "debug" => 5,
            "info" => 9,
            "warn" => 13,
            "error" => 17,
            "dpanic" => 18,
            "panic" => 19,
            "fatal" => 21,
            _ => return None,
        }))
    }

    /// Windows Event Log level names.
    pub fn from_windows_event_log(level: &str) -> Option<Self> {
        Some(Severity(match level.trim().to_ascii_lowercase().as_str() {
            "verbose" => 5,
            "information" => 9,
            "warning" => 13,
            "error" => 17,
            "critical" => 18,
            _ => return None,
        }))
    }

    /// `java.util.logging` level names.
    pub fn from_java_util_logging(level: &str) -> Option<Self> {
        Some(Severity(match level.trim().to_ascii_lowercase().as_str() {
            "finest" => 1,
            "finer" => 5,
            "fine" => 6,
            "config" => 7,
            "info" => 9,
            "warning" => 13,
            "severe" => 17,
            _ => return None,
        }))
    }
}

/// Tables that are **ours**, not transcriptions. Kept in a separate block so the Appendix B claim
/// above cannot be read as covering them.
impl Severity {
    /// §6.2's HTTP-status aliases.
    ///
    /// **Not an OTel mapping.** Appendix A *does* describe the Apache HTTP Server access log, but it
    /// assigns no `SeverityNumber` and no `SeverityText` to it — so banding a status code is a
    /// Tailhawk convention and is labelled as one.
    ///
    /// It is therefore **opt-in**, and the default for an access-log row is no severity at all.
    /// §6.2 is explicit that W3C, IIS and nginx rows leave severity empty rather than fabricating
    /// one, and this function is the "where wanted" exception, not a default.
    ///
    /// §6.2 enumerates 2xx/3xx/4xx/5xx; 1xx is folded in with them as INFO, since a continuation is
    /// no more an error than a 200 is.
    pub const fn from_http_status(status: u16) -> Option<Self> {
        Some(Severity(match status {
            100..=399 => 9,  // INFO
            400..=499 => 13, // WARN — the client's fault, not an outage
            500..=599 => 17, // ERROR
            _ => return None,
        }))
    }

    /// The generic level-word table, for a format with no table of its own.
    ///
    /// A union of the framework tables above plus the abbreviations real writers emit — Serilog's
    /// three-letter forms, syslog's words. Where a name appears in a normative table, **this table
    /// agrees with it**; `a_name_means_the_same_thing_however_it_is_resolved` holds that down.
    ///
    /// §6.2's rules, all three of which come from Loki's filed bugs: matching is
    /// **case-insensitive**; the table carries **full word forms** (`warning`, `information`,
    /// `critical`) and not just abbreviations; and it is only ever applied to a value that was found
    /// in a *level field*. Scanning a whole line for a level word is a different thing entirely —
    /// opt-out, and always marked low-confidence — and is not this function.
    pub fn from_level_text(level: &str) -> Option<Self> {
        Some(Severity(
            match level
                .trim()
                .trim_matches(|c: char| c == ':' || c == '[' || c == ']')
                .to_ascii_lowercase()
                .as_str()
            {
                "trace" | "trce" | "finest" | "vrb" | "v" => 1,
                "debug" | "dbg" | "dbug" | "finer" | "verbose" | "d" => 5,
                "fine" => 6,
                "config" => 7,
                "info" | "inf" | "information" | "informational" | "i" => 9,
                "notice" => 10,
                "warn" | "warning" | "wrn" | "w" => 13,
                "error" | "err" | "eror" | "fail" | "severe" | "e" => 17,
                "critical" | "crit" | "dpanic" => 18,
                "alert" | "panic" => 19,
                "fatal" | "ftl" | "emerg" | "emergency" | "f" => 21,
                _ => return None,
            },
        ))
    }
}

/// What the detector made of the line (`SPEC.md` §6.1).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum ParseState {
    /// A record the claiming format parsed in full.
    Parsed,
    /// A continuation line belonging to the record above — a stack frame, a wrapped message (§6.4).
    Continuation,
    /// A well-formed record of some *other* format, interleaved into this stream.
    Foreign,
    /// Nothing claimed it. Still a row, still searchable, still copyable.
    #[default]
    Unparsed,
}

/// A timezone-aware instant, without pulling in a date library.
///
/// Nanoseconds since the Unix epoch is what OTel carries. The offset is kept beside it because a log
/// line's *written* offset is information the user can see and sort by, and reducing everything to
/// UTC on ingest would throw it away.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Timestamp {
    pub unix_nanos: i64,
    pub utc_offset_minutes: i16,
}

impl Timestamp {
    pub const fn new(unix_nanos: i64, utc_offset_minutes: i16) -> Self {
        Self {
            unix_nanos,
            utc_offset_minutes,
        }
    }

    pub const fn utc(unix_nanos: i64) -> Self {
        Self::new(unix_nanos, 0)
    }
}

/// An attribute value. Typed, because §7.2's comparison operators must compare a number as a number
/// — `duration_ms > 500` is a different question from `"duration_ms" > "500"`.
#[derive(Clone, Debug, PartialEq)]
pub enum AttributeValue {
    String(String),
    Int(i64),
    Float(f64),
    Bool(bool),
}

/// A record's byte range in its source: absolute offset, and length in bytes.
///
/// Length rather than an end offset because §10.3's very long lines are truncated *for display* and
/// the untruncated length is the thing search and copy need.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct ByteSpan {
    pub offset: u64,
    pub len: u64,
}

/// W3C trace context (`SPEC.md` §9). The fields exist in v1's model even though correlation is v2 —
/// a detector that parses a `trace_id` should have somewhere to put it.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct TraceContext {
    pub trace_id: [u8; 16],
    pub span_id: [u8; 8],
    pub trace_flags: u8,
}

/// Which detector claimed the record.
///
/// `Arc<str>` rather than `&'static str` because §6.5's user-defined formats are not known at
/// compile time, and rather than an index because there is no catalogue to index into yet (E10).
/// Cloning a record clones a pointer.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct FormatId(Arc<str>);

impl FormatId {
    pub fn new(id: impl AsRef<str>) -> Self {
        FormatId(Arc::from(id.as_ref()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Per-**source** constants — host, service, the things that identify where a pane's lines came
/// from.
///
/// Deliberately **not** a field of [`Record`], and §6.1's own field table is what says so — the
/// `resource` row reads "Per-**source** constants (host, service) — belongs to the pane, not the
/// row". §6.2 adds the reason it matters: resource-per-pane against attributes-per-row is what makes
/// the merged view's column problem (§8.3) answerable. Hanging it off every row would also repeat
/// the same host and service string once per line.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Resource {
    pub attributes: Vec<(String, AttributeValue)>,
}

/// The normalised record (`SPEC.md` §6.1).
///
/// **This is a per-viewport materialisation, not per-line storage.** §7.1 computes highlights for
/// visible rows only, and §11.2 budgets no per-line record for a 10 GB file — the index (§5.3) is
/// what exists per line. Anything that builds one of these for every line in a file has
/// misunderstood the memory budget.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Record {
    pub timestamp: Option<Timestamp>,
    /// When Tailhawk read it, which is the only ordering available for a record whose own timestamp
    /// did not parse.
    pub observed_timestamp: Option<Timestamp>,
    pub severity_number: Option<Severity>,
    /// The original string as it appeared — `Warning`, `WRN`, `warn` — never normalised. The
    /// sortable form is `severity_number`; this is what the user wrote and what they expect to see.
    pub severity_text: Option<String>,
    pub body: String,
    pub trace: Option<TraceContext>,
    /// Per-record varying values. These become columns (§6.1).
    pub attributes: Vec<(String, AttributeValue)>,
    pub event_name: Option<String>,
    /// The whole record as it appeared, retained always — this is what keeps the *model* lossless
    /// (§6.1). Serilog message templates, W3C-only fields and anything else with no OTel home
    /// survive here rather than being dropped by a parser that had nowhere to put them.
    ///
    /// **This is decoded text, where §6.1 says "the original bytes".** The deviation is deliberate:
    /// every consumer of a materialised record — grid, search, highlight, export — works in decoded
    /// text, and §5.6's decode is the one lossy step already accepted everywhere else. But it *is* a
    /// loss, and the loss is not theoretical: §5.6 decodes invalid bytes to U+FFFD, and no `String`
    /// can turn a replacement character back into the byte that produced it.
    ///
    /// **[`Record::span`] is what keeps §10.2 and §10.3 satisfiable.** Both specify copy and search
    /// over *original bytes*, so byte-exactness has to live somewhere; it lives in the file, and the
    /// span is the way back to it. `raw` is the convenient form, not the authoritative one.
    pub raw: String,
    /// Where the record's bytes are in the source — what makes §10.2's "copy selection as raw text,
    /// preserving original bytes and encoding" and §10.3's "search and copy operate on the
    /// untruncated bytes" reachable from a materialised record.
    ///
    /// `None` for a record that did not come from a byte range — a merged-view row assembled from
    /// several sources, or a test fixture.
    ///
    /// **Which** source it indexes into is not here yet: there is one open file and no pane model
    /// until M3, and inventing a source id now would be inventing a registry. §8.3's merged view is
    /// what forces that, and it is a v2 concern.
    pub span: Option<ByteSpan>,
    pub format_id: Option<FormatId>,
    pub parse_state: ParseState,
}

impl Record {
    /// The record every unclaimed line becomes. A line nothing parsed is still a row.
    pub fn unparsed(raw: impl Into<String>) -> Self {
        let raw = raw.into();
        Self {
            body: raw.clone(),
            raw,
            parse_state: ParseState::Unparsed,
            ..Self::default()
        }
    }

    /// §6.2's universal error predicate. A record with no severity is **not** an error — absent is
    /// absent, not zero.
    pub fn is_error(&self) -> bool {
        self.severity_number.is_some_and(Severity::is_error)
    }

    pub fn attribute(&self, key: &str) -> Option<&AttributeValue> {
        self.attributes
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bands_are_six_of_four_and_cover_1_to_24() {
        let mut counts = [0usize; 6];
        for n in 1..=24u8 {
            let severity = Severity::new(n).expect("1..=24 is in range");
            counts[severity.band() as usize] += 1;
        }
        assert_eq!(
            counts, [4; 6],
            "every band must hold exactly four severities"
        );
    }

    #[test]
    fn zero_and_out_of_range_are_not_severities() {
        assert_eq!(
            Severity::new(0),
            None,
            "the spec's unspecified zero is None"
        );
        assert_eq!(Severity::new(25), None);
        assert!(Severity::new(1).is_some());
        assert!(Severity::new(24).is_some());
    }

    #[test]
    fn short_names_match_the_otel_table() {
        for (number, name) in [
            (1u8, "TRACE"),
            (2, "TRACE2"),
            (4, "TRACE4"),
            (5, "DEBUG"),
            (9, "INFO"),
            (10, "INFO2"),
            (13, "WARN"),
            (16, "WARN4"),
            (17, "ERROR"),
            (18, "ERROR2"),
            (21, "FATAL"),
            (24, "FATAL4"),
        ] {
            assert_eq!(
                Severity::new(number).expect("in range").short_name(),
                name,
                "severity {number}"
            );
        }
    }

    /// §6.2's universal error predicate, and the part of it that is easy to get wrong: a row with no
    /// severity is not an error.
    #[test]
    fn the_error_predicate_is_17_and_absent_is_not_an_error() {
        for n in 1..=16u8 {
            assert!(!Severity::new(n).expect("in range").is_error(), "{n}");
        }
        for n in 17..=24u8 {
            assert!(Severity::new(n).expect("in range").is_error(), "{n}");
        }
        assert!(
            !Record::unparsed("a line with no level in it").is_error(),
            "absent severity must not read as an error"
        );
    }

    /// The whole point of §6.2: one ordering across formats. A syslog Warning, a log4j WARN and a
    /// Zap Warn must be the same number, or `level >= Warning` means different things per file.
    #[test]
    fn the_same_level_is_the_same_number_in_every_format() {
        let warn = Severity::new(13).expect("in range");
        assert_eq!(Severity::from_syslog(4), Some(warn));
        assert_eq!(Severity::from_log4j("WARN"), Some(warn));
        assert_eq!(Severity::from_zap("warn"), Some(warn));
        assert_eq!(Severity::from_windows_event_log("Warning"), Some(warn));
        assert_eq!(Severity::from_level_text("Warning"), Some(warn));
        assert_eq!(
            SeverityBand::parse("Warning").map(SeverityBand::first),
            Some(warn)
        );
    }

    /// Transcribed from the OTel appendix. The two counter-intuitive rows are the reason this test
    /// spells out every entry rather than spot-checking: syslog Emergency is FATAL (21) and *not*
    /// the top of the scale, and Alert and Critical sit in the ERROR band.
    #[test]
    fn the_syslog_mapping_is_the_normative_one() {
        for (syslog, expected) in [
            (0u8, 21u8),
            (1, 19),
            (2, 18),
            (3, 17),
            (4, 13),
            (5, 10),
            (6, 9),
            (7, 5),
        ] {
            assert_eq!(
                Severity::from_syslog(syslog).map(Severity::get),
                Some(expected),
                "RFC 5424 severity {syslog}"
            );
        }
        assert_eq!(Severity::from_syslog(8), None, "RFC 5424 stops at 7");
    }

    /// Zap's DPanic and Panic are ERROR2 and ERROR3, not FATAL. "Correcting" them breaks the
    /// cross-format ordering.
    #[test]
    fn the_zap_mapping_puts_dpanic_and_panic_in_the_error_band() {
        assert_eq!(Severity::from_zap("dpanic").map(Severity::get), Some(18));
        assert_eq!(Severity::from_zap("panic").map(Severity::get), Some(19));
        assert_eq!(Severity::from_zap("fatal").map(Severity::get), Some(21));
        assert_eq!(
            Severity::from_zap("DPanic").expect("in table").band(),
            SeverityBand::Error
        );
    }

    #[test]
    fn the_log4j_and_windows_mappings_are_the_normative_ones() {
        for (level, expected) in [
            ("TRACE", 1u8),
            ("DEBUG", 5),
            ("INFO", 9),
            ("WARN", 13),
            ("ERROR", 17),
            ("FATAL", 21),
        ] {
            assert_eq!(
                Severity::from_log4j(level).map(Severity::get),
                Some(expected)
            );
        }
        for (level, expected) in [
            ("Verbose", 5u8),
            ("Information", 9),
            ("Warning", 13),
            ("Error", 17),
            ("Critical", 18),
        ] {
            assert_eq!(
                Severity::from_windows_event_log(level).map(Severity::get),
                Some(expected)
            );
        }
    }

    /// **The `level` vocabulary a real Loki deployment actually emits, measured rather than
    /// assumed**, closing one of `LOKI.md` §9's open questions on 2026-08-27.
    ///
    /// These six are every value of the `level` stream label in the owner's estate over a day. All
    /// six already resolve, which is the good outcome — but the reason this is a test and not a
    /// note is that the deployment's Loki image is pinned to a moving tag, so its vocabulary can
    /// change without a commit anywhere. If a seventh word appears and this table has not learned
    /// it, a Loki record's severity silently becomes "unknown" and every severity-banded rule,
    /// colour and filter quietly stops applying to it.
    ///
    /// Note what is *absent*: no `fatal`. The top band this deployment reaches is `critical`, so a
    /// rule written against `fatal` will match nothing here — which is a fact about the estate, not
    /// a defect in the table.
    #[test]
    fn every_level_the_owners_loki_emits_resolves_to_a_severity() {
        for (text, expected) in [
            ("trace", 1u8),
            ("debug", 5),
            ("info", 9),
            ("warn", 13),
            ("error", 17),
            ("critical", 18),
        ] {
            assert_eq!(
                Severity::from_level_text(text).map(Severity::get),
                Some(expected),
                "{text} is a level this deployment emits"
            );
        }
    }

    /// §6.2's Loki-derived rules: matching is case-insensitive and the table carries full word
    /// forms, not just abbreviations. Loki's filed bugs are what these came from.
    #[test]
    fn level_matching_is_case_insensitive_and_knows_the_long_forms() {
        for spelling in ["WARNING", "warning", "Warning", "  Warning  ", "wArNiNg"] {
            assert_eq!(
                Severity::from_level_text(spelling).map(Severity::get),
                Some(13),
                "{spelling:?}"
            );
        }
        for (text, expected) in [
            ("information", 9u8),
            ("informational", 9),
            ("critical", 18),
            ("severe", 17),
            ("notice", 10),
            ("emergency", 21),
        ] {
            assert_eq!(
                Severity::from_level_text(text).map(Severity::get),
                Some(expected),
                "{text}"
            );
        }
    }

    /// **The invariant that catches the whole class.** A name used on the right of a filter must
    /// land in the same band as a row whose level *is* that name, or `level >= Critical` excludes
    /// the rows labelled `Critical` — which is exactly what an earlier draft of this file did, with
    /// `critical` parsing as FATAL (21) while a `Critical` row resolved to 18.
    #[test]
    fn a_name_means_the_same_thing_however_it_is_resolved() {
        for name in [
            "trace",
            "debug",
            "verbose",
            "info",
            "information",
            "informational",
            "warn",
            "warning",
            "error",
            "err",
            "critical",
            "fatal",
        ] {
            let as_band = SeverityBand::parse(name).expect("a band name");
            let as_value = Severity::from_level_text(name).expect("a level word");
            assert_eq!(
                as_band,
                as_value.band(),
                "{name:?} parses as band {as_band:?} but a {name:?} row resolves to {} \
                 ({:?}) — `level >= {name}` would not match its own rows",
                as_value.get(),
                as_value.band()
            );
        }
    }

    /// The framework tables and the generic one must not contradict each other either: a `WARNING`
    /// from `java.util.logging` and a `warning` with no format claimed are the same row to a user.
    #[test]
    fn the_generic_table_agrees_with_every_normative_table() {
        type Table = (&'static str, fn(&str) -> Option<Severity>);
        let cases: [Table; 4] = [
            ("log4j", Severity::from_log4j),
            ("zap", Severity::from_zap),
            ("windows event log", Severity::from_windows_event_log),
            ("java.util.logging", Severity::from_java_util_logging),
        ];
        for name in [
            "trace",
            "finest",
            "finer",
            "fine",
            "config",
            "debug",
            "info",
            "warn",
            "warning",
            "error",
            "severe",
            "fatal",
            "verbose",
            "information",
            "critical",
            "dpanic",
            "panic",
        ] {
            for (table, resolve) in cases {
                let (Some(normative), Some(generic)) =
                    (resolve(name), Severity::from_level_text(name))
                else {
                    continue;
                };
                assert_eq!(
                    normative,
                    generic,
                    "{name:?} is {} in the {table} table but {} in the generic one",
                    normative.get(),
                    generic.get()
                );
            }
        }
    }

    /// Transcribed from Appendix B, and the reason it is spelled out: FINER is DEBUG (5) while FINE
    /// is DEBUG2 (6) — inverted from what the names suggest, and a whole band away from where an
    /// earlier draft of this file put FINER.
    #[test]
    fn the_java_util_logging_mapping_is_the_normative_one() {
        for (level, expected) in [
            ("FINEST", 1u8),
            ("FINER", 5),
            ("FINE", 6),
            ("CONFIG", 7),
            ("INFO", 9),
            ("WARNING", 13),
            ("SEVERE", 17),
        ] {
            assert_eq!(
                Severity::from_java_util_logging(level).map(Severity::get),
                Some(expected),
                "java.util.logging {level}"
            );
        }
    }

    #[test]
    fn an_unknown_level_word_is_none_rather_than_a_guess() {
        for text in ["Starting", "", "  ", "12", "informative", "warnish"] {
            assert_eq!(
                Severity::from_level_text(text),
                None,
                "{text:?} must not resolve to a severity"
            );
        }
    }

    /// The HTTP-status alias is ours rather than OTel's, and §6.2 makes it opt-in. This asserts the
    /// *shape* of that rule: a 4xx is a warning rather than an error, because a client asking for a
    /// missing page is not an outage.
    #[test]
    fn http_status_aliases_are_banded_and_bounded() {
        assert_eq!(Severity::from_http_status(200).map(Severity::get), Some(9));
        assert_eq!(Severity::from_http_status(301).map(Severity::get), Some(9));
        assert_eq!(Severity::from_http_status(404).map(Severity::get), Some(13));
        assert_eq!(Severity::from_http_status(500).map(Severity::get), Some(17));
        assert_eq!(Severity::from_http_status(99), None);
        assert_eq!(Severity::from_http_status(600), None);
        assert!(
            !Severity::from_http_status(404)
                .expect("in range")
                .is_error(),
            "a 404 is not an outage; §6.2 puts it in WARN"
        );
    }

    /// §7.2: a severity *name* on the right-hand side resolves through the banding, so
    /// `level >= Warning` is `severity_number >= 13`.
    #[test]
    fn a_band_name_resolves_to_the_bottom_of_its_band() {
        assert_eq!(SeverityBand::parse("Warning"), Some(SeverityBand::Warn));
        assert_eq!(SeverityBand::parse("warn"), Some(SeverityBand::Warn));
        assert_eq!(SeverityBand::parse("ERROR"), Some(SeverityBand::Error));
        assert_eq!(SeverityBand::parse("nonsense"), None);

        let threshold = SeverityBand::parse("Warning").expect("a band").first();
        assert_eq!(threshold.get(), 13);
        // A log4net WARN and a syslog Notice, either side of the threshold.
        assert!(Severity::from_log4j("WARN").expect("in table") >= threshold);
        assert!(Severity::from_syslog(5).expect("in table") < threshold);
    }

    #[test]
    fn an_unparsed_line_is_still_a_row_and_keeps_its_text() {
        let record = Record::unparsed("a line nothing claimed");
        assert_eq!(record.parse_state, ParseState::Unparsed);
        assert_eq!(record.raw, "a line nothing claimed");
        assert_eq!(record.body, "a line nothing claimed");
        assert_eq!(record.severity_number, None);
        assert_eq!(record.format_id, None);
    }

    #[test]
    fn attributes_keep_their_type_for_the_filter_grammar() {
        let record = Record {
            attributes: vec![
                ("duration_ms".into(), AttributeValue::Int(1500)),
                ("cached".into(), AttributeValue::Bool(false)),
                ("route".into(), AttributeValue::String("/api/v1".into())),
            ],
            ..Record::unparsed("GET /api/v1")
        };
        assert_eq!(
            record.attribute("duration_ms"),
            Some(&AttributeValue::Int(1500)),
            "a number must stay a number, or `duration_ms > 500` compares strings"
        );
        assert_eq!(record.attribute("missing"), None);
    }

    #[test]
    fn a_format_id_is_cheap_to_clone_and_compares_by_value() {
        let a = FormatId::new("serilog-file");
        assert_eq!(a.clone(), a);
        assert_eq!(a.as_str(), "serilog-file");
        assert_ne!(a, FormatId::new("nlog"));
    }

    /// §6.1 puts `resource` in the model; §6.2 puts it on the pane rather than the row. This test is
    /// the reminder — if `Record` ever grows a `resource` field, the merged view's column model
    /// (§8.3) has quietly changed.
    #[test]
    fn resource_is_per_source_and_not_carried_on_the_record() {
        let resource = Resource {
            attributes: vec![("service.name".into(), AttributeValue::String("api".into()))],
        };
        assert_eq!(resource.attributes.len(), 1);
        let record = Record::unparsed("x");
        assert!(
            record.attribute("service.name").is_none(),
            "resource constants must not be duplicated onto every row"
        );
    }
}
