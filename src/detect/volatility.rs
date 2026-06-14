//! Per-tile temporal change statistics and busy-region edge detection.

use crate::config::RoiKind;
use crate::detect::diff::TileDiff;
use crate::detect::roi::RoiSet;

/// A rolled-up snapshot of one region's recent behaviour.
#[derive(Debug, Clone)]
pub struct RegionState {
    /// Region label.
    pub label: String,
    /// Region kind.
    pub kind: RoiKind,
    /// Whether the region is currently busy (Spinner) / active (Volatile).
    pub busy: bool,
    /// Mean per-tile change rate over the window (`0.0..=1.0`).
    pub change_rate: f32,
}

/// Per-tile ring buffers of change flags plus per-region busy edge tracking.
#[derive(Debug)]
pub struct Volatility {
    window: u16,
    busy_threshold: f32,
    num_tiles: usize,
    /// Flattened ring: `ring[slot * num_tiles + tile]`.
    ring: Vec<bool>,
    /// Ones-in-window count per tile.
    ones: Vec<u16>,
    slot: usize,
    frames_seen: u32,
    /// Per-region busy state, parallel to the last `update`'s region order.
    region_busy: Vec<bool>,
    rising: Vec<String>,
    falling: Vec<String>,
    any_busy: bool,
    any_volatile_active: bool,
}

impl Volatility {
    /// Create a tracker for `num_tiles` tiles.
    pub fn new(window: u16, busy_threshold: f32, num_tiles: usize) -> Self {
        let window = window.max(1);
        Self {
            window,
            busy_threshold,
            num_tiles,
            ring: vec![false; window as usize * num_tiles],
            ones: vec![0u16; num_tiles],
            slot: 0,
            frames_seen: 0,
            region_busy: Vec::new(),
            rising: Vec::new(),
            falling: Vec::new(),
            any_busy: false,
            any_volatile_active: false,
        }
    }

    /// Per-tile change rate over the window (`ones / window`).
    #[inline]
    pub fn tile_change_rate(&self, idx: usize) -> f32 {
        if idx >= self.num_tiles {
            return 0.0;
        }
        self.ones[idx] as f32 / self.window as f32
    }

    /// Push a new diff and recompute region rollups + edges.
    pub fn update(&mut self, diff: &TileDiff, roi: &RoiSet) -> Vec<RegionState> {
        // Advance the ring: drop the slot we're about to overwrite, write new flags.
        let base = self.slot * self.num_tiles;
        for tile in 0..self.num_tiles {
            let old = self.ring[base + tile];
            let new = diff.changed.get(tile).copied().unwrap_or(false);
            if old {
                self.ones[tile] = self.ones[tile].saturating_sub(1);
            }
            if new {
                self.ones[tile] = (self.ones[tile] + 1).min(self.window);
            }
            self.ring[base + tile] = new;
        }
        self.slot = (self.slot + 1) % self.window as usize;
        self.frames_seen = self.frames_seen.saturating_add(1);

        // Region rollups.
        let regions = roi.regions();
        let mut states = Vec::with_capacity(regions.len());
        if self.region_busy.len() != regions.len() {
            self.region_busy = vec![false; regions.len()];
        }
        self.rising.clear();
        self.falling.clear();
        self.any_busy = false;
        self.any_volatile_active = false;

        for (i, region) in regions.iter().enumerate() {
            let rate = if region.tiles.is_empty() {
                0.0
            } else {
                let sum: f32 = region.tiles.iter().map(|&t| self.tile_change_rate(t)).sum();
                sum / region.tiles.len() as f32
            };

            let busy = match region.kind {
                RoiKind::Spinner => rate >= self.busy_threshold,
                // Volatile: "active" if there's any recent change at all.
                RoiKind::Volatile => rate > 0.0,
                _ => false,
            };

            // Edge detection (Spinner regions drive BusyStart/BusyEnd).
            if region.kind == RoiKind::Spinner {
                let prev = self.region_busy[i];
                if busy && !prev {
                    self.rising.push(region.label.clone());
                } else if !busy && prev {
                    self.falling.push(region.label.clone());
                }
                if busy {
                    self.any_busy = true;
                }
            }
            if region.kind == RoiKind::Volatile && busy {
                self.any_volatile_active = true;
            }
            self.region_busy[i] = busy;

            states.push(RegionState {
                label: region.label.clone(),
                kind: region.kind,
                busy,
                change_rate: rate,
            });
        }

        states
    }

    /// Spinner regions that became busy this update.
    pub fn busy_rising(&self) -> &[String] {
        &self.rising
    }

    /// Spinner regions that stopped being busy this update.
    pub fn busy_falling(&self) -> &[String] {
        &self.falling
    }

    /// Whether any Spinner region is currently busy.
    pub fn any_busy(&self) -> bool {
        self.any_busy
    }

    /// Whether any Volatile region currently has recent change.
    pub fn any_volatile_active(&self) -> bool {
        self.any_volatile_active
    }
}
