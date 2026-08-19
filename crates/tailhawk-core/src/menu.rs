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

/// One character, case-folded for matching a typed mnemonic.
///
/// `to_ascii_lowercase` would leave `Ü` alone and so make `&Ünicode` reachable only by the shifted
/// key. A fold that produces several characters — `ẞ` — keeps the first, which is the letter on the
/// key the user presses.
fn fold(c: char) -> char {
    c.to_lowercase().next().unwrap_or(c)
}

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

    /// The drawn text and where its underline goes, read from the one label in one pass — so the
    /// letter that is underlined and the letter that is typed cannot disagree.
    ///
    /// `&&` is a literal ampersand, as it is everywhere on Windows: `Fish && Chips` draws
    /// `Fish & Chips` and marks no mnemonic.
    fn parsed(&self) -> (String, Option<usize>) {
        let mut text = String::with_capacity(self.label.len());
        let mut at = None;
        let mut count = 0;
        let mut mark = false;
        for c in self.label.chars() {
            match (mark, c) {
                (false, '&') => mark = true,
                (true, '&') => {
                    text.push('&');
                    count += 1;
                    mark = false;
                }
                (true, c) => {
                    at.get_or_insert(count);
                    text.push(c);
                    count += 1;
                    mark = false;
                }
                (false, c) => {
                    text.push(c);
                    count += 1;
                }
            }
        }
        (text, at)
    }

    /// The label without its mnemonic markers, which is what a painter draws.
    pub fn text(&self) -> String {
        self.parsed().0
    }

    /// Where the underline goes, as a **character** offset into [`Item::text`], or `None` when the
    /// label marks no mnemonic.
    pub fn mnemonic_at(&self) -> Option<usize> {
        self.parsed().1
    }

    /// The letter that opens or runs this item, case-folded.
    pub fn mnemonic(&self) -> Option<char> {
        let (text, at) = self.parsed();
        text.chars().nth(at?).map(fold)
    }

    /// Whether the keyboard may land on it: a separator never, a disabled item never.
    pub fn selectable(&self) -> bool {
        self.enabled && self.kind != Kind::Separator
    }
}

/// A menu bar, or one context menu.
///
/// **The two differ in exactly one number**, and every navigation rule is written against it:
/// [`Menu::root_depth`] — the length `open` has when the *outermost* list is showing. A bar's
/// headings sit on a row, so its outermost list is one level down (`open == [k]`); a context menu
/// *is* its outermost list (`open == []`). Writing the rules against the bar's depth and letting a
/// context menu inherit them is what made `Esc` inside a context submenu tear the whole menu down.
///
/// `selected` is always `open` plus one index into that list, or empty. Nothing may leave it
/// pointing outside the list that is drawn — a highlight the user can see on a list they cannot is
/// how a mnemonic ends up running a command from somewhere else.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Menu {
    items: Vec<Item>,
    open: Vec<usize>,
    selected: Vec<usize>,
    /// Whether a list is drawn. Held explicitly rather than derived, as `palette.rs` holds its
    /// own: a context menu's `open` is empty *while it is showing*, so emptiness cannot mean shut.
    shown: bool,
    /// `Alt` on a bar: the headings have the keyboard and nothing is down yet. `UI-DESIGN.md` §12
    /// makes this a state of its own, and without it a bare arrow key opens a menu nobody asked for.
    focused: bool,
    bar: bool,
}

impl Default for Menu {
    /// An empty **bar**, closed. A `Default` that produced a context menu would be one that reports
    /// itself open before anything has been put in it.
    fn default() -> Menu {
        Menu::bar(Vec::new())
    }
}

impl Menu {
    /// §2.1's menu bar: `items` are the headings, and nothing is open yet.
    pub fn bar(items: Vec<Item>) -> Menu {
        Menu {
            items,
            open: Vec::new(),
            selected: Vec::new(),
            shown: false,
            focused: false,
            bar: true,
        }
    }

    /// `SPEC.md` §6.5's right-click: a menu that is already showing, with no headings above it.
    pub fn context(items: Vec<Item>) -> Menu {
        let mut menu = Menu {
            items,
            open: Vec::new(),
            selected: Vec::new(),
            shown: true,
            focused: false,
            bar: false,
        };
        menu.select_first();
        menu
    }

    pub fn is_bar(&self) -> bool {
        self.bar
    }

    /// The length `open` has when the outermost list is showing. See the type's note.
    fn root_depth(&self) -> usize {
        usize::from(self.bar)
    }

    /// Whether a list is drawn.
    pub fn is_open(&self) -> bool {
        self.shown
    }

    /// Whether the bar has the keyboard with nothing dropped down — `Alt`, and no more.
    pub fn is_focused(&self) -> bool {
        self.focused
    }

    pub fn items(&self) -> &[Item] {
        &self.items
    }

    /// The path to the list that is drawn, deepest last. Empty for a context menu's own list.
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
        self.at(path)?.iter().position(Item::selectable)
    }

    /// Puts the keyboard on the first usable item of whatever list is open.
    fn select_first(&mut self) {
        let path = self.open.clone();
        self.selected = match self.first_selectable(&path) {
            Some(i) => [path, vec![i]].concat(),
            None => Vec::new(),
        };
    }

    /// A heading that can actually be dropped down: enabled, and a submenu.
    fn openable(&self, top: usize) -> bool {
        self.items
            .get(top)
            .is_some_and(|i| i.selectable() && i.kind == Kind::Submenu)
    }

    /// `Alt`: the bar takes the keyboard with nothing open. A second `Alt` gives it back.
    pub fn focus(&mut self) {
        if !self.bar {
            return;
        }
        if self.focused || self.shown {
            self.close();
            return;
        }
        self.focused = true;
        self.shown = false;
        self.open.clear();
        self.selected = match self.items.iter().position(|i| i.selectable()) {
            Some(i) => vec![i],
            None => Vec::new(),
        };
    }

    /// Drops a heading open, or shuts it if it already is. `top` indexes the bar.
    pub fn open_top(&mut self, top: usize) {
        if !self.bar || !self.openable(top) {
            return;
        }
        if self.shown && self.open.first() == Some(&top) {
            self.close();
            return;
        }
        self.open = vec![top];
        self.shown = true;
        self.focused = false;
        self.select_first();
    }

    /// Closes everything. A bar keeps its headings and loses the keyboard; a context menu is done.
    pub fn close(&mut self) {
        self.open.clear();
        self.selected.clear();
        self.shown = false;
        self.focused = false;
    }

    /// `Esc`: shuts the deepest submenu and puts the highlight back on the item that opened it,
    /// which is where Windows leaves it. Only at the outermost list does it close the menu.
    ///
    /// Reports whether anything is still showing.
    pub fn escape(&mut self) -> bool {
        if self.shown && self.open.len() > self.root_depth() {
            let parent = self.open.clone();
            self.open.pop();
            self.selected = parent;
            return true;
        }
        self.close();
        false
    }

    /// `↑` / `↓` within the open list, skipping separators and disabled items. Does not wrap: a
    /// list that wraps makes the last item hard to reach by holding a key.
    ///
    /// On a bar that is merely focused, `↓` drops the highlighted heading open instead.
    pub fn step(&mut self, by: isize) {
        if self.focused && !self.shown {
            if by > 0 {
                if let Some(top) = self.selected.first().copied() {
                    self.open_top(top);
                }
            }
            return;
        }
        if !self.shown {
            return;
        }
        let head = self.open.clone();
        let Some(items) = self.at(&head) else { return };
        let Some(last) = self
            .selected
            .get(head.len())
            .copied()
            .filter(|_| self.selected.len() == head.len() + 1)
        else {
            self.select_first();
            return;
        };
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

    /// `→` opens the submenu under the highlight; `←` shuts the one that is open. Failing either,
    /// and only on a bar, they walk to the next usable heading.
    ///
    /// One gesture rather than two, which is how every Windows menu behaves.
    pub fn across(&mut self, by: isize) {
        if self.shown {
            // Inside a list — `selected` is one deeper than `open` — a submenu takes the arrow.
            let inside = self.selected.len() == self.open.len() + 1;
            if by > 0
                && inside
                && self
                    .item(&self.selected.clone())
                    .is_some_and(|i| i.kind == Kind::Submenu)
            {
                self.enter();
                return;
            }
            if by < 0 && self.open.len() > self.root_depth() {
                self.escape();
                return;
            }
        }
        if !self.bar {
            return;
        }
        self.walk_headings(by);
    }

    /// Moves along the bar to the next heading that can be used, wrapping. Keeps whatever the bar
    /// was doing: a focused bar stays focused, an open one opens the heading it lands on.
    fn walk_headings(&mut self, by: isize) {
        let count = self.items.len() as isize;
        if count == 0 || by == 0 {
            return;
        }
        let from = if self.shown {
            *self.open.first().unwrap_or(&0)
        } else if self.focused {
            *self.selected.first().unwrap_or(&0)
        } else {
            return;
        } as isize;
        let mut at = from;
        for _ in 0..count {
            at = (at + by).rem_euclid(count);
            let top = at as usize;
            if !self.items[top].selectable() {
                continue;
            }
            if self.shown {
                self.open = Vec::new();
                self.shown = false;
                self.open_top(top);
            } else {
                self.selected = vec![top];
            }
            return;
        }
    }

    /// `Enter`, or a click: opens a submenu, or reports the command chosen and closes.
    pub fn enter(&mut self) -> Option<u32> {
        if self.focused && !self.shown {
            if let Some(top) = self.selected.first().copied() {
                self.open_top(top);
            }
            return None;
        }
        let path = self.selected.clone();
        let item = self.item(&path)?;
        if !item.selectable() {
            return None;
        }
        if item.kind == Kind::Submenu {
            self.open = path;
            self.shown = true;
            self.select_first();
            return None;
        }
        let id = item.id;
        self.close();
        id
    }

    /// Puts the keyboard on `path` — the pointer moving over an item, or a click resolving to one.
    ///
    /// **Only while something is already showing.** Windows switches menus on hover once one is
    /// down, and does nothing at all when none is; a bar that dropped a menu because the pointer
    /// crossed it would be unusable. A submenu under the pointer opens; a sibling that is not one
    /// shuts whatever was open below this level, so the highlight is never on a list that is not
    /// the drawn one.
    pub fn hover(&mut self, path: &[usize]) {
        if !self.shown || path.is_empty() {
            return;
        }
        if !self.item(path).is_some_and(Item::selectable) {
            return;
        }
        let parent = &path[..path.len() - 1];
        if parent.len() < self.root_depth() {
            return;
        }
        self.open = parent.to_vec();
        self.selected = path.to_vec();
        if self.item(path).is_some_and(|i| i.kind == Kind::Submenu) {
            self.open = path.to_vec();
            self.select_first();
            self.selected = path.to_vec();
        }
    }

    /// Hovering a heading of the bar while a menu is down: switches to it, as Windows does.
    pub fn hover_top(&mut self, top: usize) {
        if !self.bar || !(self.shown || self.focused) || !self.openable(top) {
            return;
        }
        if self.shown {
            if self.open.first() != Some(&top) {
                self.open = Vec::new();
                self.shown = false;
                self.open_top(top);
            }
        } else {
            self.selected = vec![top];
        }
    }

    /// A mnemonic letter. On a bar with nothing down it opens the heading; in an open list it runs
    /// the item — which is what `Alt`, `F`, `O` does on Windows.
    ///
    /// Searches **the list the highlight is in**, which is the one being drawn. Reports the command
    /// when the letter chose one.
    pub fn mnemonic(&mut self, letter: char) -> Option<u32> {
        let letter = fold(letter);
        if self.bar && !self.shown {
            let at = self
                .items
                .iter()
                .position(|i| i.selectable() && i.mnemonic() == Some(letter))?;
            self.open_top(at);
            return None;
        }
        if !self.shown {
            return None;
        }
        let path = self.open.clone();
        let at = self
            .at(&path)?
            .iter()
            .position(|i| i.selectable() && i.mnemonic() == Some(letter))?;
        self.selected = [path, vec![at]].concat();
        self.enter()
    }

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
        assert_eq!(
            menu.selected(),
            &[0, 0],
            "on the first item that can be used"
        );
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
        assert_eq!(
            menu.selected(),
            &[0, 0],
            "the top does not wrap to the bottom"
        );
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
        assert_eq!(
            menu.open_path(),
            &[0],
            "the bar wraps even though a list does not"
        );
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
        assert_eq!(
            menu.open_path(),
            &[1, 2],
            "right opened it rather than moving on"
        );
        menu.across(-1);
        assert_eq!(
            menu.open_path(),
            &[1],
            "and left shut it rather than moving back"
        );
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
    fn a_heading_whose_items_are_all_unusable_is_not_a_dead_end() {
        // On a first run with no file open, whole menus are disabled item by item. Descending into
        // one used to leave the selection on the heading, where `→` read it as a submenu, re-opened
        // the same list, and the bar could not be walked rightwards again at all.
        let mut menu = Menu::bar(vec![
            Item::submenu("&File", vec![Item::command("&Open…", "Ctrl+O", 1)]),
            Item::submenu(
                "&Edit",
                vec![Item::command("&Copy", "Ctrl+C", 2).disabled()],
            ),
            Item::submenu("&Help", vec![Item::command("&About", "", 3)]),
        ]);
        menu.open_top(1);
        assert!(
            menu.selected().is_empty(),
            "there is nothing to select in Edit"
        );
        menu.across(1);
        assert_eq!(menu.open_path(), &[2], "and Right still reaches Help");
        menu.across(-1);
        assert_eq!(menu.open_path(), &[1]);
        menu.across(-1);
        assert_eq!(menu.open_path(), &[0]);
    }

    #[test]
    fn a_context_menu_navigates_its_own_submenus_without_being_torn_down() {
        // The bar's outermost list is one level down; a context menu *is* its outermost list. Both
        // `Esc` and `←` used to test the bar's depth, so either one destroyed a context menu from
        // inside a submenu instead of stepping back out of it.
        let mut menu = Menu::context(vec![
            Item::submenu("&More", vec![Item::command("&A", "", 1)]),
            Item::command("&B", "", 2),
        ]);
        assert_eq!(menu.selected(), &[0]);
        menu.across(1);
        assert_eq!(menu.open_path(), &[0], "Right opened the submenu");
        assert_eq!(menu.selected(), &[0, 0]);
        menu.across(-1);
        assert_eq!(menu.open_path(), &[0; 0], "and Left came back out of it");
        assert!(menu.is_open(), "without taking the menu with it");
        menu.across(1);
        assert!(
            menu.escape(),
            "Esc out of a submenu leaves the menu showing"
        );
        assert_eq!(menu.selected(), &[0], "on the item that opened it");
        assert!(!menu.escape(), "and only then closes it");
        assert!(!menu.is_open());
    }

    #[test]
    fn a_context_menu_reports_itself_closed_once_it_is() {
        let mut menu = Menu::context(vec![Item::command("&Copy", "Ctrl+C", 1)]);
        assert!(menu.is_open());
        assert_eq!(menu.enter(), Some(1));
        assert!(
            !menu.is_open(),
            "the shell has no other way to know the popup is finished"
        );
        let mut menu = Menu::context(vec![Item::command("&Copy", "Ctrl+C", 1)]);
        menu.close();
        assert!(!menu.is_open());
        assert!(!Menu::default().is_open(), "and a default menu is not up");
    }

    #[test]
    fn hovering_off_a_submenu_shuts_it_so_a_mnemonic_cannot_reach_inside_it() {
        // The highlight and the open list must name the same list. When `hover` left a deeper
        // `open` behind, typing the mnemonic of the *visible* item did nothing and the mnemonic of
        // an item in the invisible submenu ran instead.
        let mut menu = a_bar();
        menu.open_top(1);
        menu.hover(&[1, 2]);
        assert_eq!(menu.open_path(), &[1, 2], "Theme opened under the pointer");
        menu.hover(&[1, 1]);
        assert_eq!(
            menu.open_path(),
            &[1],
            "and shut again when the pointer left"
        );
        assert_eq!(menu.selected(), &[1, 1]);
        assert_eq!(
            menu.mnemonic('i'),
            Some(11),
            "Invisibles, the one on screen"
        );
    }

    #[test]
    fn a_closed_bar_is_not_opened_by_the_pointer_crossing_it() {
        let mut menu = a_bar();
        menu.hover(&[1, 0]);
        assert!(
            !menu.is_open(),
            "Windows opens on click, then tracks on hover"
        );
        menu.hover_top(1);
        assert!(!menu.is_open());
        menu.open_top(0);
        menu.hover_top(1);
        assert_eq!(menu.open_path(), &[1], "once one is down, hover switches");
    }

    #[test]
    fn alt_focuses_the_bar_without_dropping_anything_down() {
        let mut menu = a_bar();
        menu.focus();
        assert!(menu.is_focused());
        assert!(!menu.is_open(), "§12: Alt alone is a state of its own");
        assert_eq!(menu.selected(), &[0]);
        menu.across(1);
        assert_eq!(menu.selected(), &[1], "the arrows walk the headings");
        assert!(!menu.is_open(), "and still nothing is down");
        menu.step(1);
        assert!(menu.is_open(), "Down is what opens it");
        assert_eq!(menu.open_path(), &[1]);
        menu.focus();
        assert!(
            !menu.is_open() && !menu.is_focused(),
            "a second Alt gives it back"
        );
    }

    #[test]
    fn a_bare_arrow_does_nothing_to_a_bar_nobody_has_touched() {
        let mut menu = a_bar();
        menu.across(1);
        assert!(!menu.is_open(), "Right on an untouched bar opened a menu");
        menu.step(1);
        assert!(!menu.is_open());
        assert!(menu.selected().is_empty());
    }

    #[test]
    fn a_disabled_heading_can_be_neither_opened_nor_arrowed_into() {
        let mut menu = Menu::bar(vec![
            Item::submenu("&File", vec![Item::command("&A", "", 1)]),
            Item::submenu("&Edit", vec![Item::command("&B", "", 2)]).disabled(),
            Item::submenu("&View", vec![Item::command("&C", "", 3)]),
        ]);
        menu.open_top(1);
        assert!(!menu.is_open(), "a disabled heading does not drop down");
        menu.open_top(0);
        menu.across(1);
        assert_eq!(menu.open_path(), &[2], "and the arrows step over it");
        assert_eq!(menu.mnemonic('e'), None);
    }

    #[test]
    fn a_mnemonic_is_folded_for_the_key_the_user_actually_presses() {
        let mut menu = Menu::bar(vec![Item::submenu(
            "&Ünicode",
            vec![Item::command("&Ärger", "", 7)],
        )]);
        assert_eq!(menu.mnemonic('ü'), None, "the unshifted key opens it");
        assert_eq!(menu.open_path(), &[0]);
        assert_eq!(menu.mnemonic('ä'), Some(7));
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
        menu.across(1);
        assert_eq!(menu.enter(), None);
        assert_eq!(menu.mnemonic('a'), None);
        assert!(!menu.escape());
        assert!(menu.ids().is_empty());
    }
}
