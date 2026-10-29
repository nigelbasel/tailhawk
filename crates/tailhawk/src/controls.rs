//! The shared standard-controls module — the owner's directive, 2026-08-25: "implement a
//! standard set of classes that replicate the aspects of the WinUI libraries that you can just
//! use. This redrawing is eating time, and is wrong every time I look at it."
//!
//! Every surface is a native control since the filter panel became one on 2026-09-15, so what is
//! left here is the one decision they all share: which theme a control is dressed in.

/// Themes a native child control to match the window's menus — **one decision for every surface**.
///
/// The owner's screenshot of 2026-09-03 had a light native menu bar sitting on a dark toolbar
/// strip, and read it as a second, broken menu. Each control had been handed `DarkMode_Explorer`
/// unconditionally while the menus took the theme setting through `darkmode.rs` — and on a machine
/// where that best-effort ordinal call does nothing, the two disagree. So the controls follow the
/// same `dark` the frame is dressed with, here and nowhere else: a toolbar, a tab strip and a status
/// bar that are all the system's own colours, or all its dark ones, but never a mixture.
///
/// Best effort, like everything about theming a common control: a refusal leaves the control in
/// the system's default look, which is a standard Windows control and exactly what §1.1 asks for.
pub fn apply_theme(hwnd: windows::Win32::Foundation::HWND, dark: bool) {
    use windows::core::w;
    apply_theme_class(hwnd, dark, w!("DarkMode_Explorer"), w!("Explorer"));
}

/// Dresses a window **and every control inside it**, each in the class its own kind wants.
///
/// **One class for all of them does not work**, which is why this dispatches. An edit or a combo
/// handed `DarkMode_Explorer` keeps a white field; the class that darkens a field's ground is
/// `DarkMode_CFD`, the one the common file dialog uses. A list view needs `DarkMode_ItemsView`
/// *and* its three colours set — the class dresses the scroll bars and the selection, not the
/// ground, so a report list stays white without them. Statics are not themed at all: they are
/// painted by their parent, through `WM_CTLCOLORSTATIC`.
///
/// The colours are passed in as `COLORREF`s rather than read here, so this module stays clear of
/// the theme's own types — and so there is no third copy of `colourref` in the tree.
pub fn apply_theme_tree(
    hwnd: windows::Win32::Foundation::HWND,
    dark: bool,
    list_bg: u32,
    list_ink: u32,
) {
    use windows::Win32::Foundation::LPARAM;
    use windows::Win32::UI::WindowsAndMessaging::EnumChildWindows;
    apply_theme(hwnd, dark);
    // Packed into one `LPARAM` because `EnumChildWindows` carries exactly one, and a pointer to a
    // local would outlive nothing — the enumeration is synchronous and finishes before this does.
    let packed = Box::into_raw(Box::new((dark, list_bg, list_ink)));
    unsafe {
        let _ = EnumChildWindows(hwnd, Some(dress_child), LPARAM(packed as isize));
        drop(Box::from_raw(packed));
    }
}

unsafe extern "system" fn dress_child(
    hwnd: windows::Win32::Foundation::HWND,
    lparam: windows::Win32::Foundation::LPARAM,
) -> windows::Win32::Foundation::BOOL {
    use windows::core::w;
    use windows::Win32::Foundation::{BOOL, LPARAM, WPARAM};
    use windows::Win32::UI::Controls::{
        SetWindowTheme, LVM_SETBKCOLOR, LVM_SETTEXTBKCOLOR, LVM_SETTEXTCOLOR, TVM_SETBKCOLOR,
        TVM_SETLINECOLOR, TVM_SETTEXTCOLOR,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        GetClassNameW, GetWindowLongPtrW, SendMessageW, GWL_STYLE,
    };

    let (dark, list_bg, list_ink) = unsafe { *(lparam.0 as *const (bool, u32, u32)) };
    let mut buffer = [0u16; 64];
    let written = unsafe { GetClassNameW(hwnd, &mut buffer) };
    let class = String::from_utf16_lossy(&buffer[..written.max(0) as usize]);
    match class.as_str() {
        "Edit" | "ComboBox" => apply_theme_class(hwnd, dark, w!("DarkMode_CFD"), w!("CFD")),
        "SysListView32" => {
            apply_theme_class(hwnd, dark, w!("DarkMode_ItemsView"), w!("ItemsView"));
            unsafe {
                SendMessageW(hwnd, LVM_SETBKCOLOR, WPARAM(0), LPARAM(list_bg as isize));
                SendMessageW(
                    hwnd,
                    LVM_SETTEXTBKCOLOR,
                    WPARAM(0),
                    LPARAM(list_bg as isize),
                );
                SendMessageW(hwnd, LVM_SETTEXTCOLOR, WPARAM(0), LPARAM(list_ink as isize));
            }
        }
        // **A tree view needs its colours told to it, exactly as a list view does.** The class is
        // `Explorer`'s — which is right here, because Explorer's own navigation pane *is* a tree
        // view dressed that way — but the class alone leaves the ground white, so the catch-all
        // below would have given the Remote sources tree black text on white inside a dark dialog.
        // That is the defect a review found in the source editor on 2026-10-21, and this control is
        // the first `SysTreeView32` in the product: the arm goes in with it rather than after a
        // screenshot catches it.
        "SysTreeView32" => {
            apply_theme(hwnd, dark);
            unsafe {
                SendMessageW(hwnd, TVM_SETBKCOLOR, WPARAM(0), LPARAM(list_bg as isize));
                SendMessageW(hwnd, TVM_SETTEXTCOLOR, WPARAM(0), LPARAM(list_ink as isize));
                // The expand/collapse glyphs come from the theme and the lines do not, so a
                // dark tree draws its own hairlines over the dark ground unless they take the
                // ink colour too.
                SendMessageW(hwnd, TVM_SETLINECOLOR, WPARAM(0), LPARAM(list_ink as isize));
            }
        }
        // **A list view's header is a descendant, and needs the list's class, not Explorer's.**
        // `EnumChildWindows` walks the whole tree rather than the immediate children, so the
        // `SysHeader32` inside a report-view list arrives here — and the catch-all below would
        // hand it `DarkMode_Explorer`, which [`apply_theme_class`]'s own note describes as leaving
        // the band white with black text over a dark grid. `header.rs` and `main.rs` already make
        // this exception for the grid's header; a review found the five dialogs with list views
        // reproducing the defect it was written to fix.
        "SysHeader32" => apply_theme_class(hwnd, dark, w!("DarkMode_ItemsView"), w!("ItemsView")),
        // **Statics fall through to `Explorer` and take no visible harm from it** — what actually
        // colours them is their parent's `WM_CTLCOLORSTATIC` answer.
        //
        // **Buttons take `Explorer`, and for a check box and a push button it is enough** — both
        // come back dark with readable text. That is worth saying because the expectation was the
        // opposite: the menu bar's note in `docs/HANDOFF.md` records that Win32 offers no
        // supported route to a dark menu, and the same was assumed of a push button. It is not so
        // here, because `darkmode.rs` has already called `SetPreferredAppMode(ForceDark)` before
        // any window exists, which is what lets comctl32 dress them. Screenshotted 2026-09-21.
        // A radio button is the exception [`button_theme`] makes: the owner's dialog of
        // 2026-10-28 had dark radio glyphs beside black labels.
        "Button" => {
            let style = unsafe { GetWindowLongPtrW(hwnd, GWL_STYLE) } as u32;
            if button_keeps_theme(style, dark) {
                apply_theme(hwnd, dark);
            } else {
                unsafe {
                    let _ = SetWindowTheme(hwnd, w!(""), w!(""));
                }
            }
        }
        _ => apply_theme(hwnd, dark),
    }
    BOOL(1)
}

const BS_TYPEMASK: u32 = 0x000F;
const BS_RADIOBUTTON: u32 = 0x0004;
const BS_AUTORADIOBUTTON: u32 = 0x0009;
const BS_PUSHLIKE: u32 = 0x1000;

/// Whether a `Button` child keeps a visual style, or paints classic so its label takes the ink
/// its parent answers `WM_CTLCOLORSTATIC` with.
///
/// **A radio button in the dark theme takes no visual style**, because `DarkMode_Explorer`
/// dresses its glyph dark and leaves its label the theme's black — the owner's Filters dialog,
/// 2026-10-28: *"the radio buttons are visible, but the labels are not. the check box labels are
/// all visible."* Without a style the control paints classic and takes the ink its parent
/// answers `WM_CTLCOLORSTATIC` with, which is how every static in the dialog is already readable.
/// A check box and a push button come back dark with readable text under the class, so they keep
/// it, and so does a push-like radio, which is drawn as a push button; in the light theme every
/// button keeps the system's own look.
///
/// **Knowingly left**: the classic glyph is drawn in the system colours, so a dark dialog shows
/// a light radio circle beside its dark check boxes; and a disabled classic radio embosses its
/// label in the system's greys rather than the ink, which is legible on the dark ground and is
/// not the theme's grey.
pub fn button_keeps_theme(style: u32, dark: bool) -> bool {
    let radio = matches!(style & BS_TYPEMASK, BS_RADIOBUTTON | BS_AUTORADIOBUTTON)
        && style & BS_PUSHLIKE == 0;
    !dark || !radio
}

/// The same, for a control whose dark class is not `Explorer`'s.
///
/// **A header is the case that forced this.** `SetWindowTheme(header, "DarkMode_Explorer")` leaves
/// the band white with black text over a dark grid — the class exists, so the call succeeds and
/// changes nothing worth having. The list classes are what a header follows: `ItemsView` light and
/// `DarkMode_ItemsView` dark, which is the pair Explorer's own file list uses.
pub fn apply_theme_class(
    hwnd: windows::Win32::Foundation::HWND,
    dark: bool,
    when_dark: windows::core::PCWSTR,
    when_light: windows::core::PCWSTR,
) {
    use windows::core::PCWSTR;
    use windows::Win32::UI::Controls::SetWindowTheme;
    let name = if dark { when_dark } else { when_light };
    unsafe {
        let _ = SetWindowTheme(hwnd, name, PCWSTR::null());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BS_PUSHBUTTON: u32 = 0x0000;
    const BS_AUTOCHECKBOX: u32 = 0x0003;
    const BS_GROUPBOX: u32 = 0x0007;
    const WS_TABSTOP: u32 = 0x0001_0000;
    const WS_GROUP: u32 = 0x0002_0000;

    /// A radio button's label was black on the dark dialog while the check boxes beside it read
    /// fine: the dark Explorer class has no readable radio part, so a radio takes no style at all
    /// in the dark theme and paints with the ink its parent answers. Everything else keeps its
    /// class — a push-like radio is a push button to the theme — and the light theme is untouched.
    #[test]
    fn a_radio_button_takes_no_visual_style_in_the_dark_theme_and_nothing_else_changes() {
        for radio in [
            WS_TABSTOP | WS_GROUP | BS_AUTORADIOBUTTON,
            BS_AUTORADIOBUTTON,
            BS_RADIOBUTTON,
        ] {
            assert!(!button_keeps_theme(radio, true), "style {radio:#x}");
            assert!(button_keeps_theme(radio, false), "style {radio:#x}");
        }
        for other in [
            WS_TABSTOP | BS_AUTOCHECKBOX,
            WS_TABSTOP | BS_PUSHBUTTON,
            WS_GROUP | BS_GROUPBOX,
            WS_TABSTOP | BS_PUSHLIKE | BS_AUTORADIOBUTTON,
        ] {
            assert!(button_keeps_theme(other, true), "style {other:#x}");
            assert!(button_keeps_theme(other, false), "style {other:#x}");
        }
    }
}
