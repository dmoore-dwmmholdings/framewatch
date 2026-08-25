//! macOS window capture through Apple ScreenCaptureKit (macOS 14+).
//!
//! ScreenCaptureKit is the supported replacement for the older Core Graphics
//! screenshot APIs and requires the user to grant Screen Recording permission.

use crate::capture::{CaptureBackend, ControlFlow};
use crate::config::Target;
use crate::error::CaptureError;
use crate::frame::{RawFrame, Rect, WindowInfo};
use chrono::Utc;
use crossbeam_channel::{bounded, RecvTimeoutError};
use screencapturekit::cm::CMSampleBufferExt;
use screencapturekit::cv::CVPixelBufferLockFlags;
use screencapturekit::prelude::*;
use screencapturekit::shareable_content::{SCShareableContent, SCWindow};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// A streaming backend for one ScreenCaptureKit window target.
pub struct MacosBackend {
    filter: SCContentFilter,
    configuration: SCStreamConfiguration,
    info: WindowInfo,
    stop: Arc<AtomicBool>,
}

impl MacosBackend {
    /// Resolve a configured target from ScreenCaptureKit's shareable windows.
    pub fn for_target(target: &Target) -> Result<Self, CaptureError> {
        let content = shareable_content()?;
        let mut matches = content.windows().into_iter().filter_map(|window| {
            let info = window_info(&window);
            (info.rect.w > 0 && info.rect.h > 0 && target_matches(target, &window, &info))
                .then_some((window, info))
        });
        let (window, info) = match target {
            Target::ByPid(_) => matches
                .max_by_key(|(_, info)| u64::from(info.rect.w) * u64::from(info.rect.h))
                .ok_or_else(|| CaptureError::TargetNotFound(target_name(target)))?,
            _ => matches
                .next()
                .ok_or_else(|| CaptureError::TargetNotFound(target_name(target)))?,
        };

        let filter = SCContentFilter::create().with_window(&window).build();
        let configuration = SCStreamConfiguration::new()
            .with_width(info.rect.w)
            .with_height(info.rect.h)
            .with_pixel_format(PixelFormat::BGRA);
        Ok(Self {
            filter,
            configuration,
            info,
            stop: Arc::new(AtomicBool::new(false)),
        })
    }

    /// Metadata for the resolved target window.
    pub fn window(&self) -> &WindowInfo {
        &self.info
    }
}

/// Enumerate visible non-desktop windows exposed by ScreenCaptureKit.
pub fn enumerate_windows() -> Result<Vec<WindowInfo>, CaptureError> {
    Ok(shareable_content()?
        .windows()
        .into_iter()
        .map(|window| window_info(&window))
        .filter(|info| info.rect.w > 0 && info.rect.h > 0)
        .collect())
}

impl CaptureBackend for MacosBackend {
    fn run(
        &mut self,
        on_frame: &mut dyn FnMut(RawFrame) -> ControlFlow,
    ) -> Result<(), CaptureError> {
        self.stop.store(false, Ordering::Relaxed);
        let (tx, rx) = bounded(8);
        let handler = FrameHandler {
            tx,
            window: self.info.clone(),
        };
        let mut stream = SCStream::new(&self.filter, &self.configuration);
        stream.add_output_handler(handler, SCStreamOutputType::Screen);
        stream.start_capture().map_err(platform_error)?;

        let result = loop {
            if self.stop.load(Ordering::Relaxed) {
                break Ok(());
            }
            match rx.recv_timeout(Duration::from_millis(200)) {
                Ok(frame) if on_frame(frame) == ControlFlow::Stop => break Ok(()),
                Ok(_) | Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    break Err(CaptureError::Backend(
                        "macOS capture stream stopped unexpectedly".into(),
                    ));
                }
            }
        };
        let stop_result = stream.stop_capture().map_err(platform_error);
        result.and(stop_result)
    }

    fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }

    fn stop_signal(&self) -> Option<Arc<AtomicBool>> {
        Some(self.stop.clone())
    }
}

/// Copies ScreenCaptureKit's transient pixel buffers into `RawFrame`s before
/// returning control to the framework callback.
struct FrameHandler {
    tx: crossbeam_channel::Sender<RawFrame>,
    window: WindowInfo,
}

impl SCStreamOutputTrait for FrameHandler {
    fn did_output_sample_buffer(&self, sample: CMSampleBuffer, kind: SCStreamOutputType) {
        if !matches!(kind, SCStreamOutputType::Screen) {
            return;
        }
        let Some(pixel_buffer) = sample.image_buffer() else {
            return;
        };
        let Ok(guard) = pixel_buffer.lock(CVPixelBufferLockFlags::READ_ONLY) else {
            return;
        };
        let Ok(width) = guard.width().try_into() else {
            return;
        };
        let Ok(height) = guard.height().try_into() else {
            return;
        };
        let Ok(stride) = guard.bytes_per_row().try_into() else {
            return;
        };
        let bytes = guard.as_slice().to_vec();
        let frame = RawFrame {
            buffer: bytes.into(),
            width,
            height,
            stride,
            captured_at: Instant::now(),
            wall_time: Utc::now(),
            window: self.window.clone(),
        };
        let _ = self.tx.try_send(frame);
    }
}

fn shareable_content() -> Result<SCShareableContent, CaptureError> {
    // The command-line binary has not otherwise initialized a connection to
    // WindowServer. Initialize AppKit before the first ScreenCaptureKit call.
    unsafe {
        let _ = NSApplicationLoad();
    }
    SCShareableContent::create()
        .with_on_screen_windows_only(true)
        .with_exclude_desktop_windows(true)
        .get()
        .map_err(platform_error)
}

#[link(name = "AppKit", kind = "framework")]
unsafe extern "C" {
    fn NSApplicationLoad() -> bool;
}

fn window_info(window: &SCWindow) -> WindowInfo {
    let frame = window.frame();
    let app = window.owning_application();
    let width = frame.size.width.max(0.0) as u32;
    let height = frame.size.height.max(0.0) as u32;
    WindowInfo {
        hwnd: window.window_id() as isize,
        title: window.title().unwrap_or_default(),
        exe: app
            .as_ref()
            .map(|app| app.application_name())
            .unwrap_or_default(),
        class: "macOS".to_string(),
        rect: Rect::new(frame.origin.x as i32, frame.origin.y as i32, width, height),
        client_rect: Rect::new(0, 0, width, height),
        dpi: 96,
        foreground: window.is_active(),
    }
}

fn target_matches(target: &Target, window: &SCWindow, info: &WindowInfo) -> bool {
    match target {
        Target::ByHwnd(id) => info.hwnd == *id,
        Target::ByTitleRegex(title) => info.title.to_lowercase().contains(&title.to_lowercase()),
        Target::ByExe(exe) => info.exe.eq_ignore_ascii_case(exe),
        Target::ByPid(pid) => window
            .owning_application()
            .is_some_and(|app| app.process_id() == *pid as i32),
    }
}

fn target_name(target: &Target) -> String {
    match target {
        Target::ByHwnd(id) => format!("window id {id}"),
        Target::ByTitleRegex(title) => format!("title {title:?}"),
        Target::ByExe(exe) => format!("application {exe:?}"),
        Target::ByPid(pid) => format!("pid {pid}"),
    }
}

fn platform_error(error: impl std::fmt::Display) -> CaptureError {
    CaptureError::Backend(format!(
        "macOS ScreenCaptureKit failed: {error}. Grant Framewatch Screen Recording access in System Settings > Privacy & Security > Screen & System Audio Recording"
    ))
}
