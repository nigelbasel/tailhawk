//! The column header, as the real Windows control — `UI-DESIGN.md` §2.5.
//!
//! **The owner's reference is Windows Explorer**, and he supplied a screenshot of it: left-aligned
//! titles in the system UI font, a band taller than a data row, thin full-height dividers, the sort
//! caret above the column. Ours was centred, monospace, clipped at both ends (`eve` for `level`),
//! and a fill invisible against the rows at 1.11 : 1. §2.5 measured all of that and this module is
//! the answer it points to: not a better drawing, but `SysHeader32`.
//!
//! **One control per pane.** A header names the columns of the grid beneath it, and a side-by-side
//! split has two grids; one window-wide header would name the left pane's columns over the right
//! pane's rows. Each [`Document`](crate::Document) that has a layout owns one of these, placed over
//! the band its view already reserves — so nothing in the row arithmetic moves; what changes is
//! that the painter draws nothing into that band and the control sits on it.
//!
//! **It decides nothing.** Widths come from [`HeaderColumn`] in cells times the measured cell; the
//! sort mark from the layout; and every notification goes down the model path the drawn band used:
//! `HDN_ENDTRACK` → `set_column_width`, `HDN_ENDDRAG` → `Layout::move_column`,
//! `HDN_ITEMCLICK` → `cycle_sort`. A resize is accepted **in cells, rounded**, because the grid is a
//! cell grid (§3.1) and cannot draw a column two-thirds of a cell wide; the control is told the
//! rounded width back so it never shows a boundary the grid does not honour.

use crate::tabstrip::shell_font;
use tailhawk_core::rows::HeaderColumn;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{DeleteObject, HFONT, HGDIOBJ};
use windows::Win32::UI::Controls::{
    InitCommonControlsEx, HDF_LEFT, HDF_SORTDOWN, HDF_SORTUP, HDF_STRING, HDITEMW, HDI_FORMAT,
    HDI_TEXT, HDI_WIDTH, HDLAYOUT, HDM_DELETEITEM, HDM_GETITEMCOUNT, HDM_INSERTITEMW, HDM_LAYOUT,
    HDM_SETITEMW, HDS_BUTTONS, HDS_DRAGDROP, HDS_FULLDRAG, HDS_HORZ, ICC_LISTVIEW_CLASSES,
    INITCOMMONCONTROLSEX, NMHEADERW, WC_HEADERW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, SendMessageW, SetWindowPos, ShowWindow, HMENU, HWND_TOP,
    SWP_NOACTIVATE, SWP_NOZORDER, SW_HIDE, SW_SHOWNA, WINDOWPOS, WINDOW_EX_STYLE, WINDOW_STYLE,
    WM_SETFONT, WS_CHILD, WS_CLIPSIBLINGS,
};

/// The first control id; pane `n`'s header answers as `ID_HEADER_BASE + n` in `WM_NOTIFY`.
pub const ID_HEADER_BASE: i32 = 4_300;

/// How many panes may carry a header. §3.1 stops at two; the range leaves room without colliding
/// with the other controls' ids.
pub const MAX_HEADERS: i32 = 16;

/// What a notification from the control asks the model to do. **Pure data**, so the mapping from
/// `NMHEADERW` to a request is testable without a window, and the shell only acts on the answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Request {
    /// A boundary was dragged: column `item` (in display order) is now `px` wide.
    Resize { item: usize, px: i32 },
    /// A title was dropped: the column at display slot `from` now sits at slot `to`.
    Reorder { from: usize, to: usize },
    /// A title was clicked: cycle the sort on the column at display slot `item`.
    Sort { item: usize },
}

use windows::Win32::UI::Controls::{HDN_ENDDRAG, HDN_ENDTRACKW, HDN_ITEMCLICKW};

/// Turns a header notification into a [`Request`], or nothing when it is one this does not act on.
///
/// `HDN_ENDDRAG` reports the *target* slot in `pitem.iOrder`; `-1` there is the control saying the
/// drop landed nowhere, which is a cancelled drag and not a move.
pub fn request_of(code: u32, item: i32, order: i32, width: Option<i32>) -> Option<Request> {
    let item = usize::try_from(item).ok()?;
    match code {
        HDN_ENDTRACKW => Some(Request::Resize { item, px: width? }),
        HDN_ENDDRAG => {
            let to = usize::try_from(order).ok()?;
            Some(Request::Reorder { from: item, to })
        }
        HDN_ITEMCLICKW => Some(Request::Sort { item }),
        _ => None,
    }
}

/// Reads the fields a [`Request`] needs out of the raw notification.
///
/// # Safety
/// `lparam` must be the `NMHEADERW*` Windows passed with a `WM_NOTIFY` whose `idFrom` is one of
/// this module's ids; nothing else is dereferenced.
pub unsafe fn request_from_notify(lparam: LPARAM) -> Option<Request> {
    let n = unsafe { &*(lparam.0 as *const NMHEADERW) };
    let (order, width) = if n.pitem.is_null() {
        (-1, None)
    } else {
        let item = unsafe { &*n.pitem };
        // Only a width the control says it is carrying: `cxy` is meaningful when `HDI_WIDTH` is
        // in the mask and garbage otherwise.
        let carried = (item.mask.0 & HDI_WIDTH.0) != 0;
        (item.iOrder, carried.then_some(item.cxy))
    };
    request_of(n.hdr.code, n.iItem, order, width)
}

/// The width in pixels a column of `cells` cells takes, and the inverse, **rounded** — the grid is a
/// cell grid and honours nothing finer.
pub fn px_of_cells(cells: usize, cell_w: f32) -> i32 {
    (cells as f32 * cell_w).round() as i32
}

pub fn cells_of_px(px: i32, cell_w: f32) -> usize {
    if cell_w <= 0.0 {
        return 0;
    }
    (px.max(0) as f32 / cell_w).round() as usize
}

/// The real `SysHeader32`, and everything about it that needs a window.
pub struct Header {
    hwnd: HWND,
    font: HFONT,
    /// What the control was last filled with, so a frame that changes nothing does not rebuild it —
    /// a rebuild would drop a drag that is mid-track.
    shown: Vec<HeaderColumn>,
    visible: bool,
}

impl Header {
    /// Creates the control for pane `pane`, hidden until it is placed.
    pub fn create(parent: HWND, instance: HINSTANCE, pane: usize) -> Option<Header> {
        let icc = INITCOMMONCONTROLSEX {
            dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_LISTVIEW_CLASSES,
        };
        unsafe {
            let _ = InitCommonControlsEx(&icc);
        }
        let id = ID_HEADER_BASE + i32::try_from(pane).ok()?.min(MAX_HEADERS - 1);
        let hwnd = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                WC_HEADERW,
                PCWSTR::null(),
                // `HDS_BUTTONS`: titles that read as clickable, which is the "does not read as a
                // header" complaint §2.5 opens with. `HDS_DRAGDROP`: §2.1's reorder by dragging a
                // title. `HDS_FULLDRAG`: the boundary moves with the pointer instead of a ghost
                // line, which is what Explorer does.
                WINDOW_STYLE(
                    WS_CHILD.0
                        | WS_CLIPSIBLINGS.0
                        | HDS_HORZ
                        | HDS_BUTTONS
                        | HDS_DRAGDROP
                        | HDS_FULLDRAG,
                ),
                0,
                0,
                0,
                0,
                parent,
                HMENU(id as *mut core::ffi::c_void),
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
        crate::controls::apply_theme(hwnd, tailhawk_core::theme::theme().dark);
        Some(Header {
            hwnd,
            font,
            shown: Vec::new(),
            visible: false,
        })
    }

    pub fn hwnd(&self) -> HWND {
        self.hwnd
    }

    /// Fills the control from the layout's boxes. `columns` are in display order, the last taking
    /// the remainder; widths are `cells × cell_w`, the same arithmetic the grid draws by.
    pub fn set(&mut self, columns: &[HeaderColumn], cell_w: f32) {
        if self.shown == columns {
            return;
        }
        let count = unsafe { SendMessageW(self.hwnd, HDM_GETITEMCOUNT, WPARAM(0), LPARAM(0)) }.0;
        for i in (0..count.max(0)).rev() {
            unsafe {
                SendMessageW(self.hwnd, HDM_DELETEITEM, WPARAM(i as usize), LPARAM(0));
            }
        }
        for (i, column) in columns.iter().enumerate() {
            let mut wide: Vec<u16> = column.title.encode_utf16().collect();
            wide.push(0);
            // The caret comes with the box, from the model that decided it — one answer, not a
            // second mapping that could disagree with it.
            let sorted = match column.sort {
                Some(false) => HDF_SORTUP,
                Some(true) => HDF_SORTDOWN,
                None => windows::Win32::UI::Controls::HEADER_CONTROL_FORMAT_FLAGS(0),
            };
            let item = HDITEMW {
                mask: HDI_TEXT | HDI_WIDTH | HDI_FORMAT,
                cxy: px_of_cells(column.cells, cell_w),
                pszText: windows::core::PWSTR(wide.as_mut_ptr()),
                cchTextMax: wide.len() as i32,
                fmt: windows::Win32::UI::Controls::HEADER_CONTROL_FORMAT_FLAGS(
                    HDF_LEFT.0 | HDF_STRING.0 | sorted.0,
                ),
                ..Default::default()
            };
            unsafe {
                SendMessageW(
                    self.hwnd,
                    HDM_INSERTITEMW,
                    WPARAM(i),
                    LPARAM(&item as *const HDITEMW as isize),
                );
            }
        }
        self.shown = columns.to_vec();
    }

    /// Tells the control a width it asked for was rounded to the grid's cells.
    pub fn set_width(&self, item: usize, px: i32) {
        let hd = HDITEMW {
            mask: HDI_WIDTH,
            cxy: px,
            ..Default::default()
        };
        unsafe {
            SendMessageW(
                self.hwnd,
                HDM_SETITEMW,
                WPARAM(item),
                LPARAM(&hd as *const HDITEMW as isize),
            );
        }
        // The next `set` must not think the control still shows the old width.
    }

    /// How tall the control wants to be for its font — `HDM_LAYOUT`'s answer, never a constant.
    pub fn band_height(&self, width: i32) -> i32 {
        let mut rc = RECT {
            left: 0,
            top: 0,
            right: width.max(1),
            bottom: 1000,
        };
        let mut wp = WINDOWPOS::default();
        let mut layout = HDLAYOUT {
            prc: &mut rc,
            pwpos: &mut wp,
        };
        let ok = unsafe {
            SendMessageW(
                self.hwnd,
                HDM_LAYOUT,
                WPARAM(0),
                LPARAM(&mut layout as *mut HDLAYOUT as isize),
            )
        };
        if ok.0 == 0 {
            return 0;
        }
        wp.cy.clamp(1, 200)
    }

    /// Places the control over the pane's header band. `x` may be negative when the grid is scrolled
    /// horizontally: the control is wider than the viewport and shifted left by the scroll, exactly
    /// as a list view keeps its header aligned with its columns.
    pub fn place(&mut self, x: i32, top: i32, width: i32, height: i32, visible: bool) {
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
        }
        if visible != self.visible {
            unsafe {
                let _ = ShowWindow(self.hwnd, if visible { SW_SHOWNA } else { SW_HIDE });
            }
            self.visible = visible;
        }
    }
}

impl Drop for Header {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.hwnd);
            if !self.font.is_invalid() {
                let _ = DeleteObject(HGDIOBJ(self.font.0));
            }
        }
    }
}

/// SAFETY: the control is created on the window thread and every message to it is sent from that
/// thread; nothing here touches the handle from anywhere else. The impl exists because a
/// [`Document`](crate::Document) is built on a worker and sent to the window thread over a
/// channel, and a `Document` now carries an `Option<Header>` — always `None` at that moment, the
/// control being made only once the shell lays the pane out. An `HWND` is an integer the system
/// owns; moving the integer between threads is not the same as using it from one.
unsafe impl Send for Header {}

#[cfg(test)]
mod tests {
    use super::*;

    /// The three gestures §2.5 names, each from the notification the control sends for it.
    #[test]
    fn each_notification_becomes_the_request_the_drawn_band_used() {
        assert_eq!(
            request_of(HDN_ENDTRACKW, 2, -1, Some(140)),
            Some(Request::Resize { item: 2, px: 140 })
        );
        assert_eq!(
            request_of(HDN_ENDDRAG, 0, 3, None),
            Some(Request::Reorder { from: 0, to: 3 })
        );
        assert_eq!(
            request_of(HDN_ITEMCLICKW, 1, -1, None),
            Some(Request::Sort { item: 1 })
        );
    }

    /// A drag the control reports as landing nowhere is a cancelled drag, not a move to column -1.
    #[test]
    fn a_drop_that_landed_nowhere_is_not_a_reorder() {
        assert_eq!(request_of(HDN_ENDDRAG, 0, -1, None), None);
    }

    /// A track that carries no width cannot resize anything, and an item of -1 names no column.
    #[test]
    fn a_track_without_a_width_and_an_item_of_minus_one_are_ignored() {
        assert_eq!(request_of(HDN_ENDTRACKW, 2, -1, None), None);
        assert_eq!(request_of(HDN_ITEMCLICKW, -1, -1, None), None);
    }

    /// **The item is the box, gap included; the model holds the content.** The first wiring fed
    /// one into the other, so a divider pressed and released without moving grew its column by
    /// the gap — two cells — every time. The review caught it; this pins the arithmetic the
    /// notify arm must do: box in pixels → cells → minus the gap → content, and back.
    #[test]
    fn a_box_width_becomes_content_by_losing_the_gap_and_gets_it_back() {
        use tailhawk_core::columns::GAP;
        let (content, cell_w) = (5usize, 10.0);
        let box_px = px_of_cells(content + GAP, cell_w);
        let read_back = cells_of_px(box_px, cell_w).saturating_sub(GAP);
        assert_eq!(
            read_back, content,
            "a drag that moved nothing must change nothing"
        );
        assert_eq!(
            px_of_cells(read_back + GAP, cell_w),
            box_px,
            "and the control is told the same box back"
        );
    }

    /// **A width round-trips through cells and back, rounded.** The grid cannot draw two-thirds of
    /// a cell, so a boundary dragged to 137 px at a 10 px cell is fourteen cells and the control is
    /// told 140 — never left showing a divider the grid does not honour.
    #[test]
    fn a_width_is_rounded_to_whole_cells() {
        assert_eq!(cells_of_px(137, 10.0), 14);
        assert_eq!(px_of_cells(14, 10.0), 140);
        assert_eq!(cells_of_px(-5, 10.0), 0, "a negative width is no width");
        assert_eq!(cells_of_px(100, 0.0), 0, "a zero cell divides nothing");
    }
}
