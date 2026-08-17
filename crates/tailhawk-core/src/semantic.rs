//! The zero-config semantic highlight catalogue — `SPEC.md` §7.1, E23.
//!
//! §7.1: a "**zero-config semantic layer beneath user rules**: timestamps, durations, numbers,
//! IPv4/IPv6, GUIDs, URLs, Windows and UNIX paths, HTTP methods and status codes, `key=value`
//! pairs, quoted strings, hex/pointer addresses, severity keywords. klogg's fatal UX flaw is an
//! empty highlighter set on first run." `UI-DESIGN.md` §5: "on by default … Tailhawk is useful
//! before you configure anything."
//!
//! This is a **catalogue, not an engine**: a [`RuleSet`] for `highlight.rs`, built from regexes
//! written against the public shape of each thing — RFC 3339 for timestamps, RFC 4122 for GUIDs,
//! RFC 3986 for URL schemes, RFC 4291 for IPv6 text, RFC 9110 for methods and status classes.
//! Nothing here knows about frames, budgets or precedence; it only chooses what to look for, what
//! colour it is, and **in what order** — and the order is the one design decision in the file.
//!
//! ## Order is precedence, and the catalogue is ordered from the outside in
//!
//! `highlight.rs` gives the characters to the first rule that claims them. So a thing that
//! *contains* other things comes before what it contains: a **timestamp** before the numbers and
//! colon-separated hex inside it, a **URL** before the host, path and `key=value` query inside it, a
//! **GUID** before the hex it is made of, a **path** before the words and numbers along it. Numbers
//! come last, because a number is what is left when nothing more specific wanted the digits.
//! Severity words come first of all, because they are the thing a reader scans a log for and no
//! other rule should be able to take `ERROR` from them.
//!
//! ## Restraint
//!
//! `UI-DESIGN.md` §11.2: "the palette is deliberately restrained so that **user highlight colours
//! are the loudest thing on screen**." Every colour here is a tint of [`INK`](crate::INK) rather
//! than a saturated block, nothing gets a background, and the two things on every line — the
//! timestamp and the numbers — are the quietest. The severity ramp follows §11.2's table: FATAL
//! magenta-red, ERROR red-orange, WARN amber, INFO the foreground (so no rule), DEBUG muted, TRACE
//! more muted. All provisional with the rest of the palette; §11.2 pins no hex.
//!
//! ## What is deliberately loose
//!
//! These are highlights, not parsers, and a false positive costs a colour while a false negative
//! costs the feature. So the IPv6 rule accepts a few things that are not addresses, the duration
//! rule will colour `5 m` in prose, and the number rule colours every integer that nothing more
//! specific claimed. What is *not* accepted is the case that would be actively misleading: a bare
//! three-digit number is not a status code unless something says it is, and a lone `'` in `don't`
//! does not open a quoted string.

use crate::encoding::Charset;
use crate::highlight::{Colour, Rule, RuleSet};
use crate::record::SeverityBand;
use crate::search::Pattern;

/// The set's name, as `RuleSet::name` and as a user will see it in a rule list.
pub const NAME: &str = "Semantic";

// §11.2's severity ramp. INFO is the foreground and has no rule.
pub const FATAL: Colour = [0.96, 0.44, 0.64, 1.0];
pub const ERROR: Colour = [0.96, 0.47, 0.38, 1.0];
pub const WARN: Colour = [0.93, 0.73, 0.32, 1.0];
pub const DEBUG: Colour = [0.56, 0.59, 0.64, 1.0];
pub const TRACE: Colour = [0.44, 0.47, 0.52, 1.0];

/// On every line, so the quietest thing here: a cool grey a shade under the ink.
pub const TIMESTAMP: Colour = [0.52, 0.64, 0.78, 1.0];
pub const URL: Colour = [0.47, 0.70, 0.96, 1.0];
pub const IP: Colour = [0.46, 0.82, 0.80, 1.0];
pub const PATH: Colour = [0.76, 0.71, 0.94, 1.0];
pub const HTTP_METHOD: Colour = [0.72, 0.85, 0.62, 1.0];
/// 2xx. 3xx takes [`URL`]'s blue, 4xx [`WARN`]'s amber, 5xx [`ERROR`]'s red — the same meaning,
/// the same colour, so a reader learns one ramp rather than two.
pub const HTTP_OK: Colour = [0.56, 0.83, 0.56, 1.0];
pub const HEX: Colour = [0.86, 0.66, 0.86, 1.0];
pub const QUOTED: Colour = [0.86, 0.77, 0.58, 1.0];
pub const DURATION: Colour = [0.60, 0.83, 0.63, 1.0];
/// The key of a `key=value`, not the value: keys recede, values carry the information.
pub const KEY: Colour = [0.63, 0.71, 0.79, 1.0];
pub const NUMBER: Colour = [0.61, 0.79, 0.94, 1.0];

/// The words each severity band is spelled with in the wild, **in the case they must appear in.**
///
/// The long, unambiguous words are matched in any case — `error` in prose is still worth a
/// reader's eye, and `Error`, `ERROR` and `error` are all levels somewhere. The short and the
/// ambiguous ones (`ERR`, `WRN`, `FINE`, `CONFIG`, `VRB` …) are matched **as written here**, because
/// in lower case they are ordinary English and would paint half of every sentence.
///
/// **Every word here must land in the band `Severity::from_level_text` gives it** — that table in
/// `record.rs` is the authority on which words are levels — and the tests assert it, so adding a
/// word that `record.rs` does not know is a test failure rather than a colour that lies.
const SEVERITY_WORDS: [(SeverityBand, &[&str], &[&str]); 5] = [
    (
        SeverityBand::Fatal,
        &["fatal", "emergency", "emerg"],
        &["FTL"],
    ),
    (
        SeverityBand::Error,
        &["error", "severe", "critical", "alert", "panic"],
        &["ERR", "EROR", "CRIT", "DPANIC"],
    ),
    (SeverityBand::Warn, &["warn", "warning"], &["WRN"]),
    (
        SeverityBand::Debug,
        &["debug", "verbose"],
        &["DBG", "FINE", "FINER", "CONFIG"],
    ),
    (SeverityBand::Trace, &["trace"], &["FINEST", "VRB"]),
];

const fn band_colour(band: SeverityBand) -> Colour {
    match band {
        SeverityBand::Fatal => FATAL,
        SeverityBand::Error => ERROR,
        SeverityBand::Warn => WARN,
        SeverityBand::Debug => DEBUG,
        SeverityBand::Trace => TRACE,
        SeverityBand::Info => crate::INK,
    }
}

const MONTHS: &str = "Jan|Feb|Mar|Apr|May|Jun|Jul|Aug|Sep|Oct|Nov|Dec";
const HEX4: &str = "[0-9a-fA-F]{1,4}";
const OCTET: &str = r"(?:25[0-5]|2[0-4]\d|1\d\d|[1-9]?\d)";
/// A character that can end a Windows path component: not a separator, not reserved, not blank.
const WIN: &str = r#"[^\\/:*?"<>|\s]"#;

/// The catalogue, in precedence order. See the module note for why the order is what it is.
///
/// Compiles ~20 patterns, which is milliseconds; the shell does it once per document, on the
/// worker that opens the file. Every pattern is on the linear engine — a test asserts that no rule
/// falls back to §7.4's backtracker, because a catalogue that runs on every visible row of every
/// frame cannot afford one that might.
pub fn catalogue() -> RuleSet {
    let mut set = RuleSet::new(NAME);
    for (band, any_case, as_written) in SEVERITY_WORDS {
        set = set.with(
            rule(
                &format!("severity {}", band.name()),
                &severity_pattern(any_case, as_written),
            )
            .fg(band_colour(band)),
        );
    }
    set.with(rule("timestamp", &timestamp_pattern()).fg(TIMESTAMP))
        .with(
            rule(
                "identifier",
                r"\b[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}\b|\b[0-9a-fA-F]{32}\b",
            )
            .derived(),
        )
        .with(
            rule(
                "url",
                r#"\b[a-zA-Z][a-zA-Z0-9+.\-]*://[^\s"'<>()\[\]{}]*[^\s"'<>()\[\]{}.,;:!?]"#,
            )
            .fg(URL),
        )
        .with(rule("ipv6", &ipv6_pattern()).group(1, IP))
        .with(rule("ipv4", &format!(r"\b{OCTET}(?:\.{OCTET}){{3}}(?::\d{{1,5}})?\b")).fg(IP))
        .with(rule("windows path", &format!(r"\b[A-Za-z]:\\(?:{WIN}+\\)*{WIN}*")).fg(PATH))
        .with(rule("unc path", &format!(r"\\\\{WIN}+(?:\\{WIN}+)*")).fg(PATH))
        .with(rule("unix path", r#"(?:^|[\s"'=(\[,])(/(?:[\w.\-~+@%]+/?)+)"#).group(1, PATH))
        .with(
            rule(
                "http status",
                r#"(?:HTTP/\d(?:\.\d)?"? +|\b[Ss]tatus(?:[ _-]?[Cc]ode)?[=:]? *|(?:^| )- )(?:(2\d\d)|(3\d\d)|(4\d\d)|(5\d\d))\b"#,
            )
            .group(1, HTTP_OK)
            .group(2, URL)
            .group(3, WARN)
            .group(4, ERROR),
        )
        .with(
            rule(
                "http method",
                r"\b(?:GET|POST|PUT|PATCH|DELETE|HEAD|OPTIONS|CONNECT|TRACE)\b",
            )
            .fg(HTTP_METHOD),
        )
        .with(rule("hex", r"\b0[xX][0-9a-fA-F]+\b").fg(HEX))
        .with(rule("quoted", r#""(?:[^"\\\r\n]|\\.)*""#).fg(QUOTED))
        .with(rule("quoted", r"(?:^|[^\w])('(?:[^'\\\r\n]|\\.)*')").group(1, QUOTED))
        .with(
            rule(
                "duration",
                r"\b\d+(?:[.,]\d+)?\s?(?:ns|µs|us|ms|s|secs?|seconds?|m|mins?|minutes?|h|hrs?|hours?|d|days?)\b",
            )
            .fg(DURATION),
        )
        .with(rule("key", r"\b([A-Za-z_][\w.\-]*)=").group(1, KEY))
        .with(rule("number", r"\b\d+(?:\.\d+)?\b").fg(NUMBER))
}

/// The catalogue is a set of constants and a test compiles every one; a failure here is an edit
/// to a constant that skipped the tests, not a runtime condition.
fn rule(name: &str, source: &str) -> Rule {
    let pattern = Pattern::compile(source, Charset::UTF_8, false)
        .unwrap_or_else(|e| panic!("semantic rule {name:?} does not compile: {e}"));
    Rule::new(name, pattern)
}

fn severity_pattern(any_case: &[&str], as_written: &[&str]) -> String {
    format!(
        r"\b(?:(?i:{})|{})\b",
        any_case.join("|"),
        as_written.join("|")
    )
}

/// RFC 3339 / ISO 8601, a bare time, syslog's `Aug 16 09:14:02`, CLF's `16/Aug/2026:09:14:02
/// +0200`, and the slash dates IIS and .NET's defaults write. Date-with-time is one alternative so
/// the whole thing is one span rather than a date beside a time.
fn timestamp_pattern() -> String {
    format!(
        concat!(
            r"\b\d{{4}}-\d{{2}}-\d{{2}}(?:[T ]\d{{2}}:\d{{2}}:\d{{2}}(?:[.,]\d{{1,9}})?(?:Z|[+-]\d{{2}}:?\d{{2}})?)?\b",
            r"|\b\d{{2}}:\d{{2}}:\d{{2}}(?:[.,]\d{{1,9}})?\b",
            r"|\b(?:{m}) [ 0-3]?\d \d{{2}}:\d{{2}}:\d{{2}}\b",
            r"|\b\d{{1,2}}/(?:{m})/\d{{4}}:\d{{2}}:\d{{2}}:\d{{2}}(?: [+-]\d{{4}})?",
            r"|\b\d{{1,2}}/\d{{1,2}}/\d{{4}}(?: \d{{1,2}}:\d{{2}}:\d{{2}}(?: ?[AP]M)?)?\b",
        ),
        m = MONTHS
    )
}

/// RFC 4291 §2.2: the full eight groups, the `::`-compressed forms, and the IPv4-mapped form.
///
/// The address is group 1 and the rule itself is colourless, because the leading-`::` form has to
/// be anchored on what precedes it — `Foo::bar` is a scope operator, not a link-local address —
/// and the linear engine has no lookbehind, so the preceding character is matched and not claimed.
fn ipv6_pattern() -> String {
    format!(
        concat!(
            r"(?:^|[^\w:])(",
            r"::[fF]{{4}}:{o}(?:\.{o}){{3}}\b",
            r"|(?:{h}:){{7}}{h}\b",
            r"|(?:{h}:){{1,6}}:(?:{h}(?::{h}){{0,5}}\b)?",
            r"|::{h}(?::{h}){{0,6}}\b",
            r")"
        ),
        h = HEX4,
        o = OCTET
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::highlight::{derived_colour, Highlighter, Span, IDENTIFIER_PALETTE};
    use crate::record::Severity;

    /// The coloured runs of one line, as text, in order — what a reader would see.
    fn coloured(line: &str) -> Vec<(&str, Colour)> {
        let h = Highlighter::new(catalogue());
        h.begin_frame();
        let mut out: Vec<Span> = Vec::new();
        h.line(line, &mut out);
        out.iter()
            .map(|s| {
                (
                    &line[s.start..s.end],
                    s.fg.expect("every semantic span has ink"),
                )
            })
            .collect()
    }

    /// Whether `part` is a run of exactly `colour`. For the negatives: a thing that must not wear a
    /// colour, whatever else the catalogue makes of it.
    fn has(line: &str, part: &str, colour: Colour) -> bool {
        coloured(line).contains(&(part, colour))
    }

    /// The colour of the run that starts at `part`, or `None` if it is plain.
    fn colour_of(line: &str, part: &str) -> Option<Colour> {
        let at = line.find(part).expect("the part is in the line");
        coloured(line)
            .into_iter()
            .find(|(run, _)| {
                let start = run.as_ptr() as usize - line.as_ptr() as usize;
                start == at
            })
            .map(|(run, colour)| {
                assert_eq!(
                    run, part,
                    "the run starting at {part:?} is a different length"
                );
                colour
            })
    }

    #[test]
    fn every_rule_compiles_on_the_linear_engine() {
        let set = catalogue();
        assert!(set.rules.len() >= 20);
        for rule in &set.rules {
            assert!(
                !rule.pattern.backtracking(),
                "{:?} fell back to the backtracker: {}",
                rule.name,
                rule.pattern.source()
            );
        }
    }

    /// The catalogue's words and `record.rs`'s severity table cannot disagree.
    #[test]
    fn every_severity_word_lands_in_the_band_record_rs_gives_it() {
        for (band, any_case, as_written) in SEVERITY_WORDS {
            for word in any_case.iter().chain(as_written) {
                let severity = Severity::from_level_text(word)
                    .unwrap_or_else(|| panic!("{word:?} is not a level record.rs knows"));
                assert_eq!(severity.band(), band, "{word:?}");
                assert_eq!(
                    colour_of(&format!("x {word} y"), word),
                    Some(band_colour(band))
                );
            }
        }
    }

    #[test]
    fn a_level_word_is_coloured_in_any_case_but_a_short_one_only_as_written() {
        assert_eq!(colour_of("[Error] boom", "Error"), Some(ERROR));
        assert_eq!(colour_of("an error occurred", "error"), Some(ERROR));
        assert_eq!(colour_of("WRN Api", "WRN"), Some(WARN));
        assert!(!has("it went fine", "fine", DEBUG));
        assert!(!has("loading config", "config", DEBUG));
        assert!(
            !has("ErrorCode=5", "Error", ERROR),
            "part of a word is not a level"
        );
        assert!(coloured("INFO ok").is_empty(), "INFO is the foreground");
    }

    #[test]
    fn timestamps_are_one_span_and_come_before_the_numbers_inside_them() {
        for stamp in [
            "2026-08-16 09:14:02.117",
            "2026-08-16T09:14:02Z",
            "2026-08-16T09:14:02.117+02:00",
            "2026-08-16",
            "09:14:02",
            "09:14:02,117",
            "Aug 16 09:14:02",
            "Aug  6 09:14:02",
            "16/Aug/2026:09:14:02 +0200",
            "08/16/2026 09:14:02",
            "16/08/2026",
        ] {
            let line = format!("{stamp} INFO x");
            assert_eq!(coloured(&line), [(stamp, TIMESTAMP)], "{stamp}");
        }
    }

    #[test]
    fn identifiers_take_a_derived_colour_and_the_same_one_every_time() {
        let guid = "3f2504e0-4f89-11d3-9a0c-0305e82c3301";
        let trace = "4bf92f3577b34da6a3ce929d0e0e4736";
        let line = format!("req {guid} corr {trace} again {guid}");
        let runs = coloured(&line);
        assert_eq!(runs.len(), 3);
        assert_eq!(runs[0], (guid, derived_colour(guid)));
        assert_eq!(runs[1], (trace, derived_colour(trace)));
        assert_eq!(runs[2].1, runs[0].1);
        assert!(IDENTIFIER_PALETTE.contains(&runs[0].1));
    }

    #[test]
    fn a_url_is_one_span_that_stops_before_the_sentence_does() {
        assert_eq!(
            coloured("see https://example.com/a/b?x=1&y=2, then."),
            [("https://example.com/a/b?x=1&y=2", URL)]
        );
        assert_eq!(
            colour_of("(file:///C:/x/y.log)", "file:///C:/x/y.log"),
            Some(URL)
        );
    }

    #[test]
    fn ip_addresses_in_both_families_and_not_the_things_that_look_like_them() {
        assert_eq!(
            colour_of("from 10.0.0.1:8080 to", "10.0.0.1:8080"),
            Some(IP)
        );
        assert_eq!(
            colour_of("host 192.168.1.254 up", "192.168.1.254"),
            Some(IP)
        );
        assert!(
            !has("v1.2.3.4 build", "1.2.3.4", IP),
            "a version is not an address"
        );
        assert!(!has("bad 256.1.1.1 x", "256.1.1.1", IP));
        for addr in [
            "2001:0db8:85a3:0000:0000:8a2e:0370:7334",
            "2001:db8::8a2e:370:7334",
            "fe80::1",
            "::1",
            "::ffff:192.0.2.128",
        ] {
            assert_eq!(
                colour_of(&format!("addr {addr} end"), addr),
                Some(IP),
                "{addr}"
            );
            assert_eq!(
                colour_of(&format!("addr=[{addr}]"), addr),
                Some(IP),
                "{addr}"
            );
        }
        assert!(
            coloured("Foo::bar()").is_empty(),
            "a scope operator is not an address"
        );
        assert_eq!(colour_of("at 09:14:02 x", "09:14:02"), Some(TIMESTAMP));
    }

    #[test]
    fn paths_of_all_three_kinds() {
        assert_eq!(
            colour_of(
                r"read C:\logs\app\app-2026.log ok",
                r"C:\logs\app\app-2026.log"
            ),
            Some(PATH)
        );
        assert_eq!(
            colour_of(r"share \\srv01\logs\x.log now", r"\\srv01\logs\x.log"),
            Some(PATH)
        );
        assert_eq!(
            colour_of("open /var/log/app.log failed", "/var/log/app.log"),
            Some(PATH)
        );
        assert_eq!(colour_of("GET /api/users/12", "/api/users/12"), Some(PATH));
        assert!(
            !has("ratio 1/2 done", "/2", PATH),
            "a fraction is not a path"
        );
        assert!(
            !has("on 08/16/2026", "/16/2026", PATH),
            "a date is not a path"
        );
    }

    #[test]
    fn a_status_code_needs_context_and_takes_the_colour_of_its_class() {
        assert_eq!(
            colour_of(r#""GET /x HTTP/1.1" 200 512"#, "200"),
            Some(HTTP_OK)
        );
        assert_eq!(colour_of("status=301", "301"), Some(URL));
        assert_eq!(colour_of("StatusCode: 404", "404"), Some(WARN));
        assert_eq!(colour_of("responded - 503 - in", "503"), Some(ERROR));
        assert_eq!(
            colour_of("status=301", "status"),
            Some(KEY),
            "the key is still a key"
        );
        assert_eq!(
            colour_of("returned 412 rows", "412"),
            Some(NUMBER),
            "a bare number is a number"
        );
    }

    #[test]
    fn methods_hex_quotes_durations_keys_and_numbers() {
        assert_eq!(colour_of("POST /x", "POST"), Some(HTTP_METHOD));
        assert_eq!(colour_of("at 0x7FFE1234 in", "0x7FFE1234"), Some(HEX));
        assert_eq!(
            colour_of(r#"name "Api \"x\" Disp" ok"#, r#""Api \"x\" Disp""#),
            Some(QUOTED)
        );
        assert_eq!(colour_of("item 'foo bar' ok", "'foo bar'"), Some(QUOTED));
        assert!(coloured("don't know if it's").is_empty(), "apostrophes");
        assert_eq!(colour_of("took 88ms and", "88ms"), Some(DURATION));
        assert_eq!(colour_of("took 1.5 s and", "1.5 s"), Some(DURATION));
        assert_eq!(
            colour_of("timeout after 30000ms", "30000ms"),
            Some(DURATION)
        );
        assert_eq!(colour_of("elapsed=12", "elapsed"), Some(KEY));
        assert_eq!(colour_of("elapsed=12", "12"), Some(NUMBER));
        assert_eq!(colour_of("job 41982 done", "41982"), Some(NUMBER));
        assert_eq!(colour_of("v 3.14 x", "3.14"), Some(NUMBER));
    }

    /// One real-looking line, end to end, in the order a reader sees it.
    #[test]
    fn a_whole_line_reads_as_a_log_line_should() {
        let line = r#"2026-08-16 09:14:02.117 ERROR Api.Dispatch job=41982 "timeout after 30000ms" from 10.0.0.1 status=503 corr 4bf92f3577b34da6a3ce929d0e0e4736"#;
        let runs = coloured(line);
        let words: Vec<&str> = runs.iter().map(|(t, _)| *t).collect();
        assert_eq!(
            words,
            [
                "2026-08-16 09:14:02.117",
                "ERROR",
                "job",
                "41982",
                "\"timeout after 30000ms\"",
                "10.0.0.1",
                "status",
                "503",
                "4bf92f3577b34da6a3ce929d0e0e4736",
            ]
        );
        assert_eq!(runs[0].1, TIMESTAMP);
        assert_eq!(runs[1].1, ERROR);
        assert_eq!(
            runs[4].1, QUOTED,
            "a quoted string is one colour throughout"
        );
        assert_eq!(runs[7].1, ERROR, "5xx wears the error colour");
    }

    /// The whole catalogue over a screenful of ordinary lines, in the time a frame has. Not a
    /// benchmark — the bound is loose enough to pass on a loaded machine in a debug build — but a
    /// tripwire against a rule that quietly turns quadratic.
    #[test]
    fn a_screenful_costs_a_fraction_of_a_frame() {
        let h = Highlighter::new(catalogue());
        let line = r#"2026-08-16 09:14:02.117 INFO  Api.Controller line 41982 returned 412 rows in 88ms for /api/users?x=1 from 10.0.0.1 status=200 id=3f2504e0-4f89-11d3-9a0c-0305e82c3301"#;
        let mut out = Vec::new();
        h.begin_frame();
        h.line(line, &mut out);
        let started = std::time::Instant::now();
        for _ in 0..100 {
            h.line(line, &mut out);
        }
        let per_row = started.elapsed() / 100;
        eprintln!(
            "semantic catalogue: {per_row:?} per row, {} rules",
            h.set().rules.len()
        );
        assert!(
            per_row < std::time::Duration::from_micros(500),
            "{per_row:?} per row is too slow for a screenful"
        );
    }
}
