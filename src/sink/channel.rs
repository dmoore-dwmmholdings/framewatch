//! A sink that forwards owned events to a channel for an embedding host.

use crate::error::SinkError;
use crate::event::CaptureEvent;
use crate::sink::Sink;
use crossbeam_channel::Sender;

/// Forwards each event to a [`crossbeam_channel::Sender`].
///
/// Use the paired [`crossbeam_channel::Receiver`] in your host application.
pub struct ChannelSink {
    tx: Sender<CaptureEvent>,
}

impl ChannelSink {
    /// Wrap a sender.
    pub fn new(tx: Sender<CaptureEvent>) -> Self {
        Self { tx }
    }

    /// Create a bounded channel and return the sink + receiver.
    pub fn bounded(cap: usize) -> (Self, crossbeam_channel::Receiver<CaptureEvent>) {
        let (tx, rx) = crossbeam_channel::bounded(cap);
        (Self { tx }, rx)
    }

    /// Create an unbounded channel and return the sink + receiver.
    pub fn unbounded() -> (Self, crossbeam_channel::Receiver<CaptureEvent>) {
        let (tx, rx) = crossbeam_channel::unbounded();
        (Self { tx }, rx)
    }
}

impl Sink for ChannelSink {
    fn on_event(&mut self, event: &CaptureEvent) -> Result<(), SinkError> {
        self.tx
            .send(event.clone())
            .map_err(|_| SinkError::Disconnected)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::SystemClock;
    use crate::config::{Config, Target};
    use crate::engine::Engine;
    use crate::frame::{RawFrame, WindowInfo};
    use std::time::Instant;

    fn event() -> CaptureEvent {
        let f = RawFrame::from_bgra(
            vec![128u8; 16 * 16 * 4],
            16,
            16,
            Instant::now(),
            chrono::Utc::now(),
            WindowInfo::synthetic("t", 16, 16),
        );
        let cfg = Config::builder()
            .target(Target::ByExe("x".into()))
            .build()
            .unwrap();
        Engine::new(cfg, SystemClock).process(&f, Instant::now())[0].clone()
    }

    #[test]
    fn unbounded_and_bounded_forward_events() {
        let (mut sink, rx) = ChannelSink::unbounded();
        sink.on_event(&event()).unwrap();
        assert!(rx.try_recv().is_ok());

        let (mut sink, rx) = ChannelSink::bounded(4);
        sink.on_event(&event()).unwrap();
        assert_eq!(rx.len(), 1);
    }

    #[test]
    fn new_wraps_an_existing_sender() {
        let (tx, rx) = crossbeam_channel::unbounded();
        let mut sink = ChannelSink::new(tx);
        sink.on_event(&event()).unwrap();
        assert!(rx.try_recv().is_ok());
    }

    #[test]
    fn disconnected_when_receiver_dropped() {
        let (mut sink, rx) = ChannelSink::unbounded();
        drop(rx);
        assert!(matches!(
            sink.on_event(&event()),
            Err(SinkError::Disconnected)
        ));
    }
}
