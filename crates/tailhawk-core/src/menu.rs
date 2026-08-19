//! The menu model — `UI-DESIGN.md` §1.1 and §2.1, and `SPEC.md` §6.5's right-click.
//!
//! One model serves the **menu bar** and every **context menu**, because they are the same control
//! in two places: a bar is a [`Menu`] whose items are its top-level headings, a context menu is a
//! [`Menu`] opened at a point, and nothing else about them differs. Two implementations of a menu
//! drift inside a release — one grows a checkable item, the other grows nested submenus — and the
//! drift is invisible until a user finds the half that does not behave.
//!
//! Nothing here knows about Win32. There is no `HMENU`: §1.1 rejects the grey three-dimensional
//! chrome one brings with it, and the app draws its own menus in its own register. What this module
//! owns is the *shape* of a menu and the rules for moving through it; where the pixels go is the
//! shell's.
//!
//! **The accelerator is text, not a binding.** `UI-DESIGN.md` §12 owns the key map and the shell
//! dispatches it. An [`Item`] carries `Ctrl+O` as a string so the menu can *teach* the keyboard,
//! and the two are held together by a test rather than by this module taking over dispatch.

/// What an [`Item`] is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Runs `id` and closes the menu.
    Command,
    /// A rule between groups. Drawn, never selectable.
    Separator,
    /// Runs `id` and closes; drawn with a mark when `checked`.
    Check,
    /// Opens `items`.
    Submenu,
}

/// One entry of a menu.
///
/// The mnemonic is marked in the label the way Windows writes it — `&File` — rather than held in a
/// field beside it, so the letter that is underlined and the letter that is typed cannot disagree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub label: String,
    /// What §12 binds this to, as text: `Ctrl+O`. Empty when the command has no accelerator.
    pub accelerator: String,
    /// What choosing it means to the shell. `None` for a separator or a submenu.
    pub id: Option<u32>,
    pub kind: Kind,
    /// **Shown when false, never hidden** — `UI-DESIGN.md` §1.1's "never lie, never silently drop".
    /// A menu whose shape changes under the user is a menu they cannot learn.
    pub enabled: bool,
    pub checked: bool,
    pub items: Vec<Item>,
}

impl Item {
    pub fn command(label: &str, accelerator: &str, id: u32) -> Item {
        Item {
            label: label.to_owned(),
            accelerator: accelerator.to_owned(),
            id: Some(id),
            kind: Kind::Command,
            enabled: true,
            checked: false,
            items: Vec::new(),
        }
    }

    /// A command drawn with a tick when `checked` — a mode, not an action.
    pub fn check(label: &str, accelerator: &str, id: u32, checked: bool) -> Item {
        Item {
            kind: Kind::Check,
            checked,
            ..Item::command(label, accelerator, id)
        }
    }

    pub fn separator() -> Item {
        Item {
            label: String::new(),
            accelerator: String::new(),
            id: None,
            kind: Kind::Separator,
            enabled: false,
            checked: false,
            items: Vec::new(),
        }
    }

    pub fn submenu(label: &str, items: Vec<Item>) -> Item {
        Item {
            label: label.to_owned(),
            accelerator: String::new(),
            id: None,
            kind: Kind::Submenu,
            enabled: true,
            checked: false,
            items,
        }
    }

    /// Greys the item out. It still occupies its place; see [`Item::enabled`].
    pub fn disabled(mut self) -> Item {
        self.enabled = false;
        self
    }

    /// The label without its mnemonic marker, which is what a painter draws.
    pub fn text(&self) -> String {
        let mut out = String::with_capacity(self.label.len());
        let mut mark = false;
        for c in self.label.chars() {
            match c {
                '&' if !mark => mark = true,
                c => {
                    out.push(c);
                    mark = false;
                }
            }
        }
        out
    }

    /// Where the underline goes, as a **character** offset into [`Item::text`], or `None` when the
    /// label marks no mnemonic.
    pub fn mnemonic_at(&self) -> Option<usize> {
        let mut at = 0;
        let mut mark = false;
        for c in self.label.chars() {
            match c {
                '&' if !mark => mark = true,
                _ if mark => return Some(at),
                _ => at += 1,
            }
        }
        None
    }

    /// The letter that opens or runs this item, lowercased.
    pub fn mnemonic(&self) -> Option<char> {
        let text = self.text();
        let at = self.mnemonic_at()?;
        text.chars().nth(at).map(|c| c.to_ascii_lowercase())
    }

    /// Whether the keyboard may land on it: a separator never, a disabled item never.
    pub fn selectable(&self) -> bool {
        self.enabled && self.kind != Kind::Separator
    }
}

/// A menu bar, or one context menu.
///
/// `open` is the path to what is currently showing: empty when nothing is down, `[2]` when the
/// third top-level menu is open, `[2, 5]` when its sixth item is a submenu and that is open too.
/// `selected` is the path to the item under the keyboard.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Menu {
    items: Vec<Item>,
    open: Vec<usize>,
    selected: Vec<usize>,
    /// A bar keeps its headings on a row and opens downward; a context menu is the open list
    /// itself, with no row above it.
    bar: bool,
}

impl Menu {
    /// §2.1's menu bar: `items` are the headings, and nothing is open yet.
    pub fn bar(items: Vec<Item>) -> Menu {
        Menu {
            items,
            open: Vec::new(),
            selected: Vec::new(),
            bar: true,
        }
    }

    /// `SPEC.md` §6.5's right-click: a menu that is already open, with no headings above it.
    pub fn context(items: Vec<Item>) -> Menu {
        let mut menu = Menu {
            items,
            open: Vec::new(),
            selected: Vec::new(),
            bar: false,
        };
        menu.selected = menu.first_selectable(&[]).map(|i| vec![i]).unwrap_or_default();
        menu
    }

    pub fn is_bar(&self) -> bool {
        self.bar
    }

    /// Whether anything is showing: for a bar, whether a heading is down; for a context menu,
    /// always, until it is closed.
    pub fn is_open(&self) -> bool {
        !self.bar || !self.open.is_empty()
    }

    pub fn items(&self) -> &[Item] {
        &self.items
    }

    /// The path to the open list, deepest last. Empty for a context menu's own list.
    pub fn open_path(&self) -> &[usize] {
        &self.open
    }

    pub fn selected(&self) -> &[usize] {
        &self.selected
    }

    /// The items at `path`, or `None` when the path does not lead to a submenu.
    pub fn at(&self, path: &[usize]) -> Option<&[Item]> {
        let mut items = &self.items;
        for step in path {
            let item = items.get(*step)?;
            if item.kind != Kind::Submenu {
                return None;
            }
            items = &item.items;
        }
        Some(items)
    }

    /// The item at `path`.
    pub fn item(&self, path: &[usize]) -> Option<&Item> {
        let (last, head) = path.split_last()?;
        self.at(head)?.get(*last)
    }

    fn first_selectable(&self, path: &[usize]) -> Option<usize> {
        let items = self.at(path)?;
        items.iter().position(Item::selectable)
    }

    /// Drops a heading open, or closes it if it already is. `top` indexes the bar.
    pub fn open_top(&mut self, top: usize) {
        if !self.bar || self.items.get(top).is_none() {
            return;
        }
        if self.open.first() == Some(&top) {
            self.close();
            return;
        }
        self.open = vec![top];
        self.selected = match self.first_selectable(&[top]) {
            Some(i) => vec![top, i],
            None => vec![top],
        };
    }

    /// Closes everything. A bar keeps its headings; a context menu is done.
    pub fn close(&mut self) {
        self.open.clear();
        self.selected.clear();
    }

    /// `Esc`: shuts the deepest submenu, and only when there is none does it close the menu.
    /// Reports whether anything is still showing.
    pub fn escape(&mut self) -> bool {
        if self.open.len() > 1 {
            self.open.pop();
            let head = self.open.clone();
            self.selected = head;
            return true;
        }
        self.close();
        false
    }

    /// `↑` / `↓` within the open list, skipping separators and disabled items. Does not wrap past
    /// the ends, because a menu that wraps makes the last item hard to reach by holding a key.
    pub fn step(&mut self, by: isize) {
        let Some((last, head)) = self.selected.split_last().map(|(l, h)| (*l, h.to_vec())) else {
            // Nothing selected yet: the first item of whatever is open.
            let path = self.open.clone();
            if let Some(i) = self.first_selectable(&path) {
                self.selected = [path, vec![i]].concat();
            }
            return;
        };
        let Some(items) = self.at(&head) else { return };
        let mut at = last as isize;
        for _ in 0..items.len() {
            at += by;
            if at < 0 || at >= items.len() as isize {
                return;
            }
            if items[at as usize].selectable() {
                self.selected = [head, vec![at as usize]].concat();
                return;
            }
        }
    }

    /// `←` / `→` along the bar's headings, opening the one it lands on.
    ///
    /// Inside a submenu, `→` opens it and `←` shuts it — which is what makes the arrows one gesture
    /// rather than two, the way every Windows menu behaves.
    pub fn across(&mut self, by: isize) -> Option<u32> {
        if by > 0 {
            if let Some(item) = self.item(&self.selected.clone()) {
                if item.kind == Kind::Submenu {
                    self.enter();
                    return None;
                }
            }
        }
        if by < 0 && self.open.len() > 1 {
            self.escape();
            return None;
        }
        if !self.bar {
            return None;
        }
        let top = *self.open.first().unwrap_or(&0) as isize;
        let count = self.items.len() as isize;
        if count == 0 {
            return None;
        }
        let next = (top + by).rem_euclid(count) as usize;
        self.open = Vec::new();
        self.open_top(next);
        None
    }

    /// `Enter`, or a click: opens a submenu, or reports the command chosen and closes.
    pub fn enter(&mut self) -> Option<u32> {
        let path = self.selected.clone();
        let item = self.item(&path)?;
        if !item.selectable() {
            return None;
        }
        if item.kind == Kind::Submenu {
            self.open = path.clone();
            self.selected = match self.first_selectable(&path) {
                Some(i) => [path, vec![i]].concat(),
                None => path,
            };
            return None;
        }
        let id = item.id;
        self.close();
        id
    }

    /// Puts the keyboard on `path` — the mouse moving over an item, or a click resolving to one.
    /// A submenu under the pointer opens, as it does on Windows.
    pub fn hover(&mut self, path: &[usize]) {
        if self.item(path).is_some_and(Item::selectable) {
            self.selected = path.to_vec();
            if self.item(path).is_some_and(|i| i.kind == Kind::Submenu) {
                self.open = path.to_vec();
            }
        }
    }

    /// A mnemonic letter. On a closed bar it opens the heading; in an open list it runs the item,
    /// which is what `Alt`, `F`, `O` does on Windows.
    ///
    /// Reports the command when the letter chose one.
    pub fn mnemonic(&mut self, letter: char) -> Option<u32> {
        let letter = letter.to_ascii_lowercase();
        if self.bar && self.open.is_empty() {
            let at = self
                .items
                .iter()
                .position(|i| i.selectable() && i.mnemonic() == Some(letter))?;
            self.open_top(at);
            return None;
        }
        let path = self.open.clone();
        let items = self.at(&path)?;
        let at = items
            .iter()
            .position(|i| i.selectable() && i.mnemonic() == Some(letter))?;
        self.selected = [path, vec![at]].concat();
        self.enter()
    }

    /// Every command id the menu offers, at any depth — what a test walks to check §12.
    pub fn ids(&self) -> Vec<u32> {
        fn walk(items: &[Item], out: &mut Vec<u32>) {
            for item in items {
                if let Some(id) = item.id {
                    out.push(id);
                }
                walk(&item.items, out);
            }
        }
        let mut out = Vec::new();
        walk(&self.items, &mut out);
        out
    }

    /// Every (label, accelerator) the menu offers, at any depth.
    pub fn accelerators(&self) -> Vec<(String, String)> {
        fn walk(items: &[Item], out: &mut Vec<(String, String)>) {
            for item in items {
                if !item.accelerator.is_empty() {
                    out.push((item.text(), item.accelerator.clone()));
                }
                walk(&item.items, out);
            }
        }
        let mut out = Vec::new();
        walk(&self.items, &mut out);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a_bar() -> Menu {
        Menu::bar(vec![
            Item::submenu(
                "&File",
                vec![
                    Item::command("&Open…", "Ctrl+O", 1),
                    Item::command("Open &Set…", "Ctrl+Shift+O", 2),
                    Item::separator(),
                    Item::command("&Close Tab", "Ctrl+W", 3).disabled(),
                ],
            ),
            Item::submenu(
                "&View",
                vec![
                    Item::check("&Follow Tail", "F", 10, true),
                    Item::check("&Invisibles", "Ctrl+I", 11, false),
                    Item::submenu("&Theme", vec![Item::command("&Dark", "", 20)]),
                ],
            ),
        ])
    }

    #[test]
    fn a_bar_shows_nothing_until_a_heading_is_opened() {
        let mut menu = a_bar();
        assert!(!menu.is_open());
        menu.open_top(0);
        assert!(menu.is_open());
        assert_eq!(menu.open_path(), &[0]);
        assert_eq!(menu.selected(), &[0, 0], "on the first item that can be used");
    }

    #[test]
    fn a_context_menu_is_open_from_the_moment_it_exists() {
        let menu = Menu::context(vec![Item::separator(), Item::command("&Copy", "Ctrl+C", 1)]);
        assert!(menu.is_open());
        assert_eq!(
            menu.selected(),
            &[1],
            "the separator is not where the keyboard lands"
        );
    }

    #[test]
    fn clicking_an_open_heading_shuts_it_again() {
        let mut menu = a_bar();
        menu.open_top(0);
        menu.open_top(0);
        assert!(!menu.is_open());
    }

    #[test]
    fn the_arrows_skip_separators_and_disabled_items_but_do_not_wrap() {
        let mut menu = a_bar();
        menu.open_top(0);
        menu.step(1);
        assert_eq!(menu.selected(), &[0, 1]);
        menu.step(1);
        assert_eq!(
            menu.selected(),
            &[0, 1],
            "past the separator is only a disabled Close Tab, so the selection stays"
        );
        menu.step(-1);
        assert_eq!(menu.selected(), &[0, 0]);
        menu.step(-1);
        assert_eq!(menu.selected(), &[0, 0], "the top does not wrap to the bottom");
    }

    #[test]
    fn a_disabled_command_is_still_there_to_be_seen() {
        let menu = a_bar();
        let close = &menu.at(&[0]).expect("File")[3];
        assert_eq!(close.text(), "Close Tab");
        assert!(!close.enabled, "§1.1: shown, not hidden");
        assert!(!close.selectable());
    }

    #[test]
    fn enter_runs_a_command_and_closes() {
        let mut menu = a_bar();
        menu.open_top(0);
        assert_eq!(menu.enter(), Some(1));
        assert!(!menu.is_open(), "choosing is the end of the interaction");
    }

    #[test]
    fn enter_on_a_submenu_opens_it_rather_than_choosing() {
        let mut menu = a_bar();
        menu.open_top(1);
        menu.step(1);
        menu.step(1);
        assert_eq!(menu.enter(), None);
        assert_eq!(menu.open_path(), &[1, 2]);
        assert_eq!(menu.selected(), &[1, 2, 0]);
        assert_eq!(menu.enter(), Some(20), "and the item inside it does choose");
    }

    #[test]
    fn escape_unwinds_one_level_at_a_time() {
        let mut menu = a_bar();
        menu.open_top(1);
        menu.step(1);
        menu.step(1);
        menu.enter();
        assert_eq!(menu.open_path(), &[1, 2]);
        assert!(menu.escape(), "the submenu shuts, the menu stays");
        assert_eq!(menu.open_path(), &[1]);
        assert!(!menu.escape());
        assert!(!menu.is_open());
    }

    #[test]
    fn the_side_arrows_walk_the_headings_and_wrap() {
        let mut menu = a_bar();
        menu.open_top(0);
        menu.across(1);
        assert_eq!(menu.open_path(), &[1]);
        menu.across(1);
        assert_eq!(menu.open_path(), &[0], "the bar wraps even though a list does not");
        menu.across(-1);
        assert_eq!(menu.open_path(), &[1]);
    }

    #[test]
    fn the_side_arrows_open_and_shut_a_submenu_before_moving_along_the_bar() {
        let mut menu = a_bar();
        menu.open_top(1);
        menu.step(1);
        menu.step(1);
        menu.across(1);
        assert_eq!(menu.open_path(), &[1, 2], "right opened it rather than moving on");
        menu.across(-1);
        assert_eq!(menu.open_path(), &[1], "and left shut it rather than moving back");
    }

    #[test]
    fn a_mnemonic_opens_a_heading_then_runs_an_item() {
        let mut menu = a_bar();
        assert_eq!(menu.mnemonic('f'), None);
        assert_eq!(menu.open_path(), &[0]);
        assert_eq!(menu.mnemonic('s'), Some(2), "Open Set…");
        assert!(!menu.is_open());
    }

    #[test]
    fn a_mnemonic_is_case_blind_and_ignores_what_it_cannot_reach() {
        let mut menu = a_bar();
        assert_eq!(menu.mnemonic('F'), None);
        assert_eq!(menu.open_path(), &[0]);
        assert_eq!(menu.mnemonic('c'), None, "Close Tab is disabled");
        assert!(menu.is_open(), "and the menu is still up");
        assert_eq!(menu.mnemonic('z'), None, "no such mnemonic");
    }

    #[test]
    fn the_label_and_its_underline_come_from_one_string() {
        let item = Item::command("Open &Set…", "", 1);
        assert_eq!(item.text(), "Open Set…");
        assert_eq!(item.mnemonic_at(), Some(5));
        assert_eq!(item.mnemonic(), Some('s'));
        assert_eq!(
            item.text().chars().nth(5),
            Some('S'),
            "the offset indexes the drawn text, not the label"
        );
    }

    #[test]
    fn a_label_with_no_marker_has_no_mnemonic_and_an_escaped_one_is_literal() {
        let plain = Item::command("Save", "", 1);
        assert_eq!(plain.mnemonic(), None);
        assert_eq!(plain.text(), "Save");
        let escaped = Item::command("Fish && Chips", "", 1);
        assert_eq!(escaped.text(), "Fish & Chips");
        assert_eq!(escaped.mnemonic(), None);
    }

    #[test]
    fn hovering_moves_the_keyboard_and_opens_a_submenu_under_the_pointer() {
        let mut menu = a_bar();
        menu.open_top(1);
        menu.hover(&[1, 2]);
        assert_eq!(menu.selected(), &[1, 2]);
        assert_eq!(menu.open_path(), &[1, 2], "a submenu opens on hover");
        menu.hover(&[0, 3]);
        assert_eq!(
            menu.selected(),
            &[1, 2],
            "a disabled item does not take the keyboard"
        );
    }

    #[test]
    fn a_path_that_leads_nowhere_is_none_rather_than_a_panic() {
        let menu = a_bar();
        assert!(menu.item(&[9]).is_none());
        assert!(menu.item(&[0, 9]).is_none());
        assert!(menu.item(&[]).is_none());
        assert!(menu.at(&[0, 0]).is_none(), "Open… is not a submenu");
        assert!(menu.at(&[9]).is_none());
    }

    #[test]
    fn stepping_with_nothing_selected_lands_on_the_first_usable_item() {
        let mut menu = a_bar();
        menu.open_top(0);
        menu.close();
        menu.open = vec![0];
        menu.step(1);
        assert_eq!(menu.selected(), &[0, 0]);
    }

    #[test]
    fn every_id_and_accelerator_can_be_walked_for_a_check_against_the_key_map() {
        let menu = a_bar();
        assert_eq!(menu.ids(), vec![1, 2, 3, 10, 11, 20]);
        let accelerators = menu.accelerators();
        assert!(accelerators.contains(&("Open…".to_owned(), "Ctrl+O".to_owned())));
        assert!(
            accelerators.iter().all(|(_, key)| !key.is_empty()),
            "an item with no accelerator is not listed as having one"
        );
    }

    #[test]
    fn a_check_carries_its_state_without_being_a_different_kind_of_command() {
        let menu = a_bar();
        let follow = &menu.at(&[1]).expect("View")[0];
        assert_eq!(follow.kind, Kind::Check);
        assert!(follow.checked);
        assert_eq!(follow.id, Some(10));
        assert!(follow.selectable());
    }

    #[test]
    fn an_empty_bar_does_not_panic_on_any_of_the_keys() {
        let mut menu = Menu::bar(Vec::new());
        assert!(!menu.is_open());
        menu.open_top(0);
        menu.step(1);
        assert_eq!(menu.across(1), None);
        assert_eq!(menu.enter(), None);
        assert_eq!(menu.mnemonic('a'), None);
        assert!(!menu.escape());
        assert!(menu.ids().is_empty());
    }
}
