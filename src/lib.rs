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
pub mod session;
pub mod sink;

#[cfg(feature = "gui")]
pub mod gui;

pub use capture::{enumerate_windows, CaptureBackend, ControlFlow, MockBackend};
pub use clock::{Clock, MockClock, SystemClock};
pub use config::{Config, ConfigBuilder, ImageOpts, RoiHint, RoiKind, Rotation, Target};
pub use engine::Engine;
pub use error::{CaptureError, Error, SinkError};
pub use event::{CaptureEvent, CaptureMeta, EncodedImage, EventKind, ImageFormat, SaveMask};
pub use frame::{RawFrame, Rect, WindowInfo};
pub use sink::{ChannelSink, CompositeSink, DirectorySink, Sink};

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

/// One-call convenience: capture `config.target` into `sink` until interrupted
/// or the window closes.
pub fn watch(config: Config, mut sink: impl Sink) -> Result<(), Error> {
    config.validate()?;
    let mut backend = default_backend(&config)?;
    let mut engine = Engine::new(config, SystemClock);

    backend.run(&mut |frame| {
        for event in engine.process(&frame, frame.captured_at) {
            if sink.on_event(&event).is_err() {
                return ControlFlow::Stop;
            }
        }
        ControlFlow::Continue
    })?;
    sink.flush()?;
    Ok(())
}
