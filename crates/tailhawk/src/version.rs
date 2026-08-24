//! What build this is, and where to read it from.
//!
//! The number is stamped into the executable's PE **version resource** by `build.rs`, which is the
//! standard place a Windows program keeps it: Explorer's Properties → Details shows it without the
//! program running, an installer can compare it, and any tool can read it with
//! `GetFileVersionInfo` rather than having to launch the binary and ask.
//!
//! The scheme is the owner's, shared with their other products: **`YYYY.M.D.<revision>`**, the date
//! in the first three fields and a unique integer in the fourth. The revision is seconds elapsed in
//! the UTC day **divided by two** — every field of a version resource is a `u16` and so tops out at
//! 65535, which a raw second count (86400 in a day) overflows after 18:12. Halving fits the whole
//! day into 0..=43199 at two-second resolution, and is the same convention MSBuild's auto-revision
//! uses.
//!
//! **The problem this was built to solve** is recorded in `docs/HANDOFF.md`: a rename shipped, CI
//! went green, the dogfood instance was restarted — and the window still showed the old label,
//! because the release binary had never been rebuilt. Nothing anywhere could say whether a running
//! Tailhawk was the current one. Now `tools/dogfood.sh` can ask the file.

/// The version this binary was stamped with, as `build.rs` computed it.
///
/// Compiled in rather than read back from the file, because a program that has to open itself to
/// know what it is has a second way to be wrong.
pub const VERSION: &str = env!("TAILHAWK_VERSION");

#[cfg(test)]
mod tests {
    use super::*;
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::{
        GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW,
    };

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// The whole point, and the reason the resource is hand-built rather than taken from a crate:
    /// **Windows itself parses what `build.rs` wrote.**
    ///
    /// `VS_VERSIONINFO` is a nest of four-byte-aligned, UTF-16, length-prefixed structures in which
    /// a single miscounted length yields a resource that every tool silently declines to read. No
    /// amount of inspecting the bytes we wrote proves they are right — only asking the API the
    /// resource exists to serve does, which is `GetFileVersionInfo`.
    #[test]
    fn windows_reads_the_version_back_out_of_this_executable() {
        let exe = std::env::current_exe().expect("a test binary has a path");
        let path = wide(&exe.to_string_lossy());

        // SAFETY: `path` is a NUL-terminated wide string that outlives the call.
        let size = unsafe { GetFileVersionInfoSizeW(PCWSTR(path.as_ptr()), None) };
        assert!(
            size > 0,
            "Windows found no version resource in {} — the .res was not linked, or is malformed",
            exe.display()
        );

        let mut buffer = vec![0u8; size as usize];
        // SAFETY: `buffer` is exactly the size Windows just asked for.
        unsafe { GetFileVersionInfoW(PCWSTR(path.as_ptr()), 0, size, buffer.as_mut_ptr().cast()) }
            .expect("Windows could not read the version resource it just sized");

        let query = wide(r"\StringFileInfo\040904B0\FileVersion");
        let mut value: *mut core::ffi::c_void = std::ptr::null_mut();
        let mut len: u32 = 0;
        // SAFETY: `buffer` holds the block Windows filled; `query` is NUL-terminated; `value` and
        // `len` receive a pointer into `buffer` and its length in UTF-16 units.
        let ok = unsafe {
            VerQueryValueW(
                buffer.as_ptr().cast(),
                PCWSTR(query.as_ptr()),
                &mut value,
                &mut len,
            )
        };
        assert!(
            ok.as_bool() && !value.is_null() && len > 0,
            "the resource parsed but carries no FileVersion string"
        );

        // SAFETY: `value` points into `buffer` at `len` UTF-16 units, including the terminator.
        let units = unsafe { std::slice::from_raw_parts(value.cast::<u16>(), len as usize) };
        let read = String::from_utf16_lossy(units)
            .trim_end_matches('\0')
            .to_owned();

        assert_eq!(
            read, VERSION,
            "the stamp in the file disagrees with the constant compiled beside it"
        );
    }

    /// The comctl32 v6 manifest, read back the way the version stamp is: by asking Windows.
    ///
    /// `build.rs` writes it as `RT_MANIFEST` id 1 in the same `.res`. Without it
    /// `TaskDialogIndirect` refuses to open and every other native dialog draws unthemed — so a
    /// build whose manifest went missing must fail here, not in front of the owner's Help menu.
    #[test]
    fn the_executable_carries_the_common_controls_manifest() {
        use windows::Win32::System::LibraryLoader::{
            FindResourceW, GetModuleHandleW, LoadResource, LockResource, SizeofResource,
        };
        const RT_MANIFEST: usize = 24;
        const MANIFEST_ID: usize = 1;
        let module = unsafe { GetModuleHandleW(None) }.expect("a process has a module handle");
        // SAFETY: ordinals passed as `MAKEINTRESOURCE` pointers, the documented convention.
        let found = unsafe {
            FindResourceW(
                module,
                PCWSTR(MANIFEST_ID as *const u16),
                PCWSTR(RT_MANIFEST as *const u16),
            )
        };
        assert!(
            !found.is_invalid(),
            "no RT_MANIFEST in this executable — the .res lost its manifest entry"
        );
        let size = unsafe { SizeofResource(module, found) } as usize;
        let data = unsafe { LoadResource(module, found) }.expect("a found resource loads");
        let bytes = unsafe { LockResource(data) };
        assert!(!bytes.is_null() && size > 0);
        // SAFETY: `bytes` points at `size` bytes of the mapped resource, which lives as long as
        // the module.
        let text =
            String::from_utf8_lossy(unsafe { std::slice::from_raw_parts(bytes.cast(), size) })
                .into_owned();
        assert!(
            text.contains("Microsoft.Windows.Common-Controls"),
            "the manifest is present but does not declare the common-controls dependency"
        );
        assert!(
            text.contains("6.0.0.0"),
            "the dependency is not pinned to comctl32 v6"
        );
    }

    /// The shape the owner's other builds use: the date, then a revision inside the day.
    #[test]
    fn the_version_is_a_date_and_a_revision_that_fit_a_version_field() {
        let parts: Vec<&str> = VERSION.split('.').collect();
        assert_eq!(parts.len(), 4, "expected YYYY.M.D.revision, got {VERSION}");

        let numbers: Vec<u32> = parts
            .iter()
            .map(|p| p.parse().unwrap_or_else(|_| panic!("{p} is not a number")))
            .collect();

        assert!(
            (2026..=2100).contains(&numbers[0]),
            "year {} is not plausible",
            numbers[0]
        );
        assert!((1..=12).contains(&numbers[1]), "month {}", numbers[1]);
        assert!((1..=31).contains(&numbers[2]), "day {}", numbers[2]);
        // The reason for halving the seconds: every field has to survive the trip through a `u16`.
        assert!(
            numbers[3] <= 43_199,
            "revision {} exceeds half a day of seconds",
            numbers[3]
        );
        for (i, n) in numbers.iter().enumerate() {
            assert!(*n <= 65_535, "field {i} ({n}) overflows a version field");
        }
    }
}
