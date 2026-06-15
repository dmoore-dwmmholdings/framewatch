//! # framewatch
//!
//! Event-driven, change-triggered window capture that emits timestamped
//! screenshots + metadata so an AI coding agent can reconstruct what happened
//! without a continuous frame stream.
//!
//! Instead of recording every frame (wasteful) or only on demand (lossy),
//! framewatch watches **one window** and writes a frame to disk only at
//! semantically meaningful moments — when the window **settles** after activity,
//! when a **spinner** starts/stops, or as a throttled **sample** of a volatile
//! region — plus the initial and any manual frames. Everything else is collapsed
//! into a compact, timestamped [`timeline`](sink::DirectorySink).
//!
//! ## Architecture
//!
//! The [`Engine`] is pure and backend-agnostic: `(state, RawFrame, now) -> events`.
//! It does no I/O and no capture, so the hard logic is fully testable on any OS.
//! All Windows-specific code is `#[cfg(windows)]` behind the `capture::windows`
//! module (and the `wgc` feature).
//!
//! ## Embedding
//!
//! ```no_run
//! use framewatch::{Config, Target, DirectorySink, Engine, CaptureBackend, ControlFlow, Sink, SystemClock};
//!
//! # fn main() -> Result<(), framewatch::Error> {
//! let config = Config::builder()
//!     .target(Target::ByTitleRegex("Visual Studio Code".into()))
//!     .out_dir("./.framewatch")
//!     .settle_ms(350)
//!     .build()?;
//!
//! let mut engine = Engine::new(config.clone(), SystemClock);
//! let mut sink = DirectorySink::new(&config)?;
//! let mut backend = framewatch::MockBackend::new(vec![]);
//!
//! backend.run(&mut |frame| {
//!     for event in engine.process(&frame, frame.captured_at) {
//!         sink.on_event(&event).ok();
//!     }
//!     ControlFlow::Continue
//! })?;
//! # Ok(()) }
//! ```
#![warn(missing_docs)]

pub mod capture;
pub mod clock;
pub mod config;
pub mod detect;
pub mod engine;
pub mod error;
pub mod event;
pub mod frame;
pub mod recording;
pub mod session;
pub mod sink;
pub mod transcript;

mod util;

/// Pure audio helpers (downmix / resample), shared by the recording runtime and
/// the bundled whisper transcriber.
#[cfg(any(feature = "record", feature = "whisper"))]
mod audioutil;

#[cfg(feature = "gui")]
pub mod gui;

pub use capture::{enumerate_windows, CaptureBackend, ControlFlow, MockBackend};
pub use clock::{Clock, MockClock, SystemClock};
pub use config::{Config, ConfigBuilder, ImageOpts, RoiHint, RoiKind, Rotation, Target};
pub use engine::Engine;
pub use error::{CaptureError, Error, RecordError, SinkError, TranscribeError};
pub use event::{CaptureEvent, CaptureMeta, EncodedImage, EventKind, ImageFormat, SaveMask};
pub use frame::{RawFrame, Rect, WindowInfo};
pub use recording::{PackageWriter, Recording, RecordingManifest};
pub use sink::{ChannelSink, CompositeSink, DirectorySink, Sink};
pub use transcript::{Transcriber, Transcript, TranscriptSegment};
pub use util::tokenize;

/// Construct the platform default capture backend for `config`.
///
/// On Windows built with the `wgc` feature this resolves `config.target` to a
/// window and returns the Graphics Capture backend. Elsewhere it returns
/// [`Error::NoBackend`].
pub fn default_backend(config: &Config) -> Result<Box<dyn CaptureBackend>, Error> {
    #[cfg(all(windows, feature = "wgc"))]
    {
        let backend = capture::windows::wgc::WgcBackend::for_target(&config.target)?;
        Ok(Box::new(backend))
    }
    #[cfg(not(all(windows, feature = "wgc")))]
    {
        let _ = config;
        Err(Error::NoBackend(
            "live capture requires building on Windows with the `wgc` feature".into(),
        ))
    }
}

/// One-call convenience: resolve `config.target` (waiting up to `config.wait_ms`
/// for it to appear) and capture into `sink` until interrupted, the window
/// closes, or an auto-stop condition (`stop_after_ms` / `stop_after_images` /
/// `stop_after_settled`) is met.
pub fn watch(config: Config, sink: impl Sink) -> Result<(), Error> {
    config.validate()?;
    let backend = resolve_backend_waiting(&config)?;
    watch_with(config, backend, sink)
}

/// Drive an already-constructed `backend` through the engine into `sink`,
/// honouring the auto-stop conditions in `config`. Useful for embedding with a
/// custom backend (e.g. [`MockBackend`]).
pub fn watch_with<B: CaptureBackend, S: Sink>(
    config: Config,
    mut backend: B,
    mut sink: S,
) -> Result<(), Error> {
    // Duration watchdog: fires even if the window is idle and delivers no frames
    // (the backend's run loop polls its stop flag).
    if config.stop_after_ms > 0 {
        if let Some(signal) = backend.stop_signal() {
            let ms = config.stop_after_ms;
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(ms));
                signal.store(true, std::sync::atomic::Ordering::Relaxed);
            });
        }
    }

    let stop_after_ms = config.stop_after_ms;
    let stop_after_images = config.stop_after_images;
    let stop_after_settled = config.stop_after_settled;
    let crop = config.crop;
    let mut engine = Engine::new(config, SystemClock);
    let mut images: u64 = 0;
    let start = std::time::Instant::now();

    backend.run(&mut |frame| {
        // Crop to the region of interest (e.g. to clip host window chrome) so both
        // detection and the saved image cover only that region.
        let frame = match crop {
            Some(rect) => frame.crop(rect),
            None => frame,
        };
        for event in engine.process(&frame, frame.captured_at) {
            if event.image.is_some() {
                images += 1;
            }
            let is_settled = event.kind() == EventKind::Settled;
            if sink.on_event(&event).is_err() {
                return ControlFlow::Stop;
            }
            if stop_after_settled && is_settled {
                return ControlFlow::Stop;
            }
            if stop_after_images > 0 && images >= stop_after_images {
                return ControlFlow::Stop;
            }
        }
        if stop_after_ms > 0 && start.elapsed() >= std::time::Duration::from_millis(stop_after_ms) {
            return ControlFlow::Stop;
        }
        ControlFlow::Continue
    })?;
    sink.flush()?;
    Ok(())
}

fn resolve_backend_waiting(config: &Config) -> Result<Box<dyn CaptureBackend>, Error> {
    wait_for_ok(
        std::time::Duration::from_millis(config.wait_ms),
        std::time::Duration::from_millis(250),
        || default_backend(config),
    )
}

/// Retry `attempt` while it returns a retryable [`CaptureError::TargetNotFound`]
/// until `timeout` elapses, sleeping `poll` between tries. Non-retryable errors
/// (e.g. no backend on this platform) return immediately.
fn wait_for_ok<T>(
    timeout: std::time::Duration,
    poll: std::time::Duration,
    mut attempt: impl FnMut() -> Result<T, Error>,
) -> Result<T, Error> {
    let deadline = std::time::Instant::now() + timeout;
    let mut waited = false;
    loop {
        match attempt() {
            Ok(value) => return Ok(value),
            Err(e) => {
                let retryable = matches!(&e, Error::Capture(CaptureError::TargetNotFound(_)));
                if retryable && std::time::Instant::now() < deadline {
                    if !waited {
                        tracing::info!("framewatch: waiting for target window to appear...");
                        waited = true;
                    }
                    std::thread::sleep(poll);
                    continue;
                }
                return Err(e);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::time::Duration;

    #[test]
    fn wait_for_ok_no_wait_fails_fast() {
        let calls = Cell::new(0);
        let r: Result<(), Error> = wait_for_ok(Duration::ZERO, Duration::ZERO, || {
            calls.set(calls.get() + 1);
            Err(Error::Capture(CaptureError::TargetNotFound("x".into())))
        });
        assert!(r.is_err());
        assert_eq!(calls.get(), 1, "no wait => exactly one attempt");
    }

    #[test]
    fn wait_for_ok_retries_then_succeeds() {
        let calls = Cell::new(0);
        let r = wait_for_ok(Duration::from_secs(5), Duration::from_millis(1), || {
            calls.set(calls.get() + 1);
            if calls.get() < 3 {
                Err(Error::Capture(CaptureError::TargetNotFound("x".into())))
            } else {
                Ok(42)
            }
        });
        assert_eq!(r.unwrap(), 42);
        assert_eq!(calls.get(), 3);
    }

    #[test]
    fn wait_for_ok_does_not_retry_non_target_errors() {
        let calls = Cell::new(0);
        let r: Result<(), Error> = wait_for_ok(Duration::from_secs(5), Duration::ZERO, || {
            calls.set(calls.get() + 1);
            Err(Error::NoBackend("nope".into()))
        });
        assert!(r.is_err());
        assert_eq!(calls.get(), 1, "non-retryable error returns immediately");
    }
}
