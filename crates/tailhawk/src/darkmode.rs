//! Dark mode for the chrome **Windows** draws — the menus above all.
//!
//! Since §2.2 was resettled to native menus, the app no longer paints the bar, and a theme change
//! that repaints everything the app owns leaves the one surface it does not own still white. There
//! is no documented way to colour an `HMENU`: the only supported alternative is `MF_OWNERDRAW`,
//! which is drawing the menus ourselves again — the thing the owner overruled — and which looks
//! native on no version of Windows.
//!
//! So this module reaches for the undocumented `uxtheme.dll` exports every dark-mode Win32
//! application uses. They have no names; they are looked up **by ordinal**, which is why the calls
//! are gathered here and nowhere else. `CLEANROOM.md` carries the decision and its limits.
//!
//! **Everything here is best-effort.** A missing module, a missing ordinal, a refusing call: the
//! menus stay in the system's colours, which is a perfectly usable menu. Nothing in this module
//! can fail a caller.

use std::sync::OnceLock;

use windows::core::{PCSTR, PCWSTR};
use windows::Win32::Foundation::{BOOL, HMODULE, HWND};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};

/// `#135`'s argument on 1903 and later, where it is `SetPreferredAppMode`.
///
/// **Only `Default` and `ForceDark` are ever passed, and that is a correctness decision rather
/// than a shortage of ambition.** On 1809 the same ordinal is `AllowDarkModeForApp(BOOL)`, and the
/// app supports 1809 per `SPEC.md` §2.1. `Default = 0` reads as `FALSE` and `ForceDark = 2` reads
/// as truthy, so both are right under either reading — where `ForceLight = 3` would read as
/// "dark, please" on the older build, which is the opposite of what it says.
const APP_MODE_DEFAULT: i32 = 0;
const APP_MODE_FORCE_DARK: i32 = 2;

const ORDINAL_ALLOW_DARK_MODE_FOR_WINDOW: usize = 133;
const ORDINAL_SET_PREFERRED_APP_MODE: usize = 135;
const ORDINAL_REFRESH_IMMERSIVE_COLOR_POLICY_STATE: usize = 104;
const ORDINAL_FLUSH_MENU_THEMES: usize = 136;

type SetPreferredAppModeFn = unsafe extern "system" fn(i32) -> i32;
type AllowDarkModeForWindowFn = unsafe extern "system" fn(HWND, BOOL) -> BOOL;
type NoArgsFn = unsafe extern "system" fn();

/// The four entry points, resolved once. `None` for any the running Windows does not export —
/// each is used independently, so a build missing one still gets the others.
struct Uxtheme {
    set_preferred_app_mode: Option<SetPreferredAppModeFn>,
    allow_dark_mode_for_window: Option<AllowDarkModeForWindowFn>,
    refresh_immersive_color_policy_state: Option<NoArgsFn>,
    flush_menu_themes: Option<NoArgsFn>,
}

// SAFETY: the fields are function pointers into a module that is never freed — `LoadLibraryW`'s
// reference is deliberately leaked, since the process needs uxtheme for its whole life anyway.
unsafe impl Send for Uxtheme {}
unsafe impl Sync for Uxtheme {}

static UXTHEME: OnceLock<Uxtheme> = OnceLock::new();

fn uxtheme() -> &'static Uxtheme {
    UXTHEME.get_or_init(|| {
        let name: Vec<u16> = "uxtheme.dll\0".encode_utf16().collect();
        // SAFETY: a NUL-terminated wide name, and the handle is kept for the process's life.
        let module = unsafe { LoadLibraryW(PCWSTR(name.as_ptr())) }.unwrap_or_default();
        Uxtheme {
            set_preferred_app_mode: by_ordinal(module, ORDINAL_SET_PREFERRED_APP_MODE),
            allow_dark_mode_for_window: by_ordinal(module, ORDINAL_ALLOW_DARK_MODE_FOR_WINDOW),
            refresh_immersive_color_policy_state: by_ordinal(
                module,
                ORDINAL_REFRESH_IMMERSIVE_COLOR_POLICY_STATE,
            ),
            flush_menu_themes: by_ordinal(module, ORDINAL_FLUSH_MENU_THEMES),
        }
    })
}

/// One export, by ordinal — Windows' own `MAKEINTRESOURCE` convention, where a small integer
/// standing in a name's place *is* an ordinal rather than a pointer to read.
fn by_ordinal<F>(module: HMODULE, ordinal: usize) -> Option<F> {
    if module.is_invalid() {
        return None;
    }
    // SAFETY: `PCSTR(ordinal as *const u8)` is `MAKEINTRESOURCEA`, which `GetProcAddress`
    // documents; the address it answers is transmuted to the signature the ordinal is known to
    // carry, and the module outlives the pointer because it is never freed.
    unsafe {
        let found = GetProcAddress(module, PCSTR(ordinal as *const u8))?;
        Some(std::mem::transmute_copy::<
            unsafe extern "system" fn() -> isize,
            F,
        >(&found))
    }
}

/// Put the **whole process** in dark or light mode, then push the change into the menus that are
/// already built. Both halves matter: the first decides what a menu opened from now on looks
/// like, the second recolours the ones Windows has already themed — without it, a theme toggle
/// shows its effect only after the next restart.
pub fn set_app_mode(dark: bool) {
    let ux = uxtheme();
    // SAFETY: each pointer was resolved from uxtheme by its known ordinal and is called with the
    // signature that ordinal carries. Every one is best-effort; none can fail this function.
    unsafe {
        if let Some(set) = ux.set_preferred_app_mode {
            set(if dark {
                APP_MODE_FORCE_DARK
            } else {
                APP_MODE_DEFAULT
            });
        }
        if let Some(refresh) = ux.refresh_immersive_color_policy_state {
            refresh();
        }
        if let Some(flush) = ux.flush_menu_themes {
            flush();
        }
    }
}

/// Let one window's Windows-drawn parts follow the process's mode. The menu bar belongs to the
/// window, so the frame has to opt in as well as the process.
pub fn allow_for_window(hwnd: HWND, dark: bool) {
    // SAFETY: as [`set_app_mode`] — a resolved pointer, called with its ordinal's signature.
    unsafe {
        if let Some(allow) = uxtheme().allow_dark_mode_for_window {
            let _ = allow(hwnd, BOOL::from(dark));
        }
    }
}
