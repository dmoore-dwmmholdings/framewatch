//! ROI ↔ tile mapping and tile masks.

use crate::config::{RoiHint, RoiKind};

/// A per-tile boolean mask (length `cols * rows`).
#[derive(Debug, Clone)]
pub struct TileMask {
    bits: Box<[bool]>,
}

impl TileMask {
    /// An all-`false` mask of `len` tiles.
    pub fn empty(len: usize) -> Self {
        Self {
            bits: vec![false; len].into_boxed_slice(),
        }
    }

    /// Whether tile `idx` is set.
    #[inline]
    pub fn get(&self, idx: usize) -> bool {
        self.bits.get(idx).copied().unwrap_or(false)
    }

    /// Set tile `idx`.
    #[inline]
    pub fn set(&mut self, idx: usize) {
        if let Some(b) = self.bits.get_mut(idx) {
            *b = true;
        }
    }

    /// Whether any tile is set.
    pub fn any(&self) -> bool {
        self.bits.iter().any(|b| *b)
    }

    /// Number of tiles in the mask.
    pub fn len(&self) -> usize {
        self.bits.len()
    }

    /// Whether the mask has zero tiles.
    pub fn is_empty(&self) -> bool {
        self.bits.is_empty()
    }
}

/// A single region resolved to tile indices.
#[derive(Debug, Clone)]
pub struct Region {
    /// Region label.
    pub label: String,
    /// Region kind.
    pub kind: RoiKind,
    /// Tile indices covered by this region.
    pub tiles: Vec<usize>,
}

/// All ROIs resolved against a tile grid, plus derived masks.
#[derive(Debug, Clone)]
pub struct RoiSet {
    cols: u16,
    rows: u16,
    regions: Vec<Region>,
    ignore: TileMask,
    /// Tiles belonging to a Spinner or Volatile region (excluded from "meaningful").
    excluded: TileMask,
    /// Tiles belonging to a Watch region (lower change threshold).
    watch: TileMask,
}

impl RoiSet {
    /// Build a resolved ROI set for a `cols × rows` grid from hints.
    pub fn build(hints: &[RoiHint], cols: u16, rows: u16) -> Self {
        let len = cols as usize * rows as usize;
        let mut ignore = TileMask::empty(len);
        let mut excluded = TileMask::empty(len);
        let mut watch = TileMask::empty(len);
        let mut regions = Vec::with_capacity(hints.len());

        for h in hints {
            let tiles = tiles_for_rect(h.rect_norm, cols, rows);
            for &t in &tiles {
                match h.kind {
                    RoiKind::Ignore => ignore.set(t),
                    RoiKind::Spinner | RoiKind::Volatile => excluded.set(t),
                    RoiKind::Watch => watch.set(t),
                }
            }
            regions.push(Region {
                label: h.label.clone(),
                kind: h.kind,
                tiles,
            });
        }

        Self {
            cols,
            rows,
            regions,
            ignore,
            excluded,
            watch,
        }
    }

    /// Grid columns.
    pub fn cols(&self) -> u16 {
        self.cols
    }

    /// Grid rows.
    pub fn rows(&self) -> u16 {
        self.rows
    }

    /// The resolved regions.
    pub fn regions(&self) -> &[Region] {
        &self.regions
    }

    /// The ignore mask (tiles excluded from diffing).
    pub fn ignore_mask(&self) -> &TileMask {
        &self.ignore
    }

    /// Whether tile `idx` belongs to a Spinner/Volatile region.
    #[inline]
    pub fn is_excluded(&self, idx: usize) -> bool {
        self.excluded.get(idx)
    }

    /// Whether tile `idx` belongs to a Watch region.
    #[inline]
    pub fn is_watch(&self, idx: usize) -> bool {
        self.watch.get(idx)
    }

    /// Regions of the given kind, by index into [`regions`](RoiSet::regions).
    pub fn region_indices_of_kind(&self, kind: RoiKind) -> impl Iterator<Item = usize> + '_ {
        self.regions
            .iter()
            .enumerate()
            .filter(move |(_, r)| r.kind == kind)
            .map(|(i, _)| i)
    }
}

/// Map a client-normalized `[x, y, w, h]` rect to the set of covered tile indices.
pub fn tiles_for_rect(rect_norm: [f32; 4], cols: u16, rows: u16) -> Vec<usize> {
    let [x, y, w, h] = rect_norm;
    let cols_f = cols as f32;
    let rows_f = rows as f32;

    let c0 = (x * cols_f).floor().clamp(0.0, cols_f) as i64;
    let c1 = ((x + w) * cols_f).ceil().clamp(0.0, cols_f) as i64;
    let r0 = (y * rows_f).floor().clamp(0.0, rows_f) as i64;
    let r1 = ((y + h) * rows_f).ceil().clamp(0.0, rows_f) as i64;

    let mut out = Vec::new();
    for r in r0..r1.max(r0) {
        for c in c0..c1.max(c0) {
            out.push((r as usize) * cols as usize + c as usize);
        }
    }
    // Guarantee at least one tile for a tiny but non-empty region.
    if out.is_empty() && w > 0.0 && h > 0.0 {
        let c = ((x * cols_f) as i64).clamp(0, cols as i64 - 1) as usize;
        let r = ((y * rows_f) as i64).clamp(0, rows as i64 - 1) as usize;
        out.push(r * cols as usize + c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tile_mask_set_get_any_len() {
        let mut m = TileMask::empty(4);
        assert!(!m.any());
        assert_eq!(m.len(), 4);
        assert!(!m.is_empty());
        m.set(2);
        m.set(99); // out of range: ignored, no panic
        assert!(m.get(2));
        assert!(!m.get(0));
        assert!(!m.get(99));
        assert!(m.any());
        assert!(TileMask::empty(0).is_empty());
    }

    #[test]
    fn tiles_for_rect_covers_expected_cells() {
        // Top-left quarter of a 10x10 grid.
        let tiles = tiles_for_rect([0.0, 0.0, 0.5, 0.5], 10, 10);
        assert_eq!(tiles.len(), 25);
        assert!(tiles.contains(&0));
        assert!(!tiles.contains(&9)); // top-right corner not covered
    }

    #[test]
    fn tiles_for_rect_tiny_region_gets_one_tile() {
        let tiles = tiles_for_rect([0.5, 0.5, 0.001, 0.001], 4, 4);
        assert_eq!(tiles.len(), 1);
        // Zero-area region -> no tiles.
        assert!(tiles_for_rect([0.5, 0.5, 0.0, 0.0], 4, 4).is_empty());
    }

    #[test]
    fn roiset_builds_masks_and_regions() {
        let hints = vec![
            RoiHint {
                kind: RoiKind::Ignore,
                label: "clock".into(),
                rect_norm: [0.0, 0.0, 0.25, 0.25],
            },
            RoiHint {
                kind: RoiKind::Spinner,
                label: "spin".into(),
                rect_norm: [0.5, 0.5, 0.25, 0.25],
            },
            RoiHint {
                kind: RoiKind::Watch,
                label: "watch".into(),
                rect_norm: [0.75, 0.0, 0.25, 0.25],
            },
            RoiHint {
                kind: RoiKind::Volatile,
                label: "vol".into(),
                rect_norm: [0.0, 0.75, 0.25, 0.25],
            },
        ];
        let set = RoiSet::build(&hints, 4, 4);
        assert_eq!((set.cols(), set.rows()), (4, 4));
        assert_eq!(set.regions().len(), 4);
        assert!(set.ignore_mask().any());
        // tile 0 is in the Ignore region; the Spinner tile (10) is excluded.
        assert!(set.ignore_mask().get(0));
        assert!(set.is_excluded(10)); // row 2, col 2 of a 4x4 grid
        assert!(set.is_watch(3)); // row 0, col 3
        let spinners: Vec<usize> = set.region_indices_of_kind(RoiKind::Spinner).collect();
        assert_eq!(spinners, vec![1]);
    }
}
