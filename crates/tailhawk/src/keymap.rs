//! §2.2's *Keyboard map* — `UI-DESIGN.md` §12's table, built rather than written.
//!
//! ## Why it is generated
//!
//! A hand-kept list of keys is a list that disagrees with the product the first time a key moves,
//! and it disagrees silently: nothing fails, the table is simply wrong for whoever reads it next.
//! This project has already shipped one stale list of its own capabilities. So the map is built
//! from the same `(command, name, key)` register the palette and the menu bar are built from, and a
//! key can only appear here by being real.
//!
//! ## What it cannot know
//!
//! The register holds the commands. It does not hold the keys that are not commands — the arrows,
//! `PgUp`/`PgDn`, `Home`/`End`, `Space`/`b`, `Esc` unwinding a modal — because those are handled in
//! the grid's own key routing and have no `Command` to be listed against. Those come from
//! [`NAVIGATION`], which is the one hand-written part, and it is confined to movement keys that
//! `UI-DESIGN.md` §12 fixes and nothing reassigns.

/// A section of the map, with a heading a reader can scan for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeymapSection {
    pub heading: String,
    pub rows: Vec<KeymapRow>,
}

/// One binding: the keystroke, and what it does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeymapRow {
    pub keys: String,
    pub what: String,
}

/// The whole map as one frame draws it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeymapSheet {
    pub title: String,
    pub sections: Vec<KeymapSection>,
}

/// §12's movement keys — the ones the grid routes itself, which have no [`Command`] to be derived
/// from. Kept deliberately short: anything that *is* a command belongs in the generated half.
///
/// [`Command`]: crate::Command
pub const NAVIGATION: &[(&str, &str)] = &[
    ("↑ / ↓", "One line"),
    ("PgUp / PgDn", "One screen"),
    ("Space / b", "Page down / up"),
    ("Home / End", "Start / end of line"),
    ("Ctrl+Home / Ctrl+End", "Start / end of document"),
    ("← / →", "Scroll across"),
    ("Esc", "Close whatever is open, one level at a time"),
    ("Alt", "Focus the menu bar and reveal its mnemonics"),
];

/// Build the map from the command register.
///
/// `commands` is `(name, keys)` in the order the register lists them, which is the order a user
/// learns them. Entries with no keystroke are dropped: this is a map of *keys*, and a command
/// reachable only from the palette has nothing to say on it — it is not evidence of a missing
/// binding, and listing it with a blank cell reads as one.
pub fn keymap_sheet_of<'a>(commands: impl IntoIterator<Item = (&'a str, &'a str)>) -> KeymapSheet {
    let bound: Vec<KeymapRow> = commands
        .into_iter()
        .filter(|(_, keys)| !keys.is_empty())
        .map(|(name, keys)| KeymapRow {
            keys: keys.to_owned(),
            what: name.to_owned(),
        })
        .collect();
    KeymapSheet {
        title: "Keyboard map".into(),
        sections: vec![
            KeymapSection {
                heading: "Moving around".into(),
                rows: NAVIGATION
                    .iter()
                    .map(|(keys, what)| KeymapRow {
                        keys: (*keys).to_owned(),
                        what: (*what).to_owned(),
                    })
                    .collect(),
            },
            KeymapSection {
                heading: "Commands".into(),
                rows: bound,
            },
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sheet() -> KeymapSheet {
        keymap_sheet_of([
            ("Open file…", "Ctrl+O"),
            ("Search", "Ctrl+F"),
            ("Log format…", ""),
            ("Reload rules", ""),
            ("Bookmark", "Ctrl+D"),
        ])
    }

    #[test]
    fn a_command_with_no_keystroke_is_not_a_row() {
        let rows = &sheet().sections[1].rows;
        assert!(
            !rows.iter().any(|r| r.what == "Log format…"),
            "a palette-only command listed with a blank key reads as a missing binding"
        );
        assert_eq!(rows.len(), 3);
    }

    #[test]
    fn every_row_carries_both_halves() {
        for section in &sheet().sections {
            for row in &section.rows {
                assert!(!row.keys.is_empty(), "{} has no keystroke", row.what);
                assert!(!row.what.is_empty(), "{} does nothing", row.keys);
            }
        }
    }

    #[test]
    fn the_keys_are_the_registers_own_words() {
        let rows = &sheet().sections[1].rows;
        let open = rows
            .iter()
            .find(|r| r.what == "Open file…")
            .expect("listed");
        assert_eq!(
            open.keys, "Ctrl+O",
            "the map must quote the register, not restate it — that is the whole point of \
             generating it"
        );
    }

    #[test]
    fn movement_keys_are_there_even_though_they_are_not_commands() {
        let moving = &sheet().sections[0];
        assert_eq!(moving.heading, "Moving around");
        assert!(
            moving.rows.iter().any(|r| r.keys.contains("PgUp")),
            "paging has no Command and would vanish from a purely generated map"
        );
        assert!(moving.rows.iter().any(|r| r.keys == "Esc"));
    }

    #[test]
    fn an_empty_register_still_gives_a_usable_map() {
        let empty = keymap_sheet_of([]);
        assert!(
            !empty.sections[0].rows.is_empty(),
            "movement is always available, even before a file is open"
        );
        assert!(empty.sections[1].rows.is_empty());
    }
}
