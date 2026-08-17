//! Persisted state — `SPEC.md` §12.4, E28, brought forward: window placement and per-file state.
//!
//! §12.4: "**One resolution scheme for all persisted state.** Read order is always: exe-adjacent →
//! `%APPDATA%\Tailhawk\` → built-in defaults, **merged**, with the earlier tier winning per key.
//! Write target is the first writable tier, tested by an actual write probe at startup." And
//! "`--stateless` … suppresses all *writes*, and leaves *reads* untouched."
//!
//! ## The file, and why it is read by hand
//!
//! `tailhawk.settings.toml`. There is no TOML crate in the tree and this needs one table, one
//! array of tables, and four value kinds — a reader for that subset is shorter than the argument
//! for a dependency, and it is tested round-trip against its own writer. **It reads leniently**: an
//! unknown key is ignored, a malformed line is skipped, and a file that is not TOML at all yields
//! the defaults. A settings file must never be the reason the viewer does not open.
//!
//! ## What is kept
//!
//! - `[appearance]` — `theme`: `dark`, `light` or `system`, when chosen.
//! - `[window]` — `x`, `y`, `width`, `height`, `maximized`: where the window was.
//! - `[[file]]` — `path`, `chips` (each `+text` or `-text`), `collapse`, `bookmarks` (file rows),
//!   `labels` (each `n:text`): what a file was being
//!   looked at through, so opening it again shows the same view. Keyed by **path**; §12.4 says
//!   file identity, which survives a rename where a path does not, and that upgrade is recorded
//!   rather than done.

use std::path::{Path, PathBuf};

/// The settings file's name, per §12.4.
pub const FILE_NAME: &str = "tailhawk.settings.toml";

/// Where the window was.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Window {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub maximized: bool,
}

/// How one file was being looked at.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FileState {
    pub path: String,
    /// Chip texts with their polarity as the first character: `+error`, `-retry`.
    pub chips: Vec<String>,
    pub collapse: bool,
    /// E20's bookmarks, as physical (zero-based) file rows.
    pub bookmarks: Vec<u64>,
    /// The colour labels, each `n:text` — the digit key and the literal it marks.
    pub labels: Vec<String>,
    /// Column widths in cells after the user resized them; empty means the measured widths.
    pub columns: Vec<u64>,
}

/// Everything persisted.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Settings {
    pub window: Option<Window>,
    pub files: Vec<FileState>,
    /// V13: `dark`, `light` or `system`, when the user chose one.
    pub theme: Option<String>,
}

impl Settings {
    /// The state for `path`, if any was kept.
    pub fn file(&self, path: &str) -> Option<&FileState> {
        self.files
            .iter()
            .find(|f| f.path.eq_ignore_ascii_case(path))
    }

    /// Records the state for `path`, replacing what was there. A file with nothing to say — no
    /// chips, no collapse — is forgotten rather than kept as an empty entry.
    pub fn set_file(&mut self, state: FileState) {
        self.files
            .retain(|f| !f.path.eq_ignore_ascii_case(&state.path));
        if !state.chips.is_empty()
            || state.collapse
            || !state.bookmarks.is_empty()
            || !state.labels.is_empty()
            || !state.columns.is_empty()
        {
            self.files.push(state);
        }
        // Keep the newest 200: a settings file is not a history.
        if self.files.len() > 200 {
            let drop = self.files.len() - 200;
            self.files.drain(..drop);
        }
    }

    /// Merges `over` onto `self`: `over`'s keys win, per §12.4's "earlier tier winning per key",
    /// with the earlier tier passed as `over`.
    pub fn merged_under(mut self, over: Settings) -> Settings {
        if over.window.is_some() {
            self.window = over.window;
        }
        if over.theme.is_some() {
            self.theme = over.theme;
        }
        for f in over.files {
            self.set_file(f);
        }
        self
    }

    /// The file's text.
    pub fn to_toml(&self) -> String {
        let mut out =
            String::from("# Tailhawk settings — SPEC.md §12.4. Rewritten whole; edits survive.\n");
        if let Some(theme) = &self.theme {
            out.push_str(&format!("\n[appearance]\ntheme = {}\n", quote(theme)));
        }
        if let Some(w) = &self.window {
            out.push_str("\n[window]\n");
            out.push_str(&format!(
                "x = {}\ny = {}\nwidth = {}\nheight = {}\nmaximized = {}\n",
                w.x, w.y, w.width, w.height, w.maximized
            ));
        }
        for f in &self.files {
            out.push_str("\n[[file]]\n");
            out.push_str(&format!("path = {}\n", quote(&f.path)));
            if !f.chips.is_empty() {
                let chips: Vec<String> = f.chips.iter().map(|c| quote(c)).collect();
                out.push_str(&format!("chips = [{}]\n", chips.join(", ")));
            }
            if f.collapse {
                out.push_str("collapse = true\n");
            }
            if !f.bookmarks.is_empty() {
                let rows: Vec<String> = f.bookmarks.iter().map(u64::to_string).collect();
                out.push_str(&format!("bookmarks = [{}]\n", rows.join(", ")));
            }
            if !f.labels.is_empty() {
                let labels: Vec<String> = f.labels.iter().map(|l| quote(l)).collect();
                out.push_str(&format!("labels = [{}]\n", labels.join(", ")));
            }
            if !f.columns.is_empty() {
                let cols: Vec<String> = f.columns.iter().map(u64::to_string).collect();
                out.push_str(&format!("columns = [{}]\n", cols.join(", ")));
            }
        }
        out
    }

    /// Reads the subset this program writes. Lenient: see the module note.
    pub fn from_toml(text: &str) -> Settings {
        let mut settings = Settings::default();
        let mut section = Section::None;
        let mut window = Window::default();
        let mut have_window = false;
        let mut file: Option<FileState> = None;
        let flush_file = |file: &mut Option<FileState>, settings: &mut Settings| {
            if let Some(f) = file.take() {
                if !f.path.is_empty() {
                    settings.set_file(f);
                }
            }
        };
        for raw in text.lines() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some(header) = line.strip_prefix("[[").and_then(|l| l.strip_suffix("]]")) {
                flush_file(&mut file, &mut settings);
                section = if header.trim() == "file" {
                    file = Some(FileState::default());
                    Section::File
                } else {
                    Section::Other
                };
                continue;
            }
            if let Some(header) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
                flush_file(&mut file, &mut settings);
                section = match header.trim() {
                    "window" => {
                        have_window = true;
                        Section::Window
                    }
                    "appearance" => Section::Appearance,
                    _ => Section::Other,
                };
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let (key, value) = (key.trim(), value.trim());
            match section {
                Section::Appearance => {
                    if key == "theme" {
                        settings.theme = Some(unquote(value));
                    }
                }
                Section::Window => match key {
                    "x" => window.x = value.parse().unwrap_or(0),
                    "y" => window.y = value.parse().unwrap_or(0),
                    "width" => window.width = value.parse().unwrap_or(0),
                    "height" => window.height = value.parse().unwrap_or(0),
                    "maximized" => window.maximized = value == "true",
                    _ => {}
                },
                Section::File => {
                    if let Some(f) = file.as_mut() {
                        match key {
                            "path" => f.path = unquote(value),
                            "chips" => f.chips = array(value),
                            "collapse" => f.collapse = value == "true",
                            "bookmarks" => {
                                f.bookmarks = array(value)
                                    .iter()
                                    .filter_map(|v| v.parse().ok())
                                    .collect()
                            }
                            "labels" => f.labels = array(value),
                            "columns" => {
                                f.columns = array(value)
                                    .iter()
                                    .filter_map(|v| v.parse().ok())
                                    .collect()
                            }
                            _ => {}
                        }
                    }
                }
                Section::None | Section::Other => {}
            }
        }
        flush_file(&mut file, &mut settings);
        if have_window && window.width > 0 && window.height > 0 {
            settings.window = Some(window);
        }
        settings
    }
}

#[derive(Copy, Clone, PartialEq, Eq)]
enum Section {
    None,
    Window,
    Appearance,
    File,
    Other,
}

/// A TOML basic string.
fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04X}", c as u32)),
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
            Some('t') => out.push('\t'),
            Some('u') => {
                let hex: String = chars.by_ref().take(4).collect();
                if let Some(ch) = u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                    out.push(ch);
                }
            }
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

/// A `["a", "b"]` array of strings.
fn array(value: &str) -> Vec<String> {
    let Some(inner) = value.strip_prefix('[').and_then(|v| v.strip_suffix(']')) else {
        return Vec::new();
    };
    // Split on commas outside quotes.
    let mut items = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut escaped = false;
    for c in inner.chars() {
        match c {
            '\\' if in_quotes && !escaped => {
                escaped = true;
                current.push(c);
                continue;
            }
            '"' if !escaped => in_quotes = !in_quotes,
            ',' if !in_quotes => {
                let item = current.trim();
                if !item.is_empty() {
                    items.push(unquote(item));
                }
                current.clear();
                escaped = false;
                continue;
            }
            _ => {}
        }
        escaped = false;
        current.push(c);
    }
    let item = current.trim();
    if !item.is_empty() {
        items.push(unquote(item));
    }
    items
}

/// §12.4's tiers: exe-adjacent, then `%APPDATA%\Tailhawk\`. `roaming` is the caller's answer for
/// `%APPDATA%` (the shell asks Windows; a test passes a directory).
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

/// Reads and merges every tier that exists — the earlier tier winning per key.
pub fn load(tiers: &[PathBuf]) -> Settings {
    let mut merged = Settings::default();
    // Later tiers first, so an earlier one merges *over* them.
    for path in tiers.iter().rev() {
        if let Ok(text) = std::fs::read_to_string(path) {
            merged = merged.merged_under(Settings::from_toml(&text));
        }
    }
    merged
}

/// Writes to the first tier that takes a write — §12.4's probe is the write itself, atomic
/// replace-on-write. `None` if no tier was writable, which is what the "settings will not be saved"
/// chip is for. `stateless` writes nothing and says so by returning `None` too.
pub fn save(tiers: &[PathBuf], settings: &Settings, stateless: bool) -> Option<PathBuf> {
    if stateless {
        return None;
    }
    let text = settings.to_toml();
    for path in tiers {
        let Some(dir) = path.parent() else {
            continue;
        };
        if std::fs::create_dir_all(dir).is_err() {
            continue;
        }
        let tmp = path.with_extension("toml.tmp");
        if std::fs::write(&tmp, &text).is_err() {
            continue;
        }
        if std::fs::rename(&tmp, path).is_ok() {
            return Some(path.clone());
        }
        let _ = std::fs::remove_file(&tmp);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Settings {
        let mut s = Settings {
            window: Some(Window {
                x: 100,
                y: 80,
                width: 1200,
                height: 800,
                maximized: false,
            }),
            files: Vec::new(),
            theme: Some("light".to_owned()),
        };
        s.set_file(FileState {
            path: r"C:\logs\app.log".to_owned(),
            chips: vec!["+error".to_owned(), "-retry \"quoted\"".to_owned()],
            collapse: true,
            bookmarks: vec![0, 42, 1_000_000],
            labels: vec!["1:Exception".to_owned(), "9:a \"quoted\" one".to_owned()],
            columns: vec![19, 5, 0],
        });
        s.set_file(FileState {
            path: r"C:\logs\other.log".to_owned(),
            chips: vec!["+job".to_owned()],
            collapse: false,
            bookmarks: Vec::new(),
            labels: Vec::new(),
            columns: Vec::new(),
        });
        s
    }

    #[test]
    fn settings_round_trip_through_the_toml_subset() {
        let s = sample();
        let text = s.to_toml();
        assert!(text.contains("[window]"), "{text}");
        assert!(text.contains(r#"path = "C:\\logs\\app.log""#), "{text}");
        assert!(
            text.contains(r#"chips = ["+error", "-retry \"quoted\""]"#),
            "{text}"
        );
        assert_eq!(Settings::from_toml(&text), s);
    }

    #[test]
    fn a_file_with_nothing_to_say_is_forgotten_and_a_replaced_one_replaced() {
        let mut s = sample();
        s.set_file(FileState {
            path: r"C:\logs\app.log".to_owned(),
            chips: Vec::new(),
            collapse: false,
            bookmarks: Vec::new(),
            labels: Vec::new(),
            columns: Vec::new(),
        });
        assert!(s.file(r"C:\logs\app.log").is_none());
        s.set_file(FileState {
            path: r"c:\LOGS\other.log".to_owned(),
            chips: vec!["+x".to_owned()],
            collapse: false,
            bookmarks: Vec::new(),
            labels: Vec::new(),
            columns: Vec::new(),
        });
        assert_eq!(s.files.len(), 1, "case-insensitive path replaces");
        assert_eq!(s.file(r"C:\logs\other.log").unwrap().chips, ["+x"]);
    }

    #[test]
    fn a_hostile_or_foreign_file_yields_defaults_rather_than_an_error() {
        assert_eq!(
            Settings::from_toml("this is not toml\n=\n[[[\n"),
            Settings::default()
        );
        let s = Settings::from_toml("[window]\nx = 10\ny = 10\nwidth = notanumber\nheight = 5\n");
        assert_eq!(s.window, None, "a window with no width is no window");
        let s = Settings::from_toml("[future]\nthing = 1\n[window]\nx=1\ny=2\nwidth=3\nheight=4\n");
        assert_eq!(
            s.window.map(|w| w.width),
            Some(3),
            "unknown tables are skipped"
        );
    }

    #[test]
    fn tiers_merge_with_the_earlier_winning_and_the_first_writable_takes_the_write() {
        let root = std::env::temp_dir().join("tailhawk-settings-test");
        let _ = std::fs::remove_dir_all(&root);
        let exe = root.join("exe");
        let roaming = root.join("roaming");
        std::fs::create_dir_all(&exe).expect("dirs");
        std::fs::create_dir_all(&roaming).expect("dirs");
        let tiers = tiers(Some(&exe), Some(&roaming));
        assert_eq!(tiers.len(), 2);
        assert!(tiers[1].ends_with(Path::new("Tailhawk").join(FILE_NAME)));

        // Only roaming has a file: it is what loads.
        let mut personal = Settings::default();
        personal.set_file(FileState {
            path: "a.log".into(),
            chips: vec!["+a".into()],
            collapse: false,
            bookmarks: Vec::new(),
            labels: Vec::new(),
            columns: Vec::new(),
        });
        std::fs::create_dir_all(tiers[1].parent().unwrap()).unwrap();
        std::fs::write(&tiers[1], personal.to_toml()).unwrap();
        assert_eq!(load(&tiers).file("a.log").unwrap().chips, ["+a"]);

        // Exe-adjacent appears and disagrees: it wins per key, and roaming's other keys survive.
        let mut curated = Settings::default();
        curated.set_file(FileState {
            path: "a.log".into(),
            chips: vec!["+curated".into()],
            collapse: true,
            bookmarks: Vec::new(),
            labels: Vec::new(),
            columns: Vec::new(),
        });
        curated.set_file(FileState {
            path: "b.log".into(),
            chips: vec!["+b".into()],
            collapse: false,
            bookmarks: Vec::new(),
            labels: Vec::new(),
            columns: Vec::new(),
        });
        std::fs::write(&tiers[0], curated.to_toml()).unwrap();
        let merged = load(&tiers);
        assert_eq!(merged.file("a.log").unwrap().chips, ["+curated"]);
        assert_eq!(merged.file("b.log").unwrap().chips, ["+b"]);

        // The write goes to the first writable tier — the exe-adjacent one here.
        let wrote = save(&tiers, &merged, false).expect("a tier is writable");
        assert_eq!(wrote, tiers[0]);
        assert!(
            save(&tiers, &merged, true).is_none(),
            "stateless writes nothing"
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
