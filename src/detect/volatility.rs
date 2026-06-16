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
    cols: usize,
    rows: usize,
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
    // Opt-in automatic spinner detection.
    auto_spinner: bool,
    auto_max_area: f32,
    auto_excluded: Vec<bool>,
    auto_busy: bool,
    auto_busy_prev: bool,
    auto_rising: bool,
    auto_falling: bool,
}

impl Volatility {
    /// Create a tracker for `num_tiles` tiles.
    ///
    /// `auto_spinner` enables automatic detection of a small, compact cluster of
    /// high-change-rate tiles (covering at most `auto_max_area` of the frame and
    /// forming a single connected cluster) as a spinner, with no ROI hint.
    pub fn new(
        window: u16,
        busy_threshold: f32,
        cols: usize,
        rows: usize,
        auto_spinner: bool,
        auto_max_area: f32,
    ) -> Self {
        let window = window.max(1);
        let num_tiles = cols * rows;
        Self {
            window,
            busy_threshold,
            cols,
            rows,
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
            auto_spinner,
            auto_max_area,
            auto_excluded: vec![false; num_tiles],
            auto_busy: false,
            auto_busy_prev: false,
            auto_rising: false,
            auto_falling: false,
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

        self.detect_auto_spinner(roi);

        states
    }

    /// Opt-in: classify a small, compact cluster of high-change-rate tiles (not
    /// already covered by a hint and not ignored) as a spinner. Sets the
    /// auto-excluded mask and rising/falling edges.
    fn detect_auto_spinner(&mut self, roi: &RoiSet) {
        self.auto_rising = false;
        self.auto_falling = false;
        if !self.auto_spinner {
            return;
        }
        let mut hot = 0usize;
        for t in 0..self.num_tiles {
            // Exclude Watch tiles: `Engine::meaningful` drops `auto_excluded`
            // tiles *before* the `is_watch` check, so auto-excluding a watch tile
            // would silently swallow the very change the user asked to be told about.
            let is_hot = self.tile_change_rate(t) >= self.busy_threshold
                && !roi.is_excluded(t)
                && !roi.is_watch(t)
                && !roi.ignore_mask().get(t);
            self.auto_excluded[t] = is_hot;
            if is_hot {
                hot += 1;
            }
        }
        let area = hot as f32 / self.num_tiles.max(1) as f32;
        // A spinner is a *small, connected* churning cluster; larger churn is
        // real content, and hot tiles scattered across the frame (small in total
        // area but spatially dispersed) are content too — not a spinner.
        let is_spinner =
            hot > 0 && area <= self.auto_max_area && self.largest_hot_component() == hot;
        if !is_spinner {
            for b in self.auto_excluded.iter_mut() {
                *b = false;
            }
        }
        let prev = self.auto_busy_prev;
        self.auto_busy = is_spinner;
        self.auto_rising = is_spinner && !prev;
        self.auto_falling = !is_spinner && prev;
        self.auto_busy_prev = is_spinner;
    }

    /// Size of the largest 4-connected cluster of currently-hot
    /// (`auto_excluded`) tiles. A genuine spinner is one contiguous blob, so
    /// when this is less than the total hot count the churn is spatially
    /// scattered and must not be treated as a spinner.
    fn largest_hot_component(&self) -> usize {
        let (cols, rows) = (self.cols, self.rows);
        let mut visited = vec![false; self.num_tiles];
        let mut stack: Vec<usize> = Vec::new();
        let mut best = 0usize;
        for start in 0..self.num_tiles {
            if !self.auto_excluded[start] || visited[start] {
                continue;
            }
            let mut size = 0usize;
            visited[start] = true;
            stack.push(start);
            while let Some(t) = stack.pop() {
                size += 1;
                let (c, r) = (t % cols, t / cols);
                let mut visit = |n: usize, stack: &mut Vec<usize>| {
                    if self.auto_excluded[n] && !visited[n] {
                        visited[n] = true;
                        stack.push(n);
                    }
                };
                if c > 0 {
                    visit(t - 1, &mut stack);
                }
                if c + 1 < cols {
                    visit(t + 1, &mut stack);
                }
                if r > 0 {
                    visit(t - cols, &mut stack);
                }
                if r + 1 < rows {
                    visit(t + cols, &mut stack);
                }
            }
            best = best.max(size);
        }
        best
    }

    /// Whether tile `idx` belongs to the current auto-detected spinner.
    #[inline]
    pub fn auto_excluded(&self, idx: usize) -> bool {
        self.auto_excluded.get(idx).copied().unwrap_or(false)
    }

    /// Whether an auto-detected spinner is currently busy.
    pub fn auto_busy(&self) -> bool {
        self.auto_busy
    }

    /// Whether an auto-detected spinner began animating this update.
    pub fn auto_rising(&self) -> bool {
        self.auto_rising
    }

    /// Whether an auto-detected spinner stopped animating this update.
    pub fn auto_falling(&self) -> bool {
        self.auto_falling
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
