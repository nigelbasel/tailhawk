//! The application icon — the mark the taskbar, Alt+Tab, the title bar and Explorer show.
//!
//! **One copy, in the PE resource, and everything reads it from there.** `build.rs` parses
//! `assets/tailhawk.ico` into `RT_ICON` images and an `RT_GROUP_ICON` directory naming them, which
//! is the arrangement Windows itself defines: the shell picks the numerically lowest group as the
//! executable's icon, so Explorer shows the hawk on the file without the program running, and
//! `LoadImageW` gives the running window whichever cut fits the metric it asks for.
//!
//! **This module used to embed the artwork a second time** with `include_bytes!`, building icons at
//! start-up with `CreateIconFromResourceEx`. That existed because the icon came before `build.rs`
//! did, and a linker resource then meant a resource compiler or a crate with 23 transitive
//! dependencies. Once the version stamp needed a `.res` anyway the argument was spent, and carrying
//! the same eleven kilobytes twice — with two ways for the window and the file to disagree about
//! what the program looks like — was the worse half of the trade.
//!
//! The artwork itself is generated: `tools/make-icon.ps1` draws it, so a change of palette is a
//! change of two constants rather than an export from a drawing program nobody has.

use windows::core::PCWSTR;
use windows::Win32::Foundation::HINSTANCE;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    GetSystemMetrics, LoadImageW, HICON, IMAGE_ICON, LR_DEFAULTCOLOR, SM_CXICON, SM_CXSMICON,
    SM_CYICON, SM_CYSMICON,
};

/// The `RT_GROUP_ICON` id `build.rs` writes. The two have to agree, and there is no way for the
/// compiler to check it — the resource is data in a file the linker was handed.
const ICON_GROUP_ID: u16 = 1;

/// The icon at `size` square, or `None` if Windows would not build one.
///
/// A missing icon is **not** an error worth refusing to start over: the window is perfectly usable
/// with the system default, so every caller treats this as best-effort.
fn at(size: i32) -> Option<HICON> {
    // **The module handle, not `NULL`.** `LoadImageW` given a null instance searches the *system's*
    // OEM resources rather than this executable, and answers "not found" for an icon that is
    // plainly linked in. That is exactly how the first draft of this failed, and the failure says
    // nothing about the cause.
    // SAFETY: asking for this process's own module cannot fail.
    let module: HINSTANCE = unsafe { GetModuleHandleW(None) }
        .map(Into::into)
        .unwrap_or_default();
    // SAFETY: `module` is this executable, where the linker put the resource. Casting the id to a
    // pointer is `MAKEINTRESOURCE` — the documented way to name a resource by ordinal. `LoadImageW`
    // copies what it needs, so nothing is borrowed past the call. `LR_SHARED` is deliberately *not*
    // passed: a shared handle must never be destroyed, and these are the window's to own.
    let handle = unsafe {
        LoadImageW(
            module,
            PCWSTR(ICON_GROUP_ID as usize as *const u16),
            IMAGE_ICON,
            size,
            size,
            LR_DEFAULTCOLOR,
        )
    }
    .ok()?;
    Some(HICON(handle.0))
}

/// The pair the shell sets on its window: the large icon for the taskbar and Alt+Tab, and the
/// small one for the title bar.
///
/// Both are asked for at the size **this system** reports rather than at 32 and 16, so a
/// 150%-scaled display gets the 48 px cut instead of a blown-up 32.
pub fn window_icons() -> (Option<HICON>, Option<HICON>) {
    // SAFETY: `GetSystemMetrics` reads a system constant and cannot fail in a way that matters.
    let (big, small) = unsafe {
        (
            GetSystemMetrics(SM_CXICON).max(GetSystemMetrics(SM_CYICON)),
            GetSystemMetrics(SM_CXSMICON).max(GetSystemMetrics(SM_CYSMICON)),
        )
    };
    (at(big.max(16)), at(small.max(16)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **Windows finds the group `build.rs` wrote, and builds an icon from it.**
    ///
    /// This is the claim no amount of checking our own bytes can make. An `RT_GROUP_ICON` whose
    /// entries are the wrong width — the `.ico` file's sixteen-byte directory entry copied across
    /// instead of the resource's fourteen-byte one, which is the obvious mistake — parses as
    /// garbage and yields nothing, silently. Asking `LoadImageW` is what proves the layout.
    #[test]
    fn windows_builds_an_icon_from_the_linked_resource() {
        for size in [16, 24, 32, 48, 64, 256] {
            assert!(
                at(size).is_some(),
                "Windows found no icon at {size} px in this executable's resources"
            );
        }
    }

    /// The window's pair are both real, at whatever sizes this system asks for.
    #[test]
    fn the_window_gets_both_a_large_and_a_small_icon() {
        let (big, small) = window_icons();
        assert!(big.is_some(), "no large icon for the taskbar");
        assert!(small.is_some(), "no small icon for the title bar");
    }
}
