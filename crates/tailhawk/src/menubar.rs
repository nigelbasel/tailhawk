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
pub const ID_CLEAR_RECENT: u32 = 10_009;
/// File ▸ Open Recent's entries: `ID_RECENT_BASE + n` opens the n-th newest, for the shell to
/// resolve against the list it passed in. A range, because the entries are data, not commands.
pub const ID_RECENT_BASE: u32 = 10_100;

/// §2.2's seven menus, built from the command register each time the bar is opened.
///
/// **Rebuilt rather than cached**, per the model's own note: enablement is a property of the moment
/// — there is no document, or no selection, or no sort to clear — and a tree built afresh cannot
/// hold a stale answer. Seven menus of a dozen items costs nothing beside a frame.
///
/// An item that cannot act is **disabled, not hidden**, per §1.1 and §1.2's memorability rule: a
/// menu whose shape changes under the user is a menu they cannot learn.
pub fn menu_bar(
    doc: Option<&Document>,
    dark: bool,
    recent: &[String],
) -> tailhawk_core::menu::Menu {
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
    let saving = doc.is_some_and(|d| d.is_saving());

    Menu::bar(vec![
        Item::submenu(
            "&File",
            vec![
                cmd("&Open…", "Ctrl+O", Command::OpenFile),
                recent_menu(recent),
                on(cmd("&Close Tab", "Ctrl+W", Command::CloseTab), open),
                Item::separator(),
                on(cmd("&Export view…", "", Command::Export), open),
                on(cmd("&Keep saving…", "", Command::Tee), open),
                on(cmd("Stop sa&ving", "", Command::StopTee), saving),
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
                // §2.2 keeps a Font entry beside Preferences because a user looking for a font does
                // not think to look under Preferences. Both open the same sheet; this one opens it
                // on the font row.
                Item::command("&Font…", "", ID_FONT),
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
                Item::command("&Preferences…", "", ID_PREFS),
            ],
        ),
        Item::submenu(
            "&Help",
            vec![
                on(
                    Item::command("&Command palette", "Ctrl+K", ID_PALETTE),
                    open,
                ),
                // Both are available with no document open: a user who cannot remember how to open
                // a file is exactly the user who needs the keyboard map.
                Item::command("&Keyboard map", "", ID_KEYMAP),
                Item::separator(),
                Item::command("&About Tailhawk", "", ID_ABOUT),
            ],
        ),
    ])
}

/// File ▸ Open Recent: numbered entries newest first, then *Clear recent files* — and greyed
/// outright when there is no history, because an empty submenu is a dead end and a vanished one
/// breaks §1.2's memorability.
fn recent_menu(recent: &[String]) -> tailhawk_core::menu::Item {
    use tailhawk_core::menu::Item;
    if recent.is_empty() {
        return Item::submenu("Open &Recent", Vec::new()).disabled();
    }
    let mut items: Vec<Item> = recent
        .iter()
        .enumerate()
        .map(|(n, path)| Item::command(&recent_label(n, path), "", ID_RECENT_BASE + n as u32))
        .collect();
    items.push(Item::separator());
    items.push(Item::command("&Clear recent files", "", ID_CLEAR_RECENT));
    Item::submenu("Open &Recent", items)
}

/// One Open Recent entry: the customary numbered mnemonic — `&1` through `&9`, then `1&0` — and
/// the path. A literal `&` in the path is doubled so it draws as itself instead of underlining
/// the next letter.
fn recent_label(n: usize, path: &str) -> String {
    let shown = compact_path(path, RECENT_LABEL_CHARS).replace('&', "&&");
    match n {
        9 => format!("1&0  {shown}"),
        n => format!("&{}  {shown}", n + 1),
    }
}

/// What fits on a menu row comfortably, in characters.
const RECENT_LABEL_CHARS: usize = 48;

/// Elides the middle of a path that will not fit: the tail survives whole because the filename is
/// the part that identifies the entry, and enough head survives to say which drive and root.
fn compact_path(path: &str, max: usize) -> String {
    let chars: Vec<char> = path.chars().collect();
    if chars.len() <= max {
        return path.to_owned();
    }
    let head: String = chars[..6].iter().collect();
    let tail: String = chars[chars.len() - (max - 7)..].iter().collect();
    format!("{head}…{tail}")
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

/// What a click on `hit` does to the menu, and the command id it chose if it chose one.
///
/// **The decision lives here, apart from the shell, so it can be tested.** It used to be inline in
/// `Shell::menu_click`, where it needed a window and a `Shell` to exercise and so was never tested
/// at all — which is how the defect below survived being written.
///
/// A click on an item that cannot be chosen — disabled, or a separator — **does nothing**. That is
/// §1.1's rule and it is not automatic: `Menu::hover` declines to move onto a non-selectable item,
/// which is right on its own, but a handler that hovers and then calls `Menu::enter` inherits the
/// selection hover left alone and runs *that*. §2.2's Settings menu opens with `Dark theme`
/// highlighted, so clicking the greyed `Preferences…` toggled the theme. Every disabled item in
/// every menu carried the same hazard.
pub fn chosen_by_click(menu: &mut tailhawk_core::menu::Menu, hit: MenuHit) -> Option<u32> {
    match hit {
        MenuHit::Heading(i) => {
            // A second click on the open heading shuts it, as every menu bar does.
            if menu.is_open() && menu.open_path().first() == Some(&i) {
                menu.close();
            } else {
                menu.open_top(i);
            }
            None
        }
        MenuHit::Entry(i) => {
            let mut path = menu.open_path().to_vec();
            path.push(i);
            // The guard the defect above needs. Without it `hover` declines, the old selection
            // stands, and `enter` runs it.
            if !menu
                .item(&path)
                .is_some_and(tailhawk_core::menu::Item::selectable)
            {
                return None;
            }
            menu.hover(&path);
            menu.enter()
        }
    }
}

/// The hit a click at `(x, y)` landed on, if any.
///
/// Searched **last to first** so the open list wins over the bar row it overlaps. Nothing else
/// depends on the order the rects were pushed in.
/// Writes where the bar drew every heading and item, for `tools/verify-menus.ps1`.
///
/// **Only when `TAILHAWK_DUMP_MENU_HITS` names a file**, and it exists because a harness that clicks
/// menu items has to know where they are. Guessing a uniform row pitch does not work: a separator is
/// drawn shorter than an item, so the error accumulates down the list and the sweep ends up clicking
/// something other than what it reports. It clicked `Exit` while believing it had clicked a
/// separator, which is exactly the sort of false finding a sweep exists to avoid.
///
/// One line per rect: `kind index x0 y0 x1 y1`, client coordinates.
pub fn dump_hits(hits: &MenuHits) {
    let Some(path) = std::env::var_os("TAILHAWK_DUMP_MENU_HITS") else {
        return;
    };
    use std::io::Write;
    let Ok(mut f) = std::fs::File::create(path) else {
        return;
    };
    for (xs, ys, hit) in hits {
        let (kind, i) = match hit {
            MenuHit::Heading(i) => ("heading", *i),
            MenuHit::Entry(i) => ("entry", *i),
        };
        let _ = writeln!(
            f,
            "{kind} {i} {} {} {} {}",
            xs.start, ys.start, xs.end, ys.end
        );
    }
}

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
        let ids = menu_bar(None, false, &[]).ids();
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
        let menu = menu_bar(None, false, &[]);
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
        let menu = menu_bar(None, false, &[]);
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
        let mut menu = menu_bar(None, false, &[]);
        menu.open_top(0);
        assert!(menu.is_open(), "open_top did not open the menu");

        for _ in 0..5 {
            menu.rebuild(menu_bar(None, false, &[]));
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
        let mut menu = menu_bar(None, false, &[]);
        menu.focus();
        assert!(menu.is_focused(), "focus() did not focus the bar");

        for _ in 0..5 {
            menu.rebuild(menu_bar(None, false, &[]));
        }
        assert!(menu.is_focused(), "the rebuild dropped the focus");
        assert!(menu_frame_of(&menu).show_mnemonics);
    }

    /// **Clicking a disabled item chooses nothing — in every menu, not just the one reported.**
    ///
    /// The owner found that clicking the greyed `Preferences…` in Settings toggled the theme. The
    /// cause is a trap in composing two correct pieces: `Menu::hover` declines to move onto a
    /// non-selectable item, and a handler that then calls `Menu::enter` runs whatever was
    /// highlighted instead. Settings opens with `Dark theme` highlighted.
    ///
    /// This drives [`chosen_by_click`] — the real decision the shell makes — rather than a
    /// predicate invented alongside the fix. Remove the guard from that function and this fails.
    #[test]
    fn clicking_a_disabled_item_chooses_nothing() {
        let reference = menu_bar(None, false, &[]);
        for top in 0..reference.items().len() {
            let Some(items) = reference.at(&[top]).map(<[_]>::to_vec) else {
                continue;
            };
            for (i, item) in items.iter().enumerate() {
                if item.selectable() {
                    continue;
                }
                let mut menu = menu_bar(None, false, &[]);
                menu.open_top(top);
                let chosen = chosen_by_click(&mut menu, MenuHit::Entry(i));
                assert_eq!(
                    chosen,
                    None,
                    "clicking the disabled {:?} in {:?} chose a command",
                    item.text(),
                    reference.items()[top].text()
                );
            }
        }
    }

    /// The exact case the owner reported, named so a regression is recognisable rather than merely
    /// a failing assertion.
    ///
    /// **Originally written against the greyed `Preferences`**, which used to toggle the theme when
    /// clicked because the menu ran whatever it had highlighted rather than what was under the
    /// pointer. `Preferences` is built now, so the guard moved.
    ///
    /// **It moved to Rules and not to Edit, and the difference is whether the test can fail.** The
    /// bug ran the menu's *highlighted* entry, which is the first selectable one — so a menu whose
    /// every entry is disabled cannot exhibit it, and with no document open that is exactly what
    /// Edit is. Rules opens with `Highlight rules…` enabled above a disabled `Clear labels`, so
    /// removing the guard makes this return `EditRules`. Verified by doing that.
    #[test]
    fn clicking_a_greyed_item_chooses_nothing() {
        let mut menu = menu_bar(None, false, &[]);
        let rules = menu
            .items()
            .iter()
            .position(|i| i.text() == "Rules")
            .expect("a Rules menu");
        menu.open_top(rules);

        let items = menu.at(&[rules]).expect("Rules has items").to_vec();
        assert!(
            items
                .iter()
                .any(|i| i.selectable() && i.text().starts_with("Highlight rules")),
            "the test needs an enabled entry for the bug to have something to run"
        );
        let greyed = items
            .iter()
            .position(|i| i.text().starts_with("Clear labels"))
            .expect("a Clear labels item");
        assert!(
            !items[greyed].selectable(),
            "Clear labels needs a document, so with none it is greyed"
        );

        assert_eq!(
            chosen_by_click(&mut menu, MenuHit::Entry(greyed)),
            None,
            "a greyed item chose a command — without the guard this returns EditRules, the first \
             selectable entry, which is what the menu had highlighted"
        );
    }

    /// The four surfaces §2.2 used to draw disabled are built, and a menu that still greys them is
    /// a menu that hides working features.
    #[test]
    fn the_four_built_surfaces_are_no_longer_greyed() {
        let menu = menu_bar(None, false, &[]);
        for (top, label) in [
            ("Settings", "Preferences"),
            ("Help", "Keyboard map"),
            ("Help", "About Tailhawk"),
            ("Format", "Font"),
        ] {
            let at = menu
                .items()
                .iter()
                .position(|i| i.text() == top)
                .unwrap_or_else(|| panic!("a {top} menu"));
            let items = menu.at(&[at]).expect("items").to_vec();
            let item = items
                .iter()
                .find(|i| i.text().starts_with(label))
                .unwrap_or_else(|| panic!("{label} under {top}"));
            assert!(
                item.selectable(),
                "{label} is built now and must be reachable with no document open"
            );
        }
    }

    /// The other half: an enabled item still chooses its command, or the guard would deaden the
    /// whole menu and the first test would pass for the wrong reason.
    #[test]
    fn clicking_an_enabled_item_still_chooses_it() {
        let mut menu = menu_bar(None, false, &[]);
        menu.open_top(0); // File — `Open…` is never disabled.
        assert_eq!(
            chosen_by_click(&mut menu, MenuHit::Entry(0)),
            Some(command_id(Command::OpenFile))
        );
    }

    /// A click on Open Recent descends into it — and the per-frame rebuild does not throw the
    /// open submenu away, which is exactly how the live harness first saw this fail.
    #[test]
    fn clicking_open_recent_descends_and_survives_the_rebuild() {
        let paths = vec![r"C:\logs\a.log".to_owned()];
        let mut menu = menu_bar(None, false, &paths);
        menu.open_top(0);
        assert_eq!(
            chosen_by_click(&mut menu, MenuHit::Entry(1)),
            None,
            "a submenu opens rather than choosing"
        );
        assert_eq!(
            menu.open_path(),
            &[0, 1],
            "the submenu is the open list now"
        );
        let entries = menu.at(menu.open_path()).expect("the child list");
        assert!(entries[0].text().ends_with("a.log"));

        menu.rebuild(menu_bar(None, false, &paths));
        assert_eq!(
            menu.open_path(),
            &[0, 1],
            "the every-frame rebuild kept the open submenu"
        );
        let clear = menu.at(&[0, 1]).expect("child list").len() - 1;
        assert_eq!(
            chosen_by_click(&mut menu, MenuHit::Entry(0)),
            Some(ID_RECENT_BASE),
            "the first entry chooses the newest file"
        );
        let mut menu = menu_bar(None, false, &paths);
        menu.open_top(0);
        let _ = chosen_by_click(&mut menu, MenuHit::Entry(1));
        assert_eq!(
            chosen_by_click(&mut menu, MenuHit::Entry(clear)),
            Some(ID_CLEAR_RECENT)
        );
    }

    /// A click on a heading opens it, and a second click shuts it.
    #[test]
    fn clicking_a_heading_opens_it_and_clicking_again_shuts_it() {
        let mut menu = menu_bar(None, false, &[]);
        assert_eq!(chosen_by_click(&mut menu, MenuHit::Heading(1)), None);
        assert!(menu.is_open());
        assert_eq!(menu.open_path().first(), Some(&1));

        assert_eq!(chosen_by_click(&mut menu, MenuHit::Heading(1)), None);
        assert!(
            !menu.is_open(),
            "a second click on the open heading shuts it"
        );
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
            ID_CLEAR_RECENT,
            ID_RECENT_BASE,
            ID_RECENT_BASE + tailhawk_core::settings::RECENT_MAX as u32 - 1,
            ID_UNLISTED,
        ] {
            assert_eq!(command_of(id), None, "id {id} collides with the register");
        }
    }

    /// File ▸ Open Recent: greyed with no history, and with history it lists the files newest
    /// first under numbered mnemonics with *Clear recent files* at the bottom.
    #[test]
    fn the_file_menu_offers_recent_files_and_greys_the_empty_list() {
        let menu = menu_bar(None, false, &[]);
        let file = menu.at(&[0]).expect("File opens").to_vec();
        let recent = file
            .iter()
            .find(|i| i.text().starts_with("Open Recent"))
            .expect("an Open Recent entry under File");
        assert!(
            !recent.selectable(),
            "an empty history is greyed, not a dead end"
        );

        let paths = vec![
            r"C:\logs\newest.log".to_owned(),
            r"C:\logs\older.log".to_owned(),
        ];
        let menu = menu_bar(None, false, &paths);
        let at = menu
            .at(&[0])
            .expect("File opens")
            .iter()
            .position(|i| i.text().starts_with("Open Recent"))
            .expect("the entry is still where it was — §1.2's memorability");
        let entries = menu.at(&[0, at]).expect("the submenu opens").to_vec();
        assert!(
            entries[0].text().starts_with("1 ") && entries[0].text().ends_with("newest.log"),
            "the newest file is entry 1: {}",
            entries[0].text()
        );
        assert!(entries[1].text().ends_with("older.log"));
        assert!(
            entries
                .last()
                .expect("entries")
                .text()
                .contains("Clear recent files"),
            "the customary way out lives at the bottom"
        );
    }

    /// A path too long for a menu row loses its middle, never its filename.
    #[test]
    fn a_long_recent_path_keeps_its_filename() {
        let path = r"C:\a\deeply\nested\folder\structure\holding\seventy\characters\of\path\app-service.log";
        let label = recent_label(0, path);
        assert!(label.ends_with("app-service.log"), "{label}");
        assert!(label.contains('…'), "{label}");
        assert!(
            label.chars().count() <= RECENT_LABEL_CHARS + 5,
            "{label} is still too wide for a menu row"
        );
        let short = recent_label(1, r"C:\logs\b.log");
        assert!(!short.contains('…'), "a short path is shown whole");
        // The tenth entry's mnemonic is the zero, the customary MRU shape.
        assert!(recent_label(9, r"C:\x.log").starts_with("1&0"));
        // A literal ampersand draws as itself.
        assert!(recent_label(0, r"C:\a&b.log").contains("&&"));
    }

    /// A closed bar draws its headings and no entries; the mnemonics stay hidden until `Alt`.
    #[test]
    fn a_closed_bar_draws_headings_and_no_list() {
        let frame = menu_frame_of(&menu_bar(None, false, &[]));
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
        let mut menu = menu_bar(None, false, &[]);
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
        let mut menu = menu_bar(None, false, &[]);
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
            let mut menu = menu_bar(None, dark, &[]);
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
