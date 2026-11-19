//! X11 synthetic mouse input, via the XTEST extension.
//!
//! Compiled on `cfg(target_os = "linux")` with the `input` and `linux-x11`
//! features. Like `capture::linux`, this is X11 only: a Wayland compositor
//! does not let a client synthesize input for other clients, which needs a
//! portal-mediated remote-desktop session instead.

use crate::error::InputError;
use crate::frame::WindowInfo;
use crate::input::{MouseBackend, MouseButton, Point};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    Atom, ClientMessageEvent, ConfigureWindowAux, ConnectionExt as _, EventMask, InputFocus,
    StackMode, Window, BUTTON_PRESS_EVENT, BUTTON_RELEASE_EVENT, MOTION_NOTIFY_EVENT,
};
use x11rb::protocol::xtest::ConnectionExt as _;
use x11rb::rust_connection::RustConnection;

/// XTEST wants an absolute destination for a motion event, not a delta.
const ABSOLUTE_MOTION: u8 = 0;
/// XTEST's "core pointer" device.
const CORE_POINTER: u8 = 0;

/// X11 core button numbers.
const BUTTON_LEFT: u8 = 1;
const BUTTON_MIDDLE: u8 = 2;
const BUTTON_RIGHT: u8 = 3;
const BUTTON_SCROLL_UP: u8 = 4;
const BUTTON_SCROLL_DOWN: u8 = 5;
const BUTTON_SCROLL_LEFT: u8 = 6;
const BUTTON_SCROLL_RIGHT: u8 = 7;

/// The XTEST pointer backend.
pub struct X11Mouse {
    conn: RustConnection,
    root: Window,
}

impl X11Mouse {
    /// Connect to the X server named by `DISPLAY` and check for XTEST.
    pub fn new() -> Result<Self, InputError> {
        let (conn, screen) = x11rb::connect(None).map_err(platform_error)?;
        let root = conn.setup().roots[screen].root;
        // Ask for XTEST 2.2 up front: without the extension every later call
        // would fail one at a time with a much less useful message.
        conn.xtest_get_version(2, 2)
            .map_err(platform_error)?
            .reply()
            .map_err(|e| {
                InputError::Backend(format!(
                    "the X server does not offer the XTEST extension, which synthetic input \
                     needs ({e}). A Wayland session cannot provide it to a plain client."
                ))
            })?;
        Ok(Self { conn, root })
    }

    fn fake(&self, type_: u8, detail: u8, x: i16, y: i16) -> Result<(), InputError> {
        self.conn
            .xtest_fake_input(
                type_,
                detail,
                x11rb::CURRENT_TIME,
                self.root,
                x,
                y,
                CORE_POINTER,
            )
            .map_err(platform_error)?;
        // XTEST is asynchronous; without a flush the events sit in the request
        // buffer while the plan's sleeps elapse, and arrive in a burst.
        self.conn.flush().map_err(platform_error)
    }

    fn click_button(&self, button: u8) -> Result<(), InputError> {
        self.fake(BUTTON_PRESS_EVENT, button, 0, 0)?;
        self.fake(BUTTON_RELEASE_EVENT, button, 0, 0)
    }
}

fn x11_button(button: MouseButton) -> u8 {
    match button {
        MouseButton::Left => BUTTON_LEFT,
        MouseButton::Middle => BUTTON_MIDDLE,
        MouseButton::Right => BUTTON_RIGHT,
    }
}

impl MouseBackend for X11Mouse {
    fn move_to(&mut self, point: Point) -> Result<(), InputError> {
        let clamp = |v: i32| v.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
        self.fake(
            MOTION_NOTIFY_EVENT,
            ABSOLUTE_MOTION,
            clamp(point.x),
            clamp(point.y),
        )
    }

    fn button(
        &mut self,
        button: MouseButton,
        down: bool,
        // X11 clients derive multi-clicks from event timing, so the plan's
        // click index needs no restating here.
        _click_index: u32,
    ) -> Result<(), InputError> {
        let type_ = if down {
            BUTTON_PRESS_EVENT
        } else {
            BUTTON_RELEASE_EVENT
        };
        self.fake(type_, x11_button(button), 0, 0)
    }

    fn scroll(&mut self, dx: i32, dy: i32) -> Result<(), InputError> {
        // X11 has no wheel axis: a notch is a click of button 4/5 (vertical)
        // or 6/7 (horizontal).
        let vertical = if dy > 0 {
            BUTTON_SCROLL_UP
        } else {
            BUTTON_SCROLL_DOWN
        };
        for _ in 0..dy.unsigned_abs() {
            self.click_button(vertical)?;
        }
        let horizontal = if dx > 0 {
            BUTTON_SCROLL_RIGHT
        } else {
            BUTTON_SCROLL_LEFT
        };
        for _ in 0..dx.unsigned_abs() {
            self.click_button(horizontal)?;
        }
        Ok(())
    }

    fn cursor_position(&mut self) -> Result<Point, InputError> {
        let pointer = self
            .conn
            .query_pointer(self.root)
            .map_err(platform_error)?
            .reply()
            .map_err(platform_error)?;
        Ok(Point::new(pointer.root_x.into(), pointer.root_y.into()))
    }
}

/// Raise and focus `window`.
///
/// Asks the window manager first, through EWMH's `_NET_ACTIVE_WINDOW`, since
/// that respects focus-stealing policy and virtual desktops. Falls back to
/// setting focus directly for the plain window managers that ignore it.
pub fn focus_window(window: &WindowInfo) -> Result<(), InputError> {
    let (conn, screen) = x11rb::connect(None).map_err(platform_error)?;
    let root = conn.setup().roots[screen].root;
    let target = window.hwnd as Window;

    if let Ok(active) = atom(&conn, "_NET_ACTIVE_WINDOW") {
        // data[0] = 2: the request comes from a pager / direct user action,
        // which window managers honour rather than defer.
        let event = ClientMessageEvent::new(32, target, active, [2u32, 0, 0, 0, 0]);
        let sent = conn.send_event(
            false,
            root,
            EventMask::SUBSTRUCTURE_NOTIFY | EventMask::SUBSTRUCTURE_REDIRECT,
            event,
        );
        if sent.is_ok() && conn.flush().is_ok() {
            return Ok(());
        }
    }

    let _ = conn.configure_window(
        target,
        &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE),
    );
    let _ = conn.set_input_focus(InputFocus::PARENT, target, x11rb::CURRENT_TIME);
    conn.flush().map_err(platform_error)
}

fn atom(conn: &RustConnection, name: &str) -> Result<Atom, InputError> {
    Ok(conn
        .intern_atom(false, name.as_bytes())
        .map_err(platform_error)?
        .reply()
        .map_err(platform_error)?
        .atom)
}

fn platform_error(error: impl std::fmt::Display) -> InputError {
    InputError::Backend(format!(
        "X11 synthetic input failed: {error}. Run framewatch in an X11 session with DISPLAY set \
         (Wayland does not let a client synthesize input for other clients)"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buttons_map_to_x11_numbers() {
        assert_eq!(x11_button(MouseButton::Left), 1);
        assert_eq!(x11_button(MouseButton::Middle), 2);
        assert_eq!(x11_button(MouseButton::Right), 3);
    }
}
