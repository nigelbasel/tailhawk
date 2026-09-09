//! §2.2's menu bar — the drawn surface over [`tailhawk_core::menu`].
//!
//! **Its own file, and that is a decision rather than tidiness.** `main.rs` is the Win32 shell and
//! is already very long; the menu is a self-contained surface whose only tie to the rest is a
//! `MenuFrame` handed to the document that draws it, exactly as the overlays are. Everything the
//! menu knows is here: what the six menus contain, how a command's id relates to the register, and
//! how the model becomes one frame's picture. Where the pixels go is `Document::draw_menu_bar`.

use windows::core::PCWSTR;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreateMenu, CreatePopupMenu, DeleteMenu, DestroyMenu, GetMenuItemCount,
    TrackPopupMenu, HMENU, MF_BYPOSITION, MF_CHECKED, MF_GRAYED, MF_POPUP, MF_SEPARATOR, MF_STRING,
    TPM_RETURNCMD, TPM_RIGHTBUTTON, TRACK_POPUP_MENU_FLAGS,
};

use crate::{Command, Document};

/// The bar's [`tailhawk_core::menu::Item`] tree as a **real Win32 menu** — §2.2 as resettled by
/// the owner: Windows draws the menus, tracks the mouse, walks the keyboard and answers the
/// screen reader; this maps the tested content into them and nothing more.
pub fn build_bar(menu: &tailhawk_core::menu::Menu) -> Option<HMENU> {
    let bar = unsafe { CreateMenu() }.ok()?;
    for top in menu.items() {
        append_item(bar, top);
    }
    Some(bar)
}

/// Replaces `popup`'s contents with the current tree's menu `top` — the `WM_INITMENUPOPUP`
/// moment, which is how a real menu shows this instant's enablement, checks, recent files and
/// format candidates without being rebuilt while it is on screen.
pub fn refill_popup(popup: HMENU, menu: &tailhawk_core::menu::Menu, top: usize) {
    unsafe {
        while GetMenuItemCount(popup) > 0 {
            let _ = DeleteMenu(popup, 0, MF_BYPOSITION);
        }
    }
    if let Some(items) = menu.at(&[top]) {
        for child in items {
            append_item(popup, child);
        }
    }
}

/// §2.4's context menus, as the real thing: the same `Item` tree tracked by `TrackPopupMenu` at
/// screen `(x, y)`, returning the chosen id directly — `TPM_RETURNCMD`, so the caller dispatches
/// without a `WM_COMMAND` round-trip. The menu pumps its own modal loop; the caller must hold no
/// borrow across this call.
pub fn track_context(
    hwnd: HWND,
    items: &[tailhawk_core::menu::Item],
    x: i32,
    y: i32,
) -> Option<u32> {
    let popup = unsafe { CreatePopupMenu() }.ok()?;
    for item in items {
        append_item(popup, item);
    }
    let chosen = unsafe {
        TrackPopupMenu(
            popup,
            TRACK_POPUP_MENU_FLAGS(TPM_RETURNCMD.0 | TPM_RIGHTBUTTON.0),
            x,
            y,
            0,
            hwnd,
            None,
        )
    };
    unsafe {
        let _ = DestroyMenu(popup);
    }
    let id = chosen.0 as u32;
    (id != 0).then_some(id)
}

fn append_item(into: HMENU, item: &tailhawk_core::menu::Item) {
    use tailhawk_core::menu::Kind;
    let wide = |s: &str| -> Vec<u16> { s.encode_utf16().chain(std::iter::once(0)).collect() };
    unsafe {
        match item.kind {
            Kind::Separator => {
                let _ = AppendMenuW(into, MF_SEPARATOR, 0, PCWSTR::null());
            }
            Kind::Submenu => {
                let Ok(popup) = CreatePopupMenu() else { return };
                for child in &item.items {
                    append_item(popup, child);
                }
                let mut flags = MF_POPUP | MF_STRING;
                if !item.enabled {
                    flags |= MF_GRAYED;
                }
                let label = wide(&item.label);
                let _ = AppendMenuW(into, flags, popup.0 as usize, PCWSTR(label.as_ptr()));
            }
            Kind::Command | Kind::Check => {
                let mut flags = MF_STRING;
                if !item.enabled {
                    flags |= MF_GRAYED;
                }
                if item.checked {
                    flags |= MF_CHECKED;
                }
                let label = if item.accelerator.is_empty() {
                    item.label.clone()
                } else {
                    format!("{}\t{}", item.label, item.accelerator)
                };
                let label = wide(&label);
                let _ = AppendMenuW(
                    into,
                    flags,
                    item.id.unwrap_or(ID_UNLISTED) as usize,
                    PCWSTR(label.as_ptr()),
                );
            }
        }
    }
}
/// A command's id inside a [`tailhawk_core::menu::Item`] — its index in `Command::LISTED`, the
/// register the keyboard map is generated from too.
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
/// `Go to line…`. Not a register position, because [`Command::GoToLine`] carries the line it goes
/// to and the register lists commands by value — the same reason `ID_UNLISTED` exists.
pub const ID_GOTO: u32 = 10_010;
/// The toolbar's `Open remote` button. **It names no command**: the button drops
/// [`crate::toolbar::remote_menu_of`]'s menu and never sends this id anywhere, so it exists only to
/// tell one button apart from the rest in `TBN_DROPDOWN`. What the user chooses is a source id or
/// `EditSources`, which are commands the register already knows.
pub const ID_SOURCE_MENU: u32 = 10_011;
/// File's recent entries: `ID_RECENT_BASE + n` opens the n-th newest, for the shell to resolve
/// against the list it passed in. A range, because the entries are data, not commands.
pub const ID_RECENT_BASE: u32 = 10_100;
/// Format ▸ Log format's rows, the same way: `ID_FORMAT_BASE + n` is the n-th row of
/// [`crate::format_menu_of`]'s answer, resolved back through it when chosen.
pub const ID_FORMAT_BASE: u32 = 10_200;
/// `LOKI.md`'s remote sources: `ID_SOURCE_BASE + n` opens the n-th configured source. A range,
/// because the sources are data the user named rather than commands the register knows.
pub const ID_SOURCE_BASE: u32 = 10_400;
/// §2.4's context-menu items — the ones whose subject is *where the menu was summoned* (a column,
/// a selection, a panel row) rather than anything the id alone could name. The caller resolves the
/// subject before tracking and dispatches these against it.
pub const ID_CTX_SORT_ASC: u32 = 10_300;
pub const ID_CTX_SORT_DESC: u32 = 10_301;
pub const ID_CTX_TOP_N: u32 = 10_302;
pub const ID_CTX_FILTER_COLUMN: u32 = 10_303;
pub const ID_CTX_FILTER_TO: u32 = 10_304;
pub const ID_CTX_FILTER_OUT: u32 = 10_305;
pub const ID_CTX_CHIP_EDIT: u32 = 10_306;
pub const ID_CTX_CHIP_POLARITY: u32 = 10_307;
pub const ID_CTX_CHIP_TOGGLE: u32 = 10_308;
pub const ID_CTX_CHIP_REMOVE: u32 = 10_309;

/// The header's right-click menu for one column: sorting, the top-N cut, and the §7.2 route —
/// "Filter on this column…" opens the Filter dialog already scoped. `sort_here` is this column's
/// active direction; commands the register already lists dispatch by their register id, so a
/// context menu never grows a second name for the same act.
pub fn header_context(
    title: &str,
    sort_here: Option<bool>,
    any_sort: bool,
) -> Vec<tailhawk_core::menu::Item> {
    use tailhawk_core::menu::Item;
    let on = |item: Item, yes: bool| if yes { item } else { item.disabled() };
    vec![
        Item::check(
            "Sort &ascending",
            "",
            ID_CTX_SORT_ASC,
            sort_here == Some(false),
        ),
        Item::check(
            "Sort &descending",
            "",
            ID_CTX_SORT_DESC,
            sort_here == Some(true),
        ),
        Item::command(
            &format!("&Top {} by {title}", crate::TOP_N),
            "",
            ID_CTX_TOP_N,
        ),
        on(
            Item::command("Clear s&ort", "", command_id(Command::ClearSort)),
            any_sort,
        ),
        Item::separator(),
        Item::command(&format!("&Filter on {title}…"), "", ID_CTX_FILTER_COLUMN),
        Item::separator(),
        // The column chooser, on the menu the *List Views* page says to put it on: "provide a
        // column chooser on the column header context menu". It is where a person is when they
        // are thinking about columns.
        Item::command("&Select columns…", "", command_id(Command::SelectColumns)),
        Item::command("&Reset columns", "", command_id(Command::ResetColumns)),
    ]
}

/// A grid line's right-click menu: the selection's acts first, then the row's. Everything with a
/// selection subject is disabled without one — disabled, not hidden, per §1.1.
///
/// **No accelerators here, and that is the guide's rule rather than a preference.** *Menus*:
/// "Don't display shortcut key assignments within context menus. Context menus don't display the
/// shortcut key assignments because they are optimized for efficiency." The menu bar is where the
/// keys are taught; a context menu is where the hand already is.
pub fn grid_context(has_selection: bool, detail: bool) -> Vec<tailhawk_core::menu::Item> {
    use tailhawk_core::menu::Item;
    let on = |item: Item, yes: bool| if yes { item } else { item.disabled() };
    vec![
        on(
            Item::command("&Copy", "", command_id(Command::Copy)),
            has_selection,
        ),
        on(
            Item::command("Copy as TS&V", "", command_id(Command::CopyTsv)),
            has_selection,
        ),
        Item::separator(),
        on(
            Item::command("Filter &to this text", "", ID_CTX_FILTER_TO),
            has_selection,
        ),
        on(
            Item::command("Filter &out this text", "", ID_CTX_FILTER_OUT),
            has_selection,
        ),
        Item::separator(),
        Item::command("&Bookmark", "", command_id(Command::ToggleBookmark)),
        // §7, rung one: the row's trace becomes a filter. On the row menu because that is where a
        // person is when they have found the line they want to follow.
        Item::command("Follo&w this trace", "", command_id(Command::FollowTrace)),
        Item::check(
            "&Record detail",
            "",
            command_id(Command::ToggleDetail),
            detail,
        ),
        Item::separator(),
        Item::command(
            "&Define format from this line…",
            "",
            command_id(Command::DefineFormat),
        ),
    ]
}

/// A filter row's right-click menu in the panel — §2.4's per-chip acts, the same four the title
/// row's buttons and the chip's own glyphs offer, gathered where the pointer already is.
pub fn panel_row_context(enabled: bool, include: bool) -> Vec<tailhawk_core::menu::Item> {
    use tailhawk_core::menu::Item;
    vec![
        Item::command("&Edit…", "", ID_CTX_CHIP_EDIT),
        Item::command(
            if include {
                "Make e&xcluding"
            } else {
                "Make &including"
            },
            "",
            ID_CTX_CHIP_POLARITY,
        ),
        Item::check("Ena&bled", "", ID_CTX_CHIP_TOGGLE, enabled),
        Item::separator(),
        Item::command("&Remove", "", ID_CTX_CHIP_REMOVE),
    ]
}

/// §2.2's seven menus, built from the command register each time the bar is opened.
///
/// **Rebuilt rather than cached**, per the model's own note: enablement is a property of the moment
/// — there is no document, or no selection, or no sort to clear — and a tree built afresh cannot
/// hold a stale answer. Seven menus of a dozen items costs nothing beside a frame.
///
/// An item that cannot act is **disabled, not hidden**, per §1.1 and §1.2's memorability rule: a
/// menu whose shape changes under the user is a menu they cannot learn.
/// What the bar needs to know about the window it describes, beyond the document itself.
///
/// **A struct rather than eight arguments**, which is what it had grown to and what clippy stopped
/// at seven: a call site passing `true, false, false, false` in the right order by luck is a
/// mistake waiting to happen, and the fourth `bool` was the one the toolbar's icon size added.
#[derive(Copy, Clone, Debug, Default)]
pub struct BarState {
    pub dark: bool,
    pub toolbar: bool,
    pub large_icons: bool,
    pub maximised: bool,
    pub can_maximise: bool,
}

/// The configured remote sources as a menu, ending in the way to configure one.
///
/// **One function for two surfaces**: File ▸ Open remote source and the toolbar's `Open remote`
/// button drop the same list, with the same ids, because they are the same command reached two
/// ways. They did not, briefly: the File item greyed itself when no source was configured while the
/// button stayed live, which is two opposite answers to one question.
///
/// **`Remote sources…` is always last**, and that is what makes the surface safe to leave enabled
/// with nothing configured: a menu that drops empty is a dead end, and the first thing somebody
/// with no sources needs is the dialog that adds one.
///
/// **The names are the user's**, so an ampersand in one is doubled: `AppendMenuW` reads a single
/// `&` as a mnemonic marker, and a source called `R&D` would otherwise be drawn `RD`.
pub fn remote_menu_of(sources: &[String]) -> Vec<tailhawk_core::menu::Item> {
    use tailhawk_core::menu::Item;
    let mut items: Vec<Item> = sources
        .iter()
        .enumerate()
        .map(|(n, name)| Item::command(&name.replace('&', "&&"), "", ID_SOURCE_BASE + n as u32))
        .collect();
    if !items.is_empty() {
        items.push(Item::separator());
    }
    items.push(Item::command(
        "Remote sources…",
        "",
        command_id(Command::EditSources),
    ));
    items
}

/// The menu id a chrome shortcut runs, or `None` if this key is not one of them.
///
/// **This exists because a key was printed in three places and bound in none.** `Ctrl+H` was in the
/// register, in the Rules menu, in the toolbar's tooltip and in a status notice telling the user to
/// press it — and `VK_H` appeared nowhere in the shell. Nothing caught it because nothing could: the
/// keys were bound inside `&mut self` methods that need a document, and the menu's text was written
/// separately. A pure function that the menu's own text is tested against cannot drift that way.
///
/// **These are the shortcuts that need no document.** The navigation map, the find keys, the colour
/// labels and the clipboard are handled where the state they act on lives; see the test below,
/// which requires every key the menus print to be either answered here or named there.
pub fn shortcut_id(key: u16, ctrl: bool, shift: bool) -> Option<u32> {
    use windows::Win32::UI::Input::KeyboardAndMouse::{VK_F1, VK_K};
    match (key, ctrl, shift) {
        // **`Ctrl+K`, not the `Ctrl+H` this command advertised for a month.** "Never reassign
        // standard shortcut keys": the standard Edit menu gives `Ctrl+H` to Replace, and a viewer
        // that cannot replace anything should not be the program that takes the key.
        (k, true, false) if k == VK_K.0 => Some(command_id(Command::EditRules)),
        // The key every Windows user tries first, on the only help this program has.
        (k, false, false) if k == VK_F1.0 => Some(ID_KEYMAP),
        _ => None,
    }
}

pub fn menu_bar(
    doc: Option<&Document>,
    state: BarState,
    recent: &[String],
    sources: &[String],
) -> tailhawk_core::menu::Menu {
    use tailhawk_core::menu::{Item, Menu};
    let cmd = |label: &str, key: &str, c: Command| Item::command(label, key, command_id(c));
    let on = |item: Item, yes: bool| if yes { item } else { item.disabled() };
    let check = |label: &str, key: &str, c: Command, ticked: bool| {
        Item::check(label, key, command_id(c), ticked)
    };
    let BarState {
        dark,
        toolbar,
        large_icons,
        maximised,
        can_maximise,
    } = state;
    let open = doc.is_some();
    let selected = doc.is_some_and(|d| d.has_selection());
    let columns = doc.is_some_and(|d| d.has_columns());
    let following = doc.is_some_and(|d| d.is_following());
    let collapsed = doc.is_some_and(|d| d.is_collapsed());
    let invisibles = doc.is_some_and(|d| d.shows_invisibles());
    let detail = doc.is_some_and(|d| d.detail_open());
    let filters = doc.is_some_and(|d| d.filters_open());
    let filtered = doc.is_some_and(|d| d.is_filtered());
    let saving = doc.is_some_and(|d| d.is_saving());
    let back = doc.is_some_and(|d| d.can_step(true));
    let forward = doc.is_some_and(|d| d.can_step(false));

    // The recent files are entries of the File menu itself — the owner's choice of the
    // Notepad++ model over a submenu: numbered, newest first, just above Exit, and simply absent
    // when there is no history rather than greyed.
    let mut file_items = vec![
        cmd("&Open…", "Ctrl+O", Command::OpenFile),
        // `LOKI.md`'s remote sources, listed the way Open Recent lists files — and **never
        // greyed**, because [`remote_menu_of`]'s last item is the dialog that configures the first
        // source. It used to disable itself when none was configured, which left the one command
        // this program exists for reachable only by knowing that Tools held the way in.
        Item::submenu("Open re&mote source", remote_menu_of(sources)),
        on(cmd("&Close Tab", "Ctrl+W", Command::CloseTab), open),
        Item::separator(),
        on(cmd("&Export view…", "", Command::Export), open),
        on(cmd("&Keep saving…", "", Command::Tee), open),
        on(cmd("Stop sa&ving", "", Command::StopTee), saving),
        Item::separator(),
    ];
    if !recent.is_empty() {
        file_items.extend(
            recent.iter().enumerate().map(|(n, path)| {
                Item::command(&recent_label(n, path), "", ID_RECENT_BASE + n as u32)
            }),
        );
        file_items.push(Item::command("Clear &recent files", "", ID_CLEAR_RECENT));
        file_items.push(Item::separator());
    }
    file_items.push(Item::command("E&xit", "Alt+F4", ID_EXIT));

    Menu::bar(vec![
        Item::submenu("&File", file_items),
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
                // **Edit, not View.** The guide's standard Edit menu is Find… / Find next / Replace… /
                // Go to…, and a person looking for "go to line" looks where every other program
                // keeps it.
                on(Item::command("&Go to line…", "Ctrl+G", ID_GOTO), open),
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
                on(
                    cmd("Follo&w this trace", "Ctrl+T", Command::FollowTrace),
                    open,
                ),
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
                on(
                    check("Filter pane&l", "", Command::ToggleFilters, filters),
                    open,
                ),
                // §2.3: the row is hideable, and this is the only way back to it — so it is never
                // disabled, whatever is or is not open.
                // §2.3: the row is hideable, and this submenu is the only way back to it — so it
                // is never disabled, whatever is or is not open. **A submenu because that is where
                // Windows keeps this**: View ▸ Toolbars, with the sizes under it, is the shape a
                // person goes looking for, and it is also what gave the two size items a mnemonic
                // each — the View menu itself had run out, which the mnemonic test said plainly.
                Item::submenu(
                    "Toolb&ar",
                    vec![
                        check("&Show toolbar", "", Command::ToggleToolbar, toolbar),
                        Item::separator(),
                        on(
                            check("S&mall icons", "", Command::ToolbarSmallIcons, !large_icons),
                            toolbar,
                        ),
                        on(
                            check("&Large icons", "", Command::ToolbarLargeIcons, large_icons),
                            toolbar,
                        ),
                    ],
                ),
                // §3.1's arrangement: the focused pane fills the frame, or the split comes back.
                // Disabled with one pane, where it would have nothing to do.
                on(
                    check("&Maximise pane", "", Command::ToggleMaximise, maximised),
                    can_maximise,
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
                on(cmd("&Back", "Alt+←", Command::Back), back),
                on(cmd("F&orward", "Alt+→", Command::Forward), forward),
            ],
        ),
        Item::submenu(
            "F&ormat",
            vec![
                {
                    // §6.1's format choices as a real radio-marked submenu — the right-edge chip
                    // and its bespoke dropdown retire. Rows are data from the live detection, so
                    // they carry range ids like the recent files do; a literal `&` in a format
                    // name is doubled so it draws as itself.
                    let rows: Vec<Item> = doc
                        .map(|d| {
                            d.format_rows()
                                .into_iter()
                                .enumerate()
                                .map(|(i, (label, current, separator))| {
                                    if separator {
                                        Item::separator()
                                    } else {
                                        Item::check(
                                            &label.replace('&', "&&"),
                                            "",
                                            ID_FORMAT_BASE + i as u32,
                                            current,
                                        )
                                    }
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    on(Item::submenu("&Log format", rows), open)
                },
                on(cmd("&Define from a line…", "", Command::DefineFormat), open),
                on(cmd("&Import layout…", "", Command::ImportLayout), open),
                Item::separator(),
                on(cmd("Select &columns…", "", Command::SelectColumns), columns),
                on(cmd("&Reset columns", "", Command::ResetColumns), columns),
                on(cmd("Clear &sort", "", Command::ClearSort), columns),
            ],
        ),
        Item::submenu(
            "&Rules",
            vec![
                cmd("&Highlight rules…", "Ctrl+K", Command::EditRules),
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
            // **`Tools`, not `Settings`.** The UX guide forbids both words this menu used to use:
            // "Settings: Don't use as a menu label. Use Options instead" and "Preferences: Don't
            // use. Use Options instead." `Tools` is the standard category and `Options` the
            // standard item, which is also where a person goes looking for them.
            "&Tools",
            vec![
                check("&Dark theme", "", Command::ToggleTheme, dark),
                // §12.4's remote sources. Never disabled: it is where a source is first defined,
                // so needing one open to reach it would be a door locked from the inside.
                cmd("&Remote sources…", "", Command::EditSources),
                Item::command("&Font…", "", ID_FONT),
                Item::separator(),
                // **No ellipsis**: the guide lists Options among the commands whose implicit verb
                // is to show a window, which therefore do not take one.
                Item::command("&Options", "", ID_PREFS),
            ],
        ),
        Item::submenu(
            "&Help",
            vec![
                // Both are available with no document open: a user who cannot remember how to open
                // a file is exactly the user who needs the keyboard map.
                Item::command("&Keyboard map", "F1", ID_KEYMAP),
                Item::separator(),
                Item::command("&About Tailhawk", "", ID_ABOUT),
            ],
        ),
    ])
}

/// One recent-file entry: the customary numbered mnemonic — `&1` through `&9`, then `1&0` — and
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The header menu's facts: the active direction is the ticked one, Clear sort is dead
    /// without a sort anywhere, and the column's own name is in the items that act on it.
    #[test]
    fn the_header_context_reflects_the_columns_sort_and_names_it() {
        let items = header_context("level", Some(false), true);
        assert!(items[0].checked, "ascending is the active direction");
        assert!(!items[1].checked);
        assert!(items[2].label.contains("level"), "Top-N names the column");
        assert!(items[3].enabled, "a sort exists, so Clear sort can act");
        assert!(
            items.iter().any(|i| i.label == "&Filter on level…"),
            "the §7.2 route names the column"
        );
        let idle = header_context("level", None, false);
        assert!(!idle[0].checked && !idle[1].checked);
        assert!(!idle[3].enabled, "nothing to clear");
    }

    /// The grid menu's one gate: every item whose subject is the selection dies with it.
    ///
    /// Separators are skipped throughout — [`Item::separator`] is born `enabled: false`, since
    /// there is nothing there to choose, and that is not the greying this test is about.
    #[test]
    fn the_grid_context_disables_selection_acts_without_a_selection() {
        use tailhawk_core::menu::Kind;
        let choosable = |items: Vec<tailhawk_core::menu::Item>| {
            items
                .into_iter()
                .filter(|i| i.kind != Kind::Separator)
                .collect::<Vec<_>>()
        };
        let without = choosable(grid_context(false, false));
        let dead: Vec<&str> = without
            .iter()
            .filter(|i| !i.enabled)
            .map(|i| i.label.as_str())
            .collect();
        assert_eq!(
            dead,
            [
                "&Copy",
                "Copy as TS&V",
                "Filter &to this text",
                "Filter &out this text"
            ],
            "exactly the selection's acts, nothing else"
        );
        assert!(choosable(grid_context(true, false))
            .iter()
            .all(|i| i.enabled));
        assert!(
            grid_context(true, true)
                .iter()
                .any(|i| i.label == "&Record detail" && i.checked),
            "the detail pane's state shows as its tick"
        );
    }

    /// The chip menu mirrors its chip: the tick is the enabled state, and the polarity item
    /// offers the *other* polarity, which is the only one worth offering.
    #[test]
    fn the_panel_row_context_mirrors_its_chip() {
        let include = panel_row_context(true, true);
        assert!(include.iter().any(|i| i.label == "Ena&bled" && i.checked));
        assert!(include.iter().any(|i| i.label == "Make e&xcluding"));
        let exclude = panel_row_context(false, false);
        assert!(exclude.iter().any(|i| i.label == "Ena&bled" && !i.checked));
        assert!(exclude.iter().any(|i| i.label == "Make &including"));
    }

    /// No two items of one context menu share a mnemonic — the same rule the bar's menus keep,
    /// for the same reason: the second claimant is unreachable by the key drawn under it.
    #[test]
    fn no_two_items_in_one_context_menu_share_a_mnemonic() {
        for (name, items) in [
            ("header", header_context("level", None, true)),
            ("grid", grid_context(true, true)),
            ("panel row", panel_row_context(true, true)),
        ] {
            let mut seen = Vec::new();
            for m in items.iter().filter_map(|i| i.mnemonic()) {
                assert!(!seen.contains(&m), "{name}: two items claim '{m}'");
                seen.push(m);
            }
        }
    }

    /// **The one thing that makes both remote surfaces safe to leave enabled.** With no source
    /// configured the menu is not empty — it is the dialog that adds one — so neither the File
    /// item nor the toolbar button is ever a dead end, and neither is ever greyed.
    #[test]
    fn the_remote_menu_always_ends_with_the_way_to_configure_one() {
        let none = remote_menu_of(&[]);
        assert_eq!(none.len(), 1, "no sources, no separator, no blank menu");
        assert_eq!(none[0].text(), "Remote sources…");
        assert_eq!(none[0].id, Some(command_id(Command::EditSources)));

        let two = remote_menu_of(&["live".to_owned(), "qa".to_owned()]);
        let text: Vec<String> = two.iter().map(|i| i.text()).collect();
        assert_eq!(text, ["live", "qa", "", "Remote sources…"]);
        assert_eq!(two[0].id, Some(ID_SOURCE_BASE));
        assert_eq!(two[1].id, Some(ID_SOURCE_BASE + 1));
    }

    /// **A source the user called `R&D` is drawn `R&D`.** `AppendMenuW` reads a lone `&` as a
    /// mnemonic marker, so an unescaped name loses the character and underlines the next one —
    /// and a name that is all one word could take a mnemonic another item already owns.
    #[test]
    fn an_ampersand_in_a_source_name_survives_the_menu() {
        let items = remote_menu_of(&["R&D".to_owned()]);
        assert_eq!(items[0].label, "R&&D", "doubled for Windows");
        assert_eq!(items[0].text(), "R&D", "drawn as the user typed it");
    }

    /// The File menu and the toolbar drop the same list — one function, so they cannot answer the
    /// same question two ways, which is what they did for a few hours on 2026-09-09.
    #[test]
    fn the_file_menu_offers_remote_sources_even_when_none_are_configured() {
        let menu = menu_bar(
            None,
            BarState {
                toolbar: true,
                ..BarState::default()
            },
            &[],
            &[],
        );
        let file = menu.at(&[0]).expect("File opens").to_vec();
        let remote = file
            .iter()
            .find(|i| i.text() == "Open remote source")
            .expect("File offers it");
        assert!(remote.enabled, "never greyed: it is the way in");
        assert_eq!(
            remote.items.iter().map(|i| i.text()).collect::<Vec<_>>(),
            remote_menu_of(&[])
                .iter()
                .map(|i| i.text())
                .collect::<Vec<_>>()
        );
    }

    /// Walks a menu tree, returning every `(printed text, accelerator)` pair below it.
    fn advertised(items: &[tailhawk_core::menu::Item], into: &mut Vec<(String, String)>) {
        for item in items {
            if !item.accelerator.is_empty() {
                into.push((item.text(), item.accelerator.clone()));
            }
            advertised(&item.items, into);
        }
    }

    /// **The test `Ctrl+H` needed for a month and never had.** The key was printed in the Rules
    /// menu, in the register, in the toolbar's tooltip and in a status notice telling the user to
    /// press it, and `VK_H` appeared nowhere in the shell: pressing it did nothing at all.
    ///
    /// Every key any menu prints must be either answered by [`shortcut_id`] or named below as
    /// handled somewhere else — which is a deliberate act, so the next key advertised without a
    /// binding fails here instead of shipping.
    #[test]
    fn every_key_the_menus_print_is_bound_somewhere() {
        // Handled where the state they act on lives, and each was confirmed present in the shell
        // rather than assumed: the navigation map and the colour labels in `view_key`, the find
        // family in `find_key`, the dialogs in `chrome_key`, the clipboard in `WM_KEYDOWN` itself,
        // and `Alt+F4` by Windows. **`Ctrl+X` and `Ctrl+V` are here for a different reason**: the
        // Edit menu shows Cut and Paste permanently greyed, because a viewer that hid them would
        // read as broken, and a greyed item's key is not a promise.
        const ELSEWHERE: [&str; 32] = [
            "Alt+F4",
            "Alt+←",
            "Alt+→",
            "Ctrl+C",
            "Ctrl+D",
            "Ctrl+E",
            "Ctrl+End",
            "Ctrl+Enter",
            "Ctrl+F",
            "Ctrl+G",
            "Ctrl+Home",
            "Ctrl+I",
            "Ctrl+L",
            "Ctrl+O",
            "Ctrl+Shift+0",
            "Ctrl+Shift+1…9",
            "Ctrl+Shift+C",
            "Ctrl+Shift+E",
            "Ctrl+Shift+L",
            "Ctrl+Shift+Tab",
            "Ctrl+T",
            "Ctrl+Tab",
            "Ctrl+V",
            "Ctrl+W",
            "Ctrl+X",
            "Ctrl+\\",
            "Esc",
            "F2",
            "F3",
            "F6",
            "Shift+F2",
            "Shift+F3",
        ];
        let menu = menu_bar(
            None,
            BarState {
                toolbar: true,
                ..BarState::default()
            },
            &[],
            &[],
        );
        let mut keys = Vec::new();
        advertised(menu.items(), &mut keys);
        assert!(keys.len() > 20, "the walk found the menus: {}", keys.len());

        // What `shortcut_id` answers, written the way a menu prints it.
        let bound: Vec<&str> = ["Ctrl+K", "F1"].to_vec();
        let unclaimed: Vec<String> = keys
            .iter()
            .filter(|(_, key)| !bound.contains(&key.as_str()) && !ELSEWHERE.contains(&key.as_str()))
            .map(|(text, key)| format!("{text} = {key}"))
            .collect();
        assert!(
            unclaimed.is_empty(),
            "nothing claims to handle: {}",
            unclaimed.join(", ")
        );
    }

    /// The two keys [`shortcut_id`] owns, checked against the menu that prints them — the halves
    /// that drifted apart last time, tied together.
    #[test]
    fn the_keys_shortcut_id_owns_are_the_keys_the_menus_print() {
        use windows::Win32::UI::Input::KeyboardAndMouse::{VK_F1, VK_K};
        let menu = menu_bar(
            None,
            BarState {
                toolbar: true,
                ..BarState::default()
            },
            &[],
            &[],
        );
        let mut keys = Vec::new();
        advertised(menu.items(), &mut keys);
        let printed = |what: &str| -> String {
            keys.iter()
                .find(|(text, _)| text.starts_with(what))
                .map(|(_, key)| key.clone())
                .unwrap_or_else(|| panic!("{what} is in no menu"))
        };
        assert_eq!(printed("Highlight rules"), "Ctrl+K");
        assert_eq!(
            shortcut_id(VK_K.0, true, false),
            Some(command_id(Command::EditRules))
        );
        assert_eq!(printed("Keyboard map"), "F1");
        assert_eq!(shortcut_id(VK_F1.0, false, false), Some(ID_KEYMAP));
        assert_eq!(
            shortcut_id(VK_K.0, false, false),
            None,
            "the key alone is a letter, not a command"
        );
    }

    /// `UI-DESIGN.md` §1.2's discoverability rule, as a test rather than an intention: **every
    /// command in the register appears in at least one menu.**
    ///
    /// This is the rule that stops the menu bar drifting behind the command palette. A command
    /// reachable only by a keystroke someone has to already know is a command that, for anyone who
    /// does not, is not there at all.
    #[test]
    fn every_listed_command_appears_in_at_least_one_menu() {
        let ids = menu_bar(
            None,
            BarState {
                toolbar: true,
                ..BarState::default()
            },
            &[],
            &[],
        )
        .ids();
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
    ///
    /// **Both shapes of the File menu are checked, and the second is why this reads as it does.**
    /// The recent files are items of that menu rather than a submenu, so a menu built with no
    /// history is a *different menu* from the one a user of the application ever sees — and for a
    /// month this test only ever built the empty one. `tools/Menu.ps1`, reading the real `HMENU`
    /// out of the running window, found the collision the empty menu cannot contain: `&Clear
    /// recent files` claimed the `C` that `&Close Tab` already had.
    #[test]
    fn no_two_items_in_one_menu_share_a_mnemonic() {
        let recent: Vec<String> = (1..=10).map(|n| format!("C:\\logs\\app{n}.log")).collect();
        let mut clashes: Vec<String> = Vec::new();

        for (shape, menu) in [
            (
                "no recent files",
                menu_bar(
                    None,
                    BarState {
                        toolbar: true,
                        ..BarState::default()
                    },
                    &[],
                    &[],
                ),
            ),
            (
                "with recent files",
                menu_bar(
                    None,
                    BarState {
                        toolbar: true,
                        ..BarState::default()
                    },
                    &recent,
                    &[],
                ),
            ),
        ] {
            let heads: Vec<Option<char>> = menu.items().iter().map(|i| i.mnemonic()).collect();
            for (a, letter) in heads.iter().enumerate() {
                for (b, other) in heads.iter().enumerate().skip(a + 1) {
                    if letter.is_some() && letter == other {
                        clashes.push(format!("{shape}, the bar: {a} and {b} claim {letter:?}"));
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
                                "{shape}, {}: {name:?} and {other:?} both claim {letter:?}",
                                head.text()
                            ));
                        }
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
        let menu = menu_bar(
            None,
            BarState {
                toolbar: true,
                ..BarState::default()
            },
            &[],
            &[],
        );
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
        assert!(
            bare.is_empty(),
            "choosable items with no mnemonic: {bare:?}"
        );
    }

    /// The four surfaces §2.2 used to draw disabled are built, and a menu that still greys them is
    /// a menu that hides working features.
    #[test]
    fn the_four_built_surfaces_are_no_longer_greyed() {
        let menu = menu_bar(
            None,
            BarState {
                toolbar: true,
                ..BarState::default()
            },
            &[],
            &[],
        );
        // Renamed 2026-09-09 to what the UX guide calls them: `Tools` rather than `Settings`, and
        // `Options` rather than `Preferences`, with `Font…` moved to the same menu.
        for (top, label) in [
            ("Tools", "Options"),
            ("Tools", "Font"),
            ("Help", "Keyboard map"),
            ("Help", "About Tailhawk"),
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

    /// The Notepad++ model, the owner's choice: recent files are numbered entries of the File
    /// menu itself, newest first, just above Exit, *Clear recent files* beneath them — and with
    /// no history there is simply nothing, not a greyed stub.
    #[test]
    fn recent_files_are_entries_of_the_file_menu_itself() {
        let menu = menu_bar(
            None,
            BarState {
                toolbar: true,
                ..BarState::default()
            },
            &[],
            &[],
        );
        let file = menu.at(&[0]).expect("File opens").to_vec();
        assert!(
            !file.iter().any(|i| i.text().contains("Clear recent")),
            "no history, no entries"
        );
        assert!(
            !file.iter().any(|i| i.text().contains("Open Recent")),
            "the submenu is gone"
        );

        let paths = vec![
            r"C:\logs\newest.log".to_owned(),
            r"C:\logs\older.log".to_owned(),
        ];
        let menu = menu_bar(
            None,
            BarState {
                toolbar: true,
                ..Default::default()
            },
            &paths,
            &[],
        );
        let file = menu.at(&[0]).expect("File opens").to_vec();
        let first = file
            .iter()
            .position(|i| i.text().ends_with("newest.log"))
            .expect("the newest file is an entry of File directly");
        assert!(
            file[first].text().starts_with("1 "),
            "numbered, newest first: {}",
            file[first].text()
        );
        assert!(file[first + 1].text().ends_with("older.log"));
        assert!(
            file[first + 2].text().contains("Clear recent files"),
            "the customary way out sits beneath the entries"
        );
        assert!(
            file.last().expect("items").text().contains("Exit"),
            "and Exit stays the menu's last word"
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

    /// With no document open, everything that needs one is disabled — and `Open…` is not, or the
    /// menu would offer no way back.
    #[test]
    fn with_no_document_the_file_menu_still_offers_open() {
        let menu = menu_bar(
            None,
            BarState {
                toolbar: true,
                ..BarState::default()
            },
            &[],
            &[],
        );
        let file = menu.at(&[0]).expect("File").to_vec();
        assert!(
            file[0].text().starts_with("Open"),
            "got {:?}",
            file[0].text()
        );
        assert!(file[0].selectable(), "Open… must never be disabled");
        assert!(
            file.iter().any(|i| !i.selectable() && !i.text().is_empty()),
            "with no document, something should be disabled"
        );
    }

    /// The dark-theme item is ticked from the theme it was built with — the one piece of menu
    /// state that comes from neither the document nor the register.
    #[test]
    fn the_theme_item_is_ticked_to_match_the_theme() {
        for dark in [true, false] {
            let menu = menu_bar(
                None,
                BarState {
                    dark,
                    toolbar: true,
                    ..Default::default()
                },
                &[],
                &[],
            );
            // Settings is the sixth heading.
            let settings = menu.at(&[5]).expect("Settings").to_vec();
            let theme_item = settings
                .iter()
                .find(|i| i.text().contains("Dark theme"))
                .expect("the Settings menu carries the theme toggle");
            assert_eq!(theme_item.checked, dark);
        }
    }

    /// **A context menu never prints a shortcut key.** *Menus*: "Don't display shortcut key
    /// assignments within context menus. Context menus don't display the shortcut key assignments
    /// because they are optimized for efficiency." The menu bar teaches the keys; the context menu
    /// is where the hand already is. Ours printed them until 2026-09-09.
    #[test]
    fn no_context_menu_prints_a_shortcut_key() {
        let menus: Vec<(&str, Vec<tailhawk_core::menu::Item>)> = vec![
            ("grid", grid_context(true, false)),
            ("grid, no selection", grid_context(false, true)),
            ("header", header_context("timestamp", Some(true), true)),
            ("filter row", panel_row_context(true, true)),
        ];
        for (name, items) in menus {
            for item in &items {
                assert!(
                    item.accelerator.is_empty(),
                    "{name}: \"{}\" prints \"{}\"",
                    item.text(),
                    item.accelerator
                );
            }
        }
    }
}
