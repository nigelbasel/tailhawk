//! §2.2's *Preferences* — the editable half of the four unbuilt menu surfaces.
//!
//! ## What it edits, and what it deliberately does not
//!
//! `SPEC.md` §12.4's `[appearance]` table: the theme, and the grid's font face and size. Everything
//! else the product persists is either *state* rather than preference — where the window was, what a
//! file was being looked at through — or is edited where it lives, as §7.1's highlight rules are
//! edited in §5's rules editor. A preferences dialog that grows a second door to the rules would
//! make two places to change one thing, which §1.2's memorability rule exists to prevent.
//!
//! ## Why the face list is passed in
//!
//! The installed monospace faces come from `EnumFontFamiliesEx`, which needs a device context. That
//! is the shell's business; this takes the answer. Everything here is then a function of its inputs
//! and runs with no window — including the part most likely to be wrong, which is what happens when
//! the saved face is not installed on this machine.

/// Which setting a row edits.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Setting {
    Theme,
    FontFace,
    FontSize,
}

/// The smallest and largest em size offered, in device pixels at the 96-DPI baseline.
///
/// Below eight a log is unreadable and above thirty-two a row holds nothing useful; both ends are
/// clamped rather than allowed to wrap, because a size control that jumps from 32 to 8 on one extra
/// key press is a control that loses a user's place.
pub const MIN_SIZE: u16 = 8;
pub const MAX_SIZE: u16 = 32;

/// The three answers §12.4's `theme` key takes, in the order the control cycles them.
pub const THEMES: &[&str] = &["system", "light", "dark"];

/// The preferences being edited, and where the selection is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Prefs {
    theme: String,
    face: String,
    size: u16,
    selected: usize,
    /// The monospace faces installed on this machine, as the shell enumerated them.
    faces: Vec<String>,
    dirty: bool,
    face_chosen: bool,
}

/// One row as a frame draws it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrefsRow {
    pub label: String,
    pub value: String,
    pub setting: Setting,
    pub selected: bool,
    /// Set when the value is not one this machine can honour — a face saved on another machine, or
    /// carried in on a settings file from an exe-adjacent tier.
    pub unavailable: bool,
}

/// The whole sheet as a frame draws it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrefsSheet {
    pub title: String,
    pub rows: Vec<PrefsRow>,
    pub legend: String,
}

impl Prefs {
    /// Open the sheet over the settings in force.
    ///
    /// `faces` is what this machine actually has. A saved face that is not among them is **kept
    /// rather than silently replaced** — the user chose it, they may be carrying settings between
    /// machines, and quietly rewriting it to whatever is installed here loses that choice the next
    /// time the file is written. It is marked unavailable instead, which says what is wrong.
    pub fn new(theme: &str, face: &str, size: u16, faces: Vec<String>) -> Self {
        Self {
            theme: if THEMES.contains(&theme) {
                theme.to_owned()
            } else {
                "system".to_owned()
            },
            face: face.to_owned(),
            size: size.clamp(MIN_SIZE, MAX_SIZE),
            selected: 0,
            faces,
            dirty: false,
            face_chosen: false,
        }
    }

    pub fn theme(&self) -> &str {
        &self.theme
    }

    pub fn face(&self) -> &str {
        &self.face
    }

    pub fn size(&self) -> u16 {
        self.size
    }

    /// Whether anything has been changed since the sheet opened. The caller writes the settings
    /// file only when this is true — a settings file rewritten by every visit to a dialog is a
    /// settings file whose modification time means nothing.
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Whether the user picked a face here, as opposed to the sheet merely showing the one in use.
    ///
    /// The caller records a face only when this is true. The sheet opens on whatever is being drawn
    /// with, and writing that back would pin a name nobody chose — after which the built-in fallback
    /// chain could never be followed on that machine again.
    pub fn face_chosen(&self) -> bool {
        self.face_chosen
    }

    /// Put the selection on the row that edits `setting`, so *Font…* can open this sheet already
    /// pointing at the font rather than making the user find it.
    pub fn focus(&mut self, setting: Setting) {
        if let Some(i) = Self::ORDER.iter().position(|s| *s == setting) {
            self.selected = i;
        }
    }

    const ORDER: &'static [Setting] = &[Setting::Theme, Setting::FontFace, Setting::FontSize];

    /// Move the selection. Clamped rather than wrapped, for the reason [`MIN_SIZE`] gives.
    pub fn step(&mut self, down: bool) {
        let last = Self::ORDER.len() - 1;
        self.selected = if down {
            (self.selected + 1).min(last)
        } else {
            self.selected.saturating_sub(1)
        };
    }

    /// Change the selected setting's value. Returns whether anything actually changed, so the
    /// caller can decide whether the frame is worth redrawing.
    pub fn adjust(&mut self, forward: bool) -> bool {
        let before = (self.theme.clone(), self.face.clone(), self.size);
        match Self::ORDER[self.selected] {
            Setting::Theme => {
                let i = THEMES.iter().position(|t| *t == self.theme).unwrap_or(0);
                let n = THEMES.len();
                let next = if forward {
                    (i + 1) % n
                } else {
                    (i + n - 1) % n
                };
                self.theme = THEMES[next].to_owned();
            }
            Setting::FontFace => {
                if self.faces.is_empty() {
                    return false;
                }
                // A face that is not installed has no position in the list, so stepping from it
                // starts at the beginning rather than going nowhere.
                let next = match self.faces.iter().position(|f| *f == self.face) {
                    Some(i) => {
                        let n = self.faces.len();
                        if forward {
                            (i + 1) % n
                        } else {
                            (i + n - 1) % n
                        }
                    }
                    None => 0,
                };
                self.face = self.faces[next].clone();
                self.face_chosen = true;
            }
            Setting::FontSize => {
                self.size = if forward {
                    (self.size + 1).min(MAX_SIZE)
                } else {
                    self.size.saturating_sub(1).max(MIN_SIZE)
                };
            }
        }
        let changed = before != (self.theme.clone(), self.face.clone(), self.size);
        self.dirty |= changed;
        changed
    }

    /// The picture of this sheet for one frame.
    pub fn sheet(&self) -> PrefsSheet {
        let rows = Self::ORDER
            .iter()
            .enumerate()
            .map(|(i, setting)| {
                let (label, value, unavailable) = match setting {
                    Setting::Theme => ("Theme", self.theme.clone(), false),
                    Setting::FontFace => (
                        "Font",
                        self.face.clone(),
                        !self.faces.is_empty() && !self.faces.contains(&self.face),
                    ),
                    Setting::FontSize => ("Size", format!("{} px", self.size), false),
                };
                PrefsRow {
                    label: label.to_owned(),
                    value,
                    setting: *setting,
                    selected: i == self.selected,
                    unavailable,
                }
            })
            .collect();
        PrefsSheet {
            title: "Preferences".to_owned(),
            rows,
            legend: "↑↓ choose · ←→ change · Esc close".to_owned(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn faces() -> Vec<String> {
        ["Cascadia Mono", "Consolas", "Courier New"]
            .iter()
            .map(|s| (*s).to_owned())
            .collect()
    }

    fn prefs() -> Prefs {
        Prefs::new("dark", "Consolas", 16, faces())
    }

    #[test]
    fn a_saved_face_this_machine_does_not_have_is_kept_and_marked() {
        let p = Prefs::new("dark", "Berkeley Mono", 16, faces());
        assert_eq!(
            p.face(),
            "Berkeley Mono",
            "the user chose it; carrying settings between machines must not lose that"
        );
        let row = p
            .sheet()
            .rows
            .into_iter()
            .find(|r| r.setting == Setting::FontFace)
            .expect("a font row");
        assert!(
            row.unavailable,
            "but the sheet has to say it cannot be honoured here"
        );
    }

    #[test]
    fn stepping_from_an_uninstalled_face_lands_somewhere_real() {
        let mut p = Prefs::new("dark", "Berkeley Mono", 16, faces());
        p.focus(Setting::FontFace);
        assert!(p.adjust(true));
        assert!(
            faces().contains(&p.face().to_owned()),
            "got {} — an unknown face has no position, so the step must start the list",
            p.face()
        );
    }

    #[test]
    fn the_size_clamps_at_both_ends_rather_than_wrapping() {
        let mut p = Prefs::new("dark", "Consolas", MAX_SIZE, faces());
        p.focus(Setting::FontSize);
        assert!(!p.adjust(true), "already at the top, so nothing changed");
        assert_eq!(p.size(), MAX_SIZE, "and it must not wrap round to 8");

        let mut p = Prefs::new("dark", "Consolas", MIN_SIZE, faces());
        p.focus(Setting::FontSize);
        assert!(!p.adjust(false));
        assert_eq!(p.size(), MIN_SIZE);
    }

    #[test]
    fn a_size_outside_the_range_is_pulled_into_it_on_open() {
        assert_eq!(
            Prefs::new("dark", "Consolas", 400, faces()).size(),
            MAX_SIZE
        );
        assert_eq!(Prefs::new("dark", "Consolas", 1, faces()).size(), MIN_SIZE);
    }

    #[test]
    fn the_theme_cycles_both_ways_through_all_three() {
        let mut p = Prefs::new("system", "Consolas", 16, faces());
        p.focus(Setting::Theme);
        let mut seen = vec![p.theme().to_owned()];
        for _ in 0..2 {
            p.adjust(true);
            seen.push(p.theme().to_owned());
        }
        seen.sort();
        seen.dedup();
        assert_eq!(seen.len(), 3, "all three answers must be reachable");
        p.adjust(true);
        assert_eq!(p.theme(), "system", "and it comes back round");
        p.adjust(false);
        assert_eq!(p.theme(), "dark", "backwards too");
    }

    #[test]
    fn an_unrecognised_saved_theme_falls_back_rather_than_sticking() {
        let p = Prefs::new("solarized", "Consolas", 16, faces());
        assert_eq!(
            p.theme(),
            "system",
            "a theme name we cannot honour is not a theme; following Windows is the safe answer"
        );
    }

    #[test]
    fn opening_the_sheet_does_not_make_it_dirty() {
        let mut p = prefs();
        assert!(!p.is_dirty());
        p.step(true);
        assert!(
            !p.is_dirty(),
            "moving the selection changes nothing worth writing a settings file for"
        );
        p.focus(Setting::FontSize);
        p.adjust(true);
        assert!(p.is_dirty(), "changing a value does");
    }

    #[test]
    fn an_adjustment_that_changes_nothing_does_not_dirty_it() {
        let mut p = Prefs::new("dark", "Consolas", MAX_SIZE, faces());
        p.focus(Setting::FontSize);
        p.adjust(true);
        assert!(
            !p.is_dirty(),
            "a key press that hit the ceiling must not cause a write"
        );
    }

    #[test]
    fn the_selection_clamps_at_both_ends() {
        let mut p = prefs();
        for _ in 0..10 {
            p.step(false);
        }
        assert!(p.sheet().rows[0].selected);
        for _ in 0..10 {
            p.step(true);
        }
        let rows = p.sheet().rows;
        assert!(rows[rows.len() - 1].selected, "and never past the last row");
    }

    #[test]
    fn font_opens_the_sheet_already_on_the_font() {
        let mut p = prefs();
        p.focus(Setting::FontFace);
        let row = p
            .sheet()
            .rows
            .into_iter()
            .find(|r| r.selected)
            .expect("something is selected");
        assert_eq!(
            row.setting,
            Setting::FontFace,
            "Font… must not make the user hunt for the font row"
        );
    }

    #[test]
    fn a_machine_with_no_faces_enumerated_does_not_wipe_the_choice() {
        let mut p = Prefs::new("dark", "Consolas", 16, vec![]);
        p.focus(Setting::FontFace);
        assert!(!p.adjust(true), "there is nothing to change to");
        assert_eq!(p.face(), "Consolas", "so the saved face stands");
        let row = p
            .sheet()
            .rows
            .into_iter()
            .find(|r| r.setting == Setting::FontFace)
            .expect("a font row");
        assert!(
            !row.unavailable,
            "an empty enumeration is our failure to ask, not evidence the face is missing"
        );
    }

    #[test]
    fn every_row_is_labelled_and_exactly_one_is_selected() {
        let sheet = prefs().sheet();
        assert_eq!(sheet.rows.iter().filter(|r| r.selected).count(), 1);
        for row in &sheet.rows {
            assert!(!row.label.is_empty());
            assert!(!row.value.is_empty(), "{} shows nothing", row.label);
        }
    }
}
