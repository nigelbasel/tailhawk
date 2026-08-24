//! §2.2's About box — the product's own facts, as one frame draws them.
//!
//! ## Why this is a view-model and not a `MessageBoxW`
//!
//! A message box would be four lines of shell code and would not be testable, would not honour the
//! theme, and would put a second window on screen that the §13 UIA provider does not describe. Every
//! other modal in this product — §5's rules editor, §6.2's wizard, §9's palette — is an overlay over
//! the grid built from a pure view-model, and this is the smallest of them.
//!
//! ## Why the network posture is on it
//!
//! `SPEC.md` §13.1 promises no telemetry, no update ping, no font or CDN fetch and no outbound
//! connection of any kind. That guarantee is worth nothing to a user who cannot see it, and a log
//! viewer is exactly the tool people are told to run against files they are not allowed to leak —
//! so the one place a user looks for "what is this program" says it.

/// The About box as one frame draws it: a title, then labelled rows, then a closing statement.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AboutSheet {
    pub title: String,
    pub rows: Vec<AboutRow>,
    /// `SPEC.md` §13.1's guarantee, in the words a user can check.
    pub assurance: Vec<String>,
}

/// One labelled fact.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AboutRow {
    pub label: String,
    pub value: String,
}

/// What the shell knows that this box reports. Passed in rather than read here, so the mapping can
/// be exercised without a device, a window or a running renderer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AboutFacts<'a> {
    /// The PE version resource's string, e.g. `2026.8.24.16152`.
    pub version: &'a str,
    /// `hardware`, `warp`, or whatever the fallback chain settled on.
    pub backend: &'a str,
    /// The architecture this binary was built for.
    pub arch: &'a str,
    /// Whether `--stateless` is in force, which changes what the product writes and is therefore
    /// something a user reading this box is entitled to know.
    pub stateless: bool,
}

/// The picture of the About box for one frame.
///
/// The version is shown exactly as the PE resource carries it rather than reformatted: it is the
/// string a user will be asked to quote in a bug report, and a version that reads differently in
/// two places is a version that cannot be quoted.
pub fn about_sheet_of(facts: AboutFacts<'_>) -> AboutSheet {
    let mut rows = vec![
        AboutRow {
            label: "Version".into(),
            value: facts.version.into(),
        },
        AboutRow {
            label: "Renderer".into(),
            value: facts.backend.into(),
        },
        AboutRow {
            label: "Architecture".into(),
            value: facts.arch.into(),
        },
    ];
    rows.push(AboutRow {
        label: "Settings".into(),
        value: if facts.stateless {
            "stateless — nothing is written".into()
        } else {
            "saved between sessions".into()
        },
    });
    AboutSheet {
        title: "Tailhawk".into(),
        rows,
        assurance: vec![
            "No telemetry. No update check. No outbound connection of any kind.".into(),
            "Files are opened read-only and are never modified.".into(),
        ],
    }
}

/// The sheet flattened to the one string `TaskDialogIndirect` takes as its content: the labelled
/// rows, a blank line, then the assurance. The dialog is presentation only — everything it says
/// comes through here from the tested mapping above, so the native surface cannot fork from it.
pub fn dialog_content(sheet: &AboutSheet) -> String {
    let mut content = String::new();
    for row in &sheet.rows {
        if !content.is_empty() {
            content.push('\n');
        }
        content.push_str(&row.label);
        content.push_str(":  ");
        content.push_str(&row.value);
    }
    for (i, line) in sheet.assurance.iter().enumerate() {
        content.push_str(if i == 0 { "\n\n" } else { "\n" });
        content.push_str(line);
    }
    content
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts() -> AboutFacts<'static> {
        AboutFacts {
            version: "2026.8.24.16152",
            backend: "hardware",
            arch: "x86_64",
            stateless: false,
        }
    }

    #[test]
    fn the_version_is_reported_exactly_as_the_pe_resource_carries_it() {
        let sheet = about_sheet_of(facts());
        let shown = &sheet
            .rows
            .iter()
            .find(|r| r.label == "Version")
            .expect("a version row")
            .value;
        assert_eq!(
            shown, "2026.8.24.16152",
            "this is the string a user quotes in a bug report; it must not be reformatted"
        );
    }

    #[test]
    fn the_box_states_the_network_posture() {
        let sheet = about_sheet_of(facts());
        let all = sheet.assurance.join(" ").to_lowercase();
        assert!(
            all.contains("no telemetry"),
            "SPEC 13.1's guarantee is the one a log viewer's user most needs to see"
        );
        assert!(all.contains("outbound"));
        assert!(
            all.contains("read-only"),
            "and that opening a log cannot change it"
        );
    }

    #[test]
    fn stateless_is_visible_because_it_changes_what_the_product_does() {
        let saved = about_sheet_of(facts());
        let stateless = about_sheet_of(AboutFacts {
            stateless: true,
            ..facts()
        });
        assert_ne!(
            saved.rows, stateless.rows,
            "a session that writes nothing must not look identical to one that does"
        );
        assert!(stateless
            .rows
            .iter()
            .any(|r| r.value.contains("nothing is written")));
    }

    #[test]
    fn the_backend_is_reported_so_a_slow_session_can_be_explained() {
        let warp = about_sheet_of(AboutFacts {
            backend: "warp",
            ..facts()
        });
        assert!(
            warp.rows.iter().any(|r| r.value == "warp"),
            "a machine that fell back to software rendering should be able to find that out here"
        );
    }

    #[test]
    fn the_dialog_content_carries_every_row_and_the_whole_assurance() {
        let sheet = about_sheet_of(facts());
        let content = dialog_content(&sheet);
        for row in &sheet.rows {
            assert!(content.contains(&row.label), "{} is missing", row.label);
            assert!(content.contains(&row.value), "{} is missing", row.value);
        }
        for line in &sheet.assurance {
            assert!(content.contains(line.as_str()));
        }
        assert!(
            content.contains("\n\n"),
            "the assurance is set off from the facts by a blank line"
        );
        assert!(
            !content.contains(&sheet.title),
            "the title is the dialog's main instruction, not part of the content"
        );
    }

    #[test]
    fn every_row_is_labelled_and_carries_something() {
        let sheet = about_sheet_of(facts());
        assert!(!sheet.rows.is_empty());
        for row in &sheet.rows {
            assert!(!row.label.is_empty(), "an unlabelled fact is not a fact");
            assert!(!row.value.is_empty(), "{} has no value", row.label);
        }
    }
}
