//! The record detail as a **real window** — `UI-DESIGN.md` §8, rebuilt on 2026-09-09.
//!
//! **The owner's words:** *"Why are you drawing it. you should only be drawing the tail window
//! since that is what needs the high perfomance. Surely the detail window should follow a standard
//! windows convention. at present it look like it was randomly thrown there and not designed with
//! any aesthetic in mind. It feels like details should be a popup with a property page control, or
//! at least normal windows dialog sematices."*
//!
//! He is right on both counts. The pane was text composed into cells and painted over the grid by
//! the same renderer that draws fifty thousand rows a second — a renderer that exists because
//! *scrolling a log* is hard, and that has nothing to offer a static list of six fields. It also
//! could not select, could not copy, could not scroll on its own, and drew a rule line out of a box
//! character the glyph atlas refused, which is the row of squares he saw.
//!
//! **So it is a modeless dialog with a tab control**: a property page in everything but the API.
//! *Fields* is a two-column list, *Message* is the record's body, *Raw* is the bytes as they are in
//! the file. The list and the edits are Windows' own, so selection, copy, scrolling, the keyboard,
//! high contrast and the screen reader all arrive without being written.
//!
//! **Modeless, and that is the whole point of the window.** The log goes on scrolling behind it and
//! the pane follows the caret; a modal box would freeze the thing the user is reading.

use tailhawk_core::detail::Detail;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, RECT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    DestroyWindow, GetClientRect, IsWindow, SendMessageW, SetWindowPos, ShowWindow, HWND_TOP,
    SWP_NOACTIVATE, SWP_NOZORDER, SW_HIDE, SW_SHOWNA,
};

/// The detail of one record, owned, as one frame should show it.
///
/// **A view-model, in this project's sense**: no `HWND`, no device, no borrow of the row text it
/// came from. [`view_of`] is the mapping and it is pure, so what the window will show is a test.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DetailView {
    /// The physical line, one-based — the window's title says it.
    pub line: u64,
    /// The format's fields in column order. Empty when no format was accepted, which is exactly
    /// when the *Fields* page has nothing to say.
    pub fields: Vec<(String, String)>,
    /// The record's body, with its continuation lines under it, ready for an edit control: CRLF,
    /// because an edit control draws a lone `\n` as a box.
    pub body: String,
    /// The record exactly as it stands in the file, continuations included.
    pub raw: String,
}

/// The window's three pages, in order.
pub const PAGES: [&str; 3] = ["Fields", "Message", "Raw"];

/// Turns a composed [`Detail`] into the window's view-model.
///
/// `raw_first` is the record's first line as it stands in the file — `Detail::body` is only the
/// *message* once a format has taken the line apart, so the Raw page needs the line itself.
///
/// `pretty` re-indents a JSON body, which is §8's *Pretty*: a courtesy on the Message page only.
/// **Raw is never re-indented** — the page exists to answer "what does the file actually say".
pub fn view_of(detail: &Detail<'_>, raw_first: &str, pretty: bool) -> DetailView {
    let body = match pretty
        .then(|| tailhawk_core::detail::pretty_json(detail.body))
        .flatten()
    {
        Some(indented) => indented,
        None => detail.body.to_owned(),
    };
    let mut message = crlf(&body);
    for line in &detail.tail {
        message.push_str("\r\n");
        message.push_str(&crlf(line));
    }
    let mut raw = crlf(raw_first);
    for line in &detail.tail {
        raw.push_str("\r\n");
        raw.push_str(&crlf(line));
    }
    DetailView {
        line: detail.line,
        fields: detail
            .fields
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect(),
        body: message,
        raw,
    }
}

/// Line endings an edit control understands. A file's lone `\n` draws as a box in one, and a `\r`
/// on its own moves the caret without ending the line.
fn crlf(text: &str) -> String {
    text.replace("\r\n", "\n")
        .replace('\r', "\n")
        .replace('\n', "\r\n")
}

/// The title the window carries for a record: what it is, then which one.
///
/// *Window management*: "the window title is the first thing a user reads in the taskbar", so the
/// number is in it rather than in a label inside the window that a shrunken window would hide.
pub fn title_of(view: &DetailView) -> String {
    format!("Record {} — Detail", with_separators(view.line))
}

/// A line number a person can read at a glance: `1,204,915`.
fn with_separators(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (at, c) in digits.chars().enumerate() {
        if at > 0 && (digits.len() - at).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// Which page can say anything about this record.
///
/// **A page with nothing on it is worse than no page.** Without a format there are no fields, so
/// *Fields* is empty; and when the message is the whole line, *Raw* and *Message* say the same
/// thing. The guide's rule is that a control which cannot act is disabled rather than removed, but
/// a tab control has no disabled tab — so this reports which pages to build, and the window builds
/// only those.
pub fn pages_of(view: &DetailView) -> Vec<&'static str> {
    let mut pages: Vec<&'static str> = Vec::with_capacity(PAGES.len());
    if !view.fields.is_empty() {
        pages.push(PAGES[0]);
    }
    pages.push(PAGES[1]);
    if view.raw != view.body {
        pages.push(PAGES[2]);
    }
    pages
}

/// `TCM_*`, as `tabstrip.rs` declares them: the crate binds the class and not the messages.
const TCM_FIRST: u32 = 0x1300;
const TCM_GETCURSEL: u32 = TCM_FIRST + 11;
const TCM_DELETEALLITEMS: u32 = TCM_FIRST + 9;
const TCM_ADJUSTRECT: u32 = TCM_FIRST + 40;
const TCM_INSERTITEMW: u32 = TCM_FIRST + 62;

/// `TCITEMW`, laid out by hand for the same reason `tabstrip.rs` lays it out by hand.
#[repr(C)]
#[derive(Default)]
struct TcItemW {
    mask: u32,
    state: u32,
    state_mask: u32,
    text: *mut u16,
    text_max: i32,
    image: i32,
    param: isize,
}

const TCIF_TEXT: u32 = 0x0001;

/// The control ids inside the window.
pub const ID_TABS: u16 = 300;
pub const ID_LIST: u16 = 301;
pub const ID_TEXT: u16 = 302;

/// The pixels of air between the tab control and the window's edge, in dialog units.
const MARGIN: i32 = 7;

/// The detail window while it is up.
///
/// **It is not a `Document`'s** — one window shows whichever record the active document's caret is
/// on, the way Explorer's preview pane belongs to the window rather than to a folder. Switching
/// tabs moves it to the new document's record instead of leaving a second window behind.
pub struct DetailWindow {
    hwnd: HWND,
    /// What it was last filled with, so a frame that changes nothing does not rebuild a list view
    /// under a user who is selecting text in it.
    shown: Option<DetailView>,
    /// The pages the tab control currently carries, so a record with the same shape does not
    /// rebuild them either.
    pages: Vec<&'static str>,
}

impl DetailWindow {
    /// The window's handle, for the message loop's `IsDialogMessageW` and for the shell's own
    /// bookkeeping. Invalid once the user has closed it.
    pub fn hwnd(&self) -> HWND {
        self.hwnd
    }

    /// Whether the window is still there — the user can close it from its own title bar, and the
    /// shell must notice rather than go on pushing records into a destroyed window.
    pub fn alive(&self) -> bool {
        !self.hwnd.is_invalid() && unsafe { IsWindow(self.hwnd) }.as_bool()
    }

    /// Puts the tab control and the page it shows where the window's client area is now.
    ///
    /// `TCM_ADJUSTRECT` is what says where a tab control's *display area* is; guessing it is how a
    /// page ends up under the tabs at one DPI and short of them at another.
    pub fn lay_out(&self, dpi: u32) {
        let mut client = RECT::default();
        if unsafe { GetClientRect(self.hwnd, &mut client) }.is_err() {
            return;
        }
        let margin = (MARGIN * dpi.max(96) as i32) / 96;
        let tabs_rect = RECT {
            left: client.left + margin,
            top: client.top + margin,
            right: client.right - margin,
            bottom: client.bottom - margin,
        };
        let Some(tabs) = self.item(ID_TABS) else {
            return;
        };
        unsafe {
            let _ = SetWindowPos(
                tabs,
                HWND_TOP,
                tabs_rect.left,
                tabs_rect.top,
                (tabs_rect.right - tabs_rect.left).max(0),
                (tabs_rect.bottom - tabs_rect.top).max(0),
                SWP_NOACTIVATE | SWP_NOZORDER,
            );
        }
        let mut page = tabs_rect;
        unsafe {
            SendMessageW(
                tabs,
                TCM_ADJUSTRECT,
                WPARAM(0),
                LPARAM(&mut page as *mut RECT as isize),
            );
        }
        for id in [ID_LIST, ID_TEXT] {
            if let Some(child) = self.item(id) {
                unsafe {
                    let _ = SetWindowPos(
                        child,
                        HWND_TOP,
                        page.left,
                        page.top,
                        (page.right - page.left).max(0),
                        (page.bottom - page.top).max(0),
                        SWP_NOACTIVATE | SWP_NOZORDER,
                    );
                }
            }
        }
    }

    /// Shows the child that belongs to the selected page and hides the other.
    pub fn show_page(&self) {
        let Some(tabs) = self.item(ID_TABS) else {
            return;
        };
        let at = unsafe { SendMessageW(tabs, TCM_GETCURSEL, WPARAM(0), LPARAM(0)) }.0;
        let page = self.pages.get(at.max(0) as usize).copied();
        let list_wanted = page == Some(PAGES[0]);
        for (id, wanted) in [(ID_LIST, list_wanted), (ID_TEXT, !list_wanted)] {
            if let Some(child) = self.item(id) {
                unsafe {
                    let _ = ShowWindow(child, if wanted { SW_SHOWNA } else { SW_HIDE });
                }
            }
        }
    }

    fn item(&self, id: u16) -> Option<HWND> {
        unsafe { windows::Win32::UI::WindowsAndMessaging::GetDlgItem(self.hwnd, i32::from(id)) }
            .ok()
    }

    /// Fills the tab control with the pages this record has, keeping the selected one if it is
    /// still there.
    fn set_pages(&mut self, pages: &[&'static str]) {
        if self.pages == pages {
            return;
        }
        let Some(tabs) = self.item(ID_TABS) else {
            return;
        };
        unsafe {
            SendMessageW(tabs, TCM_DELETEALLITEMS, WPARAM(0), LPARAM(0));
        }
        for (at, page) in pages.iter().enumerate() {
            let mut text: Vec<u16> = page.encode_utf16().chain(std::iter::once(0)).collect();
            let mut item = TcItemW {
                mask: TCIF_TEXT,
                text: text.as_mut_ptr(),
                ..Default::default()
            };
            unsafe {
                SendMessageW(
                    tabs,
                    TCM_INSERTITEMW,
                    WPARAM(at),
                    LPARAM(&mut item as *mut TcItemW as isize),
                );
            }
        }
        self.pages = pages.to_vec();
    }
}

impl Drop for DetailWindow {
    fn drop(&mut self) {
        if self.alive() {
            unsafe {
                let _ = DestroyWindow(self.hwnd);
            }
        }
    }
}

/// The window class of the edit control the Message and Raw pages share.
pub const EDIT_CLASS: PCWSTR = w!("EDIT");

#[cfg(test)]
mod tests {
    use super::*;

    fn detail<'a>(
        fields: Vec<(&'a str, &'a str)>,
        body: &'a str,
        tail: Vec<&'a str>,
    ) -> Detail<'a> {
        Detail {
            line: 1_204_915,
            fields,
            body,
            tail,
        }
    }

    /// The view-model carries what the three pages show, and **every line ending is CRLF**: an
    /// edit control draws a lone `\n` as a box, which is the same class of thing as the row of
    /// squares that made the owner ask for this window in the first place.
    #[test]
    fn the_pages_get_crlf_and_the_raw_page_gets_the_line_itself() {
        let d = detail(
            vec![("level", "ERROR")],
            "Failed to dispatch",
            vec!["   at Api.Dispatch()", "   at Api.Run()"],
        );
        let view = view_of(&d, "2026-08-16 09:14|ERROR|Api|Failed to dispatch", false);
        assert_eq!(view.line, 1_204_915);
        assert_eq!(view.fields, [("level".to_owned(), "ERROR".to_owned())]);
        assert_eq!(
            view.body,
            "Failed to dispatch\r\n   at Api.Dispatch()\r\n   at Api.Run()"
        );
        assert!(
            view.raw.starts_with("2026-08-16 09:14|ERROR|Api|"),
            "raw is the file's line, not the message: {}",
            view.raw
        );
        assert!(!view.body.contains('\n') || view.body.contains("\r\n"));
        assert!(view.raw.ends_with("   at Api.Run()"));
    }

    /// A file line that already carries `\r\n` must not become `\r\r\n`, which an edit control
    /// shows as a blank line between every pair.
    #[test]
    fn a_line_that_is_already_crlf_is_not_doubled() {
        let d = detail(vec![], "one\r\ntwo", vec![]);
        let view = view_of(&d, "one\r\ntwo", false);
        assert_eq!(view.body, "one\r\ntwo");
        assert!(!view.body.contains("\r\r"));
    }

    /// **Pretty is the Message page's courtesy and never the Raw page's.** §8: Raw shows the
    /// original bytes exactly as they appear in the file.
    #[test]
    fn pretty_reaches_the_message_and_never_the_raw() {
        let json = r#"{"a":1,"b":[2,3]}"#;
        let d = detail(vec![("level", "INFO")], json, vec![]);
        let view = view_of(&d, json, true);
        assert!(view.body.contains("\r\n"), "re-indented: {:?}", view.body);
        assert_eq!(view.raw, json, "the file's bytes, untouched");
    }

    /// The window builds only the pages that have something on them: no format means no fields,
    /// and a line that is all message means Raw would repeat Message word for word.
    #[test]
    fn a_page_with_nothing_on_it_is_not_built() {
        let plain = view_of(&detail(vec![], "just a line", vec![]), "just a line", false);
        assert_eq!(pages_of(&plain), ["Message"]);

        let full = view_of(
            &detail(vec![("level", "WARN")], "body", vec![]),
            "12:00 WARN body",
            false,
        );
        assert_eq!(pages_of(&full), ["Fields", "Message", "Raw"]);
    }

    /// The title names the record, with the separators a person reads a line number by.
    #[test]
    fn the_title_names_the_record() {
        let view = view_of(&detail(vec![], "x", vec![]), "x", false);
        assert_eq!(title_of(&view), "Record 1,204,915 — Detail");
        assert_eq!(with_separators(0), "0");
        assert_eq!(with_separators(999), "999");
        assert_eq!(with_separators(1_000), "1,000");
    }
}
