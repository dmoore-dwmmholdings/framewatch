//! Perceptual (gradient/dHash) hashing for image dedup.

use crate::detect::diff::WorkingFrame;
use image_hasher::{HashAlg, HasherConfig, ImageHash};

/// A 64-bit perceptual hash of a frame.
#[derive(Debug, Clone)]
pub struct ImgHash(ImageHash);

impl ImgHash {
    /// Lowercase hex encoding of the hash bytes.
    pub fn to_hex(&self) -> String {
        let bytes = self.0.as_bytes();
        let mut s = String::with_capacity(bytes.len() * 2);
        for b in bytes {
            s.push(nibble(b >> 4));
            s.push(nibble(b & 0x0f));
        }
        s
    }
}

fn nibble(n: u8) -> char {
    match n {
        0..=9 => (b'0' + n) as char,
        _ => (b'a' + (n - 10)) as char,
    }
}

/// Wraps an [`image_hasher`] hasher configured for gradient (dHash) hashing.
pub struct Hasher {
    inner: image_hasher::Hasher,
}

impl std::fmt::Debug for Hasher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Hasher").finish_non_exhaustive()
    }
}

impl Default for Hasher {
    fn default() -> Self {
        Self::new()
    }
}

impl Hasher {
    /// Build a gradient/dHash hasher with an 8×8 hash size (64-bit).
    pub fn new() -> Self {
        let inner = HasherConfig::new()
            .hash_alg(HashAlg::Gradient)
            .hash_size(8, 8)
            .to_hasher();
        Self { inner }
    }

    /// Hash a working frame's luminance grid.
    pub fn hash(&self, wf: &WorkingFrame) -> ImgHash {
        let gray = image::GrayImage::from_raw(wf.cols as u32, wf.rows as u32, wf.luma.to_vec())
            .unwrap_or_else(|| image::GrayImage::new(1, 1));
        let dynimg = image::DynamicImage::ImageLuma8(gray);
        ImgHash(self.inner.hash_image(&dynimg))
    }
}

/// Hamming distance between two hashes.
pub fn hamming(a: &ImgHash, b: &ImgHash) -> u32 {
    a.0.dist(&b.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wf(cols: u16, rows: u16, fill: impl Fn(u16, u16) -> u8) -> WorkingFrame {
        let mut luma = Vec::with_capacity(cols as usize * rows as usize);
        for r in 0..rows {
            for c in 0..cols {
                luma.push(fill(c, r));
            }
        }
        WorkingFrame {
            cols,
            rows,
            luma: luma.into_boxed_slice(),
        }
    }

    #[test]
    fn identical_frames_hash_equal_distance_zero() {
        let h = Hasher::new();
        let a = wf(16, 16, |c, _| (c * 16) as u8);
        let b = wf(16, 16, |c, _| (c * 16) as u8);
        assert_eq!(hamming(&h.hash(&a), &h.hash(&b)), 0);
    }

    #[test]
    fn different_frames_differ() {
        let h = Hasher::default();
        let a = wf(16, 16, |c, _| (c * 16) as u8); // horizontal gradient
        let b = wf(16, 16, |_, r| (r * 16) as u8); // vertical gradient
        assert!(hamming(&h.hash(&a), &h.hash(&b)) > 0);
    }

    #[test]
    fn to_hex_is_lowercase_and_64_bit() {
        let h = Hasher::new();
        let hex = h.hash(&wf(16, 16, |c, r| (c ^ r) as u8)).to_hex();
        assert_eq!(hex.len(), 16); // 8 bytes -> 16 hex chars
        assert!(hex
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
    }

    #[test]
    fn degenerate_frame_does_not_panic() {
        // Empty luma -> GrayImage::from_raw returns None -> 1x1 fallback.
        let h = Hasher::new();
        let _ = h.hash(&wf(0, 0, |_, _| 0));
    }
}
