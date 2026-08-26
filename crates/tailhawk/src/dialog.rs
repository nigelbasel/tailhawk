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

use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::Controls::{
    InitCommonControlsEx, ICC_LISTVIEW_CLASSES, INITCOMMONCONTROLSEX, LIST_VIEW_ITEM_STATE_FLAGS,
    LVCF_TEXT, LVCF_WIDTH, LVCOLUMNW, LVIF_TEXT, LVITEMW, NMHDR,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateDialogIndirectParamW, DestroyWindow, DialogBoxIndirectParamW, EndDialog, GetDlgItem,
    GetDlgItemInt, GetDlgItemTextW, GetWindowLongPtrW, SendDlgItemMessageW, SendMessageW,
    SetDlgItemInt, SetDlgItemTextW, SetWindowLongPtrW, ShowWindow, DLGTEMPLATE, SW_SHOW,
    WINDOW_LONG_PTR_INDEX, WM_COMMAND, WM_DESTROY, WM_INITDIALOG, WM_NOTIFY,
};

use windows::Win32::UI::Input::KeyboardAndMouse::{EnableWindow, SetFocus};

use crate::keymap::KeymapSheet;
use crate::prefs;
use tailhawk_core::wizard::{Edge, Role, Test, Wizard};

/// The dialog-manager button ids every dialog shares.
pub const IDOK: u16 = 1;
pub const IDCANCEL: u16 = 2;

/// Control ids, shared across the dialogs — no two of these are up at once.
const ID_THEME: u16 = 100;
const ID_FACE: u16 = 101;
const ID_SIZE: u16 = 102;
const ID_MAP: u16 = 103;
const ID_F_INCLUDE: u16 = 120;
const ID_F_EXCLUDE: u16 = 121;
const ID_F_SCOPE: u16 = 122;
const ID_F_OP: u16 = 123;
const ID_F_VALUE: u16 = 124;
const ID_F_REGEX: u16 = 125;
const ID_F_CASE: u16 = 126;
const ID_F_EXPR: u16 = 127;
const ID_F_STATUS: u16 = 128;
/// The Filter dialog's grid, in dialog units. The label column is sized for the **longest** label
/// — `Expression:` — so that every field can start in one place; sizing it for the shortest is
/// what put the Expression box six units right of the Value box above it.
const F_LABEL_X: i16 = 7;
const F_LABEL_W: i16 = 40;
const F_FIELD_X: i16 = 50;
const F_RIGHT: i16 = 292;
const ID_FIND_WHAT: u16 = 110;
const ID_MATCH_CASE: u16 = 111;
const ID_REGEX: u16 = 112;
const ID_FIND_PREV: u16 = 113;
const ID_FIND_WHOLE: u16 = 114;
const ID_FIND_WRAP: u16 = 115;
const ID_FIND_STATUS: u16 = 116;
/// The Define Format dialog — §6.2 as a real dialog rather than a sheet over the grid.
const ID_W_EXAMPLE: u16 = 140;
const ID_W_FIELDS: u16 = 141;
const ID_W_ROLE: u16 = 142;
const ID_W_NAME: u16 = 143;
const ID_W_BOUNDS: u16 = 144;
const ID_W_SPLIT: u16 = 145;
const ID_W_ADD: u16 = 146;
const ID_W_MERGE: u16 = 147;
const ID_W_REMOVE: u16 = 148;
const ID_W_PATTERN: u16 = 149;
const ID_W_ERROR: u16 = 150;
const ID_W_SAVEAS: u16 = 151;
const ID_W_TEST: u16 = 152;
const ID_W_STATUS: u16 = 153;
const ID_W_PREVIEW: u16 = 154;
/// The Import Layout dialog — §6.3, the Define Format dialog's sibling over the same `Wizard`.
const ID_I_LAYOUT: u16 = 160;
const ID_I_RECOGNISED: u16 = 161;
const ID_I_FOUND: u16 = 162;
const ID_I_USE: u16 = 163;
/// The Define Format dialog's grid, in dialog units — as [`F_FIELD_X`] and friends are the
/// Filter dialog's.
const W_LEFT: i16 = 7;
const W_RIGHT: i16 = 413;
const W_BUTTON_X: i16 = 313;
const W_BUTTON_W: i16 = 100;
/// The most preview rows the dialog lists. `MAX_SAMPLES` is 200; a list view holds them all
/// without complaint, and the scrollbar is the honest way to say how many there are.
const W_PREVIEW_ROWS: usize = tailhawk_core::wizard::MAX_SAMPLES;

/// `DWLP_USER` — the per-dialog slot the data pointer rides in. Both targets are 64-bit, so the
/// two slots before it are eight bytes each.
const DWLP_USER: i32 = 16;

const CB_ADDSTRING: u32 = 0x0143;
const CB_GETCURSEL: u32 = 0x0147;
const CB_SETCURSEL: u32 = 0x014E;
const EM_SETTABSTOPS: u32 = 0x00CB;
const BM_GETCHECK: u32 = 0x00F0;
const BM_SETCHECK: u32 = 0x00F1;

/// The predefined dialog control classes, by the ordinals `DLGTEMPLATE` names them with.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Class {
    Button,
    Edit,
    Static,
    ComboBox,
    /// A window class by **name**. The four above are the pre-defined dialog classes and have
    /// ordinals; the common controls — a list view, say — have none, and a template names them.
    Named(&'static str),
}

impl Class {
    /// The pre-defined ordinal, or `None` for a class the template must spell out.
    fn ordinal(self) -> Option<u16> {
        match self {
            Class::Button => Some(0x0080),
            Class::Edit => Some(0x0081),
            Class::Static => Some(0x0082),
            Class::ComboBox => Some(0x0085),
            Class::Named(_) => None,
        }
    }
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
        match item.class {
            Class::Named(name) => push_wsz(&mut t, name),
            other => {
                t.push(0xFFFF);
                t.push(
                    other
                        .ordinal()
                        .expect("a class with no name has an ordinal"),
                );
            }
        }
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

/// The roles the Define Format dialog offers, in combo order — §6.2's `Roles` row as a list a
/// standard control can show. `Named` last, because it is the one that needs the Name box beside
/// it and the order should not make that a surprise.
pub const FORMAT_ROLES: &[(&str, Role)] = &[
    ("Timestamp", Role::Timestamp),
    ("Severity", Role::Severity),
    ("Message", Role::Message),
    ("Ignore", Role::Discard),
    ("Column…", Role::Named),
];

/// The label a role carries in the dialog, and in the field list's Role column.
pub fn role_label(role: Role) -> &'static str {
    FORMAT_ROLES
        .iter()
        .find(|(_, r)| *r == role)
        .map_or("Column…", |(label, _)| label)
}

/// One row of the Define Format dialog's field list.
pub struct FormatFieldRow {
    /// The span of the example this field covers, shown literally — a field the user cannot read
    /// is a field they cannot judge, and the example above is too wide to count characters in.
    pub text: String,
    pub role: String,
    /// The DSL token this field becomes: `<ts>`, `<level>`, `<message>`, `<_>`, or `<name>`.
    pub token: String,
}

/// §6.2's fields as the rows the list shows — **pure**, so the one mapping the whole dialog rests
/// on is testable without a window.
///
/// A wizard built from a pasted layout has no example and therefore no fields; it answers with an
/// empty list rather than a wrong one.
pub fn format_field_rows(wizard: &Wizard) -> Vec<FormatFieldRow> {
    let Some(example) = wizard.example() else {
        return Vec::new();
    };
    wizard
        .fields()
        .iter()
        .map(|f| FormatFieldRow {
            text: example
                .get(f.start..f.end)
                .unwrap_or_default()
                .replace('\t', "→"),
            role: role_label(f.role).to_owned(),
            token: f.token(),
        })
        .collect()
}

/// The preview grid as text: the tested column titles, then one row per sample — `None` where the
/// format did not match that line, which the dialog shows as a struck-through row rather than
/// silently skipping it.
pub fn format_preview(test: &Test, samples: &[String], most: usize) -> Vec<Option<Vec<String>>> {
    test.rows
        .iter()
        .enumerate()
        .take(most)
        .map(|(i, row)| {
            let line = samples.get(i)?;
            Some(
                row.as_ref()?
                    .iter()
                    .map(|span| {
                        span.as_ref()
                            .map_or(String::new(), |s| line[s.clone()].to_owned())
                    })
                    .collect(),
            )
        })
        .collect()
}

/// §6.3's paste box before anything is in it. Not a fault: reporting "nothing pasted" in the error
/// colour before the user has done anything reads as a failure they caused.
pub const IMPORT_HINT: &str =
    "paste a layout from your logging config — Serilog outputTemplate, NLog layout, log4net pattern";

/// What the Import Layout dialog prints after "Recognised as" — the language, the hint on an empty
/// box, or why the paste was not placed.
pub fn recognised_label(layout: &str) -> String {
    if layout.trim().is_empty() {
        return IMPORT_HINT.to_owned();
    }
    match tailhawk_core::wizard::recognise(layout) {
        Ok(language) => tailhawk_core::wizard::language_label(language).to_owned(),
        Err(why) => why,
    }
}

/// How many earlier findings came out of the same config file as `selected`.
///
/// [`Wizard::from_found`] numbers a second layout **within one file**, so it needs this and not
/// the list's own index: numbering by the index would call the only layout in the second file its
/// own second, and that number becomes the definition's name and the compiled format's id.
pub fn nth_in_file(found: &[tailhawk_core::template::Found], selected: usize) -> usize {
    let Some(chosen) = found.get(selected) else {
        return 0;
    };
    found[..selected]
        .iter()
        .filter(|f| f.source == chosen.source)
        .count()
}

/// One row of the Import Layout dialog's findings list.
pub struct ImportFoundRow {
    /// The config file's own name — the path is context, the file is the identity.
    pub file: String,
    pub language: String,
    pub layout: String,
}

/// §6.3's folder scan as the rows the list shows — the file, what language its layout is in, and
/// the layout itself, so the choice is made by reading rather than by guessing from a filename.
pub fn import_found_rows(found: &[tailhawk_core::template::Found]) -> Vec<ImportFoundRow> {
    found
        .iter()
        .map(|f| ImportFoundRow {
            file: f
                .source
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            language: tailhawk_core::wizard::language_label(f.language).to_owned(),
            layout: f.template.clone(),
        })
        .collect()
}

/// The byte offset into `s` that a Win32 edit control's caret position names.
///
/// **The two count different things.** `EM_GETSEL` answers in UTF-16 code units, because that is
/// what the control holds; [`Wizard`] takes byte offsets into UTF-8, and insists they land on a
/// character boundary. On an ASCII log line the two are the same number, which is exactly why this
/// is a function with a test rather than a cast — the first line with an em dash in it would
/// otherwise put a field boundary in the middle of a character and be refused with a message about
/// character boundaries that names nothing the user did.
///
/// A position past the end clamps to the end, as a caret at the end of the text does.
pub fn byte_at_utf16(s: &str, units: usize) -> usize {
    let mut seen = 0usize;
    for (at, ch) in s.char_indices() {
        if seen >= units {
            return at;
        }
        seen += ch.len_utf16();
    }
    s.len()
}

/// The line above the preview: why nothing compiled, or how much of the sample matched.
pub fn format_status(test: Option<&Test>) -> String {
    match test {
        None => "not tested yet — Test previews the next 200 lines".to_owned(),
        Some(t) => match (&t.error, t.rate()) {
            (Some(why), _) => why.clone(),
            (None, None) => "no sample lines to preview".to_owned(),
            (None, Some(rate)) => format!(
                "{} of {} matched ({:.0}%)",
                t.matched,
                t.rows.len(),
                rate * 100.0
            ),
        },
    }
}

/// The operators the Filter dialog offers over a column, in combo order. Each writes §7.2 text;
/// the first and last two write the function forms, the middle the comparison forms.
pub const FILTER_OPS: &[&str] = &[
    "contains",
    "equals",
    "does not equal",
    "less than",
    "at most",
    "greater than",
    "at least",
    "like",
    "starts with",
    "ends with",
];

/// One structured choice in the Filter dialog, composed into the §7.2 expression the model
/// actually runs — the dialog is a friendly editor over the grammar, never a second language,
/// so everything it writes can be hand-edited, saved, or passed as `--filter=`.
///
/// `column: None` is "Any column": the whole-record search, where regex and case have meaning.
/// Scoped to a column, the grammar's own forms carry the case rules (`like` and the functions are
/// case-insensitive by definition).
pub fn compose_filter(
    column: Option<&str>,
    op: usize,
    value: &str,
    regex: bool,
    match_case: bool,
) -> String {
    use tailhawk_core::filter::{Chip, Polarity, Predicate};
    let value = value.trim();
    let Some(column) = column else {
        if regex {
            return if match_case {
                format!("/{value}/")
            } else {
                format!("/{value}/i")
            };
        }
        if match_case {
            // The grammar's only case-sensitive form is a regex without the `i` flag.
            return format!("/{}/", tailhawk_core::search::Pattern::escape(value));
        }
        // Plain text — verbatim when the parser reads it as text, quoted when it would read as
        // an expression, because "timeout" and "level >= Warning" must both mean their letters.
        let as_typed = Chip::parse(value, Polarity::Include);
        return match as_typed {
            Ok(chip) if matches!(chip.predicate, Predicate::Text { .. }) => value.to_owned(),
            _ => format!("\"{}\"", value.replace('"', "")),
        };
    };
    // A column value: numbers and severity-looking names ride bare (severity banding needs the
    // bare name), anything else is quoted.
    let bare = value.parse::<f64>().is_ok()
        || matches!(
            value.to_ascii_lowercase().as_str(),
            "trace" | "debug" | "info" | "information" | "warn" | "warning" | "error" | "fatal"
        );
    let formed = if bare {
        value.to_owned()
    } else {
        format!("\"{}\"", value.replace('"', ""))
    };
    match op {
        0 => format!("contains({column}, {formed})"),
        1 => format!("{column} = {formed}"),
        2 => format!("{column} != {formed}"),
        3 => format!("{column} < {formed}"),
        4 => format!("{column} <= {formed}"),
        5 => format!("{column} > {formed}"),
        6 => format!("{column} >= {formed}"),
        7 => format!("{column} like {formed}"),
        8 => format!("startsWith({column}, {formed})"),
        _ => format!("endsWith({column}, {formed})"),
    }
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
const ES_AUTOHSCROLL: u32 = 0x0080;
const BS_DEFPUSHBUTTON: u32 = 0x0001;
const BS_AUTOCHECKBOX: u32 = 0x0003;
const EN_CHANGE: u32 = 0x0300;
const CBN_SELCHANGE: u32 = 1;
const EM_GETSEL: u32 = 0x00B0;

/// The list-view styles and messages the Define Format dialog uses. The `windows` crate types the
/// structures; these are the plain integers beside them.
const LVS_REPORT: u32 = 0x0001;
const LVS_SINGLESEL: u32 = 0x0004;
const LVS_SHOWSELALWAYS: u32 = 0x0008;
const LVS_EX_GRIDLINES: u32 = 0x0001;
const LVS_EX_FULLROWSELECT: u32 = 0x0020;
const LVM_FIRST: u32 = 0x1000;
const LVM_DELETEALLITEMS: u32 = LVM_FIRST + 9;
const LVM_GETNEXTITEM: u32 = LVM_FIRST + 12;
const LVM_DELETECOLUMN: u32 = LVM_FIRST + 28;
const LVM_SETITEMSTATE: u32 = LVM_FIRST + 43;
const LVM_SETEXTENDEDLISTVIEWSTYLE: u32 = LVM_FIRST + 54;
const LVM_INSERTITEMW: u32 = LVM_FIRST + 77;
const LVM_INSERTCOLUMNW: u32 = LVM_FIRST + 97;
const LVM_SETITEMTEXTW: u32 = LVM_FIRST + 116;
const LVNI_SELECTED: u32 = 0x0002;
const LVIS_FOCUSED: u32 = 0x0001;
const LVIS_SELECTED: u32 = 0x0002;
/// `LVN_FIRST - 1`, and `LVN_FIRST` is `-100` — so this is the wrapped `u32` Windows sends.
const LVN_ITEMCHANGED: u32 = (-101_i32) as u32;

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

/// What the Find dialog asked for, read at the moment Find Next or Find Previous was pressed.
pub struct FindRequest {
    pub query: String,
    pub match_case: bool,
    pub regex: bool,
    pub whole_word: bool,
    pub wrap: bool,
    pub forwards: bool,
}

/// What seeds the dialog when it opens: the last request, or — the manual `Default` — nothing,
/// with Wrap around on, because a search that silently stops at the bottom of a log being read
/// from the middle is the surprising one.
#[derive(Clone)]
pub struct FindSeed {
    pub query: String,
    pub match_case: bool,
    pub regex: bool,
    pub whole_word: bool,
    pub wrap: bool,
}

impl Default for FindSeed {
    fn default() -> Self {
        Self {
            query: String::new(),
            match_case: false,
            regex: false,
            whole_word: false,
            wrap: true,
        }
    }
}

fn read_find_request(hdlg: HWND, forwards: bool) -> FindRequest {
    let mut buf = [0u16; 1024];
    let len = unsafe { GetDlgItemTextW(hdlg, i32::from(ID_FIND_WHAT), &mut buf) } as usize;
    let query = String::from_utf16_lossy(&buf[..len.min(buf.len())]);
    let checked = |id: u16| unsafe {
        SendDlgItemMessageW(hdlg, i32::from(id), BM_GETCHECK, WPARAM(0), LPARAM(0)).0 == 1
    };
    FindRequest {
        query,
        match_case: checked(ID_MATCH_CASE),
        regex: checked(ID_REGEX),
        whole_word: checked(ID_FIND_WHOLE),
        wrap: checked(ID_FIND_WRAP),
        forwards,
    }
}

/// The dialog's live count line — `41 matches so far`, `no matches`, a refused pattern — pushed
/// by the shell as the streaming pass reports. Free text on a static control; the dialog decides
/// nothing about it.
pub fn set_find_status(hdlg: HWND, text: &str) {
    let text = wsz(text);
    unsafe {
        let _ = SetDlgItemTextW(hdlg, i32::from(ID_FIND_STATUS), PCWSTR(text.as_ptr()));
    }
}

unsafe extern "system" fn find_proc(
    hdlg: HWND,
    msg: u32,
    wparam: WPARAM,
    _lparam: LPARAM,
) -> isize {
    match msg {
        WM_INITDIALOG => 1,
        WM_COMMAND => {
            let id = (wparam.0 & 0xFFFF) as u16;
            match id {
                IDOK => {
                    crate::find_requested(hdlg, read_find_request(hdlg, true));
                    1
                }
                ID_FIND_PREV => {
                    crate::find_requested(hdlg, read_find_request(hdlg, false));
                    1
                }
                IDCANCEL => {
                    unsafe {
                        let _ = DestroyWindow(hdlg);
                    }
                    1
                }
                _ => 0,
            }
        }
        WM_DESTROY => {
            crate::find_dialog_closed(hdlg);
            0
        }
        _ => 0,
    }
}

/// A second `Ctrl+F` while the dialog is up: bring it forward with the query selected, ready to
/// be retyped — the standard dialog's own behaviour rather than stacking a twin.
pub fn focus_find(hdlg: HWND) {
    /// `CB_SETEDITSEL` — the combo's own select-a-range; `EM_SETSEL` does not reach the edit
    /// inside it. Start in the low word, end in the high; `0xFFFF` as the end selects to the end.
    const CB_SETEDITSEL: u32 = 0x0142;
    unsafe {
        if let Ok(combo) = GetDlgItem(hdlg, i32::from(ID_FIND_WHAT)) {
            let _ = SetFocus(combo);
        }
        SendDlgItemMessageW(
            hdlg,
            i32::from(ID_FIND_WHAT),
            CB_SETEDITSEL,
            WPARAM(0),
            LPARAM(0xFFFF_0000u32 as isize),
        );
    }
}

/// §2.1 as resettled: the classic Find dialog, **modeless** as the standard one is — the document
/// scrolls, tails and follows underneath it, and Esc or Cancel dismisses it. The owner's message
/// loop pumps it through `IsDialogMessageW`; pressing Enter is Find Next, the default button.
///
/// Seeded with the last search so reopening continues rather than restarts — `query` arrives in
/// the form the user typed, not the escaped form the engine was handed.
pub fn create_find_dialog(owner: HWND, seed: &FindSeed, history: &[String]) -> HWND {
    const CBS_DROPDOWN: u32 = 0x0002;
    const CBS_AUTOHSCROLL: u32 = 0x0040;
    let items = [
        Item::new(Class::Static, "Fi&nd what:", 0xFFFF, (7, 9, 40, 8), 0),
        // A drop-down combo rather than an edit: the history rides in it, which is how the
        // standard Find remembers what was searched.
        Item::new(
            Class::ComboBox,
            "",
            ID_FIND_WHAT,
            (50, 7, 150, 64),
            CBS_DROPDOWN | CBS_AUTOHSCROLL | WS_VSCROLL | WS_TABSTOP,
        ),
        Item::new(
            Class::Button,
            "Match &case",
            ID_MATCH_CASE,
            (50, 26, 90, 10),
            WS_TABSTOP | BS_AUTOCHECKBOX,
        ),
        Item::new(
            Class::Button,
            "Match &whole word only",
            ID_FIND_WHOLE,
            (50, 38, 100, 10),
            WS_TABSTOP | BS_AUTOCHECKBOX,
        ),
        Item::new(
            Class::Button,
            "Regular e&xpression",
            ID_REGEX,
            (50, 50, 90, 10),
            WS_TABSTOP | BS_AUTOCHECKBOX,
        ),
        Item::new(
            Class::Button,
            "W&rap around",
            ID_FIND_WRAP,
            (50, 62, 90, 10),
            WS_TABSTOP | BS_AUTOCHECKBOX,
        ),
        Item::new(Class::Static, "", ID_FIND_STATUS, (7, 80, 196, 8), 0),
        Item::new(
            Class::Button,
            "&Find Next",
            IDOK,
            (210, 7, 60, 14),
            WS_TABSTOP | BS_DEFPUSHBUTTON,
        ),
        Item::new(
            Class::Button,
            "Find &Previous",
            ID_FIND_PREV,
            (210, 25, 60, 14),
            WS_TABSTOP,
        ),
        Item::new(
            Class::Button,
            "Cancel",
            IDCANCEL,
            (210, 43, 60, 14),
            WS_TABSTOP,
        ),
    ];
    let t = template("Find", 277, 94, &items);
    let created = unsafe {
        CreateDialogIndirectParamW(
            None,
            t.as_ptr() as *const DLGTEMPLATE,
            owner,
            Some(find_proc),
            LPARAM(0),
        )
    };
    let Ok(hdlg) = created else {
        return HWND::default();
    };
    unsafe {
        for query in history {
            let text = wsz(query);
            SendDlgItemMessageW(
                hdlg,
                i32::from(ID_FIND_WHAT),
                CB_ADDSTRING,
                WPARAM(0),
                LPARAM(text.as_ptr() as isize),
            );
        }
        let text = wsz(&seed.query);
        let _ = SetDlgItemTextW(hdlg, i32::from(ID_FIND_WHAT), PCWSTR(text.as_ptr()));
        let check = |id: u16, on: bool| {
            SendDlgItemMessageW(
                hdlg,
                i32::from(id),
                BM_SETCHECK,
                WPARAM(usize::from(on)),
                LPARAM(0),
            );
        };
        check(ID_MATCH_CASE, seed.match_case);
        check(ID_REGEX, seed.regex);
        check(ID_FIND_WHOLE, seed.whole_word);
        check(ID_FIND_WRAP, seed.wrap);
        let _ = ShowWindow(hdlg, SW_SHOW);
    }
    // Focus lands in the box with the old query selected, so typing replaces it — the standard
    // dialog's own behaviour.
    focus_find(hdlg);
    hdlg
}

/// What the Filter dialog edits, in and out. `expression` seeds the box when editing an existing
/// filter and carries the accepted §7.2 text back; `manual` records that the box is authoritative
/// (an edit, or the user typed into it) until a structured control is touched again.
pub struct FilterEdit {
    pub columns: Vec<String>,
    pub include: bool,
    /// A column to open pre-scoped to — the header's "Filter on this column…" route.
    pub scope: Option<String>,
    pub expression: String,
    pub manual: bool,
    pub accepted: bool,
}

/// The validation line: the model's own answer to the expression as it stands, plus §7.2's
/// unknown-column warning checked against the live format's columns.
fn filter_status(expression: &str, include: bool, columns: &[String]) -> String {
    use tailhawk_core::filter::{Chip, Field, Polarity};
    let expression = expression.trim();
    if expression.is_empty() {
        return "Type a filter, or build one above.".to_owned();
    }
    let polarity = if include {
        Polarity::Include
    } else {
        Polarity::Exclude
    };
    match Chip::parse(expression, polarity) {
        Err(e) => format!("Not a valid filter: {e}"),
        Ok(chip) => {
            for field in chip.predicate.fields() {
                if let Field::Attribute(name) = field {
                    if !columns.iter().any(|c| c.eq_ignore_ascii_case(name)) {
                        return format!(
                            "Warning: this format has no column named \"{name}\" — an include \
                             with it matches nothing."
                        );
                    }
                }
            }
            "OK.".to_owned()
        }
    }
}

fn read_dlg_text(hdlg: HWND, id: u16) -> String {
    let mut buf = [0u16; 1024];
    let len = unsafe { GetDlgItemTextW(hdlg, i32::from(id), &mut buf) } as usize;
    String::from_utf16_lossy(&buf[..len.min(buf.len())])
}

struct FilterState {
    edit: *mut FilterEdit,
    busy: bool,
}

unsafe fn filter_recompose(hdlg: HWND, state: &mut FilterState) {
    let edit = unsafe { &mut *state.edit };
    let scope = unsafe {
        SendDlgItemMessageW(
            hdlg,
            i32::from(ID_F_SCOPE),
            CB_GETCURSEL,
            WPARAM(0),
            LPARAM(0),
        )
        .0
    };
    let op = unsafe {
        SendDlgItemMessageW(hdlg, i32::from(ID_F_OP), CB_GETCURSEL, WPARAM(0), LPARAM(0)).0
    };
    let checked = |id: u16| unsafe {
        SendDlgItemMessageW(hdlg, i32::from(id), BM_GETCHECK, WPARAM(0), LPARAM(0)).0 == 1
    };
    let value = read_dlg_text(hdlg, ID_F_VALUE);
    let column = if scope > 0 {
        edit.columns.get(scope as usize - 1).map(String::as_str)
    } else {
        None
    };
    let expression = compose_filter(
        column,
        op.max(0) as usize,
        &value,
        checked(ID_F_REGEX),
        checked(ID_F_CASE),
    );
    state.busy = true;
    let text = wsz(&expression);
    unsafe {
        let _ = SetDlgItemTextW(hdlg, i32::from(ID_F_EXPR), PCWSTR(text.as_ptr()));
    }
    state.busy = false;
    unsafe { filter_validate(hdlg, state) };
}

unsafe fn filter_validate(hdlg: HWND, state: &FilterState) {
    let edit = unsafe { &*state.edit };
    let checked = |id: u16| unsafe {
        SendDlgItemMessageW(hdlg, i32::from(id), BM_GETCHECK, WPARAM(0), LPARAM(0)).0 == 1
    };
    let status = filter_status(
        &read_dlg_text(hdlg, ID_F_EXPR),
        checked(ID_F_INCLUDE),
        &edit.columns,
    );
    let text = wsz(&status);
    unsafe {
        let _ = SetDlgItemTextW(hdlg, i32::from(ID_F_STATUS), PCWSTR(text.as_ptr()));
    }
}

unsafe extern "system" fn filter_proc(
    hdlg: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> isize {
    match msg {
        WM_INITDIALOG => {
            // The per-dialog state rides in a leaked box freed at WM_DESTROY; DWLP_USER carries
            // it, exactly as the other dialogs carry theirs.
            let edit = lparam.0 as *mut FilterEdit;
            let state = Box::into_raw(Box::new(FilterState { edit, busy: false }));
            unsafe {
                SetWindowLongPtrW(hdlg, WINDOW_LONG_PTR_INDEX(DWLP_USER), state as isize);
                let data = &*edit;
                let any = wsz("(any column)");
                SendDlgItemMessageW(
                    hdlg,
                    i32::from(ID_F_SCOPE),
                    CB_ADDSTRING,
                    WPARAM(0),
                    LPARAM(any.as_ptr() as isize),
                );
                for column in &data.columns {
                    let text = wsz(column);
                    SendDlgItemMessageW(
                        hdlg,
                        i32::from(ID_F_SCOPE),
                        CB_ADDSTRING,
                        WPARAM(0),
                        LPARAM(text.as_ptr() as isize),
                    );
                }
                SendDlgItemMessageW(
                    hdlg,
                    i32::from(ID_F_SCOPE),
                    CB_SETCURSEL,
                    WPARAM(0),
                    LPARAM(0),
                );
                for op in FILTER_OPS {
                    let text = wsz(op);
                    SendDlgItemMessageW(
                        hdlg,
                        i32::from(ID_F_OP),
                        CB_ADDSTRING,
                        WPARAM(0),
                        LPARAM(text.as_ptr() as isize),
                    );
                }
                SendDlgItemMessageW(hdlg, i32::from(ID_F_OP), CB_SETCURSEL, WPARAM(0), LPARAM(0));
                let radio = if data.include {
                    ID_F_INCLUDE
                } else {
                    ID_F_EXCLUDE
                };
                SendDlgItemMessageW(hdlg, i32::from(radio), BM_SETCHECK, WPARAM(1), LPARAM(0));
                if let Some(scope) = data.scope.as_deref() {
                    if let Some(at) = data
                        .columns
                        .iter()
                        .position(|c| c.eq_ignore_ascii_case(scope))
                    {
                        SendDlgItemMessageW(
                            hdlg,
                            i32::from(ID_F_SCOPE),
                            CB_SETCURSEL,
                            WPARAM(at + 1),
                            LPARAM(0),
                        );
                    }
                }
                if !data.expression.is_empty() {
                    let text = wsz(&data.expression);
                    let _ = SetDlgItemTextW(hdlg, i32::from(ID_F_EXPR), PCWSTR(text.as_ptr()));
                }
                filter_validate(hdlg, &*state);
                // Focus lands in the Value box — typing a filter is what the dialog is for, and
                // the default first-tabstop focus would land on the Include radio instead.
                if let Ok(value) = GetDlgItem(hdlg, i32::from(ID_F_VALUE)) {
                    let _ = SetFocus(value);
                }
            }
            0
        }
        WM_COMMAND => {
            let state = unsafe {
                (GetWindowLongPtrW(hdlg, WINDOW_LONG_PTR_INDEX(DWLP_USER)) as *mut FilterState)
                    .as_mut()
            };
            let Some(state) = state else {
                return 0;
            };
            let id = (wparam.0 & 0xFFFF) as u16;
            let code = ((wparam.0 >> 16) & 0xFFFF) as u32;
            match (id, code) {
                (IDOK, _) => {
                    let expression = read_dlg_text(hdlg, ID_F_EXPR);
                    let edit = unsafe { &mut *state.edit };
                    let include = unsafe {
                        SendDlgItemMessageW(
                            hdlg,
                            i32::from(ID_F_INCLUDE),
                            BM_GETCHECK,
                            WPARAM(0),
                            LPARAM(0),
                        )
                        .0 == 1
                    };
                    let polarity = if include {
                        tailhawk_core::filter::Polarity::Include
                    } else {
                        tailhawk_core::filter::Polarity::Exclude
                    };
                    if tailhawk_core::filter::Chip::parse(expression.trim(), polarity).is_err() {
                        // The status line already says why; OK on an invalid filter keeps the
                        // dialog up rather than swallowing the mistake.
                        unsafe { filter_validate(hdlg, state) };
                        return 1;
                    }
                    edit.expression = expression.trim().to_owned();
                    edit.include = include;
                    edit.accepted = true;
                    unsafe {
                        let _ = EndDialog(hdlg, 1);
                    }
                    1
                }
                (IDCANCEL, _) => {
                    unsafe {
                        let _ = EndDialog(hdlg, 0);
                    }
                    1
                }
                (ID_F_EXPR, EN_CHANGE) => {
                    if !state.busy {
                        unsafe { &mut *state.edit }.manual = true;
                        unsafe { filter_validate(hdlg, state) };
                    }
                    1
                }
                (ID_F_VALUE, EN_CHANGE) => {
                    unsafe { &mut *state.edit }.manual = false;
                    unsafe { filter_recompose(hdlg, state) };
                    1
                }
                (ID_F_SCOPE | ID_F_OP, CBN_SELCHANGE) => {
                    unsafe { &mut *state.edit }.manual = false;
                    unsafe { filter_recompose(hdlg, state) };
                    1
                }
                (ID_F_REGEX | ID_F_CASE | ID_F_INCLUDE | ID_F_EXCLUDE, 0) => {
                    if !unsafe { &*state.edit }.manual {
                        unsafe { filter_recompose(hdlg, state) };
                    } else {
                        unsafe { filter_validate(hdlg, state) };
                    }
                    1
                }
                _ => 0,
            }
        }
        WM_DESTROY => {
            let state = unsafe { GetWindowLongPtrW(hdlg, WINDOW_LONG_PTR_INDEX(DWLP_USER)) }
                as *mut FilterState;
            if !state.is_null() {
                drop(unsafe { Box::from_raw(state) });
            }
            0
        }
        _ => 0,
    }
}

/// The Add / Edit Filter dialog's controls and where they sit, in dialog units — pure, so the
/// one thing a template can get wrong and a test can see is testable.
///
/// **Every labelled field starts at [`F_FIELD_X`] and every full-width one ends at
/// [`F_RIGHT`].** A dialog is read down its left edge, and a control that steps sideways from the
/// one above it reads as a mistake even when nothing is wrong with it — which is precisely what
/// the Expression box did, sitting six units right of the Value box above it because its label is
/// the longest. The label column is sized for the longest label instead, and the fields keep one
/// line.
fn filter_dialog_items() -> Vec<Item> {
    const BS_AUTORADIOBUTTON: u32 = 0x0009;
    const WS_GROUP: u32 = 0x0002_0000;
    vec![
        Item::new(
            Class::Button,
            "&Include rows that match",
            ID_F_INCLUDE,
            (7, 7, 100, 10),
            WS_TABSTOP | WS_GROUP | BS_AUTORADIOBUTTON,
        ),
        Item::new(
            Class::Button,
            "E&xclude rows that match",
            ID_F_EXCLUDE,
            (117, 7, 100, 10),
            BS_AUTORADIOBUTTON,
        ),
        Item::new(
            Class::Static,
            "&Column:",
            0xFFFF,
            (F_LABEL_X, 27, F_LABEL_W, 8),
            0,
        ),
        Item::new(
            Class::ComboBox,
            "",
            ID_F_SCOPE,
            (F_FIELD_X, 25, 98, 90),
            CBS_DROPDOWNLIST | WS_VSCROLL | WS_TABSTOP | WS_GROUP,
        ),
        Item::new(Class::Static, "&Match:", 0xFFFF, (156, 27, 30, 8), 0),
        Item::new(
            Class::ComboBox,
            "",
            ID_F_OP,
            (188, 25, 104, 90),
            CBS_DROPDOWNLIST | WS_VSCROLL | WS_TABSTOP,
        ),
        Item::new(
            Class::Static,
            "&Value:",
            0xFFFF,
            (F_LABEL_X, 45, F_LABEL_W, 8),
            0,
        ),
        Item::new(
            Class::Edit,
            "",
            ID_F_VALUE,
            (F_FIELD_X, 43, F_RIGHT - F_FIELD_X, 12),
            WS_BORDER | WS_TABSTOP | ES_AUTOHSCROLL,
        ),
        Item::new(
            Class::Button,
            "&Regular expression",
            ID_F_REGEX,
            (F_FIELD_X, 61, 90, 10),
            WS_TABSTOP | BS_AUTOCHECKBOX,
        ),
        Item::new(
            Class::Button,
            "Match c&ase",
            ID_F_CASE,
            (146, 61, 70, 10),
            WS_TABSTOP | BS_AUTOCHECKBOX,
        ),
        Item::new(
            Class::Static,
            "&Expression:",
            0xFFFF,
            (F_LABEL_X, 79, F_LABEL_W, 8),
            0,
        ),
        Item::new(
            Class::Edit,
            "",
            ID_F_EXPR,
            (F_FIELD_X, 77, F_RIGHT - F_FIELD_X, 12),
            WS_BORDER | WS_TABSTOP | ES_AUTOHSCROLL,
        ),
        Item::new(
            Class::Static,
            "",
            ID_F_STATUS,
            (F_LABEL_X, 95, F_RIGHT - F_LABEL_X, 18),
            0,
        ),
        Item::new(
            Class::Button,
            "OK",
            IDOK,
            (182, 117, 50, 14),
            WS_TABSTOP | BS_DEFPUSHBUTTON,
        ),
        Item::new(
            Class::Button,
            "Cancel",
            IDCANCEL,
            (242, 117, 50, 14),
            WS_TABSTOP,
        ),
    ]
}

/// The Add / Edit Filter dialog — the powerful surface over §7.2: include/exclude, a column
/// scope with operators, regex and case for whole-record matches, and the expression itself
/// always visible and editable, validated live by the same parser that will run it.
pub fn show_filter_dialog(hwnd: HWND, data: &mut FilterEdit) -> bool {
    let title = if data.expression.is_empty() {
        "Add Filter"
    } else {
        "Edit Filter"
    };
    let t = template(title, 299, 138, &filter_dialog_items());
    unsafe {
        DialogBoxIndirectParamW(
            None,
            t.as_ptr() as *const DLGTEMPLATE,
            hwnd,
            Some(filter_proc),
            LPARAM(data as *mut FilterEdit as isize),
        )
    };
    data.accepted
}

/// The Define Format dialog's controls and where they sit, in dialog units — pure, like
/// [`filter_dialog_items`], and held to the same rule: one left edge, one right edge.
///
/// **The shape is §6.2's, not the overlay's.** The example line at the top, the fields under it as
/// a list with its verbs beside it, the pattern the fields compose into, then Test and what Test
/// found. What the overlay expressed as a strip of key hints — `←→ field`, `R role`, `Tab name` —
/// is here what it should have been: a list you click and buttons that say what they do.
fn format_dialog_items() -> Vec<Item> {
    const BS_GROUPBOX: u32 = 0x0007;
    let label =
        |text: &str, at: (i16, i16, i16, i16)| Item::new(Class::Static, text, 0xFFFF, at, 0);
    let button = |text: &str, id: u16, at: (i16, i16, i16, i16)| {
        Item::new(Class::Button, text, id, at, WS_TABSTOP)
    };
    let verb = |text: &str, id: u16, row: i16| {
        button(text, id, (W_BUTTON_X, 44 + row * 17, W_BUTTON_W, 14))
    };
    let list = |id: u16, at: (i16, i16, i16, i16)| {
        Item::new(
            Class::Named("SysListView32"),
            "",
            id,
            at,
            WS_BORDER | WS_TABSTOP | LVS_REPORT | LVS_SINGLESEL | LVS_SHOWSELALWAYS,
        )
    };
    vec![
        label(
            "&Example line — select in it to set a field's bounds:",
            (W_LEFT, 7, 300, 8),
        ),
        Item::new(
            Class::Edit,
            "",
            ID_W_EXAMPLE,
            (W_LEFT, 17, W_RIGHT - W_LEFT, 12),
            WS_BORDER | WS_TABSTOP | ES_AUTOHSCROLL | ES_READONLY,
        ),
        label("&Fields:", (W_LEFT, 34, 40, 8)),
        list(ID_W_FIELDS, (W_LEFT, 44, W_BUTTON_X - W_LEFT - 6, 72)),
        verb("Set &bounds from selection", ID_W_BOUNDS, 0),
        verb("S&plit at selection", ID_W_SPLIT, 1),
        verb("&Add from selection", ID_W_ADD, 2),
        verb("&Merge with next", ID_W_MERGE, 3),
        verb("&Remove", ID_W_REMOVE, 4),
        label("R&ole:", (W_LEFT, 124, 22, 8)),
        Item::new(
            Class::ComboBox,
            "",
            ID_W_ROLE,
            (32, 122, 80, 90),
            CBS_DROPDOWNLIST | WS_VSCROLL | WS_TABSTOP,
        ),
        label("&Name:", (120, 124, 24, 8)),
        Item::new(
            Class::Edit,
            "",
            ID_W_NAME,
            (146, 122, 100, 12),
            WS_BORDER | WS_TABSTOP | ES_AUTOHSCROLL,
        ),
        label("Pattern:", (W_LEFT, 142, 40, 8)),
        Item::new(
            Class::Edit,
            "",
            ID_W_PATTERN,
            (W_LEFT, 152, W_RIGHT - W_LEFT, 12),
            WS_BORDER | WS_TABSTOP | ES_AUTOHSCROLL | ES_READONLY,
        ),
        label("", (W_LEFT, 168, W_RIGHT - W_LEFT, 8)),
        Item::new(
            Class::Static,
            "",
            ID_W_ERROR,
            (W_LEFT, 168, W_RIGHT - W_LEFT, 8),
            0,
        ),
        label("&Save as:", (W_LEFT, 182, 34, 8)),
        Item::new(
            Class::Edit,
            "",
            ID_W_SAVEAS,
            (44, 180, 150, 12),
            WS_BORDER | WS_TABSTOP | ES_AUTOHSCROLL,
        ),
        button("&Test", ID_W_TEST, (203, 180, 50, 14)),
        Item::new(
            Class::Static,
            "",
            ID_W_STATUS,
            (259, 182, W_RIGHT - 259, 8),
            0,
        ),
        Item::new(
            Class::Button,
            "Pre&view",
            0xFFFF,
            (W_LEFT, 198, W_RIGHT - W_LEFT, 82),
            BS_GROUPBOX,
        ),
        list(ID_W_PREVIEW, (W_LEFT + 6, 210, W_RIGHT - W_LEFT - 12, 64)),
        Item::new(
            Class::Button,
            "Save",
            IDOK,
            (W_RIGHT - 110, 286, 50, 14),
            WS_TABSTOP | BS_DEFPUSHBUTTON,
        ),
        Item::new(
            Class::Button,
            "Cancel",
            IDCANCEL,
            (W_RIGHT - 50, 286, 50, 14),
            WS_TABSTOP,
        ),
    ]
}

/// §6.2's wizard as a **standard modal dialog** — the owner's direction, 2026-08-25: "Format ▸
/// define from a line is yet another terrible UI. Needs a proper dialog."
///
/// It edits `wizard` in place and answers whether Save was pressed. Everything it decides, it
/// decides by calling the model; everything it shows, it shows through [`format_field_rows`],
/// [`format_preview`] and [`format_status`], which are pure and tested.
pub fn show_format_dialog(hwnd: HWND, wizard: &mut Wizard) -> bool {
    // The list views are common controls, and a template that names a class nobody registered
    // opens as a dialog with a hole in it.
    let icc = INITCOMMONCONTROLSEX {
        dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
        dwICC: ICC_LISTVIEW_CLASSES,
    };
    unsafe {
        let _ = InitCommonControlsEx(&icc);
    }
    let mut state = FormatState {
        wizard,
        accepted: false,
    };
    let t = template("Define Format", 420, 307, &format_dialog_items());
    unsafe {
        DialogBoxIndirectParamW(
            None,
            t.as_ptr() as *const DLGTEMPLATE,
            hwnd,
            Some(format_proc),
            LPARAM(&mut state as *mut FormatState as isize),
        )
    };
    state.accepted
}

/// What [`format_proc`] works on for the life of the dialog.
struct FormatState<'a> {
    wizard: &'a mut Wizard,
    accepted: bool,
}

thread_local! {
    /// True while the Define Format dialog is writing its own controls.
    ///
    /// **An edit control cannot tell a typist from a `SetDlgItemTextW`** — both arrive as
    /// `EN_CHANGE` — and the dialog fills the Name box every time the selection moves. Without
    /// this, showing a Timestamp field put its token `ts` into the Name box, which came straight
    /// back as "set this field's name to ts" and was refused: the dialog opened reporting a fault
    /// it had caused itself, against a pattern that was perfectly good.
    ///
    /// **It lives here rather than in [`FormatState`], and that is the second version of this
    /// fix.** The flag has to be written before an FFI call and read inside the message that call
    /// dispatches — but the writer holds `&mut FormatState` and the reader takes its own from
    /// `DWLP_USER`, so the two alias, and a `&mut` promises the compiler that nothing else can
    /// observe what it writes. The store was simply gone from the optimised build: the guard read
    /// `false` every time and the dialog still opened accusing itself. A `Cell` in thread-local
    /// storage makes no such promise, and the dialog is modal on one thread by construction.
    static FORMAT_QUIET: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Runs `write`, with every `EN_CHANGE` and `CBN_SELCHANGE` it provokes marked as the dialog's own.
fn format_quietly<R>(write: impl FnOnce() -> R) -> R {
    FORMAT_QUIET.with(|q| q.set(true));
    let out = write();
    FORMAT_QUIET.with(|q| q.set(false));
    out
}

/// Adds one column to a report-view list.
fn lv_column(list: HWND, at: i32, title: &str, width: i32) {
    let mut text = wsz(title);
    let mut col = LVCOLUMNW {
        mask: LVCF_TEXT | LVCF_WIDTH,
        cx: width,
        pszText: PWSTR(text.as_mut_ptr()),
        ..Default::default()
    };
    unsafe {
        SendMessageW(
            list,
            LVM_INSERTCOLUMNW,
            WPARAM(at as usize),
            LPARAM(&mut col as *mut LVCOLUMNW as isize),
        );
    }
}

/// Clears a report-view list's rows **and** its columns, so a second Test does not inherit the
/// first's headings.
fn lv_reset(list: HWND) {
    unsafe {
        SendMessageW(list, LVM_DELETEALLITEMS, WPARAM(0), LPARAM(0));
        while SendMessageW(list, LVM_DELETECOLUMN, WPARAM(0), LPARAM(0)).0 != 0 {}
    }
}

/// Appends one row of text to a report-view list.
fn lv_row(list: HWND, at: i32, cells: &[String]) {
    let Some((first, rest)) = cells.split_first() else {
        return;
    };
    let mut text = wsz(first);
    let mut item = LVITEMW {
        mask: LVIF_TEXT,
        iItem: at,
        pszText: PWSTR(text.as_mut_ptr()),
        ..Default::default()
    };
    unsafe {
        SendMessageW(
            list,
            LVM_INSERTITEMW,
            WPARAM(0),
            LPARAM(&mut item as *mut LVITEMW as isize),
        );
    }
    for (i, cell) in rest.iter().enumerate() {
        let mut text = wsz(cell);
        let mut sub = LVITEMW {
            mask: LVIF_TEXT,
            iItem: at,
            iSubItem: i as i32 + 1,
            pszText: PWSTR(text.as_mut_ptr()),
            ..Default::default()
        };
        unsafe {
            SendMessageW(
                list,
                LVM_SETITEMTEXTW,
                WPARAM(at as usize),
                LPARAM(&mut sub as *mut LVITEMW as isize),
            );
        }
    }
}

/// The example edit's selection, as byte offsets into the example — the one gesture the dialog
/// replaces the overlay's invisible drag handles with.
///
/// `None` when nothing is selected: a caret is a point, and every verb that uses this wants a
/// span. Callers say so rather than guessing at one.
fn format_selection(hdlg: HWND, example: &str) -> Option<std::ops::Range<usize>> {
    let mut start = 0u32;
    let mut end = 0u32;
    unsafe {
        SendDlgItemMessageW(
            hdlg,
            i32::from(ID_W_EXAMPLE),
            EM_GETSEL,
            WPARAM(&mut start as *mut u32 as usize),
            LPARAM(&mut end as *mut u32 as isize),
        );
    }
    let from = byte_at_utf16(example, start as usize);
    let to = byte_at_utf16(example, end as usize);
    (from < to).then_some(from..to)
}

/// Puts the whole of `wizard` back on screen — fields, pattern, fault, and the readout — after
/// anything at all changes it. **One refresh, called from every verb**, because the alternative is
/// a dialog where four of five buttons update the pattern and the fifth is the one that ships.
fn format_refresh(hdlg: HWND, state: &mut FormatState, keep: Option<usize>) {
    let Ok(fields) = (unsafe { GetDlgItem(hdlg, i32::from(ID_W_FIELDS)) }) else {
        return;
    };
    let rows = format_field_rows(state.wizard);
    lv_reset(fields);
    lv_column(fields, 0, "Text", 150);
    lv_column(fields, 1, "Role", 70);
    lv_column(fields, 2, "Becomes", 80);
    for (i, row) in rows.iter().enumerate() {
        lv_row(
            fields,
            i as i32,
            &[row.text.clone(), row.role.clone(), row.token.clone()],
        );
    }
    if let Some(at) = keep.filter(|&a| a < rows.len()) {
        lv_select(fields, at);
    }
    let pattern = wsz(&state.wizard.template());
    unsafe {
        let _ = SetDlgItemTextW(hdlg, i32::from(ID_W_PATTERN), PCWSTR(pattern.as_ptr()));
    }
    let error = wsz(&state.wizard.error().unwrap_or_default());
    unsafe {
        let _ = SetDlgItemTextW(hdlg, i32::from(ID_W_ERROR), PCWSTR(error.as_ptr()));
    }
    let status = wsz(&format_status(state.wizard.last_test()));
    unsafe {
        let _ = SetDlgItemTextW(hdlg, i32::from(ID_W_STATUS), PCWSTR(status.as_ptr()));
    }
    format_show_role(hdlg, state, keep);
}

/// Points the Role combo and Name box at the selected field, and greys them when nothing is
/// selected — the §1.1 rule the menus keep, applied to the two controls that have a subject.
fn format_show_role(hdlg: HWND, state: &mut FormatState, at: Option<usize>) {
    let field = at.and_then(|i| state.wizard.fields().get(i));
    let on = field.is_some();
    for id in [
        ID_W_ROLE,
        ID_W_NAME,
        ID_W_BOUNDS,
        ID_W_SPLIT,
        ID_W_MERGE,
        ID_W_REMOVE,
    ] {
        if let Ok(control) = unsafe { GetDlgItem(hdlg, i32::from(id)) } {
            unsafe {
                let _ = EnableWindow(control, on);
            }
        }
    }
    let Some(field) = field else {
        return;
    };
    let chosen = FORMAT_ROLES
        .iter()
        .position(|(_, r)| *r == field.role)
        .unwrap_or(0);
    let name = wsz(&field.name);
    format_quietly(|| unsafe {
        SendDlgItemMessageW(
            hdlg,
            i32::from(ID_W_ROLE),
            CB_SETCURSEL,
            WPARAM(chosen),
            LPARAM(0),
        );
        let _ = SetDlgItemTextW(hdlg, i32::from(ID_W_NAME), PCWSTR(name.as_ptr()));
    });
}

/// Reports a refusal from the model where the user is looking. The wizard's own errors are
/// sentences — "that crosses the field before it" — and they belong on the fault line, not in a
/// message box that has to be dismissed before the next attempt.
fn format_say(hdlg: HWND, why: &str) {
    let text = wsz(why);
    unsafe {
        let _ = SetDlgItemTextW(hdlg, i32::from(ID_W_ERROR), PCWSTR(text.as_ptr()));
    }
}

/// Runs §6.2's **Test** and fills the preview from what came back.
fn format_test(hdlg: HWND, state: &mut FormatState) {
    let (columns, rows) = {
        let test = state.wizard.test().clone();
        let samples = state.wizard.samples().to_vec();
        (
            test.columns.clone(),
            format_preview(&test, &samples, W_PREVIEW_ROWS),
        )
    };
    let Ok(preview) = (unsafe { GetDlgItem(hdlg, i32::from(ID_W_PREVIEW)) }) else {
        return;
    };
    lv_reset(preview);
    if columns.is_empty() {
        lv_column(preview, 0, "Preview", 380);
    } else {
        for (i, title) in columns.iter().enumerate() {
            lv_column(preview, i as i32, title, 100);
        }
    }
    for (i, row) in rows.iter().enumerate() {
        // An unmatched sample is shown as such rather than dropped: §1.1's "never silently drop",
        // and the row the user most wants to see when the rate is not 100%.
        let cells = row.clone().unwrap_or_else(|| {
            let mut cells = vec!["— did not match —".to_owned()];
            cells.extend(std::iter::repeat_n(
                String::new(),
                columns.len().saturating_sub(1),
            ));
            cells
        });
        lv_row(preview, i as i32, &cells);
    }
    let status = wsz(&format_status(state.wizard.last_test()));
    unsafe {
        let _ = SetDlgItemTextW(hdlg, i32::from(ID_W_STATUS), PCWSTR(status.as_ptr()));
    }
}

/// The selected row of a report-view list, or `None`.
fn lv_selected(list: HWND) -> Option<usize> {
    let found = unsafe {
        SendMessageW(
            list,
            LVM_GETNEXTITEM,
            WPARAM(usize::MAX),
            LPARAM(LVNI_SELECTED as isize),
        )
    };
    usize::try_from(found.0).ok()
}

/// Selects row `at`, so an edit that reorders the fields leaves the pointer somewhere sensible.
fn lv_select(list: HWND, at: usize) {
    let item = LVITEMW {
        state: LIST_VIEW_ITEM_STATE_FLAGS(LVIS_SELECTED | LVIS_FOCUSED),
        stateMask: LIST_VIEW_ITEM_STATE_FLAGS(LVIS_SELECTED | LVIS_FOCUSED),
        ..Default::default()
    };
    unsafe {
        SendMessageW(
            list,
            LVM_SETITEMSTATE,
            WPARAM(at),
            LPARAM(&item as *const LVITEMW as isize),
        );
    }
}

/// The Import Layout dialog's controls — §6.3 as [`format_dialog_items`]'s sibling, and shaped
/// like it on purpose: the same pattern box, fault line, Save as, Test and preview in the same
/// places, so the two doors to one artefact do not have to be learned twice.
///
/// Where §6.2 has an example line and a field list, §6.3 has a **paste box** and a list of what
/// the folder scan found — because that is the whole difference between them.
fn import_dialog_items() -> Vec<Item> {
    const BS_GROUPBOX: u32 = 0x0007;
    const ES_WANTRETURN: u32 = 0x1000;
    let label =
        |text: &str, at: (i16, i16, i16, i16)| Item::new(Class::Static, text, 0xFFFF, at, 0);
    vec![
        label(
            "&Layout — paste the one from your logging config:",
            (W_LEFT, 7, 300, 8),
        ),
        Item::new(
            Class::Edit,
            "",
            ID_I_LAYOUT,
            (W_LEFT, 17, W_RIGHT - W_LEFT, 30),
            WS_BORDER | WS_TABSTOP | WS_VSCROLL | ES_MULTILINE | ES_WANTRETURN,
        ),
        label("Recognised as:", (W_LEFT, 53, 56, 8)),
        Item::new(
            Class::Static,
            "",
            ID_I_RECOGNISED,
            (66, 53, W_RIGHT - 66, 8),
            0,
        ),
        label("F&ound beside this log:", (W_LEFT, 68, 100, 8)),
        Item::new(
            Class::Named("SysListView32"),
            "",
            ID_I_FOUND,
            (W_LEFT, 78, W_BUTTON_X - W_LEFT - 6, 62),
            WS_BORDER | WS_TABSTOP | LVS_REPORT | LVS_SINGLESEL | LVS_SHOWSELALWAYS,
        ),
        Item::new(
            Class::Button,
            "&Use this one",
            ID_I_USE,
            (W_BUTTON_X, 78, W_BUTTON_W, 14),
            WS_TABSTOP,
        ),
        label("Pattern:", (W_LEFT, 146, 40, 8)),
        Item::new(
            Class::Edit,
            "",
            ID_W_PATTERN,
            (W_LEFT, 156, W_RIGHT - W_LEFT, 12),
            WS_BORDER | WS_TABSTOP | ES_AUTOHSCROLL | ES_READONLY,
        ),
        Item::new(
            Class::Static,
            "",
            ID_W_ERROR,
            (W_LEFT, 172, W_RIGHT - W_LEFT, 8),
            0,
        ),
        label("&Save as:", (W_LEFT, 186, 34, 8)),
        Item::new(
            Class::Edit,
            "",
            ID_W_SAVEAS,
            (44, 184, 150, 12),
            WS_BORDER | WS_TABSTOP | ES_AUTOHSCROLL,
        ),
        Item::new(
            Class::Button,
            "&Test",
            ID_W_TEST,
            (203, 184, 50, 14),
            WS_TABSTOP,
        ),
        Item::new(
            Class::Static,
            "",
            ID_W_STATUS,
            (259, 186, W_RIGHT - 259, 8),
            0,
        ),
        Item::new(
            Class::Button,
            "Pre&view",
            0xFFFF,
            (W_LEFT, 202, W_RIGHT - W_LEFT, 82),
            BS_GROUPBOX,
        ),
        Item::new(
            Class::Named("SysListView32"),
            "",
            ID_W_PREVIEW,
            (W_LEFT + 6, 214, W_RIGHT - W_LEFT - 12, 64),
            WS_BORDER | WS_TABSTOP | LVS_REPORT | LVS_SINGLESEL | LVS_SHOWSELALWAYS,
        ),
        Item::new(
            Class::Button,
            "Save",
            IDOK,
            (W_RIGHT - 110, 290, 50, 14),
            WS_TABSTOP | BS_DEFPUSHBUTTON,
        ),
        Item::new(
            Class::Button,
            "Cancel",
            IDCANCEL,
            (W_RIGHT - 50, 290, 50, 14),
            WS_TABSTOP,
        ),
    ]
}

/// §6.3's import as a **standard modal dialog**, for the reason §6.2's is one: it was the same
/// drawn sheet, and the owner has already said what that is.
///
/// `found` is `template::scan`'s answer, listed under the paste box rather than behind a second
/// command — taking one is a shortcut *into* the box, not a separate path, so whatever is imported
/// went through the same door and can be edited before it is saved.
pub fn show_import_dialog(
    hwnd: HWND,
    wizard: &mut Wizard,
    found: &[tailhawk_core::template::Found],
) -> bool {
    let icc = INITCOMMONCONTROLSEX {
        dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
        dwICC: ICC_LISTVIEW_CLASSES,
    };
    unsafe {
        let _ = InitCommonControlsEx(&icc);
    }
    let mut state = ImportState {
        wizard,
        found: found.to_vec(),
        accepted: false,
    };
    let t = template("Import Layout", 420, 311, &import_dialog_items());
    unsafe {
        DialogBoxIndirectParamW(
            None,
            t.as_ptr() as *const DLGTEMPLATE,
            hwnd,
            Some(import_proc),
            LPARAM(&mut state as *mut ImportState as isize),
        )
    };
    state.accepted
}

/// What [`import_proc`] works on for the life of the dialog.
struct ImportState<'a> {
    wizard: &'a mut Wizard,
    found: Vec<tailhawk_core::template::Found>,
    accepted: bool,
}

/// Puts the layout's consequences back on screen: what it was recognised as, the pattern, and the
/// fault. The paste box itself is left alone — it is where the caret is.
fn import_refresh(hdlg: HWND, state: &ImportState) {
    let layout = read_dlg_text(hdlg, ID_I_LAYOUT);
    let recognised = wsz(&recognised_label(&layout));
    let pattern = wsz(&state.wizard.template());
    // An empty box has not failed to compile; it has not been asked to.
    let error = wsz(&if layout.trim().is_empty() {
        String::new()
    } else {
        state.wizard.error().unwrap_or_default()
    });
    let status = wsz(&format_status(state.wizard.last_test()));
    unsafe {
        let _ = SetDlgItemTextW(
            hdlg,
            i32::from(ID_I_RECOGNISED),
            PCWSTR(recognised.as_ptr()),
        );
        let _ = SetDlgItemTextW(hdlg, i32::from(ID_W_PATTERN), PCWSTR(pattern.as_ptr()));
        let _ = SetDlgItemTextW(hdlg, i32::from(ID_W_ERROR), PCWSTR(error.as_ptr()));
        let _ = SetDlgItemTextW(hdlg, i32::from(ID_W_STATUS), PCWSTR(status.as_ptr()));
    }
}

/// The Import Layout dialog's proc.
unsafe extern "system" fn import_proc(
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
            let state = unsafe { &mut *(lparam.0 as *mut ImportState) };
            for id in [ID_I_FOUND, ID_W_PREVIEW] {
                if let Ok(list) = unsafe { GetDlgItem(hdlg, i32::from(id)) } {
                    unsafe {
                        SendMessageW(
                            list,
                            LVM_SETEXTENDEDLISTVIEWSTYLE,
                            WPARAM(0),
                            LPARAM((LVS_EX_FULLROWSELECT | LVS_EX_GRIDLINES) as isize),
                        );
                    }
                }
            }
            if let Ok(list) = unsafe { GetDlgItem(hdlg, i32::from(ID_I_FOUND)) } {
                lv_reset(list);
                lv_column(list, 0, "File", 110);
                lv_column(list, 1, "Language", 120);
                lv_column(list, 2, "Layout", 240);
                let rows = import_found_rows(&state.found);
                for (i, row) in rows.iter().enumerate() {
                    lv_row(
                        list,
                        i as i32,
                        &[row.file.clone(), row.language.clone(), row.layout.clone()],
                    );
                }
                if !rows.is_empty() {
                    lv_select(list, 0);
                }
            }
            let seed = wsz(match state.wizard.source() {
                tailhawk_core::wizard::Source::Layout { template, .. } => template.as_str(),
                tailhawk_core::wizard::Source::Example { .. } => "",
            });
            let name = wsz(&state.wizard.name);
            format_quietly(|| unsafe {
                let _ = SetDlgItemTextW(hdlg, i32::from(ID_I_LAYOUT), PCWSTR(seed.as_ptr()));
                let _ = SetDlgItemTextW(hdlg, i32::from(ID_W_SAVEAS), PCWSTR(name.as_ptr()));
            });
            import_refresh(hdlg, state);
            // §6.3 opens on the paste box, and **returning 0 without setting focus leaves it
            // nowhere** — which is not merely untidy: the dialog then answers no mnemonic at all,
            // so `Alt+U` for "Use this one" did nothing until this line existed.
            if let Ok(box_) = unsafe { GetDlgItem(hdlg, i32::from(ID_I_LAYOUT)) } {
                unsafe {
                    let _ = SetFocus(box_);
                }
            }
            0
        }
        WM_COMMAND => {
            let state = unsafe {
                (GetWindowLongPtrW(hdlg, WINDOW_LONG_PTR_INDEX(DWLP_USER)) as *mut ImportState)
                    .as_mut()
            };
            let Some(state) = state else {
                return 0;
            };
            let id = (wparam.0 & 0xFFFF) as u16;
            let code = ((wparam.0 >> 16) & 0xFFFF) as u32;
            if FORMAT_QUIET.with(|q| q.get()) && matches!(code, EN_CHANGE | CBN_SELCHANGE) {
                return 0;
            }
            match (id, code) {
                (IDOK, _) => {
                    let name = read_dlg_text(hdlg, ID_W_SAVEAS);
                    if name.trim().is_empty() {
                        format_say(hdlg, "name the format before saving it");
                    } else if read_dlg_text(hdlg, ID_I_LAYOUT).trim().is_empty() {
                        format_say(hdlg, "paste a layout first");
                    } else if let Some(why) = state.wizard.error() {
                        format_say(hdlg, &why);
                    } else {
                        state.wizard.name = name;
                        state.accepted = true;
                        unsafe {
                            let _ = EndDialog(hdlg, 1);
                        }
                    }
                }
                (IDCANCEL, _) => unsafe {
                    let _ = EndDialog(hdlg, 0);
                },
                (ID_I_LAYOUT, EN_CHANGE) => {
                    let text = read_dlg_text(hdlg, ID_I_LAYOUT);
                    // A refusal keeps the model as it was and is *said* — the box keeps what was
                    // typed while the model keeps what it will compile, and the two disagreeing
                    // silently is how a save writes something the box was not showing.
                    match state.wizard.repaste(&text) {
                        Err(why) => format_say(hdlg, &why),
                        Ok(()) => import_refresh(hdlg, state),
                    }
                }
                (ID_I_USE, _) => {
                    let Ok(list) = (unsafe { GetDlgItem(hdlg, i32::from(ID_I_FOUND)) }) else {
                        return 0;
                    };
                    match lv_selected(list).and_then(|at| state.found.get(at).map(|f| (at, f))) {
                        None => format_say(hdlg, "choose one of the layouts found first"),
                        Some((at, f)) => {
                            // Numbered **within its own config file**, not by the list's index —
                            // see [`nth_in_file`]. The number becomes the definition's name.
                            let mut taken = Wizard::from_found(f, nth_in_file(&state.found, at));
                            // **The list picks the layout, not the name.** Whatever the user has
                            // already typed into Save as — and the glob they chose — survives
                            // browsing the findings; silently renaming a definition because
                            // someone looked at a list is not what a list is for.
                            let chosen = read_dlg_text(hdlg, ID_W_SAVEAS);
                            if !chosen.trim().is_empty() {
                                taken.name = chosen;
                            }
                            taken.glob = state.wizard.glob.clone();
                            taken.set_samples(state.wizard.samples().to_vec());
                            let layout = wsz(&taken.template());
                            let name = wsz(&taken.name);
                            *state.wizard = taken;
                            format_quietly(|| unsafe {
                                let _ = SetDlgItemTextW(
                                    hdlg,
                                    i32::from(ID_I_LAYOUT),
                                    PCWSTR(layout.as_ptr()),
                                );
                                let _ = SetDlgItemTextW(
                                    hdlg,
                                    i32::from(ID_W_SAVEAS),
                                    PCWSTR(name.as_ptr()),
                                );
                            });
                            import_refresh(hdlg, state);
                        }
                    }
                }
                (ID_W_TEST, _) => {
                    let mut borrowed = FormatState {
                        wizard: state.wizard,
                        accepted: false,
                    };
                    format_test(hdlg, &mut borrowed);
                }
                _ => return 0,
            }
            1
        }
        WM_DESTROY => {
            unsafe {
                SetWindowLongPtrW(hdlg, WINDOW_LONG_PTR_INDEX(DWLP_USER), 0);
            }
            0
        }
        _ => 0,
    }
}

/// The Define Format dialog's proc. Every arm does the same three things: ask the model, report a
/// refusal if it refused, refresh. Nothing here decides what a field may be — that is
/// [`tailhawk_core::wizard`]'s, and it already says so in sentences.
unsafe extern "system" fn format_proc(
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
            let state = unsafe { &mut *(lparam.0 as *mut FormatState) };
            for (label, _) in FORMAT_ROLES {
                let text = wsz(label);
                unsafe {
                    SendDlgItemMessageW(
                        hdlg,
                        i32::from(ID_W_ROLE),
                        CB_ADDSTRING,
                        WPARAM(0),
                        LPARAM(text.as_ptr() as isize),
                    );
                }
            }
            let example = wsz(state.wizard.example().unwrap_or_default());
            let name = wsz(&state.wizard.name);
            unsafe {
                let _ = SetDlgItemTextW(hdlg, i32::from(ID_W_EXAMPLE), PCWSTR(example.as_ptr()));
                let _ = SetDlgItemTextW(hdlg, i32::from(ID_W_SAVEAS), PCWSTR(name.as_ptr()));
            }
            for id in [ID_W_FIELDS, ID_W_PREVIEW] {
                if let Ok(list) = unsafe { GetDlgItem(hdlg, i32::from(id)) } {
                    unsafe {
                        SendMessageW(
                            list,
                            LVM_SETEXTENDEDLISTVIEWSTYLE,
                            WPARAM(0),
                            LPARAM((LVS_EX_FULLROWSELECT | LVS_EX_GRIDLINES) as isize),
                        );
                    }
                }
            }
            format_refresh(hdlg, state, Some(0));
            // The field list is what the dialog is about, and every verb beside it acts on the
            // row selected there — so that is where the keyboard starts, not on the last button
            // Windows happens to reach.
            if let Ok(fields) = unsafe { GetDlgItem(hdlg, i32::from(ID_W_FIELDS)) } {
                unsafe {
                    let _ = SetFocus(fields);
                }
            }
            0
        }
        WM_NOTIFY => {
            let header = unsafe { &*(lparam.0 as *const NMHDR) };
            if header.idFrom == ID_W_FIELDS as usize && header.code == LVN_ITEMCHANGED {
                if let Some(state) = format_state(hdlg) {
                    if let Ok(list) = unsafe { GetDlgItem(hdlg, i32::from(ID_W_FIELDS)) } {
                        format_show_role(hdlg, state, lv_selected(list));
                    }
                }
            }
            0
        }
        WM_COMMAND => {
            let Some(state) = format_state(hdlg) else {
                return 0;
            };
            let id = (wparam.0 & 0xFFFF) as u16;
            let code = ((wparam.0 >> 16) & 0xFFFF) as u32;
            let Ok(list) = (unsafe { GetDlgItem(hdlg, i32::from(ID_W_FIELDS)) }) else {
                return 0;
            };
            if FORMAT_QUIET.with(|q| q.get()) && matches!(code, EN_CHANGE | CBN_SELCHANGE) {
                return 0;
            }
            let at = lv_selected(list);
            let example = state.wizard.example().unwrap_or_default().to_owned();
            match (id, code) {
                (IDOK, _) => {
                    // **Save refuses here, where the user is looking.** Letting a nameless or
                    // uncompilable format close the dialog and fail in the status bar would put
                    // the reason a long way from the controls that fix it — and would leave the
                    // shell holding a wizard it must then either drop or draw.
                    let name = read_dlg_text(hdlg, ID_W_SAVEAS);
                    if name.trim().is_empty() {
                        format_say(hdlg, "name the format before saving it");
                        if let Ok(box_) = unsafe { GetDlgItem(hdlg, i32::from(ID_W_SAVEAS)) } {
                            unsafe {
                                let _ = SetFocus(box_);
                            }
                        }
                    } else if let Some(why) = state.wizard.error() {
                        format_say(hdlg, &why);
                    } else {
                        state.wizard.name = name;
                        state.accepted = true;
                        unsafe {
                            let _ = EndDialog(hdlg, 1);
                        }
                    }
                }
                (IDCANCEL, _) => unsafe {
                    let _ = EndDialog(hdlg, 0);
                },
                (ID_W_TEST, _) => format_test(hdlg, state),
                (ID_W_ROLE, CBN_SELCHANGE) => {
                    let chosen = unsafe {
                        SendDlgItemMessageW(
                            hdlg,
                            i32::from(ID_W_ROLE),
                            CB_GETCURSEL,
                            WPARAM(0),
                            LPARAM(0),
                        )
                        .0
                    };
                    if let (Some(at), Some((_, role))) = (
                        at,
                        usize::try_from(chosen)
                            .ok()
                            .and_then(|c| FORMAT_ROLES.get(c)),
                    ) {
                        if let Err(why) = state.wizard.set_role(at, *role) {
                            format_say(hdlg, &why);
                        } else {
                            format_refresh(hdlg, state, Some(at));
                        }
                    }
                }
                (ID_W_NAME, EN_CHANGE) => {
                    if let Some(at) = at {
                        let name = read_dlg_text(hdlg, ID_W_NAME);
                        if let Err(why) = state.wizard.set_name(at, &name) {
                            format_say(hdlg, &why);
                        } else {
                            // The list and the pattern follow the name, but the Name box must not
                            // be rewritten from under the caret while it is being typed in.
                            let rows = format_field_rows(state.wizard);
                            if let Some(row) = rows.get(at) {
                                let token = wsz(&row.token);
                                unsafe {
                                    SendMessageW(
                                        list,
                                        LVM_SETITEMTEXTW,
                                        WPARAM(at),
                                        LPARAM(&mut LVITEMW {
                                            mask: LVIF_TEXT,
                                            iItem: at as i32,
                                            iSubItem: 2,
                                            pszText: PWSTR(token.as_ptr() as *mut u16),
                                            ..Default::default()
                                        }
                                            as *mut LVITEMW
                                            as isize),
                                    );
                                }
                            }
                            let pattern = wsz(&state.wizard.template());
                            unsafe {
                                let _ = SetDlgItemTextW(
                                    hdlg,
                                    i32::from(ID_W_PATTERN),
                                    PCWSTR(pattern.as_ptr()),
                                );
                            }
                        }
                    }
                }
                (ID_W_BOUNDS, _) => match (at, format_selection(hdlg, &example)) {
                    (Some(at), Some(span)) => {
                        // The end first: moving the start past the old end would be refused as an
                        // empty field, and a two-step edit must not fail on its own first half.
                        let widen = span.end >= state.wizard.fields()[at].end;
                        let order = if widen {
                            [(Edge::End, span.end), (Edge::Start, span.start)]
                        } else {
                            [(Edge::Start, span.start), (Edge::End, span.end)]
                        };
                        let mut failed = None;
                        for (edge, to) in order {
                            if let Err(why) = state.wizard.move_boundary(at, edge, to) {
                                failed = Some(why);
                                break;
                            }
                        }
                        match failed {
                            Some(why) => format_say(hdlg, &why),
                            None => format_refresh(hdlg, state, Some(at)),
                        }
                    }
                    (None, _) => format_say(hdlg, "select a field first"),
                    (_, None) => format_say(hdlg, "select part of the example line first"),
                },
                (ID_W_SPLIT, _) => match (at, format_selection(hdlg, &example)) {
                    (Some(at), Some(span)) => match state.wizard.split(at, span.start) {
                        Err(why) => format_say(hdlg, &why),
                        Ok(()) => format_refresh(hdlg, state, Some(at)),
                    },
                    (None, _) => format_say(hdlg, "select a field first"),
                    (_, None) => format_say(hdlg, "select where in the example to split it"),
                },
                (ID_W_ADD, _) => match format_selection(hdlg, &example) {
                    Some(span) => match state.wizard.add_field(span, Role::Named, "column") {
                        Err(why) => format_say(hdlg, &why),
                        Ok(added) => format_refresh(hdlg, state, Some(added)),
                    },
                    None => format_say(hdlg, "select the part of the example to make a field"),
                },
                (ID_W_MERGE, _) => match at {
                    Some(at) => match state.wizard.merge(at) {
                        Err(why) => format_say(hdlg, &why),
                        Ok(()) => format_refresh(hdlg, state, Some(at)),
                    },
                    None => format_say(hdlg, "select a field first"),
                },
                (ID_W_REMOVE, _) => match at {
                    Some(at) => match state.wizard.remove_field(at) {
                        Err(why) => format_say(hdlg, &why),
                        Ok(()) => format_refresh(hdlg, state, Some(at.saturating_sub(1))),
                    },
                    None => format_say(hdlg, "select a field first"),
                },
                _ => return 0,
            }
            1
        }
        WM_DESTROY => {
            unsafe {
                SetWindowLongPtrW(hdlg, WINDOW_LONG_PTR_INDEX(DWLP_USER), 0);
            }
            0
        }
        _ => 0,
    }
}

/// The dialog's state, or `None` before `WM_INITDIALOG` has stored it — which is a real moment,
/// since Windows sends a control's messages before then.
fn format_state<'a>(hdlg: HWND) -> Option<&'a mut FormatState<'a>> {
    unsafe {
        (GetWindowLongPtrW(hdlg, WINDOW_LONG_PTR_INDEX(DWLP_USER)) as *mut FormatState).as_mut()
    }
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

    /// The field list shows the example's own text, so a row can be judged by reading it — and
    /// the token beside it is what that row will become in the pattern.
    #[test]
    fn the_format_field_rows_carry_the_examples_text_and_the_token_it_becomes() {
        let mut wizard = Wizard::from_example("2026-08-07 07:50:45 INFO hello world");
        let rows = format_field_rows(&wizard);
        assert!(!rows.is_empty(), "a proposal splits the line");
        assert!(
            rows.iter().any(|r| r.token == "<ts>"),
            "the timestamp is proposed"
        );
        for row in &rows {
            assert!(!row.text.is_empty(), "no field is empty");
            assert!(row.token.starts_with('<') && row.token.ends_with('>'));
        }
        wizard.set_role(0, Role::Discard).expect("role");
        let after = format_field_rows(&wizard);
        assert_eq!(after[0].role, "Ignore");
        assert_eq!(after[0].token, "<_>");
        assert_eq!(
            after[0].text, rows[0].text,
            "a role change does not move the span"
        );
    }

    /// A wizard from a pasted layout has no example, so it has no rows — an empty list, never a
    /// list of wrong ones.
    #[test]
    fn a_pasted_layout_has_no_field_rows() {
        let wizard = Wizard::from_layout(tailhawk_core::template::Language::Log4net, "%d %m");
        assert!(format_field_rows(&wizard).is_empty());
    }

    /// "Recognised as" says the language, or the hint, or why — and an **empty** box gets the
    /// hint rather than a refusal, because a box nobody has typed in has not failed at anything.
    #[test]
    fn the_recognised_line_hints_before_it_complains() {
        assert_eq!(recognised_label(""), IMPORT_HINT);
        assert_eq!(recognised_label("   \r\n "), IMPORT_HINT);
        assert_eq!(
            recognised_label("%date %level %logger %message"),
            tailhawk_core::wizard::language_label(tailhawk_core::template::Language::Log4net)
        );
        let refused = recognised_label("this is not a layout at all");
        assert!(
            refused != IMPORT_HINT && !refused.is_empty(),
            "an unrecognised paste says why: {refused}"
        );
    }

    /// A second layout is the second **of its own config file**, not of the list — the number
    /// becomes the definition's name, so numbering by the list index would call the only layout
    /// in the second file its own second.
    #[test]
    fn a_finding_is_numbered_within_its_own_config_file() {
        let found = |file: &str, template: &str| tailhawk_core::template::Found {
            language: tailhawk_core::template::Language::NLog,
            template: template.to_owned(),
            source: std::path::PathBuf::from(r"C:\dev\ndc\Api").join(file),
        };
        let all = [
            found("NLog.config", "${longdate}|${message}"),
            found("NLog.config", "${level}|${message}"),
            found("appsettings.json", "${logger}|${message}"),
        ];
        assert_eq!(nth_in_file(&all, 0), 0);
        assert_eq!(nth_in_file(&all, 1), 1, "the second layout of NLog.config");
        assert_eq!(
            nth_in_file(&all, 2),
            0,
            "the only layout of appsettings.json, not the list's third"
        );
        assert_eq!(nth_in_file(&all, 9), 0, "out of range is not a panic");
    }

    /// The findings list is read, not guessed at: each row carries the file, the language and the
    /// layout itself.
    #[test]
    fn the_import_findings_carry_the_file_the_language_and_the_layout() {
        let found = vec![tailhawk_core::template::Found {
            language: tailhawk_core::template::Language::NLog,
            template: "${longdate} ${level} ${message}".to_owned(),
            source: std::path::PathBuf::from(r"C:\app\config\nlog.config"),
        }];
        let rows = import_found_rows(&found);
        assert_eq!(rows[0].file, "nlog.config", "the file, not the whole path");
        assert_eq!(rows[0].language, "NLog layout");
        assert_eq!(rows[0].layout, "${longdate} ${level} ${message}");
        assert!(import_found_rows(&[]).is_empty());
    }

    /// The edit control counts UTF-16 units and the wizard counts UTF-8 bytes. On an ASCII line
    /// they agree, which is the trap; on a line with an em dash in it they do not, and every
    /// answer must still land on a character boundary.
    #[test]
    fn a_carets_position_becomes_a_byte_offset_on_a_character_boundary() {
        let ascii = "2026-08-07 INFO hello";
        for units in 0..=ascii.chars().count() {
            assert_eq!(byte_at_utf16(ascii, units), units, "ASCII counts alike");
        }
        let wide = "a — b 😀 c";
        for units in 0..=wide.encode_utf16().count() {
            let at = byte_at_utf16(wide, units);
            assert!(wide.is_char_boundary(at), "{units} landed mid-character");
        }
        assert_eq!(byte_at_utf16(wide, 0), 0);
        assert_eq!(
            byte_at_utf16(wide, 2),
            "a ".len(),
            "the em dash starts here"
        );
        assert_eq!(byte_at_utf16(wide, 3), "a — ".len() - 1);
        assert_eq!(
            byte_at_utf16(wide, 9_999),
            wide.len(),
            "past the end is the end"
        );
    }

    /// The status line above the preview is the whole readout: untested, uncompilable, or the
    /// match rate — and the untested case must not read as a failure.
    #[test]
    fn the_format_status_reads_as_progress_rather_than_failure() {
        assert!(format_status(None).contains("not tested yet"));
        let broken = Test {
            error: Some("no such token".to_owned()),
            ..Test::default()
        };
        assert_eq!(format_status(Some(&broken)), "no such token");
        let tested = Test {
            columns: vec!["ts".to_owned()],
            rows: vec![None, Some(vec![Some(0..4)])],
            matched: 1,
            error: None,
        };
        assert_eq!(format_status(Some(&tested)), "1 of 2 matched (50%)");
    }

    /// The preview turns the model's spans into the sample's own text, and keeps the unmatched
    /// rows as `None` so the dialog can show them for what they are.
    #[test]
    fn the_format_preview_cuts_each_sample_by_its_own_spans() {
        let samples = vec!["alpha beta".to_owned(), "nope".to_owned()];
        let test = Test {
            columns: vec!["one".to_owned(), "two".to_owned()],
            rows: vec![Some(vec![Some(0..5), Some(6..10)]), None],
            matched: 1,
            error: None,
        };
        let rows = format_preview(&test, &samples, 10);
        assert_eq!(
            rows[0].as_deref(),
            Some(["alpha".to_owned(), "beta".to_owned()].as_slice())
        );
        assert!(rows[1].is_none(), "an unmatched sample stays unmatched");
        assert_eq!(format_preview(&test, &samples, 1).len(), 1, "the cap holds");
    }

    /// No two controls of one dialog claim the same mnemonic — the rule the menus keep, for the
    /// same reason: the second claimant is unreachable by the key drawn under it. `Pa&ttern` and
    /// `&Test` both wanted `t`, and the pattern box is read-only, so it gave the letter up.
    #[test]
    fn no_two_controls_of_a_dialog_share_a_mnemonic() {
        for (name, items) in [
            ("Define Format", format_dialog_items()),
            ("Filter", filter_dialog_items()),
        ] {
            let mut seen = Vec::new();
            for item in &items {
                let mut chars = item.text.chars().peekable();
                while let Some(c) = chars.next() {
                    if c != '&' {
                        continue;
                    }
                    match chars.peek() {
                        Some('&') => {
                            chars.next();
                        }
                        Some(m) => {
                            let m = m.to_ascii_lowercase();
                            assert!(
                                !seen.contains(&m),
                                "{name}: two controls claim '{m}' — the second is unreachable"
                            );
                            seen.push(m);
                            break;
                        }
                        None => break,
                    }
                }
            }
            assert!(
                !seen.is_empty(),
                "{name}: a dialog with no mnemonics at all"
            );
        }
    }

    /// The Filter dialog reads down one left edge. Every labelled field starts in the same column
    /// and every full-width one ends at the same right edge — the owner saw the Expression box
    /// step sideways from the Value box above it, which is what a shared column is for.
    #[test]
    fn the_filter_dialogs_fields_share_one_column_and_one_right_edge() {
        let items = filter_dialog_items();
        let at = |id: u16| items.iter().find(|i| i.id == id).expect("control present");
        for id in [ID_F_SCOPE, ID_F_VALUE, ID_F_REGEX, ID_F_EXPR] {
            assert_eq!(
                at(id).x,
                F_FIELD_X,
                "control {id} starts outside the field column"
            );
        }
        for id in [ID_F_VALUE, ID_F_EXPR, ID_F_OP, ID_F_STATUS] {
            let it = at(id);
            assert_eq!(it.x + it.w, F_RIGHT, "control {id} ends off the right edge");
        }
        for id in [ID_F_SCOPE, ID_F_VALUE, ID_F_EXPR] {
            assert!(
                at(id).x >= F_LABEL_X + F_LABEL_W,
                "control {id} overlaps its label"
            );
        }
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
    /// with a dialog that simply does not open. And a class is written one of two ways: the
    /// `0xFFFF` marker and an ordinal for the pre-defined four, the **name** for anything else,
    /// which is one word shorter or longer and therefore moves everything after it.
    #[test]
    fn every_item_is_dword_aligned_and_carries_its_class_ordinal_or_name() {
        let mut synthetic = two_items();
        synthetic.push(Item::new(
            Class::Named("SysListView32"),
            "",
            200,
            (7, 20, 80, 40),
            0,
        ));
        // **The real dialogs are walked too, not only a made-up pair.** A template Windows will
        // not open fails silently — `DialogBoxIndirectParamW` returns and nothing appears — so
        // the shape of every template the app actually ships is checked here, where a miscount is
        // an assertion rather than a dialog that never comes up.
        for (name, items) in [
            ("synthetic", synthetic),
            ("Define Format", format_dialog_items()),
            ("Import Layout", import_dialog_items()),
            ("Filter", filter_dialog_items()),
        ] {
            let t = template("x", 100, 100, &items);
            // Walk the template the way Windows does: header, then aligned items.
            let mut at = 9; // style, exstyle, cdit, x, y, cx, cy
            at += 2; // no menu, standard class — one ordinal word each
            at += "x".encode_utf16().count() + 1; // caption
            at += 1; // point size
            at += "MS Shell Dlg".encode_utf16().count() + 1;
            assert_eq!(t[4], items.len() as u16, "{name}: cdit counts the items");
            for item in &items {
                if at % 2 == 1 {
                    at += 1;
                }
                assert_eq!(at % 2, 0, "{name}: item starts on a DWORD boundary");
                let style = u32::from(t[at]) | (u32::from(t[at + 1]) << 16);
                assert_eq!(
                    style & 0x5000_0000,
                    0x5000_0000,
                    "{name}: WS_CHILD | WS_VISIBLE on every control"
                );
                assert_eq!(t[at + 8], item.id);
                at += 9;
                match item.class {
                    Class::Named(class) => {
                        let wide: Vec<u16> = class.encode_utf16().collect();
                        assert_eq!(&t[at..at + wide.len()], &wide[..], "{name}: class by name");
                        assert_eq!(t[at + wide.len()], 0, "{name}: and NUL-terminated");
                        at += wide.len() + 1;
                    }
                    other => {
                        assert_eq!(t[at], 0xFFFF, "{name}: ordinal class marker");
                        assert_eq!(t[at + 1], other.ordinal().expect("an ordinal class"));
                        at += 2;
                    }
                }
                at += item.text.encode_utf16().count() + 1;
                assert_eq!(t[at], 0, "{name}: no creation data");
                at += 1;
            }
            assert_eq!(
                at,
                t.len(),
                "{name}: the walk consumed exactly the template"
            );
        }
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

    /// The Filter dialog's composition, row by row: every structured choice must round-trip
    /// through `Chip::parse`, or the dialog would write expressions the model refuses.
    #[test]
    fn the_filter_dialog_writes_expressions_the_model_accepts() {
        use tailhawk_core::filter::{Chip, Polarity};
        let cases = [
            (None, 0, "timeout", false, false, "timeout"),
            // Text that reads as an expression is quoted so it means its letters.
            (
                None,
                0,
                "level >= Warning",
                false,
                false,
                "\"level >= Warning\"",
            ),
            (None, 0, "Retry", false, true, "/Retry/"),
            (None, 0, "a|b", true, false, "/a|b/i"),
            (None, 0, "a|b", true, true, "/a|b/"),
            (
                Some("level"),
                6,
                "Warning",
                false,
                false,
                "level >= Warning",
            ),
            (Some("elapsed"), 5, "300", false, false, "elapsed > 300"),
            (
                Some("source"),
                8,
                "Api",
                false,
                false,
                "startsWith(source, \"Api\")",
            ),
            (
                Some("message"),
                0,
                "dispatch",
                false,
                false,
                "contains(message, \"dispatch\")",
            ),
            (
                Some("source"),
                7,
                "api",
                false,
                false,
                "source like \"api\"",
            ),
        ];
        for (column, op, value, regex, case, want) in cases {
            let got = compose_filter(column, op, value, regex, case);
            assert_eq!(got, want, "compose({column:?}, {op}, {value:?})");
            assert!(
                Chip::parse(&got, Polarity::Include).is_ok(),
                "{got:?} does not parse"
            );
        }
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
