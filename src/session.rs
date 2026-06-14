//! Session identity, paths, and the `session.json` manifest.

use crate::config::{Config, RoiHint, Target};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Build the session id `"%Y-%m-%dT%H-%M-%S_<exe-stem>"`.
pub fn make_session_id(started_at: DateTime<Utc>, exe_hint: &str) -> String {
    let stem = exe_stem(exe_hint);
    format!("{}_{}", started_at.format("%Y-%m-%dT%H-%M-%S"), stem)
}

/// Reduce an exe / target string to a filesystem-friendly stem.
fn exe_stem(exe: &str) -> String {
    let base = exe.rsplit(['/', '\\']).next().unwrap_or(exe);
    let stem = base.strip_suffix(".exe").unwrap_or(base);
    let cleaned: String = stem
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let trimmed = cleaned.trim_matches('-');
    if trimmed.is_empty() {
        "window".to_string()
    } else {
        trimmed.to_string()
    }
}

/// Derive a hint for the session id from a [`Target`].
pub fn target_hint(target: &Target) -> String {
    match target {
        Target::ByExe(e) => e.clone(),
        Target::ByTitleRegex(t) => t.clone(),
        Target::ByHwnd(h) => format!("hwnd{h}"),
        Target::ByPid(p) => format!("pid{p}"),
    }
}

/// Resolved on-disk locations for a session.
#[derive(Debug, Clone)]
pub struct Session {
    /// Session identifier.
    pub id: String,
    /// The session directory (`<out_dir>/<id>`).
    pub dir: PathBuf,
    /// When the session started.
    pub started_at: DateTime<Utc>,
}

impl Session {
    /// Create a session rooted under `out_dir`, deriving the id from `started_at` and `exe_hint`.
    pub fn new(out_dir: &Path, started_at: DateTime<Utc>, exe_hint: &str) -> Self {
        let id = make_session_id(started_at, exe_hint);
        let dir = out_dir.join(&id);
        Self {
            id,
            dir,
            started_at,
        }
    }

    /// The `frames/` subdirectory.
    pub fn frames_dir(&self) -> PathBuf {
        self.dir.join("frames")
    }

    /// The `timeline.jsonl` path.
    pub fn timeline_path(&self) -> PathBuf {
        self.dir.join("timeline.jsonl")
    }

    /// The `session.json` path.
    pub fn manifest_path(&self) -> PathBuf {
        self.dir.join("session.json")
    }

    /// The `README_FOR_AGENT.md` path.
    pub fn readme_path(&self) -> PathBuf {
        self.dir.join("README_FOR_AGENT.md")
    }
}

/// Target descriptor inside the manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestTarget {
    /// Window title, if known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Executable basename, if known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exe: Option<String>,
    /// Window class, if known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub class: Option<String>,
    /// How the target was selected (e.g. `"gui"`, `"cli"`, `"config"`).
    pub selected_via: String,
}

/// A compact view of config knobs recorded in the manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestConfig {
    /// Settle threshold (ms).
    pub settle_ms: u64,
    /// `[cols, rows]` tile grid.
    pub tile_grid: [u16; 2],
    /// Dedup hamming threshold.
    pub dedup_hamming: u32,
    /// Volatile sample throttle (ms).
    pub value_sample_ms: u64,
}

/// Running counts recorded in the manifest.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct ManifestCounts {
    /// Frames observed by the engine.
    pub frames_observed: u64,
    /// Images written to disk.
    pub images_saved: u64,
    /// Events emitted.
    pub events: u64,
}

/// The `session.json` manifest, written at start and updated on shutdown.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionManifest {
    /// Session id.
    pub session_id: String,
    /// Tool name + version.
    pub tool: String,
    /// Target descriptor.
    pub target: ManifestTarget,
    /// Start timestamp.
    pub started_at: DateTime<Utc>,
    /// End timestamp (set on shutdown).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<DateTime<Utc>>,
    /// Recorded config knobs.
    pub config: ManifestConfig,
    /// ROI hints in effect.
    pub roi_hints: Vec<RoiHint>,
    /// Running counts.
    pub counts: ManifestCounts,
    /// Relative path to the timeline file.
    pub timeline: String,
}

impl SessionManifest {
    /// Build the initial manifest for a session.
    pub fn new(session: &Session, config: &Config, selected_via: &str) -> Self {
        let (title, exe) = match &config.target {
            Target::ByTitleRegex(t) => (Some(t.clone()), None),
            Target::ByExe(e) => (None, Some(e.clone())),
            Target::ByHwnd(_) | Target::ByPid(_) => (None, None),
        };
        Self {
            session_id: session.id.clone(),
            tool: format!("framewatch {}", env!("CARGO_PKG_VERSION")),
            target: ManifestTarget {
                title,
                exe,
                class: None,
                selected_via: selected_via.to_string(),
            },
            started_at: session.started_at,
            ended_at: None,
            config: ManifestConfig {
                settle_ms: config.settle_ms,
                tile_grid: [config.cols(), config.rows()],
                dedup_hamming: config.dedup_hamming,
                value_sample_ms: config.value_sample_ms,
            },
            roi_hints: config.rois.clone(),
            counts: ManifestCounts::default(),
            timeline: "timeline.jsonl".to_string(),
        }
    }
}
