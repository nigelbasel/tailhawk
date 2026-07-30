//! G3, second leg — first pixel on **D3D11 + DXGI**, the stack `SPEC.md` §3 actually specifies.
//!
//! `experiments/g3-d2d` measured D2D's `HwndRenderTarget` and concluded that graphics device
//! creation must come off the critical path, offering two untested directions. This measures the
//! specified stack and *tests the first of those directions*: creating the D3D11 device on a worker
//! thread concurrently with `CreateWindowExW`.
//!
//! Two modes, selected by argv:
//!   `serial`     — window, then device, then swapchain. The naive order.
//!   `concurrent` — device creation starts at `main()` entry on a worker thread while the main
//!                  thread creates the window; the swapchain is attached once both exist.
//!
//! Writes one CSV line to `%TEMP%\g3-d3d11-<mode>.txt`:
//! `mode,window,device_or_wait,swapchain,draw,total` in milliseconds.

#![windows_subsystem = "windows"]

use std::cell::RefCell;
use std::sync::mpsc;

use windows::core::{Result, PCWSTR};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP};
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11RenderTargetView, ID3D11Texture2D,
    D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory2, IDXGIFactory2, IDXGISwapChain1, DXGI_CREATE_FACTORY_FLAGS, DXGI_PRESENT,
    DXGI_SWAP_CHAIN_DESC1, DXGI_SWAP_EFFECT_FLIP_DISCARD, DXGI_USAGE_RENDER_TARGET_OUTPUT,
};
use windows::Win32::Graphics::Gdi::ValidateRect;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetClientRect, GetMessageW, LoadCursorW,
    PostQuitMessage, RegisterClassW, ShowWindow, TranslateMessage, CS_HREDRAW, CS_VREDRAW,
    CW_USEDEFAULT, IDC_ARROW, MSG, SW_SHOW, WINDOW_EX_STYLE, WM_DESTROY, WM_PAINT, WNDCLASSW,
    WS_OVERLAPPEDWINDOW,
};

thread_local! {
    static STATE: RefCell<Option<App>> = const { RefCell::new(None) };
}

fn now() -> i64 {
    let mut t = 0i64;
    unsafe { QueryPerformanceCounter(&mut t).expect("QPC") };
    t
}

fn freq() -> i64 {
    let mut f = 0i64;
    unsafe { QueryPerformanceFrequency(&mut f).expect("QPF") };
    f
}

/// `SPEC.md` §3 mandates hardware → WARP explicitly.
fn create_device() -> Result<(ID3D11Device, ID3D11DeviceContext, &'static str)> {
    for (kind, name) in [
        (D3D_DRIVER_TYPE_HARDWARE, "hardware"),
        (D3D_DRIVER_TYPE_WARP, "WARP"),
    ] {
        let mut device = None;
        let mut context = None;
        let hr = unsafe {
            D3D11CreateDevice(
                None,
                kind,
                None,
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut context),
            )
        };
        if hr.is_ok() {
            return Ok((
                device.expect("device on success"),
                context.expect("context on success"),
                name,
            ));
        }
    }
    Err(windows::core::Error::from_win32())
}

struct App {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    driver: &'static str,
    mode: &'static str,
    swapchain: Option<IDXGISwapChain1>,
    rtv: Option<ID3D11RenderTargetView>,
    reported: bool,

    start: i64,
    t_window: i64,
    t_device: i64,
    t_swapchain: i64,
}

impl App {
    fn paint(&mut self, hwnd: HWND) -> Result<()> {
        if self.swapchain.is_none() {
            let mut rc = RECT::default();
            unsafe { GetClientRect(hwnd, &mut rc)? };
            let factory: IDXGIFactory2 =
                unsafe { CreateDXGIFactory2(DXGI_CREATE_FACTORY_FLAGS(0))? };
            let desc = DXGI_SWAP_CHAIN_DESC1 {
                Width: (rc.right - rc.left).max(1) as u32,
                Height: (rc.bottom - rc.top).max(1) as u32,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
                BufferCount: 2,
                SwapEffect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
                ..Default::default()
            };
            let sc =
                unsafe { factory.CreateSwapChainForHwnd(&self.device, hwnd, &desc, None, None)? };
            let back: ID3D11Texture2D = unsafe { sc.GetBuffer(0)? };
            let mut rtv = None;
            unsafe {
                self.device
                    .CreateRenderTargetView(&back, None, Some(&mut rtv))?
            };
            self.swapchain = Some(sc);
            self.rtv = rtv;
            if self.t_swapchain == 0 {
                self.t_swapchain = now();
            }
        }

        let rtv = self.rtv.as_ref().expect("created above");
        let sc = self.swapchain.as_ref().expect("created above");
        unsafe {
            self.context
                .ClearRenderTargetView(rtv, &[0.09, 0.10, 0.12, 1.0]);
            // Interval 0: measure to the point the frame is submitted. As in the D2D leg, the
            // figure is a lower bound — a photon still costs up to one refresh interval more.
            sc.Present(0, DXGI_PRESENT(0)).ok()?;
        }

        if !self.reported {
            self.reported = true;
            let t_draw = now();
            let f = freq() as f64;
            let ms = |a: i64, b: i64| (b - a) as f64 * 1000.0 / f;
            // In concurrent mode `t_device` is when the worker's device was *received*, so the
            // column is a wait, not a cost — it is near zero when the device won the race.
            let line = format!(
                "{},{:.3},{:.3},{:.3},{:.3},{:.3},{}",
                self.mode,
                ms(self.start, self.t_window),
                ms(self.t_window, self.t_device),
                ms(self.t_device, self.t_swapchain),
                ms(self.t_swapchain, t_draw),
                ms(self.start, t_draw),
                self.driver,
            );
            let wide = format!("g3-d3d11 {line}\0")
                .encode_utf16()
                .collect::<Vec<u16>>();
            unsafe {
                windows::Win32::System::Diagnostics::Debug::OutputDebugStringW(PCWSTR(
                    wide.as_ptr(),
                ));
            }
            let _ = std::fs::write(
                std::env::temp_dir().join(format!("g3-d3d11-{}.txt", self.mode)),
                format!("{line}\n"),
            );
            unsafe { PostQuitMessage(0) };
        }
        Ok(())
    }
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_PAINT => {
            STATE.with(|s| {
                if let Some(app) = s.borrow_mut().as_mut() {
                    if let Err(e) = app.paint(hwnd) {
                        let t = format!("g3-d3d11 PAINT FAILED: {e:?}\0")
                            .encode_utf16()
                            .collect::<Vec<u16>>();
                        unsafe {
                            windows::Win32::System::Diagnostics::Debug::OutputDebugStringW(
                                PCWSTR(t.as_ptr()),
                            );
                        }
                    }
                }
            });
            unsafe {
                let _ = ValidateRect(hwnd, None);
            };
            LRESULT(0)
        }
        WM_DESTROY => {
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

fn main() -> Result<()> {
    let start = now();
    let concurrent = std::env::args().any(|a| a == "concurrent");
    let mode = if concurrent { "concurrent" } else { "serial" };

    // Start the device on a worker thread *before* touching the window, so the two overlap.
    // windows-rs marks the D3D11 interfaces `Send`, so they can cross the channel directly — no
    // `into_raw` round trip, which would not be `Send` and would leak on an early return.
    let rx = if concurrent {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(create_device());
        });
        Some(rx)
    } else {
        None
    };

    let instance: HINSTANCE = unsafe { GetModuleHandleW(None)?.into() };
    let class_name = windows::core::w!("TailhawkG3D3D11");
    let wc = WNDCLASSW {
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: Some(wndproc),
        hInstance: instance,
        hCursor: unsafe { LoadCursorW(None, IDC_ARROW)? },
        lpszClassName: class_name,
        ..Default::default()
    };
    if unsafe { RegisterClassW(&wc) } == 0 {
        return Err(windows::core::Error::from_win32());
    }
    let hwnd = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            class_name,
            windows::core::w!("Tailhawk G3 — D3D11 + DXGI"),
            WS_OVERLAPPEDWINDOW,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            1280,
            800,
            None,
            None,
            instance,
            None,
        )?
    };
    let t_window = now();

    let (device, context, driver) = match rx {
        Some(rx) => rx.recv().expect("device thread did not report")?,
        None => create_device()?,
    };
    let t_device = now();

    STATE.with(|s| {
        *s.borrow_mut() = Some(App {
            device,
            context,
            driver,
            mode,
            swapchain: None,
            rtv: None,
            reported: false,
            start,
            t_window,
            t_device,
            t_swapchain: 0,
        });
    });

    unsafe {
        let _ = ShowWindow(hwnd, SW_SHOW);
    };

    let mut msg = MSG::default();
    while unsafe { GetMessageW(&mut msg, None, 0, 0) }.as_bool() {
        unsafe {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    Ok(())
}
