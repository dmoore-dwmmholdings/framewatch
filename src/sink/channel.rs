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
