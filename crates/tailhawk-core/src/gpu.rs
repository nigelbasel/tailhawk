//! Presentation leaf backend — D3D11 device, swapchain and `SPEC.md` §3.2's fallback chain.
//!
//! Derived from `experiments/g3-d3d11` and `experiments/g4-glyph-atlas`, which established the
//! device/swapchain shape and measured the cost of getting here.

use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP, D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST,
};
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Buffer, ID3D11Device, ID3D11DeviceContext, ID3D11PixelShader,
    ID3D11RenderTargetView, ID3D11Texture2D, ID3D11VertexShader, D3D11_BIND_CONSTANT_BUFFER,
    D3D11_BUFFER_DESC, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_CREATE_DEVICE_FLAG,
    D3D11_SDK_VERSION, D3D11_SUBRESOURCE_DATA, D3D11_USAGE_DEFAULT, D3D11_VIEWPORT,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory2, IDXGIFactory2, IDXGISwapChain1, DXGI_CREATE_FACTORY_FLAGS, DXGI_PRESENT,
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

/// DXBC produced by `fxc` in `build.rs`. Embedded, so nothing compiles a shader at runtime and
/// the binary carries no `d3dcompiler_47.dll` import (`SPEC.md` §3.2).
const BACKGROUND_VS: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/background_vs.cso"));
const BACKGROUND_PS: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/background_ps.cso"));

pub struct Gpu {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    pub driver: Driver,
    swapchain: Option<IDXGISwapChain1>,
    rtv: Option<ID3D11RenderTargetView>,
    vs: ID3D11VertexShader,
    ps: ID3D11PixelShader,
    frame_cb: ID3D11Buffer,
    size: (u32, u32),
}

fn create_device(driver: Driver) -> Result<(ID3D11Device, ID3D11DeviceContext)> {
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
            None,
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

impl Gpu {
    pub fn new() -> Result<Self> {
        let (device, context, driver) = match create_device(Driver::Hardware) {
            Ok((d, c)) => (d, c, Driver::Hardware),
            Err(_) => {
                let (d, c) = create_device(Driver::Warp)?;
                (d, c, Driver::Warp)
            }
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

        Ok(Self {
            device,
            context,
            driver,
            swapchain: None,
            rtv: None,
            vs: vs.expect("vertex shader out param is set on success"),
            ps: ps.expect("pixel shader out param is set on success"),
            frame_cb: frame_cb.expect("constant buffer out param is set on success"),
            size: (1, 1),
        })
    }

    pub fn attach(&mut self, window: WindowHandle, width: u32, height: u32) -> Result<()> {
        if self.swapchain.is_some() {
            return Ok(());
        }
        // CreateDXGIFactory2 rather than walking device → adapter → parent, so the factory is a
        // 1.2+ interface and `CreateSwapChainForHwnd` exists on it.
        let factory: IDXGIFactory2 = unsafe { CreateDXGIFactory2(DXGI_CREATE_FACTORY_FLAGS(0))? };
        let desc = DXGI_SWAP_CHAIN_DESC1 {
            Width: width.max(1),
            Height: height.max(1),
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
            BufferCount: 3,
            SwapEffect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
            ..Default::default()
        };
        let swapchain = unsafe {
            factory.CreateSwapChainForHwnd(&self.device, HWND(window.0 as _), &desc, None, None)?
        };
        self.swapchain = Some(swapchain);
        self.size = (width.max(1), height.max(1));
        self.create_rtv()
    }

    fn create_rtv(&mut self) -> Result<()> {
        let Some(sc) = self.swapchain.as_ref() else {
            return Ok(());
        };
        let back: ID3D11Texture2D = unsafe { sc.GetBuffer(0)? };
        let mut rtv = None;
        unsafe {
            self.device
                .CreateRenderTargetView(&back, None, Some(&mut rtv))?;
        }
        self.rtv = rtv;
        Ok(())
    }

    pub fn resize(&mut self, width: u32, height: u32) -> Result<()> {
        if self.swapchain.is_none() {
            return Ok(());
        }
        // The RTV holds a reference to the back buffer and ResizeBuffers fails while it is alive.
        self.rtv = None;
        self.size = (width.max(1), height.max(1));
        {
            let sc = self.swapchain.as_ref().expect("checked above");
            unsafe {
                sc.ResizeBuffers(
                    0,
                    width.max(1),
                    height.max(1),
                    DXGI_FORMAT_B8G8R8A8_UNORM,
                    DXGI_SWAP_CHAIN_FLAG(0),
                )?;
            }
        }
        self.create_rtv()
    }

    /// Draws the background as one fullscreen triangle through the embedded shaders.
    ///
    /// A `ClearRenderTargetView` would produce the same pixels more cheaply. Drawing it exercises
    /// the offline-compiled path — bytecode, shader creation, constant buffer, draw — which is the
    /// point at M0: the pipeline the grid depends on at M3 is proven now rather than then.
    pub fn draw_background(&mut self, colour: [f32; 4]) -> Result<()> {
        let Some(rtv) = self.rtv.clone() else {
            return Ok(());
        };
        let (w, h) = self.size;
        unsafe {
            self.context
                .UpdateSubresource(&self.frame_cb, 0, None, colour.as_ptr().cast(), 0, 0);
            self.context.OMSetRenderTargets(Some(&[Some(rtv)]), None);
            self.context.RSSetViewports(Some(&[D3D11_VIEWPORT {
                TopLeftX: 0.0,
                TopLeftY: 0.0,
                Width: w as f32,
                Height: h as f32,
                MinDepth: 0.0,
                MaxDepth: 1.0,
            }]));
            self.context
                .IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
            self.context.VSSetShader(&self.vs, None);
            self.context.PSSetShader(&self.ps, None);
            self.context
                .PSSetConstantBuffers(0, Some(&[Some(self.frame_cb.clone())]));
            self.context.Draw(3, 0);
        }
        Ok(())
    }

    pub fn present(&self) -> Result<()> {
        if let Some(sc) = &self.swapchain {
            unsafe { sc.Present(1, DXGI_PRESENT(0)).ok()? };
        }
        Ok(())
    }
}
