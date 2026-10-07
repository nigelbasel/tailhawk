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
use windows::Win32::Foundation::COLORREF;
use windows::Win32::Foundation::HINSTANCE;
use windows::Win32::Foundation::{HWND, LPARAM, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CreateFontIndirectW, CreateSolidBrush, DeleteObject, HBRUSH, HFONT, HGDIOBJ,
};
use windows::Win32::UI::HiDpi::SystemParametersInfoForDpi;
use windows::Win32::UI::WindowsAndMessaging::HMENU;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, SendMessageW, SetWindowPos, ShowWindow, SystemParametersInfoW,
    HWND_TOP, NONCLIENTMETRICSW, SPI_GETNONCLIENTMETRICS, SWP_NOACTIVATE, SWP_NOZORDER, SW_HIDE,
    SW_SHOWNA, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, WINDOW_EX_STYLE, WINDOW_STYLE, WM_SETFONT,
    WS_CHILD, WS_CLIPSIBLINGS,
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

/// The ids the per-tab close buttons answer to: `ID_TAB_CLOSE_BASE + n` closes the n-th tab.
///
/// Clear of [`ID_TABS`] and of the menu's ranges, which start at 10_100 — these are children of
/// the tab control rather than of the window, so their `WM_COMMAND` reaches this module's subclass
/// and never the shell's menu dispatch, but keeping them distinct costs nothing and a shared id is
/// the sort of thing that only shows up as a wrong document closing.
const ID_TAB_CLOSE_BASE: u32 = 4_200;

/// How many close buttons an id may name, so a stray `WM_COMMAND` cannot be read as a tab index.
/// A window with more tabs than this has other problems; `RECENT_MAX` is ten and a reader with
/// sixty-four logs open is not closing them one button at a time.
const MAX_CLOSERS: u32 = 64;

/// The close button's side and the gap between it and the tab's right edge, at 96 DPI.
///
/// Small enough that it does not crowd the name, large enough to be a target: the Windows
/// guideline is a minimum of about 16 pixels for a pointer, and this is scaled by the strip's DPI
/// like everything else here.
const CLOSE_PX: i32 = 16;
const CLOSE_INSET: i32 = 3;

/// How thick a traffic bar is, at 96 DPI — a hairline that reads as a bar rather than a border.
const BAR_PX: i32 = 3;

/// `BS_FLAT` and `BS_PUSHBUTTON` — a button with no raised edge, which is what a close affordance
/// inside a tab wants. The `windows` crate binds the button styles as plain `u32` constants rather
/// than a typed set, so they are spelled here.
const BS_PUSHBUTTON: u32 = 0x0000_0000;
const BS_FLAT: u32 = 0x0000_8000;

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

/// Posted while a tab is being dragged **below** the strip, onto the grid: `wparam` is the pair
/// [`drag_pair`] packed — read it with [`drag_indices`], never as a bare index — and `lparam` is
/// the pointer in the window's client coordinates.
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
///
/// **`wparam` carries the same packed pair as its siblings**, though nothing reads it: the handler
/// only needs to know that a guide should stop. It is packed anyway because this message is posted
/// from two places — a drag that comes back onto the strip, and a button-up that never left — and
/// one of them used to send a bare index. Wiring [`drag_indices`] into this arm, which is twelve
/// lines from one that already does, would then have silently read `onto` as `0` down one path.
pub const WM_TAB_DRAG_OFF: u32 = windows::Win32::UI::WindowsAndMessaging::WM_APP + 0x44;

/// `TCHITTESTINFO`, laid out by hand beside [`TcItem`] for the same reason.
#[repr(C)]
struct TcHitTest {
    pt: windows::Win32::Foundation::POINT,
    flags: u32,
}

thread_local! {
    /// The traffic bars' brush, so the subclass can answer `WM_CTLCOLORSTATIC` for them.
    ///
    /// **A thread-local because the subclass proc has no `self`.** It is the same shape the drag
    /// state below uses, and it is sound because the bars are the *only* static children the strip
    /// has — the close buttons are buttons and answer a different message.
    static BAR_BRUSH: std::cell::Cell<isize> = const { std::cell::Cell::new(0) };
    /// Which tab the pointer went down on, while a drag is in progress.
    static DRAGGING: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
    /// Whether that drag has left the strip. Remembered because the button-up that ends it carries
    /// no history, and a release below the strip means something quite different from one inside it.
    static DRAGGED_OUT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// **Which tab was showing when the button went down** — read from the control *before* it is
    /// allowed to act on the press.
    ///
    /// This is the whole reason drag-out-to-split never split anything. `SysTabControl32` selects
    /// on button-**down** and raises `TCN_SELCHANGE` synchronously, so by the time the *posted*
    /// `WM_TAB_DROP_OUT` is handled the shell's active tab is already the tab being dragged. The
    /// shell's "you cannot drop a tab onto its own pane" guard then refused every drop, in all four
    /// directions, with no error and no feedback — the guide simply vanished. Nothing downstream
    /// can recover this: once the selection has moved, the tab that was showing is gone. So it is
    /// captured here, at the only moment it still exists.
    static SHOWING: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
}

/// Packs the dragged tab and the tab being dropped onto into one `WPARAM`.
///
/// Two small indices rather than a second message: a drag has exactly one origin and one
/// destination, and splitting them across messages would let the pair arrive half-updated.
fn drag_pair(from: usize, onto: usize) -> WPARAM {
    WPARAM((from & 0xFFFF) | ((onto & 0xFFFF) << 16))
}

/// Unpacks what [`drag_pair`] wrote: `(the tab being dragged, the tab it is being dropped onto)`.
pub fn drag_indices(wparam: WPARAM) -> (usize, usize) {
    (wparam.0 & 0xFFFF, (wparam.0 >> 16) & 0xFFFF)
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
            // **Before `DefSubclassProc`, which is where the selection moves.** Read after it, this
            // would be the tab just pressed and the drop would always be onto itself.
            let showing = unsafe { SendMessageW(hwnd, TCM_GETCURSEL, WPARAM(0), LPARAM(0)) }.0;
            SHOWING.with(|d| d.set(usize::try_from(showing).ok()));
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
                let onto = SHOWING.with(|d| d.get()).unwrap_or(from);
                tell_parent(hwnd, WM_TAB_DRAG_OUT, drag_pair(from, onto), lparam);
            } else {
                if DRAGGED_OUT.with(|d| d.replace(false)) {
                    // Came back onto the strip: the guide must go, or it hangs about promising a
                    // split that the drop will not perform.
                    let onto = SHOWING.with(|d| d.get()).unwrap_or(from);
                    tell_parent(hwnd, WM_TAB_DRAG_OFF, drag_pair(from, onto), LPARAM(0));
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
                let onto = SHOWING.with(|d| d.replace(None)).unwrap_or(from);
                tell_parent(hwnd, what, drag_pair(from, onto), lparam);
            }
        }
        // §2.1's middle-click close. It has to be handled here: the click lands on this control,
        // not on the window whose handler used to answer it.
        windows::Win32::UI::WindowsAndMessaging::WM_MBUTTONDOWN => {
            if let Some(at) = tab_at(hwnd, x, y) {
                tell_parent(hwnd, WM_TAB_CLOSE, WPARAM(at), LPARAM(0));
            }
        }
        windows::Win32::UI::WindowsAndMessaging::WM_CTLCOLORSTATIC => {
            let brush = BAR_BRUSH.with(|b| b.get());
            if brush != 0 {
                return windows::Win32::Foundation::LRESULT(brush);
            }
        }
        windows::Win32::UI::WindowsAndMessaging::WM_COMMAND => {
            let id = (wparam.0 & 0xFFFF) as u32;
            if let Some(at) = id
                .checked_sub(ID_TAB_CLOSE_BASE)
                .filter(|at| *at < MAX_CLOSERS)
            {
                tell_parent(hwnd, WM_TAB_CLOSE, WPARAM(at as usize), LPARAM(0));
                return windows::Win32::Foundation::LRESULT(0);
            }
        }
        _ => {}
    }
    unsafe { windows::Win32::UI::Shell::DefSubclassProc(hwnd, msg, wparam, lparam) }
}

/// Where a tab's close button sits inside that tab, in the control's client pixels.
///
/// **`SysTabControl32` has neither a close button nor custom draw.** The documented list of
/// controls offering `NM_CUSTOMDRAW` — header, list-view, rebar, toolbar, tooltip, trackbar,
/// tree-view — does not include the tab control, so there is no stage at which an `×` could be
/// drawn over a natively-drawn tab. The alternatives were owner-drawing every tab (taking over the
/// appearance, dark mode included, which is how four toolbar iterations went wrong) or putting the
/// glyph in the item's image list, where the control places it to the *left* of the label. The
/// owner chose a real button per tab, 2026-10-06, so the tab stays native and the button is a
/// standard control over its right edge.
///
/// **Right-aligned and vertically centred, and never outside the tab.** A tab narrower than the
/// button plus its insets yields `None` rather than a button lying over the label or past the tab's
/// edge: a tab that cannot show one does not get one, which is better than one that cannot be read.
/// The width test demands room for the label as well as the button, or the two overlap and the name
/// is the part that loses.
///
/// **The button reports through `WM_TAB_CLOSE`, the message the middle click already sends.** Being
/// a child of the tab control rather than of the window, its `WM_COMMAND` arrives at this module's
/// subclass and never at the shell's menu dispatch — so one close path serves both gestures and
/// there is no second one to keep in step.
pub fn close_button_at(tab: (i32, i32, i32, i32), size: i32, inset: i32) -> Option<(i32, i32)> {
    let (x, y, w, h) = tab;
    let size = size.max(1);
    if w < size + inset * 2 + size || h < size {
        return None;
    }
    Some((x + w - size - inset, y + (h - size) / 2))
}

/// How wide a tab's traffic bar is, given its rate and the busiest tab's.
///
/// **Relative, because the question is comparative.** The owner asked for *"a small highlight bar
/// on each tab as it gets traffic so at a glance the user can see if another tab is getting a lot
/// of new traffic"* — which is a comparison between tabs, not a reading in lines per second. So the
/// busiest tab fills its bar and every other is a fraction of it, and a window where everything is
/// equally busy shows full bars rather than none.
///
/// **A rate of nothing is no bar at all.** A quiet tab drawing a one-pixel sliver would read as
/// faint traffic rather than no traffic, and the point of the bar is to be scanned.
///
/// A visible floor otherwise: a tab at a hundredth of the busiest still gets a pixel or two, since
/// "almost nothing" and "nothing" are different answers and the first deserves to be seen.
pub fn traffic_width(rate: f32, busiest: f32, full: i32) -> i32 {
    if rate.is_nan() || busiest.is_nan() || rate <= 0.0 || busiest <= 0.0 || full <= 0 {
        return 0;
    }
    let share = (rate / busiest).clamp(0.0, 1.0);
    ((share * full as f32).round() as i32).clamp(1, full)
}

/// Windows' tab control, holding one item per open document.
pub struct TabStrip {
    hwnd: HWND,
    font: HFONT,
    /// What the control was last filled with, so a frame that changes nothing does not rebuild it —
    /// a rebuild resets the selection and flickers.
    shown: (Vec<String>, usize),
    visible: bool,
    /// The module handle the close buttons are created from, kept because `place` needs it too.
    instance: HINSTANCE,
    /// One traffic bar per tab, in tab order — a coloured static along the tab's bottom edge.
    ///
    /// **A static with a brush, not drawing.** `SysTabControl32` supports no custom draw, so the
    /// bar is a child window whose background is painted by answering `WM_CTLCOLORSTATIC` — the
    /// pattern the rules editor's colour swatches already use. Nothing here draws by hand.
    bars: Vec<HWND>,
    /// The brush the bars are painted with, made once and freed on drop.
    bar_brush: HBRUSH,
    /// One close button per tab, in tab order.
    ///
    /// **Children of the tab control, not of the window**, so they move with it and are clipped by
    /// it — and so a click on one reaches this module's subclass rather than the shell's handler,
    /// which is where the existing close already lives.
    closers: Vec<HWND>,
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
        // not something this module can fix without drawing the tabs again. The decision itself is
        // the menus' — `controls::apply_theme`, one place for every native surface.
        crate::controls::apply_theme(hwnd, tailhawk_core::theme::theme().dark);
        crate::header::trace(&format!("child: tabstrip hwnd={:?}", hwnd.0));
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
        let brush = unsafe { CreateSolidBrush(COLORREF(0x0040_C040)) };
        BAR_BRUSH.with(|b| b.set(brush.0 as isize));
        Some(TabStrip {
            hwnd,
            font,
            shown: (Vec::new(), usize::MAX),
            visible: false,
            instance,
            bars: Vec::new(),
            bar_brush: brush,
            closers: Vec::new(),
        })
    }

    /// Makes the close buttons match the tabs, and puts each over its own tab's right edge.
    ///
    /// **Called after every `set` and every `place`**, because both can move a tab: filling changes
    /// the widths and placing changes the control's own rectangle. The count is reconciled rather
    /// than rebuilt, so a frame that changed nothing destroys and creates nothing.
    ///
    /// A tab with no room for a button — a long name in a narrow strip — simply has none, and the
    /// button is hidden rather than left somewhere wrong. The menu's `Close tab` and the
    /// middle-click both still work, so nothing becomes unreachable.
    fn sync_closers(&mut self) {
        let instance = self.instance;
        let tabs = self.shown.0.len();
        while self.closers.len() > tabs {
            if let Some(dead) = self.closers.pop() {
                // SAFETY: a live child window this module created and is giving up.
                unsafe {
                    let _ = DestroyWindow(dead);
                }
            }
        }
        while self.closers.len() < tabs {
            let id = ID_TAB_CLOSE_BASE + self.closers.len() as u32;
            // SAFETY: `BUTTON` is a system class and `self.hwnd` is a live control.
            let made = unsafe {
                CreateWindowExW(
                    WINDOW_EX_STYLE::default(),
                    w!("BUTTON"),
                    w!("\u{00d7}"),
                    WINDOW_STYLE(WS_CHILD.0 | WS_CLIPSIBLINGS.0 | BS_FLAT | BS_PUSHBUTTON),
                    0,
                    0,
                    1,
                    1,
                    self.hwnd,
                    HMENU(id as *mut core::ffi::c_void),
                    instance,
                    None,
                )
            };
            match made {
                Ok(h) => {
                    unsafe {
                        SendMessageW(h, WM_SETFONT, WPARAM(self.font.0 as usize), LPARAM(1));
                    }
                    self.closers.push(h);
                }
                Err(_) => break,
            }
        }

        let dpi = unsafe { windows::Win32::UI::HiDpi::GetDpiForWindow(self.hwnd) }.max(96);
        let size = (CLOSE_PX * dpi as i32 / 96).max(8);
        let inset = (CLOSE_INSET * dpi as i32 / 96).max(1);
        for (at, closer) in self.closers.iter().enumerate() {
            let spot = self
                .item_rect(at)
                .and_then(|(x, y, w, h)| {
                    close_button_at((x as i32, y as i32, w as i32, h as i32), size, inset)
                })
                .filter(|_| self.visible);
            // SAFETY: both take a live child handle and retain nothing.
            unsafe {
                match spot {
                    Some((x, y)) => {
                        let _ = SetWindowPos(
                            *closer,
                            HWND_TOP,
                            x,
                            y,
                            size,
                            size,
                            SWP_NOACTIVATE | SWP_NOZORDER,
                        );
                        let _ = ShowWindow(*closer, SW_SHOWNA);
                    }
                    None => {
                        let _ = ShowWindow(*closer, SW_HIDE);
                    }
                }
            }
        }
    }

    /// Fills the control from the shell's labels, and marks which is current.
    ///
    /// Rebuilds only when something actually changed: `TCM_DELETEALLITEMS` followed by inserts
    /// resets the selection and repaints the whole band, so doing it every frame would flicker and
    /// fight the user's own clicks.
    /// Shows each tab's traffic as a bar along its bottom edge, scaled to the busiest.
    ///
    /// **Called every frame with the live rates**, because traffic is the one thing on a tab that
    /// changes without anything else changing: the labels are the same, the selection is the same,
    /// and `set` returns early on both. So this is separate from `set` rather than part of it.
    pub fn show_traffic(&mut self, rates: &[f32]) {
        let busiest = rates.iter().copied().fold(0.0_f32, f32::max);
        let dpi = unsafe { windows::Win32::UI::HiDpi::GetDpiForWindow(self.hwnd) }.max(96);
        let thick = (BAR_PX * dpi as i32 / 96).max(2);
        while self.bars.len() > rates.len() {
            if let Some(dead) = self.bars.pop() {
                // SAFETY: a live child window this module created and is giving up.
                unsafe {
                    let _ = DestroyWindow(dead);
                }
            }
        }
        while self.bars.len() < rates.len() {
            // SAFETY: `STATIC` is a system class and `self.hwnd` is a live control.
            let made = unsafe {
                CreateWindowExW(
                    WINDOW_EX_STYLE::default(),
                    w!("STATIC"),
                    PCWSTR::null(),
                    WINDOW_STYLE(WS_CHILD.0 | WS_CLIPSIBLINGS.0),
                    0,
                    0,
                    1,
                    1,
                    self.hwnd,
                    None,
                    self.instance,
                    None,
                )
            };
            match made {
                Ok(h) => self.bars.push(h),
                Err(_) => break,
            }
        }
        for (at, bar) in self.bars.iter().enumerate() {
            let width = rates
                .get(at)
                .map(|rate| {
                    self.item_rect(at)
                        .map(|(_, _, w, _)| traffic_width(*rate, busiest, w as i32))
                        .unwrap_or(0)
                })
                .unwrap_or(0);
            let spot = self.item_rect(at).filter(|_| self.visible && width > 0);
            // SAFETY: both take a live child handle and retain nothing.
            unsafe {
                match spot {
                    Some((x, y, _, h)) => {
                        let _ = SetWindowPos(
                            *bar,
                            HWND_TOP,
                            x as i32,
                            y as i32 + h as i32 - thick,
                            width,
                            thick,
                            SWP_NOACTIVATE | SWP_NOZORDER,
                        );
                        let _ = ShowWindow(*bar, SW_SHOWNA);
                    }
                    None => {
                        let _ = ShowWindow(*bar, SW_HIDE);
                    }
                }
            }
        }
    }

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
        self.sync_closers();
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
    /// The control's window, for the shell to re-theme when the theme changes.
    /// Re-measures the shell font for `dpi` and gives it to the control.
    ///
    /// **A control keeps the font it was created with**, and this window is created on one monitor
    /// and dragged to another: without this, moving to a 150 % display left every native child
    /// drawing at 100 % beside a grid that had rescaled. `WM_DPICHANGED` is the moment to ask
    /// again, and `SystemParametersInfoForDpi` is what makes the answer per-monitor.
    pub fn set_font(&mut self, dpi: u32) {
        let font = crate::tabstrip::shell_font_for(dpi);
        if font.is_invalid() {
            return;
        }
        unsafe {
            SendMessageW(self.hwnd, WM_SETFONT, WPARAM(font.0 as usize), LPARAM(1));
        }
        // The old one goes only after the control has been told about the new one: a GDI object
        // still selected into a live device context is undefined rather than merely untidy.
        if !self.font.is_invalid() {
            unsafe {
                let _ = DeleteObject(HGDIOBJ(self.font.0));
            }
        }
        self.font = font;
    }

    pub fn hwnd(&self) -> HWND {
        self.hwnd
    }

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

    /// Puts the control across the client area at `top`, or hides it.
    ///
    /// **`top` is the toolbar's band, and it used to be zero.** The strip was the first row and
    /// §2.3's toolbar sat underneath it, which the owner reported on 2026-10-06 as the wrong way
    /// round: every application that has both puts the toolbar above the tabs, because the toolbar
    /// acts on the window and the tabs choose what the window is showing. `UI-DESIGN.md` specified
    /// the old order and has been corrected rather than cited.
    pub fn place(&mut self, top: i32, width: i32, height: i32, visible: bool) {
        if visible {
            unsafe {
                let _ = SetWindowPos(
                    self.hwnd,
                    HWND_TOP,
                    0,
                    top.max(0),
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
        self.sync_closers();
    }
}

impl Drop for TabStrip {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.hwnd);
            if !self.font.is_invalid() {
                let _ = DeleteObject(windows::Win32::Graphics::Gdi::HGDIOBJ(self.font.0));
            }
            if !self.bar_brush.is_invalid() {
                let _ = DeleteObject(HGDIOBJ(self.bar_brush.0));
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
/// The shell's UI font **at a given DPI**.
///
/// `SystemParametersInfoW` answers for the *system* DPI, whatever monitor the window is on: a
/// window dragged to a 150 % display kept the 100 % font, so its native children were drawn small
/// beside a grid that had rescaled. `SystemParametersInfoForDpi` is the same query asked per
/// monitor, and it is what makes `WM_DPICHANGED` mean something for the chrome.
pub fn shell_font_for(dpi: u32) -> HFONT {
    let mut metrics = NONCLIENTMETRICSW {
        cbSize: std::mem::size_of::<NONCLIENTMETRICSW>() as u32,
        ..Default::default()
    };
    let dpi = if dpi == 0 { 96 } else { dpi };
    let ok = unsafe {
        SystemParametersInfoForDpi(
            SPI_GETNONCLIENTMETRICS.0,
            metrics.cbSize,
            Some(&mut metrics as *mut NONCLIENTMETRICSW as *mut core::ffi::c_void),
            0,
            dpi,
        )
    };
    if ok.is_err() {
        // Older than 1607, or a query the system refused: the system-DPI answer is still a font.
        return shell_font();
    }
    unsafe { CreateFontIndirectW(&metrics.lfMessageFont) }
}

pub fn shell_font() -> HFONT {
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

#[cfg(test)]
mod tests {
    use super::*;

    /// **The close button sits inside its tab, at the right, and never over the label.**
    ///
    /// The owner asked for a close icon on each tab, 2026-10-06, so that individual tabs can be
    /// closed directly. The tab control offers neither a close button nor custom draw, so the
    /// button is a real child window and this is the arithmetic that places it — the one part of
    /// the feature that can be tested without a window.
    #[test]
    fn a_close_button_sits_at_the_right_of_its_tab() {
        let tab = (10, 2, 120, 24);
        let (x, y) = close_button_at(tab, 12, 4).expect("a tab this wide has room");
        assert_eq!(x + 12, 10 + 120 - 4, "right-aligned inside the tab");
        assert_eq!(y, 2 + (24 - 12) / 2, "and vertically centred");
        assert!(x > 10, "and not over the tab's left edge");
    }

    /// **A tab too narrow to carry one does not get one.**
    ///
    /// Better than a button lying across the name or hanging past the tab's edge: the label is the
    /// part a reader needs, and the menu and middle-click still close the tab.
    #[test]
    fn a_tab_with_no_room_gets_no_button() {
        assert_eq!(close_button_at((0, 0, 20, 24), 12, 4), None, "too narrow");
        assert_eq!(close_button_at((0, 0, 120, 8), 12, 4), None, "too short");
        assert!(
            close_button_at((0, 0, 36, 24), 12, 4).is_some(),
            "and exactly enough room is enough"
        );
    }

    /// **Every tab's button lands within that tab**, whatever the widths, so no button can be
    /// clicked for the wrong document — which is the one failure of this shape that would be worse
    /// than having no buttons at all.
    #[test]
    fn no_button_strays_into_a_neighbouring_tab() {
        let tabs = [(0, 0, 90, 22), (90, 0, 140, 22), (230, 0, 60, 22)];
        for (x, y, w, h) in tabs {
            if let Some((bx, by)) = close_button_at((x, y, w, h), 12, 4) {
                assert!(bx >= x && bx + 12 <= x + w, "inside horizontally: {bx}");
                assert!(by >= y && by + 12 <= y + h, "and vertically: {by}");
            }
        }
    }
}

#[cfg(test)]
mod traffic_tests {
    use super::*;

    /// **The busiest tab fills its bar and the others are a fraction of it.**
    ///
    /// Asked for on 2026-10-06: a bar on each tab so that at a glance a reader can see whether
    /// another tab is getting a lot of new traffic. That is a comparison between tabs, so the
    /// scale is relative — an absolute lines-per-second would need a legend to mean anything.
    #[test]
    fn the_busiest_tab_fills_its_bar_and_the_rest_are_relative() {
        assert_eq!(traffic_width(100.0, 100.0, 40), 40, "the busiest fills it");
        assert_eq!(
            traffic_width(50.0, 100.0, 40),
            20,
            "half as busy, half as wide"
        );
        assert_eq!(traffic_width(100.0, 100.0, 40), traffic_width(7.0, 7.0, 40));
    }

    /// **No traffic is no bar**, because a one-pixel sliver on a quiet tab reads as faint traffic
    /// rather than none — and the bar exists to be scanned, so a false positive costs more than a
    /// missing one.
    #[test]
    fn a_quiet_tab_has_no_bar_at_all() {
        assert_eq!(traffic_width(0.0, 100.0, 40), 0);
        assert_eq!(traffic_width(100.0, 0.0, 40), 0, "nothing is busy yet");
        assert_eq!(
            traffic_width(f32::NAN, 100.0, 40),
            0,
            "and NaN is not traffic"
        );
        assert_eq!(traffic_width(-5.0, 100.0, 40), 0);
    }

    /// **Almost nothing is still something.** A tab at a hundredth of the busiest gets a pixel, so
    /// the difference between a trickle and silence is visible — the two are different answers.
    #[test]
    fn a_trickle_is_still_drawn() {
        assert_eq!(traffic_width(1.0, 1000.0, 40), 1);
        assert!(traffic_width(1.0, 1_000_000.0, 40) >= 1);
        assert_eq!(
            traffic_width(500.0, 100.0, 40),
            40,
            "and nothing exceeds full"
        );
        assert_eq!(
            traffic_width(10.0, 100.0, 0),
            0,
            "a tab with no room has none"
        );
    }
}
