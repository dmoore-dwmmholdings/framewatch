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

    /// Return a new, tightly-packed frame cropped to `rect` (pixel coords,
    /// relative to this frame's top-left), clamped to the frame bounds.
    ///
    /// `captured_at`/`wall_time`/`window` are preserved. If `rect` is empty or
    /// falls entirely outside the frame, the frame is returned unchanged.
    pub fn crop(&self, rect: Rect) -> RawFrame {
        let fw = self.width as i64;
        let fh = self.height as i64;
        let x0 = (rect.x as i64).clamp(0, fw);
        let y0 = (rect.y as i64).clamp(0, fh);
        let x1 = (rect.x as i64 + rect.w as i64).clamp(0, fw);
        let y1 = (rect.y as i64 + rect.h as i64).clamp(0, fh);
        if x1 <= x0 || y1 <= y0 {
            return self.clone();
        }

        let cw = (x1 - x0) as u32;
        let ch = (y1 - y0) as u32;
        let row_bytes = cw as usize * 4;
        let mut buf = vec![0u8; row_bytes * ch as usize];
        for dy in 0..ch as usize {
            let sy = y0 as usize + dy;
            let src = sy * self.stride as usize + x0 as usize * 4;
            let dst = dy * row_bytes;
            if src + row_bytes <= self.buffer.len() {
                buf[dst..dst + row_bytes].copy_from_slice(&self.buffer[src..src + row_bytes]);
            }
        }

        RawFrame {
            buffer: Arc::from(buf.into_boxed_slice()),
            width: cw,
            height: ch,
            stride: cw * 4,
            captured_at: self.captured_at,
            wall_time: self.wall_time,
            window: self.window.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    fn frame(w: u32, h: u32) -> RawFrame {
        // Each pixel's blue channel = x, green = y (mod 256), for identifiability.
        let mut buf = vec![0u8; (w * h * 4) as usize];
        for y in 0..h {
            for x in 0..w {
                let o = ((y * w + x) * 4) as usize;
                buf[o] = x as u8;
                buf[o + 1] = y as u8;
                buf[o + 2] = 7;
                buf[o + 3] = 255;
            }
        }
        RawFrame::from_bgra(
            buf,
            w,
            h,
            Instant::now(),
            chrono::Utc::now(),
            WindowInfo::synthetic("t", w, h),
        )
    }

    #[test]
    fn rect_to_array_and_new() {
        let r = Rect::new(-3, 4, 10, 20);
        assert_eq!(r.to_array(), [-3, 4, 10, 20]);
    }

    #[test]
    fn window_info_synthetic_defaults() {
        let w = WindowInfo::synthetic("hi", 100, 50);
        assert_eq!(w.title, "hi");
        assert_eq!((w.rect.w, w.rect.h, w.dpi), (100, 50, 96));
        assert!(w.foreground);
    }

    #[test]
    fn from_bgra_sets_tight_stride_and_reads_pixels() {
        let f = frame(4, 3);
        assert_eq!(f.stride, 4 * 4);
        assert_eq!(f.pixel(2, 1), (2, 1, 7, 255)); // (b, g, r, a)
                                                   // Out-of-buffer read is saturating, not a panic.
        let big = f.pixel(999, 999);
        assert_eq!(big.3, 255);
    }

    #[test]
    fn crop_subregion_is_tight_and_correct() {
        let f = frame(8, 6);
        let c = f.crop(Rect::new(2, 1, 3, 2));
        assert_eq!((c.width, c.height, c.stride), (3, 2, 3 * 4));
        // Top-left of the crop is source pixel (2,1).
        assert_eq!(c.pixel(0, 0), (2, 1, 7, 255));
    }

    #[test]
    fn crop_clamps_to_bounds() {
        let f = frame(8, 6);
        let c = f.crop(Rect::new(6, 4, 100, 100));
        assert_eq!((c.width, c.height), (2, 2)); // clamped to the frame edge
    }

    #[test]
    fn crop_fully_off_frame_or_empty_returns_full_frame() {
        let f = frame(8, 6);
        assert_eq!(f.crop(Rect::new(100, 100, 10, 10)).width, 8); // off-frame
        assert_eq!(f.crop(Rect::new(0, 0, 0, 0)).width, 8); // empty rect
    }
}
