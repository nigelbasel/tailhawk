//! Presentation leaf backend — D3D11 device, swapchain and `SPEC.md` §3.2's fallback chain.
//!
//! Derived from `experiments/g3-d3d11` and `experiments/g4-glyph-atlas`, which established the
//! device/swapchain shape and measured the cost of getting here.

use windows::Win32::Foundation::{DXGI_STATUS_OCCLUDED, E_UNEXPECTED, HWND};
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP, D3D_FEATURE_LEVEL_11_0,
    D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST,
};
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Buffer, ID3D11Device, ID3D11DeviceContext, ID3D11PixelShader,
    ID3D11RenderTargetView, ID3D11Texture2D, ID3D11VertexShader, D3D11_BIND_CONSTANT_BUFFER,
    D3D11_BUFFER_DESC, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_CREATE_DEVICE_FLAG,
    D3D11_SDK_VERSION, D3D11_SUBRESOURCE_DATA, D3D11_USAGE_DEFAULT, D3D11_VIEWPORT,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory2, IDXGIFactory2, IDXGISwapChain1, DXGI_CREATE_FACTORY_FLAGS,
    DXGI_ERROR_DEVICE_HUNG, DXGI_ERROR_DEVICE_REMOVED, DXGI_ERROR_DEVICE_RESET,
    DXGI_ERROR_DRIVER_INTERNAL_ERROR, DXGI_PRESENT, DXGI_PRESENT_TEST, DXGI_SCALING_NONE,
    DXGI_SWAP_CHAIN_DESC1, DXGI_SWAP_CHAIN_FLAG, DXGI_SWAP_EFFECT_FLIP_DISCARD,
    DXGI_USAGE_RENDER_TARGET_OUTPUT,
};

use crate::{Error, Result, WindowHandle};

impl From<windows::core::Error> for Error {
    fn from(e: windows::core::Error) -> Self {
        Error(format!("{e}"))
    }
}

/// Which rung of `SPEC.md` §3.2's chain the device came up on. The chain is
/// hardware → WARP → Direct2D/DXGI; the third rung is not implemented yet and is tracked as such
/// rather than silently omitted.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Driver {
    Hardware,
    Warp,
}

impl Driver {
    pub fn name(self) -> &'static str {
        match self {
            Driver::Hardware => "hardware",
            Driver::Warp => "WARP",
        }
    }
}

/// The failures that mean *the device object you are holding is gone*, as opposed to *that call
/// was wrong*.
///
/// `SPEC.md` §3.2: "**Never panic on device-removed** — that is the exact bug that crashes wgpu
/// apps on driver auto-update." A driver update, a TDR, a GPU reset, an external monitor being
/// unplugged and a laptop switching between its integrated and discrete GPUs all arrive here. None
/// of them is a program error, and each is recoverable by building a new device.
///
/// The list is deliberately only the four DXGI codes, each taken from the `windows` crate's own
/// generated bindings rather than written out. It is not the last word on the question —
/// [`Gpu::was_lost`] asks the device itself as well, which is what covers a call that fails with
/// some other code while the device is being removed underneath it.
fn is_device_lost(code: windows::core::HRESULT) -> bool {
    [
        DXGI_ERROR_DEVICE_REMOVED,
        DXGI_ERROR_DEVICE_RESET,
        DXGI_ERROR_DEVICE_HUNG,
        DXGI_ERROR_DRIVER_INTERNAL_ERROR,
    ]
    .contains(&code)
}

/// What to do about a device that has just been lost.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Next {
    /// Build a new device on this rung of §3.2's chain and carry on.
    Retry(Driver),
    /// Stop rebuilding. The caller reports the failure and the shell falls back to the flat
    /// background it painted before a device existed — which is still not a panic.
    GiveUp,
}

/// How many devices will be built for one unbroken run of losses before the renderer concludes
/// that the problem is not transient.
///
/// A driver auto-update loses the device once. A GPU that is genuinely failing loses it every
/// frame, and rebuilding a D3D11 device on every `WM_PAINT` is worse for the user than a static
/// window: it is a hot loop that keeps the machine busy while showing nothing.
const MAX_ATTEMPTS: u32 = 3;

/// `SPEC.md` §3.2's device-removed policy, kept apart from the D3D calls so it can be tested
/// without a GPU.
///
/// The one subtlety is what counts as recovery. Only a frame that needed **no** recovery clears
/// the streak: a device that dies and is rebuilt on every single frame would otherwise reset the
/// count each time and rebuild for ever.
struct Recovery {
    driver: Driver,
    streak: u32,
}

impl Recovery {
    fn new(driver: Driver) -> Self {
        Self { driver, streak: 0 }
    }

    /// A frame that drew and presented with no device trouble at all.
    fn on_clean_frame(&mut self) {
        self.streak = 0;
    }

    fn on_loss(&mut self) -> Next {
        self.streak += 1;
        if self.streak > MAX_ATTEMPTS {
            return Next::GiveUp;
        }
        // The first loss keeps the rung it was on: a driver update or a TDR leaves the hardware
        // perfectly able to host the next device, and dropping to WARP over one blip would trade
        // the GPU away for nothing. A device that dies *again* before a clean frame is not a blip,
        // so from the second attempt on it is WARP, which cannot be removed by a driver.
        if self.streak == 1 {
            Next::Retry(self.driver)
        } else {
            Next::Retry(Driver::Warp)
        }
    }

    /// Records the rung a rebuilt device actually landed on, which is not always the rung asked
    /// for — `create_resources` still walks the chain.
    fn on_rebuilt(&mut self, driver: Driver) {
        self.driver = driver;
    }
}

/// DXBC produced by `fxc` in `build.rs`. Embedded, so nothing compiles a shader at runtime and
/// the binary carries no `d3dcompiler_47.dll` import (`SPEC.md` §3.2).
const BACKGROUND_VS: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/background_vs.cso"));
const BACKGROUND_PS: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/background_ps.cso"));

/// Everything whose lifetime is the device's. When the device goes, all of this goes with it —
/// which is why it is one struct: recovery replaces the whole of it and cannot leave a shader or
/// a buffer behind that belongs to a dead device.
struct Resources {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    driver: Driver,
    vs: ID3D11VertexShader,
    ps: ID3D11PixelShader,
    frame_cb: ID3D11Buffer,
}

pub struct Gpu {
    res: Resources,
    swapchain: Option<IDXGISwapChain1>,
    rtv: Option<ID3D11RenderTargetView>,
    /// Remembered so a rebuilt device can re-attach itself without the shell being told anything
    /// happened.
    window: Option<WindowHandle>,
    size: (u32, u32),
    recovery: Recovery,
    /// Counts devices, not losses: it starts at 1 and rises by one per rebuild. Tests assert on
    /// it because "did the renderer really replace the device" is otherwise invisible from
    /// outside.
    generation: u32,
    /// Whether the last present said nobody can see this window. See [`Standby`].
    standby: Standby,
    #[cfg(any(test, feature = "test-hooks"))]
    inject_loss: bool,
    /// A render target standing in for a swapchain, so the **real** frame path can be read back.
    ///
    /// [`offscreen::Offscreen`] builds its own device and exists to test `text.rs` and `paint.rs` in
    /// isolation. It cannot test `Gpu` itself, and that gap had a cost: `draw_background` inherited
    /// a blend state from the text pass and **stopped drawing anything at all from the second frame
    /// onward** for fifteen sessions — which on a zeroed back buffer reads as pure black, invisibly
    /// at this palette — and was found by screenshot rather than by any test. Anything reachable
    /// only through a swapchain is untestable, and this is the smallest thing that makes it
    /// reachable.
    #[cfg(any(test, feature = "test-hooks"))]
    offscreen: Option<(ID3D11Texture2D, ID3D11Texture2D)>,
}

fn create_device(driver: Driver) -> windows::core::Result<(ID3D11Device, ID3D11DeviceContext)> {
    let driver_type = match driver {
        Driver::Hardware => D3D_DRIVER_TYPE_HARDWARE,
        Driver::Warp => D3D_DRIVER_TYPE_WARP,
    };
    let mut device = None;
    let mut context = None;
    unsafe {
        D3D11CreateDevice(
            None,
            driver_type,
            None,
            // BGRA_SUPPORT keeps the Direct2D interop path open, which the third fallback rung
            // will need.
            D3D11_CREATE_DEVICE_BGRA_SUPPORT | D3D11_CREATE_DEVICE_FLAG(0),
            // Feature level 11_0 and nothing lower. The glyph pass reads its quads from a
            // `StructuredBuffer` in the vertex shader, which is shader model 5 — on a feature-level
            // 10 device the shaders would simply fail to create, at which point there is no text.
            // Better to fall to WARP, which is always 11_0, than to come up on a device that cannot
            // draw. 11_1 is deliberately *not* requested: a runtime that does not know it rejects
            // the whole array with `E_INVALIDARG` rather than skipping the entry.
            Some(&[D3D_FEATURE_LEVEL_11_0]),
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            Some(&mut context),
        )?;
    }
    Ok((
        device.expect("device out param is set when D3D11CreateDevice succeeds"),
        context.expect("context out param is set when D3D11CreateDevice succeeds"),
    ))
}

/// Walks `SPEC.md` §3.2's chain from `prefer` downwards. Asking for WARP asks for WARP only —
/// that is what makes the escalation in [`Recovery::on_loss`] mean anything.
fn create_resources(prefer: Driver) -> Result<Resources> {
    let (device, context, driver) = match prefer {
        Driver::Warp => {
            let (d, c) = create_device(Driver::Warp)?;
            (d, c, Driver::Warp)
        }
        Driver::Hardware => match create_device(Driver::Hardware) {
            Ok((d, c)) => (d, c, Driver::Hardware),
            Err(_) => {
                let (d, c) = create_device(Driver::Warp)?;
                (d, c, Driver::Warp)
            }
        },
    };

    let mut vs = None;
    let mut ps = None;
    let mut frame_cb = None;
    // A float4 is already the 16-byte multiple a constant buffer requires, so no padding.
    let initial = crate::BACKGROUND;
    let cb_desc = D3D11_BUFFER_DESC {
        ByteWidth: std::mem::size_of_val(&initial) as u32,
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: D3D11_BIND_CONSTANT_BUFFER.0 as u32,
        ..Default::default()
    };
    let cb_data = D3D11_SUBRESOURCE_DATA {
        pSysMem: initial.as_ptr().cast(),
        ..Default::default()
    };
    unsafe {
        device.CreateVertexShader(BACKGROUND_VS, None, Some(&mut vs))?;
        device.CreatePixelShader(BACKGROUND_PS, None, Some(&mut ps))?;
        device.CreateBuffer(&cb_desc, Some(&cb_data), Some(&mut frame_cb))?;
    }

    Ok(Resources {
        device,
        context,
        driver,
        vs: vs.expect("vertex shader out param is set on success"),
        ps: ps.expect("pixel shader out param is set on success"),
        frame_cb: frame_cb.expect("constant buffer out param is set on success"),
    })
}

/// Why a frame's two failure kinds are not one type.
///
/// Device loss has to stay a `windows::core::Error` all the way to [`Gpu::was_lost`], which
/// classifies it from the `HRESULT` **and** by asking the device directly — flattening it to a
/// string first would throw away the only evidence that distinguishes "the device is gone" from
/// "that call was wrong", and getting that wrong in the permissive direction rebuilds the device
/// over a genuine bug and hides it.
///
/// **⚠ A `Draw` error is therefore never treated as device loss, and that is a deliberate narrowing
/// rather than a proof.** The text pass can in principle meet a removed device of its own — a failed
/// buffer map — and it reports that as a `crate::Error` with no `HRESULT` left to read. Such a frame
/// returns `Err` and the recovery path does not run for it; the *next* frame hits the same loss in
/// `present`, which is typed, and recovers there. One dropped frame, not a stuck renderer. Widening
/// this means giving `crate::Error` somewhere to carry an `HRESULT`, which is a change to the
/// crate's error type and is not made here.
enum FrameError {
    Device(windows::core::Error),
    Draw(Error),
}

/// Whether anyone can see the window, and what to do about it.
///
/// DXGI reports occlusion through a **success** HRESULT on `Present`, and recommends standing by
/// rather than continuing to render frames nobody will see. It also warns that the same code must
/// not be used to decide when to resume — a `DXGI_PRESENT_TEST` present is what answers that,
/// because it costs nothing and returns `S_OK` once the window is visible again.
///
/// A struct rather than a `bool` so the decision is a pure one a test can reach: everything here
/// takes an HRESULT in and answers a question, with no device and no window involved.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Standby {
    occluded: bool,
}

impl Standby {
    /// What the last present said.
    pub fn saw(&mut self, hr: windows::core::HRESULT) {
        self.occluded = hr == DXGI_STATUS_OCCLUDED;
    }

    /// Whether a frame is worth drawing at all.
    pub fn should_draw(&self) -> bool {
        !self.occluded
    }

    /// What a test-present said. `S_OK` means the window is back.
    ///
    /// Deliberately not `saw`: any result other than occluded ends standby, because a device error
    /// must reach the recovery path rather than leaving the renderer parked for ever.
    pub fn wake(&mut self, hr: windows::core::HRESULT) {
        if hr != DXGI_STATUS_OCCLUDED {
            self.occluded = false;
        }
    }
}

impl Gpu {
    pub fn new() -> Result<Self> {
        let res = create_resources(Driver::Hardware)?;
        Ok(Self {
            recovery: Recovery::new(res.driver),
            res,
            swapchain: None,
            rtv: None,
            window: None,
            size: (1, 1),
            generation: 1,
            standby: Standby::default(),
            #[cfg(any(test, feature = "test-hooks"))]
            inject_loss: false,
            #[cfg(any(test, feature = "test-hooks"))]
            offscreen: None,
        })
    }

    /// Renders to a texture instead of a window, so a frame can be read back pixel for pixel.
    ///
    /// Everything after this is the ordinary path: `render_frame_with` draws the background, runs
    /// the caller's pass and calls `present`, which with no swapchain and no window asks the device
    /// whether it is still alive — so device-loss classification still works.
    ///
    /// **Not a substitute for a window.** It cannot exercise `ResizeBuffers`, flip-model buffer
    /// rotation, or the DXGI factory staleness that `a_device_lost_with_a_window_attached_comes_
    /// back_presenting` covers in the shell. Re-attach after a rebuild: the new device cannot use
    /// the old device's texture.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn attach_offscreen(&mut self, width: u32, height: u32) -> Result<()> {
        let (width, height) = (width.max(1), height.max(1));
        let (target, staging, rtv) = offscreen::make_target(&self.res.device, width, height)?;
        self.rtv = Some(rtv);
        self.offscreen = Some((target, staging));
        self.size = (width, height);
        Ok(())
    }

    /// Fills the offscreen target with a colour, so the next frame starts from a known state.
    ///
    /// **Without this the target is indistinguishable from a correct frame.** It keeps the previous
    /// frame's pixels, so a background that is not drawn *at all* leaves the right colour behind and
    /// reads as success. A real swapchain does not behave that way — flip-model back buffers start
    /// undefined, and a fresh one is zeroed, which is why the blend-state bug showed on screen as
    /// black rather than as a stale-but-correct frame. Clearing to something no pass would ever
    /// produce restores that distinction.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn clear_offscreen(&self, colour: [f32; 4]) {
        if let Some(rtv) = self.rtv.as_ref() {
            unsafe { self.res.context.ClearRenderTargetView(rtv, &colour) };
        }
    }

    /// The last frame drawn to the offscreen target.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn read_back(&self) -> Result<offscreen::Pixels> {
        let Some((target, staging)) = self.offscreen.as_ref() else {
            return Err(Error("no offscreen target is attached".into()));
        };
        offscreen::read_target(&self.res.context, target, staging, self.size.0, self.size.1)
    }

    pub fn driver(&self) -> Driver {
        self.res.driver
    }

    pub fn generation(&self) -> u32 {
        self.generation
    }

    /// The current device and its immediate context, for resources built outside a frame.
    ///
    /// Whatever is built from these belongs to [`generation`](Self::generation) **at the moment of
    /// the call** and must be re-checked against it afterwards, because a rebuild replaces both.
    pub(crate) fn resources(&self) -> (&ID3D11Device, &ID3D11DeviceContext) {
        (&self.res.device, &self.res.context)
    }

    pub fn attach(&mut self, window: WindowHandle, width: u32, height: u32) -> Result<()> {
        if self.swapchain.is_some() {
            return Ok(());
        }
        self.window = Some(window);
        self.size = (width.max(1), height.max(1));
        self.create_swapchain()?;
        Ok(())
    }

    fn create_swapchain(&mut self) -> windows::core::Result<()> {
        let Some(window) = self.window else {
            return Ok(());
        };
        // CreateDXGIFactory2 rather than walking device → adapter → parent, so the factory is a
        // 1.2+ interface and `CreateSwapChainForHwnd` exists on it. It is also built fresh every
        // time rather than cached: a factory created before a device was removed is stale, and
        // making a swapchain from a stale factory is the classic way a "recovered" renderer
        // presents to nothing.
        let factory: IDXGIFactory2 = unsafe { CreateDXGIFactory2(DXGI_CREATE_FACTORY_FLAGS(0))? };
        let (width, height) = self.size;
        let desc = DXGI_SWAP_CHAIN_DESC1 {
            Width: width,
            Height: height,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
            BufferCount: 3,
            SwapEffect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
            // **Not the default.** `DXGI_SCALING_STRETCH` is what `Default` gives, and on a
            // flip-model chain it tells DWM to rubber-band the last presented frame to the new
            // window size for as long as it takes a fresh one to arrive. Dragging an edge then
            // stretches and squashes the text, which no Windows application does.
            //
            // `NONE` pins the buffer to the top-left and leaves the uncovered strip showing the
            // window background until the next frame — which, with the synchronous repaint in
            // `WM_SIZE`, is the same frame.
            Scaling: DXGI_SCALING_NONE,
            ..Default::default()
        };
        let swapchain = unsafe {
            factory.CreateSwapChainForHwnd(
                &self.res.device,
                HWND(window.0 as _),
                &desc,
                None,
                None,
            )?
        };
        self.swapchain = Some(swapchain);
        self.create_rtv()
    }

    fn create_rtv(&mut self) -> windows::core::Result<()> {
        let Some(sc) = self.swapchain.as_ref() else {
            return Ok(());
        };
        let back: ID3D11Texture2D = unsafe { sc.GetBuffer(0)? };
        let mut rtv = None;
        unsafe {
            self.res
                .device
                .CreateRenderTargetView(&back, None, Some(&mut rtv))?;
        }
        self.rtv = rtv;
        Ok(())
    }

    pub fn resize(&mut self, width: u32, height: u32) -> Result<()> {
        self.size = (width.max(1), height.max(1));
        if self.swapchain.is_none() {
            return Ok(());
        }
        // The RTV holds a reference to the back buffer and ResizeBuffers fails while it is alive.
        self.rtv = None;
        {
            let sc = self.swapchain.as_ref().expect("checked above");
            unsafe {
                sc.ResizeBuffers(
                    0,
                    self.size.0,
                    self.size.1,
                    DXGI_FORMAT_B8G8R8A8_UNORM,
                    DXGI_SWAP_CHAIN_FLAG(0),
                )?;
            }
        }
        self.create_rtv()?;
        Ok(())
    }

    /// Draws and presents one frame, replacing the device if it has been lost.
    ///
    /// `SPEC.md` §3.2's device-removed requirement lives here rather than in the shell, because
    /// the shell cannot act on the distinction: everything it could do about a lost device is
    /// something the renderer can do better, and the one thing it must never do is panic.
    pub fn render_frame(&mut self, colour: [f32; 4]) -> Result<()> {
        self.render_frame_with(colour, &mut |_, _, _| Ok(()))
    }

    /// The same frame, with a pass drawn between the background and the present.
    ///
    /// `draw` receives the device, its immediate context, and **the device's generation**. The
    /// generation is not a convenience: on the recovery path below, `draw` is called a second time
    /// against a *different* device, and anything it owns that the old device created — a glyph
    /// atlas texture, a vertex buffer, a shader — is dead. Handing it the generation is what lets it
    /// notice, and drawing a sheet from the previous device silently produces nothing rather than
    /// failing, which is the one outcome §3.2's recovery must not leave behind.
    pub fn render_frame_with(
        &mut self,
        colour: [f32; 4],
        draw: &mut dyn FnMut(&ID3D11Device, &ID3D11DeviceContext, u32) -> Result<()>,
    ) -> Result<()> {
        let e = match self.frame_with(colour, draw) {
            Ok(()) => {
                self.recovery.on_clean_frame();
                return Ok(());
            }
            // Not a device question — see [`FrameError`]. Out, unclassified and unretried.
            Err(FrameError::Draw(e)) => return Err(e),
            Err(FrameError::Device(e)) => e,
        };
        if !self.was_lost(&e) {
            return Err(e.into());
        }
        match self.recovery.on_loss() {
            Next::GiveUp => Err(Error(format!(
                "the graphics device was lost {MAX_ATTEMPTS} times without recovering: {e}"
            ))),
            Next::Retry(driver) => {
                self.rebuild(driver)?;
                // One retry, then out. A second loss in the same frame is next frame's problem,
                // and going round again here would let one `WM_PAINT` spin.
                //
                // `rebuild` has already bumped the generation, so `draw` sees a new one here and is
                // expected to replace whatever the dead device owned before it draws.
                self.frame_with(colour, draw).map_err(|e| match e {
                    FrameError::Draw(e) => e,
                    FrameError::Device(e) => e.into(),
                })
            }
        }
    }

    /// Distinguishes "the device is gone" from "that call was wrong".
    ///
    /// The HRESULT is not always enough on its own: a call can fail with a generic error in the
    /// instant a device is being removed, so the device is asked directly as well. Getting this
    /// wrong in the permissive direction would rebuild the device over a genuine bug and hide it.
    fn was_lost(&self, e: &windows::core::Error) -> bool {
        if is_device_lost(e.code()) {
            return true;
        }
        unsafe { self.res.device.GetDeviceRemovedReason().is_err() }
    }

    fn rebuild(&mut self, driver: Driver) -> Result<()> {
        // Order matters: everything the old device owns is released before a new one is asked
        // for, so a driver that is mid-reset is not being asked to host two devices at once.
        self.rtv = None;
        self.swapchain = None;
        self.res = create_resources(driver)?;
        self.recovery.on_rebuilt(self.res.driver);
        self.generation += 1;
        self.create_swapchain()?;
        Ok(())
    }

    fn frame_with(
        &mut self,
        colour: [f32; 4],
        draw: &mut dyn FnMut(&ID3D11Device, &ID3D11DeviceContext, u32) -> Result<()>,
    ) -> std::result::Result<(), FrameError> {
        #[cfg(any(test, feature = "test-hooks"))]
        if std::mem::take(&mut self.inject_loss) {
            return Err(FrameError::Device(windows::core::Error::from(
                DXGI_ERROR_DEVICE_REMOVED,
            )));
        }
        // Standby: nobody can see this window, so nothing is drawn. A test present costs no frame
        // and is the documented way to find out when that stops being true.
        if !self.standby.should_draw() {
            if let Some(sc) = &self.swapchain {
                let hr = unsafe { sc.Present(0, DXGI_PRESENT_TEST) };
                self.standby.wake(hr);
                if hr.is_err() {
                    return Err(FrameError::Device(windows::core::Error::from(hr)));
                }
            } else {
                self.standby.wake(windows::core::HRESULT(0));
            }
            return Ok(());
        }
        self.draw_background(colour);
        draw(&self.res.device, &self.res.context, self.generation).map_err(FrameError::Draw)?;
        self.present().map_err(FrameError::Device)
    }

    /// Draws the background as one fullscreen triangle through the embedded shaders.
    ///
    /// A `ClearRenderTargetView` would produce the same pixels more cheaply. Drawing it exercises
    /// the offline-compiled path — bytecode, shader creation, constant buffer, draw — which is the
    /// point at M0: the pipeline the grid depends on at M3 is proven now rather than then.
    fn draw_background(&mut self, colour: [f32; 4]) {
        let Some(rtv) = self.rtv.clone() else {
            return;
        };
        let (w, h) = self.size;
        // Nothing on this path returns an HRESULT. A lost device is not reported by the immediate
        // context at all — the calls are recorded and discarded — which is why `Present` is what
        // actually surfaces it.
        unsafe {
            let ctx = &self.res.context;
            ctx.UpdateSubresource(&self.res.frame_cb, 0, None, colour.as_ptr().cast(), 0, 0);
            ctx.OMSetRenderTargets(Some(&[Some(rtv)]), None);
            // **The background is opaque, and it has to say so rather than inherit.** D3D11's
            // context is global and `TextPipeline::draw` leaves its dual-source blend state bound.
            // The background's pixel shader emits one output, so from the second frame on it was
            // blended against an undefined second source — and the measured effect is that the
            // background pass **leaves the target untouched**, not that it draws something dark.
            // `the_background_survives_a_frame_of_text` shows that directly: with this line removed
            // the target keeps the magenta it was cleared to.
            //
            // On screen that read as **pure black**, because a flip-model back buffer starts
            // undefined and a fresh one is zeroed — so the symptom depends on what was already in
            // the buffer, which is why an offscreen target that keeps the previous frame looked
            // *correct* and hid the bug from the first version of that test. Frame 1 was always
            // right, every frame after it was not, and at this palette (18,20,23 against 0,0,0)
            // nobody catches that by looking. Found by dogfooding a real file, not by any test.
            ctx.OMSetBlendState(None, None, 0xFFFF_FFFF);
            ctx.RSSetViewports(Some(&[D3D11_VIEWPORT {
                TopLeftX: 0.0,
                TopLeftY: 0.0,
                Width: w as f32,
                Height: h as f32,
                MinDepth: 0.0,
                MaxDepth: 1.0,
            }]));
            ctx.IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
            ctx.VSSetShader(&self.res.vs, None);
            ctx.PSSetShader(&self.res.ps, None);
            ctx.PSSetConstantBuffers(0, Some(&[Some(self.res.frame_cb.clone())]));
            ctx.Draw(3, 0);
        }
    }

    /// Presents, and notices when nobody can see the result.
    ///
    /// **`DXGI_STATUS_OCCLUDED` is a *success* code**, so `HRESULT::ok()` — which is a sign test —
    /// returns `Ok` for it and it was being thrown away. DXGI's own guidance is to go into standby
    /// when it appears, because everything spent on that frame was wasted: a window behind a
    /// maximised one, on another virtual desktop, or minimised, was still laying out a viewport,
    /// shaping every visible row, rasterising and presenting at up to 60 Hz.
    ///
    /// The same guidance warns not to use this code to decide when to *come back* — that is what
    /// [`Standby::wake`]'s test-present is for.
    fn present(&mut self) -> windows::core::Result<()> {
        match (&self.swapchain, self.window) {
            (Some(sc), _) => {
                let hr = unsafe { sc.Present(1, DXGI_PRESENT(0)) };
                self.standby.saw(hr);
                hr.ok()
            }
            // Attached to a window but holding no swapchain is not a state that can be presented
            // from, and it is exactly what a rebuild that forgot to re-attach would leave behind.
            // Reporting it is what stops a "recovered" renderer from returning `Ok` for ever
            // while nothing reaches the screen.
            (None, Some(_)) => Err(windows::core::Error::from(E_UNEXPECTED)),
            // No window yet. The device is still asked whether it is alive, so a loss before the
            // first attach is not held back until one arrives.
            (None, None) => unsafe { self.res.device.GetDeviceRemovedReason() },
        }
    }

    #[cfg(any(test, feature = "test-hooks"))]
    pub fn simulate_device_loss(&mut self) {
        self.inject_loss = true;
    }
}

/// A device and a render target with no window and no swapchain, plus readback.
///
/// This is how the glyph pass is tested: draw into a texture, copy it to a staging texture, map it,
/// and assert on the pixels. **Reading the pixels back is the only way to know the blend equation is
/// doing what `SPEC.md` §3.2 says it does** — every call can succeed while producing the wrong
/// composite, and a swapchain's back buffer is undefined after a flip-model `Present`.
/// Under `test-hooks` as well as `cfg(test)`, since 2026-08-17: the shell's headless screenshot
/// (`Renderer::snapshot`) is the non-test consumer this used to lack, and a harness with no desktop
/// to capture from is exactly what an offscreen target is for.
#[cfg(any(test, feature = "test-hooks"))]
pub mod offscreen {
    #[cfg(test)]
    use windows::Win32::Graphics::Direct3D11::D3D11_VIEWPORT;
    use windows::Win32::Graphics::Direct3D11::{
        ID3D11Device, ID3D11DeviceContext, ID3D11RenderTargetView, ID3D11Texture2D,
        D3D11_BIND_RENDER_TARGET, D3D11_BIND_SHADER_RESOURCE, D3D11_CPU_ACCESS_READ,
        D3D11_MAP_READ, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT, D3D11_USAGE_STAGING,
    };
    use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};

    use crate::Result;

    /// A mapped copy of the target. Channels are **BGRA**, in that order, because that is the
    /// swapchain format the real renderer uses and testing a different one would test a different
    /// path.
    pub struct Pixels {
        data: Vec<u8>,
        width: u32,
        stride: usize,
    }

    impl Pixels {
        pub fn width(&self) -> u32 {
            self.width
        }

        pub fn height(&self) -> u32 {
            self.data.len().checked_div(self.stride).unwrap_or(0) as u32
        }

        pub fn at(&self, x: u32, y: u32) -> [u8; 4] {
            assert!(x < self.width, "x {x} is outside the target");
            let i = y as usize * self.stride + x as usize * 4;
            [
                self.data[i],
                self.data[i + 1],
                self.data[i + 2],
                self.data[i + 3],
            ]
        }
    }

    /// A self-contained device and target for the painter's own tests — the tests only.
    #[cfg(test)]
    pub struct Offscreen {
        device: ID3D11Device,
        context: ID3D11DeviceContext,
        target: ID3D11Texture2D,
        staging: ID3D11Texture2D,
        rtv: ID3D11RenderTargetView,
        width: u32,
        height: u32,
    }

    #[cfg(test)]
    impl Offscreen {
        pub fn new(width: u32, height: u32) -> Result<Self> {
            let (device, context) = super::create_device(super::Driver::Hardware)
                .or_else(|_| super::create_device(super::Driver::Warp))?;

            let mut desc = D3D11_TEXTURE2D_DESC {
                Width: width,
                Height: height,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                Usage: D3D11_USAGE_DEFAULT,
                BindFlags: (D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_SHADER_RESOURCE.0) as u32,
                CPUAccessFlags: 0,
                MiscFlags: 0,
            };
            let mut target = None;
            unsafe { device.CreateTexture2D(&desc, None, Some(&mut target))? };
            let target = target.expect("target out param is set on success");

            // A render target cannot be mapped, so the readback goes through a staging copy.
            desc.Usage = D3D11_USAGE_STAGING;
            desc.BindFlags = 0;
            desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
            let mut staging = None;
            unsafe { device.CreateTexture2D(&desc, None, Some(&mut staging))? };

            let mut rtv = None;
            unsafe { device.CreateRenderTargetView(&target, None, Some(&mut rtv))? };

            let offscreen = Self {
                device,
                context,
                target,
                staging: staging.expect("staging out param is set on success"),
                rtv: rtv.expect("rtv out param is set on success"),
                width,
                height,
            };
            offscreen.bind();
            Ok(offscreen)
        }

        pub fn device(&self) -> &ID3D11Device {
            &self.device
        }

        pub fn context(&self) -> &ID3D11DeviceContext {
            &self.context
        }

        fn bind(&self) {
            unsafe {
                self.context
                    .OMSetRenderTargets(Some(&[Some(self.rtv.clone())]), None);
                self.context.RSSetViewports(Some(&[D3D11_VIEWPORT {
                    TopLeftX: 0.0,
                    TopLeftY: 0.0,
                    Width: self.width as f32,
                    Height: self.height as f32,
                    MinDepth: 0.0,
                    MaxDepth: 1.0,
                }]));
            }
        }

        pub fn clear(&self, colour: [f32; 4]) {
            unsafe { self.context.ClearRenderTargetView(&self.rtv, &colour) };
        }

        pub fn read_back(&self) -> Result<Pixels> {
            read_target(
                &self.context,
                &self.target,
                &self.staging,
                self.width,
                self.height,
            )
        }
    }

    /// A render target cannot be mapped, so a readback copies it into a staging texture first.
    ///
    /// Free rather than a method because [`Gpu`](super::Gpu) needs it too: its offscreen target
    /// belongs to **its own** device, and the point of that path is to exercise the real `Gpu`
    /// rather than a parallel one built alongside it.
    pub(crate) fn read_target(
        context: &ID3D11DeviceContext,
        target: &ID3D11Texture2D,
        staging: &ID3D11Texture2D,
        width: u32,
        height: u32,
    ) -> Result<Pixels> {
        unsafe { context.CopyResource(staging, target) };
        let mut mapped = Default::default();
        unsafe { context.Map(staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))? };
        let stride = mapped.RowPitch as usize;
        let mut data = vec![0u8; stride * height as usize];
        unsafe {
            std::ptr::copy_nonoverlapping(mapped.pData.cast::<u8>(), data.as_mut_ptr(), data.len());
            context.Unmap(staging, 0);
        }
        Ok(Pixels {
            data,
            width,
            stride,
        })
    }

    /// A render target, its staging copy and a view, on a device the caller already owns.
    pub(crate) fn make_target(
        device: &ID3D11Device,
        width: u32,
        height: u32,
    ) -> Result<(ID3D11Texture2D, ID3D11Texture2D, ID3D11RenderTargetView)> {
        let mut desc = D3D11_TEXTURE2D_DESC {
            Width: width,
            Height: height,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: (D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_SHADER_RESOURCE.0) as u32,
            CPUAccessFlags: 0,
            MiscFlags: 0,
        };
        let mut target = None;
        unsafe { device.CreateTexture2D(&desc, None, Some(&mut target))? };
        let target = target.expect("target out param is set on success");

        desc.Usage = D3D11_USAGE_STAGING;
        desc.BindFlags = 0;
        desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
        let mut staging = None;
        unsafe { device.CreateTexture2D(&desc, None, Some(&mut staging))? };

        let mut rtv = None;
        unsafe { device.CreateRenderTargetView(&target, None, Some(&mut rtv))? };

        Ok((
            target,
            staging.expect("staging out param is set on success"),
            rtv.expect("rtv out param is set on success"),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::core::Interface;
    use windows::Win32::Foundation::{E_INVALIDARG, E_OUTOFMEMORY};
    use windows::Win32::Graphics::Dxgi::DXGI_ERROR_INVALID_CALL;

    // The policy half needs no GPU at all, which is the point of separating it.

    /// **Occlusion arrives as a *success* code**, which is exactly how it went unnoticed:
    /// `HRESULT::ok()` is a sign test, so `DXGI_STATUS_OCCLUDED` read as "presented fine" and a
    /// window nobody could see went on laying out, shaping, rasterising and presenting at 60 Hz.
    #[test]
    fn an_occluded_present_is_a_success_code_and_still_means_stop_drawing() {
        assert!(
            DXGI_STATUS_OCCLUDED.is_ok(),
            "if this ever becomes an error code the swallowing bug was never possible"
        );
        let mut standby = Standby::default();
        assert!(standby.should_draw(), "a fresh renderer draws");
        standby.saw(windows::core::HRESULT(0));
        assert!(standby.should_draw());
        standby.saw(DXGI_STATUS_OCCLUDED);
        assert!(
            !standby.should_draw(),
            "nobody can see it, so nothing is drawn"
        );
    }

    /// DXGI's guidance is explicit that the occlusion code says when to *enter* standby and must
    /// not be used to decide when to leave it — a test present answers that. So anything other
    /// than occluded ends standby, including an error, which must reach the recovery path rather
    /// than leaving the renderer parked for ever.
    #[test]
    fn standby_ends_on_anything_that_is_not_still_occluded() {
        let mut standby = Standby::default();
        standby.saw(DXGI_STATUS_OCCLUDED);

        standby.wake(DXGI_STATUS_OCCLUDED);
        assert!(!standby.should_draw(), "still covered, still parked");

        standby.wake(windows::core::HRESULT(0));
        assert!(standby.should_draw(), "visible again");

        standby.saw(DXGI_STATUS_OCCLUDED);
        standby.wake(DXGI_ERROR_DEVICE_REMOVED);
        assert!(
            standby.should_draw(),
            "a lost device must not be left parked — it has to reach recovery"
        );
    }

    #[test]
    fn a_first_loss_is_retried_on_the_rung_it_was_already_on() {
        let mut r = Recovery::new(Driver::Hardware);
        assert_eq!(r.on_loss(), Next::Retry(Driver::Hardware));
    }

    #[test]
    fn a_device_that_dies_again_before_a_clean_frame_drops_to_warp() {
        let mut r = Recovery::new(Driver::Hardware);
        r.on_loss();
        assert_eq!(r.on_loss(), Next::Retry(Driver::Warp));
        assert_eq!(r.on_loss(), Next::Retry(Driver::Warp));
    }

    #[test]
    fn a_clean_frame_clears_the_streak_so_the_next_loss_starts_over() {
        let mut r = Recovery::new(Driver::Hardware);
        r.on_loss();
        r.on_loss();
        r.on_clean_frame();
        assert_eq!(
            r.on_loss(),
            Next::Retry(Driver::Hardware),
            "a device that recovered and then ran normally has earned the hardware rung back"
        );
    }

    /// The loop this bounds is the reason the rule exists: without it, a GPU that loses its device
    /// every frame turns `WM_PAINT` into a device-creation hot loop.
    #[test]
    fn a_device_that_will_not_stay_up_is_eventually_given_up_on() {
        let mut r = Recovery::new(Driver::Hardware);
        for attempt in 1..=MAX_ATTEMPTS {
            assert!(
                matches!(r.on_loss(), Next::Retry(_)),
                "attempt {attempt} of {MAX_ATTEMPTS} should still be worth making"
            );
        }
        assert_eq!(r.on_loss(), Next::GiveUp);
        assert_eq!(r.on_loss(), Next::GiveUp, "give-up is not a one-shot state");
    }

    #[test]
    fn a_rebuild_that_lands_on_warp_makes_warp_the_rung_that_is_retried() {
        let mut r = Recovery::new(Driver::Hardware);
        r.on_loss();
        r.on_rebuilt(Driver::Warp);
        r.on_clean_frame();
        assert_eq!(
            r.on_loss(),
            Next::Retry(Driver::Warp),
            "hardware was already tried and did not hold; the first retry should not go back to it"
        );
    }

    #[test]
    fn only_device_loss_counts_as_device_loss() {
        for lost in [
            DXGI_ERROR_DEVICE_REMOVED,
            DXGI_ERROR_DEVICE_RESET,
            DXGI_ERROR_DEVICE_HUNG,
            DXGI_ERROR_DRIVER_INTERNAL_ERROR,
        ] {
            assert!(is_device_lost(lost), "{lost:?} means the device is gone");
        }
        // A programming error must not be laundered into a device rebuild, which would rebuild
        // the device on every frame and hide the bug behind it.
        for kept in [E_INVALIDARG, DXGI_ERROR_INVALID_CALL, E_OUTOFMEMORY] {
            assert!(!is_device_lost(kept), "{kept:?} is our bug, not the GPU's");
        }
    }

    /// The device half. A machine with neither a GPU nor WARP cannot run this, so it says so
    /// loudly rather than passing quietly — a skipped device test that reads as green is exactly
    /// how this code would rot.
    fn gpu_or_skip(what: &str) -> Option<Gpu> {
        match Gpu::new() {
            Ok(gpu) => Some(gpu),
            Err(e) => {
                eprintln!("SKIPPED {what}: no D3D11 device on this machine ({e})");
                None
            }
        }
    }

    #[test]
    fn a_lost_device_is_replaced_and_the_frame_after_it_succeeds() {
        let Some(mut gpu) =
            gpu_or_skip("a_lost_device_is_replaced_and_the_frame_after_it_succeeds")
        else {
            return;
        };
        let before = gpu.res.device.as_raw();
        assert_eq!(gpu.generation(), 1);

        gpu.simulate_device_loss();
        gpu.render_frame(crate::BACKGROUND)
            .expect("a lost device is rebuilt, not reported");

        assert_eq!(gpu.generation(), 2, "the device should have been replaced");
        assert_ne!(
            before,
            gpu.res.device.as_raw(),
            "the renderer kept the device it was told had gone"
        );
    }

    /// `SPEC.md` §3.2 forbids panicking on device-removed. It does not forbid *failing*, and the
    /// bounded-attempts rule means a persistently dead device eventually does fail — as an `Err`
    /// the shell reports, with the flat background still on screen.
    #[test]
    fn a_device_that_is_lost_every_frame_fails_rather_than_looping_or_panicking() {
        let Some(mut gpu) =
            gpu_or_skip("a_device_that_is_lost_every_frame_fails_rather_than_looping_or_panicking")
        else {
            return;
        };
        for _ in 0..MAX_ATTEMPTS {
            gpu.simulate_device_loss();
            gpu.render_frame(crate::BACKGROUND)
                .expect("attempts within the budget recover");
        }
        gpu.simulate_device_loss();
        assert!(
            gpu.render_frame(crate::BACKGROUND).is_err(),
            "the budget should have run out"
        );
        assert_eq!(
            gpu.generation(),
            1 + MAX_ATTEMPTS,
            "giving up must not build another device"
        );
    }

    /// The realistic case, and the one the attempt budget could easily break: a machine that loses
    /// its device occasionally — a driver update, a GPU switch — over a long-running session. Each
    /// loss must be recovered from indefinitely, because they are separated by frames that worked.
    #[test]
    fn losses_separated_by_clean_frames_are_recovered_from_indefinitely() {
        let Some(mut gpu) =
            gpu_or_skip("losses_separated_by_clean_frames_are_recovered_from_indefinitely")
        else {
            return;
        };
        let rounds = MAX_ATTEMPTS * 2 + 1;
        for round in 0..rounds {
            gpu.simulate_device_loss();
            gpu.render_frame(crate::BACKGROUND)
                .unwrap_or_else(|e| panic!("loss {round} should still be recoverable: {e}"));
            gpu.render_frame(crate::BACKGROUND).expect("a clean frame");
        }
        assert_eq!(gpu.generation(), 1 + rounds);
        assert_eq!(
            gpu.driver(),
            Driver::Hardware,
            "a device that keeps recovering has no reason to be demoted to WARP"
        );
    }

    /// The escalation is not merely policy: a real rebuild has to land on WARP too.
    #[test]
    fn repeated_loss_moves_the_real_device_onto_warp() {
        let Some(mut gpu) = gpu_or_skip("repeated_loss_moves_the_real_device_onto_warp") else {
            return;
        };
        for _ in 0..2 {
            gpu.simulate_device_loss();
            gpu.render_frame(crate::BACKGROUND)
                .expect("attempts within the budget recover");
        }
        assert_eq!(
            gpu.driver(),
            Driver::Warp,
            "the second loss without a clean frame should have given up on the hardware rung"
        );
    }
}
