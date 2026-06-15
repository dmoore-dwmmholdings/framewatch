//! Pure, dependency-free audio helpers for the recording runtime. Compiled only
//! with the `record` feature.

/// Average interleaved `channels`-channel f32 samples down to mono. A
/// `channels` of 0 or 1 returns the input unchanged.
pub(crate) fn downmix_to_mono(interleaved: &[f32], channels: u16) -> Vec<f32> {
    if channels <= 1 {
        return interleaved.to_vec();
    }
    let ch = channels as usize;
    let frames = interleaved.len() / ch;
    let mut out = Vec::with_capacity(frames);
    for f in 0..frames {
        let base = f * ch;
        let sum: f32 = interleaved[base..base + ch].iter().copied().sum();
        out.push(sum / ch as f32);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downmix_averages_channels() {
        // stereo: [L0,R0, L1,R1] -> [(L0+R0)/2, (L1+R1)/2]
        let stereo = [1.0, 3.0, -1.0, 1.0];
        assert_eq!(downmix_to_mono(&stereo, 2), vec![2.0, 0.0]);
        // mono passthrough
        assert_eq!(downmix_to_mono(&[0.5, 0.25], 1), vec![0.5, 0.25]);
    }
}
