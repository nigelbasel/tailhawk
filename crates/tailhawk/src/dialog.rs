//! §2.2's Keyboard map and Preferences as **standard modal dialogs** — the owner's direction,
//! 2026-08-24, replacing the overlay sheets that "dumped text on top of the window".
//!
//! ## Why the template is built by hand
//!
//! `DialogBoxIndirectParamW` takes a `DLGTEMPLATE` — a word-aligned run of 16-bit values that
//! resource compilers normally emit. Compiling an `.rc` would mean finding `rc.exe` on every
//! machine and both CI legs, which is exactly the cost `build.rs` refused for the version stamp.
//! The layout is small, documented, and — being pure arithmetic over a `Vec<u16>` — testable
//! word by word, which no `.rc` file is.
//!
//! ## The split
//!
//! [`template`], [`keymap_text`] and [`PrefsChoice`] are the pure half: bytes and strings from
//! inputs, no window anywhere, tested below. [`show_keymap`] and [`show_prefs`] are the view: they
//! hand the finished template to Windows and copy control state in and out, deciding nothing.

use windows::core::PCWSTR;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    DialogBoxIndirectParamW, EndDialog, GetDlgItemInt, GetWindowLongPtrW, SendDlgItemMessageW,
    SetDlgItemInt, SetDlgItemTextW, SetWindowLongPtrW, DLGTEMPLATE, WINDOW_LONG_PTR_INDEX,
    WM_COMMAND, WM_INITDIALOG,
};

use crate::keymap::KeymapSheet;
use crate::prefs;

/// The dialog-manager button ids every dialog shares.
pub const IDOK: u16 = 1;
pub const IDCANCEL: u16 = 2;

/// Control ids, shared across both dialogs — they are never up at once.
const ID_THEME: u16 = 100;
const ID_FACE: u16 = 101;
const ID_SIZE: u16 = 102;
const ID_MAP: u16 = 103;

/// `DWLP_USER` — the per-dialog slot the data pointer rides in. Both targets are 64-bit, so the
/// two slots before it are eight bytes each.
const DWLP_USER: i32 = 16;

const CB_ADDSTRING: u32 = 0x0143;
const CB_GETCURSEL: u32 = 0x0147;
const CB_SETCURSEL: u32 = 0x014E;
const EM_SETTABSTOPS: u32 = 0x00CB;

/// The predefined dialog control classes, by the ordinals `DLGTEMPLATE` names them with.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Class {
    Button = 0x0080,
    Edit = 0x0081,
    Static = 0x0082,
    ComboBox = 0x0085,
}

/// One control in a template: geometry in dialog units, and the style bits beyond
/// `WS_CHILD | WS_VISIBLE`, which every control gets.
pub struct Item {
    pub class: Class,
    pub text: String,
    pub id: u16,
    pub x: i16,
    pub y: i16,
    pub w: i16,
    pub h: i16,
    pub style: u32,
}

impl Item {
    fn new(class: Class, text: &str, id: u16, at: (i16, i16, i16, i16), style: u32) -> Self {
        Self {
            class,
            text: text.to_owned(),
            id,
            x: at.0,
            y: at.1,
            w: at.2,
            h: at.3,
            style,
        }
    }
}

fn push_u32(words: &mut Vec<u16>, v: u32) {
    words.push((v & 0xFFFF) as u16);
    words.push((v >> 16) as u16);
}

fn push_wsz(words: &mut Vec<u16>, s: &str) {
    words.extend(s.encode_utf16());
    words.push(0);
}

/// A whole `DLGTEMPLATE` as the words Windows reads: centred, modal-framed, captioned, in the
/// 8-point shell dialog font. Every item is aligned to a DWORD boundary, which is the one layout
/// rule a miscount breaks silently.
pub fn template(title: &str, w: i16, h: i16, items: &[Item]) -> Vec<u16> {
    const WS_POPUP: u32 = 0x8000_0000;
    const WS_CAPTION: u32 = 0x00C0_0000;
    const WS_SYSMENU: u32 = 0x0008_0000;
    const DS_MODALFRAME: u32 = 0x80;
    const DS_SETFONT: u32 = 0x40;
    const DS_CENTER: u32 = 0x0800;
    const WS_CHILD_VISIBLE: u32 = 0x5000_0000;

    let mut t = Vec::new();
    push_u32(
        &mut t,
        WS_POPUP | WS_CAPTION | WS_SYSMENU | DS_MODALFRAME | DS_SETFONT | DS_CENTER,
    );
    push_u32(&mut t, 0);
    t.push(items.len() as u16);
    t.push(0);
    t.push(0);
    t.push(w as u16);
    t.push(h as u16);
    t.push(0); // no menu
    t.push(0); // the standard dialog class
    push_wsz(&mut t, title);
    t.push(8); // point size, then the face DS_SETFONT promises
    push_wsz(&mut t, "MS Shell Dlg");
    for item in items {
        if t.len() % 2 == 1 {
            t.push(0);
        }
        push_u32(&mut t, WS_CHILD_VISIBLE | item.style);
        push_u32(&mut t, 0);
        t.push(item.x as u16);
        t.push(item.y as u16);
        t.push(item.w as u16);
        t.push(item.h as u16);
        t.push(item.id);
        t.push(0xFFFF);
        t.push(item.class as u16);
        push_wsz(&mut t, &item.text);
        t.push(0); // no creation data
    }
    t
}

/// The keyboard map flattened for a read-only edit control: one `keys<TAB>what` line per binding,
/// headings on their own lines, sections separated by a blank line, CRLF throughout because an
/// edit control shows `\n` alone as a box.
pub fn keymap_text(sheet: &KeymapSheet) -> String {
    let mut text = String::new();
    for (i, section) in sheet.sections.iter().enumerate() {
        if i > 0 {
            text.push_str("\r\n");
        }
        text.push_str(&section.heading);
        text.push_str("\r\n");
        for row in &section.rows {
            text.push_str(&row.keys);
            text.push('\t');
            text.push_str(&row.what);
            text.push_str("\r\n");
        }
    }
    text
}

/// What the Preferences dialog edits, in and out. Built by [`PrefsChoice::of`] from what is in
/// force; [`show_prefs`] copies it into the controls and, on OK, back out with `accepted` set.
#[derive(Clone)]
pub struct PrefsChoice {
    pub themes: Vec<String>,
    pub theme: usize,
    pub faces: Vec<String>,
    pub face: usize,
    pub size: u16,
    pub accepted: bool,
}

impl PrefsChoice {
    /// The current appearance as combo-box lists and selections. A face in use but not installed
    /// — saved on another machine — still has to be shown, so it joins the list at the top; the
    /// alternative is a dialog whose font box silently names some other font.
    pub fn of(theme_name: &str, face: &str, size: u16, installed: Vec<String>) -> Self {
        let themes: Vec<String> = prefs::THEMES.iter().map(|s| (*s).to_owned()).collect();
        let theme = themes.iter().position(|t| t == theme_name).unwrap_or(0);
        let mut faces = installed;
        let face = match faces.iter().position(|f| f == face) {
            Some(at) => at,
            None => {
                faces.insert(0, face.to_owned());
                0
            }
        };
        Self {
            themes,
            theme,
            faces,
            face,
            size,
            accepted: false,
        }
    }
}

fn wsz(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn combo_fill(hdlg: HWND, id: u16, entries: &[String], selected: usize) {
    for entry in entries {
        let text = wsz(entry);
        unsafe {
            SendDlgItemMessageW(
                hdlg,
                i32::from(id),
                CB_ADDSTRING,
                WPARAM(0),
                LPARAM(text.as_ptr() as isize),
            );
        }
    }
    unsafe {
        SendDlgItemMessageW(
            hdlg,
            i32::from(id),
            CB_SETCURSEL,
            WPARAM(selected),
            LPARAM(0),
        );
    }
}

fn combo_selected(hdlg: HWND, id: u16, len: usize) -> Option<usize> {
    let at =
        unsafe { SendDlgItemMessageW(hdlg, i32::from(id), CB_GETCURSEL, WPARAM(0), LPARAM(0)).0 };
    (at >= 0).then(|| (at as usize).min(len.saturating_sub(1)))
}

unsafe extern "system" fn prefs_proc(
    hdlg: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> isize {
    match msg {
        WM_INITDIALOG => {
            unsafe {
                SetWindowLongPtrW(hdlg, WINDOW_LONG_PTR_INDEX(DWLP_USER), lparam.0);
            }
            // SAFETY: `lparam` is the pointer `show_prefs` passed, to a `PrefsChoice` that
            // outlives the modal loop this proc runs inside.
            let data = unsafe { &mut *(lparam.0 as *mut PrefsChoice) };
            combo_fill(hdlg, ID_THEME, &data.themes, data.theme);
            combo_fill(hdlg, ID_FACE, &data.faces, data.face);
            unsafe {
                let _ = SetDlgItemInt(hdlg, i32::from(ID_SIZE), u32::from(data.size), false);
            }
            1
        }
        WM_COMMAND => {
            let id = (wparam.0 & 0xFFFF) as u16;
            match id {
                IDOK => {
                    // SAFETY: the pointer stashed at `WM_INITDIALOG`, same lifetime argument.
                    let data = unsafe {
                        &mut *(GetWindowLongPtrW(hdlg, WINDOW_LONG_PTR_INDEX(DWLP_USER))
                            as *mut PrefsChoice)
                    };
                    if let Some(at) = combo_selected(hdlg, ID_THEME, data.themes.len()) {
                        data.theme = at;
                    }
                    if let Some(at) = combo_selected(hdlg, ID_FACE, data.faces.len()) {
                        data.face = at;
                    }
                    // The translated flag tells an emptied field apart from a typed zero; a user
                    // who cleared the box to retype and hit OK keeps the size they had, rather
                    // than being handed the smallest font the clamp allows.
                    let mut typed = windows::Win32::Foundation::BOOL(0);
                    let size =
                        unsafe { GetDlgItemInt(hdlg, i32::from(ID_SIZE), Some(&mut typed), false) };
                    if typed.as_bool() {
                        data.size = size
                            .clamp(u32::from(prefs::MIN_SIZE), u32::from(prefs::MAX_SIZE))
                            as u16;
                    }
                    data.accepted = true;
                    unsafe {
                        let _ = EndDialog(hdlg, 1);
                    }
                    1
                }
                IDCANCEL => {
                    unsafe {
                        let _ = EndDialog(hdlg, 0);
                    }
                    1
                }
                _ => 0,
            }
        }
        _ => 0,
    }
}

unsafe extern "system" fn keymap_proc(
    hdlg: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> isize {
    match msg {
        WM_INITDIALOG => {
            // One stop, past the widest keystroke column, in dialog units.
            let stop: u32 = 78;
            unsafe {
                SendDlgItemMessageW(
                    hdlg,
                    i32::from(ID_MAP),
                    EM_SETTABSTOPS,
                    WPARAM(1),
                    LPARAM(std::ptr::addr_of!(stop) as isize),
                );
            }
            // SAFETY: `lparam` is the `&String` `show_keymap` passed, alive for the modal loop.
            let text = wsz(unsafe { &*(lparam.0 as *const String) });
            unsafe {
                let _ = SetDlgItemTextW(hdlg, i32::from(ID_MAP), PCWSTR(text.as_ptr()));
            }
            1
        }
        WM_COMMAND => {
            let id = (wparam.0 & 0xFFFF) as u16;
            if id == IDOK || id == IDCANCEL {
                unsafe {
                    let _ = EndDialog(hdlg, 0);
                }
                return 1;
            }
            0
        }
        _ => 0,
    }
}

const CBS_DROPDOWNLIST: u32 = 0x0003;
const WS_VSCROLL: u32 = 0x0020_0000;
const WS_TABSTOP: u32 = 0x0001_0000;
const WS_BORDER: u32 = 0x0080_0000;
const ES_NUMBER: u32 = 0x2000;
const ES_MULTILINE: u32 = 0x0004;
const ES_READONLY: u32 = 0x0800;
const BS_DEFPUSHBUTTON: u32 = 0x0001;

/// §2.2's Preferences as a modal dialog: theme and font in drop-down lists, the size in a numeric
/// field, OK and Cancel. Returns whether OK was chosen; `data` then carries the selections.
pub fn show_prefs(hwnd: HWND, data: &mut PrefsChoice) -> bool {
    let items = [
        Item::new(Class::Static, "&Theme:", 0xFFFF, (7, 9, 44, 8), 0),
        Item::new(
            Class::ComboBox,
            "",
            ID_THEME,
            (58, 7, 152, 60),
            CBS_DROPDOWNLIST | WS_VSCROLL | WS_TABSTOP,
        ),
        Item::new(Class::Static, "&Font:", 0xFFFF, (7, 27, 44, 8), 0),
        Item::new(
            Class::ComboBox,
            "",
            ID_FACE,
            (58, 25, 152, 90),
            CBS_DROPDOWNLIST | WS_VSCROLL | WS_TABSTOP,
        ),
        Item::new(Class::Static, "&Size:", 0xFFFF, (7, 45, 44, 8), 0),
        Item::new(
            Class::Edit,
            "",
            ID_SIZE,
            (58, 43, 32, 12),
            WS_BORDER | WS_TABSTOP | ES_NUMBER,
        ),
        Item::new(
            Class::Button,
            "OK",
            IDOK,
            (105, 65, 50, 14),
            WS_TABSTOP | BS_DEFPUSHBUTTON,
        ),
        Item::new(
            Class::Button,
            "Cancel",
            IDCANCEL,
            (160, 65, 50, 14),
            WS_TABSTOP,
        ),
    ];
    let t = template("Preferences", 217, 86, &items);
    unsafe {
        DialogBoxIndirectParamW(
            None,
            t.as_ptr() as *const DLGTEMPLATE,
            hwnd,
            Some(prefs_proc),
            LPARAM(data as *mut PrefsChoice as isize),
        )
    };
    data.accepted
}

/// §2.2's Keyboard map as a modal dialog: the generated map in a read-only, scrollable,
/// copyable edit control, and a Close button. The content arrives already flattened by
/// [`keymap_text`], which is where its shape is tested.
pub fn show_keymap(hwnd: HWND, text: &String) {
    let items = [
        Item::new(
            Class::Edit,
            "",
            ID_MAP,
            (7, 7, 266, 168),
            WS_BORDER | WS_VSCROLL | WS_TABSTOP | ES_MULTILINE | ES_READONLY,
        ),
        Item::new(
            Class::Button,
            "Close",
            IDOK,
            (230, 181, 50, 14),
            WS_TABSTOP | BS_DEFPUSHBUTTON,
        ),
    ];
    let t = template("Keyboard map", 287, 202, &items);
    unsafe {
        DialogBoxIndirectParamW(
            None,
            t.as_ptr() as *const DLGTEMPLATE,
            hwnd,
            Some(keymap_proc),
            LPARAM(text as *const String as isize),
        )
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keymap::{KeymapRow, KeymapSection};

    fn two_items() -> Vec<Item> {
        vec![
            Item::new(Class::Static, "&Theme:", 0xFFFF, (7, 9, 44, 8), 0),
            Item::new(Class::Button, "OK", IDOK, (105, 65, 50, 14), 1),
        ]
    }

    /// The header carries the item count where Windows reads it, and the title where the caption
    /// draws it — a miscount here is a dialog that silently shows fewer controls than it has.
    #[test]
    fn the_template_header_counts_its_items_and_names_the_font() {
        let t = template("Preferences", 217, 86, &two_items());
        assert_eq!(t[4], 2, "cdit is the fifth word");
        assert_eq!(t[7], 217_u16, "cx");
        assert_eq!(t[8], 86, "cy");
        let words: Vec<u16> = "Preferences".encode_utf16().collect();
        assert_eq!(&t[11..11 + words.len()], &words[..], "the caption");
        let face: Vec<u16> = "MS Shell Dlg".encode_utf16().collect();
        let at = (0..t.len() - face.len())
            .find(|&i| &t[i..i + face.len()] == &face[..])
            .expect("the DS_SETFONT face is present");
        assert_eq!(t[at - 1], 8, "eight point, the word before the face");
    }

    /// Every item begins on a DWORD boundary — the alignment rule whose violation Windows answers
    /// with a dialog that simply does not open.
    #[test]
    fn every_item_is_dword_aligned_and_carries_its_class_ordinal() {
        let items = two_items();
        let t = template("x", 100, 100, &items);
        // Walk the template the way Windows does: header, then aligned items.
        let mut at = 9; // style, exstyle, cdit, x, y, cx, cy
        at += 2; // no menu, standard class — one ordinal word each
        at += "x".encode_utf16().count() + 1; // caption
        at += 1; // point size
        at += "MS Shell Dlg".encode_utf16().count() + 1;
        for item in &items {
            if at % 2 == 1 {
                at += 1;
            }
            assert_eq!(at % 2, 0, "item starts on a DWORD boundary");
            let style = u32::from(t[at]) | (u32::from(t[at + 1]) << 16);
            assert_eq!(
                style & 0x5000_0000,
                0x5000_0000,
                "WS_CHILD | WS_VISIBLE on every control"
            );
            assert_eq!(t[at + 8], item.id);
            assert_eq!(t[at + 9], 0xFFFF, "ordinal class marker");
            assert_eq!(t[at + 10], item.class as u16);
            at += 11;
            at += item.text.encode_utf16().count() + 1;
            assert_eq!(t[at], 0, "no creation data");
            at += 1;
        }
        assert_eq!(at, t.len(), "the walk consumed exactly the template");
    }

    /// The flattening the keymap dialog shows: headings, tabbed rows, CRLF line ends.
    #[test]
    fn the_keymap_text_carries_every_binding_under_its_heading() {
        let sheet = KeymapSheet {
            title: "Keyboard map".into(),
            sections: vec![
                KeymapSection {
                    heading: "Moving".into(),
                    rows: vec![KeymapRow {
                        keys: "Ctrl+End".into(),
                        what: "End of document".into(),
                    }],
                },
                KeymapSection {
                    heading: "Finding".into(),
                    rows: vec![KeymapRow {
                        keys: "Ctrl+F".into(),
                        what: "Find".into(),
                    }],
                },
            ],
        };
        let text = keymap_text(&sheet);
        assert!(text.contains("Moving\r\n"));
        assert!(text.contains("Ctrl+End\tEnd of document\r\n"));
        assert!(text.contains("\r\nFinding\r\n"));
        assert!(text.contains("Ctrl+F\tFind\r\n"));
        assert!(
            !text.contains("\n\n"),
            "blank lines are CRLF too, or the edit control draws boxes"
        );
    }

    /// The dialog data: current selections found, and a face in use but not installed still shown.
    #[test]
    fn the_prefs_choice_keeps_an_uninstalled_face_visible() {
        let choice = PrefsChoice::of(
            "dark",
            "Consolas",
            16,
            vec!["Cascadia Mono".into(), "Consolas".into()],
        );
        assert_eq!(choice.themes[choice.theme], "dark");
        assert_eq!(choice.faces[choice.face], "Consolas");
        assert_eq!(choice.size, 16);
        assert!(!choice.accepted);

        let missing = PrefsChoice::of("system", "Fantasque", 12, vec!["Consolas".into()]);
        assert_eq!(
            missing.faces[missing.face], "Fantasque",
            "the face in use is shown even when this machine does not have it"
        );
        assert_eq!(missing.faces.len(), 2);
    }
}
