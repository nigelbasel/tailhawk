#![windows_subsystem = "windows"]

use std::cell::RefCell;

use windows::core::{Result, PCWSTR};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Direct2D::Common::{D2D1_COLOR_F, D2D_SIZE_U};
use windows::Win32::Graphics::Direct2D::{
    D2D1CreateFactory, ID2D1Factory, ID2D1HwndRenderTarget, D2D1_FACTORY_TYPE_SINGLE_THREADED,
    D2D1_HWND_RENDER_TARGET_PROPERTIES, D2D1_PRESENT_OPTIONS_NONE, D2D1_RENDER_TARGET_PROPERTIES,
};
use windows::Win32::Graphics::Gdi::ValidateRect;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetClientRect, GetMessageW, LoadCursorW,
    PostQuitMessage, RegisterClassW, ShowWindow, TranslateMessage, CS_HREDRAW, CS_VREDRAW,
    CW_USEDEFAULT, IDC_ARROW, MSG, SW_SHOW, WINDOW_EX_STYLE, WM_DESTROY, WM_PAINT, WM_SIZE,
    WNDCLASSW, WS_OVERLAPPEDWINDOW,
};

thread_local! {
    static STATE: RefCell<Option<Renderer>> = const { RefCell::new(None) };
}

fn now() -> i64 {
    let mut t = 0i64;
    unsafe { QueryPerformanceCounter(&mut t).expect("QPC") };
    t
}

struct Renderer {
    factory: ID2D1Factory,
    target: Option<ID2D1HwndRenderTarget>,
    start: i64,
    freq: i64,
    reported: bool,
    t_factory: i64,
    t_target: i64,
    t_window: i64,
}

impl Renderer {
    fn new(start: i64, freq: i64) -> Result<Self> {
        let factory: ID2D1Factory =
            unsafe { D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)? };
        let t_factory = now();
        Ok(Self {
            factory,
            target: None,
            start,
            freq,
            reported: false,
            t_factory,
            t_target: 0,
            t_window: 0,
        })
    }

    fn mark_window_created(&mut self) {
        self.t_window = now();
    }

    fn ensure_target(&mut self, hwnd: HWND) -> Result<()> {
        if self.target.is_some() {
            return Ok(());
        }
        let mut rc = RECT::default();
        unsafe { GetClientRect(hwnd, &mut rc)? };
        let size = D2D_SIZE_U {
            width: (rc.right - rc.left).max(1) as u32,
            height: (rc.bottom - rc.top).max(1) as u32,
        };
        let hwnd_props = D2D1_HWND_RENDER_TARGET_PROPERTIES {
            hwnd,
            pixelSize: size,
            presentOptions: D2D1_PRESENT_OPTIONS_NONE,
        };
        let target = unsafe {
            self.factory
                .CreateHwndRenderTarget(&D2D1_RENDER_TARGET_PROPERTIES::default(), &hwnd_props)?
        };
        self.target = Some(target);
        if self.t_target == 0 {
            self.t_target = now();
        }
        Ok(())
    }

    fn paint(&mut self, hwnd: HWND) -> Result<()> {
        self.ensure_target(hwnd)?;
        let target = self.target.as_ref().expect("target created above");
        unsafe {
            target.BeginDraw();
            target.Clear(Some(&D2D1_COLOR_F {
                r: 0.09,
                g: 0.10,
                b: 0.12,
                a: 1.0,
            }));
            target.EndDraw(None, None)?;
        }
        if !self.reported {
            self.reported = true;
            let t_paint = now();
            let ms = |a: i64, b: i64| (b - a) as f64 * 1000.0 / self.freq as f64;
            report(Phases {
                factory: ms(self.start, self.t_factory),
                window: ms(self.t_factory, self.t_window),
                target: ms(self.t_window, self.t_target),
                first_draw: ms(self.t_target, t_paint),
                total: ms(self.start, t_paint),
            });
        }
        Ok(())
    }

    fn discard_target(&mut self) {
        self.target = None;
    }
}

struct Phases {
    factory: f64,
    window: f64,
    target: f64,
    first_draw: f64,
    total: f64,
}

fn report(p: Phases) {
    let line = format!(
        "{:.3},{:.3},{:.3},{:.3},{:.3}",
        p.factory, p.window, p.target, p.first_draw, p.total
    );
    let text = format!("g3-d2d {line}\0")
        .encode_utf16()
        .collect::<Vec<u16>>();
    unsafe {
        windows::Win32::System::Diagnostics::Debug::OutputDebugStringW(PCWSTR(text.as_ptr()));
    }
    let _ = std::fs::write(
        std::env::temp_dir().join("g3-d2d-first-pixel.txt"),
        format!("{line}\n"),
    );
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_PAINT => {
            STATE.with(|s| {
                if let Some(r) = s.borrow_mut().as_mut() {
                    let _ = r.paint(hwnd);
                }
            });
            unsafe {
                let _ = ValidateRect(hwnd, None);
            };
            LRESULT(0)
        }
        WM_SIZE => {
            STATE.with(|s| {
                if let Some(r) = s.borrow_mut().as_mut() {
                    r.discard_target();
                }
            });
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
    let mut start = 0i64;
    let mut freq = 0i64;
    unsafe {
        QueryPerformanceCounter(&mut start)?;
        QueryPerformanceFrequency(&mut freq)?;
    }

    let instance: HINSTANCE = unsafe { GetModuleHandleW(None)?.into() };
    let class_name = windows::core::w!("TailhawkG3D2D");

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

    STATE.with(|s| -> Result<()> {
        *s.borrow_mut() = Some(Renderer::new(start, freq)?);
        Ok(())
    })?;

    let hwnd = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            class_name,
            windows::core::w!("Tailhawk G3 — D2D"),
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

    STATE.with(|s| {
        if let Some(r) = s.borrow_mut().as_mut() {
            r.mark_window_created();
        }
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
