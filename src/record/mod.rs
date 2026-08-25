//! The continuous A/V recording runtime behind `framewatch record`.
//!
//! Unlike the change-triggered [`Engine`](crate::Engine), which drops frames,
//! this path records *every* paced frame of one window to an `ffmpeg`-encoded
//! mp4 while capturing the microphone, then hands the finished media to a
//! [`PackageWriter`](crate::recording::PackageWriter). It uses WGC on Windows,
//! ScreenCaptureKit on macOS, and X11 on Linux when the corresponding
//! live-capture feature is enabled.

use crate::config::Target;
use crate::error::RecordError;
use crate::frame::Rect;
use chrono::{DateTime, Utc};
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

#[cfg(any(
    all(windows, feature = "wgc"),
    all(target_os = "macos", feature = "macos"),
    all(target_os = "linux", feature = "linux-x11")
))]
mod audio;
#[cfg(any(
    all(windows, feature = "wgc"),
    all(target_os = "macos", feature = "macos"),
    all(target_os = "linux", feature = "linux-x11")
))]
mod ffmpeg;
#[cfg(any(
    all(windows, feature = "wgc"),
    all(target_os = "macos", feature = "macos"),
    all(target_os = "linux", feature = "linux-x11")
))]
mod video;

/// Inputs to a recording run.
#[derive(Debug, Clone)]
pub struct RecordConfig {
    /// Which window to record.
    pub target: Target,
    /// Optional pixel crop applied to every frame (e.g. to clip host chrome).
    pub crop: Option<Rect>,
    /// Target video frames per second (clamped to `1..=60`).
    pub fps: u32,
    /// Microphone device name (substring match), or `None` for the default input.
    pub mic: Option<String>,
    /// Whether to capture the microphone. If `false` (or no input device is
    /// available) the recording is video-only.
    pub capture_audio: bool,
    /// Where the final muxed `recording.mp4` is written.
    pub video_out: PathBuf,
    /// Where the microphone `audio.wav` is written.
    pub audio_out: PathBuf,
    /// Scratch directory for the intermediate (pre-mux) video.
    pub work_dir: PathBuf,
    /// Wait up to this many ms for the target window to appear.
    pub wait_ms: u64,
    /// Shared stop flag: set by Ctrl+C or the `--duration` watchdog.
    pub stop: Arc<AtomicBool>,
}

/// What a finished recording reports back to the caller (used to build the
/// package manifest).
#[derive(Debug, Clone)]
pub struct RecordOutcome {
    /// Encoded video width.
    pub width: u32,
    /// Encoded video height.
    pub height: u32,
    /// Encoded frames per second.
    pub fps: f32,
    /// Number of frames written to the encoder.
    pub frames_written: u64,
    /// Video duration in ms.
    pub video_duration_ms: u64,
    /// Audio details, or `None` for a video-only recording (no microphone).
    pub audio: Option<AudioInfo>,
    /// Video codec (`"h264"`).
    pub codec: String,
    /// Container (`"mp4"`).
    pub container: String,
    /// Resolved window title.
    pub window_title: String,
    /// Resolved window executable basename.
    pub window_exe: String,
    /// When the recording finished.
    pub ended_at: DateTime<Utc>,
}

/// Audio details from a finished recording.
#[derive(Debug, Clone)]
pub struct AudioInfo {
    /// WAV sample rate (device native).
    pub sample_rate: u32,
    /// WAV channel count (mono).
    pub channels: u16,
    /// Audio duration in ms.
    pub duration_ms: u64,
}

/// How long to wait for the first frame after the window is resolved, in
/// addition to `RecordConfig::wait_ms`.
#[cfg(any(
    all(windows, feature = "wgc"),
    all(target_os = "macos", feature = "macos"),
    all(target_os = "linux", feature = "linux-x11")
))]
const FIRST_FRAME_WAIT_MS: u64 = 10_000;

/// Record `cfg.target` to a video + microphone WAV until `cfg.stop` is set,
/// muxing them into `cfg.video_out`. Returns metadata for the package manifest.
#[cfg(any(
    all(windows, feature = "wgc"),
    all(target_os = "macos", feature = "macos"),
    all(target_os = "linux", feature = "linux-x11")
))]
pub fn record(cfg: RecordConfig) -> Result<RecordOutcome, RecordError> {
    record_inner(cfg, None)
}

/// Record for a bounded duration measured from encoder startup (after the target
/// window has resolved and its first frame has arrived).
#[cfg(any(
    all(windows, feature = "wgc"),
    all(target_os = "macos", feature = "macos"),
    all(target_os = "linux", feature = "linux-x11")
))]
pub fn record_with_duration(
    cfg: RecordConfig,
    duration: std::time::Duration,
) -> Result<RecordOutcome, RecordError> {
    record_inner(cfg, Some(duration))
}

#[cfg(any(
    all(windows, feature = "wgc"),
    all(target_os = "macos", feature = "macos"),
    all(target_os = "linux", feature = "linux-x11")
))]
fn record_inner(
    cfg: RecordConfig,
    duration: Option<std::time::Duration>,
) -> Result<RecordOutcome, RecordError> {
    use std::sync::atomic::Ordering;
    use std::sync::{Condvar, Mutex};
    use std::time::{Duration, Instant};

    // Fail fast before touching the mic / capture so we never leave partial files.
    if !ffmpeg::ffmpeg_available() {
        return Err(RecordError::FfmpegNotFound);
    }
    let fps = cfg.fps.clamp(1, 60);

    // Resolve the window first (it may not have launched yet).
    let resolved = video::resolve_backend(&cfg.target, cfg.wait_ms)?;
    let window = resolved.window;
    let capture_stop = resolved
        .backend
        .stop_signal()
        .expect("live recording backends expose a stop signal");

    // Start the microphone. A missing/unusable input device is non-fatal — we
    // fall back to a video-only recording rather than losing the capture.
    let mut audio = if cfg.capture_audio {
        match audio::AudioRecorder::start(cfg.mic.as_deref(), &cfg.audio_out) {
            Ok(a) => Some(a),
            Err(e) => {
                tracing::warn!("framewatch: microphone unavailable ({e}); recording video only");
                None
            }
        }
    } else {
        None
    };

    // Capture thread: publish conformed frames into the mailbox.
    let mailbox: video::FrameMailbox = Arc::new(Mutex::new(None));
    let dims: video::DimsCell = Arc::new((Mutex::new(None), Condvar::new()));
    let v0: Arc<Mutex<Option<Instant>>> = Arc::new(Mutex::new(None));
    let capture = {
        let (mailbox, dims, v0, stop) =
            (mailbox.clone(), dims.clone(), v0.clone(), cfg.stop.clone());
        let crop = cfg.crop;
        std::thread::spawn(move || {
            video::run_capture(resolved.backend, crop, mailbox, dims, v0, stop)
        })
    };

    // Wait for the first frame to lock the recording dimensions.
    let locked = video::wait_for_dims(&dims, &cfg.stop, cfg.wait_ms + FIRST_FRAME_WAIT_MS);
    let (width, height) = match locked {
        Some(d) => d,
        None => {
            cfg.stop.store(true, Ordering::Relaxed);
            capture_stop.store(true, Ordering::Relaxed);
            let capture_result = join_capture(capture);
            if let Some(a) = audio.take() {
                let _ = a.finish();
            }
            capture_result?;
            return Err(RecordError::Capture(crate::error::CaptureError::Backend(
                "the target window produced no frames to record (is it visible and rendering?)"
                    .into(),
            )));
        }
    };

    // Spawn the encoder and pace frames to it at a constant rate until stop.
    let temp_video = cfg.work_dir.join(".framewatch-video.tmp.mp4");
    let mut encoder = match ffmpeg::VideoEncoder::spawn(width, height, fps, &temp_video) {
        Ok(encoder) => encoder,
        Err(error) => {
            cfg.stop.store(true, Ordering::Relaxed);
            capture_stop.store(true, Ordering::Relaxed);
            let _ = join_capture(capture);
            if let Some(a) = audio.take() {
                let _ = a.finish();
            }
            let _ = std::fs::remove_file(&temp_video);
            return Err(error);
        }
    };
    let pacing_start = Instant::now();
    let stop_at = duration.map(|duration| pacing_start + duration);
    let interval_ns = 1_000_000_000u64 / fps as u64;
    let mut k: u64 = 0;
    let mut frames_written: u64 = 0;
    while !cfg.stop.load(Ordering::Relaxed) {
        if stop_at.is_some_and(|deadline| Instant::now() >= deadline) {
            break;
        }
        let mut deadline = pacing_start + Duration::from_nanos(k.saturating_mul(interval_ns));
        if let Some(stop_at) = stop_at {
            deadline = deadline.min(stop_at);
        }
        let now = Instant::now();
        if now < deadline {
            std::thread::sleep(deadline - now);
        }
        if cfg.stop.load(Ordering::Relaxed)
            || stop_at.is_some_and(|deadline| Instant::now() >= deadline)
        {
            break;
        }
        let frame = mailbox.lock().unwrap().clone();
        if let Some(buf) = frame {
            // Re-writing the latest frame on a gap keeps the stream constant-rate.
            if encoder.write_frame(&buf).is_err() {
                break; // ffmpeg exited unexpectedly
            }
            frames_written += 1;
        }
        k += 1;
    }

    // Ordered finalize: stop capture, flush+close the encoder (so the mp4 gets
    // its moov atom), then finalize the WAV.
    cfg.stop.store(true, Ordering::Relaxed);
    capture_stop.store(true, Ordering::Relaxed);
    let encoder_result = encoder.finish();
    let capture_result = join_capture(capture);
    let audio_result = match audio.take() {
        Some(audio) => audio.finish().map(Some),
        None => Ok(None),
    };

    if let Err(error) = encoder_result {
        let _ = std::fs::remove_file(&temp_video);
        return Err(error);
    }
    if let Err(error) = capture_result {
        let _ = std::fs::remove_file(&temp_video);
        return Err(error);
    }
    let audio_stats = match audio_result {
        Ok(stats) => stats,
        Err(error) => {
            let _ = std::fs::remove_file(&temp_video);
            return Err(error);
        }
    };

    // With audio: finalize the WAV, align it to the video start, and mux.
    // Without: the encoded video is already the final output.
    let audio_info = match audio_stats {
        Some(stats) => {
            let v0_inst = *v0.lock().unwrap();
            let audio_offset_s = match (v0_inst, stats.first_sample_at) {
                (Some(v), Some(a)) if a >= v => (a - v).as_secs_f64(),
                (Some(v), Some(a)) => -(v - a).as_secs_f64(),
                _ => 0.0,
            };
            // Remove the intermediate video on every path (success or mux
            // failure) so a failed mux doesn't leave a stray temp file.
            let mux = ffmpeg::run_mux(&cfg.audio_out, &temp_video, audio_offset_s, &cfg.video_out);
            let _ = std::fs::remove_file(&temp_video);
            mux?;
            Some(AudioInfo {
                sample_rate: stats.sample_rate,
                channels: stats.channels,
                duration_ms: stats.duration_ms,
            })
        }
        None => {
            // Move the encoded video to the final path (fall back to copy across
            // volumes); no mux needed.
            std::fs::rename(&temp_video, &cfg.video_out).or_else(|_| {
                std::fs::copy(&temp_video, &cfg.video_out)
                    .and_then(|_| std::fs::remove_file(&temp_video))
            })?;
            None
        }
    };

    Ok(RecordOutcome {
        width,
        height,
        fps: fps as f32,
        frames_written,
        video_duration_ms: frames_written * 1000 / fps as u64,
        audio: audio_info,
        codec: "h264".into(),
        container: "mp4".into(),
        window_title: window.title,
        window_exe: window.exe,
        ended_at: Utc::now(),
    })
}

#[cfg(any(
    all(windows, feature = "wgc"),
    all(target_os = "macos", feature = "macos"),
    all(target_os = "linux", feature = "linux-x11")
))]
fn join_capture(
    capture: std::thread::JoinHandle<Result<(), crate::error::CaptureError>>,
) -> Result<(), RecordError> {
    capture.join().map_err(|_| {
        RecordError::Capture(crate::error::CaptureError::Backend(
            "capture thread panicked".into(),
        ))
    })??;
    Ok(())
}

/// Recording needs a native live-capture backend.
#[cfg(not(any(
    all(windows, feature = "wgc"),
    all(target_os = "macos", feature = "macos"),
    all(target_os = "linux", feature = "linux-x11")
)))]
pub fn record(_cfg: RecordConfig) -> Result<RecordOutcome, RecordError> {
    Err(RecordError::Unsupported)
}

/// Recording needs a native live-capture backend.
#[cfg(not(any(
    all(windows, feature = "wgc"),
    all(target_os = "macos", feature = "macos"),
    all(target_os = "linux", feature = "linux-x11")
)))]
pub fn record_with_duration(
    _cfg: RecordConfig,
    _duration: std::time::Duration,
) -> Result<RecordOutcome, RecordError> {
    Err(RecordError::Unsupported)
}
