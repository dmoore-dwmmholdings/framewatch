//! X11 window capture for Linux.
//!
//! This backend intentionally supports X11 only. A Wayland compositor does not
//! permit clients to enumerate or capture arbitrary windows; that needs an XDG
//! Desktop Portal session and explicit user selection.

use crate::capture::{CaptureBackend, ControlFlow};
use crate::config::Target;
use crate::error::CaptureError;
use crate::frame::{RawFrame, Rect, WindowInfo};
use chrono::Utc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};
use x11rb::connection::Connection;
use x11rb::image::{Image, PixelLayout};
use x11rb::protocol::xproto::{Atom, AtomEnum, ConnectionExt as _, ImageFormat, MapState, Window};
use x11rb::rust_connection::RustConnection;

const POLL_INTERVAL: Duration = Duration::from_millis(33);
const ALL_PROPERTY_BYTES: u32 = u32::MAX;

/// A polling capture backend for one X11 top-level window.
pub struct X11Backend {
    conn: RustConnection,
    root: Window,
    window: Window,
    info: WindowInfo,
    stop: Arc<AtomicBool>,
}

impl X11Backend {
    /// Resolve an X11 target. `DISPLAY` must name a reachable X server.
    pub fn for_target(target: &Target) -> Result<Self, CaptureError> {
        let (conn, screen_num) = x11rb::connect(None).map_err(platform_error)?;
        let root = conn.setup().roots[screen_num].root;
        let windows = windows_for(&conn, root)?;
        let info = resolve_target(target, &conn, &windows)?;
        Ok(Self {
            conn,
            root,
            window: info.hwnd as u32,
            info,
            stop: Arc::new(AtomicBool::new(false)),
        })
    }

    /// Metadata for the resolved window.
    pub fn window(&self) -> &WindowInfo {
        &self.info
    }
}

/// Enumerate viewable EWMH client windows in the current X11 session.
pub fn enumerate_windows() -> Result<Vec<WindowInfo>, CaptureError> {
    let (conn, screen_num) = x11rb::connect(None).map_err(platform_error)?;
    windows_for(&conn, conn.setup().roots[screen_num].root)
}

impl CaptureBackend for X11Backend {
    fn run(
        &mut self,
        on_frame: &mut dyn FnMut(RawFrame) -> ControlFlow,
    ) -> Result<(), CaptureError> {
        self.stop.store(false, Ordering::Relaxed);
        let mut previous = None;
        while !self.stop.load(Ordering::Relaxed) {
            let info = window_info(&self.conn, self.root, self.window)?;
            let pixels = capture_bgra(&self.conn, self.window, info.rect.w, info.rect.h)?;
            let changed = previous.as_ref().is_none_or(|old: &Vec<u8>| old != &pixels);
            if changed {
                previous = Some(pixels.clone());
                let frame = RawFrame::from_bgra(
                    pixels,
                    info.rect.w,
                    info.rect.h,
                    Instant::now(),
                    Utc::now(),
                    info.clone(),
                );
                self.info = info;
                if on_frame(frame) == ControlFlow::Stop {
                    break;
                }
            }
            thread::sleep(POLL_INTERVAL);
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

fn windows_for(conn: &RustConnection, root: Window) -> Result<Vec<WindowInfo>, CaptureError> {
    let clients = atom(conn, "_NET_CLIENT_LIST")?;
    let active = property_u32(conn, root, atom(conn, "_NET_ACTIVE_WINDOW")?)?.unwrap_or(0);
    let windows = property_u32_list(conn, root, clients)?;
    let windows = if windows.is_empty() {
        // EWMH is voluntary. A plain X11 window manager can still expose its
        // top-level children directly under the root window.
        conn.query_tree(root)
            .map_err(platform_error)?
            .reply()
            .map_err(platform_error)?
            .children
    } else {
        windows
    };
    windows
        .into_iter()
        .filter_map(|window| match window_info(conn, root, window) {
            Ok(info) if info.rect.w > 0 && info.rect.h > 0 => Some(Ok(WindowInfo {
                foreground: window == active,
                ..info
            })),
            Ok(_) => None,
            // A client can disappear between _NET_CLIENT_LIST and inspection.
            Err(CaptureError::WindowClosed) => None,
            Err(error) => Some(Err(error)),
        })
        .collect()
}

fn resolve_target(
    target: &Target,
    conn: &RustConnection,
    windows: &[WindowInfo],
) -> Result<WindowInfo, CaptureError> {
    let pid_atom = atom(conn, "_NET_WM_PID")?;
    let found = match target {
        Target::ByHwnd(id) => windows.iter().find(|info| info.hwnd == *id),
        Target::ByTitleRegex(title) => {
            let needle = title.to_lowercase();
            windows
                .iter()
                .find(|info| info.title.to_lowercase().contains(&needle))
        }
        Target::ByExe(exe) => windows
            .iter()
            .find(|info| info.exe.eq_ignore_ascii_case(exe)),
        Target::ByPid(pid) => windows
            .iter()
            .filter(|info| {
                property_u32(conn, info.hwnd as u32, pid_atom)
                    .ok()
                    .flatten()
                    == Some(*pid)
            })
            .max_by_key(|info| u64::from(info.rect.w) * u64::from(info.rect.h)),
    };
    found
        .cloned()
        .ok_or_else(|| CaptureError::TargetNotFound(target_name(target)))
}

fn window_info(
    conn: &RustConnection,
    root: Window,
    window: Window,
) -> Result<WindowInfo, CaptureError> {
    let attrs = conn
        .get_window_attributes(window)
        .map_err(platform_error)?
        .reply()
        .map_err(window_error)?;
    if attrs.map_state != MapState::VIEWABLE {
        return Err(CaptureError::WindowClosed);
    }
    let geometry = conn
        .get_geometry(window)
        .map_err(platform_error)?
        .reply()
        .map_err(window_error)?;
    let position = conn
        .translate_coordinates(window, root, 0, 0)
        .map_err(platform_error)?
        .reply()
        .map_err(window_error)?;
    let title = text_property(conn, window, atom(conn, "_NET_WM_NAME")?)
        .or_else(|_| text_property(conn, window, AtomEnum::WM_NAME.into()))
        .unwrap_or_default();
    let (instance, class) = wm_class(conn, window).unwrap_or_default();
    let exe = property_u32(conn, window, atom(conn, "_NET_WM_PID")?)
        .ok()
        .and_then(process_name)
        .unwrap_or(instance);
    Ok(WindowInfo {
        hwnd: window as isize,
        title,
        exe,
        class,
        rect: Rect::new(
            position.dst_x.into(),
            position.dst_y.into(),
            geometry.width.into(),
            geometry.height.into(),
        ),
        client_rect: Rect::new(0, 0, geometry.width.into(), geometry.height.into()),
        dpi: 96,
        foreground: false,
    })
}

/// Return the executable basename for a local client process when `/proc` is
/// available. This makes `--exe` match the same process-oriented contract as
/// the Windows and macOS backends instead of relying only on `WM_CLASS`.
fn process_name(pid: u32) -> Option<String> {
    std::fs::read_to_string(format!("/proc/{pid}/comm"))
        .ok()
        .map(|name| name.trim().to_owned())
        .filter(|name| !name.is_empty())
}

fn capture_bgra(
    conn: &RustConnection,
    window: Window,
    width: u32,
    height: u32,
) -> Result<Vec<u8>, CaptureError> {
    let width = u16::try_from(width)
        .map_err(|_| CaptureError::Backend("X11 window is wider than 65535 pixels".into()))?;
    let height = u16::try_from(height)
        .map_err(|_| CaptureError::Backend("X11 window is taller than 65535 pixels".into()))?;
    let reply = conn
        .get_image(ImageFormat::Z_PIXMAP, window, 0, 0, width, height, u32::MAX)
        .map_err(platform_error)?
        .reply()
        .map_err(window_error)?;
    let visual = reply.visual;
    let image = Image::get_from_reply(conn.setup(), width, height, reply)
        .map_err(|error| CaptureError::Backend(format!("unsupported X11 image format: {error}")))?;
    let layout = pixel_layout(conn, visual)?;

    // Raw X11 images may be 16/24/32-bit and use either byte order. Decode via
    // the server's visual masks so every backend still fulfils RawFrame's
    // tightly-packed BGRA8 contract.
    let mut bgra = Vec::with_capacity(usize::from(width) * usize::from(height) * 4);
    for y in 0..height {
        for x in 0..width {
            let (red, green, blue) = layout.decode(image.get_pixel(x, y));
            bgra.extend_from_slice(&bgra_pixel(red, green, blue));
        }
    }
    Ok(bgra)
}

fn bgra_pixel(red: u16, green: u16, blue: u16) -> [u8; 4] {
    [(blue >> 8) as u8, (green >> 8) as u8, (red >> 8) as u8, 255]
}

fn pixel_layout(conn: &RustConnection, visual: u32) -> Result<PixelLayout, CaptureError> {
    let visual_type = conn
        .setup()
        .roots
        .iter()
        .flat_map(|screen| screen.allowed_depths.iter())
        .flat_map(|depth| depth.visuals.iter())
        .find(|candidate| candidate.visual_id == visual)
        .ok_or_else(|| {
            CaptureError::Backend(format!(
                "X11 visual {visual:#x} was not described by the server"
            ))
        })?;
    PixelLayout::from_visual_type(*visual_type).map_err(|error| {
        CaptureError::Backend(format!("unsupported X11 visual {visual:#x}: {error}"))
    })
}

fn atom(conn: &RustConnection, name: &str) -> Result<Atom, CaptureError> {
    conn.intern_atom(false, name.as_bytes())
        .map_err(platform_error)?
        .reply()
        .map_err(platform_error)
        .map(|reply| reply.atom)
}

fn property_u32(
    conn: &RustConnection,
    window: Window,
    property: Atom,
) -> Result<Option<u32>, CaptureError> {
    Ok(property_u32_list(conn, window, property)?
        .into_iter()
        .next())
}

fn property_u32_list(
    conn: &RustConnection,
    window: Window,
    property: Atom,
) -> Result<Vec<u32>, CaptureError> {
    let reply = conn
        .get_property(
            false,
            window,
            property,
            AtomEnum::ANY,
            0,
            ALL_PROPERTY_BYTES,
        )
        .map_err(platform_error)?
        .reply()
        .map_err(window_error)?;
    Ok(reply
        .value32()
        .map(|values| values.collect())
        .unwrap_or_default())
}

fn text_property(
    conn: &RustConnection,
    window: Window,
    property: Atom,
) -> Result<String, CaptureError> {
    let reply = conn
        .get_property(
            false,
            window,
            property,
            AtomEnum::ANY,
            0,
            ALL_PROPERTY_BYTES,
        )
        .map_err(platform_error)?
        .reply()
        .map_err(window_error)?;
    Ok(String::from_utf8_lossy(&reply.value)
        .trim_end_matches('\0')
        .to_owned())
}

fn wm_class(conn: &RustConnection, window: Window) -> Result<(String, String), CaptureError> {
    let value = text_property(conn, window, AtomEnum::WM_CLASS.into())?;
    let mut parts = value.split('\0');
    Ok((
        parts.next().unwrap_or_default().to_owned(),
        parts.next().unwrap_or_default().to_owned(),
    ))
}

fn target_name(target: &Target) -> String {
    match target {
        Target::ByHwnd(id) => format!("X11 window id {id}"),
        Target::ByTitleRegex(title) => format!("title containing {title:?}"),
        Target::ByExe(exe) => format!("X11 instance {exe:?}"),
        Target::ByPid(pid) => format!("pid {pid}"),
    }
}

fn platform_error(error: impl std::fmt::Display) -> CaptureError {
    CaptureError::Backend(format!("X11 capture failed: {error}. Start Framewatch in an X11 session with DISPLAY set (Wayland needs a portal backend)"))
}

fn window_error(error: impl std::fmt::Display) -> CaptureError {
    let message = error.to_string();
    if message.contains("Window") {
        CaptureError::WindowClosed
    } else {
        platform_error(message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn process_name_reads_the_current_process() {
        assert!(process_name(std::process::id()).is_some());
    }

    #[test]
    fn converts_decoded_rgb_to_bgra() {
        assert_eq!(bgra_pixel(0x1200, 0x3400, 0x5600), [0x56, 0x34, 0x12, 255]);
    }
}
