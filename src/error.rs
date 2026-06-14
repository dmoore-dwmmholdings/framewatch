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
