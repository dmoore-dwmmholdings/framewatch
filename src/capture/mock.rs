//! A cross-platform mock backend that replays in-memory or decoded-PNG frames.
//!
//! Used by CI, `examples/embed.rs`, and tests so the full
//! capture → engine → sink pipeline runs on any OS.

use crate::capture::{CaptureBackend, ControlFlow};
use crate::error::CaptureError;
use crate::frame::{RawFrame, WindowInfo};
use chrono::{Duration as ChronoDuration, Utc};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Replays a fixed sequence of frames at a controllable cadence.
pub struct MockBackend {
    frames: Vec<RawFrame>,
    idx: usize,
    stopped: bool,
    /// Real-time delay between delivered frames (default 0 — replay as fast as possible).
    pace: Option<Duration>,
}

impl MockBackend {
    /// Build a backend from prepared frames.
    pub fn new(frames: Vec<RawFrame>) -> Self {
        Self {
            frames,
            idx: 0,
            stopped: false,
            pace: None,
        }
    }

    /// Sleep `dur` between delivered frames (to mimic a live feed in demos).
    pub fn with_pace(mut self, dur: Duration) -> Self {
        self.pace = Some(dur);
        self
    }

    /// Build a backend from BGRA buffers, assigning synthetic timestamps spaced
    /// `interval` apart.
    pub fn from_bgra_frames(
        buffers: Vec<(u32, u32, Vec<u8>)>,
        interval: Duration,
        window: WindowInfo,
    ) -> Self {
        let base = Instant::now();
        let base_wall = Utc::now();
        let frames = buffers
            .into_iter()
            .enumerate()
            .map(|(i, (w, h, buf))| {
                let dt = interval * i as u32;
                RawFrame {
                    buffer: Arc::from(buf.into_boxed_slice()),
                    width: w,
                    height: h,
                    stride: w * 4,
                    captured_at: base + dt,
                    wall_time: base_wall + ChronoDuration::milliseconds(dt.as_millis() as i64),
                    window: window.clone(),
                }
            })
            .collect();
        Self::new(frames)
    }

    /// Load frames by decoding PNG files matching `pattern`.
    ///
    /// `pattern` may contain a single `*` wildcard in the file-name component, e.g.
    /// `"tests/fixtures/*.png"`. Files are sorted by name and spaced 33 ms apart.
    pub fn from_pngs(pattern: &str) -> Result<Self, CaptureError> {
        let paths = expand_glob(pattern)?;
        if paths.is_empty() {
            return Err(CaptureError::Decode(format!(
                "no PNG files matched pattern: {pattern}"
            )));
        }

        let base = Instant::now();
        let base_wall = Utc::now();
        let interval = Duration::from_millis(33);
        let mut frames = Vec::with_capacity(paths.len());
        for (i, path) in paths.iter().enumerate() {
            let img = image::open(path)
                .map_err(|e| CaptureError::Decode(format!("{}: {e}", path.display())))?
                .to_rgba8();
            let (w, h) = (img.width(), img.height());
            // RGBA -> BGRA.
            let mut bgra = img.into_raw();
            for px in bgra.chunks_exact_mut(4) {
                px.swap(0, 2);
            }
            let dt = interval * i as u32;
            let title = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("mock")
                .to_string();
            frames.push(RawFrame {
                buffer: Arc::from(bgra.into_boxed_slice()),
                width: w,
                height: h,
                stride: w * 4,
                captured_at: base + dt,
                wall_time: base_wall + ChronoDuration::milliseconds(dt.as_millis() as i64),
                window: WindowInfo::synthetic(title, w, h),
            });
        }
        Ok(Self::new(frames))
    }

    /// Number of frames queued.
    pub fn len(&self) -> usize {
        self.frames.len()
    }

    /// Whether there are no frames.
    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }
}

impl CaptureBackend for MockBackend {
    fn run(
        &mut self,
        on_frame: &mut dyn FnMut(RawFrame) -> ControlFlow,
    ) -> Result<(), CaptureError> {
        self.stopped = false;
        while self.idx < self.frames.len() {
            if self.stopped {
                break;
            }
            if let Some(p) = self.pace {
                if self.idx > 0 {
                    std::thread::sleep(p);
                }
            }
            let frame = self.frames[self.idx].clone();
            self.idx += 1;
            if let ControlFlow::Stop = on_frame(frame) {
                break;
            }
        }
        Ok(())
    }

    fn stop(&mut self) {
        self.stopped = true;
    }
}

/// Expand a path pattern with at most one `*` in the file-name component.
fn expand_glob(pattern: &str) -> Result<Vec<std::path::PathBuf>, CaptureError> {
    let path = Path::new(pattern);

    // No wildcard: treat as a literal file.
    if !pattern.contains('*') {
        return Ok(vec![path.to_path_buf()]);
    }

    let parent = path.parent().filter(|p| !p.as_os_str().is_empty());
    let file = path
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| CaptureError::Decode(format!("invalid pattern: {pattern}")))?;

    let dir = parent.unwrap_or_else(|| Path::new("."));
    let (prefix, suffix) = file
        .split_once('*')
        .ok_or_else(|| CaptureError::Decode(format!("invalid pattern: {pattern}")))?;

    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if name.starts_with(prefix)
            && name.ends_with(suffix)
            && name.len() >= prefix.len() + suffix.len()
        {
            out.push(entry.path());
        }
    }
    out.sort();
    Ok(out)
}
