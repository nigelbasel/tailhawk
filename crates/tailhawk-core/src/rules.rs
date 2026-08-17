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
}

impl Spec {
    /// The rule this compiles to, or why it does not.
    pub fn compile(&self) -> Result<Rule, String> {
        if self.pattern.is_empty() {
            return Err(format!("{}: no pattern", self.name));
        }
        if self.fg.is_none() && self.bg.is_none() {
            return Err(format!("{}: neither fg nor bg", self.name));
        }
        let pattern = Pattern::compile(&self.pattern, Charset::UTF_8, self.case_insensitive)
            .map_err(|e| format!("{}: {e}", self.name))?;
        let mut rule = Rule::new(self.name.clone(), pattern);
        rule.fg = self.fg;
        rule.bg = self.bg;
        rule.whole_line = self.whole_line;
        rule.enabled = self.enabled;
        Ok(rule)
    }
}

/// The rules in `text`, in file order. Lenient: unknown keys are ignored, a malformed line is
/// skipped, a `[[rule]]` with no name is called `rule N`.
pub fn parse(text: &str) -> Vec<Spec> {
    let mut out: Vec<Spec> = Vec::new();
    let mut current: Option<Spec> = None;
    let flush = |current: &mut Option<Spec>, out: &mut Vec<Spec>| {
        if let Some(mut spec) = current.take() {
            if spec.name.is_empty() {
                spec.name = format!("rule {}", out.len() + 1);
            }
            out.push(spec);
        }
    };
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with("[[") {
            flush(&mut current, &mut out);
            if line.trim_start_matches('[').trim_end_matches(']').trim() == "rule" {
                current = Some(Spec {
                    enabled: true,
                    case_insensitive: true,
                    ..Spec::default()
                });
            }
            continue;
        }
        if line.starts_with('[') {
            flush(&mut current, &mut out);
            continue;
        }
        let Some(spec) = current.as_mut() else {
            continue;
        };
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let (key, value) = (key.trim(), strip_comment(value.trim()));
        match key {
            "name" => spec.name = unquote(value),
            "pattern" => spec.pattern = unquote(value),
            "fg" => spec.fg = colour(&unquote(value)),
            "bg" => spec.bg = colour(&unquote(value)),
            "whole_line" => spec.whole_line = value == "true",
            "enabled" => spec.enabled = value != "false",
            "case_insensitive" => spec.case_insensitive = value != "false",
            _ => {}
        }
    }
    flush(&mut current, &mut out);
    out
}

/// The text for `specs` — what a "save rules" would write, and the template for a first file.
pub fn to_toml(specs: &[Spec]) -> String {
    let mut out = String::from(
        "# Tailhawk highlight rules — SPEC.md §7.1. One [[rule]] per rule, first wins overlaps.\n\
         # pattern is a regex; fg / bg are #rrggbb; whole_line colours the line, not the match.\n",
    );
    for spec in specs {
        out.push_str("\n[[rule]]\n");
        out.push_str(&format!("name = {}\n", quote(&spec.name)));
        out.push_str(&format!("pattern = {}\n", quote(&spec.pattern)));
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
    }
    out
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
        },
        Spec {
            name: "example: whole line".to_owned(),
            pattern: "TODO-CHANGE-ME".to_owned(),
            fg: None,
            bg: Some([0.23, 0.18, 0.06, 1.0]),
            whole_line: true,
            enabled: false,
            case_insensitive: true,
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

fn hex(c: Colour) -> String {
    let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("#{:02x}{:02x}{:02x}", byte(c[0]), byte(c[1]), byte(c[2]))
}

fn strip_comment(value: &str) -> &str {
    // A `#` outside quotes starts a comment; inside a basic string it is text.
    let mut in_quotes = false;
    let mut escaped = false;
    for (i, c) in value.char_indices() {
        match c {
            '\\' if in_quotes && !escaped => {
                escaped = true;
                continue;
            }
            '"' if !escaped => in_quotes = !in_quotes,
            '#' if !in_quotes => return value[..i].trim_end(),
            _ => {}
        }
        escaped = false;
    }
    value
}

fn quote(s: &str) -> String {
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

fn unquote(value: &str) -> String {
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
        assert_eq!(specs[0].bg, Some([64.0 / 255.0, 32.0 / 255.0, 32.0 / 255.0, 1.0]));
        assert!(specs[0].whole_line);
        assert_eq!(specs[1].name, "rule 2", "unnamed rules are numbered");
        assert_eq!(specs[1].pattern, "#hash inside", "a hash in quotes is text");
        assert_eq!(specs[1].fg, Some([1.0, 0.0, 0.0, 1.0]), "#rgb expands");
        assert!(specs[2].compile().is_err(), "an unclosed group does not compile");
        assert!(specs[3].compile().is_err(), "a rule with no colour does nothing");
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
        assert_eq!(names, ["curated", "mine"], "exe-adjacent first, then personal");
        assert_eq!(failed.len(), 1);
        assert!(failed[0].starts_with("bad:"), "{failed:?}");
        let (none, _) = load(&[dir.join("missing.toml")]);
        assert!(none.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
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
