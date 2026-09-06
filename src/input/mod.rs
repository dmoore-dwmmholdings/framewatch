//! Synthetic mouse input: let an agent *act* on the window it is watching.
//!
//! Framewatch's capture side answers "what does the window look like now?".
//! This module answers the other half: "click there". An agent reads a frame,
//! works out a pixel it wants to press, and drives the real OS pointer at it —
//! so the click lands in the application under test exactly as a human's would.
//!
//! ## Coordinates
//!
//! Frames are window-relative, so input is too. A [`CoordSpace::Window`] point
//! is measured from the top-left of the *captured region* of the target window,
//! which is the same origin the saved PNG uses; [`CoordSpace::Screen`] points
//! are absolute desktop pixels. If capture was cropped with an ROI, pass the
//! same rect here ([`InputRequest::roi`]) and coordinates read off the cropped
//! image keep working unchanged.
//!
//! Window-space points are bounds-checked against the captured region, so an
//! agent's arithmetic slip is reported rather than clicked into whatever else
//! happens to be on the desktop. [`InputRequest::allow_outside`] opts out.
//!
//! ## Structure
//!
//! Like the detection engine, the interesting logic is pure and platform-free:
//! [`plan`] turns high-level [`MouseAction`]s into a flat list of
//! [`MouseOp`]s (absolute moves, button transitions, wheel ticks, sleeps),
//! which [`execute`] hands to a [`MouseBackend`]. The platform backends
//! (`SendInput`, `CGEvent`, X11 `XTEST`) are thin, and [`RecordingMouse`]
//! stands in for them in tests.
//!
//! ```no_run
//! use framewatch::input::{self, CoordSpace, InputRequest, MouseAction, MouseButton, Point};
//! use framewatch::Target;
//!
//! # fn main() -> Result<(), framewatch::Error> {
//! let request = InputRequest::new(vec![MouseAction::Click {
//!     button: MouseButton::Left,
//!     at: Some(Point::new(420, 260)),
//!     count: 1,
//! }])
//! .target(Target::ByTitleRegex("My App".into()))
//! .space(CoordSpace::Window);
//!
//! input::perform(&request)?;
//! # Ok(()) }
//! ```

use crate::config::Target;
use crate::error::{Error, InputError};
use crate::frame::{Rect, WindowInfo};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;
use std::time::Duration;

#[cfg(all(windows, feature = "input", feature = "wgc"))]
pub mod windows;

#[cfg(all(target_os = "macos", feature = "input", feature = "macos"))]
pub mod macos;

#[cfg(all(target_os = "linux", feature = "input", feature = "linux-x11"))]
pub mod linux;

/// A point in integer pixel coordinates.
///
/// Serializes as a two-element array (`[x, y]`) so action scripts stay terse.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(from = "[i32; 2]", into = "[i32; 2]")]
pub struct Point {
    /// Horizontal coordinate, in pixels.
    pub x: i32,
    /// Vertical coordinate, in pixels.
    pub y: i32,
}

impl Point {
    /// Construct a point.
    pub const fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }
}

impl From<[i32; 2]> for Point {
    fn from([x, y]: [i32; 2]) -> Self {
        Self { x, y }
    }
}

impl From<Point> for [i32; 2] {
    fn from(p: Point) -> Self {
        [p.x, p.y]
    }
}

impl fmt::Display for Point {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{},{}", self.x, self.y)
    }
}

impl FromStr for Point {
    type Err = InputError;

    /// Parse an `X,Y` pair (whitespace around either number is ignored).
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (x, y) = s
            .split_once(',')
            .ok_or_else(|| InputError::Request(format!("expected X,Y, got {s:?}")))?;
        let parse = |part: &str, which: &str| {
            part.trim().parse::<i32>().map_err(|_| {
                InputError::Request(format!(
                    "{which} in {s:?} is not an integer pixel coordinate"
                ))
            })
        };
        Ok(Self::new(parse(x, "X")?, parse(y, "Y")?))
    }
}

/// Which physical mouse button an action uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MouseButton {
    /// The primary button.
    #[default]
    Left,
    /// The secondary (context-menu) button.
    Right,
    /// The wheel / middle button.
    Middle,
}

impl fmt::Display for MouseButton {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Left => "left",
            Self::Right => "right",
            Self::Middle => "middle",
        })
    }
}

impl FromStr for MouseButton {
    type Err = InputError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "left" | "l" | "primary" => Ok(Self::Left),
            "right" | "r" | "secondary" => Ok(Self::Right),
            "middle" | "m" | "wheel" => Ok(Self::Middle),
            other => Err(InputError::Request(format!(
                "unknown mouse button {other:?} (expected left, right, or middle)"
            ))),
        }
    }
}

/// What the coordinates in an action are measured against.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoordSpace {
    /// Relative to the top-left of the target window's captured region — the
    /// same origin as the pixels in a framewatch frame. Needs a target window.
    #[default]
    Window,
    /// Absolute desktop pixels. Needs no target window.
    Screen,
}

impl fmt::Display for CoordSpace {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Window => "window",
            Self::Screen => "screen",
        })
    }
}

impl FromStr for CoordSpace {
    type Err = InputError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "window" | "win" => Ok(Self::Window),
            "screen" | "desktop" | "absolute" => Ok(Self::Screen),
            other => Err(InputError::Request(format!(
                "unknown coordinate space {other:?} (expected window or screen)"
            ))),
        }
    }
}

const fn one() -> u32 {
    1
}

const fn default_drag_steps() -> u32 {
    PlanSettings::DEFAULT_DRAG_STEPS
}

/// One high-level pointer action, as an agent would describe it.
///
/// The serde representation is the action-script format read by
/// `framewatch input script`: an object per action, tagged with `"action"`,
/// with points as `[x, y]` arrays. Omitted fields take the defaults below.
///
/// ```json
/// {"action": "click", "at": [420, 260], "button": "left", "count": 2}
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum MouseAction {
    /// Move the pointer to `to`.
    Move {
        /// Destination.
        to: Point,
    },
    /// Press and hold `button` (optionally moving to `at` first).
    Press {
        /// Which button. Defaults to the left button.
        #[serde(default)]
        button: MouseButton,
        /// Where to press. Defaults to wherever the pointer already is.
        #[serde(default)]
        at: Option<Point>,
    },
    /// Release `button` (optionally moving to `at` first).
    Release {
        /// Which button. Defaults to the left button.
        #[serde(default)]
        button: MouseButton,
        /// Where to release. Defaults to wherever the pointer already is.
        #[serde(default)]
        at: Option<Point>,
    },
    /// Press and release `button` `count` times (2 = double-click).
    Click {
        /// Which button. Defaults to the left button.
        #[serde(default)]
        button: MouseButton,
        /// Where to click. Defaults to wherever the pointer already is.
        #[serde(default)]
        at: Option<Point>,
        /// How many clicks. Defaults to 1.
        #[serde(default = "one")]
        count: u32,
    },
    /// Press at `from`, move to `to` in `steps` increments, release.
    ///
    /// The intermediate moves matter: applications that implement dragging by
    /// watching mouse-move events ignore a press/teleport/release.
    Drag {
        /// Where the drag starts. Defaults to the current pointer position.
        #[serde(default)]
        from: Option<Point>,
        /// Where the drag ends.
        to: Point,
        /// Which button to hold. Defaults to the left button.
        #[serde(default)]
        button: MouseButton,
        /// How many intermediate moves to synthesize.
        #[serde(default = "default_drag_steps")]
        steps: u32,
        /// Extra pause after pressing and before releasing, in milliseconds —
        /// drag-and-drop targets often need a moment to arm.
        #[serde(default)]
        hold_ms: u64,
    },
    /// Turn the wheel. `dy` is positive **up** (away from the user), `dx`
    /// positive **right**, both in wheel notches.
    Scroll {
        /// Where to scroll. Defaults to wherever the pointer already is.
        #[serde(default)]
        at: Option<Point>,
        /// Horizontal notches, positive right.
        #[serde(default)]
        dx: i32,
        /// Vertical notches, positive up.
        #[serde(default)]
        dy: i32,
    },
    /// Do nothing for `ms` — e.g. to let the application react before the next
    /// action, when the next action's outcome depends on it.
    Wait {
        /// How long to pause, in milliseconds.
        ms: u64,
    },
}

/// One primitive operation in absolute screen coordinates: the flattened,
/// fully-resolved form of a [`MouseAction`] that a [`MouseBackend`] performs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum MouseOp {
    /// Warp the pointer to an absolute screen pixel.
    MoveTo {
        /// Absolute screen position.
        to: Point,
    },
    /// Change a button's state.
    Button {
        /// Which button.
        button: MouseButton,
        /// `true` presses, `false` releases.
        down: bool,
        /// 1-based position within a multi-click (2 = the second click of a
        /// double-click). macOS needs this to report a double-click; Windows
        /// and X11 derive it from timing and ignore it.
        click_index: u32,
    },
    /// Turn the wheel by whole notches (`dy` positive up, `dx` positive right).
    Scroll {
        /// Horizontal notches, positive right.
        dx: i32,
        /// Vertical notches, positive up.
        dy: i32,
    },
    /// Pause. Timing is part of the plan because applications read it: a click
    /// with no dwell between press and release is often dropped.
    Sleep {
        /// How long to pause, in milliseconds.
        ms: u64,
    },
}

/// Timing knobs used when expanding actions into operations.
///
/// The defaults are deliberately unhurried — synthetic input that arrives
/// faster than a human could produce it is the usual reason a click "does
/// nothing" in a real application.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlanSettings {
    /// Dwell between a button press and its release.
    pub press_ms: u64,
    /// Gap between the clicks of a multi-click. Must stay under the platform's
    /// double-click time (500 ms by default on Windows) for `count: 2` to be
    /// seen as a double-click.
    pub multi_click_gap_ms: u64,
    /// Pause between consecutive actions.
    pub between_ms: u64,
    /// Pause after moving the pointer, before acting at the new position, so
    /// hover states have a chance to apply.
    pub move_settle_ms: u64,
    /// Pause between the intermediate moves of a drag.
    pub drag_step_ms: u64,
    /// Intermediate moves used by a drag that does not specify `steps`.
    pub drag_steps: u32,
}

impl PlanSettings {
    /// Intermediate moves in a default drag.
    pub const DEFAULT_DRAG_STEPS: u32 = 16;
}

impl Default for PlanSettings {
    fn default() -> Self {
        Self {
            press_ms: 24,
            multi_click_gap_ms: 60,
            between_ms: 40,
            move_settle_ms: 24,
            drag_step_ms: 12,
            drag_steps: Self::DEFAULT_DRAG_STEPS,
        }
    }
}

/// Maps points in a [`CoordSpace`] to absolute screen pixels, and back.
///
/// `region` is the screen rect that window-space `(0, 0)` refers to: the
/// captured region of the target window, already shifted by any ROI. A zero
/// origin and zero size means unbounded screen space.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CoordMap {
    /// The space incoming points are expressed in.
    pub space: CoordSpace,
    /// Screen rect that window-space coordinates are relative to.
    pub region: Rect,
    /// Send window-space points that fall outside `region` instead of failing.
    pub allow_outside: bool,
}

impl CoordMap {
    /// Absolute desktop coordinates; no window needed and no bounds checking.
    pub fn screen() -> Self {
        Self {
            space: CoordSpace::Screen,
            region: Rect::new(0, 0, 0, 0),
            allow_outside: true,
        }
    }

    /// Coordinates relative to `region`, a screen-space rect (typically the
    /// captured region of the target window).
    pub fn window(region: Rect) -> Self {
        Self {
            space: CoordSpace::Window,
            region,
            allow_outside: false,
        }
    }

    /// Shift the origin (and shrink the bounds) by a capture ROI, so points
    /// read off a cropped frame land in the same place they were read from.
    pub fn with_roi(mut self, roi: Rect) -> Self {
        let dx = roi.x.max(0);
        let dy = roi.y.max(0);
        let remaining_w = self.region.w.saturating_sub(dx as u32);
        let remaining_h = self.region.h.saturating_sub(dy as u32);
        self.region = Rect::new(
            self.region.x + dx,
            self.region.y + dy,
            roi.w.min(remaining_w),
            roi.h.min(remaining_h),
        );
        self
    }

    /// Allow (`true`) or reject (`false`) points outside the region.
    pub fn allow_outside(mut self, allow: bool) -> Self {
        self.allow_outside = allow;
        self
    }

    /// Whether this map bounds-checks its points.
    fn bounded(&self) -> bool {
        !self.allow_outside && self.region.w > 0 && self.region.h > 0
    }

    /// Convert a point in this map's space to absolute screen coordinates.
    pub fn to_screen(&self, point: Point) -> Result<Point, InputError> {
        if self.bounded()
            && (point.x < 0
                || point.y < 0
                || point.x >= self.region.w as i32
                || point.y >= self.region.h as i32)
        {
            return Err(InputError::OutOfBounds {
                point,
                bounds: self.region,
            });
        }
        Ok(Point::new(
            self.region.x.saturating_add(point.x),
            self.region.y.saturating_add(point.y),
        ))
    }

    /// Convert absolute screen coordinates back into this map's space.
    pub fn to_space(&self, screen: Point) -> Point {
        Point::new(
            screen.x.saturating_sub(self.region.x),
            screen.y.saturating_sub(self.region.y),
        )
    }
}

/// The straight-line path from `from` to `to` in `steps` increments, excluding
/// the start and always ending exactly on `to`.
fn interpolate(from: Point, to: Point, steps: u32) -> Vec<Point> {
    if steps <= 1 {
        return vec![to];
    }
    let steps = steps as i64;
    (1..=steps)
        .map(|i| {
            let lerp = |a: i32, b: i32| {
                let a = a as i64;
                let b = b as i64;
                (a + (b - a) * i / steps) as i32
            };
            Point::new(lerp(from.x, to.x), lerp(from.y, to.y))
        })
        .collect()
}

/// Expand high-level actions into absolute-screen operations.
///
/// Pure: no I/O, no platform calls, no clock. Every coordinate in the result is
/// already mapped through `map` and bounds-checked, so a rejected plan performs
/// nothing at all rather than half of it.
pub fn plan(
    actions: &[MouseAction],
    map: &CoordMap,
    settings: &PlanSettings,
) -> Result<Vec<MouseOp>, InputError> {
    let mut ops = Vec::new();
    // Where the pointer will be once the ops so far have run, when we know.
    // Only used to interpolate a drag whose `from` is left implicit.
    let mut at: Option<Point> = None;

    // Move to `target` if it was given, and report the resulting position.
    let move_to =
        |ops: &mut Vec<MouseOp>, target: Option<Point>| -> Result<Option<Point>, InputError> {
            match target {
                Some(point) => {
                    let screen = map.to_screen(point)?;
                    ops.push(MouseOp::MoveTo { to: screen });
                    if settings.move_settle_ms > 0 {
                        ops.push(MouseOp::Sleep {
                            ms: settings.move_settle_ms,
                        });
                    }
                    Ok(Some(screen))
                }
                None => Ok(None),
            }
        };

    for (index, action) in actions.iter().enumerate() {
        if index > 0 && settings.between_ms > 0 {
            ops.push(MouseOp::Sleep {
                ms: settings.between_ms,
            });
        }

        match action {
            MouseAction::Move { to } => {
                at = move_to(&mut ops, Some(*to))?;
            }
            MouseAction::Press { button, at: point } => {
                if let Some(moved) = move_to(&mut ops, *point)? {
                    at = Some(moved);
                }
                ops.push(MouseOp::Button {
                    button: *button,
                    down: true,
                    click_index: 1,
                });
            }
            MouseAction::Release { button, at: point } => {
                if let Some(moved) = move_to(&mut ops, *point)? {
                    at = Some(moved);
                }
                ops.push(MouseOp::Button {
                    button: *button,
                    down: false,
                    click_index: 1,
                });
            }
            MouseAction::Click {
                button,
                at: point,
                count,
            } => {
                if *count == 0 {
                    return Err(InputError::Request("click count must be at least 1".into()));
                }
                if let Some(moved) = move_to(&mut ops, *point)? {
                    at = Some(moved);
                }
                for click in 1..=*count {
                    if click > 1 && settings.multi_click_gap_ms > 0 {
                        ops.push(MouseOp::Sleep {
                            ms: settings.multi_click_gap_ms,
                        });
                    }
                    ops.push(MouseOp::Button {
                        button: *button,
                        down: true,
                        click_index: click,
                    });
                    if settings.press_ms > 0 {
                        ops.push(MouseOp::Sleep {
                            ms: settings.press_ms,
                        });
                    }
                    ops.push(MouseOp::Button {
                        button: *button,
                        down: false,
                        click_index: click,
                    });
                }
            }
            MouseAction::Drag {
                from,
                to,
                button,
                steps,
                hold_ms,
            } => {
                // No explicit start: press wherever the pointer is. We can
                // only interpolate a path if an earlier action put it somewhere.
                let start = move_to(&mut ops, *from)?.or(at);
                ops.push(MouseOp::Button {
                    button: *button,
                    down: true,
                    click_index: 1,
                });
                if *hold_ms > 0 {
                    ops.push(MouseOp::Sleep { ms: *hold_ms });
                }
                let end = map.to_screen(*to)?;
                let path = match start {
                    Some(start) => interpolate(start, end, *steps),
                    None => vec![end],
                };
                let last = path.len().saturating_sub(1);
                for (i, point) in path.into_iter().enumerate() {
                    ops.push(MouseOp::MoveTo { to: point });
                    if i < last && settings.drag_step_ms > 0 {
                        ops.push(MouseOp::Sleep {
                            ms: settings.drag_step_ms,
                        });
                    }
                }
                if *hold_ms > 0 {
                    ops.push(MouseOp::Sleep { ms: *hold_ms });
                } else if settings.move_settle_ms > 0 {
                    ops.push(MouseOp::Sleep {
                        ms: settings.move_settle_ms,
                    });
                }
                ops.push(MouseOp::Button {
                    button: *button,
                    down: false,
                    click_index: 1,
                });
                at = Some(end);
            }
            MouseAction::Scroll { at: point, dx, dy } => {
                if let Some(moved) = move_to(&mut ops, *point)? {
                    at = Some(moved);
                }
                if *dx != 0 || *dy != 0 {
                    ops.push(MouseOp::Scroll { dx: *dx, dy: *dy });
                }
            }
            MouseAction::Wait { ms } => ops.push(MouseOp::Sleep { ms: *ms }),
        }
    }
    Ok(ops)
}

/// A sink for primitive pointer operations in absolute screen coordinates.
///
/// Implemented by the platform backends and by [`RecordingMouse`].
pub trait MouseBackend {
    /// Warp the pointer to an absolute screen pixel.
    fn move_to(&mut self, point: Point) -> Result<(), InputError>;

    /// Press (`down`) or release a button. `click_index` is the 1-based
    /// position within a multi-click; backends that derive multi-clicks from
    /// timing ignore it.
    fn button(
        &mut self,
        button: MouseButton,
        down: bool,
        click_index: u32,
    ) -> Result<(), InputError>;

    /// Turn the wheel by whole notches (`dy` positive up, `dx` positive right).
    fn scroll(&mut self, dx: i32, dy: i32) -> Result<(), InputError>;

    /// The pointer's current absolute screen position.
    fn cursor_position(&mut self) -> Result<Point, InputError>;
}

/// Run a plan against `backend`, sleeping where the plan says to.
pub fn execute(ops: &[MouseOp], backend: &mut dyn MouseBackend) -> Result<(), InputError> {
    for op in ops {
        match *op {
            MouseOp::MoveTo { to } => backend.move_to(to)?,
            MouseOp::Button {
                button,
                down,
                click_index,
            } => backend.button(button, down, click_index)?,
            MouseOp::Scroll { dx, dy } => backend.scroll(dx, dy)?,
            MouseOp::Sleep { ms } => {
                if ms > 0 {
                    std::thread::sleep(Duration::from_millis(ms));
                }
            }
        }
    }
    Ok(())
}

/// A [`MouseBackend`] that records instead of acting — the input-side
/// counterpart of [`MockBackend`](crate::MockBackend). Sleeps are not recorded
/// (they belong to the plan, not the backend) and no time passes.
#[derive(Debug, Default, Clone)]
pub struct RecordingMouse {
    /// Every operation performed, in order.
    pub ops: Vec<MouseOp>,
    /// The pointer position this backend pretends to have.
    pub cursor: Point,
}

impl RecordingMouse {
    /// A recorder whose pointer starts at `cursor`.
    pub fn at(cursor: Point) -> Self {
        Self {
            ops: Vec::new(),
            cursor,
        }
    }
}

impl MouseBackend for RecordingMouse {
    fn move_to(&mut self, point: Point) -> Result<(), InputError> {
        self.cursor = point;
        self.ops.push(MouseOp::MoveTo { to: point });
        Ok(())
    }

    fn button(
        &mut self,
        button: MouseButton,
        down: bool,
        click_index: u32,
    ) -> Result<(), InputError> {
        self.ops.push(MouseOp::Button {
            button,
            down,
            click_index,
        });
        Ok(())
    }

    fn scroll(&mut self, dx: i32, dy: i32) -> Result<(), InputError> {
        self.ops.push(MouseOp::Scroll { dx, dy });
        Ok(())
    }

    fn cursor_position(&mut self) -> Result<Point, InputError> {
        Ok(self.cursor)
    }
}

/// Construct the platform pointer backend.
///
/// Available on Windows built with `input,wgc`, macOS built with `input,macos`,
/// and X11 Linux built with `input,linux-x11`.
pub fn default_mouse() -> Result<Box<dyn MouseBackend>, InputError> {
    #[cfg(all(windows, feature = "input", feature = "wgc"))]
    {
        Ok(Box::new(windows::WindowsMouse::new()))
    }
    #[cfg(all(target_os = "macos", feature = "input", feature = "macos"))]
    {
        Ok(Box::new(macos::MacosMouse::new()?))
    }
    #[cfg(all(target_os = "linux", feature = "input", feature = "linux-x11"))]
    {
        Ok(Box::new(linux::X11Mouse::new()?))
    }
    #[cfg(not(all(
        feature = "input",
        any(
            all(windows, feature = "wgc"),
            all(target_os = "macos", feature = "macos"),
            all(target_os = "linux", feature = "linux-x11")
        )
    )))]
    {
        Err(InputError::Unsupported(
            "build with `input` plus the platform live-capture feature: \
             `wgc` on Windows, `macos` on macOS, or `linux-x11` on an X11 Linux session"
                .into(),
        ))
    }
}

/// The screen rect that a captured frame of `window` covers.
///
/// This is the origin window-space input coordinates are measured from, and it
/// is not always the window rect: on Windows, Graphics Capture delivers the
/// DWM frame, which excludes the invisible resize border that `GetWindowRect`
/// includes.
pub fn capture_region(window: &WindowInfo) -> Rect {
    #[cfg(all(windows, feature = "input", feature = "wgc"))]
    {
        windows::capture_region(window)
    }
    #[cfg(not(all(windows, feature = "input", feature = "wgc")))]
    {
        window.rect
    }
}

/// Bring `window` to the foreground so that input reaches it.
///
/// On macOS this is a no-op: a synthetic click at a window's screen position
/// activates it the same way a real one does.
pub fn focus_window(window: &WindowInfo) -> Result<(), InputError> {
    #[cfg(all(windows, feature = "input", feature = "wgc"))]
    {
        windows::focus_window(window)
    }
    #[cfg(all(target_os = "linux", feature = "input", feature = "linux-x11"))]
    {
        linux::focus_window(window)
    }
    #[cfg(not(all(
        feature = "input",
        any(
            all(windows, feature = "wgc"),
            all(target_os = "linux", feature = "linux-x11")
        )
    )))]
    {
        let _ = window;
        Ok(())
    }
}

/// Make this process interpret pointer coordinates the way the capture
/// backend reports them.
///
/// Only Windows needs it: a DPI-unaware process is fed virtualized window
/// rects while Graphics Capture delivers physical pixels, so window-space
/// coordinates would be off by the display's scale factor. Best-effort and
/// idempotent; a process that is already DPI-aware is left alone.
pub fn prepare_process() {
    #[cfg(all(windows, feature = "input", feature = "wgc"))]
    {
        windows::ensure_dpi_aware();
    }
}

/// A complete request: what to act on, in which coordinate space, and how.
#[derive(Debug, Clone)]
pub struct InputRequest {
    /// The window to act on. Required for [`CoordSpace::Window`]; in screen
    /// space it is only used to focus the window first.
    pub target: Option<Target>,
    /// What the action coordinates are relative to.
    pub space: CoordSpace,
    /// The ROI capture was cropped to, if any, so coordinates read off the
    /// cropped frame keep working.
    pub roi: Option<Rect>,
    /// Focus the target window before acting.
    pub focus: bool,
    /// Permit window-space points outside the captured region.
    pub allow_outside: bool,
    /// Put the pointer back where it was when the actions are done.
    pub restore_cursor: bool,
    /// Timing used to expand actions into operations.
    pub settings: PlanSettings,
    /// The actions to perform, in order.
    pub actions: Vec<MouseAction>,
}

impl InputRequest {
    /// A request performing `actions`, in window space, focusing the target.
    pub fn new(actions: Vec<MouseAction>) -> Self {
        Self {
            target: None,
            space: CoordSpace::Window,
            roi: None,
            focus: true,
            allow_outside: false,
            restore_cursor: false,
            settings: PlanSettings::default(),
            actions,
        }
    }

    /// Act on the window matching `target`.
    pub fn target(mut self, target: Target) -> Self {
        self.target = Some(target);
        self
    }

    /// Set the coordinate space.
    pub fn space(mut self, space: CoordSpace) -> Self {
        self.space = space;
        self
    }

    /// Offset coordinates by the ROI capture was cropped to.
    pub fn roi(mut self, roi: Rect) -> Self {
        self.roi = Some(roi);
        self
    }

    /// Focus (or don't) the target window before acting.
    pub fn focus(mut self, focus: bool) -> Self {
        self.focus = focus;
        self
    }

    /// Permit window-space points outside the captured region.
    pub fn allow_outside(mut self, allow: bool) -> Self {
        self.allow_outside = allow;
        self
    }

    /// Restore the pointer position afterwards.
    pub fn restore_cursor(mut self, restore: bool) -> Self {
        self.restore_cursor = restore;
        self
    }

    /// Override the expansion timing.
    pub fn settings(mut self, settings: PlanSettings) -> Self {
        self.settings = settings;
        self
    }
}

/// What a request resolved to and what it did.
#[derive(Debug, Clone)]
pub struct InputOutcome {
    /// The resolved target window, when one was needed.
    pub window: Option<WindowInfo>,
    /// The screen rect window-space coordinates were measured against.
    pub region: Rect,
    /// The operations the request expanded to.
    pub ops: Vec<MouseOp>,
    /// Whether the operations were actually performed.
    pub performed: bool,
    /// Where the pointer ended up, in absolute screen coordinates.
    pub cursor_screen: Point,
    /// Where the pointer ended up, in the request's coordinate space.
    pub cursor_space: Point,
}

/// Resolve a request's target window and coordinate map without acting.
fn resolve(request: &InputRequest) -> Result<(Option<WindowInfo>, CoordMap), Error> {
    let needs_window = request.space == CoordSpace::Window;
    if needs_window && request.target.is_none() {
        return Err(InputError::Request(
            "window-space input needs a target window: pass a selector, or use screen space".into(),
        )
        .into());
    }

    // In screen space the window is only needed to focus it.
    let want_window = needs_window || (request.focus && request.target.is_some());
    let window = match (&request.target, want_window) {
        (Some(target), true) => Some(crate::resolve_window(target)?),
        _ => None,
    };

    let mut map = match (&window, needs_window) {
        (Some(window), true) => CoordMap::window(capture_region(window)),
        _ => CoordMap::screen(),
    };
    if needs_window {
        if let Some(roi) = request.roi {
            map = map.with_roi(roi);
        }
        map = map.allow_outside(request.allow_outside);
    }
    Ok((window, map))
}

/// Resolve and plan a request without touching the pointer.
///
/// Everything a real run would reject — an unresolvable window, a point outside
/// the captured region — is rejected here too, so an agent can check a plan
/// before it moves anything.
pub fn dry_run(request: &InputRequest) -> Result<InputOutcome, Error> {
    prepare_process();
    let (window, map) = resolve(request)?;
    let ops = plan(&request.actions, &map, &request.settings)?;
    // Nothing has moved, so the last planned position is the honest answer, and
    // reading the real cursor would only invite a caller to trust it.
    let cursor_screen = ops
        .iter()
        .rev()
        .find_map(|op| match op {
            MouseOp::MoveTo { to } => Some(*to),
            _ => None,
        })
        .unwrap_or(Point::new(map.region.x, map.region.y));
    Ok(InputOutcome {
        window,
        region: map.region,
        performed: false,
        cursor_space: map.to_space(cursor_screen),
        cursor_screen,
        ops,
    })
}

/// Resolve, plan, and perform a request.
///
/// The whole plan is built and bounds-checked before the first event is sent,
/// so a request that is going to be rejected never moves the pointer.
pub fn perform(request: &InputRequest) -> Result<InputOutcome, Error> {
    prepare_process();
    let (window, map) = resolve(request)?;
    let ops = plan(&request.actions, &map, &request.settings)?;
    let mut backend = default_mouse()?;

    if request.focus {
        if let Some(window) = &window {
            focus_window(window)?;
            // Activation is asynchronous: the window manager may still be
            // raising the window when the first event would arrive.
            std::thread::sleep(Duration::from_millis(120));
        }
    }

    let restore = if request.restore_cursor {
        Some(backend.cursor_position()?)
    } else {
        None
    };

    execute(&ops, backend.as_mut())?;

    if let Some(origin) = restore {
        backend.move_to(origin)?;
    }

    let cursor_screen = backend.cursor_position()?;
    Ok(InputOutcome {
        window,
        region: map.region,
        ops,
        performed: true,
        cursor_screen,
        cursor_space: map.to_space(cursor_screen),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn region() -> Rect {
        Rect::new(100, 50, 800, 600)
    }

    fn settings() -> PlanSettings {
        // Zero timing keeps the op stream readable in assertions; the timing
        // itself is covered separately.
        PlanSettings {
            press_ms: 0,
            multi_click_gap_ms: 0,
            between_ms: 0,
            move_settle_ms: 0,
            drag_step_ms: 0,
            drag_steps: 4,
        }
    }

    #[test]
    fn point_parses_and_renders() {
        assert_eq!("10,20".parse::<Point>().unwrap(), Point::new(10, 20));
        assert_eq!(" -3 , 4 ".parse::<Point>().unwrap(), Point::new(-3, 4));
        assert_eq!(Point::new(7, 8).to_string(), "7,8");
        assert!("10".parse::<Point>().is_err());
        assert!("a,b".parse::<Point>().is_err());
    }

    #[test]
    fn button_and_space_parse() {
        assert_eq!("LEFT".parse::<MouseButton>().unwrap(), MouseButton::Left);
        assert_eq!("m".parse::<MouseButton>().unwrap(), MouseButton::Middle);
        assert!("thumb".parse::<MouseButton>().is_err());
        assert_eq!("screen".parse::<CoordSpace>().unwrap(), CoordSpace::Screen);
        assert!("wayland".parse::<CoordSpace>().is_err());
    }

    #[test]
    fn window_space_offsets_by_the_region_origin() {
        let map = CoordMap::window(region());
        assert_eq!(
            map.to_screen(Point::new(0, 0)).unwrap(),
            Point::new(100, 50)
        );
        assert_eq!(
            map.to_screen(Point::new(10, 20)).unwrap(),
            Point::new(110, 70)
        );
        assert_eq!(map.to_space(Point::new(110, 70)), Point::new(10, 20));
    }

    #[test]
    fn window_space_rejects_points_outside_the_region() {
        let map = CoordMap::window(region());
        assert!(matches!(
            map.to_screen(Point::new(800, 10)),
            Err(InputError::OutOfBounds { .. })
        ));
        assert!(map.to_screen(Point::new(799, 599)).is_ok());
        assert!(matches!(
            map.to_screen(Point::new(-1, 0)),
            Err(InputError::OutOfBounds { .. })
        ));
        // ...unless explicitly permitted.
        assert!(map.allow_outside(true).to_screen(Point::new(-1, 0)).is_ok());
    }

    #[test]
    fn roi_shifts_the_origin_and_shrinks_the_bounds() {
        let map = CoordMap::window(region()).with_roi(Rect::new(40, 30, 200, 100));
        assert_eq!(
            map.to_screen(Point::new(0, 0)).unwrap(),
            Point::new(140, 80)
        );
        assert!(map.to_screen(Point::new(199, 99)).is_ok());
        assert!(map.to_screen(Point::new(200, 0)).is_err());
    }

    #[test]
    fn roi_larger_than_the_window_clamps_to_the_window() {
        let map = CoordMap::window(Rect::new(0, 0, 100, 80)).with_roi(Rect::new(20, 10, 500, 500));
        assert_eq!(map.region, Rect::new(20, 10, 80, 70));
    }

    #[test]
    fn screen_space_is_unbounded_and_unshifted() {
        let map = CoordMap::screen();
        assert_eq!(
            map.to_screen(Point::new(-40, 2000)).unwrap(),
            Point::new(-40, 2000)
        );
    }

    #[test]
    fn click_moves_presses_and_releases() {
        let ops = plan(
            &[MouseAction::Click {
                button: MouseButton::Left,
                at: Some(Point::new(10, 20)),
                count: 1,
            }],
            &CoordMap::window(region()),
            &settings(),
        )
        .unwrap();
        assert_eq!(
            ops,
            vec![
                MouseOp::MoveTo {
                    to: Point::new(110, 70)
                },
                MouseOp::Button {
                    button: MouseButton::Left,
                    down: true,
                    click_index: 1
                },
                MouseOp::Button {
                    button: MouseButton::Left,
                    down: false,
                    click_index: 1
                },
            ]
        );
    }

    #[test]
    fn double_click_numbers_its_clicks() {
        let ops = plan(
            &[MouseAction::Click {
                button: MouseButton::Left,
                at: None,
                count: 2,
            }],
            &CoordMap::screen(),
            &settings(),
        )
        .unwrap();
        let indices: Vec<u32> = ops
            .iter()
            .filter_map(|op| match op {
                MouseOp::Button { click_index, .. } => Some(*click_index),
                _ => None,
            })
            .collect();
        assert_eq!(indices, vec![1, 1, 2, 2]);
        // No `at` means no move: click where the pointer already is.
        assert!(!ops.iter().any(|op| matches!(op, MouseOp::MoveTo { .. })));
    }

    #[test]
    fn click_count_zero_is_rejected() {
        let err = plan(
            &[MouseAction::Click {
                button: MouseButton::Left,
                at: None,
                count: 0,
            }],
            &CoordMap::screen(),
            &settings(),
        )
        .unwrap_err();
        assert!(matches!(err, InputError::Request(_)));
    }

    #[test]
    fn drag_interpolates_between_press_and_release() {
        let ops = plan(
            &[MouseAction::Drag {
                from: Some(Point::new(0, 0)),
                to: Point::new(40, 0),
                button: MouseButton::Left,
                steps: 4,
                hold_ms: 0,
            }],
            &CoordMap::window(region()),
            &settings(),
        )
        .unwrap();
        assert_eq!(
            ops,
            vec![
                MouseOp::MoveTo {
                    to: Point::new(100, 50)
                },
                MouseOp::Button {
                    button: MouseButton::Left,
                    down: true,
                    click_index: 1
                },
                MouseOp::MoveTo {
                    to: Point::new(110, 50)
                },
                MouseOp::MoveTo {
                    to: Point::new(120, 50)
                },
                MouseOp::MoveTo {
                    to: Point::new(130, 50)
                },
                MouseOp::MoveTo {
                    to: Point::new(140, 50)
                },
                MouseOp::Button {
                    button: MouseButton::Left,
                    down: false,
                    click_index: 1
                },
            ]
        );
    }

    #[test]
    fn drag_without_a_start_presses_where_the_pointer_is() {
        let ops = plan(
            &[MouseAction::Drag {
                from: None,
                to: Point::new(5, 5),
                button: MouseButton::Left,
                steps: 8,
                hold_ms: 0,
            }],
            &CoordMap::window(region()),
            &settings(),
        )
        .unwrap();
        // Press first, then one move to the destination: with no known start
        // there is no path to interpolate.
        assert_eq!(
            ops,
            vec![
                MouseOp::Button {
                    button: MouseButton::Left,
                    down: true,
                    click_index: 1
                },
                MouseOp::MoveTo {
                    to: Point::new(105, 55)
                },
                MouseOp::Button {
                    button: MouseButton::Left,
                    down: false,
                    click_index: 1
                },
            ]
        );
    }

    #[test]
    fn a_prior_move_gives_an_implicit_drag_its_start() {
        let ops = plan(
            &[
                MouseAction::Move {
                    to: Point::new(0, 0),
                },
                MouseAction::Drag {
                    from: None,
                    to: Point::new(20, 0),
                    button: MouseButton::Left,
                    steps: 2,
                    hold_ms: 0,
                },
            ],
            &CoordMap::window(region()),
            &settings(),
        )
        .unwrap();
        let moves: Vec<Point> = ops
            .iter()
            .filter_map(|op| match op {
                MouseOp::MoveTo { to } => Some(*to),
                _ => None,
            })
            .collect();
        assert_eq!(
            moves,
            vec![
                Point::new(100, 50),
                Point::new(110, 50),
                Point::new(120, 50)
            ]
        );
    }

    #[test]
    fn out_of_bounds_anywhere_rejects_the_whole_plan() {
        let err = plan(
            &[
                MouseAction::Click {
                    button: MouseButton::Left,
                    at: Some(Point::new(10, 10)),
                    count: 1,
                },
                MouseAction::Click {
                    button: MouseButton::Left,
                    at: Some(Point::new(10_000, 10)),
                    count: 1,
                },
            ],
            &CoordMap::window(region()),
            &settings(),
        )
        .unwrap_err();
        assert!(matches!(err, InputError::OutOfBounds { .. }));
    }

    #[test]
    fn scroll_only_emits_when_there_is_something_to_scroll() {
        let ops = plan(
            &[
                MouseAction::Scroll {
                    at: None,
                    dx: 0,
                    dy: -3,
                },
                MouseAction::Scroll {
                    at: None,
                    dx: 0,
                    dy: 0,
                },
            ],
            &CoordMap::screen(),
            &settings(),
        )
        .unwrap();
        assert_eq!(ops, vec![MouseOp::Scroll { dx: 0, dy: -3 }]);
    }

    #[test]
    fn default_timing_paces_a_click_and_separates_actions() {
        let ops = plan(
            &[
                MouseAction::Click {
                    button: MouseButton::Left,
                    at: Some(Point::new(1, 1)),
                    count: 1,
                },
                MouseAction::Wait { ms: 500 },
            ],
            &CoordMap::screen(),
            &PlanSettings::default(),
        )
        .unwrap();
        let sleeps: Vec<u64> = ops
            .iter()
            .filter_map(|op| match op {
                MouseOp::Sleep { ms } => Some(*ms),
                _ => None,
            })
            .collect();
        // move settle, press dwell, inter-action gap, the explicit wait.
        assert_eq!(sleeps, vec![24, 24, 40, 500]);
    }

    #[test]
    fn execute_replays_the_plan_into_a_backend() {
        let ops = plan(
            &[MouseAction::Click {
                button: MouseButton::Right,
                at: Some(Point::new(2, 3)),
                count: 1,
            }],
            &CoordMap::screen(),
            &settings(),
        )
        .unwrap();
        let mut mouse = RecordingMouse::default();
        execute(&ops, &mut mouse).unwrap();
        assert_eq!(mouse.ops, ops);
        assert_eq!(mouse.cursor_position().unwrap(), Point::new(2, 3));
    }

    #[test]
    fn actions_round_trip_through_the_script_format() {
        let json = r#"[
            {"action": "move", "to": [10, 20]},
            {"action": "click", "at": [30, 40], "count": 2},
            {"action": "drag", "from": [1, 2], "to": [3, 4], "button": "middle"},
            {"action": "scroll", "dy": -3},
            {"action": "wait", "ms": 250}
        ]"#;
        let actions: Vec<MouseAction> = serde_json::from_str(json).unwrap();
        assert_eq!(
            actions[1],
            MouseAction::Click {
                button: MouseButton::Left,
                at: Some(Point::new(30, 40)),
                count: 2
            }
        );
        assert_eq!(
            actions[2],
            MouseAction::Drag {
                from: Some(Point::new(1, 2)),
                to: Point::new(3, 4),
                button: MouseButton::Middle,
                steps: PlanSettings::DEFAULT_DRAG_STEPS,
                hold_ms: 0,
            }
        );
        let back = serde_json::to_string(&actions).unwrap();
        assert_eq!(
            serde_json::from_str::<Vec<MouseAction>>(&back).unwrap(),
            actions
        );
    }

    #[test]
    fn window_space_without_a_target_is_rejected() {
        let request = InputRequest::new(vec![MouseAction::Wait { ms: 1 }]);
        assert!(matches!(
            resolve(&request),
            Err(Error::Input(InputError::Request(_)))
        ));
    }

    #[test]
    fn screen_space_needs_no_target() {
        let request = InputRequest::new(vec![MouseAction::Move {
            to: Point::new(5, 6),
        }])
        .space(CoordSpace::Screen);
        let outcome = dry_run(&request).unwrap();
        assert!(outcome.window.is_none());
        assert!(!outcome.performed);
        assert_eq!(outcome.cursor_screen, Point::new(5, 6));
        assert_eq!(outcome.ops.len(), 2); // move + settle
    }

    #[test]
    fn interpolation_lands_exactly_on_the_destination() {
        let path = interpolate(Point::new(0, 0), Point::new(7, 3), 3);
        assert_eq!(path.len(), 3);
        assert_eq!(path[2], Point::new(7, 3));
        assert_eq!(
            interpolate(Point::new(0, 0), Point::new(7, 3), 0),
            vec![Point::new(7, 3)]
        );
    }
}
