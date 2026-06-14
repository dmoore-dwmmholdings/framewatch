//! Shared helpers for the integration tests: synthetic BGRA frame builders.
#![allow(dead_code, clippy::too_many_arguments)]

use chrono::{DateTime, Duration as ChronoDuration, TimeZone, Utc};
use framewatch::{RawFrame, WindowInfo};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub const W: u32 = 320;
pub const H: u32 = 180;

/// A fixed wall-clock base so golden output is deterministic.
pub fn base_wall() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 6, 13, 15, 4, 5).unwrap()
}

/// A tightly-packed BGRA buffer filled with one color `(b, g, r)`.
pub fn solid(b: u8, g: u8, r: u8) -> Vec<u8> {
    let mut buf = vec![0u8; (W * H * 4) as usize];
    for px in buf.chunks_exact_mut(4) {
        px[0] = b;
        px[1] = g;
        px[2] = r;
        px[3] = 255;
    }
    buf
}

/// Paint a filled rectangle (pixel coords) into a BGRA buffer.
pub fn paint_rect(buf: &mut [u8], x: u32, y: u32, w: u32, h: u32, b: u8, g: u8, r: u8) {
    for yy in y..(y + h).min(H) {
        for xx in x..(x + w).min(W) {
            let off = ((yy * W + xx) * 4) as usize;
            buf[off] = b;
            buf[off + 1] = g;
            buf[off + 2] = r;
            buf[off + 3] = 255;
        }
    }
}

/// A monotonic clock base for frame timing.
pub fn base_instant() -> Instant {
    Instant::now()
}

/// Build a frame at `t_ms` after the bases.
pub fn frame_at(buf: Vec<u8>, base: Instant, t_ms: u64) -> (RawFrame, Instant) {
    let now = base + Duration::from_millis(t_ms);
    let f = RawFrame {
        buffer: Arc::from(buf.into_boxed_slice()),
        width: W,
        height: H,
        stride: W * 4,
        captured_at: now,
        wall_time: base_wall() + ChronoDuration::milliseconds(t_ms as i64),
        window: window_info(),
    };
    (f, now)
}

/// A stable synthetic window descriptor.
pub fn window_info() -> WindowInfo {
    WindowInfo {
        hwnd: 67890,
        title: "Build — myapp — Visual Studio Code".to_string(),
        exe: "Code.exe".to_string(),
        class: "Chrome_WidgetWin_1".to_string(),
        rect: framewatch::Rect::new(0, 0, W, H),
        client_rect: framewatch::Rect::new(0, 0, W, H),
        dpi: 96,
        foreground: true,
    }
}
