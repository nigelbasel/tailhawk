//! The toolbar as one frame should draw it — `UI-DESIGN.md` §2.3.
//!
//! **The view-model half.** A [`ToolButton`] is a button as it stands right now: its label, the
//! command id a click sends, whether it can act, and — for the three toggles — whether it is
//! pressed. [`toolbar_of`] is the model → view-model mapping, and it is pure, so a toolbar that
//! disagrees with the menu beside it is a failing test rather than a screenshot someone has to
//! notice.
//!
//! **The Win32 half is at the bottom of this file**, as `tabstrip.rs` keeps its own: [`Toolbar`]
//! creates a real `ToolbarWindow32`, fills it from the view-model above, and decides nothing.
//!
//! **The ids are `menubar::command_id`'s**, not a set of the toolbar's own. A click therefore
//! arrives as the same `WM_COMMAND` the menu sends and goes down the same dispatch, which is what
//! §1.2's *one command, one name, one path* asks for. A toolbar with private ids would be a second
//! implementation of every command it offers, free to drift from the menu without anything saying
//! so.
//!
//! **Text buttons and no image list.** §1.1 rejects "a toolbar of ambiguous, unlabelled 16×16
//! icons" and requires a label beside any icon; the simplest way to keep that promise is to have no
//! icon to keep it about. An icon set is a design commitment nobody has asked for.

use crate::menubar::command_id;
use crate::tabstrip::shell_font;
use crate::{Command, Document};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, RECT, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, CreateFontW, DeleteDC, DeleteObject, DrawTextW, GdiFlush,
    GetDC, GetGlyphIndicesW, ReleaseDC, SelectObject, SetBkMode, SetTextColor, BITMAPINFO,
    BITMAPINFOHEADER, BI_RGB, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET,
    DIB_RGB_COLORS, DT_CENTER, DT_NOCLIP, DT_SINGLELINE, DT_VCENTER, FW_NORMAL, HBITMAP, HDC,
    HFONT, HGDIOBJ, OUT_DEFAULT_PRECIS, TRANSPARENT,
};
use windows::Win32::UI::Controls::{
    ImageList_Add, ImageList_Create, ImageList_Destroy, InitCommonControlsEx, BTNS_AUTOSIZE,
    BTNS_CHECK, BTNS_SEP, BTNS_SHOWTEXT, CCS_NODIVIDER, CCS_NOPARENTALIGN, CCS_NORESIZE,
    HIMAGELIST, ICC_BAR_CLASSES, ILC_COLOR32, INITCOMMONCONTROLSEX, NMTBGETINFOTIPW, RBBIM_CHILD,
    RBBIM_CHILDSIZE, RBBIM_SIZE, RBBIM_STYLE, RBBS_CHILDEDGE, RBBS_GRIPPERALWAYS, RBS_BANDBORDERS,
    RBS_VARHEIGHT, RB_GETBANDCOUNT, RB_GETBARHEIGHT, RB_INSERTBANDW, RB_SETBANDINFOW,
    RB_SETBARINFO, REBARBANDINFOW, REBARINFO, TBBUTTON, TBSTATE_CHECKED, TBSTATE_ENABLED,
    TBSTYLE_FLAT, TBSTYLE_TOOLTIPS, TB_ADDBUTTONSW, TB_ADDSTRINGW, TB_AUTOSIZE, TB_BUTTONCOUNT,
    TB_BUTTONSTRUCTSIZE, TB_DELETEBUTTON, TB_GETITEMRECT, TB_GETMAXSIZE, TB_SETBITMAPSIZE,
    TB_SETIMAGELIST, TB_SETSTATE,
};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, GetWindowRect, SendMessageW, SetWindowPos, ShowWindow, HMENU,
    HWND_TOP, SWP_NOACTIVATE, SWP_NOZORDER, SW_HIDE, SW_SHOWNA, WINDOW_EX_STYLE, WINDOW_STYLE,
    WM_SETFONT, WM_SIZE, WS_CHILD, WS_CLIPCHILDREN, WS_CLIPSIBLINGS,
};

/// `I_IMAGENONE` — the button has no image at all, rather than image zero of an absent list.
///
/// Declared here because the `windows` crate does not bind it; the value is the documented one.
const I_IMAGENONE: i32 = -2;

/// The pixels left under the buttons, so the band is not flush against the glyphs. Two device
/// pixels either side of the buttons is what a flat toolbar leaves at 100 %.
const BAND_MARGIN: i32 = 2;

/// One button, as it stands this frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolButton {
    /// What the button is called. **Not drawn any more** — it is the tooltip's first line and the
    /// name a screen reader reads, since the owner asked for an icon-only bar on 2026-09-08.
    pub label: &'static str,
    /// The glyph drawn on the button, from Windows' own icon font. See [`Toolbar::icons`] for what
    /// happens when the font or the glyph is missing: never a tofu box.
    pub icon: char,
    /// What the tooltip says: the command and the keys that reach it, e.g. `Find…   Ctrl+F`. An
    /// icon-only toolbar is only usable because this exists, so it is part of the view-model rather
    /// than something the Win32 half composes.
    pub tip: String,
    /// The command id a click posts — the menu's, deliberately.
    pub id: u32,
    /// Whether the command can act on what is open. A button that cannot is disabled in place
    /// rather than removed, so the row does not move under the pointer.
    pub enabled: bool,
    /// Whether this is a toggle rather than a verb. §2.3: the toggles read as a status display as
    /// well as a control, so they must be drawn pressed and not merely highlighted.
    pub toggle: bool,
    /// Whether a toggle is currently on. Always `false` for a verb.
    pub pressed: bool,
    /// Whether a separator stands before this button. **Groups are what make an icon-only row
    /// readable**: without them nine identical squares are a wall, and a user has to hover every
    /// one to find the third thing they wanted.
    pub starts_group: bool,
}

/// The toolbar for this document, or for no document at all.
///
/// **Every button except Open needs something open**, which is the whole of the enabling rule for
/// the verbs; the three toggles additionally report their state. With no document the row is a line
/// of greyed icons with `Open` live at its head — deliberately still there, because a toolbar that
/// appeared when the first file did would move the grid down the moment a file arrives.
///
/// **Icons, groups and tooltips, since 2026-09-08.** The owner's decision, after asking four times
/// across three sessions for a toolbar that looks like a Windows toolbar: a row of labelled words
/// is not one. The glyphs are Windows' own; the separators mark file / find / view / rules / out;
/// and the tooltip carries the command's name and its keys, which is what makes an icon-only row
/// usable by someone who has not memorised it.
pub fn toolbar_of(doc: Option<&Document>) -> Vec<ToolButton> {
    let open = doc.is_some();
    let verb =
        |label: &'static str, icon: char, c: Command, enabled: bool, starts: bool| ToolButton {
            label,
            icon,
            tip: tip_for(label, c),
            id: command_id(c),
            enabled,
            toggle: false,
            pressed: false,
            starts_group: starts,
        };
    let toggle =
        |label: &'static str, icon: char, c: Command, pressed: bool, starts: bool| ToolButton {
            label,
            icon,
            tip: tip_for(label, c),
            id: command_id(c),
            enabled: open,
            toggle: true,
            pressed: open && pressed,
            starts_group: starts,
        };
    vec![
        verb("Open", icon::OPEN, Command::OpenFile, true, false),
        verb("Find", icon::FIND, Command::Find, open, true),
        // A toggle, not a verb, since 2026-09-03: the owner's answer to §2.5's open question. It
        // reports whether the filter panel is shown, the way Follow reports following.
        toggle(
            "Filter",
            icon::FILTER,
            Command::ToggleFilters,
            doc.is_some_and(|d| d.show_filters),
            false,
        ),
        toggle(
            "Follow",
            icon::FOLLOW,
            Command::FollowTail,
            doc.is_some_and(|d| d.is_following()),
            true,
        ),
        toggle(
            "Collapse",
            icon::COLLAPSE,
            Command::ToggleCollapse,
            doc.is_some_and(|d| d.is_collapsed()),
            false,
        ),
        toggle(
            "Detail",
            icon::DETAIL,
            Command::ToggleDetail,
            doc.is_some_and(|d| d.detail_open()),
            false,
        ),
        verb("Rules", icon::RULES, Command::EditRules, true, true),
        verb("Format", icon::FORMAT, Command::DefineFormat, open, false),
        verb("Export", icon::EXPORT, Command::Export, open, true),
    ]
}

/// The tooltip for one button: what it is called, and the keys that reach it.
///
/// The keys come from [`Command::LISTED`], which is the one register the menu also reads, so a
/// tooltip cannot promise a shortcut the menu does not offer. A command with no shortcut says only
/// its name rather than trailing an empty gap.
fn tip_for(label: &str, command: Command) -> String {
    match Command::LISTED
        .iter()
        .find(|(c, _, _)| *c == command)
        .map(|(_, _, keys)| *keys)
    {
        Some(keys) if !keys.is_empty() => format!("{label}\u{a0}\u{a0}({keys})"),
        _ => label.to_owned(),
    }
}

/// The glyphs, from Windows' own icon font — `Segoe Fluent Icons` on Windows 11, `Segoe MDL2
/// Assets` on Windows 10, which is §2.1's floor. Both fonts share these code points.
///
/// **Every one of them is checked for at runtime** before it is drawn, because a code point this
/// file believes in and the installed font does not would otherwise draw a hollow tofu box on the
/// owner's toolbar — the exact class of thing this change exists to remove. See [`Toolbar::icons`].
pub mod icon {
    /// `OpenFolder` — the folder Explorer itself puts on an Open button. `OpenFile` (`E8E5`) was
    /// tried first and draws a page with an arrow through it, which reads as *export*.
    pub const OPEN: char = '\u{E838}';
    /// `Search` — a magnifier.
    pub const FIND: char = '\u{E721}';
    /// `Filter` — a funnel.
    pub const FILTER: char = '\u{E71C}';
    /// `Down` — an arrow to the tail, which is what following is.
    pub const FOLLOW: char = '\u{E74B}';
    /// `ChevronUp`. `CollapseContent` (`E96E`) is a chevron pointing *down*, which every reader of
    /// a toolbar takes for "expand" — a sheet of the candidates drawn from the font itself is what
    /// settled this one and `OPEN` above, rather than a code point chosen from memory.
    pub const COLLAPSE: char = '\u{E70E}';
    /// `OpenPane` — the detail pane opens along an edge, which is what this draws.
    pub const DETAIL: char = '\u{E8A0}';
    /// `Color` — a palette, for the highlight rules.
    pub const RULES: char = '\u{E790}';
    /// `ViewAll` — a grid of panes, for the format that makes the columns.
    pub const FORMAT: char = '\u{E8A9}';
    /// `Save` — export writes a file.
    pub const EXPORT: char = '\u{E74E}';
}

/// `TBN_GETINFOTIPW` — the toolbar asking its parent what a button's tooltip should say.
///
/// `TBN_FIRST` is -700 and this is nineteen below it. Declared here because the `windows` crate
/// does not bind the notification codes, and the value is the documented one.
pub const TBN_GETINFOTIPW: u32 = (-719_i32) as u32;

/// The rebar's own id, so its notifications are its own.
pub const ID_REBAR: i32 = 4_201;

/// The control's own id, so a notification from it can be told apart from the tab strip's.
pub const ID_TOOLBAR: i32 = 4_200;

/// The icon font Windows 11 ships, and the one Windows 10 ships. Both carry the code points
/// [`icon`] names; the newer one is tried first so an 11 machine gets its own shapes.
const ICON_FONTS: [PCWSTR; 2] = [w!("Segoe Fluent Icons"), w!("Segoe MDL2 Assets")];

/// `GetGlyphIndicesW` writes this for a code point the font has no glyph for.
const GLYPH_MISSING: u16 = 0xFFFF;
const GGI_MARK_NONEXISTING_GLYPHS: u32 = 1;

/// An image list of the toolbar's glyphs, drawn from the system icon font at this DPI.
///
/// **`None` rather than a box.** If neither font is installed, or the font is installed but lacks
/// one of the code points, this reports nothing and the toolbar falls back to its old labelled
/// buttons. A tofu box on a toolbar is precisely the "ugly and non standard" the owner asked to be
/// rid of, and a wrong guess at a code point in this file must degrade to words rather than to a
/// hollow rectangle.
///
/// The glyphs are drawn white on transparent and then tinted, because GDI's text drawing writes no
/// alpha at all: what it leaves behind is coverage in the colour channels, which is exactly the
/// mask a 32-bit image list wants once it is turned into premultiplied alpha. Tinting from the
/// theme is what lets one set of glyphs serve a dark toolbar and a light one.
fn icon_list(px: i32, colour: u32) -> Option<HIMAGELIST> {
    let glyphs = [
        icon::OPEN,
        icon::FIND,
        icon::FILTER,
        icon::FOLLOW,
        icon::COLLAPSE,
        icon::DETAIL,
        icon::RULES,
        icon::FORMAT,
        icon::EXPORT,
    ];
    let screen = unsafe { GetDC(None) };
    let dc = unsafe { CreateCompatibleDC(screen) };
    unsafe {
        ReleaseDC(None, screen);
    }
    if dc.is_invalid() {
        return None;
    }

    let font = ICON_FONTS.iter().find_map(|face| {
        let font = unsafe {
            CreateFontW(
                -px,
                0,
                0,
                0,
                FW_NORMAL.0 as i32,
                0,
                0,
                0,
                DEFAULT_CHARSET.0 as u32,
                OUT_DEFAULT_PRECIS.0 as u32,
                CLIP_DEFAULT_PRECIS.0 as u32,
                CLEARTYPE_QUALITY.0 as u32,
                0,
                *face,
            )
        };
        if font.is_invalid() {
            return None;
        }
        let old = unsafe { SelectObject(dc, HGDIOBJ(font.0)) };
        let has_all = glyphs.iter().all(|glyph| {
            let mut index = [0u16; 1];
            let text: Vec<u16> = glyph.to_string().encode_utf16().collect();
            let read = unsafe {
                GetGlyphIndicesW(
                    dc,
                    PCWSTR(text.as_ptr()),
                    1,
                    index.as_mut_ptr(),
                    GGI_MARK_NONEXISTING_GLYPHS,
                )
            };
            read != u32::MAX && index[0] != GLYPH_MISSING
        });
        unsafe {
            SelectObject(dc, old);
        }
        if has_all {
            Some(font)
        } else {
            unsafe {
                let _ = DeleteObject(HGDIOBJ(font.0));
            }
            None
        }
    });
    let Some(font) = font else {
        unsafe {
            let _ = DeleteDC(dc);
        }
        return None;
    };

    let list = unsafe { ImageList_Create(px, px, ILC_COLOR32, glyphs.len() as i32, 0) };
    if list.is_invalid() {
        unsafe {
            let _ = DeleteObject(HGDIOBJ(font.0));
            let _ = DeleteDC(dc);
        }
        return None;
    }
    let old_font = unsafe { SelectObject(dc, HGDIOBJ(font.0)) };
    for glyph in glyphs {
        if let Some(bitmap) = draw_glyph(dc, glyph, px, colour) {
            unsafe {
                ImageList_Add(list, bitmap, HBITMAP::default());
                let _ = DeleteObject(HGDIOBJ(bitmap.0));
            }
        }
    }
    unsafe {
        SelectObject(dc, old_font);
        let _ = DeleteObject(HGDIOBJ(font.0));
        let _ = DeleteDC(dc);
    }
    Some(list)
}

/// One glyph as a premultiplied 32-bit bitmap of `px` square, tinted `colour` (`0x00RRGGBB`).
fn draw_glyph(dc: HDC, glyph: char, px: i32, colour: u32) -> Option<HBITMAP> {
    let info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: px,
            // Top-down, so the bits walk the way the rows are read below.
            biHeight: -px,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
    let bitmap = unsafe { CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, None, 0) }.ok()?;
    if bitmap.is_invalid() || bits.is_null() {
        return None;
    }

    let old = unsafe { SelectObject(dc, HGDIOBJ(bitmap.0)) };
    let mut rect = RECT {
        left: 0,
        top: 0,
        right: px,
        bottom: px,
    };
    unsafe {
        SetBkMode(dc, TRANSPARENT);
        SetTextColor(dc, COLORREF(0x00FF_FFFF));
        let mut text: Vec<u16> = glyph.to_string().encode_utf16().collect();
        DrawTextW(
            dc,
            &mut text,
            &mut rect,
            DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOCLIP,
        );
        // The DC has to be flushed before the bits are read: GDI batches, and the glyph may not be
        // in the bitmap yet.
        let _ = GdiFlush();
        SelectObject(dc, old);
    }

    // White-on-nothing becomes coverage: take the brightest channel as alpha, then write the tint
    // premultiplied by it. A pixel the glyph never touched stays fully transparent.
    let (r, g, b) = ((colour >> 16) & 0xFF, (colour >> 8) & 0xFF, colour & 0xFF);
    let pixels = unsafe { std::slice::from_raw_parts_mut(bits as *mut u8, (px * px * 4) as usize) };
    for pixel in pixels.chunks_exact_mut(4) {
        let alpha = pixel[0].max(pixel[1]).max(pixel[2]) as u32;
        pixel[0] = ((b * alpha) / 255) as u8;
        pixel[1] = ((g * alpha) / 255) as u8;
        pixel[2] = ((r * alpha) / 255) as u8;
        pixel[3] = alpha as u8;
    }
    Some(bitmap)
}

/// The real `ToolbarWindow32`, and everything about it that needs a window.
///
/// **It decides nothing.** Which buttons exist, whether each can act and whether each is on all
/// come from [`toolbar_of`]; this half inserts them, sets two state bits, and reports how tall the
/// control made itself.
pub struct Toolbar {
    /// The rebar that hosts the toolbar as a band — the gripper and the drag are its.
    rebar: Option<HWND>,
    hwnd: HWND,
    font: HFONT,
    /// The glyphs the buttons draw, owned so they can be destroyed and rebuilt when the DPI or the
    /// theme changes. `None` means the icon font could not supply them and the row is words.
    images: Option<HIMAGELIST>,
    /// Whether the icons are drawn in the larger of the two sizes. §2.3, the owner's ask of
    /// 2026-09-08.
    large: bool,
    /// The height the control asked for when it was last filled — see [`Toolbar::measure`].
    band: i32,
    /// What the control was last filled with, so a frame that changes nothing does not rebuild it.
    /// A rebuild repaints the whole band and drops a click that is mid-press.
    shown: Vec<ToolButton>,
    visible: bool,
}

impl Toolbar {
    /// Creates the control as a child of the main window, hidden until it is placed.
    ///
    /// **Icon-only, since the owner's decision of 2026-09-08.** `TBSTYLE_LIST` is gone with the
    /// labels, and `TBSTYLE_TOOLTIPS` arrives with them: an icon-only bar is only usable because
    /// hovering says what a button does, so the style and the parent's answer to
    /// `TBN_GETINFOTIPW` — see [`Toolbar::info_tip`] — are one feature and not two.
    pub fn create(parent: HWND, instance: HINSTANCE) -> Option<Toolbar> {
        let icc = INITCOMMONCONTROLSEX {
            dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_BAR_CLASSES,
        };
        unsafe {
            let _ = InitCommonControlsEx(&icc);
        }
        // The rebar first, because the toolbar is its child: the band owns the toolbar's size and
        // position, which is the arrangement the two CCS_ styles below were always written for.
        let rebar = Self::create_rebar(parent, instance);
        let host = rebar.unwrap_or(parent);
        let hwnd = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("ToolbarWindow32"),
                PCWSTR::null(),
                // **The three `CCS_*` bits are the ones a toolbar that is positioned by hand must
                // have.** `CCS_TOP` is the control's documented *default*: it re-aligns itself to
                // the top of the parent's client area, at the parent's full width, from inside its
                // own `WM_SIZE` — which `place`'s `SetWindowPos` raises. Without `CCS_NORESIZE` and
                // `CCS_NOPARENTALIGN` the control would snap back to y=0 on the next resize and sit
                // on top of the tab strip while the grid still reserved a band for it.
                // `CCS_NODIVIDER` removes the edge above it, which is chrome from a different era
                // and would also make the measured band disagree with the drawn one.
                //
                // **`TBSTYLE_TOOLTIPS` is what makes an icon usable.** The style creates the
                // tooltip window; the text comes from the parent answering `TBN_GETINFOTIPW`,
                // which `Toolbar::info_tip` fills from the same view-model the buttons came from.
                // The style alone would produce a tooltip that never says anything, which is why
                // it was refused while the buttons carried words of their own.
                WINDOW_STYLE(
                    WS_CHILD.0
                        | WS_CLIPSIBLINGS.0
                        | TBSTYLE_FLAT
                        | TBSTYLE_TOOLTIPS
                        | CCS_NODIVIDER as u32
                        | CCS_NOPARENTALIGN as u32
                        | CCS_NORESIZE as u32,
                ),
                0,
                0,
                0,
                0,
                host,
                HMENU(ID_TOOLBAR as *mut core::ffi::c_void),
                instance,
                None,
            )
        }
        .ok()?;
        // Documented as required before any other TB_ message: it is how comctl32 learns which
        // version of `TBBUTTON` this process was compiled against.
        unsafe {
            SendMessageW(
                hwnd,
                TB_BUTTONSTRUCTSIZE,
                WPARAM(std::mem::size_of::<TBBUTTON>()),
                LPARAM(0),
            );
        }
        let font = shell_font();
        if !font.is_invalid() {
            unsafe {
                SendMessageW(hwnd, WM_SETFONT, WPARAM(font.0 as usize), LPARAM(1));
            }
        }
        // The same theme decision the menus take — see `controls::apply_theme` for the screenshot
        // that made this one place rather than three.
        crate::header::trace(&format!("child: toolbar hwnd={:?}", hwnd.0));
        crate::controls::apply_theme(hwnd, tailhawk_core::theme::theme().dark);
        Some(Toolbar {
            rebar,
            hwnd,
            font,
            images: None,
            large: false,
            band: 0,
            shown: Vec::new(),
            visible: false,
        })
    }

    /// Creates the rebar that hosts the toolbar, and the toolbar inside it.
    ///
    /// **A rebar because that is the control docking lives in.** The owner asked for toolbars that
    /// dock "in the same way that most windows toolbars are", and the documentation is direct about
    /// where that behaviour comes from: *"A rebar control acts as a container for child windows…
    /// each band can have a gripper bar… As you dynamically reposition a rebar control band, the
    /// rebar control manages the size and position of the child window assigned to that band."* The
    /// gripper, the drag and the band's own layout are the control's, not this file's.
    ///
    /// It is also what the toolbar's two `CCS_` styles were always for: *"Toolbar controls that are
    /// hosted by rebar controls must set these styles because the rebar control sizes and positions
    /// the toolbar."* They were set here before there was a rebar, which left this window owning a
    /// sizing job it kept getting wrong.
    ///
    /// **What this does not give**, said plainly because the word "dockable" covers both: bands
    /// drag and reorder within the rebar, and the rebar sits where the window puts it. Docking to
    /// the left or right edge, or tearing a toolbar off into a floating window, was never a control
    /// — MFC built that on top — and it is not here.
    fn create_rebar(parent: HWND, instance: HINSTANCE) -> Option<HWND> {
        let rebar = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("ReBarWindow32"),
                PCWSTR::null(),
                WINDOW_STYLE(
                    WS_CHILD.0
                        | WS_CLIPCHILDREN.0
                        | WS_CLIPSIBLINGS.0
                        | RBS_VARHEIGHT
                        | RBS_BANDBORDERS
                        // The window positions the rebar, as it positions every other band of
                        // chrome; without these the control would align itself to the top of the
                        // client area, over the tab strip.
                        | CCS_NODIVIDER as u32
                        | CCS_NOPARENTALIGN as u32
                        | CCS_NORESIZE as u32,
                ),
                0,
                0,
                0,
                0,
                parent,
                HMENU(ID_REBAR as *mut core::ffi::c_void),
                instance,
                None,
            )
        }
        .ok()?;
        // Documented as required before any band is added, whether or not there are band images.
        let info = REBARINFO {
            cbSize: std::mem::size_of::<REBARINFO>() as u32,
            fMask: 0,
            himl: HIMAGELIST::default(),
        };
        unsafe {
            SendMessageW(
                rebar,
                RB_SETBARINFO,
                WPARAM(0),
                LPARAM(&info as *const REBARINFO as isize),
            );
        }
        crate::controls::apply_theme(rebar, tailhawk_core::theme::theme().dark);
        Some(rebar)
    }

    /// Puts the toolbar in a band, or resizes the band it is already in.
    ///
    /// The band's minimum height is the toolbar's own, which is why this runs after the buttons are
    /// in: the rebar sizes the child to the band, so a band told the wrong height would clip the
    /// icons exactly as this window's own arithmetic used to.
    fn seat_band(&mut self) {
        let Some(rebar) = self.rebar else {
            return;
        };
        let height = self.band.max(1);
        let mut band = REBARBANDINFOW {
            cbSize: std::mem::size_of::<REBARBANDINFOW>() as u32,
            fMask: RBBIM_STYLE | RBBIM_CHILD | RBBIM_CHILDSIZE | RBBIM_SIZE,
            fStyle: RBBS_CHILDEDGE | RBBS_GRIPPERALWAYS,
            hwndChild: self.hwnd,
            cxMinChild: 0,
            cyMinChild: height as u32,
            cx: 0,
            ..Default::default()
        };
        let bands = unsafe { SendMessageW(rebar, RB_GETBANDCOUNT, WPARAM(0), LPARAM(0)) }.0;
        unsafe {
            if bands > 0 {
                SendMessageW(
                    rebar,
                    RB_SETBANDINFOW,
                    WPARAM(0),
                    LPARAM(&mut band as *mut REBARBANDINFOW as isize),
                );
            } else {
                SendMessageW(
                    rebar,
                    RB_INSERTBANDW,
                    WPARAM(usize::MAX),
                    LPARAM(&mut band as *mut REBARBANDINFOW as isize),
                );
            }
        }
    }

    /// Fills the control from the view-model, rebuilding only when the row itself changed.
    ///
    /// **A state change is not a rebuild.** The nine buttons are re-added only when the *row*
    /// differs — different ids, labels or kinds — because re-adding repaints the whole band.
    ///
    /// **The state is re-asserted every frame, and the reason is a latch.** A `BTNS_CHECK` button
    /// flips its own `TBSTATE_CHECKED` before it sends `WM_COMMAND`, so after a click the control
    /// and the model disagree until something puts them back. Comparing against `self.shown` — a
    /// record of what was last *asked for*, not of what the control did to itself — cannot see that
    /// disagreement: click Follow while already following and the command turns following on again,
    /// so `pressed` never changes, no `TB_SETSTATE` is sent, and the button stays visibly unpressed
    /// over a document that is following. The menu does not have this problem because
    /// `WM_INITMENUPOPUP` refills it from the live document every single time it opens; nine
    /// idempotent `TB_SETSTATE` sends are this control's version of that, and cost nothing.
    pub fn set(&mut self, buttons: &[ToolButton]) {
        let same_row = self.shown.len() == buttons.len()
            && self
                .shown
                .iter()
                .zip(buttons)
                .all(|(a, b)| a.id == b.id && a.label == b.label && a.toggle == b.toggle);
        if same_row {
            for now in buttons {
                unsafe {
                    SendMessageW(
                        self.hwnd,
                        TB_SETSTATE,
                        WPARAM(now.id as usize),
                        LPARAM(state_bits(now) as isize),
                    );
                }
            }
        } else if !self.rebuild(buttons) {
            // The control refused them. Forget what we asked for, so the next frame asks again
            // instead of believing a row that is not there.
            self.shown.clear();
            return;
        }
        self.shown = buttons.to_vec();
    }

    /// Re-adds every button, with the separators that group them and the icons that name them.
    /// Reports whether the control accepted them.
    ///
    /// **The image list is rebuilt with the row**, because the glyphs are drawn at this window's
    /// DPI and in this theme's ink: a display change or a theme change is a different set of
    /// bitmaps, and there is exactly one row to rebuild. The old list is destroyed after the new
    /// one is set, never before — the control draws from whichever it holds.
    ///
    /// **`TB_DELETEBUTTON` does not free the string pool**, so each call appends to it. Nothing is
    /// added to that pool now that the buttons carry no text, which is one more thing the icon-only
    /// row made simpler rather than harder.
    fn rebuild(&mut self, buttons: &[ToolButton]) -> bool {
        while unsafe { SendMessageW(self.hwnd, TB_BUTTONCOUNT, WPARAM(0), LPARAM(0)) }.0 > 0 {
            unsafe {
                SendMessageW(self.hwnd, TB_DELETEBUTTON, WPARAM(0), LPARAM(0));
            }
        }

        let px = self.icon_px();
        // The ink the grid draws with, so the toolbar belongs to the same window as the log.
        let ink = tailhawk_core::theme::theme().ink;
        let byte = |c: f32| (c.clamp(0.0, 1.0) * 255.0).round() as u32;
        let colour = (byte(ink[0]) << 16) | (byte(ink[1]) << 8) | byte(ink[2]);
        let fresh = icon_list(px, colour);
        if let Some(list) = fresh {
            unsafe {
                SendMessageW(self.hwnd, TB_SETIMAGELIST, WPARAM(0), LPARAM(list.0));
                SendMessageW(
                    self.hwnd,
                    TB_SETBITMAPSIZE,
                    WPARAM(0),
                    LPARAM(((px << 16) | px) as isize),
                );
                // **The button size is the control's to choose.** Setting it to half the icon again
                // made the band enormous — "now the gap is huge", and he was right: comctl32 adds
                // the padding a Windows toolbar has, and a number invented here is a second
                // opinion about it that no shell toolbar shares.
            }
            if let Some(old) = self.images.replace(list) {
                unsafe {
                    let _ = ImageList_Destroy(old);
                }
            }
        }
        let iconic = self.images.is_some();

        let mut items: Vec<TBBUTTON> = Vec::with_capacity(buttons.len() + 4);
        for (at, button) in buttons.iter().enumerate() {
            if button.starts_group && !items.is_empty() {
                items.push(TBBUTTON {
                    iBitmap: 0,
                    idCommand: 0,
                    fsState: 0,
                    fsStyle: BTNS_SEP as u8,
                    bReserved: [0; 6],
                    dwData: 0,
                    iString: 0,
                });
            }
            // **Words when there are no glyphs.** If the icon font is missing a code point — or is
            // not installed at all — this falls back to the labelled buttons it replaced, because
            // a row of hollow boxes would be worse than the row this change set out to fix.
            let string = if iconic {
                0
            } else {
                let mut wide: Vec<u16> = button.label.encode_utf16().collect();
                wide.push(0);
                wide.push(0);
                let added = unsafe {
                    SendMessageW(
                        self.hwnd,
                        TB_ADDSTRINGW,
                        WPARAM(0),
                        LPARAM(wide.as_ptr() as isize),
                    )
                };
                // **`-1` means the pool refused the string, and it must not reach `iString`.**
                // comctl32 reads an out-of-range index as a *pointer*, so storing the failure
                // would have it dereference `0xFFFF_FFFF_FFFF_FFFF`.
                if added.0 < 0 {
                    0
                } else {
                    added.0
                }
            };
            items.push(TBBUTTON {
                iBitmap: if iconic { at as i32 } else { I_IMAGENONE },
                idCommand: button.id as i32,
                fsState: state_bits(button),
                fsStyle: (BTNS_AUTOSIZE
                    | if iconic { 0 } else { BTNS_SHOWTEXT }
                    | if button.toggle { BTNS_CHECK } else { 0 }) as u8,
                bReserved: [0; 6],
                dwData: 0,
                iString: string,
            });
        }

        let added = unsafe {
            let added = SendMessageW(
                self.hwnd,
                TB_ADDBUTTONSW,
                WPARAM(items.len()),
                LPARAM(items.as_ptr() as isize),
            );
            SendMessageW(self.hwnd, TB_AUTOSIZE, WPARAM(0), LPARAM(0));
            added.0 != 0
        };
        // **Measured after the buttons are in**, because the size the control wants is a fact about
        // what it holds. A failed insert must not be recorded as a full row either: `same_row`
        // would then be true for ever after and the band would stay reserved over an empty strip.
        self.measure();
        // And the band told what the buttons need, so the rebar sizes the toolbar to fit them.
        self.seat_band();
        added
    }

    /// The icon size for this window's DPI — sixteen pixels at 100 %, and proportionally more
    /// beyond it, which is what every shell toolbar does and what the owner's 150 % display needs.
    /// How tall the band must be, asked of the buttons the control actually laid out.
    ///
    /// **`TB_GETITEMRECT`, because this window positions the control itself.** The documentation is
    /// explicit that a toolbar sizes and positions itself from its buttons — *"the height is based
    /// on the height of the buttons in the toolbar"* — and that `CCS_NORESIZE` and
    /// `CCS_NOPARENTALIGN`, which turn that off, are what a **rebar-hosted** toolbar sets because
    /// the rebar does the sizing instead. This window has a tab strip above the toolbar, so it
    /// cannot let the control align itself to the top of the client area; it therefore takes on the
    /// sizing, and the honest way to do that is to ask where the buttons ended up.
    ///
    /// Measuring the control's own window instead is what broke the large icons: `place` had
    /// already sized that window to the previous band, so the answer was always the old height and
    /// the bigger buttons were simply clipped.
    fn measure(&mut self) {
        let mut tallest = 0;
        let count = unsafe { SendMessageW(self.hwnd, TB_BUTTONCOUNT, WPARAM(0), LPARAM(0)) }.0;
        for at in 0..count.max(0) {
            let mut rect = RECT::default();
            let got = unsafe {
                SendMessageW(
                    self.hwnd,
                    TB_GETITEMRECT,
                    WPARAM(at as usize),
                    LPARAM(&mut rect as *mut RECT as isize),
                )
            };
            if got.0 != 0 {
                tallest = tallest.max(rect.bottom);
            }
        }
        if tallest > 0 {
            // The buttons' own extent, plus the margin the control leaves under them.
            self.band = (tallest + BAND_MARGIN).clamp(1, 200);
        }
    }

    /// The icon size for this window's DPI, in the size the user asked for.
    ///
    /// Sixteen and twenty-four are the two sizes Windows toolbars have offered since Explorer had
    /// a Large Icons box, and the DPI scaling is on top of that: the owner's display is at 150 %,
    /// where small is 24 real pixels and large is 36.
    fn icon_px(&self) -> i32 {
        let dpi = unsafe { GetDpiForWindow(self.hwnd) };
        let dpi = if dpi == 0 { 96 } else { dpi };
        let logical = if self.large { 24 } else { 16 };
        (logical * dpi as i32) / 96
    }

    /// Chooses the icon size. The row is forgotten rather than resized, so the next frame rebuilds
    /// it: the bitmaps are drawn at a size and cannot be stretched without looking it.
    pub fn set_large(&mut self, large: bool) {
        if self.large != large {
            self.large = large;
            self.shown.clear();
        }
    }

    /// Answers `TBN_GETINFOTIPW` with the button's own tooltip text.
    ///
    /// **The text comes from the view-model, not from a second table.** The tooltip says the same
    /// words the menu says and names the same keys, because both read `Command::LISTED`; an
    /// icon-only toolbar whose tooltips drifted from the menu would be worse than no tooltips.
    pub fn info_tip(&self, lparam: LPARAM) -> bool {
        if lparam.0 == 0 {
            return false;
        }
        let tip = unsafe { &mut *(lparam.0 as *mut NMTBGETINFOTIPW) };
        let Some(button) = self
            .shown
            .iter()
            .find(|b| b.id as i32 == tip.iItem)
            .filter(|_| tip.cchTextMax > 1 && !tip.pszText.is_null())
        else {
            return false;
        };
        let mut text: Vec<u16> = button.tip.encode_utf16().collect();
        text.truncate(tip.cchTextMax as usize - 1);
        text.push(0);
        unsafe {
            std::ptr::copy_nonoverlapping(text.as_ptr(), tip.pszText.0, text.len());
        }
        true
    }

    /// How tall the control made itself, asked of the control.
    ///
    /// **Never a number chosen here.** The tab strip's band was once `chrome_h + 4.0`, and that
    /// four was precisely the "a bit small" the owner reported; a replacement invented in this file
    /// would be the same mistake with a different constant.
    ///
    /// **The height comes from the window, not from `TB_GETMAXSIZE`.** That message reports the
    /// extent of the *visible buttons and separators* — which is not the same thing as the height
    /// the control sized itself to at `TB_AUTOSIZE`, and reserving the smaller of the two clips the
    /// bottom row of button chrome. `rebuild` autosizes; this reads back what that produced.
    /// `TB_GETMAXSIZE` remains the fallback for the frame before any button exists.
    pub fn band_height(&self) -> i32 {
        // **The rebar's own answer when there is one**: it owns the layout, including the band
        // borders and the gripper, so its height is the only one that describes what is drawn.
        if let Some(rebar) = self.rebar {
            let tall = unsafe { SendMessageW(rebar, RB_GETBARHEIGHT, WPARAM(0), LPARAM(0)) }.0;
            if tall > 0 {
                return (tall as i32).clamp(1, 200);
            }
        }
        // What the buttons asked for at the last fill, which is the only measurement that survives
        // `place` having already sized the window to the previous band.
        if self.band > 0 {
            return self.band;
        }
        let mut rect = RECT::default();
        if unsafe { GetWindowRect(self.hwnd, &mut rect) }.is_ok() && rect.bottom > rect.top {
            return (rect.bottom - rect.top).clamp(1, 200);
        }
        let mut size = SIZE::default();
        let got = unsafe {
            SendMessageW(
                self.hwnd,
                TB_GETMAXSIZE,
                WPARAM(0),
                LPARAM(&mut size as *mut SIZE as isize),
            )
        };
        if got.0 == 0 {
            return 0;
        }
        size.cy.clamp(1, 200)
    }

    /// The control's window, so the shell can tell this control's `WM_COMMAND` from anyone else's.
    pub fn hwnd(&self) -> HWND {
        self.hwnd
    }

    /// Puts the control across the client area under the tab strip, or hides it.
    /// Puts the band of chrome where the frame wants it, and shows or hides it.
    ///
    /// **The rebar is what moves; the toolbar inside it never is.** The rebar sizes and positions
    /// its band's child, which is the whole reason for hosting the toolbar in one, and a
    /// `SetWindowPos` aimed at the toolbar would be this window arguing with the control again.
    pub fn place(&mut self, top: i32, width: i32, height: i32, visible: bool) {
        let outer = self.outer();
        if visible {
            unsafe {
                let _ = SetWindowPos(
                    outer,
                    HWND_TOP,
                    0,
                    top,
                    width.max(0),
                    height.max(0),
                    SWP_NOACTIVATE | SWP_NOZORDER,
                );
            }
            // The rebar lays its bands out on `WM_SIZE`; without this the band keeps the width it
            // was inserted with and the gripper sits at the wrong place after a resize.
            if self.rebar.is_some() {
                unsafe {
                    SendMessageW(outer, WM_SIZE, WPARAM(0), LPARAM(0));
                }
            }
        }
        if visible != self.visible {
            unsafe {
                let _ = ShowWindow(outer, if visible { SW_SHOWNA } else { SW_HIDE });
            }
            self.visible = visible;
        }
    }

    /// The window the frame positions: the rebar when there is one, the toolbar itself otherwise.
    fn outer(&self) -> HWND {
        self.rebar.unwrap_or(self.hwnd)
    }
}

impl Drop for Toolbar {
    /// **The window first, then the font**, which is the order `TabStrip` uses and the order that
    /// matters: the control has the `HFONT` selected for as long as it exists, and deleting a GDI
    /// object still selected into a live device context is undefined rather than merely untidy.
    fn drop(&mut self) {
        unsafe {
            // Destroying the rebar destroys the toolbar with it, as the documentation says: "When a
            // rebar control is destroyed, it destroys any child windows assigned to the bands".
            let _ = DestroyWindow(self.outer());
            if let Some(images) = self.images.take() {
                let _ = ImageList_Destroy(images);
            }
            if !self.font.is_invalid() {
                let _ = DeleteObject(HGDIOBJ(self.font.0));
            }
        }
    }
}

/// The `TBSTATE_*` bits for one button.
///
/// Pure, and separately, because a button that is pressed but disabled and one that is enabled but
/// not pressed differ by a single bit, and getting that wrong shows as a toolbar that looks almost
/// right.
pub fn state_bits(b: &ToolButton) -> u8 {
    let mut bits = 0;
    if b.enabled {
        bits |= TBSTATE_ENABLED;
    }
    if b.pressed {
        bits |= TBSTATE_CHECKED;
    }
    bits as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(buttons: &[ToolButton]) -> Vec<&str> {
        buttons.iter().map(|b| b.label).collect()
    }

    /// **§2.3's row, in §2.3's order.** Maximise is not in it: it is a window arrangement, not a
    /// document command, and the owner asked for the window controls to stay out of the toolbar. The order is the
    /// requirement, not a preference: the
    /// document names these nine and this sequence, and a toolbar is a thing people reach for by
    /// position after the first week.
    #[test]
    fn the_row_is_the_buttons_the_design_names_in_its_order() {
        assert_eq!(
            labels(&toolbar_of(None)),
            [
                "Open", "Find", "Filter", "Follow", "Collapse", "Detail", "Rules", "Format",
                "Export"
            ]
        );
    }

    /// **With nothing open the row is still there.** A toolbar that appeared with the first file
    /// would shift the grid down underneath the pointer at the least welcome moment, and §2.3 says
    /// an unavailable command is "disabled in place".
    #[test]
    fn an_empty_window_greys_the_row_rather_than_removing_it() {
        let empty = toolbar_of(None);
        assert_eq!(empty.len(), 9, "the row does not shrink");
        let live: Vec<&str> = empty
            .iter()
            .filter(|b| b.enabled)
            .map(|b| b.label)
            .collect();
        assert_eq!(
            live,
            ["Open", "Rules"],
            "only the two commands that do not need a document"
        );
        assert!(
            empty.iter().all(|b| !b.pressed),
            "nothing can be on when nothing is open"
        );
    }

    /// The toggles are marked as toggles and the verbs are not — the distinction the
    /// Win32 half turns into `BTNS_CHECK`. A verb given a pressed state would latch down on click
    /// and stay there.
    #[test]
    fn exactly_the_state_buttons_are_toggles() {
        let toggles: Vec<&str> = toolbar_of(None)
            .iter()
            .filter(|b| b.toggle)
            .map(|b| b.label)
            .collect();
        assert_eq!(toggles, ["Filter", "Follow", "Collapse", "Detail"]);
    }

    /// **Every id is the menu's.** This is the test that keeps §1.2's one-command-one-path rule
    /// true: if a button ever grew an id of its own, a click would stop reaching the dispatch the
    /// menu and the keystroke use, and the two surfaces could drift apart in silence.
    #[test]
    fn every_button_carries_the_command_id_the_menu_sends() {
        let expected = [
            Command::OpenFile,
            Command::Find,
            Command::ToggleFilters,
            Command::FollowTail,
            Command::ToggleCollapse,
            Command::ToggleDetail,
            Command::EditRules,
            Command::DefineFormat,
            Command::Export,
        ];
        for (button, command) in toolbar_of(None).iter().zip(expected) {
            assert_eq!(
                button.id,
                command_id(command),
                "{} must post the menu's id",
                button.label
            );
        }
    }

    /// **Enabled and pressed are two independent bits**, and every combination has to survive the
    /// trip into `TBSTATE_*`. A toggle that is on but cannot act — Follow on a document that has
    /// just been closed — must still read as on, or the row lies about the state it exists to
    /// display.
    #[test]
    fn the_two_state_bits_are_independent() {
        let button = |enabled, pressed| ToolButton {
            label: "x",
            icon: icon::FIND,
            tip: "x".to_owned(),
            id: 1,
            enabled,
            toggle: true,
            pressed,
            starts_group: false,
        };
        assert_eq!(state_bits(&button(false, false)), 0);
        assert_eq!(state_bits(&button(true, false)), TBSTATE_ENABLED as u8);
        assert_eq!(state_bits(&button(false, true)), TBSTATE_CHECKED as u8);
        assert_eq!(
            state_bits(&button(true, true)),
            (TBSTATE_ENABLED | TBSTATE_CHECKED) as u8
        );
    }

    /// `I_IMAGENONE` is hand-declared because the crate does not bind it, so it is checked rather
    /// than trusted — the WinHTTP lesson, applied to the one number this module invents. A button
    /// given image zero instead of none reserves space for a bitmap that is not there.
    #[test]
    fn the_one_hand_declared_constant_is_the_documented_value() {
        assert_eq!(I_IMAGENONE, -2);
        assert_ne!(
            I_IMAGENONE, 0,
            "image zero is a real image, not the absence"
        );
    }

    /// No two buttons share an id, because the shell dispatches on the id alone and a duplicate
    /// would make one of the pair unreachable while looking perfectly correct on screen.
    #[test]
    fn no_two_buttons_share_an_id() {
        let mut ids: Vec<u32> = toolbar_of(None).iter().map(|b| b.id).collect();
        let before = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), before);
    }
}
