//! The status bar, as the real Windows control — `UI-DESIGN.md` §1.1.
//!
//! **This replaces a drawn band, and the reason is not only that it looked wrong.** The old one
//! rendered its text through the painter's chrome atlas and was *losing glyphs*: `hardware` came
//! out `har ware`, `following` as `fo owing`, `tailhawk-spill` as `tai hawk-spi`, `probably` as
//! `probab y` — every missing character an `l` or a `d` — while the same letters drew correctly in
//! the grid a few pixels above. A surface with its own text stack is a surface with its own bugs,
//! and this one sat on screen unnoticed until the owner reported it. There is nothing here that
//! draws, so there is nothing here that can lose a letter.
//!
//! **§1.1's table is amended alongside this.** It rejected a "fixed-function status bar" in favour
//! of a row of live, clickable chips — which is the decision that produced the drawn band. A real
//! status bar takes **parts** and hit-tests them, so the chips remain available as parts of a
//! control rather than as drawing; they are simply not built yet, and the bar carries the one
//! composed sentence [`Shell::status_text`](crate::Shell) already produces.
//!
//! **The band height is the system's**, read back from the control. It follows the shell font and
//! the DPI, and the layout that has to make room for it is not the place to decide how tall
//! Windows draws its own bar.

use crate::tabstrip::shell_font;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{DeleteObject, HFONT, HGDIOBJ};
use windows::Win32::UI::Controls::{
    InitCommonControlsEx, ICC_BAR_CLASSES, INITCOMMONCONTROLSEX, SBARS_SIZEGRIP, SB_SETPARTS,
    SB_SETTEXTW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, GetWindowRect, SendMessageW, HMENU, WINDOW_EX_STYLE,
    WINDOW_STYLE, WM_SETFONT, WM_SIZE, WS_CHILD, WS_CLIPSIBLINGS, WS_VISIBLE,
};

/// The one part spans the whole bar. `-1` is the documented "to the right edge" width.
const TO_THE_EDGE: i32 = -1;

/// The window's status bar.
pub struct StatusBar {
    hwnd: HWND,
    font: HFONT,
    /// What the bar was last told to say. **Kept so it is not told again**: the shell composes its
    /// status every frame, and `SB_SETTEXTW` with unchanged text still invalidates and repaints
    /// the bar, which is a repaint per frame for a string that changes a few times a second.
    shown: String,
}

impl StatusBar {
    /// Creates the control as a child of `parent`. `None` when it could not be created, in which
    /// case the window simply has no status bar — the same way the toolbar and the tab strip fail.
    pub fn new(parent: HWND, instance: HINSTANCE) -> Option<StatusBar> {
        let init = INITCOMMONCONTROLSEX {
            dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_BAR_CLASSES,
        };
        unsafe {
            let _ = InitCommonControlsEx(&init);
        }
        let hwnd = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("msctls_statusbar32"),
                PCWSTR::null(),
                WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | WS_CLIPSIBLINGS.0 | SBARS_SIZEGRIP),
                0,
                0,
                0,
                0,
                parent,
                HMENU::default(),
                instance,
                None,
            )
        }
        .ok()?;
        let font = shell_font();
        if !font.is_invalid() {
            unsafe {
                SendMessageW(hwnd, WM_SETFONT, WPARAM(font.0 as usize), LPARAM(1));
            }
        }
        // The same theme decision the menus take — `controls::apply_theme`, one place for all.
        crate::controls::apply_theme(hwnd, tailhawk_core::theme::theme().dark);
        let bar = StatusBar {
            hwnd,
            font,
            shown: String::new(),
        };
        bar.one_part();
        Some(bar)
    }

    /// One part, spanning the bar. Re-sent after a resize because the part widths are in pixels.
    fn one_part(&self) {
        let edges = [TO_THE_EDGE];
        unsafe {
            SendMessageW(
                self.hwnd,
                SB_SETPARTS,
                WPARAM(1),
                LPARAM(edges.as_ptr() as isize),
            );
        }
    }

    /// The control's window, for the shell to re-theme when the theme changes.
    pub fn hwnd(&self) -> HWND {
        self.hwnd
    }

    /// How tall the bar is, in client pixels — the system's answer, not ours. The caller subtracts
    /// this from the client area so the grid and its scroll bar end above the bar instead of
    /// beside it.
    pub fn band_height(&self) -> i32 {
        let mut rc = RECT::default();
        if unsafe { GetWindowRect(self.hwnd, &mut rc) }.is_err() {
            return 0;
        }
        (rc.bottom - rc.top).max(0)
    }

    /// Docks the bar along the bottom of its parent's client area.
    ///
    /// **A status bar positions itself**, which is why this forwards `WM_SIZE` rather than calling
    /// `SetWindowPos`: the control reads the parent's client rectangle and takes the bottom strip,
    /// including the corner the size grip needs. Placing it by hand is how a status bar ends up a
    /// pixel out from the window it belongs to.
    pub fn resize(&self) {
        unsafe {
            SendMessageW(self.hwnd, WM_SIZE, WPARAM(0), LPARAM(0));
        }
        self.one_part();
    }

    /// Sets what the bar says, if it is not already saying it.
    pub fn set(&mut self, text: &str) {
        if self.shown == text {
            return;
        }
        text.clone_into(&mut self.shown);
        let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
        unsafe {
            SendMessageW(
                self.hwnd,
                SB_SETTEXTW,
                WPARAM(0),
                LPARAM(wide.as_ptr() as isize),
            );
        }
    }
}

impl Drop for StatusBar {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.hwnd);
            if !self.font.is_invalid() {
                let _ = DeleteObject(HGDIOBJ(self.font.0));
            }
        }
    }
}
