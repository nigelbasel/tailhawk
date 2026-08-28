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
