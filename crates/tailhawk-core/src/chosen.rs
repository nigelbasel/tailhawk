//! The columns a reader chose for a tail — the Starberry request of 2026-10-27.
//!
//! A Loki window is a CLEF spill in which every label and every piece of structured metadata is
//! a flat top-level string key, and a record kind carries about thirty of them. Detection turns at
//! most six into columns, ranked by how many head rows carry them, and hides those until the
//! chooser ticks them back — so `status`, `method` and `path` never reached the chooser at all.
//! This module is the other half of `LOKI.md` §2's *"everything else becomes an `attributes`
//! entry and hence a column"*: which keys a window carries ([`keys_seen`]), which of them a tail
//! shows by default ([`default_columns`]), and the list the *Select columns* dialog draws for a
//! remote window and what it hands back ([`rows_of`], [`choice_of`]).
//!
//! **The record kind is `app` and `log_file_kind` together.** One `sb-gateway` tail mixes four
//! kinds of record told apart by `log_file_kind`, so the app alone no longer says what a row is;
//! [`kind_of`] names a row by both where the record says, and by the app alone where it does not.
//! Nothing here re-keys detection on the kind, because there is nothing per-kind to re-key — the
//! body is never parsed and every record's keys are read the same way; what the kind decides is
//! what the chooser says beside each key, so a reader sees that `method` is a
//! `sb-gateway/p2s-access` thing and `session_time` an `sb-edge/p2s-stream` one.
//!
//! All of it is pure: the shell samples the window, and acts on the answer.

use crate::columns::Layout;
use crate::detect::{json_fields, json_keys};
use crate::format::{LEVEL, MSG, TS};

/// The label a record's application is under, and the metadata key that tells one kind of record
/// from another within an application.
pub const APP_KEY: &str = "app";
pub const KIND_KEY: &str = "log_file_kind";

/// The columns a tail shows when the reader has chosen none, in this order, each kept only where
/// the window carries it — the seven the request names for the Starberry edge tails.
pub const DEFAULT_COLUMNS: [&str; 7] = [
    "host_name",
    "status",
    "method",
    "path",
    "client_addr",
    "rt",
    "trace_id",
];

/// One attribute key a window's records carry: on how many sampled rows, and under which record
/// kinds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeySeen {
    pub name: String,
    pub rows: usize,
    /// The record kinds — see [`kind_of`] — that carried this key, sorted, without repeats.
    pub kinds: Vec<String>,
}

/// What kind of record a row is: `app/log_file_kind` where the record says which file it came
/// from, the app alone where it does not, and nothing where it says neither.
pub fn kind_of(app: Option<&str>, log_file_kind: Option<&str>) -> Option<String> {
    match (app, log_file_kind) {
        (Some(app), Some(kind)) => Some(format!("{app}/{kind}")),
        (Some(app), None) => Some(app.to_owned()),
        (None, Some(kind)) => Some(kind.to_owned()),
        (None, None) => None,
    }
}

/// Every text-valued top-level key the sampled lines carry, by name, with the rows and the record
/// kinds that carry it. The three keys the grid understands — the timestamp, level and message —
/// are not attributes and are left out, as is Loki's own `__`-prefixed bookkeeping. A line that
/// is not a JSON object contributes nothing.
pub fn keys_seen(lines: &[String]) -> Vec<KeySeen> {
    let mut seen: Vec<KeySeen> = Vec::new();
    for line in lines {
        let Some(keys) = json_keys(line) else {
            continue;
        };
        let kind = json_fields(line, &[APP_KEY, KIND_KEY]).and_then(|fields| {
            let text = |i: usize| fields.get(i).cloned().flatten().map(|r| &line[r]);
            kind_of(text(0), text(1))
        });
        for key in keys {
            if !key.is_text()
                || crate::format::understood_key(&key.name).is_some()
                || key.name.starts_with("__")
            {
                continue;
            }
            let entry = match seen.iter_mut().find(|s| s.name == key.name) {
                Some(entry) => entry,
                None => {
                    seen.push(KeySeen {
                        name: key.name.clone(),
                        rows: 0,
                        kinds: Vec::new(),
                    });
                    seen.last_mut().expect("just pushed")
                }
            };
            entry.rows += 1;
            if let Some(kind) = kind.as_deref() {
                if !entry.kinds.iter().any(|k| k == kind) {
                    entry.kinds.push(kind.to_owned());
                }
            }
        }
    }
    for entry in &mut seen {
        entry.kinds.sort();
    }
    seen.sort_by(|a, b| a.name.cmp(&b.name));
    seen
}

/// The columns a tail with no choice of its own shows: [`DEFAULT_COLUMNS`] in that order, each
/// only where some record in the window carries it. Empty when the window carries none of them,
/// which is the shell's cue to leave the window to detection as before.
pub fn default_columns(seen: &[KeySeen]) -> Vec<String> {
    DEFAULT_COLUMNS
        .iter()
        .filter(|name| seen.iter().any(|s| s.name == **name))
        .map(|name| (*name).to_owned())
        .collect()
}

/// One line of the chooser for a remote window: an attribute key, or one of the three the grid
/// understands, whose `name` is the raw key (`@t`, `@l`, `@m`, or the attribute).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub name: String,
    /// What the list shows — the column's heading as the header draws it.
    pub title: String,
    /// Which record kinds carry this key, or what to say when none in the window does. Blank for
    /// the three understood columns.
    pub seen_on: String,
    pub shown: bool,
    /// The message column cannot be hidden.
    pub locked: bool,
}

/// What the chooser said back, read by [`choice_of`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Choice {
    /// The attribute keys ticked, in list order — what the tail remembers.
    pub columns: Vec<String>,
    /// The display order the list asked for, as indices into the columns of
    /// [`format::json_chosen`](crate::format::json_chosen) over `columns`: the timestamp is `0`,
    /// the level `1`, the `k`th chosen key `2 + k`. Every column before the message is named once.
    pub order: Vec<usize>,
    pub timestamp_shown: bool,
    pub level_shown: bool,
}

/// What a chosen key not in the window is said to be seen on.
pub const NOT_IN_WINDOW: &str = "not in what was fetched";

/// The chooser's list for a remote window under `layout` — a layout over
/// [`format::json_chosen`](crate::format::json_chosen): its columns in display order, each ticked
/// as it is shown, then every key the window carries that is not a column, unticked and in name
/// order, and the message last. A chosen key that no sampled record carries is still listed,
/// ticked, and says so — the choice is the reader's, not the window's.
pub fn rows_of(layout: &Layout, seen: &[KeySeen]) -> Vec<Row> {
    let last = layout.widths.len().saturating_sub(1);
    let seen_on = |name: &str| {
        seen.iter()
            .find(|s| s.name == name)
            .map(|s| s.kinds.join(", "))
            .unwrap_or_else(|| NOT_IN_WINDOW.to_owned())
    };
    let mut rows: Vec<Row> = Vec::with_capacity(layout.widths.len() + seen.len());
    // **A chosen column is ticked whatever its width.** The window's fit hides a trailing column
    // that does not fit the row, and a tick read off the width would then drop it from the choice
    // the next OK writes back — a key the reader chose, lost to the width of the window on the
    // day. Only a layout detection built — one that invented its columns — is read by width,
    // because there a hidden column is one nobody chose.
    let invented = layout.format.has_invented_columns();
    for &column in layout.shown_order() {
        let capture = layout.format.columns.get(column).copied().unwrap_or("");
        let shown = layout.widths.get(column).is_some_and(|w| *w > 0);
        let (name, understood) = match capture {
            TS => ("@t".to_owned(), true),
            LEVEL => ("@l".to_owned(), true),
            _ => (layout.title(column).to_owned(), false),
        };
        rows.push(Row {
            seen_on: if understood {
                String::new()
            } else {
                seen_on(&name)
            },
            title: layout.display_title(column),
            name,
            shown: shown || !(understood || invented),
            locked: false,
        });
    }
    for key in seen {
        if rows.iter().any(|row| row.name == key.name) {
            continue;
        }
        rows.push(Row {
            name: key.name.clone(),
            title: key.name.clone(),
            seen_on: key.kinds.join(", "),
            shown: false,
            locked: false,
        });
    }
    if last < layout.widths.len() {
        rows.push(Row {
            name: "@m".to_owned(),
            title: layout.display_title(last),
            seen_on: String::new(),
            shown: true,
            locked: true,
        });
    }
    rows
}

/// What the list came back as: the ticked attribute keys in order, the display order of every
/// column, and whether the timestamp and level — always columns, so a hidden level still has a
/// severity for the rules — are shown. The message row is ignored wherever it sits.
pub fn choice_of(rows: &[Row]) -> Choice {
    let mut choice = Choice {
        columns: Vec::new(),
        order: Vec::new(),
        timestamp_shown: true,
        level_shown: true,
    };
    for row in rows {
        match row.name.as_str() {
            "@t" => {
                choice.timestamp_shown = row.shown;
                choice.order.push(0);
            }
            "@l" => {
                choice.level_shown = row.shown;
                choice.order.push(1);
            }
            "@m" => {}
            name if row.shown => {
                choice.order.push(2 + choice.columns.len());
                choice.columns.push(name.to_owned());
            }
            _ => {}
        }
    }
    for understood in [0usize, 1] {
        if !choice.order.contains(&understood) {
            choice.order.insert(understood, understood);
        }
    }
    choice
}

/// Gives every chosen column a width, where the sample gave it none.
///
/// [`Layout::from_sample`] leaves out a column no sampled line populated, which is right for a
/// format that merely found its keys and wrong for one a reader chose: an edge-only window has no
/// `method` on any row, and the request asks for that column blank *"without disturbing the
/// layout"*, not gone. The timestamp and level keep the sample's answer, so a window with no
/// level still shows none; a chosen column is shown at least at its title's width, which is the
/// least that can show its own name. Reports whether anything changed.
pub fn widen_to_titles(layout: &mut Layout) -> bool {
    let cells = crate::cell::CellModel::new();
    let last = layout.widths.len().saturating_sub(1);
    let mut changed = false;
    for column in 0..last {
        let capture = layout.format.columns.get(column).copied().unwrap_or("");
        if matches!(capture, TS | LEVEL | MSG) || layout.widths[column] > 0 {
            continue;
        }
        layout.widths[column] = cells.cell_count(&layout.display_title(column)).max(1);
        changed = true;
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::json_chosen;

    const ACCESS: &str = r#"{"@t":"2026-10-27T11:02:37Z","@l":"warn","__stream_shard__":"1","app":"sb-gateway","client_addr":"203.0.113.9","host_name":"gate-fw-a","log_file_kind":"p2s-access","method":"GET","path":"/","rt":"0.000","status":"444","@m":"GET host/ -> 444"}"#;
    const STREAM: &str = r#"{"@t":"2026-10-27T11:02:37Z","@l":"info","app":"sb-edge","bytes_in":"717","client_addr":"203.0.113.9","host_name":"ip-edge-web107","log_file_kind":"p2s-stream","session_time":"0.384","status":"200","@m":"203.0.113.9:62220 -> :443"}"#;
    const JOURNAL: &str = r#"{"@t":"2026-10-27T11:02:38Z","@l":"info","app":"sb-gateway","host_name":"gate-fw-a","identifier":"sshd","pid":"812","unit":"ssh.service","@m":"Accepted publickey"}"#;

    fn lines(text: &[&str]) -> Vec<String> {
        text.iter().map(|s| (*s).to_owned()).collect()
    }

    fn seen_named(seen: &[KeySeen], name: &str) -> KeySeen {
        seen.iter()
            .find(|s| s.name == name)
            .cloned()
            .unwrap_or_else(|| panic!("{name} should have been seen"))
    }

    /// Every attribute key the window carries is listed once, by name, with the rows and the
    /// kinds that carry it; the understood keys and Loki's own bookkeeping are not attributes.
    #[test]
    fn the_keys_a_window_carries_are_counted_by_name_and_told_apart_by_kind() {
        let seen = keys_seen(&lines(&[ACCESS, STREAM, JOURNAL, "not json", ACCESS]));
        let names: Vec<&str> = seen.iter().map(|s| s.name.as_str()).collect();
        assert!(names.windows(2).all(|w| w[0] < w[1]), "sorted: {names:?}");
        assert!(
            !names.contains(&"@t") && !names.contains(&"@m"),
            "{names:?}"
        );
        assert!(!names.contains(&"__stream_shard__"), "{names:?}");
        assert_eq!(
            seen_named(&seen, "status"),
            KeySeen {
                name: "status".to_owned(),
                rows: 3,
                kinds: vec![
                    "sb-edge/p2s-stream".to_owned(),
                    "sb-gateway/p2s-access".to_owned()
                ],
            }
        );
        assert_eq!(seen_named(&seen, "method").rows, 2);
        assert_eq!(seen_named(&seen, "method").kinds, ["sb-gateway/p2s-access"]);
        assert_eq!(
            seen_named(&seen, "unit").kinds,
            ["sb-gateway"],
            "a journal record says no log_file_kind, so its kind is the app alone"
        );
    }

    #[test]
    fn a_record_kind_is_the_app_and_the_file_kind_together_where_both_are_said() {
        assert_eq!(
            kind_of(Some("sb-gateway"), Some("p2s-access")).as_deref(),
            Some("sb-gateway/p2s-access")
        );
        assert_eq!(
            kind_of(Some("sb-gateway"), None).as_deref(),
            Some("sb-gateway")
        );
        assert_eq!(
            kind_of(None, Some("nginx-error")).as_deref(),
            Some("nginx-error")
        );
        assert_eq!(kind_of(None, None), None);
    }

    /// The defaults are the seven the request names, in its order, and only those the window
    /// carries: an edge-only window has no `method` or `path` and does not get blank columns for
    /// them by default; a window with none of the seven gets no default at all.
    #[test]
    fn the_defaults_are_the_requests_seven_kept_where_the_window_carries_them() {
        let both = keys_seen(&lines(&[ACCESS, STREAM]));
        assert_eq!(
            default_columns(&both),
            ["host_name", "status", "method", "path", "client_addr", "rt"]
        );
        let edge = keys_seen(&lines(&[STREAM]));
        assert_eq!(
            default_columns(&edge),
            ["host_name", "status", "client_addr"]
        );
        let journal = keys_seen(&lines(&[JOURNAL]));
        assert_eq!(default_columns(&journal), ["host_name"]);
        let serilog = keys_seen(&lines(&[
            r#"{"@t":"a","app":"identity","scope_name":"Token","@m":"m"}"#,
        ]));
        assert!(default_columns(&serilog).is_empty());
    }

    fn layout(chosen: &[&str]) -> Layout {
        let keys: Vec<String> = chosen.iter().map(|s| (*s).to_owned()).collect();
        let mut layout = Layout::from_sample(json_chosen(&keys), &lines(&[ACCESS, STREAM]));
        widen_to_titles(&mut layout);
        layout
    }

    /// The list is the layout's columns in display order, ticked as shown, then every other key
    /// the window carries unticked and in name order, then the message, locked.
    #[test]
    fn the_rows_are_the_columns_in_order_then_the_rest_of_the_window_then_the_message() {
        let mut layout = layout(&["status", "path"]);
        layout.move_column(1, 2);
        layout.widths[0] = 0;
        let seen = keys_seen(&lines(&[ACCESS, STREAM]));
        let rows = rows_of(&layout, &seen);
        let names: Vec<&str> = rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "@t",
                "status",
                "@l",
                "path",
                "app",
                "bytes_in",
                "client_addr",
                "host_name",
                "log_file_kind",
                "method",
                "rt",
                "session_time",
                "@m"
            ]
        );
        assert_eq!(rows[0].title, "Timestamp");
        assert!(!rows[0].shown, "a hidden timestamp is listed unticked");
        assert!(rows[1].shown && rows[2].shown && rows[3].shown);
        assert_eq!(rows[1].seen_on, "sb-edge/p2s-stream, sb-gateway/p2s-access");
        assert_eq!(rows[3].seen_on, "sb-gateway/p2s-access");
        assert!(rows[4..12].iter().all(|r| !r.shown));
        assert_eq!(rows[9].name, "method");
        assert_eq!(rows[9].seen_on, "sb-gateway/p2s-access");
        let message = rows.last().expect("the message row");
        assert!(message.locked && message.shown);
        assert_eq!(message.title, "Message");
    }

    /// A chosen column the window's fit hid for want of room is still the reader's choice, and is
    /// listed ticked; a hidden timestamp is not, and a column detection merely found is not.
    #[test]
    fn a_chosen_column_hidden_by_the_fit_is_still_ticked() {
        let mut layout = layout(&["status", "path"]);
        layout.widths[3] = 0;
        layout.widths[0] = 0;
        let rows = rows_of(&layout, &[]);
        let path = rows.iter().find(|r| r.name == "path").expect("listed");
        assert!(path.shown, "chosen, so ticked, whatever the fit did");
        assert!(!rows[0].shown, "the timestamp reads its width");

        let found = Layout::from_sample(
            crate::format::json_lines(
                &crate::detect::json_template(&lines(&[ACCESS, STREAM])).expect("a template"),
            ),
            &lines(&[ACCESS, STREAM]),
        );
        let mut found = found;
        found.only_understood();
        let rows = rows_of(&found, &[]);
        assert!(
            rows.iter()
                .filter(|r| !r.locked)
                .all(|r| !r.shown || r.name == "@t" || r.name == "@l"),
            "a column detection found and hid is nobody's choice: {rows:?}"
        );
    }

    /// The understood rows are named by their role, not their title — the catalogue's `ndjson`
    /// reader, which a spill with no extra keys lands on, titles them `timestamp` and `level`,
    /// and a choice that took those for attribute keys would write them to the tail.
    #[test]
    fn the_understood_rows_are_named_by_role_whatever_the_format_titles_them() {
        let format = crate::format::by_id("ndjson").expect("catalogue");
        let layout = Layout::from_sample(format, &lines(&[ACCESS]));
        let mut rows = rows_of(&layout, &keys_seen(&lines(&[ACCESS])));
        assert_eq!(rows[0].name, "@t");
        assert_eq!(rows[1].name, "@l");
        assert_eq!(rows[0].title, "Timestamp");
        rows[0].shown = false;
        let choice = choice_of(&rows);
        assert!(choice.columns.is_empty());
        assert!(!choice.timestamp_shown);
        assert_eq!(choice.order, [0, 1]);
    }

    /// A chosen key no record in the window carries is still a column and still listed, ticked,
    /// and says where it stands — the choice is the reader's.
    #[test]
    fn a_chosen_key_the_window_lacks_is_still_listed_ticked_and_says_so() {
        let layout = layout(&["status", "referer"]);
        let seen = keys_seen(&lines(&[ACCESS, STREAM]));
        let rows = rows_of(&layout, &seen);
        let referer = rows.iter().find(|r| r.name == "referer").expect("listed");
        assert!(referer.shown);
        assert_eq!(referer.seen_on, NOT_IN_WINDOW);
        assert!(
            layout.widths[3] > 0,
            "widen_to_titles gives a chosen column the sample never populated its title's width"
        );
    }

    /// The choice is the ticked keys in list order, with the understood columns' ticks and the
    /// display order every column before the message is named in once — a level moved beneath
    /// `status` stays there, and an unticked level is still a column, hidden.
    #[test]
    fn the_choice_is_the_ticked_keys_in_order_and_the_order_names_every_column_once() {
        let layout = layout(&["status", "path"]);
        let seen = keys_seen(&lines(&[ACCESS, STREAM]));
        let mut rows = rows_of(&layout, &seen);
        rows[1].shown = false;
        rows.swap(1, 2);
        let method = rows
            .iter()
            .position(|r| r.name == "method")
            .expect("listed");
        rows[method].shown = true;
        let at = rows.iter().position(|r| r.name == "path").expect("listed");
        rows[at].shown = false;
        let choice = choice_of(&rows);
        assert_eq!(choice.columns, ["status", "method"]);
        assert!(choice.timestamp_shown);
        assert!(!choice.level_shown);
        assert_eq!(choice.order, [0, 2, 1, 3]);
        let mut named = choice.order.clone();
        named.sort_unstable();
        assert_eq!(named, [0, 1, 2, 3]);
    }

    /// A list that lost its understood rows — a chooser built for a window without them — still
    /// answers an order that names them, so the layout never refuses it.
    #[test]
    fn an_order_always_names_the_timestamp_and_the_level() {
        let rows = vec![Row {
            name: "status".to_owned(),
            title: "status".to_owned(),
            seen_on: String::new(),
            shown: true,
            locked: false,
        }];
        assert_eq!(choice_of(&rows).order, [0, 1, 2]);
    }
}
