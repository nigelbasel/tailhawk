//! The application icon — the mark the taskbar, Alt+Tab and the title bar show.
//!
//! **Carried as images in the binary, not as a linker resource.** The usual way to give a Windows
//! program an icon is a `.rc` compiled by a resource compiler and handed to the linker, which
//! means a `build.rs` and a build-time dependency. This project gates its dependency tree with
//! `cargo deny` and its binary with a size test, and the icon is wanted for the *taskbar* — so the
//! images are embedded with [`include_bytes!`] and turned into `HICON`s at start-up instead. The
//! whole set costs about 11 KB against a 15 MB gate.
//!
//! The one thing this does not buy is Explorer's icon for `tailhawk.exe` on disk, which is read
//! from a linker resource and nothing else. `assets/tailhawk.ico` is written by
//! `tools/make-icon.ps1` alongside these, ready for M9's installer to point at when that arrives.
//!
//! The artwork itself is generated: `tools/make-icon.ps1` draws it, so a change of palette is a
//! change of two constants rather than an export from a drawing program nobody has.

use windows::Win32::UI::WindowsAndMessaging::{
    CreateIconFromResourceEx, GetSystemMetrics, HICON, IMAGE_FLAGS, LR_DEFAULTCOLOR, SM_CXICON,
    SM_CXSMICON, SM_CYICON, SM_CYSMICON,
};

/// The icon at each size it was cut for, ascending. Windows will scale whichever is handed to it,
/// but a 16 px icon scaled from 256 is mud — the small sizes are drawn at their own size, with the
/// corner radius and inset re-proportioned, so each is crisp rather than resampled.
const IMAGES: &[(i32, &[u8])] = &[
    (16, include_bytes!("../../../assets/icon-16.png")),
    (24, include_bytes!("../../../assets/icon-24.png")),
    (32, include_bytes!("../../../assets/icon-32.png")),
    (48, include_bytes!("../../../assets/icon-48.png")),
    (64, include_bytes!("../../../assets/icon-64.png")),
    (256, include_bytes!("../../../assets/icon-256.png")),
];

/// The version stamp `CreateIconFromResourceEx` wants: 3.0, the only value it accepts.
const ICON_RESOURCE_VERSION: u32 = 0x0003_0000;

/// The image cut closest to `size` without going under it, so scaling is always *down*. Scaling a
/// small image up is what makes an icon look soft on a high-DPI display.
fn best_fit(size: i32) -> &'static [u8] {
    IMAGES
        .iter()
        .find(|(cut, _)| *cut >= size)
        .or_else(|| IMAGES.last())
        .map(|(_, png)| *png)
        .expect("IMAGES is never empty")
}

/// The icon at `size` square, or `None` if Windows would not build one.
///
/// A missing icon is **not** an error worth refusing to start over: the window is perfectly usable
/// with the system default, so every caller treats this as best-effort.
fn at(size: i32) -> Option<HICON> {
    let png = best_fit(size);
    // SAFETY: `png` is a `'static` slice of the binary's own read-only data and outlives the call.
    // Windows copies the bits it needs; the `HICON` afterwards owns no borrow of the slice. PNG is
    // an accepted icon-resource encoding from Vista onward, and `SPEC.md` §2.1 scopes this to
    // Windows 10 1809+.
    unsafe {
        CreateIconFromResourceEx(
            png,
            windows::Win32::Foundation::BOOL(1),
            ICON_RESOURCE_VERSION,
            size,
            size,
            IMAGE_FLAGS(LR_DEFAULTCOLOR.0),
        )
    }
    .ok()
}

/// The pair the shell sets on its window: the large icon for the taskbar and Alt+Tab, and the
/// small one for the title bar.
///
/// Both are asked for at the size **this system** reports rather than at 32 and 16, so a
/// 150%-scaled display gets a 48 px icon cut from the 48 px drawing instead of a blown-up 32.
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

    /// Every embedded image is a PNG, and the set is ascending — `best_fit` walks it in order and
    /// would silently pick the wrong cut if it were not sorted.
    #[test]
    fn the_embedded_images_are_pngs_in_ascending_order() {
        const PNG_MAGIC: &[u8] = &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        let mut previous = 0;
        for (size, bytes) in IMAGES {
            assert!(
                bytes.starts_with(PNG_MAGIC),
                "the {size} px icon is not a PNG"
            );
            assert!(*size > previous, "IMAGES is not ascending at {size}");
            previous = *size;
        }
    }

    /// The size actually asked for is never scaled up: the cut chosen is at least as big.
    #[test]
    fn a_requested_size_is_never_scaled_up_from_a_smaller_cut() {
        for asked in [16, 17, 20, 24, 31, 32, 40, 48, 64, 96, 128, 256] {
            let png = best_fit(asked);
            let cut = IMAGES
                .iter()
                .find(|(_, bytes)| std::ptr::eq(*bytes, png))
                .map(|(size, _)| *size)
                .expect("best_fit returns one of IMAGES");
            assert!(
                cut >= asked,
                "{asked} px would be scaled up from the {cut} px cut"
            );
        }
    }

    /// Past the largest cut there is nothing to grow into, so the largest is what comes back
    /// rather than nothing at all.
    #[test]
    fn a_size_past_the_largest_cut_falls_back_to_it() {
        let (largest, bytes) = IMAGES.last().copied().expect("IMAGES is never empty");
        assert_eq!(largest, 256);
        assert!(std::ptr::eq(best_fit(512), bytes));
    }

    /// The real thing: Windows builds an icon from the embedded bits. This is what proves the PNG
    /// encoding is one `CreateIconFromResourceEx` accepts — a claim no amount of byte-checking
    /// makes, and the whole reason the linker-resource route was skipped.
    #[test]
    fn windows_builds_an_icon_from_the_embedded_bits() {
        for size in [16, 32, 48] {
            assert!(
                at(size).is_some(),
                "Windows refused to build the {size} px icon from its embedded PNG"
            );
        }
    }
}
