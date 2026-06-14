//! Configuration: [`Config`], [`ConfigBuilder`], TOML load, and ROI hints.

use crate::error::Error;
use crate::event::{ImageFormat, SaveMask};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// How to select the target window.
///
/// In TOML / JSON this is an externally-tagged map: `{ title = "..." }`,
/// `{ exe = "..." }`, or `{ hwnd = 1234 }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Target {
    /// Match by native window handle.
    #[serde(rename = "hwnd")]
    ByHwnd(isize),
    /// Match the window title against a regular expression.
    #[serde(rename = "title")]
    ByTitleRegex(String),
    /// Match by executable basename (e.g. `"Code.exe"`).
    #[serde(rename = "exe")]
    ByExe(String),
}

impl Default for Target {
    fn default() -> Self {
        Target::ByTitleRegex(String::new())
    }
}

/// The semantic kind of a region-of-interest hint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoiKind {
    /// Lower the change threshold here; always counts as meaningful.
    Watch,
    /// A busy/animation indicator; its changes never count as meaningful.
    Spinner,
    /// A rapidly-changing value; sampled on a throttle, not saved per change.
    Volatile,
    /// Excluded from diffing entirely (clocks, cursors, the WGC border).
    Ignore,
}

/// A region-of-interest hint, in client-normalized coordinates.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoiHint {
    /// The kind of region.
    pub kind: RoiKind,
    /// A human label, surfaced in the timeline.
    pub label: String,
    /// `[x, y, w, h]` in `0.0..=1.0` of the client rect.
    pub rect_norm: [f32; 4],
}

/// Image output options.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ImageOpts {
    /// Output format.
    pub format: ImageFormat,
    /// Scale factor applied before encoding (`1.0` = native).
    pub scale: f32,
}

impl Default for ImageOpts {
    fn default() -> Self {
        Self {
            format: ImageFormat::Png,
            scale: 1.0,
        }
    }
}

/// Output rotation limits for the [`DirectorySink`](crate::sink::DirectorySink).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Rotation {
    /// Maximum number of saved frames before the oldest are pruned.
    pub max_frames: u64,
    /// Maximum total bytes of saved frames before the oldest are pruned.
    pub max_bytes: u64,
}

impl Default for Rotation {
    fn default() -> Self {
        Self {
            max_frames: 5000,
            max_bytes: 2 * 1024 * 1024 * 1024, // 2 GiB
        }
    }
}

/// The full framewatch configuration.
///
/// Defaults are chosen so a typical app produces a handful of `settled` frames
/// per workflow, not hundreds. Construct via [`Config::builder`] or load from TOML.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// How to select the target window.
    pub target: Target,
    /// Output directory (a per-session subdirectory is created inside it).
    pub out_dir: PathBuf,
    /// Maximum frames per second processed.
    pub fps_cap: u32,
    /// Wait up to this many ms for the target window to appear before failing
    /// (poll/retry). `0` fails immediately if the window is absent. Default 0.
    pub wait_ms: u64,
    /// Auto-stop capture after this many ms. `0` runs until interrupted.
    pub stop_after_ms: u64,
    /// Auto-stop after this many images have been saved. `0` is unlimited.
    pub stop_after_images: u64,
    /// Auto-stop after the first `Settled` event (one-shot "money frame").
    pub stop_after_settled: bool,
    /// Floor between saved images, in milliseconds.
    pub min_emit_interval_ms: u64,
    /// Quiescence required to declare "settled", in milliseconds.
    pub settle_ms: u64,
    /// Force a capture after this many ms of *sustained* activity that never
    /// quiesces (e.g. a fullscreen video/animation), so long-running activity
    /// still yields periodic frames. `0` disables. Default 5000.
    pub max_active_ms: u64,
    /// `(cols, rows)` tile grid.
    pub tile_grid: (u16, u16),
    /// Per-tile luma delta to count a tile as changed.
    pub tile_change_threshold: u8,
    /// Minimum changed-area ratio to count as meaningful activity.
    pub meaningful_area_ratio: f32,
    /// dHash hamming distance under which an image is deduped.
    pub dedup_hamming: u32,
    /// Number of frames in the per-tile change-rate window.
    pub volatility_window: u16,
    /// Region change-rate above which a region is "busy".
    pub busy_rate_threshold: f32,
    /// Throttle for volatile-region samples, in milliseconds.
    pub value_sample_ms: u64,
    /// Whether to emit `TransitionStart` events.
    pub emit_transition_start: bool,
    /// Which event kinds get an image saved.
    pub save_image_for: SaveMask,
    /// Image output options.
    pub image: ImageOpts,
    /// Region-of-interest hints.
    pub rois: Vec<RoiHint>,
    /// Output rotation limits.
    pub rotation: Rotation,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            target: Target::default(),
            out_dir: PathBuf::from("./.framewatch"),
            fps_cap: 30,
            wait_ms: 0,
            stop_after_ms: 0,
            stop_after_images: 0,
            stop_after_settled: false,
            min_emit_interval_ms: 200,
            settle_ms: 350,
            max_active_ms: 5000,
            tile_grid: (32, 18),
            tile_change_threshold: 12,
            meaningful_area_ratio: 0.002,
            dedup_hamming: 8,
            volatility_window: 32,
            busy_rate_threshold: 0.5,
            value_sample_ms: 1000,
            emit_transition_start: false,
            save_image_for: SaveMask::default(),
            image: ImageOpts::default(),
            rois: Vec::new(),
            rotation: Rotation::default(),
        }
    }
}

impl Config {
    /// Start building a config.
    pub fn builder() -> ConfigBuilder {
        ConfigBuilder::new()
    }

    /// Load a config from a TOML file.
    pub fn from_toml_path(path: impl AsRef<Path>) -> Result<Self, Error> {
        let text = std::fs::read_to_string(path.as_ref())?;
        Self::from_toml_str(&text)
    }

    /// Parse a config from a TOML string.
    pub fn from_toml_str(text: &str) -> Result<Self, Error> {
        toml::from_str(text).map_err(|e| Error::Config(e.to_string()))
    }

    /// Serialize this config to a TOML string.
    pub fn to_toml_string(&self) -> Result<String, Error> {
        toml::to_string_pretty(self).map_err(|e| Error::Config(e.to_string()))
    }

    /// Tile-grid columns.
    pub fn cols(&self) -> u16 {
        self.tile_grid.0
    }

    /// Tile-grid rows.
    pub fn rows(&self) -> u16 {
        self.tile_grid.1
    }

    /// Validate internal consistency.
    pub fn validate(&self) -> Result<(), Error> {
        match &self.target {
            Target::ByTitleRegex(s) | Target::ByExe(s) if s.is_empty() => {
                return Err(Error::Config("target is empty".into()));
            }
            _ => {}
        }
        if self.tile_grid.0 == 0 || self.tile_grid.1 == 0 {
            return Err(Error::Config("tile_grid must be non-zero".into()));
        }
        Ok(())
    }
}

/// Fluent builder for [`Config`].
#[derive(Debug, Clone)]
pub struct ConfigBuilder {
    cfg: Config,
}

impl Default for ConfigBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl ConfigBuilder {
    /// Start from defaults.
    pub fn new() -> Self {
        Self {
            cfg: Config::default(),
        }
    }

    /// Start from an existing config.
    pub fn from_config(cfg: Config) -> Self {
        Self { cfg }
    }

    /// Set the target window.
    pub fn target(mut self, target: Target) -> Self {
        self.cfg.target = target;
        self
    }

    /// Set the output directory.
    pub fn out_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.cfg.out_dir = dir.into();
        self
    }

    /// Set the settle threshold (ms).
    pub fn settle_ms(mut self, ms: u64) -> Self {
        self.cfg.settle_ms = ms;
        self
    }

    /// Set the sustained-activity keyframe interval (ms); `0` disables.
    pub fn max_active_ms(mut self, ms: u64) -> Self {
        self.cfg.max_active_ms = ms;
        self
    }

    /// Wait up to `ms` for the target window to appear before failing.
    pub fn wait_ms(mut self, ms: u64) -> Self {
        self.cfg.wait_ms = ms;
        self
    }

    /// Auto-stop capture after `ms` (`0` runs until interrupted).
    pub fn stop_after_ms(mut self, ms: u64) -> Self {
        self.cfg.stop_after_ms = ms;
        self
    }

    /// Auto-stop after `n` images have been saved (`0` is unlimited).
    pub fn stop_after_images(mut self, n: u64) -> Self {
        self.cfg.stop_after_images = n;
        self
    }

    /// Auto-stop after the first `Settled` event.
    pub fn stop_after_settled(mut self, on: bool) -> Self {
        self.cfg.stop_after_settled = on;
        self
    }

    /// Set the volatile-region sample throttle (ms).
    pub fn value_sample_ms(mut self, ms: u64) -> Self {
        self.cfg.value_sample_ms = ms;
        self
    }

    /// Set the tile grid `(cols, rows)`.
    pub fn tile_grid(mut self, cols: u16, rows: u16) -> Self {
        self.cfg.tile_grid = (cols, rows);
        self
    }

    /// Set the FPS cap.
    pub fn fps_cap(mut self, fps: u32) -> Self {
        self.cfg.fps_cap = fps;
        self
    }

    /// Set the minimum interval between saved images (ms).
    pub fn min_emit_interval_ms(mut self, ms: u64) -> Self {
        self.cfg.min_emit_interval_ms = ms;
        self
    }

    /// Set the dedup hamming threshold.
    pub fn dedup_hamming(mut self, d: u32) -> Self {
        self.cfg.dedup_hamming = d;
        self
    }

    /// Set which kinds get images saved.
    pub fn save_image_for(mut self, mask: SaveMask) -> Self {
        self.cfg.save_image_for = mask;
        self
    }

    /// Enable/disable `TransitionStart` events.
    pub fn emit_transition_start(mut self, on: bool) -> Self {
        self.cfg.emit_transition_start = on;
        self
    }

    /// Set the image output scale.
    pub fn image_scale(mut self, scale: f32) -> Self {
        self.cfg.image.scale = scale;
        self
    }

    /// Set the image output format.
    pub fn image_format(mut self, format: ImageFormat) -> Self {
        self.cfg.image.format = format;
        self
    }

    /// Add an arbitrary ROI hint.
    pub fn roi(mut self, hint: RoiHint) -> Self {
        self.cfg.rois.push(hint);
        self
    }

    fn push_roi(mut self, kind: RoiKind, label: impl Into<String>, rect: [f32; 4]) -> Self {
        self.cfg.rois.push(RoiHint {
            kind,
            label: label.into(),
            rect_norm: rect,
        });
        self
    }

    /// Add a `Spinner` ROI.
    pub fn spinner_roi(self, label: impl Into<String>, rect: [f32; 4]) -> Self {
        self.push_roi(RoiKind::Spinner, label, rect)
    }

    /// Add a `Volatile` ROI.
    pub fn volatile_roi(self, label: impl Into<String>, rect: [f32; 4]) -> Self {
        self.push_roi(RoiKind::Volatile, label, rect)
    }

    /// Add a `Watch` ROI.
    pub fn watch_roi(self, label: impl Into<String>, rect: [f32; 4]) -> Self {
        self.push_roi(RoiKind::Watch, label, rect)
    }

    /// Add an `Ignore` ROI.
    pub fn ignore_roi(self, label: impl Into<String>, rect: [f32; 4]) -> Self {
        self.push_roi(RoiKind::Ignore, label, rect)
    }

    /// Set rotation limits.
    pub fn rotation(mut self, rotation: Rotation) -> Self {
        self.cfg.rotation = rotation;
        self
    }

    /// Finish building, validating the result.
    pub fn build(self) -> Result<Config, Error> {
        self.cfg.validate()?;
        Ok(self.cfg)
    }

    /// Finish building without validation.
    pub fn build_unchecked(self) -> Config {
        self.cfg
    }
}

/// Convenience: `EventKind` re-export for building [`SaveMask`]s near config.
pub use crate::event::EventKind as Kind;
