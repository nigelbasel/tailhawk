//! G3, second leg — first pixel on **D3D11 + DXGI**, the stack `SPEC.md` §3 actually specifies.
//!
//! `experiments/g3-d2d` measured D2D's `HwndRenderTarget` and concluded that graphics device
//! creation must come off the critical path, offering two untested directions. This measures the
//! specified stack and *tests the first of those directions*: creating the D3D11 device on a worker
//! thread concurrently with `CreateWindowExW`.
//!
//! Four modes, selected by argv:
//!   `serial`     — window, then device, then swapchain. The naive order.
//!   `concurrent` — device creation starts at `main()` entry on a worker thread while the main
//!                  thread creates the window; the swapchain is attached once both exist.
//!   `earlypaint` — as `concurrent`, but the first `WM_PAINT` fills the client area with GDI
//!                  **before** the device is ready, so a pixel is on screen without waiting for any
//!                  D3D initialisation. This is the surviving direction from the D2D leg's list.
//!   `classbrush` — as `earlypaint`, but the fill comes from the window class background brush, so
//!                  the system erases during ShowWindow and no paint handler runs at all.
//!
//! Writes one CSV line to `%TEMP%\g3-d3d11-<mode>.txt`:
//! `mode,window,device_or_wait,swapchain,draw,total` in milliseconds — and for `earlypaint`,
//! `mode,window,first_pixel,d3d_ready` instead, because the two are different questions.

#![windows_subsystem = "windows"]

use std::cell::RefCell;
use std::sync::mpsc;

use windows::core::{Result, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
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
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateSolidBrush, DeleteObject, EndPaint, FillRect, ValidateRect, HBRUSH,
    PAINTSTRUCT,
};
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
    device: Option<ID3D11Device>,
    context: Option<ID3D11DeviceContext>,
    driver: &'static str,
    /// `earlypaint` only: the worker's device, not yet arrived.
    pending: Option<mpsc::Receiver<Result<(ID3D11Device, ID3D11DeviceContext, &'static str)>>>,
    mode: &'static str,
    swapchain: Option<IDXGISwapChain1>,
    rtv: Option<ID3D11RenderTargetView>,
    reported: bool,

    start: i64,
    t_window: i64,
    t_device: i64,
    t_swapchain: i64,
    /// `earlypaint` only: when the GDI fill put the first pixel on screen.
    t_early: i64,
}

/// `earlypaint`: fill the client area with a solid colour straight from `BeginPaint`'s HDC. No D3D,
/// no DXGI, nothing that can block on driver initialisation — just the window and GDI.
fn gdi_fill(hwnd: HWND) {
    unsafe {
        let mut ps = PAINTSTRUCT::default();
        let hdc = BeginPaint(hwnd, &mut ps);
        if !hdc.is_invalid() {
            let mut rc = RECT::default();
            let _ = GetClientRect(hwnd, &mut rc);
            // 0x001A1917 is the same 0.09/0.10/0.12 background the D3D path clears to, as BGR.
            let brush = CreateSolidBrush(COLORREF(0x001A1917));
            FillRect(hdc, &rc, brush);
            let _ = DeleteObject(brush);
        }
        let _ = EndPaint(hwnd, &ps);
    }
}

impl App {
    fn paint(&mut self, hwnd: HWND) -> Result<()> {
        // earlypaint: get a pixel up with GDI first, then keep re-checking for the device. The
        // whole point is that this path never touches D3D or DXGI, so it cannot block on driver
        // initialisation — the only prerequisites are an HWND and a device context.
        if self.mode == "earlypaint" || self.mode == "classbrush" {
            // classbrush needs no fill of its own — the system already erased with the class
            // brush, and t_early was stamped right after ShowWindow returned.
            if self.t_early == 0 && self.mode == "earlypaint" {
                gdi_fill(hwnd);
                self.t_early = now();
            }
            // The pixel is already on screen, so blocking here costs the user nothing — it is
            // exactly what the real app would do: show something, then bring up the renderer.
            if let Some(rx) = self.pending.take() {
                let (d, c, n) = rx.recv().expect("device thread did not report")?;
                self.device = Some(d);
                self.context = Some(c);
                self.driver = n;
                self.t_device = now();
            }
        }

        let (Some(device), Some(context)) = (self.device.clone(), self.context.clone()) else {
            return Ok(());
        };

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
            let sc = unsafe { factory.CreateSwapChainForHwnd(&device, hwnd, &desc, None, None)? };
            let back: ID3D11Texture2D = unsafe { sc.GetBuffer(0)? };
            let mut rtv = None;
            unsafe { device.CreateRenderTargetView(&back, None, Some(&mut rtv))? };
            self.swapchain = Some(sc);
            self.rtv = rtv;
            if self.t_swapchain == 0 {
                self.t_swapchain = now();
            }
        }

        let rtv = self.rtv.as_ref().expect("created above");
        let sc = self.swapchain.as_ref().expect("created above");
        unsafe {
            context.ClearRenderTargetView(rtv, &[0.09, 0.10, 0.12, 1.0]);
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
            let line = if self.mode == "earlypaint" || self.mode == "classbrush" {
                // The two numbers that matter here are different from the other modes: how soon a
                // pixel was on screen, and how much later the real renderer became usable.
                format!(
                    "{},{:.3},{:.3},{:.3},{}",
                    self.mode,
                    ms(self.start, self.t_window),
                    ms(self.start, self.t_early),
                    ms(self.start, t_draw),
                    self.driver,
                )
            } else {
                format!(
                    "{},{:.3},{:.3},{:.3},{:.3},{:.3},{}",
                    self.mode,
                    ms(self.start, self.t_window),
                    ms(self.t_window, self.t_device),
                    ms(self.t_device, self.t_swapchain),
                    ms(self.t_swapchain, t_draw),
                    ms(self.start, t_draw),
                    self.driver,
                )
            };
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
            // Deliberately does NOT PostQuitMessage. A subject that exits on its own is never
            // reaped under the agent's shell — its handle lingers and the dead process keeps its
            // D3D device. Session 5 accumulated 49 such zombies and watched D3D11CreateDevice
            // degrade from 55 ms to over 1200 ms, which silently corrupted every measurement taken
            // after the first. g3-d2d never self-terminates, gets killed by the harness, and leaked
            // exactly zero. So: report, then wait to be killed.
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
                            windows::Win32::System::Diagnostics::Debug::OutputDebugStringW(PCWSTR(
                                t.as_ptr(),
                            ));
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
    let args: Vec<String> = std::env::args().collect();
    let mode = if args.iter().any(|a| a == "classbrush") {
        "classbrush"
    } else if args.iter().any(|a| a == "earlypaint") {
        "earlypaint"
    } else if args.iter().any(|a| a == "concurrent") {
        "concurrent"
    } else {
        "serial"
    };
    // Both off-main-thread modes start the device before touching the window, so they overlap.
    let off_thread = mode != "serial";

    // windows-rs marks the D3D11 interfaces `Send`, so they can cross the channel directly — no
    // `into_raw` round trip, which would not be `Send` and would leak on an early return.
    let rx = if off_thread {
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
    // classbrush: give the *class* a background brush. The system then erases the window with it
    // as part of showing it, so a pixel is on screen without a WM_PAINT handler running at all —
    // which is where earlypaint's remaining ~48 ms was going.
    let wc = WNDCLASSW {
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: Some(wndproc),
        hInstance: instance,
        hCursor: unsafe { LoadCursorW(None, IDC_ARROW)? },
        hbrBackground: if mode == "classbrush" {
            unsafe { CreateSolidBrush(COLORREF(0x001A1917)) }
        } else {
            HBRUSH::default()
        },
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

    // earlypaint deliberately does NOT wait for the device here — the whole point is to reach
    // WM_PAINT and get a pixel up while the worker is still initialising the driver.
    let (device, context, driver, pending, t_device) =
        if mode == "earlypaint" || mode == "classbrush" {
            (None, None, "pending", rx, 0)
        } else {
            let (d, c, n) = match rx {
                Some(rx) => rx.recv().expect("device thread did not report")?,
                None => create_device()?,
            };
            (Some(d), Some(c), n, None, now())
        };

    STATE.with(|s| {
        *s.borrow_mut() = Some(App {
            device,
            context,
            driver,
            pending,
            mode,
            swapchain: None,
            rtv: None,
            reported: false,
            start,
            t_window,
            t_device,
            t_swapchain: 0,
            t_early: 0,
        });
    });

    unsafe {
        let _ = ShowWindow(hwnd, SW_SHOW);
    };

    // classbrush: the system erased the client area with the class brush as part of showing the
    // window, so the pixel is already up. Stamp it here — there is no paint handler to stamp it in.
    if mode == "classbrush" {
        let t = now();
        STATE.with(|s| {
            if let Some(app) = s.borrow_mut().as_mut() {
                app.t_early = t;
            }
        });
    }

    let mut msg = MSG::default();
    while unsafe { GetMessageW(&mut msg, None, 0, 0) }.as_bool() {
        unsafe {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    Ok(())
}
