//! Error types for framewatch.

use thiserror::Error;

/// Errors produced by a [`CaptureBackend`](crate::capture::CaptureBackend).
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum CaptureError {
    /// The requested target window could not be found / resolved.
    #[error("target window not found: {0}")]
    TargetNotFound(String),

    /// The target window was closed while capturing.
    #[error("target window closed")]
    WindowClosed,

    /// A platform / OS capture API error.
    #[error("capture backend error: {0}")]
    Backend(String),

    /// I/O error (e.g. reading mock fixtures).
    #[error(transparent)]
    Io(#[from] std::io::Error),

    /// Image decode error (mock backend reading PNGs).
    #[error("image decode error: {0}")]
    Decode(String),
}

/// Errors produced by a [`Sink`](crate::sink::Sink).
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum SinkError {
    /// I/O error writing artifacts.
    #[error(transparent)]
    Io(#[from] std::io::Error),

    /// JSON (de)serialization error for timeline / manifest.
    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),

    /// Image encoding error.
    #[error("image encode error: {0}")]
    Encode(String),

    /// The channel receiver for a [`ChannelSink`](crate::sink::ChannelSink) was dropped.
    #[error("channel receiver disconnected")]
    Disconnected,
}

/// Errors produced while transcribing recorded audio (the `record` package's
/// voice narration). Used by [`Transcriber`](crate::transcript::Transcriber).
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum TranscribeError {
    /// I/O error reading audio / writing the transcript.
    #[error(transparent)]
    Io(#[from] std::io::Error),

    /// JSON (de)serialization error for the transcript.
    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),

    /// An external `--transcribe-cmd` exited non-zero (code, captured stderr).
    #[error("transcribe command exited with code {0}: {1}")]
    CommandFailed(i32, String),

    /// The transcriber's output could not be parsed as JSON or SRT.
    #[error("could not parse transcriber output: {0}")]
    Parse(String),

    /// Audio decode / format error (e.g. reading the WAV).
    #[error("audio error: {0}")]
    Audio(String),

    /// A bundled whisper.cpp error (only meaningful with the `whisper` feature).
    #[error("whisper error: {0}")]
    Whisper(String),
}

/// Errors produced by the `record` A/V capture runtime.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum RecordError {
    /// `ffmpeg` was not found on `PATH` (needed to encode/mux the recording).
    #[error("ffmpeg not found on PATH — install ffmpeg and ensure it is on PATH")]
    FfmpegNotFound,

    /// `ffmpeg` exited with a non-success status.
    #[error("ffmpeg exited with status {0}")]
    FfmpegFailed(std::process::ExitStatus),

    /// No microphone / audio input device is available.
    #[error("no microphone / audio input device available")]
    NoInputDevice,

    /// The input device's sample format is not supported.
    #[error("unsupported audio input sample format")]
    UnsupportedSampleFormat,

    /// An audio-device error (cpal build/play/stream), stringified.
    #[error("audio device error: {0}")]
    Audio(String),

    /// A WAV write error (hound), stringified.
    #[error("wav write error: {0}")]
    Wav(String),

    /// A capture-backend error (resolving / capturing the target window).
    #[error(transparent)]
    Capture(#[from] CaptureError),

    /// Recording is not available on this platform / build.
    #[error("recording requires a Windows build with the `record` feature")]
    Unsupported,

    /// I/O error (spawning ffmpeg, writing temp files, …).
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// Top-level crate error, returned by [`watch`](crate::watch) and [`default_backend`](crate::default_backend).
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum Error {
    /// A capture-backend error.
    #[error(transparent)]
    Capture(#[from] CaptureError),

    /// A sink error.
    #[error(transparent)]
    Sink(#[from] SinkError),

    /// A recording-runtime error.
    #[error(transparent)]
    Record(#[from] RecordError),

    /// A transcription error.
    #[error(transparent)]
    Transcribe(#[from] TranscribeError),

    /// Invalid configuration.
    #[error("invalid configuration: {0}")]
    Config(String),

    /// The current platform / build has no capture backend available.
    #[error("no capture backend available: {0}")]
    NoBackend(String),

    /// I/O error.
    #[error(transparent)]
    Io(#[from] std::io::Error),
}
