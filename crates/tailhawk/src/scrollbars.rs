//! The scroll bars, as real controls inside the client area — `UI-DESIGN.md` §2.1.
//!
//! **The frame used to own them, and that is what put two resize grips on screen.** The window was
//! created `WS_OVERLAPPEDWINDOW | WS_VSCROLL | WS_HSCROLL`, and Microsoft's own wording settles
//! what that means: *"A standard scroll bar is located in the nonclient area of a window"*, with
//! `WS_HSCROLL` placing it at the bottom of the client area — meaning immediately outside it.
//! `msctls_statusbar32` docks itself *inside* the client rectangle. So the horizontal bar sat
//! permanently **below** the status bar, as a second band with the frame's sizing corner at its
//! right-hand end, while the status bar drew its own `SBARS_SIZEGRIP` above it. No arithmetic in
//! the layout pass could have lifted it: non-client space is not the client area's to spend.
//!
//! The owner reported exactly that on 2026-10-05 — the grip "appears twice, once on the window
//! border, and once on the status bar" — and diagnosed it correctly too: the status bar should be
//! the bottom-most element. It can only be that if the scroll bars move inside the client area,
//! which is what this module is for. It is also what the comparison he offered does: Notepad++
//! hangs its scroll bars on its edit control, not on its frame.
//!
//! **A control is a window, and that has one documented cost, taken deliberately.** *"As a separate
//! window, a scroll bar control takes direct input focus"*, which a standard scroll bar does not —
//! so the frame takes focus back after a scroll, or the grid would lose the keyboard to a bar the
//! reader only meant to drag. The tab strip solves the same problem with `TCS_FOCUSNEVER`; there is
//! no `SBS_` equivalent, so it is done by hand.
//!
//! **What this module does not touch is what the bars report.** `sync_scrollbar` keeps its fixed
//! `0..SCROLL_RANGE` fraction and `sync_hscrollbar` its pixel extent; only the handle they are set
//! on changes, from `(frame, SB_VERT)` to `(control, SB_CTL)`. The defect was where the bars live,
//! not what they say.
//!
//! **The geometry is a pure function.** [`layout`] decides both rectangles and what is left for the
//! content, and it is tested with no window anywhere near it — which is this project's standing
//! rule after three defects in one session all turned out to be decisions tangled into the shell.

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HINSTANCE, HWND};
use windows::Win32::UI::HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, SetWindowPos, ShowWindow, HWND_TOP, SM_CXVSCROLL, SM_CYHSCROLL,
    SWP_NOACTIVATE, SW_HIDE, SW_SHOWNOACTIVATE, WINDOW_EX_STYLE, WINDOW_STYLE, WS_CHILD,
    WS_CLIPSIBLINGS,
};

/// `SBS_HORZ` — a horizontal scroll bar at the rectangle given, with no align style, so the
/// caller's rectangle is used whole. *"you must always specify the x- and y-coordinates and the
/// other dimensions of the scroll bar"*, which is why neither bar relies on a default.
const SBS_HORZ: u32 = 0x0000;

/// `SBS_VERT` — the same, vertically.
const SBS_VERT: u32 = 0x0001;

/// A rectangle in client pixels: left, top, width, height.
///
/// Deliberately not Win32's `RECT`, whose `right`/`bottom` are edges rather than extents — the
/// layout arithmetic here is in widths and heights, and converting once at the edge is better than
/// mixing the two conventions through a file of geometry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Box2 {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

/// Where the two bars go, and what is left for the panes.
///
/// **A view-model**, in this codebase's sense: the bars "as one frame should place them", carrying
/// no `HWND`. [`layout`] is the mapping that produces it and [`Bars::place`] is the only thing that
/// acts on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Frame {
    /// The vertical bar, always present — `sync_scrollbar` passes `SIF_DISABLENOSCROLL`, so a file
    /// that fits the window gets a full-height thumb rather than a bar that vanishes and takes the
    /// content's width with it as the reader scrolls past the end.
    pub vertical: Box2,
    /// The horizontal bar, present only when a line is wider than the window. Absent rather than
    /// disabled, matching what the frame's own bar did: `sync_hscrollbar` deliberately omits
    /// `SIF_DISABLENOSCROLL` so that a log which fits has **no** horizontal bar rather than a dead
    /// full-width one.
    pub horizontal: Option<Box2>,
    /// What the panes get: the client area less the bars, the bands above and the status bar.
    pub content: (i32, i32),
}

/// The thickness of each bar, from the system rather than from a number chosen here.
///
/// `SM_CXVSCROLL` is the width of a vertical bar and `SM_CYHSCROLL` the height of a horizontal one.
/// They follow the user's own metrics, which is the whole reason to ask: a hard-coded 17 pixels is
/// right on exactly one machine.
///
/// **Asked for this window's DPI, not the primary monitor's.** This process is Per-Monitor-V2, so
/// plain `GetSystemMetrics` answers for the primary display and would under-measure both bars on a
/// window dragged to a monitor at a different scale — leaving the content a few pixels wide of the
/// bar it is supposed to stop beside. `GetSystemMetricsForDpi` is the per-monitor form, and
/// `GetDpiForWindow` is where the window actually is.
pub fn thickness(hwnd: HWND) -> (i32, i32) {
    // SAFETY: both read system constants for a live window handle and cannot fail in a way that
    // matters; a zero DPI answer falls back to the per-monitor default of 96.
    unsafe {
        let dpi = match GetDpiForWindow(hwnd) {
            0 => 96,
            d => d,
        };
        (
            GetSystemMetricsForDpi(SM_CXVSCROLL, dpi).max(1),
            GetSystemMetricsForDpi(SM_CYHSCROLL, dpi).max(1),
        )
    }
}

/// Places the bars inside the client area, between the bands above and the status bar below.
///
/// `client` is the whole client area. `top_px` is what the tab strip and toolbar have already taken
/// from the top, and `status_px` what the status bar takes from the bottom — both asked of their
/// controls by the caller, as they already are. `want_h` is whether a line is wider than the
/// window. `thick` is [`thickness`], passed in so this stays testable with no system to ask.
///
/// **The corner stays empty, and that is the point of the whole change.** The vertical bar stops
/// above the horizontal one and the horizontal stops short of the vertical, so the square where
/// they would meet is left alone — and the status bar, which the caller has already subtracted,
/// is below both with the only resize grip on screen.
///
/// Everything is clamped at zero: a window dragged smaller than its own chrome is a real thing that
/// happens during a resize, and a negative width reaches `SetWindowPos` as an enormous one.
pub fn layout(
    client: (i32, i32),
    top_px: i32,
    status_px: i32,
    want_h: bool,
    thick: (i32, i32),
) -> Frame {
    let (cw, ch) = (client.0.max(0), client.1.max(0));
    let (v_w, h_h) = (thick.0.max(1), thick.1.max(1));
    let top = top_px.clamp(0, ch);
    let bottom = status_px.clamp(0, ch - top);

    let band_h = if want_h { h_h } else { 0 };
    let content_w = (cw - v_w).max(0);
    let content_h = (ch - top - bottom - band_h).max(0);

    let vertical = Box2 {
        x: content_w,
        y: top,
        w: v_w.min(cw),
        h: content_h,
    };
    let horizontal = want_h.then(|| Box2 {
        x: 0,
        y: top + content_h,
        w: content_w,
        h: band_h.min((ch - top - bottom).max(0)),
    });

    Frame {
        vertical,
        horizontal,
        content: (content_w, content_h),
    }
}

/// The two controls.
pub struct Bars {
    vertical: HWND,
    horizontal: HWND,
    /// What was last applied, so a frame that moved nothing sends no `SetWindowPos`. This runs every
    /// paint, and the status bar's own guard exists for exactly this reason.
    applied: Option<Frame>,
}

impl Bars {
    /// Creates both bars as children of `parent`, hidden and at zero size until the first
    /// [`Bars::place`].
    ///
    /// Each is given an explicit rectangle, per the documented requirement, even though it is
    /// immediately replaced — a control created with no dimensions and no align style has no
    /// defined size to be shown at.
    pub fn create(parent: HWND, instance: HINSTANCE) -> Option<Bars> {
        let one = |style: u32| -> Option<HWND> {
            // SAFETY: `SCROLLBAR` is a system class, `parent` is a live window, and the rectangle
            // is in range. A failure returns an error, which becomes `None`.
            unsafe {
                CreateWindowExW(
                    WINDOW_EX_STYLE::default(),
                    w!("SCROLLBAR"),
                    PCWSTR::null(),
                    WINDOW_STYLE(WS_CHILD.0 | WS_CLIPSIBLINGS.0 | style),
                    0,
                    0,
                    1,
                    1,
                    parent,
                    None,
                    instance,
                    None,
                )
                .ok()
            }
        };
        Some(Bars {
            vertical: one(SBS_VERT)?,
            horizontal: one(SBS_HORZ)?,
            applied: None,
        })
    }

    /// The vertical bar's handle, for `SetScrollInfo` with `SB_CTL`.
    pub fn vertical(&self) -> HWND {
        self.vertical
    }

    /// The horizontal bar's handle, for `SetScrollInfo` with `SB_CTL`.
    pub fn horizontal(&self) -> HWND {
        self.horizontal
    }

    /// Whether `hwnd` is one of these two bars.
    ///
    /// `WM_HSCROLL` and `WM_VSCROLL` carry the control's handle in `lParam` for a scroll bar
    /// control and nothing for a frame bar, so this is how the frame tells a message from its own
    /// bars apart from one sent by anything else.
    pub fn owns(&self, hwnd: HWND) -> bool {
        hwnd == self.vertical || hwnd == self.horizontal
    }

    /// Applies a [`Frame`], moving and showing or hiding each bar.
    ///
    /// **Visibility is applied every time, and the geometry only when it changed.** The two are
    /// separated deliberately: `SetWindowPos` every frame would reorder siblings on every paint,
    /// while `ShowWindow` is what keeps this the *single authority* on whether the horizontal bar is
    /// on screen. `sync_hscrollbar` passes `SIF_DISABLENOSCROLL` for the same reason — so Windows
    /// never hides or shows that bar behind this method's back. Two deciders was a real defect: a
    /// cached frame would skip the correction and leave either a dead band or a stray bar over the
    /// bottom rows.
    pub fn place(&mut self, frame: Frame) {
        let shown = frame.horizontal.filter(|b| b.w > 0 && b.h > 0);
        if self.applied != Some(frame) {
            self.applied = Some(frame);
            let v = (frame.vertical.w > 0 && frame.vertical.h > 0).then_some(frame.vertical);
            self.put(self.vertical, v);
            self.put(self.horizontal, shown);
        }
        self.show(self.vertical, frame.vertical.w > 0 && frame.vertical.h > 0);
        self.show(self.horizontal, shown.is_some());
    }

    /// **`HWND_TOP` without `SWP_NOZORDER`, which is the point.** The bars are created with the
    /// window and the header and filter panel are created lazily, so those are *later* siblings and
    /// therefore above these — and a header laid out wider than its pane would be drawn over the
    /// top of the vertical bar, hiding its up-arrow. A scroll bar belongs above the content it
    /// scrolls. The z-order is only reasserted when the geometry changed, so this is not a reorder
    /// on every paint.
    fn put(&self, hwnd: HWND, at: Option<Box2>) {
        let Some(b) = at else {
            return;
        };
        // SAFETY: a live child handle, and a rectangle `layout` has already clamped.
        unsafe {
            let _ = SetWindowPos(hwnd, HWND_TOP, b.x, b.y, b.w, b.h, SWP_NOACTIVATE);
        }
    }

    fn show(&self, hwnd: HWND, visible: bool) {
        // SAFETY: a live child handle; `ShowWindow` retains nothing.
        unsafe {
            let _ = ShowWindow(hwnd, if visible { SW_SHOWNOACTIVATE } else { SW_HIDE });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const THICK: (i32, i32) = (17, 17);

    /// **The corner between the bars is empty, and the status bar's strip is untouched.**
    ///
    /// This is the defect the module exists for, stated as arithmetic: whatever the caller reserved
    /// for the status bar is below everything placed here, and neither bar reaches into it. If this
    /// assertion ever fails, the two grips are back.
    #[test]
    fn neither_bar_reaches_the_status_bars_strip() {
        let f = layout((1000, 600), 50, 22, true, THICK);
        let status_top = 600 - 22;
        assert!(
            f.vertical.y + f.vertical.h <= status_top,
            "the vertical bar ran into the status bar: {:?}",
            f.vertical
        );
        let h = f.horizontal.expect("wanted");
        assert!(
            h.y + h.h <= status_top,
            "the horizontal bar ran into the status bar: {h:?}"
        );
        assert_eq!(h.y + h.h, status_top, "and it sits directly above it");
        assert_eq!(
            h.x + h.w,
            f.vertical.x,
            "the horizontal bar stops where the vertical one starts, leaving the corner empty"
        );
    }

    /// **The content is the client area less the bars and the bands**, which is what the panes are
    /// laid out in. An off-by-one here hides a row behind a bar or leaves a strip of nothing.
    #[test]
    fn the_content_is_what_is_left_over() {
        let f = layout((1000, 600), 50, 22, true, THICK);
        assert_eq!(f.content, (1000 - 17, 600 - 50 - 22 - 17));
        assert_eq!(
            f.vertical.x, f.content.0,
            "the bar starts where content ends"
        );
        assert_eq!(f.vertical.h, f.content.1, "and is exactly as tall");
    }

    /// **No horizontal bar means its height goes back to the content**, not that a gap is left.
    ///
    /// `sync_hscrollbar` omits `SIF_DISABLENOSCROLL` precisely so a log that fits has no horizontal
    /// bar, so the common case is the absent one and it must not cost a band of nothing.
    #[test]
    fn a_log_that_fits_gives_its_band_back() {
        let with = layout((1000, 600), 50, 22, true, THICK);
        let without = layout((1000, 600), 50, 22, false, THICK);
        assert!(without.horizontal.is_none());
        assert_eq!(without.content.1, with.content.1 + 17);
        assert_eq!(without.content.0, with.content.0, "width is unchanged");
        assert_eq!(without.vertical.y + without.vertical.h, 600 - 22);
    }

    /// **A window smaller than its own chrome yields zeroes, never negatives.**
    ///
    /// A resize drag really does pass through sizes this small, and a negative width handed to
    /// `SetWindowPos` is an enormous unsigned one — a bar drawn across the whole screen.
    #[test]
    fn a_window_smaller_than_its_chrome_clamps_at_zero() {
        for size in [(0, 0), (4, 4), (20, 30), (1, 600)] {
            let f = layout(size, 50, 22, true, THICK);
            assert!(f.content.0 >= 0 && f.content.1 >= 0, "{size:?} -> {f:?}");
            assert!(f.vertical.w >= 0 && f.vertical.h >= 0, "{size:?} -> {f:?}");
            assert!(f.vertical.w <= size.0.max(0), "{size:?} -> {f:?}");
            if let Some(h) = f.horizontal {
                assert!(h.w >= 0 && h.h >= 0, "{size:?} -> {h:?}");
                assert!(h.y + h.h <= size.1.max(0), "{size:?} -> {h:?}");
            }
        }
    }

    /// **The thickness is the system's and is used for both axes independently.** They are two
    /// metrics, `SM_CXVSCROLL` and `SM_CYHSCROLL`, and a machine where they differ must not get a
    /// square guess.
    #[test]
    fn each_axis_takes_its_own_metric() {
        let f = layout((1000, 600), 0, 0, true, (13, 29));
        assert_eq!(f.vertical.w, 13);
        assert_eq!(f.horizontal.expect("wanted").h, 29);
        assert_eq!(f.content, (1000 - 13, 600 - 29));
    }

    /// **The bands above are honoured**, so the vertical bar starts below the toolbar rather than
    /// beside it — which is one thing the frame's own bar could never do, since non-client space
    /// runs the full height of the window.
    #[test]
    fn the_vertical_bar_starts_below_the_bands() {
        let f = layout((1000, 600), 74, 22, false, THICK);
        assert_eq!(f.vertical.y, 74);
        assert_eq!(f.vertical.h, 600 - 74 - 22);
    }
}
