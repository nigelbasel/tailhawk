//! G4 — glyph atlas composition: colour and mono in one pass.
//!
//! `PLAN.md` §3 G4. The claim under test is the parenthetical in the "if it fails" column:
//! *"a premultiplied colour atlas cannot share the mono atlas's blend state, so it breaks the
//! one-instanced-draw rule that the whole renderer design rests on."*

#![windows_subsystem = "windows"]

mod atlas;
mod gpu;
mod grid;
mod text;

use std::cell::RefCell;

use windows::core::{Result, PCWSTR};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
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
    static STATE: RefCell<Option<App>> = const { RefCell::new(None) };
}

pub fn now() -> i64 {
    let mut t = 0i64;
    unsafe { QueryPerformanceCounter(&mut t).expect("QPC") };
    t
}

pub fn qpc_freq() -> i64 {
    let mut f = 0i64;
    unsafe { QueryPerformanceFrequency(&mut f).expect("QPF") };
    f
}

struct App {
    gpu: gpu::Gpu,
    grid: Option<grid::Grid>,
    freq: i64,
    reported: bool,
}

impl App {
    fn new() -> Result<Self> {
        Ok(Self {
            gpu: gpu::Gpu::new()?,
            grid: None,
            freq: qpc_freq(),
            reported: false,
        })
    }

    fn paint(&mut self, hwnd: HWND) -> Result<()> {
        let mut rc = RECT::default();
        unsafe { GetClientRect(hwnd, &mut rc)? };
        let (w, h) = (
            (rc.right - rc.left).max(1) as u32,
            (rc.bottom - rc.top).max(1) as u32,
        );
        self.gpu.attach(hwnd, w, h)?;

        if self.grid.is_none() {
            self.grid = Some(grid::Grid::new(&self.gpu)?);
        }
        let grid = self.grid.as_mut().expect("created above");
        grid.render(&self.gpu, w, h)?;
        self.gpu.present()?;

        if !self.reported {
            self.reported = true;
            let report = grid.report(self.freq, self.gpu.driver);
            emit(&report);
        }
        Ok(())
    }
}

thread_local! {
    static STEP: RefCell<String> = const { RefCell::new(String::new()) };
}

/// Breadcrumb so an HRESULT from deep in the COM setup names the call that produced it.
pub fn step(name: &str) {
    STEP.with(|s| *s.borrow_mut() = name.to_string());
}

pub fn last_step() -> String {
    STEP.with(|s| s.borrow().clone())
}

/// Shader-compile diagnostics and other failures, appended so nothing is lost.
pub fn emit_err(text: &str) {
    let wide = format!("g4 ERROR: {text}\0")
        .encode_utf16()
        .collect::<Vec<u16>>();
    unsafe {
        windows::Win32::System::Diagnostics::Debug::OutputDebugStringW(PCWSTR(wide.as_ptr()));
    }
    let path = std::env::temp_dir().join("g4-glyph-atlas-errors.txt");
    let prev = std::fs::read_to_string(&path).unwrap_or_default();
    let _ = std::fs::write(path, format!("{prev}{text}\n"));
}

fn emit(text: &str) {
    let wide = format!("{text}\0").encode_utf16().collect::<Vec<u16>>();
    unsafe {
        windows::Win32::System::Diagnostics::Debug::OutputDebugStringW(PCWSTR(wide.as_ptr()));
    }
    let _ = std::fs::write(std::env::temp_dir().join("g4-glyph-atlas.txt"), text);
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_PAINT => {
            STATE.with(|s| {
                if let Some(app) = s.borrow_mut().as_mut() {
                    if let Err(e) = app.paint(hwnd) {
                        emit(&format!("PAINT FAILED after '{}': {e:?}", last_step()));
                    }
                }
            });
            unsafe {
                let _ = ValidateRect(hwnd, None);
            };
            LRESULT(0)
        }
        WM_SIZE => {
            STATE.with(|s| {
                if let Some(app) = s.borrow_mut().as_mut() {
                    let mut rc = RECT::default();
                    unsafe {
                        let _ = GetClientRect(hwnd, &mut rc);
                    };
                    let _ = app.gpu.resize(
                        (rc.right - rc.left).max(1) as u32,
                        (rc.bottom - rc.top).max(1) as u32,
                    );
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
    // No console under `windows_subsystem = "windows"`, so a panic would otherwise be silent.
    std::panic::set_hook(Box::new(|info| {
        emit_err(&format!("PANIC: {info}"));
    }));

    let instance: HINSTANCE = unsafe { GetModuleHandleW(None)?.into() };
    let class_name = windows::core::w!("TailhawkG4Atlas");

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
        *s.borrow_mut() = Some(App::new()?);
        Ok(())
    })?;

    let hwnd = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            class_name,
            windows::core::w!("Tailhawk G4 — glyph atlas"),
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
