//! The `record` output package: the [`Recording`] paths, the
//! [`RecordingManifest`], and the [`PackageWriter`] that lays a finished
//! recording out on disk for an LLM to consume.
//!
//! A package is a single directory (a sibling of a capture [`Session`]) holding
//! the screen recording, the microphone audio, the timestamped transcript (as
//! both JSON and SRT), a manifest, a generated `PROMPT.md`, and a
//! `README_FOR_AGENT.md`. Each transcript segment's `start_ms`/`end_ms` is
//! measured from the start of `recording.mp4`, so a model can correlate spoken
//! instructions with on-screen actions, either ingesting the video directly or
//! extracting a frame at a timestamp with `ffmpeg -ss`.
//!
//! [`Session`]: crate::session::Session

use crate::config::Target;
use crate::error::SinkError;
use crate::session::{make_session_id, ManifestTarget};
use crate::transcript::{format_srt_timestamp, Transcript};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// The package filenames, kept in one place so the manifest, writer, and docs
/// agree.
pub mod files {
    /// The muxed screen recording (video + narration).
    pub const VIDEO: &str = "recording.mp4";
    /// The raw microphone narration.
    pub const AUDIO: &str = "audio.wav";
    /// The machine-readable transcript.
    pub const TRANSCRIPT_JSON: &str = "transcript.json";
    /// The transcript as SubRip subtitles.
    pub const TRANSCRIPT_SRT: &str = "transcript.srt";
    /// The package manifest.
    pub const MANIFEST: &str = "recording.json";
    /// The generated LLM prompt.
    pub const PROMPT: &str = "PROMPT.md";
    /// The agent-facing consumption guide.
    pub const README: &str = "README_FOR_AGENT.md";
}

/// Resolved on-disk locations for a recording package.
#[derive(Debug, Clone)]
pub struct Recording {
    /// Package identifier (same shape as a [`Session`](crate::session::Session) id).
    pub id: String,
    /// The package directory (`<out_dir>/<id>`).
    pub dir: PathBuf,
    /// When the recording started.
    pub started_at: DateTime<Utc>,
}

impl Recording {
    /// Create a recording rooted under `out_dir`, deriving the id from
    /// `started_at` and `exe_hint` (reusing the session id format).
    pub fn new(out_dir: &Path, started_at: DateTime<Utc>, exe_hint: &str) -> Self {
        let id = make_session_id(started_at, exe_hint);
        let dir = out_dir.join(&id);
        Self {
            id,
            dir,
            started_at,
        }
    }

    /// Path to the muxed `recording.mp4`.
    pub fn video_path(&self) -> PathBuf {
        self.dir.join(files::VIDEO)
    }
    /// Path to the `audio.wav` narration.
    pub fn audio_path(&self) -> PathBuf {
        self.dir.join(files::AUDIO)
    }
    /// Path to `transcript.json`.
    pub fn transcript_json_path(&self) -> PathBuf {
        self.dir.join(files::TRANSCRIPT_JSON)
    }
    /// Path to `transcript.srt`.
    pub fn transcript_srt_path(&self) -> PathBuf {
        self.dir.join(files::TRANSCRIPT_SRT)
    }
    /// Path to the `recording.json` manifest.
    pub fn manifest_path(&self) -> PathBuf {
        self.dir.join(files::MANIFEST)
    }
    /// Path to the generated `PROMPT.md`.
    pub fn prompt_path(&self) -> PathBuf {
        self.dir.join(files::PROMPT)
    }
    /// Path to the `README_FOR_AGENT.md`.
    pub fn readme_path(&self) -> PathBuf {
        self.dir.join(files::README)
    }
}

/// Video stream metadata recorded in the manifest.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VideoMeta {
    /// Relative path to the video (`recording.mp4`).
    pub path: String,
    /// Container, e.g. `"mp4"`.
    pub container: String,
    /// Video codec, e.g. `"h264"`.
    pub codec: String,
    /// Frames per second the video was encoded at.
    pub fps: f32,
    /// Encoded width in pixels.
    pub width: u32,
    /// Encoded height in pixels.
    pub height: u32,
    /// Video duration in ms.
    pub duration_ms: u64,
}

/// Audio stream metadata recorded in the manifest.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AudioMeta {
    /// Relative path to the audio (`audio.wav`).
    pub path: String,
    /// Sample rate in Hz.
    pub sample_rate: u32,
    /// Channel count.
    pub channels: u16,
    /// Audio duration in ms.
    pub duration_ms: u64,
}

/// Transcript metadata recorded in the manifest.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TranscriptMeta {
    /// Relative path to `transcript.json`.
    pub path: String,
    /// Relative path to `transcript.srt`.
    pub srt: String,
    /// Engine used: `"whisper.cpp"`, `"command"`, or `"none"`.
    pub engine: String,
    /// Model file / command template, if applicable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Number of segments.
    pub segment_count: usize,
    /// Detected/forced language code, if known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
}

/// The `recording.json` manifest — the recording-package analogue of
/// [`SessionManifest`](crate::session::SessionManifest).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordingManifest {
    /// Package id.
    pub session_id: String,
    /// Tool name + version.
    pub tool: String,
    /// Always `"recording"` — distinguishes this from a capture `session.json`.
    pub kind: String,
    /// Target descriptor.
    pub target: ManifestTarget,
    /// Start timestamp.
    pub started_at: DateTime<Utc>,
    /// End timestamp.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<DateTime<Utc>>,
    /// Video metadata.
    pub video: VideoMeta,
    /// Audio metadata (absent for a video-only recording — e.g. no microphone).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audio: Option<AudioMeta>,
    /// Transcript metadata.
    pub transcript: TranscriptMeta,
    /// Relative paths of all artifacts in the package.
    pub artifacts: Vec<String>,
}

impl RecordingManifest {
    /// Assemble a manifest. `transcript` supplies the per-engine fields via
    /// `engine` / `model` and its own segment count / language.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        recording: &Recording,
        target: &Target,
        selected_via: &str,
        video: VideoMeta,
        audio: Option<AudioMeta>,
        transcript: &Transcript,
        engine: &str,
        model: Option<String>,
        ended_at: DateTime<Utc>,
    ) -> Self {
        let transcript_meta = TranscriptMeta {
            path: files::TRANSCRIPT_JSON.to_string(),
            srt: files::TRANSCRIPT_SRT.to_string(),
            engine: engine.to_string(),
            model,
            segment_count: transcript.segments.len(),
            language: transcript.language.clone(),
        };
        let mut artifacts = vec![files::VIDEO.to_string()];
        if audio.is_some() {
            artifacts.push(files::AUDIO.to_string());
        }
        artifacts.extend([
            files::TRANSCRIPT_JSON.to_string(),
            files::TRANSCRIPT_SRT.to_string(),
            files::MANIFEST.to_string(),
            files::PROMPT.to_string(),
            files::README.to_string(),
        ]);
        Self {
            session_id: recording.id.clone(),
            tool: format!("framewatch {}", env!("CARGO_PKG_VERSION")),
            kind: "recording".to_string(),
            target: ManifestTarget::from_target(target, selected_via),
            started_at: recording.started_at,
            ended_at: Some(ended_at),
            video,
            audio,
            transcript: transcript_meta,
            artifacts,
        }
    }
}

const README_FOR_AGENT: &str = r#"# framewatch recording package

This directory is a single screen recording of one application window with a
synchronized voice narration and its transcript. A human recorded their screen
while speaking instructions; your job is to follow those instructions, using the
video to see exactly what they pointed at.

Files:
1. `PROMPT.md`        — START HERE. The task prompt with the full transcript inline.
2. `recording.mp4`    — the screen recording (the narration is also muxed in).
3. `audio.wav`        — the raw microphone narration (PCM).
4. `transcript.json`  — machine-readable transcript: segments with `start_ms`/`end_ms`/`text`.
5. `transcript.srt`   — the same transcript as SubRip subtitles (HH:MM:SS,mmm).
6. `recording.json`   — manifest: target window, time range, video/audio/transcript meta.

How to consume:
- A text-only model can work entirely from `PROMPT.md` — the transcript is inline.
- A multimodal model SHOULD also look at the video. Each transcript segment's
  `start_ms`/`end_ms` is measured from the start of `recording.mp4`, so when the
  narration says "click *this*", seek the video to that timestamp to see what
  "this" was. Extract a frame at a timestamp with ffmpeg (`-ss` is in seconds):

      ffmpeg -ss 12.500 -i recording.mp4 -frames:v 1 frame.png

  (start_ms 12500 -> -ss 12.500)
"#;

/// Writes a finished recording's metadata, transcript, prompt, and readme into
/// the package directory. The media (`recording.mp4` / `audio.wav`) is written
/// by the recording runtime directly into [`Recording::dir`].
pub struct PackageWriter {
    recording: Recording,
}

impl PackageWriter {
    /// Create the package directory under `out_dir` and return a writer for it.
    pub fn new(
        out_dir: &Path,
        started_at: DateTime<Utc>,
        exe_hint: &str,
    ) -> Result<Self, SinkError> {
        let recording = Recording::new(out_dir, started_at, exe_hint);
        std::fs::create_dir_all(&recording.dir)?;
        Ok(Self { recording })
    }

    /// The recording (paths) this writer targets.
    pub fn recording(&self) -> &Recording {
        &self.recording
    }

    /// Write `transcript.json` and `transcript.srt`.
    pub fn write_transcript(&self, transcript: &Transcript) -> Result<(), SinkError> {
        let json = serde_json::to_string_pretty(transcript)?;
        std::fs::write(self.recording.transcript_json_path(), json)?;
        std::fs::write(self.recording.transcript_srt_path(), transcript.to_srt())?;
        Ok(())
    }

    /// Write `recording.json`, `PROMPT.md`, and `README_FOR_AGENT.md`. Call
    /// after [`write_transcript`](PackageWriter::write_transcript) and after the
    /// media has been placed in the directory.
    pub fn finalize(
        &self,
        manifest: &RecordingManifest,
        transcript: &Transcript,
    ) -> Result<(), SinkError> {
        let json = serde_json::to_string_pretty(manifest)?;
        std::fs::write(self.recording.manifest_path(), json)?;
        std::fs::write(self.recording.readme_path(), README_FOR_AGENT)?;
        std::fs::write(
            self.recording.prompt_path(),
            render_prompt(manifest, transcript),
        )?;
        Ok(())
    }
}

/// Render the human/LLM-facing `PROMPT.md` from the manifest + transcript.
pub fn render_prompt(manifest: &RecordingManifest, transcript: &Transcript) -> String {
    let dur = human_duration(manifest.video.duration_ms);
    let title = manifest
        .target
        .title
        .clone()
        .or_else(|| manifest.target.exe.clone())
        .unwrap_or_else(|| "the target window".to_string());
    let exe = manifest.target.exe.clone().unwrap_or_default();
    let window_label = if exe.is_empty() {
        format!("\"{title}\"")
    } else {
        format!("\"{title}\" ({exe})")
    };

    let mut s = String::new();
    s.push_str("# Task from a screen recording\n\n");
    s.push_str(&format!(
        "A human recorded their screen for {dur} while narrating instructions out loud. \
The recording is in this package. Read the narration below, then carry out what they \
asked. The video lets you see exactly what they were pointing at or referring to.\n\n"
    ));

    let audio_note = if manifest.audio.is_some() {
        " The narration audio is muxed into the video and also available standalone as `audio.wav`."
    } else {
        " (This recording has no audio track.)"
    };
    s.push_str("## What you have\n");
    s.push_str(&format!(
        "- `recording.mp4` — the screen capture of {window_label}, {}x{} at {} fps, {dur} long.{audio_note}\n",
        manifest.video.width,
        manifest.video.height,
        fmt_fps(manifest.video.fps),
    ));
    s.push_str(
        "- The full narration transcript is inline below. Every line is timestamped in \
mm:ss,mmm from the start of the video, so each spoken instruction maps to a specific \
moment on screen.\n\n",
    );

    s.push_str("## How to use the video\n");
    s.push_str(
        "- If you can ingest video directly, watch `recording.mp4` and follow along with the \
timestamps below.\n",
    );
    s.push_str(
        "- Otherwise, pull a still frame at any timestamp with ffmpeg. `start_ms` is in \
milliseconds; ffmpeg `-ss` takes seconds, so divide by 1000:\n\n",
    );
    s.push_str("      ffmpeg -ss <seconds> -i recording.mp4 -frames:v 1 frame.png\n\n");
    s.push_str(
        "  Example — to see what was on screen when the narrator spoke at start_ms 12500:\n\n",
    );
    s.push_str("      ffmpeg -ss 12.500 -i recording.mp4 -frames:v 1 frame.png\n\n");
    s.push_str(
        "- Correlate words with actions: when the narration says \"open this menu\" at a given \
timestamp, extract the frame at that timestamp to see which menu.\n\n",
    );

    s.push_str("## Narration transcript (timestamps are mm:ss,mmm from video start)\n");
    if transcript.segments.is_empty() {
        s.push_str("_(no narration transcript — rely on the video.)_\n\n");
    } else {
        for seg in &transcript.segments {
            s.push_str(&format!(
                "- [{} → {}] {}\n",
                format_srt_timestamp(seg.start_ms),
                format_srt_timestamp(seg.end_ms),
                seg.text,
            ));
        }
        s.push('\n');
    }

    s.push_str("## Your task\n");
    s.push_str(
        "Follow the narrated instructions above in order. Where an instruction is visual \
(\"this\", \"here\", \"that button\"), use the timestamp to locate the on-screen target in \
the video before acting.\n",
    );
    s
}

/// Format a duration in ms as a short human string, e.g. `"1m 23s"` or `"8.5s"`.
fn human_duration(ms: u64) -> String {
    let secs = ms as f64 / 1000.0;
    if secs < 60.0 {
        format!("{secs:.1}s")
    } else {
        let total = (secs.round()) as u64;
        format!("{}m {}s", total / 60, total % 60)
    }
}

/// Format an fps value without a trailing `.0` for whole numbers.
fn fmt_fps(fps: f32) -> String {
    if (fps.fract()).abs() < f32::EPSILON {
        format!("{}", fps as i64)
    } else {
        format!("{fps:.2}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transcript::TranscriptSegment;

    fn sample_manifest(recording: &Recording, transcript: &Transcript) -> RecordingManifest {
        RecordingManifest::new(
            recording,
            &Target::ByTitleRegex("My Game".into()),
            "cli",
            VideoMeta {
                path: files::VIDEO.into(),
                container: "mp4".into(),
                codec: "h264".into(),
                fps: 30.0,
                width: 1920,
                height: 1080,
                duration_ms: 83_000,
            },
            Some(AudioMeta {
                path: files::AUDIO.into(),
                sample_rate: 48_000,
                channels: 1,
                duration_ms: 83_000,
            }),
            transcript,
            "command",
            Some("whisper-cli".into()),
            recording.started_at,
        )
    }

    #[test]
    fn human_duration_formats() {
        assert_eq!(human_duration(8_500), "8.5s");
        assert_eq!(human_duration(83_000), "1m 23s");
    }

    #[test]
    fn prompt_embeds_transcript_and_ffmpeg_recipe() {
        let started = DateTime::parse_from_rfc3339("2026-06-14T06:22:17Z")
            .unwrap()
            .with_timezone(&Utc);
        let rec = Recording::new(Path::new("/tmp"), started, "Game.exe");
        let transcript = Transcript {
            language: Some("en".into()),
            duration_ms: 4800,
            segments: vec![TranscriptSegment {
                start_ms: 1250,
                end_ms: 4800,
                text: "open the settings panel".into(),
            }],
        };
        let manifest = sample_manifest(&rec, &transcript);
        let prompt = render_prompt(&manifest, &transcript);
        assert!(prompt.contains("frames:v 1 frame.png"));
        assert!(prompt.contains("-ss 12.500"));
        assert!(prompt.contains("[00:00:01,250 → 00:00:04,800] open the settings panel"));
        assert!(prompt.contains("\"My Game\""));
    }

    #[test]
    fn empty_transcript_prompt_has_no_narration_line() {
        let started = Utc::now();
        let rec = Recording::new(Path::new("/tmp"), started, "Game.exe");
        let transcript = Transcript::default();
        let manifest = sample_manifest(&rec, &transcript);
        let prompt = render_prompt(&manifest, &transcript);
        assert!(prompt.contains("no narration transcript"));
    }

    #[test]
    fn manifest_roundtrips_and_records_engine() {
        let started = Utc::now();
        let rec = Recording::new(Path::new("/tmp"), started, "Game.exe");
        let transcript = Transcript::default();
        let manifest = sample_manifest(&rec, &transcript);
        assert_eq!(manifest.kind, "recording");
        assert_eq!(manifest.transcript.engine, "command");
        assert_eq!(manifest.artifacts.len(), 7);
        let json = serde_json::to_string(&manifest).unwrap();
        let back: RecordingManifest = serde_json::from_str(&json).unwrap();
        assert_eq!(back.session_id, manifest.session_id);
        assert_eq!(back.video.width, 1920);
    }
}
