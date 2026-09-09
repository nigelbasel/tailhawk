//! The column chooser — which columns are shown, and in what order.
//!
//! **The owner's words, 2026-09-09:** *"this is a non standard way to show and hide columns, and so
//! should not be used, rather research standard ways to do this and suggest one"*, and then *"for
//! instance, a select columns dialog would be better"*. Dragging a boundary until the column
//! vanished was ours; a list of columns with tick boxes is Windows'. Explorer calls it *Choose
//! details*, the *List Views* page calls it a column chooser and says a list view with more than a
//! few columns should have one.
//!
//! **This file is the decision; `dialog.rs` shows it and `main.rs` acts on it.** [`rows_of`] turns
//! a layout into the list the dialog draws, in the order it draws it, and [`apply`] turns what came
//! back into widths and an order. Both are pure, so what the dialog does to a layout is a test and
//! not a screenshot.

use tailhawk_core::columns::Layout;

/// One line of the chooser: a column, whether it is ticked, and whether the tick can be cleared.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnRow {
    /// The layout column this names — **not** its position in the list, which changes as the user
    /// moves it.
    pub column: usize,
    pub title: String,
    pub shown: bool,
    /// **The message column cannot be hidden**, so its tick is fixed. A log viewer with the message
    /// turned off shows nothing at all, and an option whose only outcome is an empty window is one
    /// the guide would rather we did not offer.
    pub locked: bool,
}

/// The chooser's list for this layout: the shown columns first in display order, then the hidden
/// ones, and the message column last of all.
///
/// **Hidden columns are in the list, unticked** — that is the whole point of the dialog. They have
/// no place in the display order, since that order only names what is drawn, so they follow it in
/// the layout's own numbering, which is stable and is the order the user first saw them in.
pub fn rows_of(layout: &Layout) -> Vec<ColumnRow> {
    let last = layout.widths.len().saturating_sub(1);
    let shown = |c: usize| layout.widths.get(c).is_some_and(|w| *w > 0);
    let mut rows: Vec<ColumnRow> = Vec::with_capacity(layout.widths.len());
    for &column in layout.shown_order() {
        if shown(column) {
            rows.push(ColumnRow {
                column,
                title: layout.title(column).to_owned(),
                shown: true,
                locked: false,
            });
        }
    }
    for &column in layout.shown_order() {
        if !shown(column) {
            rows.push(ColumnRow {
                column,
                title: layout.title(column).to_owned(),
                shown: false,
                locked: false,
            });
        }
    }
    if last < layout.widths.len() {
        rows.push(ColumnRow {
            column: last,
            title: layout.title(last).to_owned(),
            shown: true,
            locked: true,
        });
    }
    rows
}

/// Moves the row at `at` one place up or down, reporting where it ended up.
///
/// **`None` means the move was refused**, and the caller leaves the list alone: the message column
/// neither moves nor is moved past, because `Layout` keeps it last by construction and a list that
/// said otherwise would be a promise the layout cannot keep.
///
/// *Accessibility*: "Don't make dragging the only way to perform an action." Reordering was a
/// header drag and nothing else; this is the keyboard's way to do it, and the buttons that drive it
/// are what a person who cannot drag reaches for.
pub fn move_row(rows: &mut [ColumnRow], at: usize, up: bool) -> Option<usize> {
    if at >= rows.len() || rows[at].locked {
        return None;
    }
    let to = if up {
        at.checked_sub(1)?
    } else {
        let next = at + 1;
        if next >= rows.len() || rows[next].locked {
            return None;
        }
        next
    };
    rows.swap(at, to);
    Some(to)
}

/// Applies the chooser's answer to the layout: the ticks become widths, the order becomes the
/// display order. Reports whether anything actually changed.
///
/// `defaults` is the width each column was measured at, so a column ticked back on comes back the
/// width it had rather than one cell. A column with no default is given its title's width, which is
/// the least that can show its own name.
///
/// **The message column is ignored wherever it appears in `rows`.** It is always last and always
/// shown; `Layout::order` names only the columns before it, and putting it in that list would make
/// `shown_order` reject the whole order as malformed.
pub fn apply(layout: &mut Layout, defaults: &[usize], rows: &[ColumnRow]) -> bool {
    let cells = tailhawk_core::cell::CellModel::default();
    let last = layout.widths.len().saturating_sub(1);
    let mut order: Vec<usize> = Vec::with_capacity(last);
    let mut widths = layout.widths.clone();
    for row in rows {
        if row.column >= last {
            continue;
        }
        order.push(row.column);
        let width = if row.shown {
            let measured = defaults.get(row.column).copied().unwrap_or(0);
            // **Cells, not characters.** Everything else that measures a column asks the cell
            // model — a wide character takes two cells — and counting `chars` would give a CJK
            // title half the room it needs, which `Layout::header` would then cut.
            measured
                .max(cells.cell_count(layout.title(row.column)))
                .max(1)
        } else {
            0
        };
        if widths.get(row.column) == Some(&0) || width == 0 {
            widths[row.column] = width;
        } else {
            // A column that was already shown keeps the width the user gave it: the chooser is
            // about *which* columns and in what order, and resizing one behind their back would
            // undo a drag they made deliberately.
            widths[row.column] = layout.widths[row.column];
        }
    }
    // Any column the dialog did not mention keeps its width and joins the end of the order, so a
    // list that somehow arrives short cannot drop a column out of the layout altogether.
    for column in 0..last {
        if !order.contains(&column) {
            order.push(column);
        }
    }
    if order == layout.order && widths == layout.widths {
        return false;
    }
    layout.order = order;
    layout.widths = widths;
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout_of(widths: Vec<usize>, order: Vec<usize>) -> Layout {
        // A real catalogue format, so the titles are the ones the dialog would draw. NLog has
        // timestamp, level, logger and the message, which is the shape this file cares
        // about: three that can be hidden and one that cannot.
        let format = tailhawk_core::format::by_id("nlog").expect("catalogue");
        Layout {
            format,
            widths,
            order,
            sort: None,
        }
    }

    /// The list the dialog draws: what is shown, in the order it is drawn, then what is hidden,
    /// then the message — which is ticked and cannot be unticked.
    #[test]
    fn the_list_is_shown_then_hidden_with_the_message_last_and_locked() {
        let layout = layout_of(vec![10, 0, 8, 0], vec![2, 0, 1]);
        let rows = rows_of(&layout);
        let names: Vec<&str> = rows.iter().map(|r| r.title.as_str()).collect();
        assert_eq!(names, ["logger", "timestamp", "level", "message"]);
        assert_eq!(
            rows.iter().map(|r| r.shown).collect::<Vec<_>>(),
            [true, true, false, true]
        );
        assert!(rows[3].locked, "the message column cannot be hidden");
        assert!(rows.iter().take(3).all(|r| !r.locked));
    }

    /// **Unticking is what hiding is now**, since the boundary drag that used to do it was the
    /// owner's "non standard way" and is gone.
    #[test]
    fn unticking_a_column_hides_it_and_ticking_it_brings_its_width_back() {
        let defaults = [10, 5, 8, 0];
        let mut layout = layout_of(vec![10, 5, 8, 0], vec![0, 1, 2]);
        let mut rows = rows_of(&layout);
        rows[1].shown = false;
        assert!(apply(&mut layout, &defaults, &rows));
        assert_eq!(layout.widths[1], 0, "hidden");

        let mut rows = rows_of(&layout);
        assert!(!rows[2].shown, "and it is in the list, unticked");
        rows[2].shown = true;
        assert!(apply(&mut layout, &defaults, &rows));
        assert_eq!(layout.widths[1], 5, "back at the width it was measured at");
    }

    /// Moving a row moves the column. The message column is not in the order the layout keeps —
    /// including it would make `shown_order` reject the order as malformed and silently fall back
    /// to the natural one, which reads as the dialog having done nothing.
    #[test]
    fn the_rows_order_becomes_the_display_order_without_the_message() {
        let defaults = [10, 5, 8, 0];
        let mut layout = layout_of(vec![10, 5, 8, 0], vec![0, 1, 2]);
        let mut rows = rows_of(&layout);
        let moved = rows.remove(2);
        rows.insert(0, moved);
        assert!(apply(&mut layout, &defaults, &rows));
        assert_eq!(layout.order, vec![2, 0, 1]);
        assert_eq!(layout.shown_order(), [2, 0, 1], "the layout accepts it");
    }

    /// A width the user dragged is theirs. The chooser changes which columns and in what order; a
    /// column that was already shown must come back the width it was, or every visit to the dialog
    /// would quietly undo their sizing.
    #[test]
    fn a_column_that_stays_shown_keeps_the_width_it_was_given() {
        let defaults = [10, 5, 8, 0];
        let mut layout = layout_of(vec![30, 5, 8, 0], vec![0, 1, 2]);
        let rows = rows_of(&layout);
        assert!(!apply(&mut layout, &defaults, &rows), "nothing changed");
        assert_eq!(layout.widths[0], 30, "not reset to the measured 10");
    }

    /// The move the buttons make, and the two it refuses: the message column does not move, and
    /// nothing moves past it. Everything else is a swap with the neighbour.
    #[test]
    fn a_row_moves_one_place_and_never_past_the_message() {
        let layout = layout_of(vec![10, 5, 8, 0], vec![0, 1, 2]);
        let mut rows = rows_of(&layout);
        assert_eq!(rows.len(), 4);
        assert_eq!(move_row(&mut rows, 0, true), None, "already first");
        assert_eq!(move_row(&mut rows, 1, true), Some(0));
        assert_eq!(
            rows.iter().map(|r| r.column).collect::<Vec<_>>(),
            [1, 0, 2, 3]
        );
        assert_eq!(
            move_row(&mut rows, 2, false),
            None,
            "the message column is last and stays last"
        );
        assert_eq!(move_row(&mut rows, 3, true), None, "and it does not move");
        assert_eq!(move_row(&mut rows, 9, true), None, "no such row");
    }

    /// A column the dialog never mentioned keeps its place rather than vanishing from the layout.
    #[test]
    fn a_column_missing_from_the_answer_is_kept_not_dropped() {
        let defaults = [10, 5, 8, 0];
        let mut layout = layout_of(vec![10, 5, 8, 0], vec![0, 1, 2]);
        let rows = vec![ColumnRow {
            column: 2,
            title: "Logger".to_owned(),
            shown: true,
            locked: false,
        }];
        apply(&mut layout, &defaults, &rows);
        assert_eq!(layout.order, vec![2, 0, 1]);
        assert_eq!(layout.widths, vec![10, 5, 8, 0], "widths untouched");
    }
}
