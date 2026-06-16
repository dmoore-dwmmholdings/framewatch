//! Voice-narration transcript types, SubRip (`.srt`) formatting, and the
//! [`Transcriber`] that turns a recorded `audio.wav` into a [`Transcript`].
//!
//! These types are the timestamped-text half of a `record` package. Each
//! [`TranscriptSegment`] carries `start_ms`/`end_ms` measured from the start of
//! the recording — the same clock as the video — so a consumer can map a spoken
//! instruction to the exact moment it refers to in `recording.mp4`.
//!
//! framewatch bundles no speech-to-text engine: transcription is done by
//! shelling out to a local transcriber via `--transcribe-cmd` (see
//! [`Transcriber`]). Everything here — the types and the SRT/JSON
//! formatting/parsing — is pure and cross-platform, exercised on every CI target.

use crate::error::TranscribeError;
use crate::util::tokenize;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// One spoken span. `start_ms`/`end_ms` are milliseconds from the start of the
/// recording (the same clock as the video), so a consumer can seek the video to
/// the moment an instruction was spoken.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TranscriptSegment {
    /// Start of the span, in ms from recording start.
    pub start_ms: u64,
    /// End of the span, in ms from recording start.
    pub end_ms: u64,
    /// The spoken text.
    pub text: String,
}

/// A full voice-narration transcript, serialized to `transcript.json`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Transcript {
    /// Detected/forced language code (e.g. `"en"`), if known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    /// Total duration covered, in ms (the maximum segment `end_ms`).
    pub duration_ms: u64,
    /// Segments, in chronological order.
    pub segments: Vec<TranscriptSegment>,
}

impl Transcript {
    /// Whether the transcript has no segments.
    pub fn is_empty(&self) -> bool {
        self.segments.is_empty()
    }

    /// Render the transcript as SubRip (`.srt`) subtitles.
    pub fn to_srt(&self) -> String {
        let mut out = String::new();
        for (i, seg) in self.segments.iter().enumerate() {
            out.push_str(&(i + 1).to_string());
            out.push('\n');
            out.push_str(&format_srt_timestamp(seg.start_ms));
            out.push_str(" --> ");
            out.push_str(&format_srt_timestamp(seg.end_ms));
            out.push('\n');
            out.push_str(&seg.text);
            out.push('\n');
            out.push('\n');
        }
        out
    }

    /// Parse SubRip (`.srt`) text into a transcript — the inverse of
    /// [`to_srt`](Transcript::to_srt). Multi-line cue text is joined with
    /// spaces; `language` is left `None`.
    pub fn from_srt(srt: &str) -> Result<Self, TranscribeError> {
        let normalized = srt.replace("\r\n", "\n").replace('\r', "\n");
        let mut segments = Vec::new();
        for block in normalized.split("\n\n") {
            let mut lines = block.lines().filter(|l| !l.trim().is_empty());
            // The first line is an optional numeric index; the timing line is the
            // one containing "-->".
            let first = match lines.next() {
                Some(l) => l,
                None => continue,
            };
            let timing = if first.contains("-->") {
                first
            } else {
                match lines.next() {
                    Some(l) => l,
                    None => continue,
                }
            };
            let (start_ms, end_ms) = parse_srt_timing(timing).ok_or_else(|| {
                TranscribeError::Parse(format!("bad SRT timing line: {timing:?}"))
            })?;
            let text = lines.collect::<Vec<_>>().join(" ").trim().to_string();
            if text.is_empty() {
                continue;
            }
            segments.push(TranscriptSegment {
                start_ms,
                end_ms,
                text,
            });
        }
        let duration_ms = segments.iter().map(|s| s.end_ms).max().unwrap_or(0);
        Ok(Self {
            language: None,
            duration_ms,
            segments,
        })
    }
}

/// Format milliseconds as an SRT timestamp `HH:MM:SS,mmm` (comma separator, per
/// the SubRip spec).
pub fn format_srt_timestamp(ms: u64) -> String {
    let h = ms / 3_600_000;
    let m = (ms % 3_600_000) / 60_000;
    let s = (ms % 60_000) / 1_000;
    let milli = ms % 1_000;
    format!("{h:02}:{m:02}:{s:02},{milli:03}")
}

fn parse_srt_timing(line: &str) -> Option<(u64, u64)> {
    let (a, b) = line.split_once("-->")?;
    Some((
        parse_srt_timestamp(a.trim())?,
        parse_srt_timestamp(b.trim())?,
    ))
}

fn parse_srt_timestamp(s: &str) -> Option<u64> {
    // HH:MM:SS,mmm — also tolerate '.' as the millisecond separator.
    let (hms, millis) = s.trim().split_once([',', '.'])?;
    let mut parts = hms.split(':');
    let h: u64 = parts.next()?.trim().parse().ok()?;
    let m: u64 = parts.next()?.trim().parse().ok()?;
    let sec: u64 = parts.next()?.trim().parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    let ms: u64 = millis.trim().parse().ok()?;
    Some(((h * 60 + m) * 60 + sec) * 1000 + ms)
}

/// How a recording's voice narration is turned into a [`Transcript`].
///
/// framewatch does not bundle a speech-to-text engine; local transcription is
/// done by shelling out to one you have ([`Command`](Transcriber::Command)),
/// e.g. whisper.cpp's prebuilt `whisper-cli`.
#[derive(Debug, Clone)]
pub enum Transcriber {
    /// Shell out to an external transcriber, run over `audio.wav`.
    ///
    /// The template is tokenized (quotes group args); `{audio}` is replaced with
    /// the WAV path and `{output}` with a framewatch-chosen output base path. If
    /// neither placeholder appears, the audio path is appended as the final arg
    /// and framewatch reads the command's **stdout**. The command must emit a
    /// framewatch [`Transcript`] JSON (`{ "segments": [...] }`) or SubRip (SRT).
    Command {
        /// The command template.
        template: String,
    },
    /// Produce an empty transcript (`--no-transcribe`, or no audio).
    Disabled,
}

impl Transcriber {
    /// A `(engine, model)` label pair recorded in the package manifest.
    pub fn engine_meta(&self) -> (&'static str, Option<String>) {
        match self {
            Transcriber::Command { template } => ("command", Some(template.clone())),
            Transcriber::Disabled => ("none", None),
        }
    }

    /// Transcribe `audio_wav`, using `work_dir` for any scratch output.
    pub fn transcribe(
        &self,
        audio_wav: &Path,
        work_dir: &Path,
    ) -> Result<Transcript, TranscribeError> {
        match self {
            Transcriber::Disabled => Ok(Transcript::default()),
            Transcriber::Command { template } => transcribe_command(template, audio_wav, work_dir),
        }
    }
}

/// Substitute placeholders into a `--transcribe-cmd` template, run it, and parse
/// the resulting transcript (from the `{output}` file if written, else stdout).
fn transcribe_command(
    template: &str,
    audio_wav: &Path,
    work_dir: &Path,
) -> Result<Transcript, TranscribeError> {
    let audio = audio_wav.to_string_lossy().into_owned();
    // Output *base* path (no extension) so tools that append `.srt`/`.json`
    // (e.g. whisper-cli `-of`) land somewhere we can find.
    let output_base = work_dir.join("transcript_raw");
    let output_json = output_base.with_extension("json");
    let output_srt = output_base.with_extension("srt");
    let output_base_str = output_base.to_string_lossy().into_owned();

    // Catch malformed templates early: `tokenize` silently swallows a dangling
    // quote, which would otherwise fail later with a less actionable error.
    if template.chars().filter(|&c| c == '"').count() % 2 != 0 {
        return Err(TranscribeError::Parse(
            "unbalanced quotes in --transcribe-cmd".into(),
        ));
    }
    let tokens = tokenize(template);
    if tokens.is_empty() {
        return Err(TranscribeError::Parse("empty --transcribe-cmd".into()));
    }

    let mut used_audio = false;
    let mut used_output = false;
    let mut argv: Vec<String> = tokens
        .into_iter()
        .map(|t| {
            let mut s = t;
            if s.contains("{audio}") {
                s = s.replace("{audio}", &audio);
                used_audio = true;
            }
            if s.contains("{output}") {
                s = s.replace("{output}", &output_base_str);
                used_output = true;
            }
            s
        })
        .collect();
    if !used_audio {
        argv.push(audio);
    }

    // Clear any leftover {output} files first, so a transcriber that writes
    // nothing this run can't make us read a stale transcript from a reused dir.
    if used_output {
        for p in [&output_base, &output_json, &output_srt] {
            let _ = std::fs::remove_file(p);
        }
    }

    let program = argv.remove(0);
    let out = std::process::Command::new(&program).args(&argv).output()?;
    if !out.status.success() {
        let code = out.status.code().unwrap_or(-1);
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(TranscribeError::CommandFailed(code, stderr));
    }

    // Prefer a written {output} file (extension tells us the format); else stdout.
    if used_output {
        for (cand, fmt) in [(&output_json, Blob::Json), (&output_srt, Blob::Srt)] {
            if let Ok(raw) = std::fs::read_to_string(cand) {
                if !raw.trim().is_empty() {
                    return parse_transcript_text(&raw, Some(fmt));
                }
            }
        }
        if let Ok(raw) = std::fs::read_to_string(&output_base) {
            if !raw.trim().is_empty() {
                return parse_transcript_text(&raw, None);
            }
        }
    }

    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    if stdout.trim().is_empty() {
        return Err(TranscribeError::Parse(
            "transcriber produced no output (wrote no {output} file and empty stdout)".into(),
        ));
    }
    parse_transcript_text(&stdout, None)
}

/// Which transcript wire format a blob is.
enum Blob {
    Json,
    Srt,
}

/// Parse a transcript blob as JSON or SRT. With no `format` hint, JSON is
/// detected by a leading `{`, otherwise the blob is parsed as SRT.
fn parse_transcript_text(raw: &str, format: Option<Blob>) -> Result<Transcript, TranscribeError> {
    let is_json = match format {
        Some(Blob::Json) => true,
        Some(Blob::Srt) => false,
        None => raw.trim_start().starts_with('{'),
    };
    if is_json {
        Ok(serde_json::from_str(raw)?)
    } else {
        Transcript::from_srt(raw)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn srt_timestamp_formatting() {
        assert_eq!(format_srt_timestamp(0), "00:00:00,000");
        assert_eq!(format_srt_timestamp(1_250), "00:00:01,250");
        assert_eq!(format_srt_timestamp(61_000), "00:01:01,000");
        assert_eq!(format_srt_timestamp(3_661_001), "01:01:01,001");
    }

    #[test]
    fn json_roundtrip() {
        let t = Transcript {
            language: Some("en".into()),
            duration_ms: 4800,
            segments: vec![TranscriptSegment {
                start_ms: 1250,
                end_ms: 4800,
                text: "open the settings panel".into(),
            }],
        };
        let json = serde_json::to_string(&t).unwrap();
        let back: Transcript = serde_json::from_str(&json).unwrap();
        assert_eq!(t, back);
    }

    #[test]
    fn srt_roundtrip() {
        let t = Transcript {
            language: None,
            duration_ms: 8200,
            segments: vec![
                TranscriptSegment {
                    start_ms: 1250,
                    end_ms: 4800,
                    text: "first instruction".into(),
                },
                TranscriptSegment {
                    start_ms: 5000,
                    end_ms: 8200,
                    text: "second instruction".into(),
                },
            ],
        };
        let srt = t.to_srt();
        assert!(srt.starts_with("1\n00:00:01,250 --> 00:00:04,800\nfirst instruction\n\n"));
        let back = Transcript::from_srt(&srt).unwrap();
        assert_eq!(back.segments, t.segments);
        assert_eq!(back.duration_ms, t.duration_ms);
    }

    #[test]
    fn parse_detects_json_vs_srt() {
        let json = r#"{"duration_ms":10,"segments":[{"start_ms":0,"end_ms":10,"text":"x"}]}"#;
        assert_eq!(parse_transcript_text(json, None).unwrap().segments.len(), 1);
        let srt = "1\n00:00:00,000 --> 00:00:00,010\nx\n";
        assert_eq!(parse_transcript_text(srt, None).unwrap().segments.len(), 1);
    }

    #[test]
    fn disabled_is_empty() {
        let t = Transcriber::Disabled
            .transcribe(Path::new("nonexistent.wav"), Path::new("."))
            .unwrap();
        assert!(t.is_empty());
        assert_eq!(Transcriber::Disabled.engine_meta().0, "none");
    }

    #[test]
    fn command_rejects_unbalanced_quotes() {
        let t = Transcriber::Command {
            template: "prog \"unterminated".into(),
        };
        let err = t
            .transcribe(Path::new("audio.wav"), Path::new("."))
            .unwrap_err();
        assert!(matches!(err, TranscribeError::Parse(_)));
    }

    #[test]
    fn command_rejects_empty_template() {
        let err = Transcriber::Command {
            template: "   ".into(),
        }
        .transcribe(Path::new("a.wav"), Path::new("."))
        .unwrap_err();
        assert!(matches!(err, TranscribeError::Parse(_)));
    }

    #[test]
    fn from_srt_handles_index_crlf_dot_separator_and_multiline() {
        let srt = "1\r\n00:00:01,000 --> 00:00:02,000\r\nhello\r\nworld\r\n\r\n\
                   2\r\n00:00:03.500 --> 00:00:04,000\r\nbye\r\n";
        let t = Transcript::from_srt(srt).unwrap();
        assert_eq!(t.segments.len(), 2);
        assert_eq!(t.segments[0].text, "hello world"); // multi-line cue joined
        assert_eq!(t.segments[0].start_ms, 1000);
        assert_eq!(t.segments[1].start_ms, 3500); // '.' separator tolerated
        assert_eq!(t.duration_ms, 4000);
    }

    #[test]
    fn from_srt_empty_and_bad_timing() {
        assert!(Transcript::from_srt("").unwrap().is_empty());
        assert!(Transcript::from_srt("1\nnot a timing line\ntext\n").is_err());
        // hours component present.
        let t = Transcript::from_srt("1\n01:02:03,004 --> 01:02:04,000\nx\n").unwrap();
        assert_eq!(t.segments[0].start_ms, 3_723_004);
    }

    #[test]
    fn engine_meta_labels() {
        assert_eq!(Transcriber::Disabled.engine_meta(), ("none", None));
        let (eng, model) = Transcriber::Command {
            template: "whisper-cli -f {audio}".into(),
        }
        .engine_meta();
        assert_eq!(eng, "command");
        assert_eq!(model.as_deref(), Some("whisper-cli -f {audio}"));
    }
}
