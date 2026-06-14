//! Capture backends: the [`CaptureBackend`] trait, window enumeration, and the
//! cross-platform [`MockBackend`]. The live Windows Graphics Capture backend
//! lives under the `windows` submodule (gated on `cfg(windows)` + the `wgc` feature).

pub mod mock;
pub use mock::MockBackend;

#[cfg(all(windows, feature = "wgc"))]
pub mod windows;

use crate::error::CaptureError;
use crate::frame::{RawFrame, WindowInfo};

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
}

/// Enumerate capturable top-level windows.
///
/// Returns an error on platforms/builds without a capture backend (i.e. anything
/// other than Windows built with the `wgc` feature).
pub fn enumerate_windows() -> Result<Vec<WindowInfo>, CaptureError> {
    #[cfg(all(windows, feature = "wgc"))]
    {
        windows::enumerate::enumerate_windows()
    }
    #[cfg(not(all(windows, feature = "wgc")))]
    {
        Err(CaptureError::Backend(
            "window enumeration requires building on Windows with the `wgc` feature".into(),
        ))
    }
}
