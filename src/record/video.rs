//! Windows video capture for `record`: drive the WGC backend continuously,
//! conform every frame to locked dimensions and tight rows, and publish the
//! latest frame into a single-slot mailbox the pacing loop reads.

use crate::capture::windows::wgc::WgcBackend;
use crate::capture::{CaptureBackend, ControlFlow};
use crate::config::Target;
use crate::error::{CaptureError, RecordError};
use crate::frame::{RawFrame, Rect};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

/// Single-slot "latest frame" mailbox: tight BGRA bytes of the most recent
/// captured frame, conformed to the locked dimensions. `Arc<[u8]>` so the pacing
/// loop reads it without copying.
pub(crate) type FrameMailbox = Arc<Mutex<Option<Arc<[u8]>>>>;

/// Locked `(width, height)` publication, signalled once the first frame lands.
pub(crate) type DimsCell = Arc<(Mutex<Option<(u32, u32)>>, Condvar)>;

/// Copy `frame` into `out` as exactly `lock_w * lock_h * 4` tightly-packed BGRA
/// bytes: rows/columns beyond the frame are zero-padded; a larger frame is
/// cropped to the top-left. This both repacks away any stride padding and keeps
/// every emitted buffer the exact size ffmpeg's `-video_size` expects, even if
/// the window is resized mid-recording.
pub(crate) fn conform_frame(frame: &RawFrame, lock_w: u32, lock_h: u32, out: &mut Vec<u8>) {
    let row_bytes = lock_w as usize * 4;
    out.clear();
    out.resize(row_bytes * lock_h as usize, 0);
    let copy_w = frame.width.min(lock_w) as usize * 4;
    let copy_h = frame.height.min(lock_h) as usize;
    let stride = frame.stride as usize;
    for y in 0..copy_h {
        let src = y * stride;
        let dst = y * row_bytes;
        if src + copy_w <= frame.buffer.len() {
            out[dst..dst + copy_w].copy_from_slice(&frame.buffer[src..src + copy_w]);
        }
    }
}

/// Resolve `target` to a WGC backend, retrying for up to `wait_ms` while the
/// window is merely absent (not yet launched).
pub(crate) fn resolve_wgc(target: &Target, wait_ms: u64) -> Result<WgcBackend, RecordError> {
    let deadline = Instant::now() + Duration::from_millis(wait_ms);
    loop {
        match WgcBackend::for_target(target) {
            Ok(b) => return Ok(b),
            Err(CaptureError::TargetNotFound(_)) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(250));
            }
            Err(e) => return Err(RecordError::Capture(e)),
        }
    }
}

/// Block until the first frame publishes locked dimensions, the `stop` flag is
/// set, or `timeout_ms` elapses. Wakes periodically to observe `stop`.
pub(crate) fn wait_for_dims(
    dims: &DimsCell,
    stop: &Arc<AtomicBool>,
    timeout_ms: u64,
) -> Option<(u32, u32)> {
    let (lock, cv) = &**dims;
    let mut guard = lock.lock().unwrap();
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    while guard.is_none() {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        let now = Instant::now();
        if now >= deadline {
            break;
        }
        let wait = (deadline - now).min(Duration::from_millis(200));
        let (g, _) = cv.wait_timeout(guard, wait).unwrap();
        guard = g;
    }
    *guard
}

/// Run the WGC backend to completion, publishing each frame into `mailbox`.
///
/// The first frame locks the recording dimensions (after the optional `crop`),
/// records its capture instant into `v0` (for A/V sync), and signals `dims`.
/// Subsequent frames are conformed to the locked size. Returns when `stop` is
/// observed or the window closes.
pub(crate) fn run_capture(
    mut backend: WgcBackend,
    crop: Option<Rect>,
    mailbox: FrameMailbox,
    dims: DimsCell,
    v0: Arc<Mutex<Option<Instant>>>,
    stop: Arc<AtomicBool>,
) {
    let mut locked: Option<(u32, u32)> = None;
    let mut scratch: Vec<u8> = Vec::new();

    let _ = backend.run(&mut |frame| {
        if stop.load(Ordering::Relaxed) {
            return ControlFlow::Stop;
        }
        let frame = match crop {
            Some(rect) => frame.crop(rect),
            None => frame,
        };
        let (lw, lh) = match locked {
            Some(d) => d,
            None => {
                let d = (frame.width.max(1), frame.height.max(1));
                locked = Some(d);
                *v0.lock().unwrap() = Some(frame.captured_at);
                let (lock, cv) = &*dims;
                *lock.lock().unwrap() = Some(d);
                cv.notify_all();
                d
            }
        };
        conform_frame(&frame, lw, lh, &mut scratch);
        *mailbox.lock().unwrap() = Some(Arc::from(scratch.as_slice()));
        ControlFlow::Continue
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::WindowInfo;
    use chrono::Utc;
    use std::time::Instant;

    fn frame(w: u32, h: u32, fill: u8) -> RawFrame {
        let buf = vec![fill; (w * h * 4) as usize];
        RawFrame::from_bgra(
            buf,
            w,
            h,
            Instant::now(),
            Utc::now(),
            WindowInfo::synthetic("t", w, h),
        )
    }

    #[test]
    fn conform_exact_size_is_passthrough() {
        let f = frame(4, 2, 0xAB);
        let mut out = Vec::new();
        conform_frame(&f, 4, 2, &mut out);
        assert_eq!(out.len(), 4 * 2 * 4);
        assert!(out.iter().all(|&b| b == 0xAB));
    }

    #[test]
    fn conform_pads_smaller_and_crops_larger() {
        // Smaller frame -> zero-padded to locked size.
        let small = frame(2, 1, 0xFF);
        let mut out = Vec::new();
        conform_frame(&small, 4, 2, &mut out);
        assert_eq!(out.len(), 4 * 2 * 4);
        assert!(out[0..8].iter().all(|&b| b == 0xFF)); // first 2 px copied
        assert!(out[8..].iter().all(|&b| b == 0)); // rest zero-padded

        // Larger frame -> cropped to the top-left locked region.
        let big = frame(8, 8, 0x10);
        let mut out2 = Vec::new();
        conform_frame(&big, 4, 2, &mut out2);
        assert_eq!(out2.len(), 4 * 2 * 4);
        assert!(out2.iter().all(|&b| b == 0x10));
    }

    #[test]
    fn conform_handles_row_stride_padding() {
        // width 2, but stride is 3 px worth of bytes (1 px padding per row).
        let w = 2u32;
        let h = 2u32;
        let stride = 3 * 4;
        let mut buf = vec![0u8; stride as usize * h as usize];
        // mark real pixels 0x7F, padding stays 0
        for y in 0..h as usize {
            for b in 0..(w as usize * 4) {
                buf[y * stride as usize + b] = 0x7F;
            }
        }
        let f = RawFrame {
            buffer: buf.into(),
            width: w,
            height: h,
            stride,
            captured_at: Instant::now(),
            wall_time: Utc::now(),
            window: WindowInfo::synthetic("t", w, h),
        };
        let mut out = Vec::new();
        conform_frame(&f, w, h, &mut out);
        // Tight output: every byte is a real pixel (no padding carried over).
        assert_eq!(out.len(), (w * h * 4) as usize);
        assert!(out.iter().all(|&b| b == 0x7F));
    }
}
