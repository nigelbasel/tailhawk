//! Tailhawk core — the portable half of the seam in `SPEC.md` §3.1.
//!
//! The core owns the whole grid *including rendering*; the shell owns window, input and IME and
//! hands the core a drawable. At M0 there is no grid yet, so what is here is the seam itself and
//! the presentation leaf backend.
//!
//! Nothing in this crate may reference an `HWND`, a message loop, or any other shell concept. The
//! drawable crosses the seam as an opaque [`WindowHandle`].

#![deny(unsafe_op_in_unsafe_fn)]

pub mod encoding;
pub mod lines;

#[cfg(windows)]
mod gpu;

#[cfg(windows)]
pub use gpu::Driver;

pub use encoding::{detect, Charset, Confidence, Detection, Sample};
pub use lines::LineDecoder;

/// An opaque platform window handle. On Windows this is an `HWND`; the core never interprets it,
/// it only passes it to the presentation backend.
///
/// This exists so that `SPEC.md` §3.1's rule — the core is portable, the shell is not — is
/// enforced by the type system rather than by discipline.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct WindowHandle(pub isize);

/// The window background, in linear-ish sRGB values for the render target.
///
/// **Both stages of the two-stage first paint must use this exact colour** (`SPEC.md` §3.2): the
/// shell fills the client area with it before a device exists, the renderer clears to it once the
/// device is up, and the transition is invisible only if they agree. That is why it lives in the
/// core rather than in the shell — the shell borrows it, so the two cannot drift apart.
///
/// Provisional: `UI-DESIGN.md` §10 describes the palette qualitatively and pins no hex value yet.
pub const BACKGROUND: [f32; 4] = [0.071, 0.078, 0.090, 1.0];

/// The same colour as 8-bit sRGB, for the shell's GDI stage.
pub const fn background_rgb8() -> (u8, u8, u8) {
    (18, 20, 23)
}

/// Errors that cross the seam. Deliberately opaque — the shell can report one but cannot act on
/// the distinction, and `SPEC.md` §3.2 forbids panicking on device loss.
#[derive(Debug)]
pub struct Error(pub String);

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

/// The renderer the shell drives.
///
/// Construction creates the graphics device and is the expensive step — `experiments/g3-d3d11`
/// measured it as the dominant cost of first paint — so the shell is expected to call
/// [`Renderer::new`] on a worker thread while the window comes up on the main thread.
#[cfg(windows)]
pub struct Renderer {
    gpu: gpu::Gpu,
}

#[cfg(windows)]
impl Renderer {
    /// Creates the graphics device, walking `SPEC.md` §3.2's fallback chain.
    ///
    /// **Call this off the thread that owns the window.** Beyond the ~8.5 ms it saves on an idle
    /// machine, `experiments/g3-d3d11` found serial device creation degrading past a second under
    /// GPU-context pressure while the off-thread path held at ~135 ms.
    pub fn new() -> Result<Self> {
        Ok(Self {
            gpu: gpu::Gpu::new()?,
        })
    }

    /// Which rung of the fallback chain the device actually came up on.
    pub fn driver(&self) -> Driver {
        self.gpu.driver
    }

    /// Binds the renderer to a window. Idempotent.
    pub fn attach(&mut self, window: WindowHandle, width: u32, height: u32) -> Result<()> {
        self.gpu.attach(window, width, height)
    }

    pub fn resize(&mut self, width: u32, height: u32) -> Result<()> {
        self.gpu.resize(width, height)
    }

    /// Draws one frame and presents it. At M0 this is the background clear and nothing else —
    /// the grid arrives at M3.
    pub fn paint(&mut self) -> Result<()> {
        self.gpu.draw_background(BACKGROUND)?;
        self.gpu.present()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The two stages of the first paint must be the same colour (`SPEC.md` §3.2). They are
    /// necessarily expressed twice — GDI wants 8-bit channels, the render target wants floats —
    /// and if they ever drift apart the handover from the class brush to the renderer becomes a
    /// visible flash on every cold start. That is the whole reason both live in this crate.
    #[test]
    fn both_paint_stages_are_the_same_colour() {
        let (r, g, b) = background_rgb8();
        for (channel, expected) in BACKGROUND[..3].iter().zip([r, g, b]) {
            let as_8bit = (channel * 255.0).round() as u8;
            assert_eq!(
                as_8bit,
                expected,
                "BACKGROUND {BACKGROUND:?} and background_rgb8() {:?} disagree",
                (r, g, b)
            );
        }
    }

    #[test]
    fn background_is_opaque() {
        assert_eq!(
            BACKGROUND[3], 1.0,
            "a translucent background would composite against whatever is behind the window"
        );
    }
}
