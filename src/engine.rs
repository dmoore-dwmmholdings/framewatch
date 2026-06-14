//! The detection engine: a pure, backend-agnostic state machine.
//!
//! [`Engine::process`] is a function of `(state, RawFrame, now) -> events`. It does
//! no I/O and no capture; the clock is injected, so the hard logic is fully
//! unit-testable on any OS.

use crate::clock::{Clock, SystemClock};
use crate::config::Config;
use crate::detect::{
    diff, hamming, Hasher, ImgHash, RegionState, RoiSet, Volatility, WorkingFrame,
};
use crate::event::{
    BusyMeta, CaptureEvent, CaptureMeta, ChangeMeta, EventKind, RegionMeta, TimingMeta, WindowMeta,
};
use crate::frame::RawFrame;
use chrono::{DateTime, Utc};
use smallvec::SmallVec;
use std::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Idle,
    Active,
}

/// The framewatch detection engine.
pub struct Engine<C: Clock = SystemClock> {
    cfg: Config,
    clock: C,
    cols: u16,
    rows: u16,
    roi: RoiSet,
    vol: Volatility,
    hasher: Hasher,

    prev: Option<WorkingFrame>,
    state: State,
    first_done: bool,

    session_start: Option<Instant>,
    session_wall: Option<DateTime<Utc>>,
    active_start: Option<Instant>,
    last_activity: Option<Instant>,
    last_value_sample: Option<Instant>,
    last_emit: Option<Instant>,
    last_saved_dhash: Option<ImgHash>,
    frames_since_save: u32,
    seq: u64,
}

impl<C: Clock> Engine<C> {
    /// Construct an engine for `cfg`, with an injected `clock`.
    pub fn new(cfg: Config, clock: C) -> Self {
        let (cols, rows) = cfg.tile_grid;
        let cols = cols.max(1);
        let rows = rows.max(1);
        let roi = RoiSet::build(&cfg.rois, cols, rows);
        let vol = Volatility::new(
            cfg.volatility_window,
            cfg.busy_rate_threshold,
            cols as usize * rows as usize,
        );
        Self {
            cfg,
            clock,
            cols,
            rows,
            roi,
            vol,
            hasher: Hasher::new(),
            prev: None,
            state: State::Idle,
            first_done: false,
            session_start: None,
            session_wall: None,
            active_start: None,
            last_activity: None,
            last_value_sample: None,
            last_emit: None,
            last_saved_dhash: None,
            frames_since_save: 0,
            seq: 0,
        }
    }

    /// The active configuration.
    pub fn config(&self) -> &Config {
        &self.cfg
    }

    /// Number of events emitted so far.
    pub fn events_emitted(&self) -> u64 {
        self.seq
    }

    /// Process one frame using the injected clock for "now".
    pub fn process_now(&mut self, frame: &RawFrame) -> SmallVec<[CaptureEvent; 2]> {
        let now = self.clock.now();
        self.process(frame, now)
    }

    /// Emit a forced `Manual` capture for `frame`.
    pub fn manual(&mut self, frame: &RawFrame, now: Instant) -> CaptureEvent {
        let wf = WorkingFrame::from_raw(frame, self.cols, self.rows);
        self.ensure_session(now, frame);
        let regions: Vec<RegionState> = Vec::new();
        let ev = self.emit(EventKind::Manual, frame, now, &wf, None, &regions, true);
        self.prev = Some(wf);
        ev
    }

    fn ensure_session(&mut self, now: Instant, frame: &RawFrame) {
        if self.session_start.is_none() {
            self.session_start = Some(now);
            self.session_wall = Some(frame.wall_time);
        }
    }

    /// Process one frame. Returns 0..n events to hand to the sink(s).
    pub fn process(&mut self, frame: &RawFrame, now: Instant) -> SmallVec<[CaptureEvent; 2]> {
        let mut events: SmallVec<[CaptureEvent; 2]> = SmallVec::new();
        self.ensure_session(now, frame);

        let wf = WorkingFrame::from_raw(frame, self.cols, self.rows);

        // 1. Initial frame.
        if !self.first_done {
            self.first_done = true;
            let regions: Vec<RegionState> = Vec::new();
            let ev = self.emit(EventKind::Initial, frame, now, &wf, None, &regions, true);
            self.prev = Some(wf);
            events.push(ev);
            return events;
        }

        // 2. Diff + volatility.
        let prev = self.prev.take().unwrap_or_else(|| wf.clone());
        let td = diff(
            &prev,
            &wf,
            self.cfg.tile_change_threshold,
            self.roi.ignore_mask(),
            frame.width,
            frame.height,
        );
        let regions = self.vol.update(&td, &self.roi);

        let rising: Vec<String> = self.vol.busy_rising().to_vec();
        let falling: Vec<String> = self.vol.busy_falling().to_vec();
        let busy_now = self.vol.any_busy();

        let mut any_saved = false;

        // 4. Busy edges.
        for _label in &rising {
            let ev = self.emit(
                EventKind::BusyStart,
                frame,
                now,
                &wf,
                Some(&td),
                &regions,
                false,
            );
            any_saved |= ev.image.is_some();
            events.push(ev);
        }
        for _label in &falling {
            let ev = self.emit(
                EventKind::BusyEnd,
                frame,
                now,
                &wf,
                Some(&td),
                &regions,
                false,
            );
            any_saved |= ev.image.is_some();
            events.push(ev);
        }

        // 3. Meaningful change.
        let meaningful = self.meaningful(&td);
        let activity = meaningful || busy_now || !rising.is_empty() || !falling.is_empty();
        if activity {
            self.last_activity = Some(now);
        }

        // 5. Transition start.
        if self.state == State::Idle && (meaningful || !rising.is_empty()) {
            self.state = State::Active;
            self.active_start = Some(now);
            if self.cfg.emit_transition_start && meaningful {
                let ev = self.emit(
                    EventKind::TransitionStart,
                    frame,
                    now,
                    &wf,
                    Some(&td),
                    &regions,
                    false,
                );
                any_saved |= ev.image.is_some();
                events.push(ev);
            }
        }

        // 6. Volatile sampling (throttled).
        if self.vol.any_volatile_active() {
            let due = match self.last_value_sample {
                None => true,
                Some(t) => now.duration_since(t).as_millis() >= self.cfg.value_sample_ms as u128,
            };
            if due {
                let ev = self.emit(
                    EventKind::ValueSample,
                    frame,
                    now,
                    &wf,
                    Some(&td),
                    &regions,
                    false,
                );
                any_saved |= ev.image.is_some();
                self.last_value_sample = Some(now);
                events.push(ev);
            }
        }

        // 7. Quiescence / settle.
        if self.state == State::Active && !busy_now {
            let quiet_for = self
                .last_activity
                .map(|t| now.duration_since(t).as_millis())
                .unwrap_or(u128::MAX);
            if quiet_for >= self.cfg.settle_ms as u128 {
                self.state = State::Idle;
                let ev = self.emit(
                    EventKind::Settled,
                    frame,
                    now,
                    &wf,
                    Some(&td),
                    &regions,
                    true,
                );
                any_saved |= ev.image.is_some();
                events.push(ev);
            }
        }

        self.prev = Some(wf);
        if !any_saved {
            self.frames_since_save = self.frames_since_save.saturating_add(1);
        }
        events
    }

    /// Meaningful change = changed tiles outside spinner/volatile regions with
    /// area >= `meaningful_area_ratio`; any changed `Watch` tile also counts.
    fn meaningful(&self, td: &crate::detect::TileDiff) -> bool {
        let total = td.changed.len().max(1) as f32;
        let mut count = 0u32;
        let mut watch_changed = false;
        for (i, &c) in td.changed.iter().enumerate() {
            if !c || self.roi.is_excluded(i) {
                continue;
            }
            count += 1;
            if self.roi.is_watch(i) {
                watch_changed = true;
            }
        }
        watch_changed || (count as f32 / total) >= self.cfg.meaningful_area_ratio
    }

    #[allow(clippy::too_many_arguments)]
    fn emit(
        &mut self,
        kind: EventKind,
        frame: &RawFrame,
        now: Instant,
        wf: &WorkingFrame,
        td: Option<&crate::detect::TileDiff>,
        regions: &[RegionState],
        force: bool,
    ) -> CaptureEvent {
        let prev_emit = self.last_emit;
        let mut save = self.cfg.save_image_for.contains(kind) || force;
        let mut hamming_val: Option<u32> = None;
        let mut this_hash: Option<ImgHash> = None;

        if save {
            if !force {
                if let Some(le) = self.last_emit {
                    if now.duration_since(le).as_millis() < self.cfg.min_emit_interval_ms as u128 {
                        save = false;
                    }
                }
            }
            if save {
                let h = self.hasher.hash(wf);
                if let Some(prev) = &self.last_saved_dhash {
                    let d = hamming(&h, prev);
                    hamming_val = Some(d);
                    if !force && d <= self.cfg.dedup_hamming {
                        save = false;
                    }
                }
                this_hash = Some(h);
            }
        }

        let image = if save {
            match crate::sink::encode(frame, &self.cfg.image) {
                Ok(img) => Some(img),
                Err(e) => {
                    tracing::warn!("framewatch: image encode failed: {e}");
                    None
                }
            }
        } else {
            None
        };

        let saved = image.is_some();
        let coalesced = self.frames_since_save;
        let dhash_hex = if saved {
            this_hash.as_ref().map(|h| h.to_hex())
        } else {
            None
        };
        if saved {
            self.last_saved_dhash = this_hash;
            self.last_emit = Some(now);
            self.frames_since_save = 0;
        }

        let seq = self.seq;
        self.seq += 1;

        let meta = self.build_meta(
            seq,
            kind,
            frame,
            now,
            td,
            regions,
            dhash_hex,
            hamming_val,
            coalesced,
            prev_emit,
        );

        CaptureEvent { meta, image }
    }

    #[allow(clippy::too_many_arguments)]
    fn build_meta(
        &self,
        seq: u64,
        kind: EventKind,
        frame: &RawFrame,
        now: Instant,
        td: Option<&crate::detect::TileDiff>,
        regions: &[RegionState],
        dhash_hex: Option<String>,
        hamming_val: Option<u32>,
        coalesced: u32,
        prev_emit: Option<Instant>,
    ) -> CaptureMeta {
        let elapsed_ms = self
            .session_start
            .map(|s| now.duration_since(s).as_millis() as u64)
            .unwrap_or(0);

        let w = &frame.window;
        let window = WindowMeta {
            title: w.title.clone(),
            exe: w.exe.clone(),
            class: w.class.clone(),
            hwnd: w.hwnd,
            rect: w.rect.to_array(),
            dpi: w.dpi,
            foreground: w.foreground,
        };

        let change = match td {
            Some(d) => ChangeMeta {
                changed_tiles: d.changed_count,
                tile_grid: [self.cols, self.rows],
                area_ratio: d.area_ratio,
                bboxes: d.bboxes.iter().map(|r| r.to_array()).collect(),
                dhash: dhash_hex,
                hamming_to_prev_emit: hamming_val,
            },
            None => ChangeMeta {
                changed_tiles: 0,
                tile_grid: [self.cols, self.rows],
                area_ratio: 0.0,
                bboxes: Vec::new(),
                dhash: dhash_hex,
                hamming_to_prev_emit: hamming_val,
            },
        };

        let busy = BusyMeta {
            active: self.vol.any_busy(),
            regions: regions
                .iter()
                .map(|r| RegionMeta {
                    label: r.label.clone(),
                    active: r.busy,
                })
                .collect(),
        };

        let timing = TimingMeta {
            since_prev_emit_ms: prev_emit.map(|t| now.duration_since(t).as_millis() as u64),
            active_for_ms: self
                .active_start
                .map(|t| now.duration_since(t).as_millis() as u64),
            quiescent_for_ms: self
                .last_activity
                .map(|t| now.duration_since(t).as_millis() as u64),
        };

        let note = self.note_for(kind, &change, regions, &timing, coalesced);

        CaptureMeta {
            session_id: String::new(),
            seq,
            id: format!("f{seq:06}"),
            kind,
            wall_time: frame.wall_time,
            elapsed_ms,
            image: None,
            window,
            change,
            busy,
            timing,
            coalesced_frames: coalesced,
            note,
        }
    }

    fn note_for(
        &self,
        kind: EventKind,
        change: &ChangeMeta,
        regions: &[RegionState],
        timing: &TimingMeta,
        coalesced: u32,
    ) -> String {
        let active_regions: Vec<&str> = regions
            .iter()
            .filter(|r| r.busy)
            .map(|r| r.label.as_str())
            .collect();
        match kind {
            EventKind::Initial => "Session start.".to_string(),
            EventKind::TransitionStart => "Activity started.".to_string(),
            EventKind::BusyStart => {
                if active_regions.is_empty() {
                    "Busy region started animating.".to_string()
                } else {
                    format!("Busy started: {} active.", active_regions.join(", "))
                }
            }
            EventKind::BusyEnd => {
                let secs = timing.active_for_ms.unwrap_or(0) as f32 / 1000.0;
                format!(
                    "Busy region stopped after {secs:.2}s; {coalesced} animation frames collapsed."
                )
            }
            EventKind::ValueSample => "Throttled sample of a volatile region.".to_string(),
            EventKind::Settled => {
                let secs = timing.active_for_ms.unwrap_or(0) as f32 / 1000.0;
                let nboxes = change.bboxes.len();
                format!("Settled after {secs:.2}s of activity in {nboxes} region(s).")
            }
            EventKind::Manual => "Manual capture.".to_string(),
        }
    }
}
