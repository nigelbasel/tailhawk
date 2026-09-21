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
    use windows::Win32::UI::Controls::{LVM_SETBKCOLOR, LVM_SETTEXTBKCOLOR, LVM_SETTEXTCOLOR};
    use windows::Win32::UI::WindowsAndMessaging::{GetClassNameW, SendMessageW};

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
        // **Buttons take `Explorer`, and it is enough** — check boxes, radio buttons *and* push
        // buttons all come back dark. That is worth saying because the expectation was the
        // opposite: the menu bar's note in `docs/HANDOFF.md` records that Win32 offers no
        // supported route to a dark menu, and the same was assumed of a push button. It is not so
        // here, because `darkmode.rs` has already called `SetPreferredAppMode(ForceDark)` before
        // any window exists, which is what lets comctl32 dress them. Screenshotted 2026-09-21.
        _ => apply_theme(hwnd, dark),
    }
    BOOL(1)
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
