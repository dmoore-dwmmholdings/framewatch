//! Platform video capture for `record`: drive the native backend continuously,
//! conform every frame to locked dimensions and tight rows, and publish the
//! latest frame into a single-slot mailbox the pacing loop reads.

use crate::capture::{CaptureBackend, ControlFlow};
use crate::config::Target;
use crate::error::{CaptureError, RecordError};
use crate::frame::{RawFrame, Rect, WindowInfo};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

/// Single-slot "latest frame" mailbox: tight BGRA bytes of the most recent
/// captured frame, conformed to the locked dimensions. `Arc<[u8]>` so the pacing
/// loop reads it without copying.
pub(crate) type FrameMailbox = Arc<Mutex<Option<Arc<[u8]>>>>;

/// Locked `(width, height)` publication, signalled once the first frame lands.
pub(crate) type DimsCell = Arc<(Mutex<Option<(u32, u32)>>, Condvar)>;

/// A target resolved to a platform capture backend and its stable metadata.
pub(crate) struct ResolvedBackend {
    pub(crate) backend: Box<dyn CaptureBackend + Send>,
    pub(crate) window: WindowInfo,
}

/// Round a captured dimension up to an even value for H.264/yuv420p. Real
/// windows and user crops can be odd-sized, but libx264 rejects odd output
/// dimensions with this pixel format.
fn encoder_dimension(value: u32) -> u32 {
    let value = value.max(1);
    if value.is_multiple_of(2) {
        value
    } else {
        value.checked_add(1).unwrap_or(value - 1)
    }
}

fn encoder_dimensions(width: u32, height: u32) -> (u32, u32) {
    (encoder_dimension(width), encoder_dimension(height))
}

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

/// Resolve `target` to the native continuous backend, retrying for up to
/// `wait_ms` while the window is merely absent (not yet launched).
pub(crate) fn resolve_backend(
    target: &Target,
    wait_ms: u64,
) -> Result<ResolvedBackend, RecordError> {
    let deadline = Instant::now() + Duration::from_millis(wait_ms);
    loop {
        match resolve_once(target) {
            Ok(b) => return Ok(b),
            Err(CaptureError::TargetNotFound(_)) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(250));
            }
            Err(e) => return Err(RecordError::Capture(e)),
        }
    }
}

#[cfg(not(any(
    all(windows, feature = "wgc"),
    all(target_os = "macos", feature = "macos"),
    all(target_os = "linux", feature = "linux-x11")
)))]
fn resolve_once(_target: &Target) -> Result<ResolvedBackend, CaptureError> {
    Err(CaptureError::Backend(
        "recording has no live capture backend on this platform".into(),
    ))
}

#[cfg(all(windows, feature = "wgc"))]
fn resolve_once(target: &Target) -> Result<ResolvedBackend, CaptureError> {
    let backend = crate::capture::windows::wgc::WgcBackend::for_target(target)?;
    let window = backend.window().clone();
    Ok(ResolvedBackend {
        backend: Box::new(backend),
        window,
    })
}

#[cfg(all(target_os = "macos", feature = "macos"))]
fn resolve_once(target: &Target) -> Result<ResolvedBackend, CaptureError> {
    let backend = crate::capture::macos::MacosBackend::for_target(target)?;
    let window = backend.window().clone();
    Ok(ResolvedBackend {
        backend: Box::new(backend),
        window,
    })
}

#[cfg(all(target_os = "linux", feature = "linux-x11"))]
fn resolve_once(target: &Target) -> Result<ResolvedBackend, CaptureError> {
    let backend = crate::capture::linux::X11Backend::for_target(target)?;
    let window = backend.window().clone();
    Ok(ResolvedBackend {
        backend: Box::new(backend),
        window,
    })
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

/// Run the native live-capture backend to completion, publishing each frame
/// into `mailbox`.
///
/// The first frame locks the recording dimensions (after the optional `crop`),
/// records its capture instant into `v0` (for A/V sync), and signals `dims`.
/// Subsequent frames are conformed to the locked size. Returns when `stop` is
/// observed or the window closes. Always sets `stop` and wakes `dims` before
/// returning so the pacing/first-frame waiters cannot outlive the capture.
pub(crate) fn run_capture(
    mut backend: Box<dyn CaptureBackend + Send>,
    crop: Option<Rect>,
    mailbox: FrameMailbox,
    dims: DimsCell,
    v0: Arc<Mutex<Option<Instant>>>,
    stop: Arc<AtomicBool>,
) -> Result<(), CaptureError> {
    let mut locked: Option<(u32, u32)> = None;
    let mut scratch: Vec<u8> = Vec::new();

    let result = backend.run(&mut |frame| {
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
                let d = encoder_dimensions(frame.width, frame.height);
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

    stop.store(true, Ordering::Relaxed);
    dims.1.notify_all();
    result
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

    #[test]
    fn encoder_dimensions_are_nonzero_and_even() {
        assert_eq!(encoder_dimensions(1920, 1080), (1920, 1080));
        assert_eq!(encoder_dimensions(641, 481), (642, 482));
        assert_eq!(encoder_dimensions(0, 1), (2, 2));
    }

    #[test]
    fn capture_completion_stops_consumers_and_publishes_even_dimensions() {
        let backend: Box<dyn CaptureBackend + Send> = Box::new(crate::capture::MockBackend::new(
            vec![frame(641, 481, 0x44)],
        ));
        let mailbox: FrameMailbox = Arc::new(Mutex::new(None));
        let dims: DimsCell = Arc::new((Mutex::new(None), Condvar::new()));
        let v0 = Arc::new(Mutex::new(None));
        let stop = Arc::new(AtomicBool::new(false));

        run_capture(
            backend,
            None,
            mailbox.clone(),
            dims.clone(),
            v0,
            stop.clone(),
        )
        .unwrap();

        assert!(stop.load(Ordering::Relaxed));
        assert_eq!(*dims.0.lock().unwrap(), Some((642, 482)));
        assert_eq!(
            mailbox.lock().unwrap().as_ref().unwrap().len(),
            642 * 482 * 4
        );
    }
}
