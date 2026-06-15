//! The continuous A/V recording runtime behind `framewatch record`.
//!
//! Unlike the change-triggered [`Engine`](crate::Engine), which drops frames,
//! this path records *every* paced frame of one window to an `ffmpeg`-encoded
//! mp4 while capturing the microphone, then hands the finished media to a
//! [`PackageWriter`](crate::recording::PackageWriter). It is Windows-only (it
//! drives the WGC backend); on other platforms [`record`] returns
//! [`RecordError::Unsupported`].

use crate::config::Target;
use crate::error::RecordError;
use crate::frame::Rect;
use chrono::{DateTime, Utc};
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

#[cfg(windows)]
mod audio;
#[cfg(windows)]
mod ffmpeg;
#[cfg(windows)]
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
    /// Audio sample rate (device native).
    pub audio_sample_rate: u32,
    /// Audio channel count in the WAV (mono).
    pub audio_channels: u16,
    /// Audio duration in ms.
    pub audio_duration_ms: u64,
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

/// How long to wait for the first frame after the window is resolved, in
/// addition to `RecordConfig::wait_ms`.
#[cfg(windows)]
const FIRST_FRAME_WAIT_MS: u64 = 10_000;

/// Record `cfg.target` to a video + microphone WAV until `cfg.stop` is set,
/// muxing them into `cfg.video_out`. Returns metadata for the package manifest.
#[cfg(windows)]
pub fn record(cfg: RecordConfig) -> Result<RecordOutcome, RecordError> {
    use crate::capture::CaptureBackend;
    use std::sync::atomic::Ordering;
    use std::sync::{Condvar, Mutex};
    use std::time::{Duration, Instant};

    // Fail fast before touching the mic / capture so we never leave partial files.
    if !ffmpeg::ffmpeg_available() {
        return Err(RecordError::FfmpegNotFound);
    }
    let fps = cfg.fps.clamp(1, 60);

    // Resolve the window first (it may not have launched yet).
    let backend = video::resolve_wgc(&cfg.target, cfg.wait_ms)?;
    let window = backend.window().clone();
    let wgc_stop = backend
        .stop_signal()
        .expect("WGC backend exposes a stop signal");

    // Start the microphone.
    let audio = audio::AudioRecorder::start(cfg.mic.as_deref(), &cfg.audio_out)?;

    // Capture thread: publish conformed frames into the mailbox.
    let mailbox: video::FrameMailbox = Arc::new(Mutex::new(None));
    let dims: video::DimsCell = Arc::new((Mutex::new(None), Condvar::new()));
    let v0: Arc<Mutex<Option<Instant>>> = Arc::new(Mutex::new(None));
    let capture = {
        let (mailbox, dims, v0, stop) =
            (mailbox.clone(), dims.clone(), v0.clone(), cfg.stop.clone());
        let crop = cfg.crop;
        std::thread::spawn(move || video::run_capture(backend, crop, mailbox, dims, v0, stop))
    };

    // Wait for the first frame to lock the recording dimensions.
    let locked = video::wait_for_dims(&dims, &cfg.stop, cfg.wait_ms + FIRST_FRAME_WAIT_MS);
    let (width, height) = match locked {
        Some(d) => d,
        None => {
            cfg.stop.store(true, Ordering::Relaxed);
            wgc_stop.store(true, Ordering::Relaxed);
            let _ = capture.join();
            let _ = audio.finish();
            return Err(RecordError::Capture(crate::error::CaptureError::Backend(
                "the target window produced no frames to record (is it visible and rendering?)"
                    .into(),
            )));
        }
    };

    // Spawn the encoder and pace frames to it at a constant rate until stop.
    let temp_video = cfg.work_dir.join(".framewatch-video.tmp.mp4");
    let mut encoder = ffmpeg::VideoEncoder::spawn(width, height, fps, &temp_video)?;
    let pacing_start = Instant::now();
    let interval_ns = 1_000_000_000u64 / fps as u64;
    let mut k: u64 = 0;
    let mut frames_written: u64 = 0;
    while !cfg.stop.load(Ordering::Relaxed) {
        let deadline = pacing_start + Duration::from_nanos(k.saturating_mul(interval_ns));
        let now = Instant::now();
        if now < deadline {
            std::thread::sleep(deadline - now);
        }
        if cfg.stop.load(Ordering::Relaxed) {
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
    wgc_stop.store(true, Ordering::Relaxed);
    encoder.finish()?;
    let _ = capture.join();
    let audio_stats = audio.finish()?;

    // Align audio start to video start and mux.
    let v0_inst = *v0.lock().unwrap();
    let audio_offset_s = match (v0_inst, audio_stats.first_sample_at) {
        (Some(v), Some(a)) if a >= v => (a - v).as_secs_f64(),
        (Some(v), Some(a)) => -(v - a).as_secs_f64(),
        _ => 0.0,
    };
    ffmpeg::run_mux(&cfg.audio_out, &temp_video, audio_offset_s, &cfg.video_out)?;
    let _ = std::fs::remove_file(&temp_video);

    Ok(RecordOutcome {
        width,
        height,
        fps: fps as f32,
        frames_written,
        video_duration_ms: frames_written * 1000 / fps as u64,
        audio_sample_rate: audio_stats.sample_rate,
        audio_channels: audio_stats.channels,
        audio_duration_ms: audio_stats.duration_ms,
        codec: "h264".into(),
        container: "mp4".into(),
        window_title: window.title,
        window_exe: window.exe,
        ended_at: Utc::now(),
    })
}

/// Recording is only implemented on Windows (via the WGC backend).
#[cfg(not(windows))]
pub fn record(_cfg: RecordConfig) -> Result<RecordOutcome, RecordError> {
    Err(RecordError::Unsupported)
}
