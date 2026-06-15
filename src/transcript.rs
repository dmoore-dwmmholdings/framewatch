//! Voice-narration transcript types, SubRip (`.srt`) formatting, and the
//! [`Transcriber`] that turns a recorded `audio.wav` into a [`Transcript`].
//!
//! These types are the timestamped-text half of a `record` package. Each
//! [`TranscriptSegment`] carries `start_ms`/`end_ms` measured from the start of
//! the recording — the same clock as the video — so a consumer can map a spoken
//! instruction to the exact moment it refers to in `recording.mp4`.
//!
//! Two engines are supported (see [`Transcriber`]): bundled whisper.cpp (the
//! `whisper` feature) and an external `--transcribe-cmd`. The external path and
//! all of the formatting/parsing here are pure and cross-platform, so they are
//! exercised on every CI target without building whisper.cpp.

use crate::error::TranscribeError;
use crate::util::tokenize;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

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

    /// Build a transcript from `(t0, t1, text)` tuples whose timestamps are in
    /// **centiseconds** (whisper.cpp's unit), converting to ms (`* 10`).
    ///
    /// Negative timestamps are clamped to `0`, blank segments are dropped, and
    /// `duration_ms` is set to the maximum `end_ms`.
    pub fn from_centisecond_segments(
        items: impl IntoIterator<Item = (i64, i64, String)>,
        language: Option<String>,
    ) -> Self {
        let cs_to_ms = |cs: i64| -> u64 { (cs.max(0) as u64).saturating_mul(10) };
        let mut segments = Vec::new();
        for (t0, t1, text) in items {
            let text = text.trim().to_string();
            if text.is_empty() {
                continue;
            }
            segments.push(TranscriptSegment {
                start_ms: cs_to_ms(t0),
                end_ms: cs_to_ms(t1),
                text,
            });
        }
        let duration_ms = segments.iter().map(|s| s.end_ms).max().unwrap_or(0);
        Self {
            language,
            duration_ms,
            segments,
        }
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
#[derive(Debug, Clone)]
pub enum Transcriber {
    /// Bundled whisper.cpp via the `whisper-rs` crate (requires the `whisper`
    /// feature). The model file is user-supplied — never auto-downloaded.
    Whisper {
        /// Path to a GGML/GGUF whisper model (`.bin`).
        model_path: PathBuf,
        /// Force a language (e.g. `"en"`), or `None` to auto-detect.
        language: Option<String>,
        /// Worker threads (`0` selects a sensible default).
        n_threads: u32,
    },
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
    /// Produce an empty transcript (`--no-transcribe`).
    Disabled,
}

impl Transcriber {
    /// A `(engine, model)` label pair recorded in the package manifest.
    pub fn engine_meta(&self) -> (&'static str, Option<String>) {
        match self {
            Transcriber::Whisper { model_path, .. } => (
                "whisper.cpp",
                model_path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned()),
            ),
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
            Transcriber::Whisper {
                model_path,
                language,
                n_threads,
            } => transcribe_whisper(model_path, language.as_deref(), *n_threads, audio_wav),
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
    let output_base_str = output_base.to_string_lossy().into_owned();

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

    let program = argv.remove(0);
    let out = std::process::Command::new(&program).args(&argv).output()?;
    if !out.status.success() {
        let code = out.status.code().unwrap_or(-1);
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(TranscribeError::CommandFailed(code, stderr));
    }

    // Prefer a written {output} file (extension tells us the format); else stdout.
    if used_output {
        for (ext, fmt) in [("json", Blob::Json), ("srt", Blob::Srt)] {
            let cand = PathBuf::from(format!("{output_base_str}.{ext}"));
            if let Ok(raw) = std::fs::read_to_string(&cand) {
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

#[cfg(feature = "whisper")]
fn transcribe_whisper(
    model_path: &Path,
    language: Option<&str>,
    n_threads: u32,
    audio_wav: &Path,
) -> Result<Transcript, TranscribeError> {
    use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

    // 1. Read the i16 WAV the recorder wrote.
    let mut reader =
        hound::WavReader::open(audio_wav).map_err(|e| TranscribeError::Audio(e.to_string()))?;
    let spec = reader.spec();
    let samples_i16: Vec<i16> = reader
        .samples::<i16>()
        .collect::<Result<_, _>>()
        .map_err(|e| TranscribeError::Audio(e.to_string()))?;

    // 2. i16 -> f32, downmix to mono, resample to 16 kHz (pure helpers).
    let samples_f32: Vec<f32> = samples_i16.iter().map(|&s| s as f32 / 32768.0).collect();
    let mono16k = crate::audioutil::to_mono_16k(&samples_f32, spec.channels, spec.sample_rate);

    // 3. Run whisper. `new_with_params` takes `impl AsRef<Path>`.
    let ctx = WhisperContext::new_with_params(model_path, WhisperContextParameters::default())
        .map_err(|e| TranscribeError::Whisper(e.to_string()))?;
    let mut state = ctx
        .create_state()
        .map_err(|e| TranscribeError::Whisper(e.to_string()))?;

    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
    if let Some(lang) = language {
        params.set_language(Some(lang));
    }
    if n_threads > 0 {
        params.set_n_threads(n_threads as i32);
    }
    params.set_translate(false);
    params.set_print_special(false);
    params.set_print_progress(false);
    params.set_print_realtime(false);
    params.set_print_timestamps(false);

    state
        .full(params, &mono16k)
        .map_err(|e| TranscribeError::Whisper(e.to_string()))?;

    // 4. Read back segments. whisper timestamps are centiseconds; the pure
    //    `from_centisecond_segments` does the `* 10` conversion.
    let n = state.full_n_segments();
    let mut raw = Vec::new();
    for i in 0..n {
        if let Some(seg) = state.get_segment(i) {
            let text = seg
                .to_str_lossy()
                .map_err(|e| TranscribeError::Whisper(e.to_string()))?
                .into_owned();
            raw.push((seg.start_timestamp(), seg.end_timestamp(), text));
        }
    }
    Ok(Transcript::from_centisecond_segments(
        raw,
        language.map(str::to_string),
    ))
}

#[cfg(not(feature = "whisper"))]
fn transcribe_whisper(
    _model_path: &Path,
    _language: Option<&str>,
    _n_threads: u32,
    _audio_wav: &Path,
) -> Result<Transcript, TranscribeError> {
    Err(TranscribeError::Whisper(
        "this build has no bundled whisper engine — rebuild with `--features whisper`, \
         or use --transcribe-cmd"
            .into(),
    ))
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
    fn centisecond_conversion() {
        let t = Transcript::from_centisecond_segments(
            [(125, 480, "hi".to_string()), (0, 10, "  ".to_string())],
            Some("en".into()),
        );
        // blank segment dropped; centiseconds * 10 = ms.
        assert_eq!(t.segments.len(), 1);
        assert_eq!(t.segments[0].start_ms, 1250);
        assert_eq!(t.segments[0].end_ms, 4800);
        assert_eq!(t.duration_ms, 4800);
        assert_eq!(t.language.as_deref(), Some("en"));
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
}
