//! macOS window capture through Apple ScreenCaptureKit (macOS 14+).
//!
//! ScreenCaptureKit is the supported replacement for the older Core Graphics
//! screenshot APIs and requires the user to grant Screen Recording permission.

use crate::capture::{CaptureBackend, ControlFlow};
use crate::config::Target;
use crate::error::CaptureError;
use crate::frame::{RawFrame, Rect, WindowInfo};
use chrono::Utc;
use screencapturekit::prelude::*;
use screencapturekit::screenshot_manager::{CGImageExt, SCScreenshotManager};
use screencapturekit::shareable_content::{SCShareableContent, SCWindow};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

const POLL_INTERVAL: Duration = Duration::from_millis(100);

/// A polling backend for one ScreenCaptureKit window target.
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
        while !self.stop.load(Ordering::Relaxed) {
            let image = SCScreenshotManager::capture_image(&self.filter, &self.configuration)
                .map_err(platform_error)?;
            let width = image
                .width()
                .try_into()
                .map_err(|_| CaptureError::Backend("macOS screenshot width exceeds u32".into()))?;
            let height = image
                .height()
                .try_into()
                .map_err(|_| CaptureError::Backend("macOS screenshot height exceeds u32".into()))?;
            let bgra = image.bgra_data().map_err(platform_error)?;
            let frame = RawFrame::from_bgra(
                bgra,
                width,
                height,
                Instant::now(),
                Utc::now(),
                self.info.clone(),
            );
            if on_frame(frame) == ControlFlow::Stop {
                break;
            }
            std::thread::sleep(POLL_INTERVAL);
        }
        Ok(())
    }

    fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }

    fn stop_signal(&self) -> Option<Arc<AtomicBool>> {
        Some(self.stop.clone())
    }
}

fn shareable_content() -> Result<SCShareableContent, CaptureError> {
    // ScreenCaptureKit's screenshot manager is also used by the command-line
    // binary, where AppKit has not otherwise initialized a connection to the
    // WindowServer. Initialize it before the first ScreenCaptureKit call.
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
