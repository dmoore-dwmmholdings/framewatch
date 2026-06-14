//! The default sink: writes PNGs, `timeline.jsonl`, `session.json`, and a
//! `README_FOR_AGENT.md` into a per-session directory.

use crate::config::{Config, Rotation};
use crate::error::SinkError;
use crate::event::CaptureEvent;
use crate::session::{Session, SessionManifest};
use crate::sink::Sink;
use chrono::Utc;
use std::collections::VecDeque;
use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::PathBuf;

const README_FOR_AGENT: &str = r#"# framewatch session

This directory is an automatically-captured, de-duplicated visual log of a single
application window. To understand what happened:

1. Read `session.json` — target app, time range, config, and region hints.
2. Stream `timeline.jsonl` (one JSON event per line, chronological). Each event has a
   `kind`, `elapsed_ms`, an optional `image` path, and a human `note`.
3. You usually only need to open images for events with `kind` = "settled" or "busy_end";
   those are stable, meaningful states. `coalesced_frames` tells you how much activity
   each image represents. Use "value_sample"/"busy_start" notes for timing without images.

Frames are PNGs under `frames/`. There is intentionally no continuous stream — the gaps
are quiescent or were collapsed as animation/noise.

Notes:
- `window.rect` is `[x, y, width, height]` (NOT `[left, top, right, bottom]`), in
  virtual-desktop pixels — `x`/`y` may be negative or large on multi-monitor setups.
- If the session was captured with a crop/ROI, the saved images are that sub-region,
  while `window.rect` still describes the full source window.
- A perfectly static target may only produce the `initial` frame (nothing to settle
  from); that frame is the stable capture.
"#;

/// Writes a full framewatch session to a directory.
pub struct DirectorySink {
    session: Session,
    manifest: SessionManifest,
    image_ext: String,
    rotation: Rotation,
    saved: VecDeque<(PathBuf, u64)>,
    total_bytes: u64,
    timeline: BufWriter<File>,
}

impl DirectorySink {
    /// Create a sink for `config`, deriving the session id from the current time.
    pub fn new(config: &Config) -> Result<Self, SinkError> {
        Self::with_options(config, Utc::now(), "config")
    }

    /// Create a sink, specifying the start time and how the target was selected.
    pub fn with_options(
        config: &Config,
        started_at: chrono::DateTime<Utc>,
        selected_via: &str,
    ) -> Result<Self, SinkError> {
        let hint = crate::session::target_hint(&config.target);
        let session = Session::new(&config.out_dir, started_at, &hint);
        std::fs::create_dir_all(session.frames_dir())?;

        // README + initial manifest.
        std::fs::write(session.readme_path(), README_FOR_AGENT)?;
        let manifest = SessionManifest::new(&session, config, selected_via);
        write_manifest(&session.manifest_path(), &manifest)?;

        let timeline = OpenOptions::new()
            .create(true)
            .append(true)
            .open(session.timeline_path())?;

        Ok(Self {
            session,
            manifest,
            image_ext: config.image.format.ext().to_string(),
            rotation: config.rotation,
            saved: VecDeque::new(),
            total_bytes: 0,
            timeline: BufWriter::new(timeline),
        })
    }

    /// The session this sink writes to.
    pub fn session(&self) -> &Session {
        &self.session
    }

    fn enforce_rotation(&mut self) {
        while self.saved.len() as u64 > self.rotation.max_frames
            || self.total_bytes > self.rotation.max_bytes
        {
            let Some((path, size)) = self.saved.pop_front() else {
                break;
            };
            let _ = std::fs::remove_file(&path);
            self.total_bytes = self.total_bytes.saturating_sub(size);
        }
    }

    fn write_manifest_now(&mut self) -> Result<(), SinkError> {
        write_manifest(&self.session.manifest_path(), &self.manifest)
    }
}

fn write_manifest(path: &std::path::Path, manifest: &SessionManifest) -> Result<(), SinkError> {
    let json = serde_json::to_string_pretty(manifest)?;
    std::fs::write(path, json)?;
    Ok(())
}

impl Sink for DirectorySink {
    fn on_event(&mut self, event: &CaptureEvent) -> Result<(), SinkError> {
        let mut meta = event.meta.clone();
        meta.session_id = self.session.id.clone();

        // Write the image, if any.
        if let Some(img) = &event.image {
            let fname = format!("{:06}_{}.{}", meta.seq, meta.kind.as_str(), self.image_ext);
            let rel = format!("frames/{fname}");
            let abs = self.session.frames_dir().join(&fname);
            std::fs::write(&abs, &img.bytes)?;
            let size = img.bytes.len() as u64;
            meta.image = Some(rel);
            self.manifest.counts.images_saved += 1;
            self.total_bytes += size;
            self.saved.push_back((abs, size));
            self.enforce_rotation();
        }

        // Append the timeline line.
        let line = serde_json::to_string(&meta)?;
        self.timeline.write_all(line.as_bytes())?;
        self.timeline.write_all(b"\n")?;
        self.timeline.flush()?;

        // Update counts and rewrite the manifest (events are sparse by design).
        self.manifest.counts.events += 1;
        self.manifest.counts.frames_observed += 1 + meta.coalesced_frames as u64;
        self.write_manifest_now()?;

        Ok(())
    }

    fn flush(&mut self) -> Result<(), SinkError> {
        self.manifest.ended_at = Some(Utc::now());
        self.write_manifest_now()?;
        self.timeline.flush()?;
        Ok(())
    }
}

impl Drop for DirectorySink {
    fn drop(&mut self) {
        if self.manifest.ended_at.is_none() {
            self.manifest.ended_at = Some(Utc::now());
            let _ = self.write_manifest_now();
        }
        let _ = self.timeline.flush();
    }
}
