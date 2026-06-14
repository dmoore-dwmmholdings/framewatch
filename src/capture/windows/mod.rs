//! Windows-only capture: window enumeration and the Graphics Capture backend.
//!
//! Compiled only on `cfg(windows)` with the `wgc` feature.

pub mod enumerate;
pub mod wgc;

use crate::frame::{Rect, WindowInfo};
use ::windows::Win32::Foundation::{HWND, RECT};
use ::windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
use ::windows::Win32::UI::HiDpi::GetDpiForWindow;
use ::windows::Win32::UI::WindowsAndMessaging::{
    GetClassNameW, GetClientRect, GetForegroundWindow, GetWindowRect,
};

/// Convert an `isize` handle to a Win32 `HWND`.
#[inline]
pub(crate) fn hwnd_from_isize(h: isize) -> HWND {
    HWND(h as *mut std::ffi::c_void)
}

fn rect_from_win32(r: RECT) -> Rect {
    Rect::new(
        r.left,
        r.top,
        (r.right - r.left).max(0) as u32,
        (r.bottom - r.top).max(0) as u32,
    )
}

/// Whether a window is DWM-cloaked (hidden virtual-desktop / UWP suspended).
pub(crate) fn is_cloaked(hwnd: HWND) -> bool {
    let mut cloaked: u32 = 0;
    let ok = unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED,
            (&mut cloaked as *mut u32).cast(),
            std::mem::size_of::<u32>() as u32,
        )
    };
    ok.is_ok() && cloaked != 0
}

fn class_name(hwnd: HWND) -> String {
    let mut buf = [0u16; 256];
    let len = unsafe { GetClassNameW(hwnd, &mut buf) };
    if len <= 0 {
        return String::new();
    }
    String::from_utf16_lossy(&buf[..len as usize])
}

/// Assemble a [`WindowInfo`] for `hwnd`, given a title and exe already resolved
/// via `windows-capture`'s `Window`.
pub(crate) fn fill_window_info(hwnd: HWND, title: String, exe: String) -> WindowInfo {
    let mut wr = RECT::default();
    let _ = unsafe { GetWindowRect(hwnd, &mut wr) };
    let mut cr = RECT::default();
    let _ = unsafe { GetClientRect(hwnd, &mut cr) };
    let dpi = unsafe { GetDpiForWindow(hwnd) };
    let foreground = unsafe { GetForegroundWindow() } == hwnd;

    WindowInfo {
        hwnd: hwnd.0 as isize,
        title,
        exe,
        class: class_name(hwnd),
        rect: rect_from_win32(wr),
        client_rect: rect_from_win32(cr),
        dpi: if dpi == 0 { 96 } else { dpi },
        foreground,
    }
}
