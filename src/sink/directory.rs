//! The default sink: writes PNGs, `timeline.jsonl`, `session.json`, and a
//! `README_FOR_AGENT.md` into a per-session directory.

use crate::config::{Config, Rotation};
use crate::error::SinkError;
use crate::event::CaptureEvent;
use crate::mark::{self, MarkRecord, MarkTail};
use crate::session::{Session, SessionManifest};
use crate::sink::Sink;
use chrono::Utc;
use std::collections::VecDeque;
use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// A queue of marks waiting to be attached to the next captured frame.
///
/// Shared with whatever is feeding labels in — a `--labels-file` tail, or a
/// watcher noticing `framewatch mark`. Cloning it is how a producer gets a
/// handle; the sink drains it on every event.
#[derive(Clone, Default)]
pub struct MarkInbox {
    queue: Arc<Mutex<Vec<PendingMark>>>,
}

/// One queued mark, and whether the sink still owes it a timeline line.
#[derive(Clone)]
struct PendingMark {
    record: MarkRecord,
    /// `false` when the producer already appended the line itself.
    write_line: bool,
}

impl MarkInbox {
    /// Queue a mark and let the sink write its timeline line.
    pub fn push(&self, record: MarkRecord) {
        self.enqueue(record, true);
    }

    /// Queue a mark whose timeline line another process already wrote.
    ///
    /// `framewatch mark` appends its own line so the mark survives with no
    /// watcher running; the sink must not write a second one.
    pub fn push_already_written(&self, record: MarkRecord) {
        self.enqueue(record, false);
    }

    fn enqueue(&self, record: MarkRecord, write_line: bool) {
        // A poisoned lock means a producer thread panicked mid-push. Dropping
        // the mark is better than taking the capture loop down with it.
        if let Ok(mut queue) = self.queue.lock() {
            queue.push(PendingMark { record, write_line });
        }
    }

    fn drain(&self) -> Vec<PendingMark> {
        match self.queue.lock() {
            Ok(mut queue) => std::mem::take(&mut *queue),
            Err(_) => Vec::new(),
        }
    }
}

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

Marks:
- A line whose `kind` is "mark" is an application label, not a capture — no window,
  change, or image fields. Read it for what the app said it was doing.
- A frame that follows one or more marks repeats them in `marks_since_last_frame`,
  and when exactly one preceded it the file is named after it
  (`frames/000004_settled_before-checkout.png`). Prefer that to matching timestamps.

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
    marks: MarkInbox,
    tails: Vec<LabelTail>,
}

/// A file being followed for labels, and who owes the timeline line.
struct LabelTail {
    tail: MarkTail,
    /// `false` for `marks.pending`, whose lines `framewatch mark` already wrote.
    write_line: bool,
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

        let pending = session.dir.join(crate::mark::PENDING_FILE);

        Ok(Self {
            session,
            manifest,
            image_ext: config.image.format.ext().to_string(),
            rotation: config.rotation,
            saved: VecDeque::new(),
            total_bytes: 0,
            timeline: BufWriter::new(timeline),
            marks: MarkInbox::default(),
            // A `framewatch mark` in another process announces itself here. The
            // file usually never exists, which costs one failed metadata call
            // per frame.
            tails: vec![LabelTail {
                tail: MarkTail::new(pending),
                write_line: false,
            }],
        })
    }

    /// The session this sink writes to.
    pub fn session(&self) -> &Session {
        &self.session
    }

    /// A handle for pushing marks that should label the next frame.
    pub fn marks(&self) -> MarkInbox {
        self.marks.clone()
    }

    /// Follow a file the application appends labels to.
    ///
    /// Read at frame time along with everything else, so a label written just
    /// before a frame lands on that frame rather than the one after it.
    pub fn tail_labels(&mut self, path: impl Into<std::path::PathBuf>) {
        self.tails.push(LabelTail {
            tail: MarkTail::new(path),
            write_line: true,
        });
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

    /// Read every followed file, queueing whatever has arrived.
    fn collect_tailed(&mut self) {
        for source in &mut self.tails {
            for line in source.tail.poll() {
                if source.write_line {
                    if let Some(record) = mark::parse_label_line(&line) {
                        self.marks.push(record);
                    }
                } else if let Ok(record) = serde_json::from_str::<MarkRecord>(&line) {
                    self.marks.push_already_written(record);
                }
            }
        }
    }

    /// Drain the inbox, writing the lines this sink owes, and return the labels.
    fn take_marks(&mut self) -> Result<Vec<String>, SinkError> {
        self.collect_tailed();
        let pending = self.marks.drain();
        for queued in &pending {
            if !queued.write_line {
                continue;
            }
            let mut record = queued.record.clone();
            record.session_id = self.session.id.clone();
            if record.elapsed_ms.is_none() {
                record = record.in_session(&self.session.id, self.session.started_at);
            }
            let line = serde_json::to_string(&record)?;
            self.timeline.write_all(line.as_bytes())?;
            self.timeline.write_all(b"\n")?;
        }
        Ok(pending
            .iter()
            .map(|queued| queued.record.note.clone())
            .collect())
    }

    /// Write whatever is queued without a frame to hang it on.
    fn write_pending_marks(&mut self) -> Result<(), SinkError> {
        self.take_marks()?;
        self.timeline.flush()?;
        Ok(())
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

        // Marks happened before this frame, so their lines go first and the
        // labels ride along on the frame they describe.
        meta.marks_since_last_frame = self.take_marks()?;

        // Write the image, if any.
        if let Some(img) = &event.image {
            // One label is a name; several are ambiguous, so the frame keeps its
            // plain name and the labels stay in `marks_since_last_frame`.
            let fname = match meta.marks_since_last_frame.as_slice() {
                [only] => {
                    let slug = mark::slug(only);
                    if slug.is_empty() {
                        format!("{:06}_{}.{}", meta.seq, meta.kind.as_str(), self.image_ext)
                    } else {
                        format!(
                            "{:06}_{}_{}.{}",
                            meta.seq,
                            meta.kind.as_str(),
                            slug,
                            self.image_ext
                        )
                    }
                }
                _ => format!("{:06}_{}.{}", meta.seq, meta.kind.as_str(), self.image_ext),
            };
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
        // Marks queued after the last frame would otherwise be dropped, and the
        // labels that matter most — an error the app reported just before the
        // run ended — tend to arrive exactly there.
        self.write_pending_marks()?;
        self.manifest.ended_at = Some(Utc::now());
        self.write_manifest_now()?;
        self.timeline.flush()?;
        Ok(())
    }
}

impl Drop for DirectorySink {
    fn drop(&mut self) {
        let _ = self.write_pending_marks();
        if self.manifest.ended_at.is_none() {
            self.manifest.ended_at = Some(Utc::now());
            let _ = self.write_manifest_now();
        }
        let _ = self.timeline.flush();
    }
}
