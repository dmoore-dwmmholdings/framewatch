//! Agent-supplied marks: labels that land in `timeline.jsonl` next to captures.
//!
//! A capture event says *when* the window changed. A mark says *what the
//! application thought it was doing* — "signed in", "before-checkout",
//! "permission-denied" — and it comes from the app, not from pixels. Putting
//! both in one file means a reader never has to align two clocks: the label is
//! already sitting beside the frame it describes.
//!
//! Two producers, one shape:
//!
//! - `framewatch mark` appends the line itself, so a mark survives whether or
//!   not a watcher is running.
//! - `framewatch watch --labels-file <path>` tails a file the application
//!   appends to, and the watcher writes the lines.
//!
//! Marks written while a watcher is live are also attached to the next captured
//! frame as `marks_since_last_frame`, which is what lets that frame be named
//! after the label.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;

/// The `kind` value every mark line carries.
pub const MARK_KIND: &str = "mark";

/// The file a running watcher tails to learn about out-of-process marks.
pub const PENDING_FILE: &str = "marks.pending";

/// One mark, as written to `timeline.jsonl`.
///
/// Deliberately not a [`CaptureMeta`](crate::CaptureMeta): a mark has no window
/// rect, no diff, and no image, and inventing those would make a reader trust
/// numbers nobody measured. Timeline lines are a union discriminated by `kind`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarkRecord {
    /// Session this mark belongs to. Empty when the writer does not know it.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub session_id: String,
    /// Always `"mark"`.
    pub kind: String,
    /// Wall-clock time the mark was made.
    pub wall_time: DateTime<Utc>,
    /// Milliseconds since session start, when it can be worked out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub elapsed_ms: Option<u64>,
    /// The label itself.
    pub note: String,
    /// Anything structured the application wanted to carry along.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

impl MarkRecord {
    /// A mark made now, with no session context.
    pub fn new(note: impl Into<String>) -> Self {
        Self {
            session_id: String::new(),
            kind: MARK_KIND.to_string(),
            wall_time: Utc::now(),
            elapsed_ms: None,
            note: note.into(),
            data: None,
        }
    }

    /// Attach the session id and the elapsed time measured from `started_at`.
    pub fn in_session(mut self, session_id: &str, started_at: DateTime<Utc>) -> Self {
        self.session_id = session_id.to_string();
        let ms = self
            .wall_time
            .signed_duration_since(started_at)
            .num_milliseconds();
        self.elapsed_ms = Some(ms.max(0) as u64);
        self
    }

    /// Attach a structured payload.
    pub fn with_data(mut self, data: serde_json::Value) -> Self {
        self.data = Some(data);
        self
    }
}

/// Append one JSON line to `path`, creating it if needed.
///
/// Opened in append mode for every write and closed again immediately: a
/// watcher holds `timeline.jsonl` open at the same time, and appending a single
/// short line is the one write pattern both processes can do safely without a
/// lock. Each line is written in one `write_all`, so a reader never sees half a
/// record.
pub fn append_line(path: &Path, line: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    let mut buf = String::with_capacity(line.len() + 1);
    buf.push_str(line);
    buf.push('\n');
    file.write_all(buf.as_bytes())?;
    file.flush()?;
    Ok(())
}

/// Append a mark to a session's `timeline.jsonl`.
pub fn append_to_timeline(session_dir: &Path, mark: &MarkRecord) -> std::io::Result<()> {
    let line = serde_json::to_string(mark)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    append_line(&session_dir.join("timeline.jsonl"), &line)
}

/// Tell a running watcher about a mark, so it can label the next frame.
///
/// Separate from the timeline write on purpose: whoever writes the timeline line
/// owns it, and this file only carries the notification. If no watcher is
/// running the file is simply never read.
pub fn notify_pending(session_dir: &Path, mark: &MarkRecord) -> std::io::Result<()> {
    let line = serde_json::to_string(mark)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    append_line(&session_dir.join(PENDING_FILE), &line)
}

/// Follows a newline-delimited file, returning complete new lines on demand.
///
/// Read at the moment a frame is captured rather than on a background poll: a
/// poll interval is a window in which a mark made just before a frame captions
/// the *next* one instead, and "just before the frame" is exactly when an
/// application marks things. Reading here costs one metadata call plus the new
/// bytes, which is nothing next to encoding an image.
///
/// Starts at the end of an existing file, so a re-run does not replay the last
/// run's labels.
#[derive(Debug)]
pub struct MarkTail {
    path: std::path::PathBuf,
    offset: u64,
    carry: String,
}

impl MarkTail {
    /// Follow `path` from its current end.
    pub fn new(path: impl Into<std::path::PathBuf>) -> Self {
        let path = path.into();
        let offset = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        Self {
            path,
            offset,
            carry: String::new(),
        }
    }

    /// The file being followed.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Every complete line appended since the last call.
    ///
    /// A trailing fragment is held back until its newline arrives, so a reader
    /// never sees half a label. A file that shrank was truncated or replaced, so
    /// reading restarts from the beginning rather than from a stale offset.
    pub fn poll(&mut self) -> Vec<String> {
        let Ok(len) = std::fs::metadata(&self.path).map(|m| m.len()) else {
            return Vec::new();
        };
        if len < self.offset {
            self.offset = 0;
            self.carry.clear();
        }
        if len == self.offset {
            return Vec::new();
        }
        let Ok(bytes) = read_range(&self.path, self.offset, len) else {
            return Vec::new();
        };
        self.offset = len;
        self.carry.push_str(&String::from_utf8_lossy(&bytes));

        let Some(at) = self.carry.rfind('\n') else {
            return Vec::new();
        };
        let complete: String = self.carry.drain(..=at).collect();
        complete.lines().map(|line| line.to_string()).collect()
    }
}

fn read_range(path: &Path, from: u64, to: u64) -> std::io::Result<Vec<u8>> {
    use std::io::{Read, Seek, SeekFrom};
    let mut file = std::fs::File::open(path)?;
    file.seek(SeekFrom::Start(from))?;
    let mut buf = vec![0u8; (to - from) as usize];
    file.read_exact(&mut buf)?;
    Ok(buf)
}

/// Turn a label into something safe for a filename.
///
/// Lowercase, ASCII alphanumerics plus `-` and `.`; every run of anything else
/// collapses to a single `-`. Truncated to 48 characters so a long label cannot
/// push a path over the limit, and cut at a `-` when one is near the end so the
/// result does not stop mid-word.
pub fn slug(label: &str) -> String {
    let mut out = String::with_capacity(label.len());
    let mut last_dash = false;
    for ch in label.chars() {
        if ch.is_ascii_alphanumeric() || ch == '.' {
            out.push(ch.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash {
            out.push('-');
            last_dash = true;
        }
    }
    let trimmed = out.trim_matches('-');
    if trimmed.len() <= 48 {
        return trimmed.to_string();
    }
    let cut = &trimmed[..48];
    match cut.rfind('-') {
        Some(at) if at >= 24 => cut[..at].to_string(),
        _ => cut.trim_end_matches('-').to_string(),
    }
}

/// Read a line an application wrote to a `--labels-file` into a mark.
///
/// A bare line is the label. A JSON object is kept whole in `data`, and its
/// `note`, `label`, or `kind` field — whichever comes first — becomes the label,
/// so an app can append its own event objects without reshaping them. A blank
/// line is skipped.
pub fn parse_label_line(line: &str) -> Option<MarkRecord> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) else {
        return Some(MarkRecord::new(trimmed));
    };
    let serde_json::Value::Object(ref map) = value else {
        return Some(MarkRecord::new(trimmed));
    };
    let note = ["note", "label", "kind"]
        .iter()
        .find_map(|key| map.get(*key).and_then(|v| v.as_str()))
        .unwrap_or(trimmed)
        .to_string();
    Some(MarkRecord::new(note).with_data(value))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_keeps_words_and_collapses_the_rest() {
        assert_eq!(slug("before-checkout"), "before-checkout");
        assert_eq!(
            slug("permission-denied @ orders"),
            "permission-denied-orders"
        );
        assert_eq!(slug("Ready!"), "ready");
        assert_eq!(slug("buyer.01.k3f9"), "buyer.01.k3f9");
        assert_eq!(slug("  spaced  out  "), "spaced-out");
    }

    #[test]
    fn slug_survives_a_label_made_only_of_punctuation() {
        assert_eq!(slug("///"), "");
        assert_eq!(slug(""), "");
    }

    #[test]
    fn slug_truncates_on_a_word_boundary_when_it_can() {
        let long = "before-we-place-the-order-and-then-check-the-receipt-page-loads";
        let s = slug(long);
        assert!(s.len() <= 48, "{s} is {} chars", s.len());
        assert!(!s.ends_with('-'));
        assert!(long.starts_with(&s));
    }

    #[test]
    fn slug_truncates_hard_when_there_is_no_boundary() {
        let long = "a".repeat(80);
        assert_eq!(slug(&long).len(), 48);
    }

    #[test]
    fn a_plain_line_is_the_label() {
        let mark = parse_label_line("ready").unwrap();
        assert_eq!(mark.note, "ready");
        assert_eq!(mark.kind, MARK_KIND);
        assert!(mark.data.is_none());
    }

    #[test]
    fn a_json_line_keeps_its_payload_and_finds_a_label() {
        let mark = parse_label_line(r#"{"kind":"route","route":"/orders","seq":3}"#).unwrap();
        assert_eq!(mark.note, "route");
        assert_eq!(mark.data.as_ref().unwrap()["route"], "/orders");

        let labelled = parse_label_line(r#"{"note":"before-admin","seq":4}"#).unwrap();
        assert_eq!(labelled.note, "before-admin");
    }

    #[test]
    fn blank_lines_are_skipped_and_broken_json_is_a_label() {
        assert!(parse_label_line("   ").is_none());
        assert!(parse_label_line("").is_none());
        let mark = parse_label_line(r#"{"kind":"#).unwrap();
        assert_eq!(mark.note, r#"{"kind":"#);
    }

    #[test]
    fn elapsed_is_measured_from_the_session_start() {
        let started = Utc::now() - chrono::Duration::milliseconds(2500);
        let mark = MarkRecord::new("x").in_session("s1", started);
        assert_eq!(mark.session_id, "s1");
        let ms = mark.elapsed_ms.unwrap();
        assert!((2400..3000).contains(&ms), "elapsed_ms was {ms}");
    }

    #[test]
    fn a_mark_made_before_the_session_started_clamps_to_zero() {
        let started = Utc::now() + chrono::Duration::seconds(5);
        let mark = MarkRecord::new("x").in_session("s1", started);
        assert_eq!(mark.elapsed_ms, Some(0));
    }

    #[test]
    fn append_line_writes_one_record_per_call() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("out.jsonl");
        append_line(&path, "one").unwrap();
        append_line(&path, "two").unwrap();
        let body = std::fs::read_to_string(&path).unwrap();
        assert_eq!(body, "one\ntwo\n");
    }

    #[test]
    fn timeline_and_pending_are_separate_files() {
        let dir = tempfile::tempdir().unwrap();
        let mark = MarkRecord::new("ready");
        append_to_timeline(dir.path(), &mark).unwrap();
        notify_pending(dir.path(), &mark).unwrap();

        let timeline = std::fs::read_to_string(dir.path().join("timeline.jsonl")).unwrap();
        let pending = std::fs::read_to_string(dir.path().join(PENDING_FILE)).unwrap();
        assert!(timeline.contains(r#""kind":"mark""#));
        assert_eq!(timeline.lines().count(), 1);
        assert_eq!(pending.lines().count(), 1);
    }
}
