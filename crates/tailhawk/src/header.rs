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
use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{DeleteObject, ScreenToClient, SetTextColor, HFONT, HGDIOBJ};
use windows::Win32::UI::Controls::{
    InitCommonControlsEx, HDF_FIXEDWIDTH, HDF_LEFT, HDF_SORTDOWN, HDF_SORTUP, HDF_STRING,
    HDHITTESTINFO, HDITEMW, HDI_FORMAT, HDI_TEXT, HDI_WIDTH, HDLAYOUT, HDM_DELETEITEM,
    HDM_GETITEMCOUNT, HDM_HITTEST, HDM_INSERTITEMW, HDM_LAYOUT, HDM_SETITEMW, HDS_BUTTONS,
    HDS_DRAGDROP, HDS_FULLDRAG, HDS_HORZ, ICC_LISTVIEW_CLASSES, INITCOMMONCONTROLSEX, NMCUSTOMDRAW,
    NMCUSTOMDRAW_DRAW_STAGE, NMHEADERW, WC_HEADERW,
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
    /// A boundary was double-clicked: column `item` goes back to the width it was measured at.
    /// §2.5's other lost gesture, and the way back from [`width_for`]'s hide.
    Reset { item: usize },
}

use windows::Win32::UI::Controls::{
    HDN_DIVIDERDBLCLICKW, HDN_ENDDRAG, HDN_ENDTRACKW, HDN_ITEMCLICKW,
};

/// Whether `code` is one of the three notifications whose `lParam` really is an `NMHEADERW` with a
/// `pitem` worth reading. Everything else the control sends shares the `NMHDR` and nothing more.
/// `NM_CUSTOMDRAW`, which the `windows` crate does not bind as a `u32`.
pub const NM_CUSTOMDRAW: u32 = (-12_i32) as u32;

/// The custom-draw stages and answers this uses. Named here because the crate binds them as typed
/// constants of three different types and the message wants a plain `LRESULT`.
const CDDS_PREPAINT: NMCUSTOMDRAW_DRAW_STAGE = NMCUSTOMDRAW_DRAW_STAGE(0x0000_0001);
const CDDS_ITEMPREPAINT: NMCUSTOMDRAW_DRAW_STAGE = NMCUSTOMDRAW_DRAW_STAGE(0x0001_0001);
const CDRF_DODEFAULT: isize = 0;
const CDRF_NEWFONT: isize = 2;
const CDRF_NOTIFYITEMDRAW: isize = 0x20;

/// Answers the header's `NM_CUSTOMDRAW` so its titles are drawn in the theme's ink.
///
/// **The dark class gives a dark band and leaves the text where it was.** `DarkMode_ItemsView` is
/// the right class — it is what Explorer's list gives its header, and it is what turned this band
/// from white to dark — but the title text kept the light theme's near-black, which over the dark
/// band is the owner's *"the column header is now black, but so is the text"*. The colour of item
/// text in a custom-drawn control is the device context's, so this sets it at
/// `CDDS_ITEMPREPAINT` and answers `CDRF_NEWFONT`, which is how a control is told the attributes
/// it should now draw with.
///
/// Returns the value the window procedure must return, or `None` when this is not a stage worth
/// answering — a caller that returns zero for those is correct, `CDRF_DODEFAULT` being zero.
///
/// # Safety
///
/// `lparam` must be the `NMCUSTOMDRAW` a header sent with `NM_CUSTOMDRAW`. The stage is read
/// before anything else in the structure, and only the device context is touched.
pub unsafe fn custom_draw(lparam: LPARAM, ink: u32) -> Option<isize> {
    if lparam.0 == 0 {
        return None;
    }
    let draw = unsafe { &*(lparam.0 as *const NMCUSTOMDRAW) };
    match draw.dwDrawStage {
        CDDS_PREPAINT => Some(CDRF_NOTIFYITEMDRAW),
        CDDS_ITEMPREPAINT => {
            unsafe {
                SetTextColor(draw.hdc, COLORREF(ink));
            }
            Some(CDRF_NEWFONT)
        }
        _ => Some(CDRF_DODEFAULT),
    }
}

/// The theme's header ink as a `COLORREF` — `0x00BBGGRR`, which is the byte order GDI wants and
/// the reverse of the one everything else in this program writes.
pub fn header_ink() -> u32 {
    let ink = tailhawk_core::theme::theme().header_ink;
    let byte = |c: f32| (c.clamp(0.0, 1.0) * 255.0).round() as u32;
    byte(ink[0]) | (byte(ink[1]) << 8) | (byte(ink[2]) << 16)
}

/// The width a boundary drag asks for, in cells — or **zero, meaning hide the column**.
///
/// §2.5 gave the drawn band two gestures that the control's own drag did not carry over: a
/// boundary pulled all the way in hides its column, and a double-click on a boundary puts it back.
/// This is the first of them. A drag that arrives narrower than the gap between columns is a drag
/// to nothing, and a one-cell column that cannot show a single character is not a narrower column,
/// it is a column the user is trying to get rid of.
///
/// `gap` is `columns::GAP`, the space the layout puts between columns: the item's width includes
/// it, the model's does not.
pub fn width_for(px: i32, cell_w: f32, gap: usize) -> usize {
    let cells = cells_of_px(px, cell_w);
    if cells <= gap {
        return 0;
    }
    cells - gap
}

pub fn carries_item(code: u32) -> bool {
    matches!(
        code,
        HDN_ENDTRACKW | HDN_ENDDRAG | HDN_ITEMCLICKW | HDN_DIVIDERDBLCLICKW
    )
}

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
        // **No width is read for this one**, and that is deliberate: `HDN_DIVIDERDBLCLICK` carries
        // the divider's index and nothing else worth having. `request_from_notify` tolerates a null
        // `pitem` for exactly this reason.
        HDN_DIVIDERDBLCLICKW => Some(Request::Reset { item }),
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
    // **The code is read before anything else is believed.** A header sends far more than the
    // three notifications this acts on — `NM_CUSTOMDRAW` on every paint, for one — and those
    // arrive as *other* structs behind the same `NMHDR`. Reading `pitem` out of an `NMCUSTOMDRAW`
    // is reading a pointer out of its `rc`, and following it from inside a paint callback is an
    // access violation the kernel swallows on x64: no crash, no panic, a thread that never comes
    // back. That was the hang.
    if !carries_item(n.hdr.code) {
        return None;
    }
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

/// Diagnostic: appends a line to the file `TAILHAWK_ATLAS_LOG` names, if it names one — the same
/// switch the glyph cache logs under, so one file carries the frame in order.
pub(crate) fn trace(line: &str) {
    use std::io::Write;
    use std::sync::OnceLock;
    static PATH: OnceLock<Option<String>> = OnceLock::new();
    // Stamped with elapsed milliseconds since the first line, so a stage's cost is a subtraction.
    let Some(path) = PATH.get_or_init(|| std::env::var("TAILHAWK_ATLAS_LOG").ok()) else {
        return;
    };
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs_f64() * 1000.0)
            .unwrap_or(0.0);
        let _ = writeln!(f, "[{ms:.3}] {line}");
    }
}

/// The header box a control item names, or `None` for the gutter item at 0, which names nothing.
///
/// Every gesture goes through this before touching the model: a resize of item 0 is refused by the
/// control (`HDF_FIXEDWIDTH`), but a drag *onto* it or a click on it still arrives, and the model
/// must never be asked to act on a column that is not there.
pub fn item_to_box(item: usize) -> Option<usize> {
    item.checked_sub(1)
}

/// The header item under a screen point, for the context menu that had no way in.
///
/// **§2.4's header menu was unreachable.** `Document::header_hit` returns `None` whenever the real
/// control exists — right in itself, since the control owns its band — but that was the only branch
/// the header's context menu was built under, and the control exists for every pane with columns.
/// `Sort ascending`, `Sort descending`, `Top N…`, `Filter on <column>…` and `Clear sort` could
/// therefore be reached by neither mouse nor keyboard. The control has to answer for its own band,
/// and `HDM_HITTEST` is how it says which item a point is on.
///
/// A free function taking the handle, so the caller can send this message with no `STATE` borrow
/// held. `None` means the point is on no item — the strip past the last column, most often.
pub fn item_at(hwnd: HWND, screen_x: i32, screen_y: i32) -> Option<usize> {
    let mut hit = HDHITTESTINFO {
        pt: POINT {
            x: screen_x,
            y: screen_y,
        },
        ..Default::default()
    };
    unsafe {
        if !ScreenToClient(hwnd, &mut hit.pt).as_bool() {
            return None;
        }
        SendMessageW(
            hwnd,
            HDM_HITTEST,
            WPARAM(0),
            LPARAM(&mut hit as *mut HDHITTESTINFO as isize),
        );
    }
    (hit.iItem >= 0).then_some(hit.iItem as usize)
}

/// The layout column a control item names, given the boxes the control was filled from.
///
/// **The box knows its column; the item's number does not.** Items are `header_columns`' boxes in
/// order after the gutter, but a hidden column leaves the layout's numbering with a gap — so
/// arithmetic on the item index would act on the neighbour of the column the user clicked. This is
/// the same read the notification path does, given a name so the context menu cannot do it a
/// second, different way.
pub fn column_of_item(boxes: &[HeaderColumn], item: usize) -> Option<usize> {
    item_to_box(item)
        .and_then(|b| boxes.get(b))
        .map(|b| b.column)
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
    /// The blank first item's width when the control was last filled — the gutter's, so the
    /// header spans it the way Explorer's spans its list from the left edge.
    gutter: i32,
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
        trace(&format!("child: header hwnd={:?}", hwnd.0));
        // **The list classes, not Explorer's.** A header themed `DarkMode_Explorer` stays white with
        // black text over a dark grid; `DarkMode_ItemsView` is the class Explorer's own file list
        // gives its header.
        crate::controls::apply_theme_class(
            hwnd,
            tailhawk_core::theme::theme().dark,
            windows::core::w!("DarkMode_ItemsView"),
            windows::core::w!("ItemsView"),
        );
        Some(Header {
            hwnd,
            font,
            shown: Vec::new(),
            gutter: 0,
            visible: false,
        })
    }

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

    /// Fills the control from the layout's boxes. `columns` are in display order, the last taking
    /// the remainder; widths are `cells × cell_w`, the same arithmetic the grid draws by.
    pub fn set(&mut self, columns: &[HeaderColumn], cell_w: f32, gutter_px: i32) {
        if self.shown == columns && self.gutter == gutter_px {
            return;
        }
        trace(&format!("header.set rebuild items={}", columns.len()));
        let count = unsafe { SendMessageW(self.hwnd, HDM_GETITEMCOUNT, WPARAM(0), LPARAM(0)) }.0;
        for i in (0..count.max(0)).rev() {
            unsafe {
                SendMessageW(self.hwnd, HDM_DELETEITEM, WPARAM(i as usize), LPARAM(0));
            }
        }
        // **Item 0 is the gutter: blank, fixed, and never a column.** The owner's answer on
        // 2026-09-03 was "same as Explorer", whose header runs from the list's left edge; ours
        // starts where the line numbers end, so the band over them was bare. A fixed-width item
        // cannot be dragged to resize, and [`item_to_box`] keeps it out of every gesture.
        let gutter_item = HDITEMW {
            mask: HDI_WIDTH | HDI_FORMAT,
            cxy: gutter_px.max(0),
            fmt: windows::Win32::UI::Controls::HEADER_CONTROL_FORMAT_FLAGS(
                HDF_LEFT.0 | HDF_FIXEDWIDTH.0,
            ),
            ..Default::default()
        };
        unsafe {
            SendMessageW(
                self.hwnd,
                HDM_INSERTITEMW,
                WPARAM(0),
                LPARAM(&gutter_item as *const HDITEMW as isize),
            );
        }
        for (i, column) in columns.iter().enumerate().map(|(i, c)| (i + 1, c)) {
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
        self.gutter = gutter_px;
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
        trace("header.band_height enter");
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
        trace(&format!(
            "header.place x={x} top={top} w={width} h={height} visible={visible}"
        ));
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

    /// **A hidden column is why this is a read and not a subtraction.** The boxes skip what is
    /// hidden, so with column 1 hidden the second visible box still names column 2 — and a context
    /// menu that had worked out "item 2 means column 1" would sort the column beside the one the
    /// user right-clicked, which is the sort of wrong that looks like a bug in sorting.
    #[test]
    fn an_item_names_its_boxs_column_even_when_one_is_hidden() {
        let box_at = |title: &str, column: usize| HeaderColumn {
            title: title.to_owned(),
            column,
            start: 0,
            cells: 8,
            content: 7,
            sort: None,
        };
        let boxes = [
            box_at("timestamp", 0),
            box_at("message", 2),
            box_at("trace", 5),
        ];
        assert_eq!(column_of_item(&boxes, 0), None, "the gutter names nothing");
        assert_eq!(column_of_item(&boxes, 1), Some(0));
        assert_eq!(column_of_item(&boxes, 2), Some(2), "not column 1");
        assert_eq!(column_of_item(&boxes, 3), Some(5));
        assert_eq!(column_of_item(&boxes, 4), None, "past the last box");
    }

    /// Item 0 is the gutter and names no box; every other item names the box one before it.
    #[test]
    fn the_gutter_item_names_no_box_and_the_rest_shift_by_one() {
        assert_eq!(item_to_box(0), None);
        assert_eq!(item_to_box(1), Some(0));
        assert_eq!(item_to_box(4), Some(3));
    }

    /// **`NM_CUSTOMDRAW` carries no item and must never be read as one.** The header sends it on
    /// every paint, and the first wiring read `pitem` out of it — a pointer taken from the middle of
    /// an `NMCUSTOMDRAW` — and followed it from inside the paint: an access violation the kernel
    /// swallows on x64, and a window that never painted again after its first follow tick.
    #[test]
    fn only_the_three_item_notifications_are_read_as_items() {
        use windows::Win32::UI::Controls::{HDN_ITEMCHANGINGW, HDN_TRACKW, NM_CUSTOMDRAW};
        assert!(carries_item(HDN_ENDTRACKW));
        assert!(carries_item(HDN_ENDDRAG));
        assert!(carries_item(HDN_ITEMCLICKW));
        assert!(
            !carries_item(NM_CUSTOMDRAW),
            "custom draw is not an item notification"
        );
        assert!(!carries_item(HDN_ITEMCHANGINGW));
        assert!(!carries_item(HDN_TRACKW));
    }

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

    /// §2.5's hide: a boundary pulled in past the gap is not a narrower column, it is a column the
    /// user is getting rid of. The drawn band did this and the control's own drag did not carry it
    /// over, which is why a column could be squeezed to one useless cell and never removed.
    #[test]
    fn a_boundary_pulled_all_the_way_in_hides_the_column() {
        let cell = 8.0;
        let gap = tailhawk_core::columns::GAP;
        assert_eq!(width_for(0, cell, gap), 0);
        assert_eq!(width_for(4, cell, gap), 0, "half a cell is nothing");
        assert_eq!(
            width_for(px_of_cells(gap, cell), cell, gap),
            0,
            "the gap alone is nothing"
        );
        assert_eq!(
            width_for(px_of_cells(gap + 1, cell), cell, gap),
            1,
            "one cell past the gap is one cell of content"
        );
        assert_eq!(width_for(px_of_cells(gap + 12, cell), cell, gap), 12);
    }

    /// §2.5's way back. **No width is read for this notification**, which is the whole reason it is
    /// safe to add to `carries_item`: `HDN_DIVIDERDBLCLICK` carries the divider's index and a
    /// `pitem` that may be null, and `request_from_notify` already tolerates that.
    #[test]
    fn a_double_clicked_boundary_asks_for_a_reset() {
        assert_eq!(
            request_of(HDN_DIVIDERDBLCLICKW, 3, -1, None),
            Some(Request::Reset { item: 3 })
        );
        assert!(carries_item(HDN_DIVIDERDBLCLICKW));
        assert_eq!(
            request_of(HDN_ENDTRACKW, 3, -1, None),
            None,
            "a resize with no width is still not a request"
        );
    }
}
