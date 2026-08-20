//! §2.2's menu bar — the drawn surface over [`tailhawk_core::menu`].
//!
//! **Its own file, and that is a decision rather than tidiness.** `main.rs` is the Win32 shell and
//! is already very long; the menu is a self-contained surface whose only tie to the rest is a
//! `MenuFrame` handed to the document that draws it, exactly as the overlays are. Everything the
//! menu knows is here: what the six menus contain, how a command's id relates to the register, and
//! how the model becomes one frame's picture. Where the pixels go is `Document::draw_menu_bar`.

use crate::{Command, Document};

/// What a click on the menu bar landed on.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum MenuHit {
    /// A heading on the bar row: opens it, or shuts it if it is already down.
    Heading(usize),
    /// An item of the open list.
    Entry(usize),
}

/// One heading of the bar, as a frame draws it.
#[derive(Clone)]
pub struct MenuHeading {
    pub text: String,
    /// Where the underline goes, as a **character** offset into `text`; `None` when the label marks
    /// no mnemonic. Drawn only once `Alt` has been asked for, as Windows does.
    pub mnemonic: Option<usize>,
    pub open: bool,
    pub enabled: bool,
}

/// One item of the list hanging below an open heading.
#[derive(Clone)]
pub struct MenuEntry {
    pub text: String,
    pub accelerator: String,
    pub checked: bool,
    pub submenu: bool,
    pub separator: bool,
    pub enabled: bool,
    pub selected: bool,
}

/// §2.2's bar as one frame should draw it.
#[derive(Clone, Default)]
pub struct MenuFrame {
    pub headings: Vec<MenuHeading>,
    /// Which heading is down, when one is.
    pub open: Option<usize>,
    pub entries: Vec<MenuEntry>,
    /// `Alt` has been pressed: the underlines appear, and not before.
    pub show_mnemonics: bool,
}

/// The picture of `menu` for one frame.
///
/// Free rather than a method, for the reason `rules_overlay_of` is: it is pure mapping over the
/// model, it can be tested with no window, and it is the one place the drawn menu's reading of the
/// model is decided.
pub fn menu_frame_of(menu: &tailhawk_core::menu::Menu) -> MenuFrame {
    use tailhawk_core::menu::Kind;
    let selected = menu.selected();
    let open_top = menu.open_path().first().copied().filter(|_| menu.is_open());
    MenuFrame {
        headings: menu
            .items()
            .iter()
            .enumerate()
            .map(|(i, item)| MenuHeading {
                text: item.text(),
                mnemonic: item.mnemonic_at(),
                open: open_top == Some(i),
                enabled: item.selectable(),
            })
            .collect(),
        open: open_top,
        entries: match (menu.is_open(), menu.at(menu.open_path())) {
            (true, Some(items)) => items
                .iter()
                .enumerate()
                .map(|(i, item)| MenuEntry {
                    text: item.text(),
                    accelerator: item.accelerator.clone(),
                    checked: item.checked,
                    submenu: item.kind == Kind::Submenu,
                    separator: item.kind == Kind::Separator,
                    enabled: item.enabled,
                    selected: selected.len() == menu.open_path().len() + 1
                        && selected.last() == Some(&i),
                })
                .collect(),
            _ => Vec::new(),
        },
        // §2.2: `Alt` alone focuses the bar, and that is when the underlines appear.
        show_mnemonics: menu.is_focused() || menu.is_open(),
    }
}

/// A command's id inside a [`tailhawk_core::menu::Item`] — its index in `Command::LISTED`, the
/// register the palette draws from too.
///
/// **The register is the single source, and that is §1.2's memorability rule.** One command has one
/// name wherever it appears, and an id that *is* a position in that list is what lets `Menu::ids()`
/// be walked against it: §1.2's discoverability rule — every command appears in at least one menu —
/// becomes a failing test rather than an intention.
pub fn command_id(c: Command) -> u32 {
    Command::LISTED
        .iter()
        .position(|(listed, _, _)| *listed == c)
        .map_or(ID_UNLISTED, |at| at as u32)
}

/// The command an id names, or `None` for one of the menu's own items.
pub fn command_of(id: u32) -> Option<Command> {
    Command::LISTED.get(id as usize).map(|(c, _, _)| *c)
}

/// An item whose command the register does not list by that exact value — one carrying a payload,
/// like `GoToLine(n)`. Drawn and navigable; dispatch for these is the menu's own business.
pub const ID_UNLISTED: u32 = 9_000;

/// Ids for the menu's own items, the ones no other surface can reach. Above every register id.
pub const ID_EXIT: u32 = 10_001;
pub const ID_PALETTE: u32 = 10_002;
pub const ID_KEYMAP: u32 = 10_003;
pub const ID_ABOUT: u32 = 10_004;
/// Shown so the Edit menu reads as an Edit menu, and permanently disabled: Tailhawk is a viewer
/// and nothing in it edits a log. See the Edit menu in [`menu_bar`].
pub const ID_CUT: u32 = 10_005;
pub const ID_PASTE: u32 = 10_006;
/// Surfaces that are planned but not built. Disabled, never hidden.
pub const ID_FONT: u32 = 10_007;
pub const ID_PREFS: u32 = 10_008;

/// §2.2's seven menus, built from the command register each time the bar is opened.
///
/// **Rebuilt rather than cached**, per the model's own note: enablement is a property of the moment
/// — there is no document, or no selection, or no sort to clear — and a tree built afresh cannot
/// hold a stale answer. Seven menus of a dozen items costs nothing beside a frame.
///
/// An item that cannot act is **disabled, not hidden**, per §1.1 and §1.2's memorability rule: a
/// menu whose shape changes under the user is a menu they cannot learn.
pub fn menu_bar(doc: Option<&Document>, dark: bool) -> tailhawk_core::menu::Menu {
    use tailhawk_core::menu::{Item, Menu};
    let cmd = |label: &str, key: &str, c: Command| Item::command(label, key, command_id(c));
    let on = |item: Item, yes: bool| if yes { item } else { item.disabled() };
    let check = |label: &str, key: &str, c: Command, ticked: bool| {
        Item::check(label, key, command_id(c), ticked)
    };
    let open = doc.is_some();
    let selected = doc.is_some_and(|d| d.has_selection());
    let columns = doc.is_some_and(|d| d.has_columns());
    let following = doc.is_some_and(|d| d.is_following());
    let collapsed = doc.is_some_and(|d| d.is_collapsed());
    let invisibles = doc.is_some_and(|d| d.shows_invisibles());
    let detail = doc.is_some_and(|d| d.detail_open());
    let filtered = doc.is_some_and(|d| d.is_filtered());

    Menu::bar(vec![
        Item::submenu(
            "&File",
            vec![
                cmd("&Open…", "Ctrl+O", Command::OpenFile),
                on(cmd("&Close Tab", "Ctrl+W", Command::CloseTab), open),
                Item::separator(),
                on(cmd("&Export view…", "", Command::Export), open),
                on(cmd("&Keep saving…", "", Command::Tee), open),
                on(cmd("Stop sa&ving", "", Command::StopTee), open),
                Item::separator(),
                Item::command("E&xit", "Alt+F4", ID_EXIT),
            ],
        ),
        Item::submenu(
            "&Edit",
            vec![
                // **Cut and Paste are shown and permanently disabled**, and that is a deliberate
                // answer rather than an oversight. Tailhawk is a *viewer*: §5.1 opens files
                // read-only and nothing in it edits a log. Leaving the two out entirely makes the
                // Edit menu read as broken to anyone who has used a Windows application; showing
                // them greyed says "this program does not do that", which is the honest message and
                // the one §1.1's "never lie, never silently drop" asks for.
                Item::command("Cu&t", "Ctrl+X", ID_CUT).disabled(),
                on(cmd("&Copy", "Ctrl+C", Command::Copy), selected),
                Item::command("&Paste", "Ctrl+V", ID_PASTE).disabled(),
                on(
                    cmd("Copy as TS&V", "Ctrl+Shift+C", Command::CopyTsv),
                    selected && columns,
                ),
                Item::separator(),
                on(cmd("&Find…", "Ctrl+F", Command::Find), open),
                on(cmd("Find &next", "F3", Command::FindNext), open),
                on(
                    cmd("Find previo&us", "Shift+F3", Command::FindPrevious),
                    open,
                ),
                on(cmd("C&lear search", "Esc", Command::ClearSearch), open),
                Item::separator(),
                on(
                    cmd("Filter: &include", "Ctrl+L", Command::FilterInclude),
                    open,
                ),
                on(
                    cmd("Filter: e&xclude", "Ctrl+Shift+L", Command::FilterExclude),
                    open,
                ),
                on(cmd("Clear filte&rs", "", Command::ClearFilter), filtered),
                on(
                    cmd("&Edit last chip", "Ctrl+Shift+E", Command::EditLastChip),
                    filtered,
                ),
                Item::separator(),
                on(cmd("&Bookmark", "Ctrl+D", Command::ToggleBookmark), open),
                on(cmd("Next book&mark", "F2", Command::NextBookmark), open),
                on(
                    cmd("Previous bookmar&k", "Shift+F2", Command::PreviousBookmark),
                    open,
                ),
            ],
        ),
        Item::submenu(
            "&View",
            vec![
                on(
                    check("&Follow tail", "Ctrl+End", Command::FollowTail, following),
                    open,
                ),
                on(
                    check(
                        "&Collapse continuations",
                        "Ctrl+E",
                        Command::ToggleCollapse,
                        collapsed,
                    ),
                    open && columns,
                ),
                on(
                    check(
                        "&Invisibles",
                        "Ctrl+I",
                        Command::RevealInvisibles,
                        invisibles,
                    ),
                    open,
                ),
                on(
                    check(
                        "&Record detail",
                        "Ctrl+Enter",
                        Command::ToggleDetail,
                        detail,
                    ),
                    open,
                ),
                on(
                    cmd("&Pretty-print JSON", "", Command::TogglePretty),
                    open && detail,
                ),
                Item::separator(),
                on(cmd("Go to &top", "Ctrl+Home", Command::GoToTop), open),
                on(cmd("&Split pane", "Ctrl+\\", Command::Split), open),
                on(
                    cmd("Focus other pa&ne", "F6", Command::FocusOtherPane),
                    open,
                ),
                Item::separator(),
                on(cmd("Ne&xt tab", "Ctrl+Tab", Command::NextTab), open),
                on(
                    cmd("Pre&vious tab", "Ctrl+Shift+Tab", Command::PreviousTab),
                    open,
                ),
                Item::separator(),
                on(cmd("&Back", "Alt+←", Command::Back), open),
                on(cmd("F&orward", "Alt+→", Command::Forward), open),
            ],
        ),
        Item::submenu(
            "F&ormat",
            vec![
                on(cmd("&Log format…", "", Command::FormatMenu), open),
                on(cmd("&Define from a line…", "", Command::DefineFormat), open),
                on(cmd("&Import layout…", "", Command::ImportLayout), open),
                Item::separator(),
                on(cmd("&Reset columns", "", Command::ResetColumns), columns),
                on(cmd("Clear &sort", "", Command::ClearSort), columns),
                Item::separator(),
                // The font picker is not built. Shown disabled rather than omitted, so the shape of
                // the menu does not change the day it arrives — §1.2's memorability rule.
                Item::command("&Font…", "", ID_FONT).disabled(),
            ],
        ),
        Item::submenu(
            "&Rules",
            vec![
                cmd("&Highlight rules…", "Ctrl+H", Command::EditRules),
                cmd("&Open rules file", "", Command::OpenRules),
                cmd("&Reload rules", "", Command::ReloadRules),
                Item::separator(),
                on(
                    cmd("&Colour-label lines", "Ctrl+Shift+1…9", Command::Label(1)),
                    selected,
                ),
                on(
                    cmd("Clear &labels", "Ctrl+Shift+0", Command::ClearLabels),
                    open,
                ),
            ],
        ),
        Item::submenu(
            "&Settings",
            vec![
                check("&Dark theme", "", Command::ToggleTheme, dark),
                Item::separator(),
                Item::command("&Preferences…", "", ID_PREFS).disabled(),
            ],
        ),
        Item::submenu(
            "&Help",
            vec![
                on(
                    Item::command("&Command palette", "Ctrl+K", ID_PALETTE),
                    open,
                ),
                // Neither surface is built yet. **Disabled rather than absent**, per §1.1 and
                // §1.2's memorability rule: a menu that grows entries as they are implemented is a
                // menu nobody can learn, and an item that silently does nothing is worse than one
                // that says it cannot.
                Item::command("&Keyboard map", "", ID_KEYMAP).disabled(),
                Item::separator(),
                Item::command("&About Tailhawk", "", ID_ABOUT).disabled(),
            ],
        ),
    ])
}

/// What was drawn where, so a click can be resolved: an x-range, a y-range, and what is there.
///
/// **Both axes, unlike the command bar's hits.** The bar is one row, so x alone tells you which
/// heading was clicked — but the list below it is a *column* of rows that all share the same
/// x-range, and an x-only hit would resolve every one of them to the first. §6.2's ruler learned
/// the same lesson and carries both; this follows it.
pub type MenuHits = Vec<(std::ops::Range<f32>, std::ops::Range<f32>, MenuHit)>;

/// The bar row's height for a chrome line height.
///
/// Sized from the **chrome face** rather than the grid's row: §1.1 draws the chrome in the system
/// UI font, so a bar measured in grid rows is a band too tall for the text sitting in it.
pub fn bar_height(chrome_h: f32) -> f32 {
    (chrome_h + 6.0).round()
}

/// Draws §2.2's bar row at `top`. Returns its height and the x the open heading sits at, which is
/// where [`draw_open_list`] will hang the list from.
///
/// **The row and the list are two calls, and that is forced by paint order.** The list hangs down
/// over the tab strip, the command bar and the grid; whatever is drawn after it covers it. So the
/// caller draws this row first — it is part of the chrome band and the strip below it must not be
/// painted over — then everything else, then the list last of all.
pub fn draw_bar(
    painter: &mut tailhawk_core::paint::Painter,
    frame: &MenuFrame,
    hits: &mut MenuHits,
    top: f32,
    width: f32,
) -> (f32, f32) {
    use tailhawk_core::theme::theme;

    hits.clear();
    if frame.headings.is_empty() {
        return (0.0, 0.0);
    }
    let chrome_h = painter.chrome_line_height();
    let pad = painter.chrome_measure("n").max(4.0);
    let bar_h = bar_height(chrome_h);
    let text_y = top + ((bar_h - chrome_h) * 0.5).floor();

    painter.fill(0.0, top, width, bar_h, theme().chrome_bg);

    let mut x = pad * 0.5;
    let mut open_x = x;
    for (i, head) in frame.headings.iter().enumerate() {
        let label_w = painter.chrome_measure(&head.text);
        let box_w = label_w + pad * 2.0;
        if head.open {
            painter.fill(x, top, box_w, bar_h, theme().palette_selected_bg);
            open_x = x;
        }
        let ink = if head.enabled {
            theme().ink
        } else {
            theme().field_hint
        };
        painter.chrome_run(&head.text, x + pad, text_y, ink);
        // §2.2: the underlines appear once `Alt` has been asked for, and not before — a bar that
        // shows them always is noisier than the system's own and teaches nothing extra.
        if frame.show_mnemonics {
            if let Some(at) = head.mnemonic {
                let before: String = head.text.chars().take(at).collect();
                let letter: String = head.text.chars().skip(at).take(1).collect();
                let ux = x + pad + painter.chrome_measure(&before);
                let uw = painter.chrome_measure(&letter);
                painter.fill(ux, text_y + chrome_h - 1.0, uw, 1.0, ink);
            }
        }
        hits.push((x..x + box_w, top..top + bar_h, MenuHit::Heading(i)));
        x += box_w;
    }

    (bar_h, open_x)
}

/// The list under the open heading, if one is open. Drawn **last in the frame**, over everything
/// the chrome and the grid have put down — see [`draw_bar`].
pub fn draw_open_list(
    painter: &mut tailhawk_core::paint::Painter,
    frame: &MenuFrame,
    hits: &mut MenuHits,
    open_x: f32,
    top: f32,
    width: f32,
) {
    if frame.open.is_none() || frame.entries.is_empty() {
        return;
    }
    let chrome_h = painter.chrome_line_height();
    let pad = painter.chrome_measure("n").max(4.0);
    draw_list(painter, frame, hits, open_x, top, width, chrome_h, pad);
}

/// The list hanging below an open heading.
///
/// The three columns — tick gutter, label, accelerator — are each measured across **every** entry
/// and then shared, so the accelerators line up down the menu instead of ragging against their
/// labels. §1.2's learnability rule is what wants that: an accelerator is only teachable if it
/// reads as a column.
#[allow(clippy::too_many_arguments)]
fn draw_list(
    painter: &mut tailhawk_core::paint::Painter,
    frame: &MenuFrame,
    hits: &mut MenuHits,
    x: f32,
    top: f32,
    width: f32,
    chrome_h: f32,
    pad: f32,
) {
    use tailhawk_core::theme::theme;

    let row_h = (chrome_h + 4.0).round();
    let rule_h = (row_h * 0.5).round();
    // **U+25CF, not U+2713.** The chrome face is Segoe UI Variable Text, which has no check mark:
    // `the_chrome_face_has_the_markers_the_chrome_draws` fails on it, and a face with no per-glyph
    // fallback draws a missing glyph as a box. The rules editor reached the same dot for the same
    // reason.
    let tick = "\u{25CF}";
    let arrow = "\u{25BA}";
    let tick_w = painter
        .chrome_measure(tick)
        .max(painter.chrome_measure(arrow));
    let label_w = frame
        .entries
        .iter()
        .map(|e| painter.chrome_measure(&e.text))
        .fold(0.0f32, f32::max);
    let key_w = frame
        .entries
        .iter()
        .map(|e| painter.chrome_measure(&e.accelerator))
        .fold(0.0f32, f32::max);

    let box_w = (pad + tick_w + pad + label_w + pad * 3.0 + key_w + pad).min(width);
    // A menu opened under the rightmost heading would otherwise run off the edge; the system
    // slides it left until it fits, and so does this.
    let box_x = x.min((width - box_w).max(0.0));
    let box_h = frame
        .entries
        .iter()
        .map(|e| if e.separator { rule_h } else { row_h })
        .sum::<f32>()
        + 6.0;

    painter.fill(box_x, top, box_w, box_h, theme().palette_bg);
    // The command bar sits directly under this box and its text is already queued. Chrome text is
    // composited over every fill, so without this the search field's placeholder reads straight
    // through the open menu — see [`Painter::occlude_chrome`].
    painter.occlude_chrome(box_x, top, box_w, box_h);
    painter.fill(box_x, top, box_w, 1.0, theme().pane_edge);
    painter.fill(box_x, top + box_h - 1.0, box_w, 1.0, theme().pane_edge);
    painter.fill(box_x, top, 1.0, box_h, theme().pane_edge);
    painter.fill(box_x + box_w - 1.0, top, 1.0, box_h, theme().pane_edge);

    let mut y = top + 3.0;
    for (i, entry) in frame.entries.iter().enumerate() {
        if entry.separator {
            painter.fill(
                box_x + pad,
                y + (rule_h * 0.5).floor(),
                box_w - pad * 2.0,
                1.0,
                theme().pane_edge,
            );
            // A separator is still given a rect. It is not selectable, but a click that lands on
            // one must be swallowed by the menu rather than falling through to the grid behind it.
            hits.push((box_x..box_x + box_w, y..y + rule_h, MenuHit::Entry(i)));
            y += rule_h;
            continue;
        }
        if entry.selected {
            painter.fill(box_x, y, box_w, row_h, theme().palette_selected_bg);
        }
        let ink = if entry.enabled {
            theme().ink
        } else {
            theme().field_hint
        };
        let text_y = y + ((row_h - chrome_h) * 0.5).floor();
        if entry.checked {
            painter.chrome_run(tick, box_x + pad, text_y, ink);
        }
        painter.chrome_run(&entry.text, box_x + pad + tick_w + pad, text_y, ink);
        if entry.submenu {
            let ax = box_x + box_w - pad - painter.chrome_measure(arrow);
            painter.chrome_run(arrow, ax, text_y, theme().field_hint);
        } else if !entry.accelerator.is_empty() {
            let kx = box_x + box_w - pad - painter.chrome_measure(&entry.accelerator);
            // The accelerator is always the hint ink, even on a disabled row: it is teaching the
            // key, not offering the action.
            painter.chrome_run(&entry.accelerator, kx, text_y, theme().field_hint);
        }
        hits.push((box_x..box_x + box_w, y..y + row_h, MenuHit::Entry(i)));
        y += row_h;
    }
}

/// The hit a click at `(x, y)` landed on, if any.
///
/// Searched **last to first** so the open list wins over the bar row it overlaps. Nothing else
/// depends on the order the rects were pushed in.
pub fn hit_at(hits: &MenuHits, x: f32, y: f32) -> Option<MenuHit> {
    hits.iter()
        .rev()
        .find(|(xs, ys, _)| xs.contains(&x) && ys.contains(&y))
        .map(|(_, _, hit)| *hit)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `UI-DESIGN.md` §1.2's discoverability rule, as a test rather than an intention: **every
    /// command in the register appears in at least one menu.**
    ///
    /// This is the rule that stops the menu bar drifting behind the command palette. A command
    /// reachable only by a keystroke someone has to already know is a command that, for anyone who
    /// does not, is not there at all.
    #[test]
    fn every_listed_command_appears_in_at_least_one_menu() {
        let ids = menu_bar(None, false).ids();
        let missing: Vec<&str> = Command::LISTED
            .iter()
            .enumerate()
            .filter(|(at, _)| !ids.contains(&(*at as u32)))
            .map(|(_, (_, label, _))| *label)
            .collect();
        assert!(
            missing.is_empty(),
            "§1.2: these commands are in no menu: {missing:#?}"
        );
    }

    /// No two items of the same menu claim the same mnemonic letter.
    ///
    /// A collision is not cosmetic: `Menu::mnemonic` answers with the **first** match, so the
    /// second item becomes unreachable by the very key drawn underlined beneath it. Nine items were
    /// added to menus that already had mnemonics to satisfy §1.2's discoverability rule, and this
    /// is what stops that fix quietly breaking §1.2's usability one.
    #[test]
    fn no_two_items_in_one_menu_share_a_mnemonic() {
        let menu = menu_bar(None, false);
        let mut clashes: Vec<String> = Vec::new();

        let heads: Vec<Option<char>> = menu.items().iter().map(|i| i.mnemonic()).collect();
        for (a, letter) in heads.iter().enumerate() {
            for (b, other) in heads.iter().enumerate().skip(a + 1) {
                if letter.is_some() && letter == other {
                    clashes.push(format!("the bar: {a} and {b} both claim {letter:?}"));
                }
            }
        }

        for (top, head) in menu.items().iter().enumerate() {
            let Some(items) = menu.at(&[top]) else {
                continue;
            };
            let letters: Vec<(String, Option<char>)> = items
                .iter()
                .map(|i| (i.text(), i.mnemonic()))
                .filter(|(_, m)| m.is_some())
                .collect();
            for (a, (name, letter)) in letters.iter().enumerate() {
                for (other, mark) in letters.iter().skip(a + 1) {
                    if letter == mark {
                        clashes.push(format!(
                            "{}: {name:?} and {other:?} both claim {letter:?}",
                            head.text()
                        ));
                    }
                }
            }
        }
        assert!(clashes.is_empty(), "mnemonic collisions: {clashes:#?}");
    }

    /// Every item that can be chosen carries a mnemonic. One that does not is unreachable from the
    /// keyboard once the menu is open, which §1.2 requires to be a complete path.
    #[test]
    fn every_choosable_item_marks_a_mnemonic() {
        let menu = menu_bar(None, false);
        let mut bare: Vec<String> = Vec::new();
        for top in 0..menu.items().len() {
            let Some(items) = menu.at(&[top]) else {
                continue;
            };
            for item in items {
                let text = item.text();
                if !text.is_empty() && item.mnemonic().is_none() {
                    bare.push(text);
                }
            }
        }
        assert!(bare.is_empty(), "these items mark no mnemonic: {bare:#?}");
    }

    /// **An open menu survives the per-frame rebuild.** This is the exact sequence the shell runs:
    /// a click opens a heading, and then the very next frame rebuilds the tree from the register.
    /// If the rebuild dropped the open path the menu would shut within milliseconds of being
    /// opened — and on a tailing file, frames never stop, so it would never appear at all.
    #[test]
    fn an_open_menu_survives_the_rebuild_the_next_frame_does() {
        let mut menu = menu_bar(None, false);
        menu.open_top(0);
        assert!(menu.is_open(), "open_top did not open the menu");

        for _ in 0..5 {
            menu.rebuild(menu_bar(None, false));
        }
        assert!(menu.is_open(), "the rebuild shut the menu");

        let frame = menu_frame_of(&menu);
        assert_eq!(frame.open, Some(0), "the frame lost which menu is down");
        assert!(!frame.entries.is_empty(), "the frame drew an empty list");
    }

    /// The same for `Alt` alone: focus is a state the rebuild must carry, or the mnemonics flash
    /// for one frame and vanish.
    #[test]
    fn alt_focus_survives_the_rebuild_the_next_frame_does() {
        let mut menu = menu_bar(None, false);
        menu.focus();
        assert!(menu.is_focused(), "focus() did not focus the bar");

        for _ in 0..5 {
            menu.rebuild(menu_bar(None, false));
        }
        assert!(menu.is_focused(), "the rebuild dropped the focus");
        assert!(menu_frame_of(&menu).show_mnemonics);
    }

    /// The id a command is given round-trips back to the same command — the whole basis of routing
    /// a chosen menu item back through `Command::LISTED`.
    #[test]
    fn a_commands_id_round_trips_back_to_it() {
        for (command, label, _) in Command::LISTED {
            let id = command_id(*command);
            assert_ne!(id, ID_UNLISTED, "{label} is not in the register");
            assert_eq!(command_of(id), Some(*command), "{label} did not round-trip");
        }
    }

    /// The menu's own ids are outside the register's range, so `command_of` can never mistake one
    /// for a command — `ID_EXIT` resolving to whatever command happens to sit at that index would
    /// be a menu item that quietly does the wrong thing.
    #[test]
    fn the_menus_own_ids_are_not_register_positions() {
        for id in [
            ID_EXIT,
            ID_PALETTE,
            ID_KEYMAP,
            ID_ABOUT,
            ID_CUT,
            ID_PASTE,
            ID_FONT,
            ID_PREFS,
            ID_UNLISTED,
        ] {
            assert_eq!(command_of(id), None, "id {id} collides with the register");
        }
    }

    /// A closed bar draws its headings and no entries; the mnemonics stay hidden until `Alt`.
    #[test]
    fn a_closed_bar_draws_headings_and_no_list() {
        let frame = menu_frame_of(&menu_bar(None, false));
        assert_eq!(frame.headings.len(), 7);
        assert!(frame.entries.is_empty());
        assert!(frame.open.is_none());
        assert!(!frame.show_mnemonics);
        // The `&` markers are consumed into the mnemonic offset, never drawn.
        for head in &frame.headings {
            assert!(!head.text.contains('&'), "{} kept its marker", head.text);
            assert!(head.mnemonic.is_some(), "{} marks no mnemonic", head.text);
        }
    }

    /// Opening a heading fills the list from it, and asking for `Alt` reveals the underlines.
    #[test]
    fn opening_a_heading_fills_the_list_and_reveals_the_mnemonics() {
        let mut menu = menu_bar(None, false);
        menu.open_top(0);
        let frame = menu_frame_of(&menu);
        assert_eq!(frame.open, Some(0));
        assert!(!frame.entries.is_empty());
        assert!(frame.show_mnemonics);
        assert!(frame.headings[0].open);
    }

    /// With no document open, everything that needs one is disabled — and `Open…` is not, or the
    /// menu would offer no way back.
    #[test]
    fn with_no_document_the_file_menu_still_offers_open() {
        let mut menu = menu_bar(None, false);
        menu.open_top(0);
        let frame = menu_frame_of(&menu);
        let open = &frame.entries[0];
        assert!(open.text.starts_with("Open"), "got {:?}", open.text);
        assert!(open.enabled, "Open… must never be disabled");
        assert!(
            frame.entries.iter().any(|e| !e.enabled),
            "with no document, something should be disabled"
        );
    }

    /// The dark-theme item is ticked from the theme it was built with — the one piece of menu
    /// state that comes from neither the document nor the register.
    #[test]
    fn the_theme_item_is_ticked_to_match_the_theme() {
        for dark in [true, false] {
            let mut menu = menu_bar(None, dark);
            // Settings is the sixth heading.
            menu.open_top(5);
            let frame = menu_frame_of(&menu);
            let theme_item = frame
                .entries
                .iter()
                .find(|e| e.text.contains("Dark theme"))
                .expect("the Settings menu carries the theme toggle");
            assert_eq!(theme_item.checked, dark);
        }
    }

    /// A click resolves by **both** axes: rows of the open list share an x-range, so an x-only
    /// search would answer the first of them for a click on any.
    #[test]
    fn a_click_on_a_list_row_resolves_to_that_row_and_not_the_first() {
        let hits: MenuHits = vec![
            (0.0..40.0, 0.0..20.0, MenuHit::Heading(0)),
            (0.0..120.0, 20.0..40.0, MenuHit::Entry(0)),
            (0.0..120.0, 40.0..60.0, MenuHit::Entry(1)),
            (0.0..120.0, 60.0..80.0, MenuHit::Entry(2)),
        ];
        assert_eq!(hit_at(&hits, 10.0, 10.0), Some(MenuHit::Heading(0)));
        assert_eq!(hit_at(&hits, 10.0, 30.0), Some(MenuHit::Entry(0)));
        assert_eq!(hit_at(&hits, 10.0, 50.0), Some(MenuHit::Entry(1)));
        assert_eq!(hit_at(&hits, 10.0, 70.0), Some(MenuHit::Entry(2)));
        assert_eq!(hit_at(&hits, 200.0, 30.0), None);
        assert_eq!(hit_at(&hits, 10.0, 200.0), None);
    }

    /// The open list is searched before the bar row it hangs from, so a list that overlaps the bar
    /// still takes the click.
    #[test]
    fn the_open_list_wins_over_the_bar_row_it_overlaps() {
        let hits: MenuHits = vec![
            (0.0..40.0, 0.0..20.0, MenuHit::Heading(0)),
            (0.0..120.0, 10.0..30.0, MenuHit::Entry(4)),
        ];
        assert_eq!(hit_at(&hits, 10.0, 15.0), Some(MenuHit::Entry(4)));
    }

    /// The bar's height follows the chrome face, so it is right at any DPI — and is always at
    /// least tall enough for the text in it.
    #[test]
    fn the_bar_is_always_taller_than_its_own_text() {
        for chrome_h in [10.0, 12.5, 16.0, 24.0, 33.0] {
            assert!(bar_height(chrome_h) > chrome_h);
        }
    }
}
