//! Capture events, their metadata (the public agent contract), and encoded images.

use chrono::{DateTime, Utc};
use serde::de::{self, Deserializer};
use serde::ser::Serializer;
use serde::{Deserialize, Serialize};

/// The kind of a capture event.
///
/// Serializes as `snake_case` (`initial`, `transition_start`, `busy_start`,
/// `busy_end`, `value_sample`, `settled`, `manual`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    /// The very first frame of a session.
    Initial,
    /// `Idle -> Active`: a meaningful change began (opt-in).
    TransitionStart,
    /// A spinner/animation region began animating.
    BusyStart,
    /// A spinner/animation region stopped animating.
    BusyEnd,
    /// A throttled sample of a volatile region.
    ValueSample,
    /// The window settled after activity — the "money frame".
    Settled,
    /// A user-requested manual capture.
    Manual,
}

impl EventKind {
    /// All variants, in declaration order.
    pub const ALL: [EventKind; 7] = [
        EventKind::Initial,
        EventKind::TransitionStart,
        EventKind::BusyStart,
        EventKind::BusyEnd,
        EventKind::ValueSample,
        EventKind::Settled,
        EventKind::Manual,
    ];

    /// The lowercase `snake_case` string for this kind.
    pub fn as_str(self) -> &'static str {
        match self {
            EventKind::Initial => "initial",
            EventKind::TransitionStart => "transition_start",
            EventKind::BusyStart => "busy_start",
            EventKind::BusyEnd => "busy_end",
            EventKind::ValueSample => "value_sample",
            EventKind::Settled => "settled",
            EventKind::Manual => "manual",
        }
    }

    #[inline]
    fn bit(self) -> u8 {
        match self {
            EventKind::Initial => 1 << 0,
            EventKind::TransitionStart => 1 << 1,
            EventKind::BusyStart => 1 << 2,
            EventKind::BusyEnd => 1 << 3,
            EventKind::ValueSample => 1 << 4,
            EventKind::Settled => 1 << 5,
            EventKind::Manual => 1 << 6,
        }
    }
}

/// A set of [`EventKind`]s describing which events should get an image saved.
///
/// Serializes as a JSON/TOML array of kind strings, e.g. `["initial", "settled"]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SaveMask(u8);

impl SaveMask {
    /// An empty mask (no kinds save images).
    pub const NONE: SaveMask = SaveMask(0);

    /// Build a mask from a slice of kinds.
    pub fn from_kinds(kinds: &[EventKind]) -> Self {
        let mut bits = 0u8;
        for k in kinds {
            bits |= k.bit();
        }
        SaveMask(bits)
    }

    /// Whether `kind` is in the mask.
    #[inline]
    pub fn contains(self, kind: EventKind) -> bool {
        self.0 & kind.bit() != 0
    }

    /// Add a kind.
    pub fn with(mut self, kind: EventKind) -> Self {
        self.0 |= kind.bit();
        self
    }

    /// The kinds in this mask, in [`EventKind::ALL`] order.
    pub fn kinds(self) -> Vec<EventKind> {
        EventKind::ALL
            .iter()
            .copied()
            .filter(|k| self.contains(*k))
            .collect()
    }
}

impl Default for SaveMask {
    /// `Initial | Settled | BusyEnd | Manual`.
    fn default() -> Self {
        SaveMask::from_kinds(&[
            EventKind::Initial,
            EventKind::Settled,
            EventKind::BusyEnd,
            EventKind::Manual,
        ])
    }
}

impl Serialize for SaveMask {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        self.kinds().serialize(s)
    }
}

impl<'de> Deserialize<'de> for SaveMask {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let kinds = Vec::<EventKind>::deserialize(d).map_err(de::Error::custom)?;
        Ok(SaveMask::from_kinds(&kinds))
    }
}

/// Image container/format for an [`EncodedImage`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ImageFormat {
    /// PNG (default; always available).
    Png,
    /// JPEG (requires the `jpeg` feature).
    Jpeg,
    /// WebP (requires the `webp` feature).
    Webp,
}

impl ImageFormat {
    /// File extension (without the dot).
    pub fn ext(self) -> &'static str {
        match self {
            ImageFormat::Png => "png",
            ImageFormat::Jpeg => "jpg",
            ImageFormat::Webp => "webp",
        }
    }
}

/// An encoded image, ready to write to any sink. Encoded once by the engine.
#[derive(Debug, Clone)]
pub struct EncodedImage {
    /// Encoded image bytes.
    pub bytes: Vec<u8>,
    /// The container format of `bytes`.
    pub format: ImageFormat,
    /// Encoded width in pixels.
    pub width: u32,
    /// Encoded height in pixels.
    pub height: u32,
}

/// Window metadata embedded in a [`CaptureMeta`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowMeta {
    /// Window title.
    pub title: String,
    /// Executable basename.
    pub exe: String,
    /// Window class.
    pub class: String,
    /// Native window handle.
    pub hwnd: isize,
    /// `[x, y, w, h]` window bounds.
    pub rect: [i32; 4],
    /// Window DPI.
    pub dpi: u32,
    /// Whether the window was foreground.
    pub foreground: bool,
}

/// A single changed-region bounding box in pixel coords: `[x, y, w, h]`.
pub type BBox = [i32; 4];

/// Change/diff metadata embedded in a [`CaptureMeta`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangeMeta {
    /// Number of tiles flagged changed.
    pub changed_tiles: u32,
    /// `[cols, rows]` tile grid.
    pub tile_grid: [u16; 2],
    /// Fraction of tiles changed.
    pub area_ratio: f32,
    /// Pixel bounding boxes of changed clusters.
    pub bboxes: Vec<BBox>,
    /// dHash of the saved frame as a hex string (only set when an image was saved).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dhash: Option<String>,
    /// Hamming distance to the previously-saved image's dHash.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hamming_to_prev_emit: Option<u32>,
}

/// One busy region's state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegionMeta {
    /// Region label.
    pub label: String,
    /// Whether the region is currently active/busy.
    pub active: bool,
}

/// Busy/animation metadata embedded in a [`CaptureMeta`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BusyMeta {
    /// Whether any busy region is currently active.
    pub active: bool,
    /// Per-region states.
    pub regions: Vec<RegionMeta>,
}

/// Timing metadata embedded in a [`CaptureMeta`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimingMeta {
    /// Milliseconds since the previous *saved image* emit.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub since_prev_emit_ms: Option<u64>,
    /// How long the window was active before this event.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_for_ms: Option<u64>,
    /// How long the window has been quiescent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quiescent_for_ms: Option<u64>,
}

/// The machine-readable metadata for one event — the public agent contract.
///
/// One of these is serialized per line into `timeline.jsonl`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptureMeta {
    /// Session identifier. Populated by [`DirectorySink`] (which owns the
    /// session); empty when produced by the engine directly unless set via
    /// [`Engine::set_session_id`]. Other sinks (e.g. [`ChannelSink`]) leave it as
    /// the engine produced it.
    ///
    /// [`DirectorySink`]: crate::sink::DirectorySink
    /// [`ChannelSink`]: crate::sink::ChannelSink
    /// [`Engine::set_session_id`]: crate::Engine::set_session_id
    pub session_id: String,
    /// Monotonic event sequence number.
    pub seq: u64,
    /// Stable event id, e.g. `"f000042"`.
    pub id: String,
    /// Event kind.
    pub kind: EventKind,
    /// Wall-clock time of the underlying frame.
    pub wall_time: DateTime<Utc>,
    /// Milliseconds since session start (monotonic).
    pub elapsed_ms: u64,
    /// Relative path to the saved image, or `None` for image-less events.
    ///
    /// Set by [`DirectorySink`](crate::sink::DirectorySink) when it writes the
    /// encoded image file (format per `config.image.format` — PNG/JPEG/WebP); the
    /// engine emits `None` here (the encoded bytes live on
    /// [`CaptureEvent::image`]) so direct consumers read the bytes, not a path.
    pub image: Option<String>,
    /// Window metadata.
    pub window: WindowMeta,
    /// Change/diff metadata.
    pub change: ChangeMeta,
    /// Busy/animation metadata.
    pub busy: BusyMeta,
    /// Timing metadata.
    pub timing: TimingMeta,
    /// Frames observed and collapsed since the previous saved image.
    pub coalesced_frames: u32,
    /// Human/agent-facing note.
    pub note: String,
}

/// A capture event: metadata plus an optional encoded image.
#[derive(Debug, Clone)]
pub struct CaptureEvent {
    /// The event metadata (serialized to the timeline).
    pub meta: CaptureMeta,
    /// The encoded image, or `None` for timeline-only events.
    pub image: Option<EncodedImage>,
}

impl CaptureEvent {
    /// The event kind (shortcut for `self.meta.kind`).
    pub fn kind(&self) -> EventKind {
        self.meta.kind
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_kind_as_str_round_trips_via_serde() {
        for k in EventKind::ALL {
            let json = serde_json::to_string(&k).unwrap();
            assert_eq!(json, format!("\"{}\"", k.as_str()));
            let back: EventKind = serde_json::from_str(&json).unwrap();
            assert_eq!(back, k);
        }
        assert_eq!(EventKind::ALL.len(), 7);
    }

    #[test]
    fn image_format_ext() {
        assert_eq!(ImageFormat::Png.ext(), "png");
        assert_eq!(ImageFormat::Jpeg.ext(), "jpg");
        assert_eq!(ImageFormat::Webp.ext(), "webp");
    }

    #[test]
    fn save_mask_construction_and_membership() {
        assert!(!SaveMask::NONE.contains(EventKind::Settled));
        let m = SaveMask::from_kinds(&[EventKind::Initial, EventKind::Settled]);
        assert!(m.contains(EventKind::Initial));
        assert!(m.contains(EventKind::Settled));
        assert!(!m.contains(EventKind::ValueSample));
        let m2 = SaveMask::NONE.with(EventKind::Manual);
        assert!(m2.contains(EventKind::Manual));
        // `kinds()` returns members in ALL order.
        assert_eq!(m.kinds(), vec![EventKind::Initial, EventKind::Settled]);
    }

    #[test]
    fn save_mask_default_contains_money_frames() {
        let d = SaveMask::default();
        for k in [
            EventKind::Initial,
            EventKind::Settled,
            EventKind::BusyEnd,
            EventKind::Manual,
        ] {
            assert!(d.contains(k), "default should save {k:?}");
        }
        assert!(!d.contains(EventKind::ValueSample));
    }

    #[test]
    fn save_mask_serde_is_kind_array() {
        let m = SaveMask::from_kinds(&[EventKind::Initial, EventKind::BusyEnd]);
        let json = serde_json::to_string(&m).unwrap();
        assert_eq!(json, r#"["initial","busy_end"]"#);
        let back: SaveMask = serde_json::from_str(&json).unwrap();
        assert_eq!(back, m);
    }
}
