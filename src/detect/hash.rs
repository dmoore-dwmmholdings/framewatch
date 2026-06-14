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
