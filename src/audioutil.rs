//! Pure, dependency-free audio helpers shared by the recording runtime
//! (microphone capture) and the bundled whisper transcriber: downmix to mono
//! and resample to whisper's required 16 kHz. Compiled only when one of those
//! features is on.

/// whisper.cpp consumes 16 kHz mono f32 PCM.
pub(crate) const WHISPER_SAMPLE_RATE: u32 = 16_000;

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

/// Resample mono f32 PCM from `from_hz` to `to_hz` by linear interpolation.
///
/// Cheap and dependency-free — good enough for speech-to-text (whisper itself
/// is robust to mild resampling artifacts). Returns the input unchanged when the
/// rates match. Returns empty for empty input or a zero source rate.
pub(crate) fn resample_linear(input: &[f32], from_hz: u32, to_hz: u32) -> Vec<f32> {
    if from_hz == to_hz || from_hz == 0 {
        return input.to_vec();
    }
    if input.is_empty() {
        return Vec::new();
    }
    // Number of output samples = len * to/from, rounded to nearest.
    let out_len = ((input.len() as u64 * to_hz as u64) / from_hz as u64) as usize;
    if out_len == 0 {
        return Vec::new();
    }
    let step = from_hz as f64 / to_hz as f64;
    let mut out = Vec::with_capacity(out_len);
    let last = input.len() - 1;
    for i in 0..out_len {
        let src = i as f64 * step;
        let i0 = src.floor() as usize;
        if i0 >= last {
            out.push(input[last]);
        } else {
            let frac = (src - i0 as f64) as f32;
            out.push(input[i0] * (1.0 - frac) + input[i0 + 1] * frac);
        }
    }
    out
}

/// Downmix to mono and resample to 16 kHz in one step.
pub(crate) fn to_mono_16k(interleaved: &[f32], channels: u16, sample_rate: u32) -> Vec<f32> {
    let mono = downmix_to_mono(interleaved, channels);
    resample_linear(&mono, sample_rate, WHISPER_SAMPLE_RATE)
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

    #[test]
    fn resample_scales_length() {
        let input = vec![0.0f32; 48_000]; // 1s @ 48k
        let out = resample_linear(&input, 48_000, 16_000);
        assert_eq!(out.len(), 16_000); // -> 1s @ 16k
                                       // identity when rates match
        assert_eq!(resample_linear(&input, 16_000, 16_000).len(), 48_000);
        // empty / zero-rate guards
        assert!(resample_linear(&[], 48_000, 16_000).is_empty());
        assert!(resample_linear(&input, 0, 16_000).len() == input.len());
    }

    #[test]
    fn resample_interpolates_endpoints() {
        // Upsample 2 -> 3 samples: endpoints preserved.
        let out = resample_linear(&[0.0, 1.0], 2, 3);
        assert_eq!(out.len(), 3);
        assert_eq!(out[0], 0.0);
    }
}
