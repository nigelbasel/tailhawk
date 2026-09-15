//! §2.1's docked filter panel — the owner's refinement of the classic-dialogs decision, pointing
//! at Visual Studio's tool windows: filters are toggled constantly while reading, so their surface
//! is non-modal, fixed above the status bar in the band the record-detail pane already proved.
//!
//! The shape is TextAnalysisTool.NET's — the tool `SPEC.md` §7.3 names as the model: one row per
//! filter with its enabled mark, polarity and text. **No floating and no drag-docking in v1**, per
//! §2.1 as resettled.
//!
//! **Since 2026-09-15 the panel is Windows controls, not a drawing** — the owner's choice, following
//! UX-REVIEW part two finding 4: the painted panel had no tab stops, so no filter could be reached
//! from the keyboard, and a screen reader saw only what a hand-built element chose to say.
//!
//! **The pure half is at the top**: chips in, the list's rows, the band's height and which check
//! boxes changed out — the height as one arithmetic answer asked by both the reserver and the
//! placer, because reserving from one number and placing from another is how the status bar once
//! ended up a different size from its hole. **[`FilterPanel`] is the Win32 half**: a child dialog
//! holding a checkbox list view, deciding nothing.
//!
//! **No buttons.** The first native panel had `Add…`, `Edit…`, `Remove` and `Clear all` along its
//! top, carried over from the drawing; the owner, 2026-09-15: "surely commands should all be on menu
//! and toolbar?" Every one is an Edit menu command, and the row's own menu has the rest.

use tailhawk_core::filter::{Chip, Polarity};

/// One filter as the panel's list view shows it: a check box, the words, and which way it cuts.
///
/// **The polarity is a word, not only a tint.** The drawn panel showed a coloured `+` or `−`; a
/// list row says `Include` or `Exclude`, and the tint reinforces it — *Accessibility*: colour
/// reinforces a meaning, it does not carry it, and a screen reader cannot read a colour.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListRow {
    pub checked: bool,
    pub include: bool,
    pub text: String,
}

impl ListRow {
    /// The polarity column's text.
    pub fn polarity(&self) -> &'static str {
        if self.include {
            "Include"
        } else {
            "Exclude"
        }
    }
}

/// The chips as the list view's rows, in chip order — order is display-only, per §7.2.
pub fn list_rows_of(chips: &[Chip]) -> Vec<ListRow> {
    chips
        .iter()
        .map(|chip| ListRow {
            checked: chip.enabled,
            include: chip.polarity == Polarity::Include,
            text: chip.source.clone(),
        })
        .collect()
}

/// The most list rows the panel shows before the list scrolls instead of growing.
pub const MAX_LIST_ROWS: usize = 6;

/// The air around the controls, in pixels, above, between and below.
const LIST_PAD_PX: f32 = 6.0;

/// The band's height for the native panel: the list inside its frame, or nothing when hidden.
///
/// **The list grows a row per chip up to [`MAX_LIST_ROWS`] and then scrolls**, because a panel that
/// grew without limit would take the log it is filtering off the screen. An empty list still shows
/// one row, so the first filter has somewhere visible to land. `frame_px` is the list's border,
/// top and bottom together, which the rows sit inside and must not be reserved from.
pub fn band_height(chips: usize, visible: bool, row_px: f32, frame_px: f32) -> f32 {
    if !visible {
        return 0.0;
    }
    let rows = chips.clamp(1, MAX_LIST_ROWS) as f32;
    RULE_PX + LIST_PAD_PX + frame_px + rows * row_px + LIST_PAD_PX
}

/// Which chips a list view's check boxes now disagree with — the chips to flip.
///
/// `checks` is the boxes as the list view holds them, which may be fewer than the chips while the
/// list is being filled; only the rows it has are compared.
pub fn flipped(rows: &[ListRow], checks: &[bool]) -> Vec<usize> {
    rows.iter()
        .zip(checks)
        .enumerate()
        .filter(|(_, (row, checked))| row.checked != **checked)
        .map(|(at, _)| at)
        .collect()
}

/// The list view's control id inside the panel. Above every id the dialogs use, so a notification
/// that reaches the main window cannot be mistaken for one of theirs.
pub const ID_P_LIST: u16 = 3_110;

/// The panel's one control, as a template. The size is a starting point only:
/// [`FilterPanel::lay_out`] replaces it from the band the grid reserves, every time the band moves.
fn panel_items() -> Vec<crate::dialog::Item> {
    use crate::dialog::{
        Class, Item, LVS_REPORT, LVS_SHOWSELALWAYS, LVS_SINGLESEL, WS_BORDER, WS_TABSTOP,
    };
    const LVS_NOCOLUMNHEADER: u32 = 0x4000;
    vec![Item::new(
        Class::Named("SysListView32"),
        "",
        ID_P_LIST,
        (4, 4, 300, 40),
        WS_BORDER
            | WS_TABSTOP
            | LVS_REPORT
            | LVS_SINGLESEL
            | LVS_SHOWSELALWAYS
            | LVS_NOCOLUMNHEADER,
    )]
}

thread_local! {
    /// Whether the list view's changes right now are the panel's own. Filling the list raises
    /// `LVN_ITEMCHANGED` once a row, synchronously — the hazard `dialog.rs`'s `APPS_QUIET` documents —
    /// and a fill read back as a user's click would flip every chip it inserted.
    static PANEL_QUIET: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn quietly<R>(write: impl FnOnce() -> R) -> R {
    PANEL_QUIET.with(|q| q.set(true));
    let out = write();
    PANEL_QUIET.with(|q| q.set(false));
    out
}

/// §2.1's filter panel as **Windows controls** — the owner's choice of 2026-09-15, following
/// UX-REVIEW part two finding 4.
///
/// The painted panel was glyphs and click rectangles: nothing in it was a tab stop, so the
/// keyboard could not reach a single filter, and nothing in it was a window, so a screen reader saw
/// only what a hand-built accessibility element chose to say. A list view with check boxes gets a
/// tab stop, selection, `Space` to toggle and a screen reader's reading of every row from Windows,
/// without any of it being written here.
///
/// **It decides nothing.** Which rows and how tall the band is come from the pure functions above;
/// this inserts them and routes what the user does back to the shell.
pub struct FilterPanel {
    hwnd: windows::Win32::Foundation::HWND,
    /// What the list was last filled with, so a frame that changes nothing touches nothing — a
    /// refill under a user's pointer would drop the selection they are about to act on.
    shown: Vec<ListRow>,
    visible: bool,
    /// The background the panel and its labels are painted with, from the theme. Owned, because a
    /// `WM_CTLCOLOR*` answer must be a brush that outlives the message, and **per panel**, kept in
    /// the panel window's `GWLP_USERDATA` for `panel_proc` to answer with: every tab has its own
    /// panel, and one brush shared between them was deleted by whichever tab closed first while the
    /// others were still painting with it. Not read from the shell either — the panel paints
    /// synchronously while the frame places it, inside the `STATE` borrow.
    brush: windows::Win32::Graphics::Gdi::HBRUSH,
}

impl FilterPanel {
    /// Creates the panel as a child of `parent`, hidden until the shell places it.
    pub fn create(parent: windows::Win32::Foundation::HWND) -> Option<FilterPanel> {
        use windows::Win32::Foundation::{LPARAM, WPARAM};
        use windows::Win32::UI::Controls::LVM_SETEXTENDEDLISTVIEWSTYLE;
        use windows::Win32::UI::WindowsAndMessaging::{
            CreateDialogIndirectParamW, SendMessageW, DLGTEMPLATE,
        };
        let template = crate::dialog::template_child(310, 64, &panel_items());
        let hwnd = unsafe {
            CreateDialogIndirectParamW(
                None,
                template.as_ptr() as *const DLGTEMPLATE,
                parent,
                Some(panel_proc),
                LPARAM(0),
            )
        }
        .ok()?;
        let mut panel = FilterPanel {
            hwnd,
            shown: Vec::new(),
            visible: false,
            brush: windows::Win32::Graphics::Gdi::HBRUSH::default(),
        };
        if let Some(list) = panel.list() {
            unsafe {
                SendMessageW(
                    list,
                    LVM_SETEXTENDEDLISTVIEWSTYLE,
                    WPARAM(0),
                    LPARAM(
                        (crate::dialog::LVS_EX_CHECKBOXES | crate::dialog::LVS_EX_FULLROWSELECT)
                            as isize,
                    ),
                );
            }
            quietly(|| {
                crate::dialog::lv_reset(list);
                crate::dialog::lv_column(list, 0, "Filter", 240);
                crate::dialog::lv_column(list, 1, "Kind", 70);
            });
        }
        panel.adopt_theme();
        Some(panel)
    }

    pub fn hwnd(&self) -> windows::Win32::Foundation::HWND {
        self.hwnd
    }

    /// The list view inside the panel.
    pub fn list(&self) -> Option<windows::Win32::Foundation::HWND> {
        self.item(ID_P_LIST)
    }

    fn item(&self, id: u16) -> Option<windows::Win32::Foundation::HWND> {
        unsafe { windows::Win32::UI::WindowsAndMessaging::GetDlgItem(self.hwnd, i32::from(id)) }
            .ok()
    }

    /// Re-reads the theme: the dark or light class on every control, and the brush the panel's own
    /// background is painted with. Called at creation and whenever the theme changes.
    pub fn adopt_theme(&mut self) {
        use windows::Win32::Graphics::Gdi::{CreateSolidBrush, DeleteObject, HGDIOBJ};
        let theme = tailhawk_core::theme::theme();
        let dark = theme.dark;
        crate::controls::apply_theme(self.hwnd, dark);
        if let Some(list) = self.list() {
            use windows::core::w;
            use windows::Win32::Foundation::{LPARAM, WPARAM};
            use windows::Win32::UI::Controls::{
                LVM_SETBKCOLOR, LVM_SETTEXTBKCOLOR, LVM_SETTEXTCOLOR,
            };
            use windows::Win32::UI::WindowsAndMessaging::SendMessageW;
            crate::controls::apply_theme_class(
                list,
                dark,
                w!("DarkMode_ItemsView"),
                w!("ItemsView"),
            );
            // The theme class dresses the scroll bars and the selection, not the ground: without
            // its colours set, a report list stays white in the dark theme.
            let ground = LPARAM(colourref(theme.pane_bg) as isize);
            unsafe {
                SendMessageW(list, LVM_SETBKCOLOR, WPARAM(0), ground);
                SendMessageW(list, LVM_SETTEXTBKCOLOR, WPARAM(0), ground);
                SendMessageW(
                    list,
                    LVM_SETTEXTCOLOR,
                    WPARAM(0),
                    LPARAM(colourref(theme.ink) as isize),
                );
            }
        }
        let fresh = unsafe {
            CreateSolidBrush(windows::Win32::Foundation::COLORREF(colourref(
                theme.pane_bg,
            )))
        };
        unsafe {
            windows::Win32::UI::WindowsAndMessaging::SetWindowLongPtrW(
                self.hwnd,
                windows::Win32::UI::WindowsAndMessaging::GWLP_USERDATA,
                fresh.0 as isize,
            );
        }
        let old = std::mem::replace(&mut self.brush, fresh);
        if !old.is_invalid() {
            unsafe {
                let _ = DeleteObject(HGDIOBJ(old.0));
            }
        }
    }

    /// Fills the list from the model, touching it only when the rows changed.
    pub fn set(&mut self, rows: &[ListRow], selected: Option<usize>) {
        if rows == self.shown.as_slice() {
            return;
        }
        let Some(list) = self.list() else {
            return;
        };
        quietly(|| {
            use windows::Win32::Foundation::{LPARAM, WPARAM};
            use windows::Win32::UI::Controls::LVM_DELETEALLITEMS;
            use windows::Win32::UI::WindowsAndMessaging::SendMessageW;
            unsafe {
                SendMessageW(list, LVM_DELETEALLITEMS, WPARAM(0), LPARAM(0));
            }
            for (at, row) in rows.iter().enumerate() {
                crate::dialog::lv_row(
                    list,
                    at as i32,
                    &[row.text.clone(), row.polarity().to_owned()],
                );
                crate::dialog::lv_check(list, at as i32, row.checked);
            }
            if let Some(at) = selected.filter(|at| *at < rows.len()) {
                crate::dialog::lv_select(list, at);
            }
        });
        self.shown = rows.to_vec();
    }

    /// The check boxes as the list view holds them, for [`flipped`].
    pub fn checks(&self) -> Vec<bool> {
        use windows::Win32::Foundation::{LPARAM, WPARAM};
        use windows::Win32::UI::Controls::LVM_GETITEMCOUNT;
        use windows::Win32::UI::WindowsAndMessaging::SendMessageW;
        let Some(list) = self.list() else {
            return Vec::new();
        };
        let items = unsafe { SendMessageW(list, LVM_GETITEMCOUNT, WPARAM(0), LPARAM(0)) }.0;
        (0..items.max(0))
            .map(|at| crate::dialog::lv_checked(list, at as i32))
            .collect()
    }

    /// The row the list view has selected, if any.
    pub fn selected(&self) -> Option<usize> {
        self.list().and_then(crate::dialog::lv_selected)
    }

    /// Records that the list's checks now match the model, after the shell has flipped the chips a
    /// click changed — so the next `set` compares against what is really on screen.
    pub fn accept_checks(&mut self, rows: &[ListRow]) {
        self.shown = rows.to_vec();
    }

    /// Puts the panel over the band the grid reserves, or hides it.
    pub fn place(&mut self, x: i32, top: i32, width: i32, height: i32, visible: bool) {
        use windows::Win32::UI::WindowsAndMessaging::{
            SetWindowPos, ShowWindow, HWND_TOP, SWP_NOACTIVATE, SWP_NOZORDER, SW_HIDE, SW_SHOWNA,
        };
        if visible {
            unsafe {
                let _ = SetWindowPos(
                    self.hwnd,
                    HWND_TOP,
                    x,
                    top,
                    width.max(0),
                    height.max(0),
                    SWP_NOACTIVATE | SWP_NOZORDER,
                );
            }
            self.lay_out();
        }
        if visible != self.visible {
            unsafe {
                let _ = ShowWindow(self.hwnd, if visible { SW_SHOWNA } else { SW_HIDE });
            }
            self.visible = visible;
        }
    }

    /// The list filling the panel inside the air around it.
    ///
    /// **In the same unscaled pixels [`band_height`] reserves**, not scaled by the window's DPI: the
    /// band is the rule, the pads, the frame and the rows, and a layout that grew the pads by the
    /// scale while the band did not took the difference out of the rows — at 150% the second row
    /// showed half its height.
    pub fn lay_out(&self) {
        use windows::Win32::Foundation::RECT;
        use windows::Win32::UI::WindowsAndMessaging::{
            GetClientRect, SetWindowPos, HWND_TOP, SWP_NOACTIVATE, SWP_NOZORDER,
        };
        let mut client = RECT::default();
        if unsafe { GetClientRect(self.hwnd, &mut client) }.is_err() {
            return;
        }
        let pad = LIST_PAD_PX as i32;
        let top = RULE_PX as i32 + pad;
        if let Some(list) = self.list() {
            unsafe {
                let _ = SetWindowPos(
                    list,
                    HWND_TOP,
                    pad,
                    top,
                    (client.right - pad * 2).max(0),
                    (client.bottom - top - pad).max(0),
                    SWP_NOACTIVATE | SWP_NOZORDER,
                );
            }
        }
    }

    /// The height of one list row and of the list's frame — its border, top and bottom together —
    /// in pixels, for [`band_height`]. A row is measured from the first item, so an empty list
    /// answers with a stand-in; the shell fills the list before it asks.
    pub fn metrics(&self) -> (f32, f32) {
        use windows::Win32::Foundation::{LPARAM, RECT, WPARAM};
        use windows::Win32::UI::Controls::LVM_GETITEMRECT;
        use windows::Win32::UI::WindowsAndMessaging::{GetClientRect, GetWindowRect, SendMessageW};
        let Some(list) = self.list() else {
            return (20.0, 4.0);
        };
        let (mut outer, mut inner, mut item) = (RECT::default(), RECT::default(), RECT::default());
        let got = unsafe {
            let _ = GetWindowRect(list, &mut outer);
            let _ = GetClientRect(list, &mut inner);
            SendMessageW(
                list,
                LVM_GETITEMRECT,
                WPARAM(0),
                LPARAM(&mut item as *mut RECT as isize),
            )
        };
        let frame = ((outer.bottom - outer.top) - (inner.bottom - inner.top)).max(0) as f32;
        let row = if got.0 != 0 {
            (item.bottom - item.top) as f32
        } else {
            20.0
        };
        (row, frame)
    }

    /// Gives the keyboard to the list — F6's landing.
    pub fn focus(&self) {
        use windows::Win32::UI::Input::KeyboardAndMouse::SetFocus;
        if let Some(list) = self.list() {
            unsafe {
                let _ = SetFocus(list);
            }
        }
    }

    /// Whether the panel is on screen — what F6 and the message loop ask before offering it the
    /// keyboard.
    pub fn is_visible(&self) -> bool {
        self.visible
    }

    /// Whether the keyboard is somewhere inside the panel.
    pub fn has_focus(&self) -> bool {
        use windows::Win32::UI::Input::KeyboardAndMouse::GetFocus;
        use windows::Win32::UI::WindowsAndMessaging::IsChild;
        let focus = unsafe { GetFocus() };
        !focus.is_invalid()
            && (focus == self.hwnd || unsafe { IsChild(self.hwnd, focus) }.as_bool())
    }
}

impl Drop for FilterPanel {
    fn drop(&mut self) {
        use windows::Win32::Graphics::Gdi::{DeleteObject, HGDIOBJ};
        use windows::Win32::UI::WindowsAndMessaging::DestroyWindow;
        unsafe {
            let _ = DestroyWindow(self.hwnd);
            if !self.brush.is_invalid() {
                let _ = DeleteObject(HGDIOBJ(self.brush.0));
            }
        }
    }
}

/// SAFETY: the panel is created by the shell on the window thread, the first frame its document
/// shows filters, and every message to it is sent from that thread; nothing here touches its
/// handle or its brush from anywhere else. The impl exists for the reason `header::Header`'s does:
/// a [`Document`](crate::Document) is built on a worker and sent to the window thread over a
/// channel, and a `Document` carries an `Option<FilterPanel>` — always `None` at that moment, the
/// panel being made only once the shell lays the pane out. An `HWND` and an `HBRUSH` are integers
/// the system owns; moving the integers between threads is not the same as using them from one.
unsafe impl Send for FilterPanel {}

/// A theme colour as GDI's `0x00BBGGRR`.
fn colourref(colour: tailhawk_core::theme::Colour) -> u32 {
    let byte = |c: f32| (c.clamp(0.0, 1.0) * 255.0).round() as u32;
    byte(colour[0]) | (byte(colour[1]) << 8) | (byte(colour[2]) << 16)
}

/// `NM_DBLCLK`, `NM_RETURN` and `LVN_KEYDOWN`, which the crate does not bind as `u32`s.
const NM_DBLCLK: u32 = (-3_i32) as u32;
const NM_RETURN: u32 = (-4_i32) as u32;
const LVN_KEYDOWN: u32 = (-155_i32) as u32;

/// `NMLVKEYDOWN`, laid out by hand: the header, the virtual key, and flags nobody reads.
#[repr(C)]
struct NmLvKeyDown {
    hdr: windows::Win32::UI::Controls::NMHDR,
    key: u16,
    flags: u32,
}

/// The panel's procedure. **It decides nothing**: every gesture goes to the shell, which owns the
/// chips, through the functions `main.rs` exposes for exactly this.
unsafe extern "system" fn panel_proc(
    hdlg: windows::Win32::Foundation::HWND,
    msg: u32,
    wparam: windows::Win32::Foundation::WPARAM,
    lparam: windows::Win32::Foundation::LPARAM,
) -> isize {
    use windows::Win32::Graphics::Gdi::{SetBkColor, SetTextColor, HDC};
    use windows::Win32::UI::Controls::{LVN_ITEMCHANGED, NMHDR};
    use windows::Win32::UI::Input::KeyboardAndMouse::VK_DELETE;
    use windows::Win32::UI::WindowsAndMessaging::{
        WM_COMMAND, WM_CONTEXTMENU, WM_CTLCOLORDLG, WM_CTLCOLORSTATIC, WM_INITDIALOG, WM_NOTIFY,
    };
    match msg {
        WM_INITDIALOG => 0,
        WM_CTLCOLORDLG | WM_CTLCOLORSTATIC => {
            let theme = tailhawk_core::theme::theme();
            let dc = HDC(wparam.0 as *mut core::ffi::c_void);
            unsafe {
                SetTextColor(
                    dc,
                    windows::Win32::Foundation::COLORREF(colourref(theme.ink)),
                );
                SetBkColor(
                    dc,
                    windows::Win32::Foundation::COLORREF(colourref(theme.pane_bg)),
                );
            }
            unsafe {
                windows::Win32::UI::WindowsAndMessaging::GetWindowLongPtrW(
                    hdlg,
                    windows::Win32::UI::WindowsAndMessaging::GWLP_USERDATA,
                )
            }
        }
        WM_COMMAND => {
            let id = (wparam.0 & 0xFFFF) as u16;
            match id {
                // Enter, when the dialog manager turns it into `IDOK` rather than letting the list
                // have it as `NM_RETURN` — either way it is Edit ▸ Edit filter…, and the key goes
                // to one of the two, never both.
                1 => {
                    crate::filter_panel_command(hdlg, crate::Command::EditFilter);
                    1
                }
                // Esc, which the dialog manager turns into `IDCANCEL`: back to the log.
                2 => {
                    crate::filter_panel_escape(hdlg);
                    1
                }
                _ => 0,
            }
        }
        WM_NOTIFY => {
            let header = unsafe { &*(lparam.0 as *const NMHDR) };
            if header.idFrom != usize::from(ID_P_LIST) {
                return 0;
            }
            match header.code {
                LVN_ITEMCHANGED if !PANEL_QUIET.with(|q| q.get()) => {
                    crate::filter_panel_changed(hdlg);
                }
                NM_DBLCLK | NM_RETURN => {
                    crate::filter_panel_command(hdlg, crate::Command::EditFilter)
                }
                LVN_KEYDOWN => {
                    let key = unsafe { &*(lparam.0 as *const NmLvKeyDown) }.key;
                    if key == VK_DELETE.0 {
                        crate::filter_panel_command(hdlg, crate::Command::RemoveFilter);
                    }
                }
                _ => {}
            }
            0
        }
        // A row's menu is the list's. A right-click on the panel's margin reaches here too, and is
        // answered with nothing rather than the selected row's menu — or, returned unhandled, the
        // grid's.
        WM_CONTEXTMENU => {
            let on = windows::Win32::Foundation::HWND(wparam.0 as *mut core::ffi::c_void);
            let list = unsafe {
                windows::Win32::UI::WindowsAndMessaging::GetDlgItem(hdlg, ID_P_LIST.into())
            };
            if list.is_ok_and(|list| list == on) {
                let x = (lparam.0 & 0xFFFF) as i16 as i32;
                let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as i32;
                crate::filter_panel_context(hdlg, x, y);
            }
            1
        }
        _ => 0,
    }
}

/// The rule drawn along the panel's top edge, and the air under it.
pub const RULE_PX: f32 = 1.0;

#[cfg(test)]
mod tests {
    use super::*;

    fn chip(source: &str, include: bool, enabled: bool) -> Chip {
        let polarity = if include {
            Polarity::Include
        } else {
            Polarity::Exclude
        };
        let mut chip = Chip::parse(source, polarity).expect("a plain word parses");
        chip.enabled = enabled;
        chip
    }

    /// **The list view's rows, one per chip, in chip order.** The check box is the chip's enabled
    /// state and the polarity is a word as well as a tint — *Accessibility*: colour reinforces a
    /// meaning, it does not carry it — so a screen reader reading the row says what it does.
    #[test]
    fn a_list_row_carries_the_chips_state_polarity_and_text() {
        let rows = list_rows_of(&[chip("error", true, true), chip("retry", false, false)]);
        assert_eq!(rows.len(), 2);
        assert!(rows[0].checked);
        assert!(rows[0].include);
        assert_eq!(rows[0].text, "error");
        assert_eq!(rows[0].polarity(), "Include");
        assert!(!rows[1].checked, "a disabled chip is an unticked row");
        assert!(!rows[1].include);
        assert_eq!(rows[1].polarity(), "Exclude");
    }

    /// **Hidden is nothing; shown is the list, and every row it shows is whole.** The list grows a
    /// row per chip up to a cap and then scrolls, because a panel that grew without limit would take
    /// the log it is filtering off the screen — and an empty list still shows one row, so the first
    /// filter has somewhere visible to land. The list's own frame is in the band too: reserved from
    /// the rows alone, the border ate into them and the second row showed half its height.
    #[test]
    fn the_band_holds_a_capped_list_of_whole_rows_inside_its_frame() {
        let (row, frame) = (18.0, 4.0);
        assert_eq!(band_height(5, false, row, frame), 0.0, "hidden");
        let one = band_height(1, true, row, frame);
        assert_eq!(
            band_height(0, true, row, frame),
            one,
            "an empty list still shows a row"
        );
        assert_eq!(band_height(3, true, row, frame) - one, 2.0 * row);
        assert_eq!(
            band_height(MAX_LIST_ROWS + 40, true, row, frame),
            band_height(MAX_LIST_ROWS, true, row, frame),
            "past the cap the list scrolls instead of growing"
        );
        assert_eq!(
            band_height(2, true, row, frame) - band_height(2, true, row, 0.0),
            frame,
            "the list's border is reserved on top of its rows"
        );
        assert!(one >= row + frame, "a whole row fits inside the frame");
    }

    /// **Which chips a click on the check boxes changed.** The list view reports a state change,
    /// not which box or which way; comparing the boxes against the chips is the answer that cannot
    /// be read the wrong way round, and it ignores rows the list does not have yet — a fill in
    /// progress raises the same notification for every row it inserts.
    #[test]
    fn a_check_box_that_disagrees_with_its_chip_is_a_flip() {
        let rows = list_rows_of(&[
            chip("a", true, true),
            chip("b", true, false),
            chip("c", false, true),
        ]);
        assert_eq!(flipped(&rows, &[true, false, true]), Vec::<usize>::new());
        assert_eq!(flipped(&rows, &[false, false, true]), vec![0]);
        assert_eq!(flipped(&rows, &[true, true, false]), vec![1, 2]);
        assert_eq!(
            flipped(&rows, &[false]),
            vec![0],
            "only as many rows as the list has"
        );
    }
}
