//! The tab strip, as Windows' own control rather than as something we paint.
//!
//! The strip used to be drawn: a filled rectangle and a text run per tab, with the band's height
//! computed as `chrome_h + 4.0` — four pixels around a line of chrome text. That is the whole of
//! the owner's report on 2026-08-28 that the tabs were "a bit small, and dont look like normal
//! windows tabs". They were small because a text run plus four pixels *is* small, and they did not
//! look like Windows tabs because they were not.
//!
//! **The height is asked of the control, never chosen.** `TCM_ADJUSTRECT` answers what a tab
//! control needs for its own band at the current font and DPI, which is the only number that is
//! right on every machine — and any constant picked here by hand would be the previous mistake
//! wearing a different value.
//!
//! # The risk this module is also an experiment in
//!
//! This is the **first child window the main window has ever had**, and the swapchain is
//! `DXGI_SWAP_EFFECT_FLIP_DISCARD`. Microsoft's flip-model guidance is explicit that child windows
//! over such a swapchain are not composited the way they would be over a bitblt one;
//! `WS_CLIPCHILDREN` on the parent is the documented remedy and is applied there. If it proves
//! insufficient the fallback is to move the swapchain onto its own child window and make the chrome
//! its siblings — the shape the toolbar and MDI both need anyway, which is why the tab control is
//! the cheap place to find out. **No test can settle it; it needs an eye on screen.**

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{CreateFontIndirectW, DeleteObject, HFONT};
use windows::Win32::UI::Controls::SetWindowTheme;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, SendMessageW, SetWindowPos, ShowWindow, SystemParametersInfoW,
    HWND_TOP, NONCLIENTMETRICSW, SPI_GETNONCLIENTMETRICS, SWP_NOACTIVATE, SWP_NOZORDER, SW_HIDE,
    SW_SHOWNA, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, WINDOW_EX_STYLE, WM_SETFONT, WS_CHILD,
    WS_CLIPSIBLINGS,
};

const TCM_FIRST: u32 = 0x1300;
const TCM_DELETEALLITEMS: u32 = TCM_FIRST + 9;
const TCM_GETCURSEL: u32 = TCM_FIRST + 11;
const TCM_SETCURSEL: u32 = TCM_FIRST + 12;
const TCM_ADJUSTRECT: u32 = TCM_FIRST + 40;
const TCM_INSERTITEMW: u32 = TCM_FIRST + 62;
const TCIF_TEXT: u32 = 0x0001;

/// `TCN_SELCHANGE` — the notification that the shown tab changed. `TCN_FIRST` is `-550`.
pub const TCN_SELCHANGE: u32 = (-551_i32) as u32;

/// **The tab control never takes the focus.** Clicking a tab must not move the caret off the grid:
/// the grid is where every key goes, and a strip that stole focus would make a mouse click quietly
/// change what the keyboard does.
const TCS_FOCUSNEVER: u32 = 0x0000_8000;

/// The id the control answers to in `WM_NOTIFY`, so the shell can tell it from any other child.
pub const ID_TABS: i32 = 4_100;

const TCM_GETITEMRECT: u32 = TCM_FIRST + 10;
const TCM_HITTEST: u32 = TCM_FIRST + 13;
const TCM_SETPADDING: u32 = TCM_FIRST + 43;

/// Breathing room around a tab's label, in pixels at 96 DPI, scaled with the strip's font.
///
/// The control's own default is 6 across and 3 down, which the owner read on 2026-08-28 as "a bit
/// cramped … might need some white space on either side of the label … could also do with being a
/// bit higher". These are the numbers that answer both, and they are the *only* place a size is
/// chosen: [`TabStrip::band_height`] still asks the control how tall it has become, so the band and
/// the tabs cannot disagree the way they would if a height were set here too.
const PAD_X: i32 = 14;
const PAD_Y: i32 = 7;

/// Posted to the parent when a tab is dragged onto another's place: `wparam` is where it came
/// from, `lparam` where it now belongs. The shell owns the document order, so the control asks
/// rather than reorders — reordering the control alone would put the strip and the documents into
/// different orders, which is the same class of lie as a stale menu tick.
pub const WM_TAB_MOVED: u32 = windows::Win32::UI::WindowsAndMessaging::WM_APP + 0x40;

/// Posted to the parent when a tab is middle-clicked: `wparam` is the tab to close.
///
/// **`UI-DESIGN.md` §2.1's middle-click stopped working the moment the strip became a control**,
/// and it did so silently. The handler lives on the *main window*, and a middle click over the
/// strip is now delivered to the child; it also asked `Shell::tab_at`, which reads a hit list the
/// drawn strip used to fill and nothing fills any more. Two independent reasons for the same
/// nothing, which is why it was mistaken for dead code rather than a broken feature.
pub const WM_TAB_CLOSE: u32 = windows::Win32::UI::WindowsAndMessaging::WM_APP + 0x41;

/// Posted while a tab is being dragged **below** the strip, onto the grid: `wparam` is the tab,
/// `lparam` the pointer in the window's client coordinates.
///
/// `SPEC.md` §1069's drag-out-to-split. The control keeps the mouse captured for the whole drag, so
/// it goes on receiving moves after the pointer has left it — which is what makes leaving
/// detectable at all, and why this lives here rather than in the window's own handler.
///
/// The strip sits at the window's origin, so the control's client coordinates *are* the window's.
/// Nothing converts, and nothing has to know where the strip is.
pub const WM_TAB_DRAG_OUT: u32 = windows::Win32::UI::WindowsAndMessaging::WM_APP + 0x42;

/// Posted when a drag that had left the strip is released. Same arguments as [`WM_TAB_DRAG_OUT`].
pub const WM_TAB_DROP_OUT: u32 = windows::Win32::UI::WindowsAndMessaging::WM_APP + 0x43;

/// Posted when a drag ends or leaves without dropping, so the guide stops being drawn.
pub const WM_TAB_DRAG_OFF: u32 = windows::Win32::UI::WindowsAndMessaging::WM_APP + 0x44;

/// `TCHITTESTINFO`, laid out by hand beside [`TcItem`] for the same reason.
#[repr(C)]
struct TcHitTest {
    pt: windows::Win32::Foundation::POINT,
    flags: u32,
}

thread_local! {
    /// Which tab the pointer went down on, while a drag is in progress.
    static DRAGGING: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
    /// Whether that drag has left the strip. Remembered because the button-up that ends it carries
    /// no history, and a release below the strip means something quite different from one inside it.
    static DRAGGED_OUT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// How far below the strip the pointer must go before a drag counts as having left it.
///
/// Not zero: a reorder drag along the strip wanders a pixel or two past the bottom edge, and
/// treating that as "you want to split" would turn every reorder into a near-miss.
const OUT_SLACK: i32 = 6;

/// The strip's own height, for deciding whether the pointer is still over it.
fn strip_height(hwnd: HWND) -> i32 {
    let mut rect = RECT::default();
    if unsafe { windows::Win32::UI::WindowsAndMessaging::GetClientRect(hwnd, &mut rect) }.is_err() {
        return i32::MAX;
    }
    rect.bottom - rect.top
}

/// Posts one of this module's messages up to the window that owns the strip.
///
/// The control never acts on its own: the shell owns which documents exist and which is shown, so
/// the strip reports the gesture and lets the shell decide. A control that closed its own tab would
/// leave the strip and the documents disagreeing.
fn tell_parent(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) {
    if let Ok(parent) = unsafe { windows::Win32::UI::WindowsAndMessaging::GetParent(hwnd) } {
        unsafe {
            let _ =
                windows::Win32::UI::WindowsAndMessaging::PostMessageW(parent, msg, wparam, lparam);
        }
    }
}

/// Which tab is under a point in the control's own client coordinates.
fn tab_at(hwnd: HWND, x: i16, y: i16) -> Option<usize> {
    let mut hit = TcHitTest {
        pt: windows::Win32::Foundation::POINT {
            x: i32::from(x),
            y: i32::from(y),
        },
        flags: 0,
    };
    let at = unsafe {
        SendMessageW(
            hwnd,
            TCM_HITTEST,
            WPARAM(0),
            LPARAM(&mut hit as *mut TcHitTest as isize),
        )
    };
    usize::try_from(at.0).ok()
}

/// Drag-to-reorder, which `SPEC.md` §1069 asks for and a tab control does not provide.
///
/// **Live reordering, as a browser does it**, rather than a drop at the end: the tab follows the
/// pointer, so the order you can see is the order you will get. The control is not touched here —
/// the shell is told, and the next frame rebuilds the strip from the new document order, so the
/// strip cannot end up in a different order from the documents it names.
unsafe extern "system" fn drag_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    _data: usize,
) -> windows::Win32::Foundation::LRESULT {
    use windows::Win32::UI::WindowsAndMessaging::{WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE};
    let x = (lparam.0 & 0xFFFF) as i16;
    let y = ((lparam.0 >> 16) & 0xFFFF) as i16;
    match msg {
        WM_LBUTTONDOWN => {
            DRAGGING.with(|d| d.set(tab_at(hwnd, x, y)));
            DRAGGED_OUT.with(|d| d.set(false));
            // **Capture, or a drag can never leave the strip.** This module assumed the control
            // took the mouse itself and it does not: reorder worked because those moves are inside
            // the control anyway, while §1069's drag-*out* saw nothing at all, because a move over
            // the grid is delivered to whatever is under the pointer. The owner reported exactly
            // that on 2026-08-28 — reorder fine, drag-out dead.
            unsafe {
                windows::Win32::UI::Input::KeyboardAndMouse::SetCapture(hwnd);
            }
        }
        WM_MOUSEMOVE if wparam.0 & 0x0001 != 0 => {
            let Some(from) = DRAGGING.with(|d| d.get()) else {
                return unsafe {
                    windows::Win32::UI::Shell::DefSubclassProc(hwnd, msg, wparam, lparam)
                };
            };
            // Below the strip is §1069's drag-out. Above or beside it is a reorder that has
            // wandered, and is left to the reorder path.
            if i32::from(y) > strip_height(hwnd) + OUT_SLACK {
                DRAGGED_OUT.with(|d| d.set(true));
                tell_parent(hwnd, WM_TAB_DRAG_OUT, WPARAM(from), lparam);
            } else {
                if DRAGGED_OUT.with(|d| d.replace(false)) {
                    // Came back onto the strip: the guide must go, or it hangs about promising a
                    // split that the drop will not perform.
                    tell_parent(hwnd, WM_TAB_DRAG_OFF, WPARAM(from), LPARAM(0));
                }
                if let Some(to) = tab_at(hwnd, x, y) {
                    if from != to {
                        DRAGGING.with(|d| d.set(Some(to)));
                        tell_parent(hwnd, WM_TAB_MOVED, WPARAM(from), LPARAM(to as isize));
                    }
                }
            }
        }
        WM_LBUTTONUP => {
            unsafe {
                let _ = windows::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture();
            }
            let from = DRAGGING.with(|d| d.replace(None));
            let out = DRAGGED_OUT.with(|d| d.replace(false));
            if let Some(from) = from {
                let what = if out {
                    WM_TAB_DROP_OUT
                } else {
                    WM_TAB_DRAG_OFF
                };
                tell_parent(hwnd, what, WPARAM(from), lparam);
            }
        }
        // §2.1's middle-click close. It has to be handled here: the click lands on this control,
        // not on the window whose handler used to answer it.
        windows::Win32::UI::WindowsAndMessaging::WM_MBUTTONDOWN => {
            if let Some(at) = tab_at(hwnd, x, y) {
                tell_parent(hwnd, WM_TAB_CLOSE, WPARAM(at), LPARAM(0));
            }
        }
        _ => {}
    }
    unsafe { windows::Win32::UI::Shell::DefSubclassProc(hwnd, msg, wparam, lparam) }
}

/// Windows' tab control, holding one item per open document.
pub struct TabStrip {
    hwnd: HWND,
    font: HFONT,
    /// What the control was last filled with, so a frame that changes nothing does not rebuild it —
    /// a rebuild resets the selection and flickers.
    shown: (Vec<String>, usize),
    visible: bool,
}

impl TabStrip {
    /// Creates the control as a child of the main window, hidden until there is more than one tab.
    pub fn create(
        parent: HWND,
        instance: windows::Win32::Foundation::HINSTANCE,
    ) -> Option<TabStrip> {
        let hwnd = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("SysTabControl32"),
                PCWSTR::null(),
                windows::Win32::UI::WindowsAndMessaging::WINDOW_STYLE(
                    WS_CHILD.0 | WS_CLIPSIBLINGS.0 | TCS_FOCUSNEVER,
                ),
                0,
                0,
                0,
                0,
                parent,
                windows::Win32::UI::WindowsAndMessaging::HMENU(ID_TABS as *mut core::ffi::c_void),
                instance,
                None,
            )
        }
        .ok()?;

        // Without this the control uses the ancient system bitmap font, which is both ugly and the
        // wrong size — the shell font is what every other Windows tab strip is drawn in.
        let font = shell_font();
        if !font.is_invalid() {
            unsafe {
                SendMessageW(hwnd, WM_SETFONT, WPARAM(font.0 as usize), LPARAM(1));
            }
        }
        // Best effort, and documented as such: the tab control honours far less of the dark theme
        // than a list view does. A light strip under a dark theme is a known limit of the control,
        // not something this module can fix without drawing the tabs again.
        unsafe {
            let _ = SetWindowTheme(hwnd, w!("DarkMode_Explorer"), PCWSTR::null());
        }
        // Padding, scaled for this monitor: the label gets room either side and the tab gets
        // taller. Sent after the font, because the control lays a tab out from both together.
        let dpi = unsafe { windows::Win32::UI::HiDpi::GetDpiForWindow(hwnd) }.max(96);
        let scale = |n: i32| n * dpi as i32 / 96;
        unsafe {
            SendMessageW(
                hwnd,
                TCM_SETPADDING,
                WPARAM(0),
                LPARAM(((scale(PAD_Y) << 16) | (scale(PAD_X) & 0xFFFF)) as isize),
            );
        }
        // §1069's drag-to-reorder, which the control has no notion of.
        unsafe {
            let _ = windows::Win32::UI::Shell::SetWindowSubclass(
                hwnd,
                Some(drag_proc),
                ID_TABS as usize,
                0,
            );
        }
        Some(TabStrip {
            hwnd,
            font,
            shown: (Vec::new(), usize::MAX),
            visible: false,
        })
    }

    /// Fills the control from the shell's labels, and marks which is current.
    ///
    /// Rebuilds only when something actually changed: `TCM_DELETEALLITEMS` followed by inserts
    /// resets the selection and repaints the whole band, so doing it every frame would flicker and
    /// fight the user's own clicks.
    pub fn set(&mut self, labels: &[String], active: usize) {
        if self.shown.0 == labels && self.shown.1 == active {
            return;
        }
        if self.shown.0 != labels {
            unsafe {
                SendMessageW(self.hwnd, TCM_DELETEALLITEMS, WPARAM(0), LPARAM(0));
            }
            for (i, label) in labels.iter().enumerate() {
                let mut text: Vec<u16> = label.encode_utf16().chain(std::iter::once(0)).collect();
                let item = TcItem {
                    mask: TCIF_TEXT,
                    state: 0,
                    state_mask: 0,
                    text: text.as_mut_ptr(),
                    text_max: 0,
                    image: -1,
                    param: 0,
                };
                unsafe {
                    SendMessageW(
                        self.hwnd,
                        TCM_INSERTITEMW,
                        WPARAM(i),
                        LPARAM(&item as *const TcItem as isize),
                    );
                }
            }
        }
        unsafe {
            SendMessageW(self.hwnd, TCM_SETCURSEL, WPARAM(active), LPARAM(0));
        }
        self.shown = (labels.to_vec(), active);
    }

    /// One tab's rectangle in the strip's own client coordinates, which are also the window's here
    /// because the strip sits at the origin.
    ///
    /// **The accessibility tree needs a truthful rectangle**, and the drawn strip's hit list — which
    /// is where it used to come from — is no longer filled by anything. An element that claims to be
    /// a tab and cannot say where it is, is worse than one that is absent.
    pub fn item_rect(&self, at: usize) -> Option<(f32, f32, f32, f32)> {
        let mut rect = RECT::default();
        let ok = unsafe {
            SendMessageW(
                self.hwnd,
                TCM_GETITEMRECT,
                WPARAM(at),
                LPARAM(&mut rect as *mut RECT as isize),
            )
        };
        if ok.0 == 0 {
            return None;
        }
        Some((
            rect.left as f32,
            rect.top as f32,
            (rect.right - rect.left) as f32,
            (rect.bottom - rect.top) as f32,
        ))
    }

    /// Which tab the control says is current — read back after `TCN_SELCHANGE` rather than guessed.
    pub fn selected(&self) -> Option<usize> {
        let at = unsafe { SendMessageW(self.hwnd, TCM_GETCURSEL, WPARAM(0), LPARAM(0)) };
        usize::try_from(at.0).ok()
    }

    /// The height of the tab band, **asked of the control**.
    ///
    /// `TCM_ADJUSTRECT` maps a control rectangle to the display area inside it; the difference at
    /// the top is exactly the band the tabs occupy at this font and DPI. Asking is the point — the
    /// drawn strip's `chrome_h + 4.0` is the smallness the owner reported.
    pub fn band_height(&self, width: i32) -> i32 {
        let mut rect = RECT {
            left: 0,
            top: 0,
            right: width.max(1),
            bottom: 200,
        };
        unsafe {
            SendMessageW(
                self.hwnd,
                TCM_ADJUSTRECT,
                WPARAM(0),
                LPARAM(&mut rect as *mut RECT as isize),
            );
        }
        rect.top.clamp(1, 200)
    }

    /// Puts the control across the top of the client area, or hides it when there is one tab.
    pub fn place(&mut self, width: i32, height: i32, visible: bool) {
        if visible {
            unsafe {
                let _ = SetWindowPos(
                    self.hwnd,
                    HWND_TOP,
                    0,
                    0,
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

impl Drop for TabStrip {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.hwnd);
            if !self.font.is_invalid() {
                let _ = DeleteObject(windows::Win32::Graphics::Gdi::HGDIOBJ(self.font.0));
            }
        }
    }
}

/// `TCITEMW`, laid out by hand so the module does not depend on the binding being present.
#[repr(C)]
struct TcItem {
    mask: u32,
    state: u32,
    state_mask: u32,
    text: *mut u16,
    text_max: i32,
    image: i32,
    param: isize,
}

/// The font Windows draws its own chrome in, so the strip matches every other tabbed application.
fn shell_font() -> HFONT {
    let mut metrics = NONCLIENTMETRICSW {
        cbSize: std::mem::size_of::<NONCLIENTMETRICSW>() as u32,
        ..Default::default()
    };
    let ok = unsafe {
        SystemParametersInfoW(
            SPI_GETNONCLIENTMETRICS,
            metrics.cbSize,
            Some(&mut metrics as *mut NONCLIENTMETRICSW as *mut core::ffi::c_void),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    };
    if ok.is_err() {
        return HFONT::default();
    }
    unsafe { CreateFontIndirectW(&metrics.lfMessageFont) }
}
