//! Raw frame and window-metadata representation.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Instant;

/// A rectangle in integer pixel coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rect {
    /// Left edge, in pixels.
    pub x: i32,
    /// Top edge, in pixels.
    pub y: i32,
    /// Width, in pixels.
    pub w: u32,
    /// Height, in pixels.
    pub h: u32,
}

impl Rect {
    /// Construct a rect.
    pub const fn new(x: i32, y: i32, w: u32, h: u32) -> Self {
        Self { x, y, w, h }
    }

    /// `[x, y, w, h]` form used in the JSON contract.
    pub fn to_array(self) -> [i32; 4] {
        [self.x, self.y, self.w as i32, self.h as i32]
    }
}

/// Metadata about the captured window at the moment a frame was produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowInfo {
    /// Native window handle (HWND on Windows), as an `isize`.
    pub hwnd: isize,
    /// Window title text.
    pub title: String,
    /// Executable basename, e.g. `"chrome.exe"`.
    pub exe: String,
    /// Window class name, e.g. `"Chrome_WidgetWin_1"`.
    pub class: String,
    /// Window bounds in screen coordinates.
    pub rect: Rect,
    /// Client-area bounds; ROIs are stored relative to this.
    pub client_rect: Rect,
    /// Effective DPI for the window.
    pub dpi: u32,
    /// Whether the window was the foreground window.
    pub foreground: bool,
}

impl WindowInfo {
    /// A minimal placeholder used by the mock backend / tests.
    pub fn synthetic(title: impl Into<String>, w: u32, h: u32) -> Self {
        Self {
            hwnd: 0,
            title: title.into(),
            exe: "mock.exe".to_string(),
            class: "Mock".to_string(),
            rect: Rect::new(0, 0, w, h),
            client_rect: Rect::new(0, 0, w, h),
            dpi: 96,
            foreground: true,
        }
    }
}

/// A single raw frame delivered by a [`CaptureBackend`](crate::capture::CaptureBackend).
///
/// The pixel buffer is `BGRA8`, top-down, and shared via [`Arc`] so the engine can
/// hand it to the encoder without copying.
#[derive(Debug, Clone)]
pub struct RawFrame {
    /// BGRA8 pixels, top-down rows.
    pub buffer: Arc<[u8]>,
    /// Frame width in pixels.
    pub width: u32,
    /// Frame height in pixels.
    pub height: u32,
    /// Bytes per row; may exceed `width * 4` due to padding.
    pub stride: u32,
    /// Monotonic capture time; used by the engine for all timing.
    pub captured_at: Instant,
    /// Human/agent-facing wall-clock timestamp.
    pub wall_time: DateTime<Utc>,
    /// Window metadata at capture time.
    pub window: WindowInfo,
}

impl RawFrame {
    /// Build a frame from a tightly-packed BGRA buffer (stride = `width * 4`).
    pub fn from_bgra(
        buffer: impl Into<Arc<[u8]>>,
        width: u32,
        height: u32,
        captured_at: Instant,
        wall_time: DateTime<Utc>,
        window: WindowInfo,
    ) -> Self {
        Self {
            buffer: buffer.into(),
            width,
            height,
            stride: width * 4,
            captured_at,
            wall_time,
            window,
        }
    }

    /// Read the BGRA pixel at `(x, y)`, honouring `stride`. Returns `(b, g, r, a)`.
    #[inline]
    pub fn pixel(&self, x: u32, y: u32) -> (u8, u8, u8, u8) {
        let off = (y * self.stride + x * 4) as usize;
        let b = self.buffer.get(off).copied().unwrap_or(0);
        let g = self.buffer.get(off + 1).copied().unwrap_or(0);
        let r = self.buffer.get(off + 2).copied().unwrap_or(0);
        let a = self.buffer.get(off + 3).copied().unwrap_or(255);
        (b, g, r, a)
    }
}
