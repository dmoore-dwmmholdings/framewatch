//! Window enumeration via `windows-capture`'s `Window`, enriched with class/DPI
//! info from the `windows` crate.

use crate::capture::windows::{fill_window_info, hwnd_from_isize, is_cloaked};
use crate::error::CaptureError;
use crate::frame::WindowInfo;
use windows_capture::window::Window;

/// Enumerate capturable top-level windows (visible, non-cloaked, non-zero-size).
pub fn enumerate_windows() -> Result<Vec<WindowInfo>, CaptureError> {
    let windows = Window::enumerate().map_err(|e| CaptureError::Backend(e.to_string()))?;
    let mut out = Vec::with_capacity(windows.len());
    for w in windows {
        let hwnd_ptr = w.as_raw_hwnd();
        let hwnd = hwnd_from_isize(hwnd_ptr as isize);
        if is_cloaked(hwnd) {
            continue;
        }
        let title = w.title().unwrap_or_default();
        let exe = w.process_name().unwrap_or_default();
        let info = fill_window_info(hwnd, title, exe);
        if info.rect.w == 0 || info.rect.h == 0 {
            continue;
        }
        out.push(info);
    }
    Ok(out)
}
