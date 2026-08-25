//! Capture backends: the [`CaptureBackend`] trait, window enumeration, and the
//! cross-platform [`MockBackend`]. Native live capture is implemented behind
//! platform features: Windows Graphics Capture, ScreenCaptureKit, and X11.

pub mod mock;
pub use mock::MockBackend;

#[cfg(all(target_os = "linux", feature = "linux-x11"))]
pub mod linux;
#[cfg(all(target_os = "macos", feature = "macos"))]
pub mod macos;
#[cfg(all(windows, feature = "wgc"))]
pub mod windows;

use crate::error::CaptureError;
use crate::frame::{RawFrame, WindowInfo};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

/// Whether the host loop wants more frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlFlow {
    /// Keep delivering frames.
    Continue,
    /// Stop capturing.
    Stop,
}

/// A source of frames for a single target window.
pub trait CaptureBackend {
    /// Begin capture. `on_frame` is invoked once per delivered frame.
    ///
    /// Implementations should deliver frames only when the window content changes
    /// (the Windows Graphics Capture backend does this natively).
    fn run(
        &mut self,
        on_frame: &mut dyn FnMut(RawFrame) -> ControlFlow,
    ) -> Result<(), CaptureError>;

    /// Request that capture stop as soon as possible.
    fn stop(&mut self);

    /// An optional shared flag the host can set to request the backend stop,
    /// even while the window is idle and delivering no frames (used by the
    /// duration watchdog). Returns `None` if the backend can't be signalled.
    fn stop_signal(&self) -> Option<Arc<AtomicBool>> {
        None
    }
}

impl CaptureBackend for Box<dyn CaptureBackend> {
    fn run(
        &mut self,
        on_frame: &mut dyn FnMut(RawFrame) -> ControlFlow,
    ) -> Result<(), CaptureError> {
        (**self).run(on_frame)
    }

    fn stop(&mut self) {
        (**self).stop()
    }

    fn stop_signal(&self) -> Option<Arc<AtomicBool>> {
        (**self).stop_signal()
    }
}

/// Enumerate capturable top-level windows.
///
/// Returns an error on platforms/builds without a capture backend.
pub fn enumerate_windows() -> Result<Vec<WindowInfo>, CaptureError> {
    #[cfg(all(windows, feature = "wgc"))]
    {
        windows::enumerate::enumerate_windows()
    }
    #[cfg(all(target_os = "macos", feature = "macos"))]
    {
        macos::enumerate_windows()
    }
    #[cfg(all(target_os = "linux", feature = "linux-x11"))]
    {
        linux::enumerate_windows()
    }
    #[cfg(not(any(
        all(windows, feature = "wgc"),
        all(target_os = "macos", feature = "macos"),
        all(target_os = "linux", feature = "linux-x11")
    )))]
    {
        Err(CaptureError::Backend(
            "window enumeration requires Windows with `wgc`, macOS with `macos`, or an X11 Linux session with `linux-x11`".into(),
        ))
    }
}
