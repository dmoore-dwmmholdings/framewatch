//! Detection pipeline: tile diffing, perceptual hashing, ROI mapping, and
//! temporal volatility / busy-region detection.
//!
//! All steps operate on a cheap [`WorkingFrame`] (the full frame downsampled to a
//! tile grid). Downsampling is the only per-frame full-buffer pass; everything
//! downstream is `O(tiles)`.

pub mod diff;
pub mod hash;
pub mod roi;
pub mod volatility;

pub use diff::{diff, TileDiff, WorkingFrame};
pub use hash::{hamming, Hasher, ImgHash};
pub use roi::{tiles_for_rect, Region, RoiSet, TileMask};
pub use volatility::{RegionState, Volatility};
