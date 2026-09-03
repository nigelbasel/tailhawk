//! The toolbar as one frame should draw it — `UI-DESIGN.md` §2.3.
//!
//! **The view-model half.** A [`ToolButton`] is a button as it stands right now: its label, the
//! command id a click sends, whether it can act, and — for the three toggles — whether it is
//! pressed. [`toolbar_of`] is the model → view-model mapping, and it is pure, so a toolbar that
//! disagrees with the menu beside it is a failing test rather than a screenshot someone has to
//! notice.
//!
//! **The Win32 half is at the bottom of this file**, as `tabstrip.rs` keeps its own: [`Toolbar`]
//! creates a real `ToolbarWindow32`, fills it from the view-model above, and decides nothing.
//!
//! **The ids are `menubar::command_id`'s**, not a set of the toolbar's own. A click therefore
//! arrives as the same `WM_COMMAND` the menu sends and goes down the same dispatch, which is what
//! §1.2's *one command, one name, one path* asks for. A toolbar with private ids would be a second
//! implementation of every command it offers, free to drift from the menu without anything saying
//! so.
//!
//! **Text buttons and no image list.** §1.1 rejects "a toolbar of ambiguous, unlabelled 16×16
//! icons" and requires a label beside any icon; the simplest way to keep that promise is to have no
//! icon to keep it about. An icon set is a design commitment nobody has asked for.

use crate::menubar::command_id;
use crate::tabstrip::shell_font;
use crate::{Command, Document};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, RECT, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::{DeleteObject, HFONT, HGDIOBJ};
use windows::Win32::UI::Controls::{
    InitCommonControlsEx, BTNS_AUTOSIZE, BTNS_CHECK, BTNS_SHOWTEXT, CCS_NODIVIDER,
    CCS_NOPARENTALIGN, CCS_NORESIZE, ICC_BAR_CLASSES, INITCOMMONCONTROLSEX, TBBUTTON,
    TBSTATE_CHECKED, TBSTATE_ENABLED, TBSTYLE_FLAT, TBSTYLE_LIST, TB_ADDBUTTONSW, TB_ADDSTRINGW,
    TB_AUTOSIZE, TB_BUTTONCOUNT, TB_BUTTONSTRUCTSIZE, TB_DELETEBUTTON, TB_GETMAXSIZE, TB_SETSTATE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, GetWindowRect, SendMessageW, SetWindowPos, ShowWindow, HMENU,
    HWND_TOP, SWP_NOACTIVATE, SWP_NOZORDER, SW_HIDE, SW_SHOWNA, WINDOW_EX_STYLE, WINDOW_STYLE,
    WM_SETFONT, WS_CHILD, WS_CLIPSIBLINGS,
};

/// `I_IMAGENONE` — the button has no image at all, rather than image zero of an absent list.
///
/// Declared here because the `windows` crate does not bind it; the value is the documented one.
const I_IMAGENONE: i32 = -2;

/// One button, as it stands this frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolButton {
    /// What the button says. The same words as the menu item, without its accelerator ampersand.
    pub label: &'static str,
    /// The command id a click posts — the menu's, deliberately.
    pub id: u32,
    /// Whether the command can act on what is open. A button that cannot is disabled in place
    /// rather than removed, so the row does not move under the pointer.
    pub enabled: bool,
    /// Whether this is a toggle rather than a verb. §2.3: the toggles read as a status display as
    /// well as a control, so they must be drawn pressed and not merely highlighted.
    pub toggle: bool,
    /// Whether a toggle is currently on. Always `false` for a verb.
    pub pressed: bool,
}

/// The toolbar for this document, or for no document at all.
///
/// **Every button except Open needs something open**, which is the whole of the enabling rule for
/// the verbs; the three toggles additionally report their state. With no document the row is a line
/// of greyed labels with `Open` live at its head — deliberately still there, because a toolbar that
/// appears when the first file does would move the grid down the moment a file arrives.
pub fn toolbar_of(doc: Option<&Document>) -> Vec<ToolButton> {
    let open = doc.is_some();
    let verb = |label: &'static str, c: Command, enabled: bool| ToolButton {
        label,
        id: command_id(c),
        enabled,
        toggle: false,
        pressed: false,
    };
    let toggle = |label: &'static str, c: Command, pressed: bool| ToolButton {
        label,
        id: command_id(c),
        enabled: open,
        toggle: true,
        pressed: open && pressed,
    };
    vec![
        verb("Open", Command::OpenFile, true),
        verb("Find", Command::Find, open),
        verb("Filter", Command::ToggleFilters, open),
        toggle(
            "Follow",
            Command::FollowTail,
            doc.is_some_and(|d| d.is_following()),
        ),
        toggle(
            "Collapse",
            Command::ToggleCollapse,
            doc.is_some_and(|d| d.is_collapsed()),
        ),
        toggle(
            "Detail",
            Command::ToggleDetail,
            doc.is_some_and(|d| d.detail_open()),
        ),
        verb("Rules", Command::EditRules, true),
        verb("Format", Command::DefineFormat, open),
        verb("Export", Command::Export, open),
    ]
}

/// The control's own id, so a notification from it can be told apart from the tab strip's.
pub const ID_TOOLBAR: i32 = 4_200;

/// The real `ToolbarWindow32`, and everything about it that needs a window.
///
/// **It decides nothing.** Which buttons exist, whether each can act and whether each is on all
/// come from [`toolbar_of`]; this half inserts them, sets two state bits, and reports how tall the
/// control made itself.
pub struct Toolbar {
    hwnd: HWND,
    font: HFONT,
    /// What the control was last filled with, so a frame that changes nothing does not rebuild it.
    /// A rebuild repaints the whole band and drops a click that is mid-press.
    shown: Vec<ToolButton>,
    visible: bool,
}

impl Toolbar {
    /// Creates the control as a child of the main window, hidden until it is placed.
    ///
    /// `TBSTYLE_LIST` puts the label beside the (absent) image rather than under it, which is what
    /// keeps a text-only toolbar one row high instead of two.
    pub fn create(parent: HWND, instance: HINSTANCE) -> Option<Toolbar> {
        let icc = INITCOMMONCONTROLSEX {
            dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_BAR_CLASSES,
        };
        unsafe {
            let _ = InitCommonControlsEx(&icc);
        }
        let hwnd = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("ToolbarWindow32"),
                PCWSTR::null(),
                // **The three `CCS_*` bits are the ones a toolbar that is positioned by hand must
                // have.** `CCS_TOP` is the control's documented *default*: it re-aligns itself to
                // the top of the parent's client area, at the parent's full width, from inside its
                // own `WM_SIZE` — which `place`'s `SetWindowPos` raises. Without `CCS_NORESIZE` and
                // `CCS_NOPARENTALIGN` the control would snap back to y=0 on the next resize and sit
                // on top of the tab strip while the grid still reserved a band for it.
                // `CCS_NODIVIDER` removes the edge above it, which is chrome from a different era
                // and would also make the measured band disagree with the drawn one.
                //
                // **No `TBSTYLE_TOOLTIPS`.** The style creates a tooltip window, but the text has
                // to come from the parent answering `TTN_GETDISPINFOW`, and nothing does; the style
                // alone produces a tooltip that never says anything. The buttons carry their own
                // labels, which is what §1.1 asked for instead of tooltips in the first place.
                WINDOW_STYLE(
                    WS_CHILD.0
                        | WS_CLIPSIBLINGS.0
                        | TBSTYLE_FLAT
                        | TBSTYLE_LIST
                        | CCS_NODIVIDER as u32
                        | CCS_NOPARENTALIGN as u32
                        | CCS_NORESIZE as u32,
                ),
                0,
                0,
                0,
                0,
                parent,
                HMENU(ID_TOOLBAR as *mut core::ffi::c_void),
                instance,
                None,
            )
        }
        .ok()?;
        // Documented as required before any other TB_ message: it is how comctl32 learns which
        // version of `TBBUTTON` this process was compiled against.
        unsafe {
            SendMessageW(
                hwnd,
                TB_BUTTONSTRUCTSIZE,
                WPARAM(std::mem::size_of::<TBBUTTON>()),
                LPARAM(0),
            );
        }
        let font = shell_font();
        if !font.is_invalid() {
            unsafe {
                SendMessageW(hwnd, WM_SETFONT, WPARAM(font.0 as usize), LPARAM(1));
            }
        }
        // The same theme decision the menus take — see `controls::apply_theme` for the screenshot
        // that made this one place rather than three.
        crate::controls::apply_theme(hwnd, tailhawk_core::theme::theme().dark);
        Some(Toolbar {
            hwnd,
            font,
            shown: Vec::new(),
            visible: false,
        })
    }

    /// Fills the control from the view-model, rebuilding only when the row itself changed.
    ///
    /// **A state change is not a rebuild.** The nine buttons are re-added only when the *row*
    /// differs — different ids, labels or kinds — because re-adding repaints the whole band.
    ///
    /// **The state is re-asserted every frame, and the reason is a latch.** A `BTNS_CHECK` button
    /// flips its own `TBSTATE_CHECKED` before it sends `WM_COMMAND`, so after a click the control
    /// and the model disagree until something puts them back. Comparing against `self.shown` — a
    /// record of what was last *asked for*, not of what the control did to itself — cannot see that
    /// disagreement: click Follow while already following and the command turns following on again,
    /// so `pressed` never changes, no `TB_SETSTATE` is sent, and the button stays visibly unpressed
    /// over a document that is following. The menu does not have this problem because
    /// `WM_INITMENUPOPUP` refills it from the live document every single time it opens; nine
    /// idempotent `TB_SETSTATE` sends are this control's version of that, and cost nothing.
    pub fn set(&mut self, buttons: &[ToolButton]) {
        let same_row = self.shown.len() == buttons.len()
            && self
                .shown
                .iter()
                .zip(buttons)
                .all(|(a, b)| a.id == b.id && a.label == b.label && a.toggle == b.toggle);
        if same_row {
            for now in buttons {
                unsafe {
                    SendMessageW(
                        self.hwnd,
                        TB_SETSTATE,
                        WPARAM(now.id as usize),
                        LPARAM(state_bits(now) as isize),
                    );
                }
            }
        } else if !self.rebuild(buttons) {
            // The control refused them. Forget what we asked for, so the next frame asks again
            // instead of believing a row that is not there.
            self.shown.clear();
            return;
        }
        self.shown = buttons.to_vec();
    }

    /// Re-adds every button. Reports whether the control accepted them.
    ///
    /// **`TB_DELETEBUTTON` does not free the string pool**, so each call appends nine more strings
    /// to it. That costs nothing today because `toolbar_of` always returns the same nine ids,
    /// labels and kinds, so `same_row` is true from the second frame on and this runs exactly once
    /// per control. A row that ever varies — a per-format button, a plug-in — turns that into a
    /// leak, and the fix then is `TB_SETBUTTONINFO` on the existing buttons rather than a rebuild.
    fn rebuild(&mut self, buttons: &[ToolButton]) -> bool {
        while unsafe { SendMessageW(self.hwnd, TB_BUTTONCOUNT, WPARAM(0), LPARAM(0)) }.0 > 0 {
            unsafe {
                SendMessageW(self.hwnd, TB_DELETEBUTTON, WPARAM(0), LPARAM(0));
            }
        }
        // One string per `TB_ADDSTRINGW`, keeping the index it comes back with. The message can
        // take a whole double-null-terminated block at once, and that is the form whose terminator
        // is easy to get wrong for no gain at nine buttons.
        let items: Vec<TBBUTTON> = buttons
            .iter()
            .map(|b| {
                let mut wide: Vec<u16> = b.label.encode_utf16().collect();
                wide.push(0);
                wide.push(0);
                let at = unsafe {
                    SendMessageW(
                        self.hwnd,
                        TB_ADDSTRINGW,
                        WPARAM(0),
                        LPARAM(wide.as_ptr() as isize),
                    )
                };
                TBBUTTON {
                    iBitmap: I_IMAGENONE,
                    idCommand: b.id as i32,
                    fsState: state_bits(b),
                    fsStyle: (BTNS_SHOWTEXT | BTNS_AUTOSIZE | if b.toggle { BTNS_CHECK } else { 0 })
                        as u8,
                    bReserved: [0; 6],
                    dwData: 0,
                    // **`-1` means the pool refused the string, and it must not reach `iString`.**
                    // comctl32 reads an out-of-range index as a *pointer*, so storing the failure
                    // would have it dereference `0xFFFF_FFFF_FFFF_FFFF`. A button with no string
                    // is a blank button, which is survivable; the alternative is not.
                    iString: if at.0 < 0 { 0 } else { at.0 },
                }
            })
            .collect();
        unsafe {
            let added = SendMessageW(
                self.hwnd,
                TB_ADDBUTTONSW,
                WPARAM(items.len()),
                LPARAM(items.as_ptr() as isize),
            );
            SendMessageW(self.hwnd, TB_AUTOSIZE, WPARAM(0), LPARAM(0));
            // A failed insert must not be recorded as nine buttons: `same_row` would then be true
            // for ever after and the band would stay reserved over an empty strip. Reporting it
            // lets the caller forget, so the next frame tries again.
            added.0 != 0
        }
    }

    /// How tall the control made itself, asked of the control.
    ///
    /// **Never a number chosen here.** The tab strip's band was once `chrome_h + 4.0`, and that
    /// four was precisely the "a bit small" the owner reported; a replacement invented in this file
    /// would be the same mistake with a different constant.
    ///
    /// **The height comes from the window, not from `TB_GETMAXSIZE`.** That message reports the
    /// extent of the *visible buttons and separators* — which is not the same thing as the height
    /// the control sized itself to at `TB_AUTOSIZE`, and reserving the smaller of the two clips the
    /// bottom row of button chrome. `rebuild` autosizes; this reads back what that produced.
    /// `TB_GETMAXSIZE` remains the fallback for the frame before any button exists.
    pub fn band_height(&self) -> i32 {
        let mut rect = RECT::default();
        if unsafe { GetWindowRect(self.hwnd, &mut rect) }.is_ok() && rect.bottom > rect.top {
            return (rect.bottom - rect.top).clamp(1, 200);
        }
        let mut size = SIZE::default();
        let got = unsafe {
            SendMessageW(
                self.hwnd,
                TB_GETMAXSIZE,
                WPARAM(0),
                LPARAM(&mut size as *mut SIZE as isize),
            )
        };
        if got.0 == 0 {
            return 0;
        }
        size.cy.clamp(1, 200)
    }

    /// The control's window, so the shell can tell this control's `WM_COMMAND` from anyone else's.
    pub fn hwnd(&self) -> HWND {
        self.hwnd
    }

    /// Puts the control across the client area under the tab strip, or hides it.
    pub fn place(&mut self, top: i32, width: i32, height: i32, visible: bool) {
        if visible {
            unsafe {
                let _ = SetWindowPos(
                    self.hwnd,
                    HWND_TOP,
                    0,
                    top,
                    width.max(0),
                    height.max(0),
                    SWP_NOACTIVATE | SWP_NOZORDER,
                );
            }
        }
        if visible != self.visible {
            unsafe {
                let _ = ShowWindow(self.hwnd, if visible { SW_SHOWNA } else { SW_HIDE });
            }
            self.visible = visible;
        }
    }
}

impl Drop for Toolbar {
    /// **The window first, then the font**, which is the order `TabStrip` uses and the order that
    /// matters: the control has the `HFONT` selected for as long as it exists, and deleting a GDI
    /// object still selected into a live device context is undefined rather than merely untidy.
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.hwnd);
            if !self.font.is_invalid() {
                let _ = DeleteObject(HGDIOBJ(self.font.0));
            }
        }
    }
}

/// The `TBSTATE_*` bits for one button.
///
/// Pure, and separately, because a button that is pressed but disabled and one that is enabled but
/// not pressed differ by a single bit, and getting that wrong shows as a toolbar that looks almost
/// right.
pub fn state_bits(b: &ToolButton) -> u8 {
    let mut bits = 0;
    if b.enabled {
        bits |= TBSTATE_ENABLED;
    }
    if b.pressed {
        bits |= TBSTATE_CHECKED;
    }
    bits as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(buttons: &[ToolButton]) -> Vec<&str> {
        buttons.iter().map(|b| b.label).collect()
    }

    /// **§2.3's row, in §2.3's order.** Maximise is not in it: it is a window arrangement, not a
    /// document command, and the owner asked for the window controls to stay out of the toolbar. The order is the
    /// requirement, not a preference: the
    /// document names these nine and this sequence, and a toolbar is a thing people reach for by
    /// position after the first week.
    #[test]
    fn the_row_is_the_buttons_the_design_names_in_its_order() {
        assert_eq!(
            labels(&toolbar_of(None)),
            [
                "Open", "Find", "Filter", "Follow", "Collapse", "Detail", "Rules", "Format",
                "Export"
            ]
        );
    }

    /// **With nothing open the row is still there.** A toolbar that appeared with the first file
    /// would shift the grid down underneath the pointer at the least welcome moment, and §2.3 says
    /// an unavailable command is "disabled in place".
    #[test]
    fn an_empty_window_greys_the_row_rather_than_removing_it() {
        let empty = toolbar_of(None);
        assert_eq!(empty.len(), 9, "the row does not shrink");
        let live: Vec<&str> = empty
            .iter()
            .filter(|b| b.enabled)
            .map(|b| b.label)
            .collect();
        assert_eq!(
            live,
            ["Open", "Rules"],
            "only the two commands that do not need a document"
        );
        assert!(
            empty.iter().all(|b| !b.pressed),
            "nothing can be on when nothing is open"
        );
    }

    /// The toggles are marked as toggles and the verbs are not — the distinction the
    /// Win32 half turns into `BTNS_CHECK`. A verb given a pressed state would latch down on click
    /// and stay there.
    #[test]
    fn exactly_the_state_buttons_are_toggles() {
        let toggles: Vec<&str> = toolbar_of(None)
            .iter()
            .filter(|b| b.toggle)
            .map(|b| b.label)
            .collect();
        assert_eq!(toggles, ["Follow", "Collapse", "Detail"]);
    }

    /// **Every id is the menu's.** This is the test that keeps §1.2's one-command-one-path rule
    /// true: if a button ever grew an id of its own, a click would stop reaching the dispatch the
    /// menu and the keystroke use, and the two surfaces could drift apart in silence.
    #[test]
    fn every_button_carries_the_command_id_the_menu_sends() {
        let expected = [
            Command::OpenFile,
            Command::Find,
            Command::ToggleFilters,
            Command::FollowTail,
            Command::ToggleCollapse,
            Command::ToggleDetail,
            Command::EditRules,
            Command::DefineFormat,
            Command::Export,
        ];
        for (button, command) in toolbar_of(None).iter().zip(expected) {
            assert_eq!(
                button.id,
                command_id(command),
                "{} must post the menu's id",
                button.label
            );
        }
    }

    /// **Enabled and pressed are two independent bits**, and every combination has to survive the
    /// trip into `TBSTATE_*`. A toggle that is on but cannot act — Follow on a document that has
    /// just been closed — must still read as on, or the row lies about the state it exists to
    /// display.
    #[test]
    fn the_two_state_bits_are_independent() {
        let button = |enabled, pressed| ToolButton {
            label: "x",
            id: 1,
            enabled,
            toggle: true,
            pressed,
        };
        assert_eq!(state_bits(&button(false, false)), 0);
        assert_eq!(state_bits(&button(true, false)), TBSTATE_ENABLED as u8);
        assert_eq!(state_bits(&button(false, true)), TBSTATE_CHECKED as u8);
        assert_eq!(
            state_bits(&button(true, true)),
            (TBSTATE_ENABLED | TBSTATE_CHECKED) as u8
        );
    }

    /// `I_IMAGENONE` is hand-declared because the crate does not bind it, so it is checked rather
    /// than trusted — the WinHTTP lesson, applied to the one number this module invents. A button
    /// given image zero instead of none reserves space for a bitmap that is not there.
    #[test]
    fn the_one_hand_declared_constant_is_the_documented_value() {
        assert_eq!(I_IMAGENONE, -2);
        assert_ne!(
            I_IMAGENONE, 0,
            "image zero is a real image, not the absence"
        );
    }

    /// No two buttons share an id, because the shell dispatches on the id alone and a duplicate
    /// would make one of the pair unreachable while looking perfectly correct on screen.
    #[test]
    fn no_two_buttons_share_an_id() {
        let mut ids: Vec<u32> = toolbar_of(None).iter().map(|b| b.id).collect();
        let before = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), before);
    }
}
