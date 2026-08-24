//! §2.2's *Preferences* — what the dialog offers, and its limits.
//!
//! The interactive sheet model that used to live here went with the overlay it drew into:
//! Preferences is a standard modal dialog now (`dialog.rs`, the owner's direction 2026-08-24),
//! and the dialog's combo boxes carry the choices directly. What remains is the one thing both
//! doors to these settings must agree on — the ranges.

/// The smallest and largest em size offered, in device pixels at the 96-DPI baseline.
///
/// Below eight a log is unreadable and above thirty-two a row holds nothing useful; both ends are
/// clamped rather than allowed to wrap, because a size control that jumps from 32 to 8 on one
/// extra key press is a control that loses a user's place.
pub const MIN_SIZE: u16 = 8;
pub const MAX_SIZE: u16 = 32;

/// The three answers §12.4's `theme` key takes, in the order the dialog lists them.
pub const THEMES: &[&str] = &["system", "light", "dark"];
