//! Templates from an application's own configuration — `SPEC.md` §6.5, E11.
//!
//! An app that writes `2026-08-16 09:14:02.117 +02:00 [INF] …` says so in its `appsettings.json`
//! (`"outputTemplate": "{Timestamp:yyyy-MM-dd HH:mm:ss.fff zzz} [{Level:u3}] {Message:lj}…"`),
//! its `nlog.config` (`layout="${longdate}|${level:uppercase=true}|${logger}|${message}"`) or its
//! `log4net.config` (`<conversionPattern value="%date [%thread] %-5level %logger - %message%newline"/>`).
//! This module reads those three template languages and compiles each into a [`Format`], so the
//! catalogue's guess is replaced by what the application actually declared.
//!
//! ## What a token becomes
//!
//! Each language is a sequence of literals and tokens; a literal is escaped into the pattern and a
//! token becomes a named capture whose shape the token's own format decides. A timestamp format
//! string (`yyyy-MM-dd HH:mm:ss.fff zzz`) is translated symbol by symbol; a level token declares
//! the level words its format emits (`Level:u3` → `VRB|DBG|INF|WRN|ERR|FTL`); the message is
//! `.*`; `NewLine` and `${newline}` and `%n` end the first line; an exception token is the
//! continuation and contributes nothing to the first line. **A token whose width the template does
//! not fix** — `{SourceContext}`, `${logger}`, `%c` — is `\S+` when a literal follows it and `.*?`
//! when a space does, which is what an ambiguous template can honestly promise.
//!
//! ## Where the templates are found
//!
//! [`scan`] looks in the log's directory and up to three parents for `appsettings*.json`,
//! `nlog.config`, `log4net.config`, `web.config` and `app.config`, and pulls every template it can
//! recognise out of them by shape — no JSON or XML parser, because the three keys have one shape
//! each and a parser would be a dependency for the sake of a regex. Every template found is
//! compiled and handed to detection as a candidate at specificity 0.92: above every catalogue
//! entry but RFC 5424, because a template beside the file is the strongest evidence short of the
//! file describing itself. Detection still scores it — a stale config does not get to lie.

use std::path::{Path, PathBuf};

use regex::Regex;

use crate::format::{custom, Custom, Format, Level, Stamp, LEVEL, MSG, TS};

/// Specificity of a compiled template. See the module note.
pub const SPECIFICITY: f32 = 0.92;

/// The three languages this module reads.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Language {
    Serilog,
    NLog,
    /// log4net's `PatternLayout` — and Logback's pattern, which is the same language with `%msg`,
    /// `%n` and Java's `SSS` for milliseconds.
    Log4net,
    /// §6.5's pattern DSL: `<ts> [<thread>] <level> <logger> - <message>`, `<_>` discards. Not a
    /// regex — a token is a word, a timestamp, a level or the rest of the line, and nothing else.
    Dsl,
}

/// A template found in a config file, before compilation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    pub language: Language,
    pub template: String,
    pub source: PathBuf,
}

/// Compiles one template in `language` into a format named for where it came from.
pub fn compile(
    language: Language,
    template: &str,
    origin: &str,
) -> Result<&'static Format, String> {
    let (pattern, stamp, level, levels, columns, continuation) = match language {
        Language::Serilog => serilog(template)?,
        Language::NLog => nlog(template)?,
        Language::Log4net => log4net(template)?,
        Language::Dsl => dsl(template)?,
    };
    let what = match language {
        Language::Serilog => "Serilog",
        Language::NLog => "NLog",
        Language::Log4net => "log4net",
        Language::Dsl => "pattern",
    };
    custom(Custom {
        id: format!("template:{origin}"),
        name: format!("{what} ({origin})"),
        specificity: SPECIFICITY,
        pattern,
        stamp,
        level,
        levels,
        continuation,
        columns,
    })
}

type Compiled = (
    String,
    Stamp,
    Level,
    Vec<String>,
    Vec<String>,
    Option<String>,
);

const DOTNET_CONTINUATION: &str = r"^\s*(at\s|--->\s|--- End of inner exception|\S+Exception\b)";

/// What a run of spaces or tabs in a template's literal text compiles to. See [`Build::literal`].
const BLANK_RUN: &str = r"[ \t]+";

/// A pattern being assembled: literals escaped, tokens appended, columns remembered.
struct Build {
    pattern: String,
    columns: Vec<String>,
    stamp: Stamp,
    level: Level,
    levels: Vec<String>,
    /// Whether the last thing appended was a token of unfixed width, so the next literal can
    /// decide whether it was `\S+` or `.*?`.
    open: Option<usize>,
    ended: bool,
}

impl Build {
    fn new() -> Self {
        Self {
            pattern: String::from("^"),
            columns: Vec::new(),
            stamp: Stamp::None,
            level: Level::None,
            levels: Vec::new(),
            open: None,
            ended: false,
        }
    }

    /// Appends literal text between two tokens — **with a run of spaces or tabs matching a run of
    /// any length**.
    ///
    /// Layouts pad. `%-5level` writes `INFO ` and `ERROR`, so a template read off an `INFO` line
    /// carries two spaces where an `ERROR` line has one; escaping the run verbatim would build a
    /// format that matches the line it was taken from and fails on every longer level in the file.
    /// The wizard found this the moment it previewed a real log: three lines of five.
    fn literal(&mut self, text: &str) {
        if self.ended || text.is_empty() {
            return;
        }
        // The token before this literal was left open; a literal starting with a space lets it be
        // `.*?`, anything else pins it to `\S+`.
        if let Some(at) = self.open.take() {
            let shape = if text.starts_with(' ') { ".*?" } else { r"\S+" };
            self.pattern.replace_range(at..at, shape);
        }
        let mut rest = text;
        while !rest.is_empty() {
            let blank = rest
                .find(|c: char| c != ' ' && c != '\t')
                .unwrap_or(rest.len());
            if blank > 0 {
                self.pattern.push_str(BLANK_RUN);
                rest = &rest[blank..];
                continue;
            }
            let plain = rest.find([' ', '\t']).unwrap_or(rest.len());
            self.pattern.push_str(&regex::escape(&rest[..plain]));
            rest = &rest[plain..];
        }
    }

    fn capture(&mut self, name: &str, shape: &str) {
        if self.ended {
            return;
        }
        self.settle();
        let name = capture_name(name);
        self.pattern.push_str(&format!("(?P<{name}>{shape})"));
        self.columns.push(name);
    }

    /// A capture whose width the template does not say — decided by what follows it.
    fn open_capture(&mut self, name: &str) {
        if self.ended {
            return;
        }
        self.settle();
        let name = capture_name(name);
        self.pattern.push_str(&format!("(?P<{name}>"));
        self.open = Some(self.pattern.len());
        self.pattern.push(')');
        self.columns.push(name);
    }

    /// Two open captures in a row, or an open one at the end: the earlier is `\S+`.
    fn settle(&mut self) {
        if let Some(at) = self.open.take() {
            self.pattern.replace_range(at..at, r"\S+");
        }
    }

    fn end(&mut self) {
        if self.ended {
            return;
        }
        // An open capture at the end of the line takes the rest of it.
        if let Some(at) = self.open.take() {
            self.pattern.replace_range(at..at, ".*");
        }
        // A literal space before a token that contributed nothing (`${exception}` at the end of
        // an NLog layout) must not demand a trailing space of every line. Since `literal` now
        // compiles a run of blanks to `[ \t]+`, that is the shape to look for as well as a bare
        // space left by anything that appends one directly.
        if let Some(head) = self.pattern.strip_suffix(BLANK_RUN) {
            self.pattern.truncate(head.len());
            self.pattern.push_str(r"\s*");
        } else {
            let trimmed = self.pattern.trim_end_matches(' ').len();
            if trimmed < self.pattern.len() {
                self.pattern.truncate(trimmed);
                self.pattern.push_str(r"\s*");
            }
        }
        self.pattern.push('$');
        self.ended = true;
    }

    fn finish(mut self, continuation: Option<&str>) -> Result<Compiled, String> {
        self.end();
        if !self
            .columns
            .iter()
            .any(|c| c == MSG || c == TS || c == LEVEL)
        {
            return Err("the template names no timestamp, level or message".into());
        }
        Ok((
            self.pattern,
            self.stamp,
            self.level,
            self.levels,
            self.columns,
            continuation.map(str::to_owned),
        ))
    }
}

/// A capture name from a token name: the three the record model understands, and the rest made
/// legal (`SourceContext` stays; `event-properties` becomes `event_properties`).
fn capture_name(name: &str) -> String {
    match name.to_ascii_lowercase().as_str() {
        "timestamp" | "date" | "longdate" | "shortdate" | "time" | "d" => TS.to_owned(),
        "level" | "p" | "loglevel" => LEVEL.to_owned(),
        "message" | "m" | "msg" => MSG.to_owned(),
        _ => {
            let mut out: String = name
                .chars()
                .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
                .collect();
            if out.starts_with(|c: char| c.is_ascii_digit()) {
                out.insert(0, 'f');
            }
            out
        }
    }
}

/// Whether a .NET date format carries a date at all: one with no year and no day is a time of day,
/// valid but yielding no instant — Serilog's console template is the common case.
fn stamp_of(format: &str) -> Stamp {
    if format.contains('y') || format.contains('d') {
        Stamp::Iso
    } else {
        Stamp::TimeOnly
    }
}

/// A .NET custom date/time format string as a regex, symbol by symbol.
fn dotnet_date(format: &str) -> String {
    let mut out = String::new();
    let chars: Vec<char> = format.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let run = chars[i..].iter().take_while(|&&x| x == c).count();
        let piece = match (c, run) {
            ('y', n) if n >= 4 => Some(r"\d{4}"),
            ('y', _) => Some(r"\d{2}"),
            ('M', n) if n >= 4 => Some(r"[A-Z][a-z]+"),
            ('M', 3) => Some(r"[A-Z][a-z]{2}"),
            ('M', _) | ('d', _) | ('H', _) | ('h', _) | ('m', _) | ('s', _) => Some(r"\d{1,2}"),
            ('f', n) | ('F', n) | ('S', n) => {
                out.push_str(&format!(r"\d{{{n}}}"));
                i += run;
                continue;
            }
            ('z', n) if n >= 3 => Some(r"[+-]\d{2}:\d{2}"),
            ('z', _) => Some(r"[+-]\d{1,2}"),
            ('K', _) => Some(r"(?:Z|[+-]\d{2}:\d{2})?"),
            ('t', _) => Some(r"[AP]M?"),
            ('\'', _) => {
                // A quoted literal.
                let close = chars[i + 1..].iter().position(|&x| x == '\'');
                let lit: String = match close {
                    Some(n) => chars[i + 1..i + 1 + n].iter().collect(),
                    None => chars[i + 1..].iter().collect(),
                };
                out.push_str(&regex::escape(&lit));
                i += lit.chars().count() + 2;
                continue;
            }
            _ => None,
        };
        match piece {
            Some(p) => {
                out.push_str(p);
                i += run;
            }
            None => {
                out.push_str(&regex::escape(&c.to_string()));
                i += 1;
            }
        }
    }
    out
}

/// Serilog: `{Name,alignment:format}` tokens; `{{` and `}}` are literal braces.
fn serilog(template: &str) -> Result<Compiled, String> {
    let mut b = Build::new();
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        if rest[open..].starts_with("{{") {
            b.literal(&rest[..open + 1]);
            rest = &rest[open + 2..];
            continue;
        }
        b.literal(&rest[..open]);
        let Some(close) = rest[open..].find('}') else {
            return Err("unclosed { in template".into());
        };
        let token = &rest[open + 1..open + close];
        rest = &rest[open + close + 1..];
        let (name, format) = match token.split_once(':') {
            Some((n, f)) => (n, Some(f)),
            None => (token, None),
        };
        let (name, aligned) = match name.split_once(',') {
            Some((n, _)) => (n, true),
            None => (name, false),
        };
        // Alignment pads the value with spaces on one side; the pattern allows them on both.
        if aligned {
            b.settle();
            b.pattern.push_str(r"\s*");
        }
        match name {
            "Timestamp" => {
                let f = format.unwrap_or("yyyy-MM-dd HH:mm:ss.fff zzz");
                b.stamp = stamp_of(f);
                let shape = dotnet_date(f);
                b.capture(TS, &shape);
            }
            "Level" => {
                b.level = Level::Word;
                let (shape, words): (&str, &[&str]) = match format.unwrap_or("") {
                    "u3" => (
                        "VRB|DBG|INF|WRN|ERR|FTL",
                        &["VRB", "DBG", "INF", "WRN", "ERR", "FTL"],
                    ),
                    "w3" => (
                        "vrb|dbg|inf|wrn|err|ftl",
                        &["vrb", "dbg", "inf", "wrn", "err", "ftl"],
                    ),
                    "u" => (
                        "VERBOSE|DEBUG|INFORMATION|WARNING|ERROR|FATAL",
                        &[
                            "VERBOSE",
                            "DEBUG",
                            "INFORMATION",
                            "WARNING",
                            "ERROR",
                            "FATAL",
                        ],
                    ),
                    "w" => (
                        "verbose|debug|information|warning|error|fatal",
                        &[
                            "verbose",
                            "debug",
                            "information",
                            "warning",
                            "error",
                            "fatal",
                        ],
                    ),
                    _ => (
                        "Verbose|Debug|Information|Warning|Error|Fatal",
                        &[
                            "Verbose",
                            "Debug",
                            "Information",
                            "Warning",
                            "Error",
                            "Fatal",
                        ],
                    ),
                };
                b.levels = words.iter().map(|w| w.to_string()).collect();
                b.capture(LEVEL, shape);
            }
            "Message" => b.capture(MSG, ".*"),
            "NewLine" => b.end(),
            "Exception" | "Properties" => {}
            other => b.open_capture(other),
        }
        if aligned {
            b.pattern.push_str(r"\s*");
        }
    }
    b.literal(rest);
    b.finish(Some(DOTNET_CONTINUATION))
}

/// NLog: `${name:opt=value:...}` renderers.
fn nlog(layout: &str) -> Result<Compiled, String> {
    let mut b = Build::new();
    let mut rest = layout;
    while let Some(open) = rest.find("${") {
        b.literal(&rest[..open]);
        let Some(close) = rest[open..].find('}') else {
            return Err("unclosed ${ in layout".into());
        };
        // NLog escapes a colon inside an option as `\:`; hide it from the split.
        let token = rest[open + 2..open + close].replace("\\:", "\u{0}");
        rest = &rest[open + close + 1..];
        let mut parts = token.split(':');
        let name = parts.next().unwrap_or("").trim();
        let opts: Vec<(&str, &str)> = parts
            .filter_map(|p| p.split_once('='))
            .map(|(k, v)| (k.trim(), v.trim()))
            .collect();
        let unescape = |v: &str| v.replace('\u{0}', ":");
        let opt = |k: &str| opts.iter().find(|(key, _)| *key == k).map(|(_, v)| *v);
        match name {
            "longdate" => {
                b.stamp = Stamp::Iso;
                b.capture(TS, r"\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}\.\d{4}");
            }
            "shortdate" => {
                b.stamp = Stamp::Iso;
                b.capture(TS, r"\d{4}-\d{2}-\d{2}");
            }
            "time" => b.capture("time", r"\d{2}:\d{2}:\d{2}\.\d{4}"),
            "date" => {
                let f = unescape(opt("format").unwrap_or("yyyy-MM-dd HH:mm:ss.ffff"));
                b.stamp = stamp_of(&f);
                let shape = dotnet_date(&f);
                b.capture(TS, &shape);
            }
            "level" => {
                b.level = Level::Word;
                let upper = opt("uppercase").is_some_and(|v| v.eq_ignore_ascii_case("true"));
                let words: &[&str] = if upper {
                    &["TRACE", "DEBUG", "INFO", "WARN", "ERROR", "FATAL"]
                } else {
                    &["Trace", "Debug", "Info", "Warn", "Error", "Fatal"]
                };
                b.levels = words.iter().map(|w| w.to_string()).collect();
                b.capture(LEVEL, &words.join("|"));
            }
            "message" => b.capture(MSG, ".*"),
            "newline" => b.end(),
            "exception" | "onexception" | "when" => {}
            "event-properties" | "event-property" | "event-context" => {
                let item = opt("item").unwrap_or("property");
                b.open_capture(item);
            }
            other => b.open_capture(other),
        }
    }
    b.literal(rest);
    b.finish(Some(DOTNET_CONTINUATION))
}

/// log4net: `%[-][min][.max]name[{format}]` conversion specifiers; `%%` is a percent.
fn log4net(pattern: &str) -> Result<Compiled, String> {
    let spec =
        Regex::new(r"%(-?\d*(?:\.\d+)?)([A-Za-z]+)(?:\{([^}]*)\})?").map_err(|e| e.to_string())?;
    let mut b = Build::new();
    let mut last = 0;
    let text = pattern.replace("%%", "\u{0}");
    for caps in spec.captures_iter(&text) {
        let m = caps.get(0).expect("group 0");
        b.literal(&text[last..m.start()].replace('\u{0}', "%"));
        last = m.end();
        let padded = !caps.get(1).map_or("", |m| m.as_str()).is_empty();
        let name = &caps[2];
        let format = caps.get(3).map(|m| m.as_str());
        if padded {
            b.pattern.push_str(r"\s*");
        }
        match name {
            "date" | "d" | "utcdate" | "u" => {
                b.stamp = match format {
                    Some("ABSOLUTE") => Stamp::TimeOnly,
                    Some("ISO8601") | Some("DATE") | None => Stamp::Iso,
                    Some(f) => stamp_of(f),
                };
                let shape = match format {
                    Some("ISO8601") | None => {
                        r"\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2},\d{3}".to_owned()
                    }
                    Some("ABSOLUTE") => r"\d{2}:\d{2}:\d{2},\d{3}".to_owned(),
                    Some("DATE") => r"\d{2} [A-Z][a-z]{2} \d{4} \d{2}:\d{2}:\d{2},\d{3}".to_owned(),
                    Some(f) => dotnet_date(f),
                };
                b.capture(TS, &shape);
            }
            "level" | "p" => {
                b.level = Level::Word;
                let words = ["TRACE", "DEBUG", "INFO", "WARN", "ERROR", "FATAL"];
                b.levels = words.iter().map(|w| w.to_string()).collect();
                b.capture(LEVEL, "[A-Z]+");
            }
            "message" | "m" => b.capture(MSG, ".*"),
            "newline" | "n" => b.end(),
            "exception" => {}
            "logger" | "c" | "C" | "class" => b.open_capture("logger"),
            "thread" | "t" => b.open_capture("thread"),
            other => b.open_capture(other),
        }
        if padded {
            b.pattern.push_str(r"\s*");
        }
    }
    b.literal(&text[last..].replace('\u{0}', "%"));
    b.finish(Some(DOTNET_CONTINUATION))
}

/// §6.5's DSL: `<name>` tokens between literals. `<ts>` is any timestamp the semantic catalogue
/// would recognise, `<level>` a level word, `<message>` / `<msg>` the rest of the line, `<_>` a
/// discarded word, anything else a named word.
///
/// **`<<` is a literal `<`.** Without it a line with angle brackets in it cannot be described at
/// all, and lines like that are ordinary: a Serilog console template of
/// `[{Timestamp:HH:mm:ss.fff}] <{Instance}> [{Level}] {Message}` writes the tenant in brackets, and
/// every NDC service in the owner's estate emits one. Read out of `appsettings.json` such a
/// template compiles without trouble — there `<` is ordinary text — so the gap was only ever in
/// what a person could type here.
///
/// `>` needs no escape: it is special only *inside* a token, and a token is entered by a `<` that
/// this rule can decline to be.
fn dsl(pattern: &str) -> Result<Compiled, String> {
    let mut b = Build::new();
    let mut rest = pattern;
    while let Some(open) = rest.find('<') {
        b.literal(&rest[..open]);
        if rest[open + 1..].starts_with('<') {
            b.literal("<");
            rest = &rest[open + 2..];
            continue;
        }
        let Some(close) = rest[open..].find('>') else {
            return Err("unclosed < in pattern".into());
        };
        let name = rest[open + 1..open + close].trim();
        rest = &rest[open + close + 1..];
        match name {
            "ts" | "timestamp" | "date" | "time" => {
                b.stamp = Stamp::Iso;
                b.capture(TS, TIMESTAMP_SHAPE);
            }
            "level" | "severity" => {
                b.level = Level::Word;
                b.capture(LEVEL, "[A-Za-z]+");
            }
            "message" | "msg" | "body" => b.capture(MSG, ".*"),
            "_" => {
                b.settle();
                b.pattern.push_str(r"\S+");
            }
            other => b.open_capture(other),
        }
    }
    b.literal(rest);
    b.finish(Some(DOTNET_CONTINUATION))
}

/// The timestamps a `<ts>` token accepts — the shapes `semantic.rs` colours, less the slash dates.
///
/// The zone offset may be preceded by a space: ISO 8601 does not allow one, but Serilog's default
/// file template writes `2026-08-16 09:14:03.884 +02:00`, and a wizard that left the offset outside
/// the token would bake the user's current offset into the format as a mandatory literal.
pub(crate) const TIMESTAMP_SHAPE: &str = r"\d{4}-\d{2}-\d{2}(?:[T ]\d{2}:\d{2}:\d{2}(?:[.,]\d{1,9})?(?: ?(?:Z|[+-]\d{2}:?\d{2}))?)?|\d{2}:\d{2}:\d{2}(?:[.,]\d{1,9})?|[A-Z][a-z]{2} [ 0-3]?\d \d{2}:\d{2}:\d{2}";

/// Finds templates in config files beside `log` and up to three directories above it.
pub fn scan(log: &Path) -> Vec<Found> {
    let mut found = Vec::new();
    let mut dir = log.parent().map(Path::to_path_buf);
    for _ in 0..4 {
        let Some(d) = dir.clone() else { break };
        let Ok(entries) = std::fs::read_dir(&d) else {
            break;
        };
        let mut names: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
        names.sort();
        for path in names {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            let lower = name.to_ascii_lowercase();
            let is_json = lower.starts_with("appsettings") && lower.ends_with(".json");
            let is_xml = matches!(
                lower.as_str(),
                "nlog.config" | "log4net.config" | "web.config" | "app.config"
            );
            if !(is_json || is_xml) {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            if text.len() > 4 * 1024 * 1024 {
                continue;
            }
            found.extend(templates_in(&text, &path));
        }
        dir = d.parent().map(Path::to_path_buf);
    }
    found
}

/// The templates in one config file's text, by the shape of the three keys.
pub fn templates_in(text: &str, source: &Path) -> Vec<Found> {
    let mut out = Vec::new();
    let mut push = |language: Language, template: String| {
        let template = template.trim().to_owned();
        if !template.is_empty() && !out.iter().any(|f: &Found| f.template == template) {
            out.push(Found {
                language,
                template,
                source: source.to_path_buf(),
            });
        }
    };
    let serilog = Regex::new(r#""outputTemplate"\s*:\s*"((?:[^"\\]|\\.)*)""#).expect("regex");
    for caps in serilog.captures_iter(text) {
        push(Language::Serilog, unescape_json(&caps[1]));
    }
    let nlog = Regex::new(r#"layout\s*=\s*"([^"]*\$\{[^"]*)""#).expect("regex");
    for caps in nlog.captures_iter(text) {
        push(Language::NLog, unescape_xml(&caps[1]));
    }
    let log4net = Regex::new(r#"<conversionPattern\s+value\s*=\s*"([^"]*)""#).expect("regex");
    for caps in log4net.captures_iter(text) {
        push(Language::Log4net, unescape_xml(&caps[1]));
    }
    out
}

fn unescape_json(s: &str) -> String {
    s.replace("\\\"", "\"")
        .replace("\\\\", "\\")
        .replace("\\n", "\n")
        .replace("\\t", "\t")
}

fn unescape_xml(s: &str) -> String {
    s.replace("&quot;", "\"")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serilogs_default_file_template_compiles_and_parses_its_own_output() {
        let f = compile(
            Language::Serilog,
            "{Timestamp:yyyy-MM-dd HH:mm:ss.fff zzz} [{Level:u3}] {Message:lj}{NewLine}{Exception}",
            "appsettings.json",
        )
        .expect("compile");
        let r = f
            .parse("2026-08-16 09:14:03.884 +02:00 [ERR] Failed to dispatch job 41982")
            .expect("parse");
        assert!(r.is_error());
        assert_eq!(r.body, "Failed to dispatch job 41982");
        assert!(r.timestamp.is_some());
        assert_eq!(f.columns, ["ts", "level", "msg"]);
        assert!(f.is_continuation("   at Api.Dispatch.Run()"));
    }

    #[test]
    fn a_serilog_template_with_a_source_context_and_alignment() {
        let f = compile(
            Language::Serilog,
            "[{Timestamp:HH:mm:ss} {Level:u3}] {SourceContext,-30} {Message:lj}{NewLine}",
            "x",
        )
        .expect("compile");
        let r = f
            .parse("[09:14:03 WRN] Api.Sql                        slow query took 1240ms")
            .expect("parse");
        assert_eq!(r.body, "slow query took 1240ms");
        assert_eq!(
            r.attributes[0].0, "SourceContext",
            "the property is a column: {:?}",
            r.attributes
        );
        assert_eq!(r.severity_text.as_deref(), Some("WRN"));
    }

    #[test]
    fn nlogs_default_layout_and_a_custom_date_compile() {
        let f = compile(
            Language::NLog,
            "${longdate}|${level:uppercase=true}|${logger}|${message}",
            "nlog.config",
        )
        .expect("compile");
        let r = f
            .parse("2026-08-16 09:14:03.8840|ERROR|Api.Dispatch|Failed to dispatch job 41982")
            .expect("parse");
        assert!(r.is_error());
        assert_eq!(r.body, "Failed to dispatch job 41982");

        let f = compile(
            Language::NLog,
            "${date:format=HH\\:mm\\:ss} ${level} ${message} ${exception:format=tostring}",
            "nlog.config",
        )
        .expect("compile");
        assert!(f.parse("09:14:03 Warn slow query").is_some(), "{f:?}");
    }

    #[test]
    fn log4nets_default_pattern_compiles_and_matches_the_owners_compact_one_too() {
        let f = compile(
            Language::Log4net,
            "%date [%thread] %-5level %logger - %message%newline",
            "log4net.config",
        )
        .expect("compile");
        let r = f
            .parse("2026-08-16 09:14:03,884 [main] ERROR Api.Dispatch - Failed")
            .expect("parse");
        assert!(r.is_error());
        assert_eq!(r.body, "Failed");

        let f = compile(
            Language::Log4net,
            "%date %-5level %logger %message%newline",
            "x",
        )
        .expect("compile");
        let r = f
            .parse("2026-08-07 07:50:45,075 INFO  Compiler.Program Template Set Compiler")
            .expect("parse");
        assert_eq!(r.body, "Template Set Compiler");
    }

    #[test]
    fn templates_are_found_in_config_text_by_shape() {
        let json = r#"{ "Serilog": { "WriteTo": [ { "Name": "File", "Args": { "path": "log.txt",
            "outputTemplate": "{Timestamp:HH:mm:ss} [{Level:u3}] {Message:lj}{NewLine}{Exception}" } } ] } }"#;
        let found = templates_in(json, Path::new("appsettings.json"));
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].language, Language::Serilog);
        assert!(found[0].template.starts_with("{Timestamp:HH:mm:ss}"));

        let xml = r#"<nlog><targets><target name="f" xsi:type="File" fileName="app.log"
            layout="${longdate}|${level:uppercase=true}|${logger}|${message}" /></targets></nlog>"#;
        let found = templates_in(xml, Path::new("nlog.config"));
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].language, Language::NLog);

        let xml = r#"<log4net><appender name="a" type="log4net.Appender.RollingFileAppender">
            <layout type="log4net.Layout.PatternLayout"><conversionPattern value="%date [%thread] %-5level %logger - %message%newline" /></layout>
            </appender></log4net>"#;
        let found = templates_in(xml, Path::new("log4net.config"));
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].language, Language::Log4net);
    }

    #[test]
    fn a_template_naming_nothing_useful_is_refused() {
        assert!(compile(Language::Serilog, "{Properties}", "x").is_err());
    }
    /// `<<` is a literal `<`, which is the only way to describe a line that has one.
    ///
    /// **The case that found this was a real console template.** A Serilog sink writing
    /// `[{Timestamp:HH:mm:ss.fff}] <{Instance}> [{Level}] {Message}` puts the tenant in angle
    /// brackets, and every NDC service in the owner's estate emits it — so a line reads
    /// `[11:19:32.064] <bym2013> [Information]  Request starting`. Read from `appsettings.json`
    /// that compiles without trouble, because a Serilog template's `<` is ordinary text. Typed by
    /// hand as `--column-pattern` it could not be expressed at all: `<` always opened a token and
    /// there was no way to spell one that stood for itself.
    ///
    /// `>` needs no escape. It is only ever special *inside* a token, and a token is entered by a
    /// `<` that this rule can now decline to be.
    #[test]
    fn a_doubled_angle_bracket_is_a_literal_one() {
        let f = compile(
            Language::Dsl,
            "[<ts>] <<<instance>> [<level>]  <message>",
            "x",
        )
        .expect("compile");
        let r = f
            .parse("[11:19:32.064] <bym2013> [Information]  Request starting")
            .expect("a line shaped like the one the pattern describes");
        assert_eq!(r.body, "Request starting");
        let instance = r
            .attributes
            .iter()
            .find(|(k, _)| *k == "instance")
            .map(|(_, v)| format!("{v:?}"))
            .expect("the tenant is captured from between the literal brackets");
        assert!(instance.contains("bym2013"), "{instance}");

        // A pattern that is nothing but an escaped bracket still matches one.
        let f = compile(Language::Dsl, "<<<message>", "x").expect("compile");
        assert_eq!(f.parse("<hello").expect("parse").body, "hello");

        // And the diagnostic for a genuinely unclosed token is unchanged.
        assert!(compile(Language::Dsl, "<ts> <oops", "x").is_err());
    }

    /// §6.5's DSL, and Logback through the log4net compiler.
    #[test]
    fn the_pattern_dsl_and_a_logback_pattern_compile() {
        let f = compile(
            Language::Dsl,
            "<ts> [<thread>] <level> <logger> - <message>",
            "x",
        )
        .expect("compile");
        let r = f
            .parse("2026-08-16 09:14:03,884 [main] ERROR Api.Dispatch - Failed")
            .expect("parse");
        assert!(r.is_error());
        assert_eq!(r.body, "Failed");
        assert_eq!(r.attributes[0].0, "thread");
        let f = compile(Language::Dsl, "<ts> <_> <level>: <message>", "x").expect("compile");
        let r = f.parse("09:14:03 host01 WARN: slow").expect("parse");
        assert_eq!(r.body, "slow");
        assert_eq!(
            f.columns,
            ["ts", "level", "msg"],
            "a discard is not a column"
        );

        let f = compile(
            Language::Log4net,
            "%d{HH:mm:ss.SSS} [%thread] %-5level %logger{36} - %msg%n",
            "logback.xml",
        )
        .expect("compile");
        let r = f
            .parse("09:14:03.884 [main] ERROR c.e.Api.Dispatch - Failed")
            .expect("parse");
        assert!(r.is_error());
        assert_eq!(r.body, "Failed");
    }
}
