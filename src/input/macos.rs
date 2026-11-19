//! macOS synthetic mouse input, via Core Graphics events.
//!
//! Compiled on `cfg(target_os = "macos")` with the `input` and `macos`
//! features. The Core Graphics entry points are declared directly rather than
//! pulled in as a dependency, the same way `capture::macos` reaches
//! `NSApplicationLoad`.
//!
//! Two macOS-specific details matter:
//!
//! * **Posting events needs Accessibility permission.** `CGEventPost` returns
//!   nothing and fails silently without it, so the trust check happens up
//!   front, where it can say what to grant.
//! * **A move with a button held is a different event.** Applications track
//!   drags through `…MouseDragged`, not `MouseMoved`, so this backend
//!   remembers which button is down and posts accordingly.

use crate::error::InputError;
use crate::input::{MouseBackend, MouseButton, Point};
use std::ffi::c_void;

type CGEventRef = *mut c_void;
type CGEventSourceRef = *mut c_void;

/// Core Graphics' floating-point point, in global display coordinates.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct CGPoint {
    x: f64,
    y: f64,
}

// CGEventType values.
const K_CG_EVENT_LEFT_MOUSE_DOWN: u32 = 1;
const K_CG_EVENT_LEFT_MOUSE_UP: u32 = 2;
const K_CG_EVENT_RIGHT_MOUSE_DOWN: u32 = 3;
const K_CG_EVENT_RIGHT_MOUSE_UP: u32 = 4;
const K_CG_EVENT_MOUSE_MOVED: u32 = 5;
const K_CG_EVENT_LEFT_MOUSE_DRAGGED: u32 = 6;
const K_CG_EVENT_RIGHT_MOUSE_DRAGGED: u32 = 7;
const K_CG_EVENT_OTHER_MOUSE_DOWN: u32 = 25;
const K_CG_EVENT_OTHER_MOUSE_UP: u32 = 26;
const K_CG_EVENT_OTHER_MOUSE_DRAGGED: u32 = 27;

// CGMouseButton values.
const K_CG_MOUSE_BUTTON_LEFT: u32 = 0;
const K_CG_MOUSE_BUTTON_RIGHT: u32 = 1;
const K_CG_MOUSE_BUTTON_CENTER: u32 = 2;

/// `kCGMouseEventClickState`: how many clicks this event is part of.
const K_CG_MOUSE_EVENT_CLICK_STATE: u32 = 1;
/// `kCGScrollEventUnitLine`: scroll in lines rather than pixels.
const K_CG_SCROLL_EVENT_UNIT_LINE: u32 = 1;
/// `kCGHIDEventTap`: post at the lowest level, as real hardware would.
const K_CG_HID_EVENT_TAP: u32 = 0;

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGEventCreate(source: CGEventSourceRef) -> CGEventRef;
    fn CGEventCreateMouseEvent(
        source: CGEventSourceRef,
        mouse_type: u32,
        cursor: CGPoint,
        button: u32,
    ) -> CGEventRef;
    fn CGEventCreateScrollWheelEvent2(
        source: CGEventSourceRef,
        units: u32,
        wheel_count: u32,
        wheel1: i32,
        wheel2: i32,
        wheel3: i32,
    ) -> CGEventRef;
    fn CGEventGetLocation(event: CGEventRef) -> CGPoint;
    fn CGEventSetIntegerValueField(event: CGEventRef, field: u32, value: i64);
    fn CGEventPost(tap: u32, event: CGEventRef);
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRelease(cf: *const c_void);
}

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXIsProcessTrusted() -> bool;
}

/// Post `event` and release it. Takes ownership of a `CGEvent*`.
fn post(event: CGEventRef) -> Result<(), InputError> {
    if event.is_null() {
        return Err(InputError::Backend(
            "Core Graphics refused to create the event".into(),
        ));
    }
    unsafe {
        CGEventPost(K_CG_HID_EVENT_TAP, event);
        CFRelease(event);
    }
    Ok(())
}

/// The Core Graphics pointer backend.
#[derive(Debug, Default)]
pub struct MacosMouse {
    /// The button currently held, if any: a move while one is down has to be
    /// posted as a drag.
    held: Option<MouseButton>,
}

impl MacosMouse {
    /// Construct the backend, checking that this process may post events.
    pub fn new() -> Result<Self, InputError> {
        if !unsafe { AXIsProcessTrusted() } {
            return Err(InputError::Backend(
                "this process is not trusted to post input events. Grant it Accessibility \
                 access in System Settings > Privacy & Security > Accessibility (for a CLI, \
                 add the terminal application running it), then try again."
                    .into(),
            ));
        }
        Ok(Self { held: None })
    }

    fn location(&self) -> Result<CGPoint, InputError> {
        let event = unsafe { CGEventCreate(std::ptr::null_mut()) };
        if event.is_null() {
            return Err(InputError::Backend(
                "Core Graphics would not report the pointer position".into(),
            ));
        }
        let point = unsafe { CGEventGetLocation(event) };
        unsafe { CFRelease(event) };
        Ok(point)
    }
}

/// The Core Graphics button identifier for a framewatch button.
fn cg_button(button: MouseButton) -> u32 {
    match button {
        MouseButton::Left => K_CG_MOUSE_BUTTON_LEFT,
        MouseButton::Right => K_CG_MOUSE_BUTTON_RIGHT,
        MouseButton::Middle => K_CG_MOUSE_BUTTON_CENTER,
    }
}

/// The event type for pressing or releasing a button.
fn transition_event(button: MouseButton, down: bool) -> u32 {
    match (button, down) {
        (MouseButton::Left, true) => K_CG_EVENT_LEFT_MOUSE_DOWN,
        (MouseButton::Left, false) => K_CG_EVENT_LEFT_MOUSE_UP,
        (MouseButton::Right, true) => K_CG_EVENT_RIGHT_MOUSE_DOWN,
        (MouseButton::Right, false) => K_CG_EVENT_RIGHT_MOUSE_UP,
        (MouseButton::Middle, true) => K_CG_EVENT_OTHER_MOUSE_DOWN,
        (MouseButton::Middle, false) => K_CG_EVENT_OTHER_MOUSE_UP,
    }
}

/// The event type for moving the pointer, given what is held.
fn move_event(held: Option<MouseButton>) -> u32 {
    match held {
        None => K_CG_EVENT_MOUSE_MOVED,
        Some(MouseButton::Left) => K_CG_EVENT_LEFT_MOUSE_DRAGGED,
        Some(MouseButton::Right) => K_CG_EVENT_RIGHT_MOUSE_DRAGGED,
        Some(MouseButton::Middle) => K_CG_EVENT_OTHER_MOUSE_DRAGGED,
    }
}

impl MouseBackend for MacosMouse {
    fn move_to(&mut self, point: Point) -> Result<(), InputError> {
        let event = unsafe {
            CGEventCreateMouseEvent(
                std::ptr::null_mut(),
                move_event(self.held),
                CGPoint {
                    x: point.x as f64,
                    y: point.y as f64,
                },
                cg_button(self.held.unwrap_or_default()),
            )
        };
        post(event)
    }

    fn button(
        &mut self,
        button: MouseButton,
        down: bool,
        click_index: u32,
    ) -> Result<(), InputError> {
        let at = self.location()?;
        let event = unsafe {
            CGEventCreateMouseEvent(
                std::ptr::null_mut(),
                transition_event(button, down),
                at,
                cg_button(button),
            )
        };
        if event.is_null() {
            return Err(InputError::Backend(
                "Core Graphics refused to create the event".into(),
            ));
        }
        // Unlike Windows, macOS does not infer a double-click from timing: the
        // click state has to be carried on the event itself.
        unsafe {
            CGEventSetIntegerValueField(
                event,
                K_CG_MOUSE_EVENT_CLICK_STATE,
                click_index.max(1) as i64,
            );
        }
        self.held = if down { Some(button) } else { None };
        post(event)
    }

    fn scroll(&mut self, dx: i32, dy: i32) -> Result<(), InputError> {
        // Core Graphics' horizontal axis runs the opposite way to the
        // `dx`-positive-right contract shared with Windows and X11.
        let event = unsafe {
            CGEventCreateScrollWheelEvent2(
                std::ptr::null_mut(),
                K_CG_SCROLL_EVENT_UNIT_LINE,
                2,
                dy,
                -dx,
                0,
            )
        };
        post(event)
    }

    fn cursor_position(&mut self) -> Result<Point, InputError> {
        let point = self.location()?;
        Ok(Point::new(point.x.round() as i32, point.y.round() as i32))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_held_button_turns_moves_into_drags() {
        assert_eq!(move_event(None), K_CG_EVENT_MOUSE_MOVED);
        assert_eq!(
            move_event(Some(MouseButton::Left)),
            K_CG_EVENT_LEFT_MOUSE_DRAGGED
        );
        assert_eq!(
            move_event(Some(MouseButton::Middle)),
            K_CG_EVENT_OTHER_MOUSE_DRAGGED
        );
    }

    #[test]
    fn transitions_map_to_their_button() {
        assert_eq!(
            transition_event(MouseButton::Right, true),
            K_CG_EVENT_RIGHT_MOUSE_DOWN
        );
        assert_eq!(
            transition_event(MouseButton::Right, false),
            K_CG_EVENT_RIGHT_MOUSE_UP
        );
        assert_eq!(cg_button(MouseButton::Middle), K_CG_MOUSE_BUTTON_CENTER);
    }

    /// Reading the pointer position touches no other process, so it is a safe
    /// end-to-end check that the Core Graphics bindings are wired up right.
    #[test]
    fn reads_the_real_pointer_position() {
        if !unsafe { AXIsProcessTrusted() } {
            eprintln!("skipping: this process is not trusted for Accessibility");
            return;
        }
        let mut mouse = MacosMouse::new().unwrap();
        let point = mouse.cursor_position().unwrap();
        // Any real desktop position; the assertion is that the call returned.
        assert!(point.x > -100_000 && point.y > -100_000);
    }
}
