//! Configuration: [`Config`], [`ConfigBuilder`], TOML load, and ROI hints.

use crate::error::Error;
use crate::event::{ImageFormat, SaveMask};
use crate::frame::Rect;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// How to select the target window.
///
/// In TOML / JSON this is an externally-tagged map: `{ title = "..." }`,
/// `{ exe = "..." }`, `{ hwnd = 1234 }`, or `{ pid = 4321 }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum Target {
    /// Match by native window handle.
    #[serde(rename = "hwnd")]
    ByHwnd(isize),
    /// Match the window title by a case-insensitive substring.
    ///
    /// (The variant name is historical — matching is a literal substring, not a
    /// regex, since window titles routinely contain regex-special characters.)
    #[serde(rename = "title")]
    ByTitleRegex(String),
    /// Match by executable basename (e.g. `"Code.exe"`).
    #[serde(rename = "exe")]
    ByExe(String),
    /// Match the window owned by this process id (exact — avoids latching onto a
    /// stale window of an earlier run of the same exe).
    #[serde(rename = "pid")]
    ByPid(u32),
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
#[non_exhaustive]
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
    /// Also apply dHash dedup to *forced* `Settled` / `Manual` emits (the first
    /// `Initial` frame is always kept). `false` (default) preserves the prior
    /// behaviour of always saving forced money-frames even if byte-identical.
    pub dedup_forced: bool,
    /// Number of frames in the per-tile change-rate window.
    pub volatility_window: u16,
    /// Region change-rate above which a region is "busy".
    pub busy_rate_threshold: f32,
    /// Opt-in: automatically treat a small, compact cluster of high-change-rate
    /// tiles (no ROI hint needed) as a spinner — its changes don't count as
    /// meaningful and it drives `busy_start`/`busy_end`. Default `false`.
    /// Volatile-value detection still requires an explicit `Volatile` ROI.
    pub auto_detect_spinners: bool,
    /// Maximum fraction of the frame a high-change-rate cluster may cover to be
    /// auto-classified as a spinner (larger churn is treated as real content).
    /// Used only when `auto_detect_spinners` is set. Default `0.05`.
    pub auto_spinner_max_area: f32,
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
    /// Optional crop: capture/detect/save only this pixel region of the frame
    /// (e.g. to clip host window chrome). `None` keeps the whole frame.
    pub crop: Option<Rect>,
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
            dedup_forced: false,
            volatility_window: 32,
            busy_rate_threshold: 0.5,
            auto_detect_spinners: false,
            auto_spinner_max_area: 0.05,
            value_sample_ms: 1000,
            emit_transition_start: false,
            save_image_for: SaveMask::default(),
            image: ImageOpts::default(),
            rois: Vec::new(),
            crop: None,
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

    /// Validate internal consistency. Called by [`watch`](crate::watch) /
    /// [`watch_with`](crate::watch_with) and `ConfigBuilder::build`.
    pub fn validate(&self) -> Result<(), Error> {
        match &self.target {
            Target::ByTitleRegex(s) | Target::ByExe(s) if s.is_empty() => {
                return Err(Error::Config("target is empty".into()));
            }
            _ => {}
        }
        let (cols, rows) = self.tile_grid;
        if cols == 0 || rows == 0 {
            return Err(Error::Config("tile_grid must be non-zero".into()));
        }
        // Bound the grid: the volatility ring allocates `window * cols * rows`
        // bools, so an unbounded grid can exhaust memory.
        if cols > 512 || rows > 512 {
            return Err(Error::Config(format!(
                "tile_grid {cols}x{rows} is too large (max 512x512)"
            )));
        }
        if !(self.image.scale.is_finite() && self.image.scale > 0.0) {
            return Err(Error::Config(format!(
                "image.scale must be a positive number (got {})",
                self.image.scale
            )));
        }
        if !(self.auto_spinner_max_area.is_finite()
            && self.auto_spinner_max_area > 0.0
            && self.auto_spinner_max_area <= 1.0)
        {
            return Err(Error::Config(
                "auto_spinner_max_area must be in (0.0, 1.0]".into(),
            ));
        }
        for r in &self.rois {
            let [x, y, w, h] = r.rect_norm;
            let in_unit = |v: f32| v.is_finite() && (0.0..=1.0).contains(&v);
            let ok = in_unit(x)
                && in_unit(y)
                && w.is_finite()
                && h.is_finite()
                && w > 0.0
                && h > 0.0
                && x + w <= 1.0001
                && y + h <= 1.0001;
            if !ok {
                return Err(Error::Config(format!(
                    "ROI {:?} has out-of-range rect_norm {:?} (need x,y in 0..=1, w,h > 0, x+w<=1, y+h<=1)",
                    r.label, r.rect_norm
                )));
            }
        }
        if let Some(c) = self.crop {
            if c.w == 0 || c.h == 0 {
                return Err(Error::Config(
                    "crop width and height must be non-zero".into(),
                ));
            }
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

    /// Also dedup forced `Settled`/`Manual` emits (see [`Config::dedup_forced`]).
    pub fn dedup_forced(mut self, on: bool) -> Self {
        self.cfg.dedup_forced = on;
        self
    }

    /// Enable opt-in automatic spinner detection (see
    /// [`Config::auto_detect_spinners`]).
    pub fn auto_detect_spinners(mut self, on: bool) -> Self {
        self.cfg.auto_detect_spinners = on;
        self
    }

    /// Set the auto-spinner max area fraction (see
    /// [`Config::auto_spinner_max_area`]).
    pub fn auto_spinner_max_area(mut self, frac: f32) -> Self {
        self.cfg.auto_spinner_max_area = frac;
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

    /// Crop capture/detection/output to this pixel region (clips host chrome).
    pub fn crop(mut self, rect: Rect) -> Self {
        self.cfg.crop = Some(rect);
        self
    }

    /// Crop to `x, y, w, h` pixels (convenience over [`crop`](ConfigBuilder::crop)).
    pub fn crop_xywh(self, x: i32, y: i32, w: u32, h: u32) -> Self {
        self.crop(Rect::new(x, y, w, h))
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

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> Config {
        Config::builder()
            .target(Target::ByExe("x.exe".into()))
            .build()
            .unwrap()
    }

    #[test]
    fn rejects_empty_target() {
        let mut c = base();
        c.target = Target::ByTitleRegex(String::new());
        assert!(c.validate().is_err());
        c.target = Target::ByExe(String::new());
        assert!(c.validate().is_err());
    }

    #[test]
    fn rejects_zero_and_oversized_tile_grid() {
        let mut c = base();
        c.tile_grid = (0, 18);
        assert!(c.validate().is_err());
        c.tile_grid = (32, 0);
        assert!(c.validate().is_err());
        c.tile_grid = (1000, 1000);
        assert!(c.validate().is_err());
    }

    #[test]
    fn rejects_bad_image_scale() {
        for bad in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            let mut c = base();
            c.image.scale = bad;
            assert!(c.validate().is_err(), "scale {bad} should be rejected");
        }
    }

    #[test]
    fn rejects_bad_auto_spinner_area() {
        for bad in [0.0, -0.1, 1.5, f32::NAN] {
            let mut c = base();
            c.auto_spinner_max_area = bad;
            assert!(c.validate().is_err());
        }
    }

    #[test]
    fn rejects_out_of_range_roi() {
        let roi = |rect_norm| RoiHint {
            kind: RoiKind::Spinner,
            label: "r".into(),
            rect_norm,
        };
        for bad in [
            [0.9, 0.0, 0.5, 0.1],  // x + w > 1
            [0.0, 0.0, 0.0, 0.1],  // w == 0
            [0.0, 0.0, 0.1, 0.0],  // h == 0
            [-0.1, 0.0, 0.2, 0.2], // x < 0
            [0.0, 0.9, 0.1, 0.5],  // y + h > 1
        ] {
            let mut c = base();
            c.rois.push(roi(bad));
            assert!(c.validate().is_err(), "rect {bad:?} should be rejected");
        }
    }

    #[test]
    fn rejects_zero_size_crop() {
        let mut c = base();
        c.crop = Some(Rect::new(0, 0, 0, 10));
        assert!(c.validate().is_err());
        c.crop = Some(Rect::new(0, 0, 10, 0));
        assert!(c.validate().is_err());
    }

    #[test]
    fn accepts_a_sane_config() {
        let mut c = base();
        c.rois.push(RoiHint {
            kind: RoiKind::Watch,
            label: "w".into(),
            rect_norm: [0.1, 0.1, 0.2, 0.2],
        });
        c.crop = Some(Rect::new(0, 0, 100, 100));
        c.auto_spinner_max_area = 0.05;
        c.image.scale = 0.5;
        assert!(c.validate().is_ok());
    }

    #[test]
    fn toml_roundtrips_new_fields() {
        let c = Config::builder()
            .target(Target::ByExe("x.exe".into()))
            .auto_detect_spinners(true)
            .auto_spinner_max_area(0.1)
            .dedup_forced(true)
            .build()
            .unwrap();
        let back = Config::from_toml_str(&c.to_toml_string().unwrap()).unwrap();
        assert!(back.auto_detect_spinners);
        assert!(back.dedup_forced);
        assert_eq!(back.auto_spinner_max_area, 0.1);
    }

    #[test]
    fn builder_unchecked_skips_validation() {
        let cfg = ConfigBuilder::new().tile_grid(0, 0).build_unchecked();
        assert_eq!(cfg.tile_grid, (0, 0));
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn builder_methods_set_every_field() {
        let cfg = Config::builder()
            .target(Target::ByExe("z.exe".into()))
            .out_dir("/tmp/x")
            .settle_ms(111)
            .max_active_ms(2222)
            .wait_ms(333)
            .stop_after_ms(444)
            .stop_after_images(5)
            .stop_after_settled(true)
            .value_sample_ms(666)
            .tile_grid(40, 20)
            .fps_cap(15)
            .min_emit_interval_ms(77)
            .dedup_hamming(3)
            .dedup_forced(true)
            .auto_detect_spinners(true)
            .auto_spinner_max_area(0.2)
            .save_image_for(SaveMask::from_kinds(&[Kind::Settled]))
            .emit_transition_start(true)
            .image_scale(0.5)
            .image_format(ImageFormat::Png)
            .crop_xywh(1, 2, 3, 4)
            .spinner_roi("s", [0.0, 0.0, 0.1, 0.1])
            .volatile_roi("v", [0.1, 0.0, 0.1, 0.1])
            .watch_roi("w", [0.2, 0.0, 0.1, 0.1])
            .ignore_roi("i", [0.3, 0.0, 0.1, 0.1])
            .roi(RoiHint {
                kind: RoiKind::Watch,
                label: "extra".into(),
                rect_norm: [0.4, 0.0, 0.1, 0.1],
            })
            .rotation(Rotation {
                max_frames: 9,
                max_bytes: 99,
            })
            .build()
            .unwrap();
        assert_eq!((cfg.cols(), cfg.rows()), (40, 20));
        assert_eq!(cfg.settle_ms, 111);
        assert_eq!(cfg.max_active_ms, 2222);
        assert_eq!(cfg.fps_cap, 15);
        assert_eq!(cfg.crop, Some(Rect::new(1, 2, 3, 4)));
        assert_eq!(cfg.rois.len(), 5);
        assert_eq!(cfg.rotation.max_frames, 9);
        assert!(cfg.dedup_forced && cfg.auto_detect_spinners);

        // from_config seeds a builder from an existing config.
        let again = ConfigBuilder::from_config(cfg.clone()).build().unwrap();
        assert_eq!(again.settle_ms, cfg.settle_ms);
    }

    #[test]
    fn from_toml_path_reads_a_file() {
        let p = std::env::temp_dir().join("fw_cfg_roundtrip_test.toml");
        std::fs::write(&p, "settle_ms = 999\ntarget = { exe = \"a.exe\" }\n").unwrap();
        let cfg = Config::from_toml_path(&p).unwrap();
        assert_eq!(cfg.settle_ms, 999);
        let _ = std::fs::remove_file(&p);
        // A bad path is a clean error, not a panic.
        assert!(Config::from_toml_path("/no/such/framewatch.toml").is_err());
        // Malformed TOML is a Config error.
        assert!(Config::from_toml_str("settle_ms = \"not a number\"").is_err());
    }
}
