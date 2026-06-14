//! Downsampled working frame and per-tile diffing.

use crate::detect::roi::TileMask;
use crate::frame::{RawFrame, Rect};

/// A frame downsampled to a tile grid, one mean-luminance byte per tile.
///
/// Building this is the only per-frame full-buffer pass; everything downstream
/// is `O(tiles)`.
#[derive(Debug, Clone)]
pub struct WorkingFrame {
    /// Grid columns.
    pub cols: u16,
    /// Grid rows.
    pub rows: u16,
    /// Mean luminance per tile, length `cols * rows`, row-major.
    pub luma: Box<[u8]>,
}

impl WorkingFrame {
    /// Box-average each tile of `frame` into a `cols × rows` luminance grid.
    pub fn from_raw(frame: &RawFrame, cols: u16, rows: u16) -> Self {
        let cols = cols.max(1);
        let rows = rows.max(1);
        let w = frame.width.max(1);
        let h = frame.height.max(1);
        let mut luma = vec![0u8; cols as usize * rows as usize];

        for ty in 0..rows as u32 {
            // Pixel row range for this tile.
            let y0 = (ty * h) / rows as u32;
            let y1 = (((ty + 1) * h) / rows as u32).max(y0 + 1).min(h);
            for tx in 0..cols as u32 {
                let x0 = (tx * w) / cols as u32;
                let x1 = (((tx + 1) * w) / cols as u32).max(x0 + 1).min(w);

                let mut sum: u64 = 0;
                let mut count: u64 = 0;
                // Sample a bounded number of pixels per tile to keep this cheap on
                // very large windows (step so we touch ~<=16x16 samples per tile).
                let step_x = ((x1 - x0) / 16).max(1);
                let step_y = ((y1 - y0) / 16).max(1);
                let mut py = y0;
                while py < y1 {
                    // usize math avoids u32 overflow on very large frames.
                    let row_off = py as usize * frame.stride as usize;
                    let mut px = x0;
                    while px < x1 {
                        let off = row_off + px as usize * 4;
                        let b = frame.buffer.get(off).copied().unwrap_or(0) as u64;
                        let g = frame.buffer.get(off + 1).copied().unwrap_or(0) as u64;
                        let r = frame.buffer.get(off + 2).copied().unwrap_or(0) as u64;
                        // Integer luma approximation: (54*R + 183*G + 19*B) >> 8.
                        sum += (54 * r + 183 * g + 19 * b) >> 8;
                        count += 1;
                        px += step_x;
                    }
                    py += step_y;
                }
                let mean = if count == 0 { 0 } else { (sum / count) as u8 };
                luma[(ty * cols as u32 + tx) as usize] = mean;
            }
        }

        Self {
            cols,
            rows,
            luma: luma.into_boxed_slice(),
        }
    }

    /// Total number of tiles.
    pub fn len(&self) -> usize {
        self.luma.len()
    }

    /// Whether the grid is empty.
    pub fn is_empty(&self) -> bool {
        self.luma.is_empty()
    }
}

/// The result of diffing two working frames.
#[derive(Debug, Clone)]
pub struct TileDiff {
    /// Per-tile changed mask, length `cols * rows`.
    pub changed: Box<[bool]>,
    /// Number of changed tiles.
    pub changed_count: u32,
    /// `changed_count / total_tiles`.
    pub area_ratio: f32,
    /// Pixel bounding boxes of 4-connected changed clusters.
    pub bboxes: Vec<Rect>,
    /// Grid columns (for downstream callers).
    pub cols: u16,
    /// Grid rows.
    pub rows: u16,
}

impl TileDiff {
    /// An empty diff for a grid of `cols × rows`.
    pub fn empty(cols: u16, rows: u16) -> Self {
        Self {
            changed: vec![false; cols as usize * rows as usize].into_boxed_slice(),
            changed_count: 0,
            area_ratio: 0.0,
            bboxes: Vec::new(),
            cols,
            rows,
        }
    }
}

/// Diff `cur` against `prev`. A tile is "changed" if its luma delta exceeds
/// `threshold`, unless it is set in `ignore`.
///
/// `frame_w`/`frame_h` are used to scale tile bounding boxes back to pixels.
pub fn diff(
    prev: &WorkingFrame,
    cur: &WorkingFrame,
    threshold: u8,
    ignore: &TileMask,
    frame_w: u32,
    frame_h: u32,
) -> TileDiff {
    let cols = cur.cols;
    let rows = cur.rows;
    let len = cur.luma.len();
    let mut changed = vec![false; len].into_boxed_slice();
    let mut count = 0u32;

    if prev.luma.len() == len {
        for i in 0..len {
            if ignore.get(i) {
                continue;
            }
            let d = prev.luma[i].abs_diff(cur.luma[i]);
            if d > threshold {
                changed[i] = true;
                count += 1;
            }
        }
    }

    let total = len.max(1) as f32;
    let bboxes = connected_bboxes(&changed, cols, rows, frame_w, frame_h);

    TileDiff {
        changed,
        changed_count: count,
        area_ratio: count as f32 / total,
        bboxes,
        cols,
        rows,
    }
}

/// 4-connected component pass over the tile mask, returning pixel-space bounding boxes.
fn connected_bboxes(
    changed: &[bool],
    cols: u16,
    rows: u16,
    frame_w: u32,
    frame_h: u32,
) -> Vec<Rect> {
    let cols = cols as usize;
    let rows = rows as usize;
    let mut visited = vec![false; changed.len()];
    let mut boxes = Vec::new();
    let mut stack: Vec<usize> = Vec::new();

    for start in 0..changed.len() {
        if !changed[start] || visited[start] {
            continue;
        }
        // BFS/DFS flood fill from this tile.
        let (mut min_c, mut min_r, mut max_c, mut max_r) = (usize::MAX, usize::MAX, 0usize, 0usize);
        stack.clear();
        stack.push(start);
        visited[start] = true;
        while let Some(idx) = stack.pop() {
            let c = idx % cols;
            let r = idx / cols;
            min_c = min_c.min(c);
            min_r = min_r.min(r);
            max_c = max_c.max(c);
            max_r = max_r.max(r);

            // 4-neighbours
            if c > 0 {
                let n = idx - 1;
                if changed[n] && !visited[n] {
                    visited[n] = true;
                    stack.push(n);
                }
            }
            if c + 1 < cols {
                let n = idx + 1;
                if changed[n] && !visited[n] {
                    visited[n] = true;
                    stack.push(n);
                }
            }
            if r > 0 {
                let n = idx - cols;
                if changed[n] && !visited[n] {
                    visited[n] = true;
                    stack.push(n);
                }
            }
            if r + 1 < rows {
                let n = idx + cols;
                if changed[n] && !visited[n] {
                    visited[n] = true;
                    stack.push(n);
                }
            }
        }

        // Scale tile box -> pixels.
        let px0 = (min_c as u32 * frame_w) / cols as u32;
        let py0 = (min_r as u32 * frame_h) / rows as u32;
        let px1 = ((max_c as u32 + 1) * frame_w) / cols as u32;
        let py1 = ((max_r as u32 + 1) * frame_h) / rows as u32;
        boxes.push(Rect::new(
            px0 as i32,
            py0 as i32,
            px1.saturating_sub(px0),
            py1.saturating_sub(py0),
        ));
    }

    boxes
}
