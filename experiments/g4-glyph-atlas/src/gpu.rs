//! D3D11 device, swapchain and the SPEC §3 fallback chain.

use windows::core::{Interface, Result};
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP};
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11RenderTargetView, ID3D11Texture2D,
    D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_CREATE_DEVICE_FLAG,
    D3D11_MAPPED_SUBRESOURCE, D3D11_MAP_READ, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC,
    D3D11_USAGE_STAGING,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory2, IDXGIDevice, IDXGIFactory2, IDXGISwapChain1, DXGI_CREATE_FACTORY_FLAGS,
    DXGI_PRESENT, DXGI_SWAP_CHAIN_DESC1, DXGI_SWAP_CHAIN_FLAG, DXGI_SWAP_EFFECT_FLIP_DISCARD,
    DXGI_USAGE_RENDER_TARGET_OUTPUT,
};

/// Which driver the device actually came up on. SPEC §3 mandates an explicit chain
/// hardware → WARP, and mandates never panicking on device loss.
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

pub struct Gpu {
    pub device: ID3D11Device,
    pub context: ID3D11DeviceContext,
    pub driver: Driver,
    swapchain: Option<IDXGISwapChain1>,
    rtv: Option<ID3D11RenderTargetView>,
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
            // BGRA_SUPPORT keeps a future D2D interop path open, which SPEC §3's third
            // fallback rung needs.
            D3D11_CREATE_DEVICE_BGRA_SUPPORT | D3D11_CREATE_DEVICE_FLAG(0),
            None,
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            Some(&mut context),
        )?;
    }
    Ok((
        device.expect("device out param set on success"),
        context.expect("context out param set on success"),
    ))
}

impl Gpu {
    /// Walks the SPEC §3 chain. Returns the first rung that comes up.
    pub fn new() -> Result<Self> {
        let (device, context, driver) = match create_device(Driver::Hardware) {
            Ok((d, c)) => (d, c, Driver::Hardware),
            Err(_) => {
                let (d, c) = create_device(Driver::Warp)?;
                (d, c, Driver::Warp)
            }
        };
        Ok(Self {
            device,
            context,
            driver,
            swapchain: None,
            rtv: None,
        })
    }

    pub fn attach(&mut self, hwnd: HWND, width: u32, height: u32) -> Result<()> {
        if self.swapchain.is_some() {
            return Ok(());
        }
        let dxgi_device: IDXGIDevice = self.device.cast()?;
        let adapter = unsafe { dxgi_device.GetAdapter()? };
        // CreateDXGIFactory2 rather than adapter.GetParent so the factory is a 1.2+ interface
        // and CreateSwapChainForHwnd is available.
        let _ = adapter;
        let factory: IDXGIFactory2 =
            unsafe { CreateDXGIFactory2(DXGI_CREATE_FACTORY_FLAGS(0))? };

        let desc = DXGI_SWAP_CHAIN_DESC1 {
            Width: width.max(1),
            Height: height.max(1),
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
            // Three buffers so an unthrottled Present is not immediately blocked on the queue.
            BufferCount: 3,
            SwapEffect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
            ..Default::default()
        };
        let swapchain =
            unsafe { factory.CreateSwapChainForHwnd(&self.device, hwnd, &desc, None, None)? };
        self.swapchain = Some(swapchain);
        self.create_rtv()?;
        Ok(())
    }

    fn create_rtv(&mut self) -> Result<()> {
        let sc = self.swapchain.as_ref().expect("attach() called first");
        let back: ID3D11Texture2D = unsafe { sc.GetBuffer(0)? };
        let mut rtv = None;
        unsafe {
            self.device
                .CreateRenderTargetView(&back, None, Some(&mut rtv))?;
        }
        self.rtv = rtv;
        Ok(())
    }

    pub fn rtv(&self) -> Option<&ID3D11RenderTargetView> {
        self.rtv.as_ref()
    }

    pub fn resize(&mut self, width: u32, height: u32) -> Result<()> {
        if self.swapchain.is_none() {
            return Ok(());
        }
        self.rtv = None;
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

    pub fn present(&self) -> Result<()> {
        if let Some(sc) = &self.swapchain {
            unsafe { sc.Present(1, DXGI_PRESENT(0)).ok()? };
        }
        Ok(())
    }

    /// Copy the back buffer to system memory as BGRA8. Used to verify what was actually drawn
    /// rather than inferring correctness from timings.
    pub fn readback(&self, width: u32, height: u32) -> Result<Vec<u8>> {
        let sc = self.swapchain.as_ref().expect("attach() first");
        let back: ID3D11Texture2D = unsafe { sc.GetBuffer(0)? };
        let desc = D3D11_TEXTURE2D_DESC {
            Width: width,
            Height: height,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_STAGING,
            BindFlags: 0,
            CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
            MiscFlags: 0,
        };
        let mut staging = None;
        unsafe { self.device.CreateTexture2D(&desc, None, Some(&mut staging))? };
        let staging = staging.expect("staging texture");
        unsafe { self.context.CopyResource(&staging, &back) };

        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        unsafe {
            self.context
                .Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))?
        };
        let mut out = vec![0u8; (width * height * 4) as usize];
        unsafe {
            for y in 0..height as usize {
                let src = (mapped.pData as *const u8).add(y * mapped.RowPitch as usize);
                let dst = out.as_mut_ptr().add(y * width as usize * 4);
                std::ptr::copy_nonoverlapping(src, dst, width as usize * 4);
            }
            self.context.Unmap(&staging, 0);
        }
        Ok(out)
    }

    /// Present without waiting for vblank. Benchmarks must not be quantised to the refresh rate.
    pub fn present_now(&self) -> Result<()> {
        if let Some(sc) = &self.swapchain {
            unsafe { sc.Present(0, DXGI_PRESENT(0)).ok()? };
        }
        Ok(())
    }
}
