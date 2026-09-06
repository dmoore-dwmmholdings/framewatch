//! Windows synthetic mouse input, via `SendInput`.
//!
//! Compiled on `cfg(windows)` with the `input` and `wgc` features.
//!
//! Two Windows-specific details matter for landing a click where the agent
//! meant it:
//!
//! * **Absolute moves are normalized.** `SendInput` takes 0..=65535 across the
//!   whole virtual desktop, not pixels, so every move is scaled through
//!   [`virtual_screen`].
//! * **The captured region is not the window rect.** Graphics Capture delivers
//!   the DWM frame, while `GetWindowRect` includes the invisible resize border
//!   (about 7px per side on a resizable window). Window-space coordinates come
//!   off a captured frame, so they are measured against
//!   `DWMWA_EXTENDED_FRAME_BOUNDS`.

use crate::capture::windows::hwnd_from_isize;
use crate::error::InputError;
use crate::frame::{Rect, WindowInfo};
use crate::input::{MouseBackend, MouseButton, Point};
use ::windows::Win32::Foundation::{POINT, RECT};
use ::windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_EXTENDED_FRAME_BOUNDS};
use ::windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
use ::windows::Win32::UI::HiDpi::{
    SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use ::windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_HWHEEL,
    MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP,
    MOUSEEVENTF_MOVE, MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_VIRTUALDESK,
    MOUSEEVENTF_WHEEL, MOUSEINPUT,
};
use ::windows::Win32::UI::WindowsAndMessaging::{
    GetCursorPos, GetForegroundWindow, GetSystemMetrics, GetWindowThreadProcessId, IsIconic,
    SetForegroundWindow, ShowWindow, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN,
    SM_YVIRTUALSCREEN, SW_RESTORE,
};

/// One wheel notch, in the units `MOUSEINPUT::mouseData` expects.
const WHEEL_NOTCH: i32 = 120;

/// The `SendInput` pointer backend.
#[derive(Debug, Default)]
pub struct WindowsMouse;

impl WindowsMouse {
    /// Construct the backend. Cheap: `SendInput` needs no handle or session.
    pub fn new() -> Self {
        Self
    }
}

impl MouseBackend for WindowsMouse {
    fn move_to(&mut self, point: Point) -> Result<(), InputError> {
        let (dx, dy) = normalize(point);
        send(MOUSEINPUT {
            dx,
            dy,
            mouseData: 0,
            dwFlags: MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
            time: 0,
            dwExtraInfo: 0,
        })
    }

    fn button(
        &mut self,
        button: MouseButton,
        down: bool,
        // Windows derives multi-clicks from the interval between events, so the
        // plan's click index is nothing this backend has to restate.
        _click_index: u32,
    ) -> Result<(), InputError> {
        let flags = match (button, down) {
            (MouseButton::Left, true) => MOUSEEVENTF_LEFTDOWN,
            (MouseButton::Left, false) => MOUSEEVENTF_LEFTUP,
            (MouseButton::Right, true) => MOUSEEVENTF_RIGHTDOWN,
            (MouseButton::Right, false) => MOUSEEVENTF_RIGHTUP,
            (MouseButton::Middle, true) => MOUSEEVENTF_MIDDLEDOWN,
            (MouseButton::Middle, false) => MOUSEEVENTF_MIDDLEUP,
        };
        send(MOUSEINPUT {
            dx: 0,
            dy: 0,
            mouseData: 0,
            dwFlags: flags,
            time: 0,
            dwExtraInfo: 0,
        })
    }

    fn scroll(&mut self, dx: i32, dy: i32) -> Result<(), InputError> {
        // A single event carries one axis, so a diagonal scroll is two events.
        if dy != 0 {
            send(MOUSEINPUT {
                dx: 0,
                dy: 0,
                mouseData: notches(dy),
                dwFlags: MOUSEEVENTF_WHEEL,
                time: 0,
                dwExtraInfo: 0,
            })?;
        }
        if dx != 0 {
            send(MOUSEINPUT {
                dx: 0,
                dy: 0,
                mouseData: notches(dx),
                dwFlags: MOUSEEVENTF_HWHEEL,
                time: 0,
                dwExtraInfo: 0,
            })?;
        }
        Ok(())
    }

    fn cursor_position(&mut self) -> Result<Point, InputError> {
        let mut point = POINT::default();
        unsafe { GetCursorPos(&mut point) }
            .map_err(|e| InputError::Backend(format!("GetCursorPos failed: {e}")))?;
        Ok(Point::new(point.x, point.y))
    }
}

/// `mouseData` for `n` wheel notches, as Windows' signed-in-a-u32 field.
fn notches(n: i32) -> u32 {
    n.saturating_mul(WHEEL_NOTCH) as u32
}

fn send(mi: MOUSEINPUT) -> Result<(), InputError> {
    let input = INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 { mi },
    };
    let sent = unsafe { SendInput(&[input], std::mem::size_of::<INPUT>() as i32) };
    if sent == 0 {
        return Err(InputError::Backend(
            "SendInput delivered no events — the event was blocked. This is usually \
             UIPI: a process at normal integrity cannot send input to an elevated \
             window, so run framewatch elevated if the target application is."
                .into(),
        ));
    }
    Ok(())
}

/// The bounding rect of all displays, in physical pixels.
///
/// `SendInput`'s absolute coordinates are normalized against this rect rather
/// than against the primary display, so a window on a secondary monitor (which
/// has negative coordinates when it sits left of the primary) is reachable.
pub fn virtual_screen() -> Rect {
    let metric = |index| unsafe { GetSystemMetrics(index) };
    let w = metric(SM_CXVIRTUALSCREEN);
    let h = metric(SM_CYVIRTUALSCREEN);
    if w > 0 && h > 0 {
        Rect::new(
            metric(SM_XVIRTUALSCREEN),
            metric(SM_YVIRTUALSCREEN),
            w as u32,
            h as u32,
        )
    } else {
        // The metrics are unavailable (a session with no desktop, say). A 1x1
        // fallback keeps the arithmetic total; the move simply won't be useful.
        Rect::new(0, 0, 1, 1)
    }
}

/// Scale an absolute screen pixel into `SendInput`'s 0..=65535 range.
///
/// The normalized form is coarser than pixels on a wide desktop, and Windows
/// rounds back to pixels with its own arithmetic, so a move can land within a
/// pixel of the request. Clicks are unaffected in practice; if a target ever
/// needs exact placement, this scaling is the single place to correct it.
fn normalize(point: Point) -> (i32, i32) {
    scale_to_virtual_screen(point, virtual_screen())
}

/// The pure half of [`normalize`], separated so it can be tested off-device.
fn scale_to_virtual_screen(point: Point, screen: Rect) -> (i32, i32) {
    // 65535 maps onto the *last* pixel, so the divisor is one less than the
    // extent: with `w` used directly, the rightmost column is unreachable.
    let axis = |value: i32, origin: i32, extent: u32| -> i32 {
        let span = (extent as i64 - 1).max(1);
        let offset = (value as i64 - origin as i64).clamp(0, span);
        ((offset * 65535 + span / 2) / span) as i32
    };
    (
        axis(point.x, screen.x, screen.w),
        axis(point.y, screen.y, screen.h),
    )
}

/// The screen rect a Graphics Capture frame of `window` covers.
pub fn capture_region(window: &WindowInfo) -> Rect {
    let hwnd = hwnd_from_isize(window.hwnd);
    let mut bounds = RECT::default();
    let result = unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            (&mut bounds as *mut RECT).cast(),
            std::mem::size_of::<RECT>() as u32,
        )
    };
    if result.is_ok() && bounds.right > bounds.left && bounds.bottom > bounds.top {
        Rect::new(
            bounds.left,
            bounds.top,
            (bounds.right - bounds.left) as u32,
            (bounds.bottom - bounds.top) as u32,
        )
    } else {
        // DWM composition off, or the window went away: the window rect is the
        // best remaining answer, and it is right to within the resize border.
        window.rect
    }
}

/// Bring `window` to the foreground.
///
/// Windows only lets the foreground process (or one it launched) call
/// `SetForegroundWindow`, so this falls back to briefly attaching to the
/// current foreground window's input queue — the standard workaround for a
/// tool that must activate a window it does not own. Failure is reported as a
/// warning, not an error: a click at the window's coordinates usually
/// activates it anyway.
pub fn focus_window(window: &WindowInfo) -> Result<(), InputError> {
    let hwnd = hwnd_from_isize(window.hwnd);
    if unsafe { IsIconic(hwnd) }.as_bool() {
        let _ = unsafe { ShowWindow(hwnd, SW_RESTORE) };
    }
    if unsafe { SetForegroundWindow(hwnd) }.as_bool() {
        return Ok(());
    }

    let foreground = unsafe { GetForegroundWindow() };
    let theirs = unsafe { GetWindowThreadProcessId(foreground, None) };
    let ours = unsafe { GetCurrentThreadId() };
    if theirs == 0 || theirs == ours {
        tracing::warn!("framewatch: could not bring the target window to the foreground");
        return Ok(());
    }

    let attached = unsafe { AttachThreadInput(ours, theirs, true) }.as_bool();
    let raised = unsafe { SetForegroundWindow(hwnd) }.as_bool();
    if attached {
        let _ = unsafe { AttachThreadInput(ours, theirs, false) };
    }
    if !raised {
        tracing::warn!("framewatch: could not bring the target window to the foreground");
    }
    Ok(())
}

/// Opt this process into per-monitor DPI awareness.
///
/// Without it a scaled display hands us virtualized window rects while
/// Graphics Capture delivers physical pixels, so window-space coordinates
/// would be wrong by the scale factor. Best effort: a process that is already
/// DPI-aware (via a manifest, say) reports an error here, which is the state
/// we wanted anyway.
pub fn ensure_dpi_aware() {
    let _ = unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalization_spans_the_whole_axis() {
        let screen = Rect::new(0, 0, 1920, 1080);
        assert_eq!(scale_to_virtual_screen(Point::new(0, 0), screen), (0, 0));
        assert_eq!(
            scale_to_virtual_screen(Point::new(1919, 1079), screen),
            (65535, 65535)
        );
        let (mid, _) = scale_to_virtual_screen(Point::new(960, 0), screen);
        assert!((32750..32800).contains(&mid), "midpoint was {mid}");
    }

    #[test]
    fn normalization_is_relative_to_the_virtual_desktop_origin() {
        // A second monitor to the left of the primary starts at a negative x.
        let screen = Rect::new(-1920, 0, 3840, 1080);
        assert_eq!(scale_to_virtual_screen(Point::new(-1920, 0), screen).0, 0);
        assert_eq!(
            scale_to_virtual_screen(Point::new(1919, 0), screen).0,
            65535
        );
    }

    #[test]
    fn normalization_clamps_off_desktop_points() {
        let screen = Rect::new(0, 0, 1920, 1080);
        assert_eq!(
            scale_to_virtual_screen(Point::new(-50, -50), screen),
            (0, 0)
        );
        assert_eq!(
            scale_to_virtual_screen(Point::new(99_999, 99_999), screen),
            (65535, 65535)
        );
    }

    #[test]
    fn wheel_notches_are_signed() {
        assert_eq!(notches(1), 120);
        assert_eq!(notches(-1) as i32, -120);
    }
}
