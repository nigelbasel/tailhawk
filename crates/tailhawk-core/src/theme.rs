//! The theme — V13, `UI-DESIGN.md` §11.2: every colour the grid and the chrome draw with, as one
//! value with a dark and a light set.
//!
//! ## One slot, read per frame
//!
//! A colour is not per-document state, and threading a theme through the painter, the row source
//! and thirty draw sites would carry it for no reason. So there is one process-wide [`Theme`],
//! [`theme`] returns a copy of it — it is a few hundred bytes of `Copy` data — and a switch is
//! [`set_theme`] followed by a repaint. Nothing caches a colour across frames except the semantic
//! catalogue, whose rules carry theirs; a caller that switches rebuilds it.
//!
//! ## What the light set is
//!
//! The dark values are the ones `UI-DESIGN.md` §10 left provisional and every earlier module
//! chose by eye; the light values are chosen the same way against a near-white ground: the same
//! hues, pulled darker so they carry on white, and the chrome a shade off the paper. The severity
//! ramp is §11.2's table — dark magenta, dark red, dark amber on light. All of it is as
//! provisional as the dark set, and lives in one place so a palette pass changes one file.
//!
//! ## High Contrast
//!
//! §11.2: "system colours are respected and user highlight rules are suppressed, with a visible
//! chip explaining why". [`Theme::high_contrast`] builds a theme from a system foreground and
//! background — the shell reads them from `GetSysColor` — with the semantic hues collapsed to the
//! foreground and the labels to the background, and sets [`Theme::suppress_rules`]. The shell
//! shows the chip.

use std::sync::RwLock;

/// An RGBA colour, as the render target takes it.
pub type Colour = [f32; 4];

/// The zero-config semantic layer's hues — `semantic.rs` builds its catalogue from these.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Semantic {
    pub fatal: Colour,
    pub error: Colour,
    pub warn: Colour,
    pub debug: Colour,
    pub trace: Colour,
    pub timestamp: Colour,
    pub url: Colour,
    pub ip: Colour,
    pub path: Colour,
    pub http_method: Colour,
    pub http_ok: Colour,
    pub hex: Colour,
    pub quoted: Colour,
    pub duration: Colour,
    pub key: Colour,
    pub number: Colour,
}

/// Every colour the product draws with.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Theme {
    /// Whether this is a dark theme — the ground is dark and the ink light.
    pub dark: bool,
    /// High Contrast: user highlight rules are not applied. See the module note.
    pub suppress_rules: bool,
    pub background: Colour,
    pub ink: Colour,
    pub selection_ink: Colour,
    pub match_bg: Colour,
    pub current_match_bg: Colour,
    pub current_match_ink: Colour,
    pub reveal_mark: Colour,
    pub continuation_ink: Colour,
    pub header_bg: Colour,
    pub header_ink: Colour,
    /// The line under the column header, and the divider between its columns.
    ///
    /// **A separate colour because it is the part that has to survive High Contrast.** There,
    /// `header_bg` *is* `background` — a header told apart from the rows by its fill alone
    /// disappears completely. A rule drawn in the system foreground is still a rule. This is what
    /// Explorer's own header leans on too: its fill is nearly the colour of the list beneath it,
    /// and the bottom border does the work.
    pub header_rule: Colour,
    pub gutter_ink: Colour,
    pub chrome_bg: Colour,
    pub field_bg: Colour,
    pub field_bg_focused: Colour,
    pub field_hint: Colour,
    pub field_selection_bg: Colour,
    pub caret: Colour,
    pub chip_include_bg: Colour,
    pub chip_exclude_bg: Colour,
    pub pane_bg: Colour,
    pub pane_edge: Colour,
    pub palette_bg: Colour,
    pub palette_selected_bg: Colour,
    pub bookmark_mark: Colour,
    pub tab_bg: Colour,
    pub tab_active_bg: Colour,
    /// `Ctrl+Shift+1…9`'s label backgrounds, in key order.
    pub labels: [Colour; 9],
    /// §7.1's derived identifier colours.
    pub identifiers: [Colour; 8],
    pub semantic: Semantic,
}

impl Theme {
    pub const fn dark() -> Self {
        Self {
            dark: true,
            suppress_rules: false,
            background: [0.071, 0.078, 0.090, 1.0],
            ink: [0.878, 0.890, 0.906, 1.0],
            selection_ink: [0.45, 0.72, 1.0, 1.0],
            match_bg: [0.36, 0.29, 0.06, 1.0],
            current_match_bg: [0.95, 0.62, 0.16, 1.0],
            current_match_ink: [0.071, 0.078, 0.090, 1.0],
            reveal_mark: [0.85, 0.65, 0.25, 1.0],
            continuation_ink: [0.56, 0.59, 0.64, 1.0],
            header_bg: [0.16, 0.175, 0.200, 1.0],
            header_ink: [0.84, 0.86, 0.89, 1.0],
            header_rule: [0.38, 0.41, 0.48, 1.0],
            gutter_ink: [0.40, 0.44, 0.50, 1.0],
            chrome_bg: [0.10, 0.11, 0.13, 1.0],
            field_bg: [0.14, 0.15, 0.18, 1.0],
            field_bg_focused: [0.18, 0.20, 0.24, 1.0],
            field_hint: [0.42, 0.45, 0.50, 1.0],
            field_selection_bg: [0.20, 0.36, 0.60, 1.0],
            caret: [0.88, 0.89, 0.91, 1.0],
            chip_include_bg: [0.14, 0.26, 0.20, 1.0],
            chip_exclude_bg: [0.30, 0.16, 0.16, 1.0],
            pane_bg: [0.11, 0.12, 0.15, 1.0],
            pane_edge: [0.30, 0.32, 0.38, 1.0],
            palette_bg: [0.16, 0.17, 0.21, 1.0],
            palette_selected_bg: [0.24, 0.30, 0.42, 1.0],
            bookmark_mark: [0.85, 0.65, 0.20, 1.0],
            tab_bg: [0.13, 0.14, 0.17, 1.0],
            tab_active_bg: [0.20, 0.22, 0.26, 1.0],
            labels: [
                [0.45, 0.30, 0.10, 1.0],
                [0.15, 0.40, 0.20, 1.0],
                [0.15, 0.30, 0.50, 1.0],
                [0.45, 0.20, 0.45, 1.0],
                [0.15, 0.42, 0.42, 1.0],
                [0.50, 0.15, 0.15, 1.0],
                [0.40, 0.40, 0.15, 1.0],
                [0.30, 0.30, 0.45, 1.0],
                [0.35, 0.35, 0.35, 1.0],
            ],
            identifiers: [
                [0.55, 0.78, 0.98, 1.0],
                [0.62, 0.86, 0.66, 1.0],
                [0.86, 0.72, 0.98, 1.0],
                [0.55, 0.88, 0.86, 1.0],
                [0.92, 0.80, 0.55, 1.0],
                [0.98, 0.68, 0.78, 1.0],
                [0.72, 0.80, 0.98, 1.0],
                [0.80, 0.90, 0.55, 1.0],
            ],
            semantic: Semantic {
                fatal: [0.96, 0.44, 0.64, 1.0],
                error: [0.96, 0.47, 0.38, 1.0],
                warn: [0.93, 0.73, 0.32, 1.0],
                debug: [0.56, 0.59, 0.64, 1.0],
                trace: [0.44, 0.47, 0.52, 1.0],
                timestamp: [0.52, 0.64, 0.78, 1.0],
                url: [0.47, 0.70, 0.96, 1.0],
                ip: [0.46, 0.82, 0.80, 1.0],
                path: [0.76, 0.71, 0.94, 1.0],
                http_method: [0.72, 0.85, 0.62, 1.0],
                http_ok: [0.56, 0.83, 0.56, 1.0],
                hex: [0.86, 0.66, 0.86, 1.0],
                quoted: [0.86, 0.77, 0.58, 1.0],
                duration: [0.60, 0.83, 0.63, 1.0],
                key: [0.63, 0.71, 0.79, 1.0],
                number: [0.61, 0.79, 0.94, 1.0],
            },
        }
    }

    pub const fn light() -> Self {
        Self {
            dark: false,
            suppress_rules: false,
            background: [0.985, 0.985, 0.98, 1.0],
            ink: [0.13, 0.14, 0.16, 1.0],
            selection_ink: [0.10, 0.35, 0.75, 1.0],
            match_bg: [0.98, 0.90, 0.60, 1.0],
            current_match_bg: [0.98, 0.62, 0.16, 1.0],
            current_match_ink: [0.10, 0.08, 0.05, 1.0],
            reveal_mark: [0.80, 0.55, 0.15, 1.0],
            continuation_ink: [0.45, 0.47, 0.52, 1.0],
            header_bg: [0.88, 0.88, 0.87, 1.0],
            header_ink: [0.20, 0.21, 0.25, 1.0],
            header_rule: [0.62, 0.63, 0.68, 1.0],
            gutter_ink: [0.58, 0.60, 0.65, 1.0],
            chrome_bg: [0.94, 0.94, 0.93, 1.0],
            field_bg: [1.0, 1.0, 1.0, 1.0],
            field_bg_focused: [1.0, 1.0, 1.0, 1.0],
            field_hint: [0.55, 0.57, 0.62, 1.0],
            field_selection_bg: [0.72, 0.82, 0.98, 1.0],
            caret: [0.13, 0.14, 0.16, 1.0],
            chip_include_bg: [0.78, 0.92, 0.82, 1.0],
            chip_exclude_bg: [0.98, 0.82, 0.82, 1.0],
            pane_bg: [0.95, 0.95, 0.94, 1.0],
            pane_edge: [0.75, 0.76, 0.80, 1.0],
            palette_bg: [0.96, 0.96, 0.97, 1.0],
            palette_selected_bg: [0.80, 0.86, 0.96, 1.0],
            bookmark_mark: [0.85, 0.55, 0.10, 1.0],
            tab_bg: [0.90, 0.90, 0.89, 1.0],
            tab_active_bg: [1.0, 1.0, 1.0, 1.0],
            labels: [
                [0.99, 0.88, 0.70, 1.0],
                [0.80, 0.94, 0.82, 1.0],
                [0.78, 0.87, 0.99, 1.0],
                [0.95, 0.82, 0.95, 1.0],
                [0.78, 0.94, 0.94, 1.0],
                [0.99, 0.80, 0.80, 1.0],
                [0.96, 0.96, 0.72, 1.0],
                [0.86, 0.86, 0.96, 1.0],
                [0.88, 0.88, 0.88, 1.0],
            ],
            identifiers: [
                [0.10, 0.40, 0.75, 1.0],
                [0.75, 0.38, 0.05, 1.0],
                [0.12, 0.50, 0.20, 1.0],
                [0.60, 0.20, 0.55, 1.0],
                [0.05, 0.50, 0.55, 1.0],
                [0.60, 0.48, 0.05, 1.0],
                [0.40, 0.30, 0.75, 1.0],
                [0.35, 0.50, 0.08, 1.0],
            ],
            semantic: Semantic {
                fatal: [0.60, 0.05, 0.40, 1.0],
                error: [0.72, 0.12, 0.08, 1.0],
                warn: [0.62, 0.42, 0.02, 1.0],
                debug: [0.45, 0.48, 0.53, 1.0],
                trace: [0.58, 0.60, 0.65, 1.0],
                timestamp: [0.32, 0.42, 0.58, 1.0],
                url: [0.08, 0.35, 0.72, 1.0],
                ip: [0.05, 0.48, 0.48, 1.0],
                path: [0.42, 0.32, 0.68, 1.0],
                http_method: [0.28, 0.48, 0.12, 1.0],
                http_ok: [0.12, 0.50, 0.18, 1.0],
                hex: [0.55, 0.25, 0.55, 1.0],
                quoted: [0.55, 0.40, 0.08, 1.0],
                duration: [0.15, 0.48, 0.22, 1.0],
                key: [0.35, 0.42, 0.52, 1.0],
                number: [0.12, 0.38, 0.65, 1.0],
            },
        }
    }

    /// §11.2's High Contrast: the system's foreground and background, the semantic hues collapsed
    /// to the foreground, labels and chips to the background, and user rules suppressed.
    pub fn high_contrast(foreground: Colour, background: Colour, highlight: Colour) -> Self {
        let dark = luminance(background) < luminance(foreground);
        let semantic = Semantic {
            fatal: foreground,
            error: foreground,
            warn: foreground,
            debug: foreground,
            trace: foreground,
            timestamp: foreground,
            url: foreground,
            ip: foreground,
            path: foreground,
            http_method: foreground,
            http_ok: foreground,
            hex: foreground,
            quoted: foreground,
            duration: foreground,
            key: foreground,
            number: foreground,
        };
        Self {
            dark,
            suppress_rules: true,
            background,
            ink: foreground,
            selection_ink: highlight,
            match_bg: highlight,
            current_match_bg: highlight,
            current_match_ink: background,
            reveal_mark: foreground,
            continuation_ink: foreground,
            header_bg: background,
            header_ink: foreground,
            // The whole reason this colour exists: here the band and the rows share a fill, so the
            // rule is the only thing left that can say "header".
            header_rule: foreground,
            gutter_ink: foreground,
            chrome_bg: background,
            field_bg: background,
            field_bg_focused: background,
            field_hint: foreground,
            field_selection_bg: highlight,
            caret: foreground,
            chip_include_bg: background,
            chip_exclude_bg: background,
            pane_bg: background,
            pane_edge: foreground,
            palette_bg: background,
            palette_selected_bg: highlight,
            bookmark_mark: foreground,
            tab_bg: background,
            tab_active_bg: highlight,
            labels: [background; 9],
            identifiers: [foreground; 8],
            semantic,
        }
    }

    /// The background as 8-bit sRGB, for a GDI brush.
    pub fn background_rgb8(&self) -> (u8, u8, u8) {
        let [r, g, b, _] = self.background;
        (to8(r), to8(g), to8(b))
    }
}

fn to8(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// Relative luminance, roughly — enough to tell a dark ground from a light one.
fn luminance(c: Colour) -> f32 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

impl Default for Theme {
    /// Light — a `Default` for callers that need *a* theme, not the one a session opens with.
    ///
    /// What a real session opens with is [`chosen`], which follows Windows. Nothing about black on
    /// white is settled in `UI-DESIGN.md`; an earlier version of this comment claimed §11.2 said so
    /// and §11.2 is the severity ramp.
    fn default() -> Self {
        Self::light()
    }
}

static THEME: RwLock<Theme> = RwLock::new(Theme::dark());

/// The theme in force. A copy — see the module note.
pub fn theme() -> Theme {
    *THEME.read().unwrap_or_else(|e| e.into_inner())
}

/// Replaces the theme. The caller repaints, and rebuilds anything that cached a colour.
pub fn set_theme(theme: Theme) {
    *THEME.write().unwrap_or_else(|e| e.into_inner()) = theme;
}

/// `--theme=dark|light`. `system` is the shell's to resolve, since it needs the registry.
pub fn by_name(name: &str) -> Option<Theme> {
    match name {
        "dark" => Some(Theme::dark()),
        "light" => Some(Theme::light()),
        _ => None,
    }
}

/// Where a frame's ground comes from, once the question has been decided.
///
/// This names the *source* rather than carrying a [`Theme`], for two reasons. High Contrast could
/// not carry one anyway — its colours are the ones the user configured in Windows, and reading them
/// needs `SystemParametersInfoW` and `GetSysColor`. And a palette is some six hundred bytes, so an
/// enum with one in it is an enum that is expensive to return in order to say "light".
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ThemeChoice {
    /// High Contrast is on: the shell reads the system's own colours and uses those.
    SystemHighContrast,
    /// [`Theme::dark`].
    Dark,
    /// [`Theme::light`].
    Light,
}

/// Decide which palette a session opens with.
///
/// `asked` is `--theme=` if it was given, else the saved setting, else nothing at all. The two
/// system facts arrive as arguments rather than being read here, so the whole decision is a
/// function of its inputs and can be exercised without a window.
///
/// High Contrast wins over everything — a user who has turned it on has said something louder than
/// a config file. Otherwise `"system"`, an unrecognised name and no answer at all all mean the same
/// thing: follow Windows. A Windows 11 app that stays white on a dark desktop looks broken, and its
/// title bar — which the compositor draws, not us — will follow the system regardless of what we
/// paint underneath it.
pub fn chosen(asked: Option<&str>, high_contrast: bool, system_is_light: bool) -> ThemeChoice {
    if high_contrast {
        return ThemeChoice::SystemHighContrast;
    }
    let follow_system = if system_is_light {
        ThemeChoice::Light
    } else {
        ThemeChoice::Dark
    };
    match asked {
        Some("dark") => ThemeChoice::Dark,
        Some("light") => ThemeChoice::Light,
        _ => follow_system,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// WCAG relative luminance, and the contrast ratio between two colours.
    ///
    /// The theme's floats are **sRGB-encoded**, not linear: every render target is
    /// `DXGI_FORMAT_B8G8R8A8_UNORM` rather than `_SRGB`, so nothing converts them on the way to the
    /// display and they have to be linearised here.
    fn linear(c: f32) -> f32 {
        if c <= 0.03928 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    }

    fn relative_luminance(c: Colour) -> f32 {
        0.2126 * linear(c[0]) + 0.7152 * linear(c[1]) + 0.0722 * linear(c[2])
    }

    fn contrast(a: Colour, b: Colour) -> f32 {
        let (a, b) = (relative_luminance(a), relative_luminance(b));
        let (hi, lo) = if a > b { (a, b) } else { (b, a) };
        (hi + 0.05) / (lo + 0.05)
    }

    /// §2.5: the column header has to be tellable from the rows beneath it.
    ///
    /// **This is a regression guard on a defect that shipped.** The header band was drawn all
    /// along, at 1.11 : 1 in the dark theme and 1.16 : 1 in the light — below the threshold where
    /// anything is discernible — so it read as another log line, and the natural conclusion was
    /// that the header had no formatting at all rather than formatting nobody could see.
    ///
    /// The rule carries the separation and the fill only supports it, which is why the fill's
    /// threshold here is modest: principle 5 wants the user's own highlight colours to be the
    /// loudest thing on screen, so a header that shouts is its own kind of wrong.
    #[test]
    fn the_column_header_is_distinguishable_from_the_rows() {
        for theme in [Theme::dark(), Theme::light()] {
            let which = if theme.dark { "dark" } else { "light" };

            let band = contrast(theme.header_bg, theme.background);
            assert!(
                band >= 1.25,
                "{which}: the header band is {band:.2} : 1 against the rows — invisible"
            );

            let rule = contrast(theme.header_rule, theme.header_bg);
            assert!(
                rule >= 1.8,
                "{which}: the header rule is {rule:.2} : 1 against its own band"
            );

            // The strip that names the columns must not be fainter than the data it names, which
            // is what made it recede rather than stand out.
            let header_ink = contrast(theme.header_ink, theme.header_bg);
            let row_ink = contrast(theme.ink, theme.background);
            assert!(
                header_ink >= row_ink * 0.6,
                "{which}: header ink is {header_ink:.1} : 1 where the rows are {row_ink:.1} : 1"
            );
        }
    }

    /// The rule is the part that has to survive High Contrast, and this is why it is its own
    /// colour: there `header_bg` **is** `background`, so a header told apart by its fill alone is
    /// not told apart at all.
    #[test]
    fn under_high_contrast_the_header_rule_still_separates() {
        let fg = [1.0, 1.0, 1.0, 1.0];
        let bg = [0.0, 0.0, 0.0, 1.0];
        let theme = Theme::high_contrast(fg, bg, [0.0, 0.4, 1.0, 1.0]);

        assert_eq!(
            contrast(theme.header_bg, theme.background),
            1.0,
            "the premise of this test: the fill carries nothing here"
        );
        assert!(
            contrast(theme.header_rule, theme.header_bg) > 15.0,
            "the rule is all that is left, and it has to be unmissable"
        );
    }

    #[test]
    fn high_contrast_outranks_every_other_answer() {
        for asked in [None, Some("dark"), Some("light"), Some("system")] {
            assert_eq!(
                chosen(asked, true, true),
                ThemeChoice::SystemHighContrast,
                "High Contrast is the user shouting; {asked:?} does not talk over it"
            );
            assert_eq!(chosen(asked, true, false), ThemeChoice::SystemHighContrast);
        }
    }

    #[test]
    fn an_explicit_name_is_obeyed_whatever_windows_thinks() {
        for system_is_light in [true, false] {
            assert_eq!(
                chosen(Some("dark"), false, system_is_light),
                ThemeChoice::Dark
            );
            assert_eq!(
                chosen(Some("light"), false, system_is_light),
                ThemeChoice::Light
            );
        }
    }

    #[test]
    fn system_means_system() {
        assert_eq!(chosen(Some("system"), false, true), ThemeChoice::Light);
        assert_eq!(chosen(Some("system"), false, false), ThemeChoice::Dark);
    }

    #[test]
    fn asked_nothing_follows_windows_rather_than_forcing_white() {
        assert_eq!(chosen(None, false, true), ThemeChoice::Light);
        assert_eq!(
            chosen(None, false, false),
            ThemeChoice::Dark,
            "a first run on a dark desktop opened white under a dark title bar"
        );
    }

    #[test]
    fn a_name_we_do_not_know_is_not_a_vote_for_white() {
        assert_eq!(
            chosen(Some("solarized"), false, false),
            ThemeChoice::Dark,
            "a typo in the settings file fell through to light on a dark desktop"
        );
        assert_eq!(chosen(Some("solarized"), false, true), ThemeChoice::Light);
    }

    #[test]
    fn the_two_sets_are_opposite_in_ground_and_the_default_is_light() {
        let dark = Theme::dark();
        let light = Theme::light();
        assert!(dark.dark && !light.dark);
        assert!(luminance(dark.background) < 0.2);
        assert!(luminance(light.background) > 0.9);
        assert!(luminance(dark.ink) > 0.7);
        assert!(luminance(light.ink) < 0.3);
        // A palette asked for by name is exactly itself; which one an unasked session opens with
        // is `chosen`'s business, not this test's.
        assert_eq!(Theme::default(), light);
        assert_eq!(dark.background_rgb8(), (18, 20, 23));
        assert_eq!(by_name("light"), Some(light));
        assert_eq!(by_name("system"), None);
    }

    #[test]
    fn every_light_hue_carries_on_white_and_every_dark_one_on_black() {
        let light = Theme::light();
        let dark = Theme::dark();
        let semantic_l = [
            light.semantic.fatal,
            light.semantic.error,
            light.semantic.warn,
            light.semantic.timestamp,
            light.semantic.url,
            light.semantic.number,
        ];
        for c in semantic_l.iter().chain(light.identifiers.iter()) {
            assert!(luminance(*c) < 0.5, "{c:?} is too pale for a white ground");
        }
        let semantic_d = [
            dark.semantic.fatal,
            dark.semantic.error,
            dark.semantic.warn,
            dark.semantic.timestamp,
            dark.semantic.url,
            dark.semantic.number,
        ];
        for c in semantic_d.iter().chain(dark.identifiers.iter()) {
            assert!(luminance(*c) > 0.4, "{c:?} is too dark for a black ground");
        }
        for label in light.labels {
            assert!(
                luminance(label) > 0.7,
                "a light label tint keeps dark ink readable"
            );
        }
    }

    #[test]
    fn high_contrast_uses_the_system_pair_and_suppresses_rules() {
        let hc = Theme::high_contrast([1.0; 4], [0.0, 0.0, 0.0, 1.0], [0.0, 0.6, 1.0, 1.0]);
        assert!(hc.dark && hc.suppress_rules);
        assert_eq!(hc.ink, [1.0; 4]);
        assert_eq!(hc.semantic.error, [1.0; 4]);
        assert_eq!(hc.labels[3], [0.0, 0.0, 0.0, 1.0]);
        let hc = Theme::high_contrast([0.0, 0.0, 0.0, 1.0], [1.0; 4], [0.0, 0.6, 1.0, 1.0]);
        assert!(!hc.dark);
    }

    #[test]
    fn the_slot_is_read_and_replaced() {
        // Tests run in parallel and every other test reads the default, so this one only puts
        // back what is there — the write path is exercised, the value is not changed.
        let before = theme();
        set_theme(before);
        assert_eq!(theme(), before);
    }
}
