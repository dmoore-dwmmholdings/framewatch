//! The Windows Graphics Capture backend, wrapping the `windows-capture` crate.
//!
//! Frames are delivered on `windows-capture`'s own thread and bridged to the
//! host loop over a bounded channel; an idle window costs nothing because WGC
//! only delivers a frame when the window repaints.

use crate::capture::windows::{fill_window_info, hwnd_from_isize};
use crate::capture::{CaptureBackend, ControlFlow};
use crate::config::Target;
use crate::error::CaptureError;
use crate::frame::{RawFrame, WindowInfo};
use chrono::Utc;
use crossbeam_channel::{bounded, RecvTimeoutError, Sender};
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use windows_capture::capture::{Context, GraphicsCaptureApiHandler};
use windows_capture::frame::Frame;
use windows_capture::graphics_capture_api::InternalCaptureControl;
use windows_capture::settings::{
    ColorFormat, CursorCaptureSettings, DirtyRegionSettings, DrawBorderSettings,
    MinimumUpdateIntervalSettings, SecondaryWindowSettings, Settings,
};
use windows_capture::window::Window;

type HandlerError = Box<dyn std::error::Error + Send + Sync>;

/// Data handed to the capture handler via `Settings` flags.
struct CaptureFlags {
    tx: Sender<RawFrame>,
    window: WindowInfo,
    stop: Arc<AtomicBool>,
}

/// The `windows-capture` handler: turns each WGC frame into a [`RawFrame`].
struct Handler {
    tx: Sender<RawFrame>,
    window: WindowInfo,
    stop: Arc<AtomicBool>,
    frame_count: u64,
}

impl GraphicsCaptureApiHandler for Handler {
    type Flags = CaptureFlags;
    type Error = HandlerError;

    fn new(ctx: Context<Self::Flags>) -> Result<Self, Self::Error> {
        Ok(Self {
            tx: ctx.flags.tx,
            window: ctx.flags.window,
            stop: ctx.flags.stop,
            frame_count: 0,
        })
    }

    fn on_frame_arrived(
        &mut self,
        frame: &mut Frame,
        capture_control: InternalCaptureControl,
    ) -> Result<(), Self::Error> {
        if self.stop.load(Ordering::Relaxed) {
            capture_control.stop();
            return Ok(());
        }

        // Keep window geometry (rect/dpi/foreground) fresh across resizes and
        // fullscreen transitions; ~every 30 frames to stay cheap.
        if self.frame_count.is_multiple_of(30) {
            crate::capture::windows::refresh_geometry(&mut self.window);
        }
        self.frame_count = self.frame_count.wrapping_add(1);

        let width = frame.width();
        let height = frame.height();
        let mut fb = frame.buffer().map_err(|e| Box::new(e) as HandlerError)?;
        let stride = fb.row_pitch();
        let buffer: Arc<[u8]> = Arc::from(fb.as_raw_buffer().to_vec().into_boxed_slice());

        let rf = RawFrame {
            buffer,
            width,
            height,
            stride,
            captured_at: Instant::now(),
            wall_time: Utc::now(),
            window: self.window.clone(),
        };

        // Drop frames on backpressure rather than block the capture thread; the
        // downstream engine coalesces, so dropping is correct here, not a failure.
        match self.tx.try_send(rf) {
            Ok(()) => {}
            Err(crossbeam_channel::TrySendError::Full(_)) => {}
            Err(crossbeam_channel::TrySendError::Disconnected(_)) => capture_control.stop(),
        }
        Ok(())
    }

    fn on_closed(&mut self) -> Result<(), Self::Error> {
        self.stop.store(true, Ordering::Relaxed);
        Ok(())
    }
}

/// The live Windows Graphics Capture backend.
pub struct WgcBackend {
    hwnd: isize,
    window: WindowInfo,
    stop: Arc<AtomicBool>,
}

impl WgcBackend {
    /// Resolve `target` to a window and build a backend for it.
    pub fn for_target(target: &Target) -> Result<Self, CaptureError> {
        let win = resolve_target(target)?;
        let hwnd_ptr = win.as_raw_hwnd();
        let hwnd = hwnd_from_isize(hwnd_ptr as isize);
        let title = win.title().unwrap_or_default();
        let exe = win.process_name().unwrap_or_default();
        let window = fill_window_info(hwnd, title, exe);
        Ok(Self {
            hwnd: hwnd_ptr as isize,
            window,
            stop: Arc::new(AtomicBool::new(false)),
        })
    }

    /// The resolved window metadata.
    pub fn window(&self) -> &WindowInfo {
        &self.window
    }
}

fn resolve_target(target: &Target) -> Result<Window, CaptureError> {
    match target {
        Target::ByHwnd(h) => {
            let w = Window::from_raw_hwnd(*h as *mut c_void);
            if w.is_valid() {
                Ok(w)
            } else {
                Err(CaptureError::TargetNotFound(format!("hwnd {h}")))
            }
        }
        Target::ByTitleRegex(query) => {
            // Case-insensitive literal substring match. Window titles routinely
            // contain regex-special characters (paths with `\`, `(beta)`, `.`),
            // so matching the query as plain text is far less surprising than a
            // regex — and `--title "discord"` still matches "Discord".
            let needle = query.to_lowercase();
            let windows = Window::enumerate().map_err(|e| CaptureError::Backend(e.to_string()))?;
            windows
                .into_iter()
                .find(|w| {
                    w.title()
                        .map(|t| t.to_lowercase().contains(&needle))
                        .unwrap_or(false)
                })
                .ok_or_else(|| CaptureError::TargetNotFound(format!("title containing {query:?}")))
        }
        Target::ByExe(name) => {
            let windows = Window::enumerate().map_err(|e| CaptureError::Backend(e.to_string()))?;
            windows
                .into_iter()
                .find(|w| {
                    w.process_name()
                        .map(|p| p.eq_ignore_ascii_case(name))
                        .unwrap_or(false)
                })
                .ok_or_else(|| CaptureError::TargetNotFound(format!("exe {name}")))
        }
        Target::ByPid(pid) => {
            let windows = Window::enumerate().map_err(|e| CaptureError::Backend(e.to_string()))?;
            windows
                .into_iter()
                .filter(|w| w.process_id().map(|p| p == *pid).unwrap_or(false))
                // A process can own several windows; pick the largest (the main one).
                .max_by_key(|w| {
                    let wd = w.width().unwrap_or(0).max(0) as i64;
                    let ht = w.height().unwrap_or(0).max(0) as i64;
                    wd * ht
                })
                .ok_or_else(|| CaptureError::TargetNotFound(format!("pid {pid}")))
        }
    }
}

impl CaptureBackend for WgcBackend {
    fn run(
        &mut self,
        on_frame: &mut dyn FnMut(RawFrame) -> ControlFlow,
    ) -> Result<(), CaptureError> {
        self.stop.store(false, Ordering::Relaxed);
        let (tx, rx) = bounded::<RawFrame>(8);
        let flags = CaptureFlags {
            tx,
            window: self.window.clone(),
            stop: self.stop.clone(),
        };

        let item = Window::from_raw_hwnd(self.hwnd as *mut c_void);
        let settings = Settings::new(
            item,
            CursorCaptureSettings::Default,
            DrawBorderSettings::Default,
            SecondaryWindowSettings::Default,
            MinimumUpdateIntervalSettings::Default,
            DirtyRegionSettings::Default,
            ColorFormat::Bgra8,
            flags,
        );

        let control = Handler::start_free_threaded(settings)
            .map_err(|e| CaptureError::Backend(format!("{e:?}")))?;

        loop {
            match rx.recv_timeout(Duration::from_millis(200)) {
                Ok(frame) => {
                    if let ControlFlow::Stop = on_frame(frame) {
                        self.stop.store(true, Ordering::Relaxed);
                        break;
                    }
                }
                Err(RecvTimeoutError::Timeout) => {
                    if self.stop.load(Ordering::Relaxed) || control.is_finished() {
                        break;
                    }
                }
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }

        let _ = control.stop();
        Ok(())
    }

    fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }

    fn stop_signal(&self) -> Option<Arc<AtomicBool>> {
        Some(self.stop.clone())
    }
}
