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
