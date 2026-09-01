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
//!   `labels` (each `n:text`), `columns` (widths in cells), `filters_hidden`: what a file was being
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
    /// §2.1's filter panel, when the user closed it on a file that has filters.
    ///
    /// **Stored the negative way round on purpose.** A settings file written before this field
    /// existed says nothing about the panel, and so does every file whose panel was left open —
    /// both read back as `false`, and `false` has to mean *shown*, because a remembered file
    /// opens with its filters already in force. The other way round, every such file would come
    /// back filtered with nothing on screen saying so.
    pub filters_hidden: bool,
}

/// Everything persisted.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Settings {
    pub window: Option<Window>,
    pub files: Vec<FileState>,
    /// V13: `dark`, `light` or `system`, when the user chose one.
    pub theme: Option<String>,
    /// §2.2 Preferences: the grid font family, when the user chose one.
    pub font: Option<String>,
    /// §2.2 Preferences: the grid em size at the 96-DPI baseline, when the user chose one.
    pub font_size: Option<u16>,
    /// §2.3: whether the toolbar row is shown. Absent means shown — the default is the row.
    pub toolbar: Option<bool>,
    /// File ▸ Open Recent, newest first, at most [`RECENT_MAX`].
    pub recent: Vec<String>,
    /// The Find dialog's query history, newest first, at most [`FIND_MAX`].
    pub find_queries: Vec<String>,
}

/// How many files Open Recent keeps — the platform's customary MRU depth.
pub const RECENT_MAX: usize = 10;
/// How many queries the Find dialog's history holds.
pub const FIND_MAX: usize = 10;

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

    /// Puts `query` at the front of the Find history, moving it there if it is already present —
    /// compared exactly, because case matters in a query — and dropping the oldest past
    /// [`FIND_MAX`].
    pub fn remember_query(&mut self, query: &str) {
        self.find_queries.retain(|q| q != query);
        self.find_queries.insert(0, query.to_owned());
        self.find_queries.truncate(FIND_MAX);
    }

    /// Puts `path` at the front of the recent list, moving it there if it is already present —
    /// compared case-insensitively, as Windows paths are — and dropping the oldest past
    /// [`RECENT_MAX`].
    pub fn remember_recent(&mut self, path: &str) {
        let folded = path.to_lowercase();
        self.recent.retain(|p| p.to_lowercase() != folded);
        self.recent.insert(0, path.to_owned());
        self.recent.truncate(RECENT_MAX);
    }

    /// Merges `over` onto `self`: `over`'s keys win, per §12.4's "earlier tier winning per key",
    /// with the earlier tier passed as `over`.
    pub fn merged_under(mut self, over: Settings) -> Settings {
        if over.window.is_some() {
            self.window = over.window;
        }
        if over.toolbar.is_some() {
            self.toolbar = over.toolbar;
        }
        if over.theme.is_some() {
            self.theme = over.theme;
        }
        if !over.recent.is_empty() {
            self.recent = over.recent;
            self.recent.truncate(RECENT_MAX);
        }
        if !over.find_queries.is_empty() {
            self.find_queries = over.find_queries;
            self.find_queries.truncate(FIND_MAX);
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
        // One `[appearance]` heading however many of its keys are set — a second heading for the
        // same table is legal TOML but reads as a mistake to anyone editing the file by hand, which
        // §12.4 expects them to do.
        if self.theme.is_some()
            || self.font.is_some()
            || self.font_size.is_some()
            || self.toolbar.is_some()
        {
            out.push_str("\n[appearance]\n");
            if let Some(theme) = &self.theme {
                out.push_str(&format!("theme = {}\n", quote(theme)));
            }
            if let Some(font) = &self.font {
                out.push_str(&format!("font = {}\n", quote(font)));
            }
            if let Some(size) = self.font_size {
                out.push_str(&format!("font_size = {size}\n"));
            }
            if let Some(shown) = self.toolbar {
                out.push_str(&format!("toolbar = {shown}\n"));
            }
        }
        if !self.recent.is_empty() {
            let files: Vec<String> = self.recent.iter().map(|p| quote(p)).collect();
            out.push_str(&format!("\n[recent]\nfiles = [{}]\n", files.join(", ")));
        }
        if !self.find_queries.is_empty() {
            let queries: Vec<String> = self.find_queries.iter().map(|q| quote(q)).collect();
            out.push_str(&format!("\n[find]\nqueries = [{}]\n", queries.join(", ")));
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
            if f.filters_hidden {
                out.push_str("filters_hidden = true\n");
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
                    "recent" => Section::Recent,
                    "find" => Section::Find,
                    _ => Section::Other,
                };
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let (key, value) = (key.trim(), value.trim());
            match section {
                Section::Appearance => match key {
                    "theme" => settings.theme = Some(unquote(value)),
                    "font" => settings.font = Some(unquote(value)),
                    "font_size" => settings.font_size = value.parse().ok(),
                    // Only the two spellings TOML has. Anything else leaves the key unset, which
                    // means the default rather than "hidden" — a typo must not silently take the
                    // row away, since the way back to it is the row's own menu item.
                    "toolbar" if value == "true" || value == "false" => {
                        settings.toolbar = Some(value == "true");
                    }
                    _ => {}
                },
                Section::Recent => {
                    if key == "files" {
                        settings.recent = array(value);
                        settings.recent.truncate(RECENT_MAX);
                    }
                }
                Section::Find => {
                    if key == "queries" {
                        settings.find_queries = array(value);
                        settings.find_queries.truncate(FIND_MAX);
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
                            "filters_hidden" => f.filters_hidden = value == "true",
                            "bookmarks" => {
                                f.bookmarks =
                                    array(value).iter().filter_map(|v| v.parse().ok()).collect()
                            }
                            "labels" => f.labels = array(value),
                            "columns" => {
                                f.columns =
                                    array(value).iter().filter_map(|v| v.parse().ok()).collect()
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
    Recent,
    Find,
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

    /// File ▸ Open Recent's list: newest first, re-opening moves to the front, and two spellings
    /// of one Windows path are one entry — paths compare case-insensitively on this platform.
    #[test]
    fn the_recent_list_remembers_in_order_and_dedupes_windows_paths() {
        let mut s = Settings::default();
        s.remember_recent(r"C:\logs\a.log");
        s.remember_recent(r"C:\logs\b.log");
        s.remember_recent(r"C:\LOGS\A.LOG");
        assert_eq!(
            s.recent,
            vec![r"C:\LOGS\A.LOG".to_owned(), r"C:\logs\b.log".to_owned()],
            "re-opening moved a to the front under its new spelling, not a third entry"
        );
        let read = Settings::from_toml(&s.to_toml());
        assert_eq!(read.recent, s.recent, "the list survives the file");
    }

    /// Ten entries — the platform's customary MRU depth — and the oldest falls off.
    #[test]
    fn the_recent_list_holds_ten_newest() {
        let mut s = Settings::default();
        for i in 0..12 {
            s.remember_recent(&format!(r"C:\logs\{i}.log"));
        }
        assert_eq!(s.recent.len(), RECENT_MAX);
        assert_eq!(s.recent[0], r"C:\logs\11.log");
        assert!(!s.recent.contains(&r"C:\logs\0.log".to_owned()));
        assert!(!s.recent.contains(&r"C:\logs\1.log".to_owned()));
    }

    /// The Find dialog's history: newest first, deduplicated **exactly** — case matters in a
    /// query in a way it does not in a Windows path — and capped like the recent files are.
    #[test]
    fn the_find_history_round_trips_and_dedupes_exactly() {
        let mut s = Settings::default();
        s.remember_query("Error");
        s.remember_query("timeout");
        s.remember_query("Error");
        assert_eq!(
            s.find_queries,
            vec!["Error".to_owned(), "timeout".to_owned()],
            "re-searching moves the query to the front, once"
        );
        s.remember_query("error");
        assert_eq!(
            s.find_queries.len(),
            3,
            "a different case is a different query"
        );
        let read = Settings::from_toml(&s.to_toml());
        assert_eq!(
            read.find_queries, s.find_queries,
            "the history survives the file"
        );
        for i in 0..12 {
            s.remember_query(&format!("q{i}"));
        }
        assert_eq!(s.find_queries.len(), FIND_MAX);
        assert_eq!(s.find_queries[0], "q11");
    }

    /// §12.4's per-key tier merge covers the list: an earlier tier's list wins whole.
    #[test]
    fn the_recent_list_merges_as_one_key() {
        let mut under = Settings::default();
        under.remember_recent(r"C:\old.log");
        let mut over = Settings::default();
        over.remember_recent(r"C:\new.log");
        let merged = under.clone().merged_under(over);
        assert_eq!(merged.recent, vec![r"C:\new.log".to_owned()]);
        let merged = under.merged_under(Settings::default());
        assert_eq!(
            merged.recent,
            vec![r"C:\old.log".to_owned()],
            "an empty over-tier does not erase the list"
        );
    }

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
            font: Some("Cascadia Mono".to_owned()),
            font_size: Some(18),
            toolbar: Some(false),
            recent: vec![r"C:\logs\app.log".to_owned()],
            find_queries: vec!["ERROR".to_owned(), r"time\d+".to_owned()],
        };
        s.set_file(FileState {
            path: r"C:\logs\app.log".to_owned(),
            chips: vec!["+error".to_owned(), "-retry \"quoted\"".to_owned()],
            collapse: true,
            bookmarks: vec![0, 42, 1_000_000],
            labels: vec!["1:Exception".to_owned(), "9:a \"quoted\" one".to_owned()],
            columns: vec![19, 5, 0],
            filters_hidden: false,
        });
        s.set_file(FileState {
            path: r"C:\logs\other.log".to_owned(),
            chips: vec!["+job".to_owned()],
            collapse: false,
            bookmarks: Vec::new(),
            labels: Vec::new(),
            columns: Vec::new(),
            filters_hidden: false,
        });
        s
    }

    /// A closed filter panel is remembered; an open one writes nothing.
    ///
    /// The asymmetry is the point. Absence of the key has to read as *shown*, because that is
    /// what every settings file written before the key existed means, and what every file whose
    /// panel was left open means. Only the deliberate choice to close it is worth a line.
    #[test]
    fn a_closed_filter_panel_is_remembered_and_an_open_one_says_nothing() {
        let mut s = Settings::default();
        s.set_file(FileState {
            path: r"C:\logs\hidden.log".to_owned(),
            chips: vec!["+error".to_owned()],
            filters_hidden: true,
            ..FileState::default()
        });
        s.set_file(FileState {
            path: r"C:\logs\shown.log".to_owned(),
            chips: vec!["+error".to_owned()],
            ..FileState::default()
        });

        let text = s.to_toml();
        assert_eq!(
            text.matches("filters_hidden").count(),
            1,
            "only the closed one is written: {text}"
        );

        let back = Settings::from_toml(&text);
        assert!(
            back.file(r"C:\logs\hidden.log")
                .expect("the hidden one")
                .filters_hidden,
            "{text}"
        );
        assert!(
            !back
                .file(r"C:\logs\shown.log")
                .expect("the shown one")
                .filters_hidden,
            "an absent key reads as shown: {text}"
        );
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
            filters_hidden: false,
        });
        assert!(s.file(r"C:\logs\app.log").is_none());
        s.set_file(FileState {
            path: r"c:\LOGS\other.log".to_owned(),
            chips: vec!["+x".to_owned()],
            collapse: false,
            bookmarks: Vec::new(),
            labels: Vec::new(),
            columns: Vec::new(),
            filters_hidden: false,
        });
        assert_eq!(s.files.len(), 1, "case-insensitive path replaces");
        assert_eq!(s.file(r"C:\logs\other.log").unwrap().chips, ["+x"]);
    }

    /// **An unreadable `toolbar` value leaves the row alone.** The only way back to a hidden
    /// toolbar is the toolbar's own menu item, so a typo in a hand-edited settings file must not be
    /// able to take it away — absent and unparseable both have to mean "the default", and the
    /// default is shown.
    #[test]
    fn only_a_real_boolean_hides_the_toolbar() {
        let read = |text: &str| Settings::from_toml(text).toolbar;
        assert_eq!(read("[appearance]\ntoolbar = false\n"), Some(false));
        assert_eq!(read("[appearance]\ntoolbar = true\n"), Some(true));
        assert_eq!(read("[appearance]\ntoolbar = \"no\"\n"), None, "a string");
        assert_eq!(read("[appearance]\ntoolbar = 0\n"), None, "a number");
        assert_eq!(read("[appearance]\ntoolbar = False\n"), None, "wrong case");
        assert_eq!(read("[appearance]\ntheme = \"dark\"\n"), None, "absent");
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
            filters_hidden: false,
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
            filters_hidden: false,
        });
        curated.set_file(FileState {
            path: "b.log".into(),
            chips: vec!["+b".into()],
            collapse: false,
            bookmarks: Vec::new(),
            labels: Vec::new(),
            columns: Vec::new(),
            filters_hidden: false,
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
