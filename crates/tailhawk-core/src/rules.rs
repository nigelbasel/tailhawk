//! User highlight rules from a file — `SPEC.md` §7.1's rule sets, with the file as the editor
//! until V9's editor exists.
//!
//! `tailhawk.rules.toml`, in §12.4's tiers (exe-adjacent, then `%APPDATA%\Tailhawk\`), read and
//! merged like the settings — every rule from every tier, the exe-adjacent tier's first, so a
//! curated set beside the exe outranks a personal one. Each `[[rule]]` names a pattern and what
//! it does:
//!
//! ```toml
//! [[rule]]
//! name = "exceptions"
//! pattern = "\\bException\\b"
//! fg = "#ff7b6b"          # optional; a foreground
//! bg = "#3a1e1e"          # optional; a background
//! whole_line = true       # optional; colour the whole line, not just the match
//! enabled = true          # optional; default true
//! case_insensitive = true # optional; default true
//! literal = false         # optional; default false — true matches the pattern as plain text
//! ```
//!
//! **A rule that does not compile is skipped and named**, never fatal — the settings module's
//! rule that a file must not stop the viewer opening applies here too. Where the rules sit in
//! the highlighter is the shell's decision: above the zero-config catalogue, below the ad-hoc
//! labels.

use std::path::{Path, PathBuf};

use crate::encoding::Charset;
use crate::highlight::{Colour, Rule};
use crate::search::Pattern;

/// The rules file's name.
pub const FILE_NAME: &str = "tailhawk.rules.toml";

/// One rule as written.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Spec {
    pub name: String,
    pub pattern: String,
    pub fg: Option<Colour>,
    pub bg: Option<Colour>,
    pub whole_line: bool,
    pub enabled: bool,
    pub case_insensitive: bool,
    /// `SPEC.md` §7.1: a rule is **plain text or regex**. When set, `pattern` is matched literally
    /// — `UI-DESIGN.md` §5's `Ab` against its `.*`. Independent of `case_insensitive`, which is the
    /// other axis: all four combinations are askable for.
    pub literal: bool,
    /// A `SPEC.md` §7.2 predicate the rule tests against the parsed record instead of a pattern
    /// against the text — `reason in ["probe", "payload"]`, `status = 413`. A rule has a `pattern`
    /// or a `where`, never both; a `where` rule colours the whole line, since there is no match
    /// span inside the text, and a record the predicate cannot judge (no such attribute, or no
    /// record at all) is simply not matched. Every text key of a JSON record is in reach, not
    /// only the ones that are columns — see `Format::parse_every_key`.
    pub where_: Option<String>,
}

/// A named set of rules and what it applies to — `SPEC.md` §7.1's set, "bound to a file glob or
/// a detected format".
///
/// `apply_to` holds bindings, each one of three kinds told apart by its spelling: `format:<name>`
/// is the detected or chosen format's name; anything with a `*` or `?` is a glob, with the format
/// wizard's own semantics (a bare pattern matches the file name, one with a separator the whole
/// path); anything else is a path or a `loki://` URL, matched whole without regard to case. A set
/// with no bindings applies nowhere until chosen by hand.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SetSpec {
    pub name: String,
    pub apply_to: Vec<String>,
    pub rules: Vec<Spec>,
}

/// One rules file: the rules outside any set, which are the Default set and apply everywhere,
/// and the named sets. A file written before sets existed is a `default` and nothing else, and
/// reads exactly as it always did.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct File {
    pub default: Vec<Spec>,
    pub sets: Vec<SetSpec>,
}

impl Spec {
    /// The rule this compiles to, or why it does not.
    pub fn compile(&self) -> Result<Rule, String> {
        // A rule being typed into V9's editor has no name yet, and "…: no pattern" with nothing
        // before the colon reads as a bug. Unnamed rules say only what is wrong.
        let named = |what: &str| {
            if self.name.is_empty() {
                what.to_owned()
            } else {
                format!("{}: {what}", self.name)
            }
        };
        if self.where_.is_some() && !self.pattern.is_empty() {
            return Err(named(
                "both a pattern and a where; a rule matches by one or the other",
            ));
        }
        if self.fg.is_none() && self.bg.is_none() {
            return Err(named("neither fg nor bg"));
        }
        if let Some(text) = self.where_.as_deref() {
            let predicate = crate::filter::parse(text).map_err(|e| named(&e.to_string()))?;
            let mut rule = Rule::where_(self.name.clone(), predicate);
            rule.fg = self.fg;
            rule.bg = self.bg;
            rule.enabled = self.enabled;
            return Ok(rule);
        }
        if self.pattern.is_empty() {
            return Err(named("no pattern"));
        }
        let compiled = if self.literal {
            Pattern::literal(&self.pattern, Charset::UTF_8, self.case_insensitive)
        } else {
            Pattern::compile(&self.pattern, Charset::UTF_8, self.case_insensitive)
        };
        let pattern = compiled.map_err(|e| named(&e.to_string()))?;
        let mut rule = Rule::new(self.name.clone(), pattern);
        rule.fg = self.fg;
        rule.bg = self.bg;
        rule.whole_line = self.whole_line;
        rule.enabled = self.enabled;
        Ok(rule)
    }
}

/// The Default set's rules in `text`, in file order — the reading every file had before sets
/// existed. See [`parse_file`] for the sets.
pub fn parse(text: &str) -> Vec<Spec> {
    parse_file(text).default
}

/// Where the parser is in the file: what the next `key = value` belongs to.
enum Cursor {
    /// Before any table, or after a table that is none of ours.
    Nowhere,
    /// Inside a `[[rule]]`: a rule of the Default set.
    Rule,
    /// Inside a `[[set]]` header: its name and bindings.
    Set,
    /// Inside a `[[set.rule]]`: a rule of the current set.
    SetRule,
}

/// The whole file: the Default set's rules and the named sets, in file order. Lenient: unknown
/// keys are ignored, a malformed line is skipped, a `[[rule]]` with no name is called `rule N`, a
/// `[[set.rule]]` before any `[[set]]` is a rule of the Default set.
pub fn parse_file(text: &str) -> File {
    let mut file = File::default();
    let mut rule: Option<Spec> = None;
    let mut cursor = Cursor::Nowhere;
    fn flush(rule: &mut Option<Spec>, into: &mut Vec<Spec>) {
        if let Some(mut spec) = rule.take() {
            if spec.name.is_empty() {
                spec.name = format!("rule {}", into.len() + 1);
            }
            into.push(spec);
        }
    }
    fn fresh() -> Spec {
        Spec {
            enabled: true,
            case_insensitive: true,
            ..Spec::default()
        }
    }
    // **The mark, before anything else.** `str::trim` does not remove `U+FEFF` — it stopped being
    // White_Space in Unicode 4.0 — so without this the first `[[rule]]` of a file saved by
    // Notepad, by VS Code, or by PowerShell's `Set-Content -Encoding utf8` arrives as
    // `\u{feff}[[rule]]`, matches nothing, and takes the rule under it with it. The set still
    // loads and still applies; it is simply missing its first colour, which is exactly the kind of
    // quiet wrong answer §1.1 forbids.
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    // **An array may run over several lines**, which is how every TOML formatter writes one of
    // more than a few items. The opening line is held until the line with the closing bracket,
    // and the whole is then read as the one value it is.
    let mut open_array: Option<(String, String)> = None;
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((key, mut value)) = open_array.take() {
            value.push(' ');
            value.push_str(strip_comment(line));
            if !line.contains(']') {
                open_array = Some((key, value));
                continue;
            }
            if let (Cursor::Set, Some(set)) = (&cursor, file.sets.last_mut()) {
                if key == "apply_to" {
                    set.apply_to = list(&value);
                }
            }
            continue;
        }
        if line.starts_with('[') {
            match (&cursor, file.sets.last_mut()) {
                (Cursor::SetRule, Some(set)) => flush(&mut rule, &mut set.rules),
                _ => flush(&mut rule, &mut file.default),
            }
            let table = line.trim_start_matches('[').trim_end_matches(']').trim();
            cursor = match (line.starts_with("[["), table) {
                (true, "rule") => {
                    rule = Some(fresh());
                    Cursor::Rule
                }
                (true, "set") => {
                    file.sets.push(SetSpec::default());
                    Cursor::Set
                }
                (true, "set.rule") => {
                    rule = Some(fresh());
                    if file.sets.is_empty() {
                        Cursor::Rule
                    } else {
                        Cursor::SetRule
                    }
                }
                _ => Cursor::Nowhere,
            };
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let (key, value) = (key.trim(), strip_comment(value.trim()));
        if value.starts_with('[') && !value.contains(']') {
            open_array = Some((key.to_owned(), value.to_owned()));
            continue;
        }
        match cursor {
            Cursor::Nowhere => {}
            Cursor::Set => {
                if let Some(set) = file.sets.last_mut() {
                    match key {
                        "name" => set.name = unquote(value),
                        "apply_to" => set.apply_to = list(value),
                        _ => {}
                    }
                }
            }
            Cursor::Rule | Cursor::SetRule => {
                let Some(spec) = rule.as_mut() else {
                    continue;
                };
                match key {
                    "name" => spec.name = unquote(value),
                    "pattern" => spec.pattern = unquote(value),
                    "where" => spec.where_ = Some(unquote(value)),
                    "fg" => spec.fg = colour(&unquote(value)),
                    "bg" => spec.bg = colour(&unquote(value)),
                    "whole_line" => spec.whole_line = value == "true",
                    "enabled" => spec.enabled = value != "false",
                    "case_insensitive" => spec.case_insensitive = value != "false",
                    "literal" => spec.literal = value == "true",
                    _ => {}
                }
            }
        }
    }
    match (&cursor, file.sets.last_mut()) {
        (Cursor::SetRule, Some(set)) => flush(&mut rule, &mut set.rules),
        _ => flush(&mut rule, &mut file.default),
    }
    for (n, set) in file.sets.iter_mut().enumerate() {
        if set.name.is_empty() {
            set.name = format!("set {}", n + 1);
        }
    }
    file
}

/// The strings of a TOML array of strings — `["a", "b"]` — each unquoted. Anything that is not
/// an array is read as one bare value.
fn list(value: &str) -> Vec<String> {
    let Some(inner) = value.strip_prefix('[').and_then(|v| v.strip_suffix(']')) else {
        return vec![unquote(value)];
    };
    let mut out = Vec::new();
    let mut item = String::new();
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for c in inner.chars() {
        match quote {
            Some(q) => {
                item.push(c);
                if escaped {
                    escaped = false;
                } else if c == '\\' && q == '"' {
                    escaped = true;
                } else if c == q {
                    quote = None;
                }
            }
            None => match c {
                '"' | '\'' => {
                    quote = Some(c);
                    item.push(c);
                }
                ',' => {
                    let text = item.trim();
                    if !text.is_empty() {
                        out.push(unquote(text));
                    }
                    item.clear();
                }
                _ => item.push(c),
            },
        }
    }
    let text = item.trim();
    if !text.is_empty() {
        out.push(unquote(text));
    }
    out
}

/// The text for `specs` — what a "save rules" would write, and the template for a first file.
pub fn to_toml(specs: &[Spec]) -> String {
    to_toml_file(&File {
        default: specs.to_vec(),
        sets: Vec::new(),
    })
}

/// The text for a whole file: the Default set's rules as `[[rule]]`, then each set as a `[[set]]`
/// with its `[[set.rule]]`s. Rewritten whole, so a file edited by hand keeps everything it said.
pub fn to_toml_file(file: &File) -> String {
    let mut out = String::from(
        "# Tailhawk highlight rules — SPEC.md §7.1. One [[rule]] per rule, first wins overlaps.\n\
         # pattern is a regex unless literal = true; fg / bg are #rrggbb; whole_line colours the\n\
          # line, not the match. A [[set]] names rules that apply only where its apply_to says:\n\
          # a path, a glob, or format:<name>. A rule's where is a filter expression on the record.\n\
          # Write paths and expressions in single quotes: 'C:\\logs\\*.log', 'status = 413'.\n",
    );
    write_rules(&mut out, &file.default, "rule");
    for set in &file.sets {
        out.push_str("\n[[set]]\n");
        out.push_str(&format!("name = {}\n", quote(&set.name)));
        if !set.apply_to.is_empty() {
            let items: Vec<String> = set.apply_to.iter().map(|b| quote(b)).collect();
            out.push_str(&format!("apply_to = [{}]\n", items.join(", ")));
        }
        write_rules(&mut out, &set.rules, "set.rule");
    }
    out
}

fn write_rules(out: &mut String, specs: &[Spec], table: &str) {
    for spec in specs {
        out.push_str(&format!("\n[[{table}]]\n"));
        out.push_str(&format!("name = {}\n", quote(&spec.name)));
        match spec.where_.as_deref() {
            Some(predicate) => out.push_str(&format!("where = {}\n", quote(predicate))),
            None => out.push_str(&format!("pattern = {}\n", quote(&spec.pattern))),
        }
        if let Some(fg) = spec.fg {
            out.push_str(&format!("fg = \"{}\"\n", hex(fg)));
        }
        if let Some(bg) = spec.bg {
            out.push_str(&format!("bg = \"{}\"\n", hex(bg)));
        }
        if spec.whole_line {
            out.push_str("whole_line = true\n");
        }
        if !spec.enabled {
            out.push_str("enabled = false\n");
        }
        if !spec.case_insensitive {
            out.push_str("case_insensitive = false\n");
        }
        if spec.literal {
            out.push_str("literal = true\n");
        }
    }
}

/// Whether one binding of a set's `apply_to` takes the document at `path` (a file path or a
/// `loki://` URL, `None` for a pipe) with the format named `format` (`None` with no format).
/// See [`SetSpec`] for the three spellings.
pub fn binds(binding: &str, path: Option<&str>, format: Option<&str>) -> bool {
    if let Some(name) = binding.strip_prefix("format:") {
        return format.is_some_and(|f| f.eq_ignore_ascii_case(name.trim()));
    }
    let Some(path) = path else {
        return false;
    };
    // Both ways of writing a Windows separator are the same separator; a `loki://` URL keeps
    // its slashes either way, since a binding written for one is written with the same ones.
    let binding = binding.replace('/', "\\");
    let path = path.replace('/', "\\");
    if binding.contains('*') || binding.contains('?') {
        if binding.contains('\\') {
            return crate::glob::matches(&binding, &path);
        }
        let name = path.rsplit('\\').next().unwrap_or(&path);
        return crate::glob::matches(&binding, name);
    }
    binding.to_lowercase() == path.to_lowercase()
}

/// The rules a document gets, in precedence order: every bound set that takes it, file by file,
/// then every file's Default set — so a curated tier's sets outrank a personal tier's, and any
/// bound set outranks the defaults. Labels are the document's own and sit above all of this.
pub fn for_document<'a>(
    files: &'a [File],
    path: Option<&str>,
    format: Option<&str>,
) -> Vec<&'a Spec> {
    let mut out = Vec::new();
    for file in files {
        for set in &file.sets {
            if set.apply_to.iter().any(|b| binds(b, path, format)) {
                out.extend(set.rules.iter());
            }
        }
    }
    for file in files {
        out.extend(file.default.iter());
    }
    out
}

/// The first bound set that takes the document — what the status bar names as active.
pub fn active_set<'a>(
    files: &'a [File],
    path: Option<&str>,
    format: Option<&str>,
) -> Option<&'a SetSpec> {
    files
        .iter()
        .flat_map(|f| f.sets.iter())
        .find(|set| set.apply_to.iter().any(|b| binds(b, path, format)))
}

/// A first file: two rules to edit, so the format teaches itself.
pub fn template() -> String {
    to_toml(&[
        Spec {
            name: "exception".to_owned(),
            pattern: r"\bException\b|\bTraceback\b".to_owned(),
            fg: Some([1.0, 0.48, 0.42, 1.0]),
            bg: None,
            whole_line: false,
            enabled: true,
            case_insensitive: false,
            literal: false,
            where_: None,
        },
        Spec {
            name: "example: whole line".to_owned(),
            pattern: "TODO-CHANGE-ME".to_owned(),
            fg: None,
            bg: Some([0.23, 0.18, 0.06, 1.0]),
            whole_line: true,
            enabled: false,
            case_insensitive: true,
            literal: true,
            where_: None,
        },
    ])
}

/// The rules file in each tier, exe-adjacent first.
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

/// Every rule from every tier that exists, exe-adjacent first; and the names of the ones that did
/// not compile.
pub fn load(tiers: &[PathBuf]) -> (Vec<Rule>, Vec<String>) {
    let mut rules = Vec::new();
    let mut failed = Vec::new();
    for path in tiers {
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        for spec in parse(&text) {
            match spec.compile() {
                Ok(rule) => rules.push(rule),
                Err(why) => failed.push(why),
            }
        }
    }
    (rules, failed)
}

/// Every tier's file that exists, exe-adjacent first, read whole; and the names of the rules in
/// them that did not compile.
pub fn load_files(tiers: &[PathBuf]) -> (Vec<File>, Vec<String>) {
    let mut files = Vec::new();
    let mut failed = Vec::new();
    for path in tiers {
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        let file = parse_file(&text);
        for spec in file
            .default
            .iter()
            .chain(file.sets.iter().flat_map(|s| s.rules.iter()))
        {
            if let Err(why) = spec.compile() {
                failed.push(why);
            }
        }
        files.push(file);
    }
    (files, failed)
}

/// `#rrggbb` or `#rgb` to a colour; anything else is `None`.
pub fn colour(text: &str) -> Option<Colour> {
    let hex = text.trim().strip_prefix('#')?;
    let channel = |s: &str| u8::from_str_radix(s, 16).ok().map(|v| f32::from(v) / 255.0);
    match hex.len() {
        6 => Some([
            channel(&hex[0..2])?,
            channel(&hex[2..4])?,
            channel(&hex[4..6])?,
            1.0,
        ]),
        3 => {
            let double = |i: usize| {
                let c = &hex[i..i + 1];
                channel(&format!("{c}{c}"))
            };
            Some([double(0)?, double(1)?, double(2)?, 1.0])
        }
        _ => None,
    }
}

/// A colour in the `#rrggbb` form the rules file is written in and the rules editor shows.
///
/// The inverse of [`colour`], and public for the same reason that one is: the shell has to put a
/// colour a user picked back in front of them as text they could have typed themselves.
pub fn hex(c: Colour) -> String {
    let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("#{:02x}{:02x}{:02x}", byte(c[0]), byte(c[1]), byte(c[2]))
}

pub(crate) fn strip_comment(value: &str) -> &str {
    // A `#` outside quotes starts a comment; inside a basic or a literal string it is text.
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for (i, c) in value.char_indices() {
        match (quote, c) {
            (Some('"'), '\\') if !escaped => {
                escaped = true;
                continue;
            }
            (Some(q), c) if c == q && !escaped => quote = None,
            (None, '"' | '\'') => quote = Some(c),
            (None, '#') => return value[..i].trim_end(),
            _ => {}
        }
        escaped = false;
    }
    value
}

pub(crate) fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

pub(crate) fn unquote(value: &str) -> String {
    // **A literal string has no escapes**, which is why it is the way to write a predicate full
    // of double quotes — `where = 'reason in ["probe", "payload"]'` — and a Windows path.
    if let Some(inner) = value.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')) {
        return inner.to_owned();
    }
    let Some(inner) = value.strip_prefix('"').and_then(|v| v.strip_suffix('"')) else {
        return value.to_owned();
    };
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('"') => out.push('"'),
            Some('\\') => out.push('\\'),
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('t') => out.push('\t'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A byte-order mark does not cost the file its first rule.
    ///
    /// **This is a file the user is invited to edit by hand**, and every ordinary way of doing that
    /// on Windows writes a BOM: Notepad, VS Code's default for a new file, and PowerShell's
    /// `Set-Content -Encoding utf8`. `str::trim` does not remove `U+FEFF` — it has not been
    /// White_Space since Unicode 4.0 — so the first `[[rule]]` arrived as `\u{feff}[[rule]]`,
    /// matched nothing, and the rule under it was silently dropped. Silently is the problem: the
    /// set still loads, still applies, and is simply missing its first colour.
    ///
    /// [`crate::ruleset::import`] already tolerated one. This is the same courtesy on the path a
    /// user actually takes.
    #[test]
    fn a_byte_order_mark_does_not_swallow_the_first_rule() {
        let text = template();
        let with_bom = format!("\u{feff}{text}");
        assert_eq!(
            parse(&with_bom),
            parse(&text),
            "a BOM changes nothing about what the file says"
        );

        // And one in front of the very first `[[rule]]`, with no comment header to absorb it.
        let bare = "\u{feff}[[rule]]\nname = \"first\"\npattern = \"ERROR\"\nfg = \"#ff0000\"\n";
        let specs = parse(bare);
        assert_eq!(specs.len(), 1, "the rule survives the mark in front of it");
        assert_eq!(specs[0].name, "first");
    }

    #[test]
    fn rules_round_trip_through_the_file_and_compile() {
        let text = template();
        let specs = parse(&text);
        assert_eq!(specs.len(), 2);
        assert_eq!(specs[0].name, "exception");
        assert_eq!(specs[0].pattern, r"\bException\b|\bTraceback\b");
        assert!(specs[0].fg.is_some() && specs[0].bg.is_none());
        assert!(!specs[0].case_insensitive);
        assert!(specs[1].whole_line && !specs[1].enabled && specs[1].case_insensitive);
        assert_eq!(parse(&to_toml(&specs)), specs, "writer and reader agree");
        let rule = specs[0].compile().expect("compiles");
        assert_eq!(rule.name, "exception");
        assert!(rule.enabled && !rule.whole_line);
        let disabled = specs[1].compile().expect("a disabled rule still compiles");
        assert!(!disabled.enabled && disabled.whole_line);
    }

    #[test]
    fn a_file_by_hand_is_read_leniently() {
        let text = "\
# my rules
[[rule]]
name = \"errors\"   # trailing comment
pattern = \"ERROR|FATAL\"
bg = \"#402020\"
whole_line = true
unknown = 7

[[rule]]
pattern = \"#hash inside\"
fg = \"#f00\"

[[rule]]
name = \"broken\"
pattern = \"(unclosed\"
fg = \"#ffffff\"

[[rule]]
name = \"colourless\"
pattern = \"x\"

[other]
name = \"not a rule\"
";
        let specs = parse(text);
        assert_eq!(specs.len(), 4);
        assert_eq!(specs[0].name, "errors");
        assert_eq!(
            specs[0].bg,
            Some([64.0 / 255.0, 32.0 / 255.0, 32.0 / 255.0, 1.0])
        );
        assert!(specs[0].whole_line);
        assert_eq!(specs[1].name, "rule 2", "unnamed rules are numbered");
        assert_eq!(specs[1].pattern, "#hash inside", "a hash in quotes is text");
        assert_eq!(specs[1].fg, Some([1.0, 0.0, 0.0, 1.0]), "#rgb expands");
        assert!(
            specs[2].compile().is_err(),
            "an unclosed group does not compile"
        );
        assert!(
            specs[3].compile().is_err(),
            "a rule with no colour does nothing"
        );
        assert!(specs[0].compile().is_ok());
    }

    #[test]
    fn tiers_load_and_merge_exe_adjacent_first() {
        let dir = std::env::temp_dir().join("tailhawk_rules_test");
        let _ = std::fs::remove_dir_all(&dir);
        let exe = dir.join("exe");
        let roaming = dir.join("roaming");
        std::fs::create_dir_all(&exe).unwrap();
        std::fs::create_dir_all(roaming.join("Tailhawk")).unwrap();
        std::fs::write(
            exe.join(FILE_NAME),
            "[[rule]]\nname = \"curated\"\npattern = \"a\"\nfg = \"#fff\"\n",
        )
        .unwrap();
        std::fs::write(
            roaming.join("Tailhawk").join(FILE_NAME),
            "[[rule]]\nname = \"mine\"\npattern = \"b\"\nfg = \"#000\"\n[[rule]]\nname = \"bad\"\npattern = \"(\"\nfg = \"#000\"\n",
        )
        .unwrap();
        let tiers = tiers(Some(&exe), Some(&roaming));
        assert_eq!(tiers.len(), 2);
        let (rules, failed) = load(&tiers);
        let names: Vec<_> = rules.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(
            names,
            ["curated", "mine"],
            "exe-adjacent first, then personal"
        );
        assert_eq!(failed.len(), 1);
        assert!(failed[0].starts_with("bad:"), "{failed:?}");
        let (none, _) = load(&[dir.join("missing.toml")]);
        assert!(none.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The owner, 2026-10-28: named sets, each applied to a file or a file type. A set is a
    /// `[[set]]` with `apply_to`; rules outside any set are the Default set, which is what every
    /// file written before sets existed is. A rule's `where` is a §7.2 predicate on the record.
    #[test]
    fn sets_and_their_bindings_round_trip_and_the_flat_file_is_the_default_set() {
        let text = "[[rule]]\nname = \"Error\"\npattern = \"error\"\nbg = \"#ff0000\"\nwhole_line = true\n\n\
                    [[set]]\nname = \"Starberry edge\"\napply_to = [\"loki://starberry*\", \"format:JSON lines\"]\n\n\
                    [[set.rule]]\nname = \"attack\"\nwhere = 'reason in [\"probe\", \"payload\"]'\nbg = \"#7a1010\"\n\n\
                    [[set.rule]]\nname = \"bad request\"\npattern = \"-> 400\"\nfg = \"#400040\"\n\n\
                    [[set]]\nname = \"Drafts\"\n\n[[set.rule]]\nname = \"todo\"\npattern = \"TODO\"\nfg = \"#fff\"\n";
        let file = parse_file(text);
        assert_eq!(file.default.len(), 1);
        assert_eq!(file.default[0].name, "Error");
        assert_eq!(file.sets.len(), 2);
        assert_eq!(file.sets[0].name, "Starberry edge");
        assert_eq!(
            file.sets[0].apply_to,
            ["loki://starberry*", "format:JSON lines"]
        );
        assert_eq!(file.sets[0].rules.len(), 2);
        assert_eq!(
            file.sets[0].rules[0].where_.as_deref(),
            Some("reason in [\"probe\", \"payload\"]")
        );
        assert!(file.sets[0].rules[0].pattern.is_empty());
        assert!(
            file.sets[1].apply_to.is_empty(),
            "a set with no binding applies nowhere until chosen"
        );
        assert_eq!(
            parse(text).len(),
            1,
            "the flat reading is the Default set alone, as it always was"
        );
        let written = to_toml_file(&file);
        assert_eq!(
            parse_file(&written),
            file,
            "written and read back, the same file:\n{written}"
        );
        assert!(
            file.sets[0].rules[0].compile().is_ok(),
            "a where rule compiles"
        );
        let both = Spec {
            pattern: "x".into(),
            where_: Some("a = 1".into()),
            fg: Some([1.0; 4]),
            ..Spec::default()
        };
        assert!(
            both.compile().is_err(),
            "a rule matches by text or by predicate, not both"
        );
    }

    /// What a hand-written file does that the writer never would: a `#` inside a literal string,
    /// an array over several lines, a path with forward slashes.
    #[test]
    fn a_hand_written_file_keeps_its_hashes_its_long_arrays_and_its_slashes() {
        let text = "[[set]]\nname = \"chat\"\napply_to = [\n    'C:/logs/*.log',  # the logs\n    '#archive/*.log',\n    \"format:JSON lines\",\n]\n\
                    [[set.rule]]\nname = \"general\"\nwhere = 'channel = #general'  # the channel\nbg = \"#333333\"\n";
        let file = parse_file(text);
        assert_eq!(file.sets.len(), 1);
        assert_eq!(
            file.sets[0].apply_to,
            ["C:/logs/*.log", "#archive/*.log", "format:JSON lines"],
            "an array over four lines is one array, and a # inside a literal string is text"
        );
        assert_eq!(
            file.sets[0].rules[0].where_.as_deref(),
            Some("channel = #general"),
            "a # inside a literal string is text, not a comment"
        );
        assert!(binds("C:/logs/*.log", Some("C:\\logs\\app.log"), None));
        assert!(binds("C:\\logs\\app.log", Some("C:/logs/app.log"), None));
    }

    #[test]
    fn a_binding_is_a_path_a_glob_or_a_format() {
        assert!(
            binds("C:\\logs\\app.log", Some("c:\\LOGS\\app.log"), None),
            "a path, without regard to case"
        );
        assert!(!binds(
            "C:\\logs\\app.log",
            Some("C:\\logs\\other.log"),
            None
        ));
        assert!(
            binds("*.log", Some("C:\\logs\\app.log"), None),
            "a bare glob is the file name"
        );
        assert!(!binds("*.txt", Some("C:\\logs\\app.log"), None));
        assert!(
            binds("C:\\logs\\*.log", Some("C:\\logs\\app.log"), None),
            "a rooted glob is the whole path"
        );
        assert!(!binds("C:\\other\\*.log", Some("C:\\logs\\app.log"), None));
        assert!(
            binds(
                "loki://starberry*",
                Some("loki://starberry?apps=sb-edge,sb-gateway"),
                None
            ),
            "a remote tail binds by its URL"
        );
        assert!(binds("format:JSON lines", None, Some("JSON lines")));
        assert!(
            binds("format:json LINES", None, Some("JSON lines")),
            "a format's name, without regard to case"
        );
        assert!(!binds(
            "format:JSON lines",
            Some("C:\\x.log"),
            Some("Serilog")
        ));
        assert!(
            !binds("*.log", None, Some("JSON lines")),
            "a pipe has no path to match"
        );
    }

    #[test]
    fn a_document_gets_the_bound_sets_then_the_defaults_curated_first() {
        let curated = parse_file(
            "[[rule]]\nname = \"c-default\"\npattern = \"a\"\nfg = \"#fff\"\n\
             [[set]]\nname = \"c-json\"\napply_to = [\"format:JSON lines\"]\n\
             [[set.rule]]\nname = \"c-json-rule\"\npattern = \"b\"\nfg = \"#fff\"\n",
        );
        let personal = parse_file(
            "[[rule]]\nname = \"p-default\"\npattern = \"a\"\nfg = \"#fff\"\n\
             [[set]]\nname = \"p-logs\"\napply_to = [\"*.log\"]\n\
             [[set.rule]]\nname = \"p-logs-rule\"\npattern = \"b\"\nfg = \"#fff\"\n\
             [[set]]\nname = \"p-other\"\napply_to = [\"*.txt\"]\n\
             [[set.rule]]\nname = \"p-other-rule\"\npattern = \"b\"\nfg = \"#fff\"\n",
        );
        let files = [curated, personal];
        let names = |specs: Vec<&Spec>| specs.iter().map(|s| s.name.clone()).collect::<Vec<_>>();
        assert_eq!(
            names(for_document(&files, Some("C:\\a.log"), Some("JSON lines"))),
            ["c-json-rule", "p-logs-rule", "c-default", "p-default"]
        );
        assert_eq!(
            names(for_document(&files, Some("C:\\a.csv"), Some("Serilog"))),
            ["c-default", "p-default"],
            "nothing bound: the defaults alone"
        );
        assert_eq!(
            active_set(&files, Some("C:\\a.log"), Some("JSON lines")).map(|s| s.name.as_str()),
            Some("c-json")
        );
        assert!(active_set(&files, Some("C:\\a.csv"), None).is_none());
    }

    #[test]
    fn colours_parse_and_print() {
        assert_eq!(colour("#ff8000"), Some([1.0, 128.0 / 255.0, 0.0, 1.0]));
        assert_eq!(colour("#F80"), Some([1.0, 136.0 / 255.0, 0.0, 1.0]));
        assert_eq!(colour("ff8000"), None);
        assert_eq!(colour("#12"), None);
        assert_eq!(colour("#gg0000"), None);
        assert_eq!(hex([1.0, 0.5, 0.0, 1.0]), "#ff8000");
    }
}
