//! Point-to-pixel arithmetic for backends whose window geometry is in points
//! (macOS) while frames are captured in device pixels.
//!
//! Kept platform-neutral so it is tested on every build.

#![cfg_attr(not(all(target_os = "macos", feature = "macos")), allow(dead_code))]

use crate::frame::BASE_DPI;

/// The largest backing scale accepted from the platform. Real displays report
/// 1.0, 2.0 or 3.0; anything far beyond that is a bad reading.
const MAX_SCALE: f64 = 8.0;

/// Turn a platform-reported point-to-pixel scale into one safe to size a
/// capture with. A missing (0), non-finite or implausible value falls back to
/// 1.0, which captures at point resolution as before rather than failing.
pub(crate) fn sanitize_scale(raw: f64) -> f64 {
    if raw.is_finite() && (1.0..=MAX_SCALE).contains(&raw) {
        raw
    } else {
        1.0
    }
}

/// The `WindowInfo::dpi` that reports `scale` captured pixels per point.
pub(crate) fn dpi_for_scale(scale: f64) -> u32 {
    (f64::from(BASE_DPI) * sanitize_scale(scale)).round() as u32
}

/// A length in points converted to device pixels, never zero for a non-zero
/// length.
pub(crate) fn to_pixels(points: u32, scale: f64) -> u32 {
    if points == 0 {
        return 0;
    }
    ((f64::from(points) * sanitize_scale(scale)).round() as u32).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retina_window_is_captured_at_twice_its_point_size() {
        // The DOCW-78 measurement: a 1512x913 pt window on a 3024x1964 px panel.
        assert_eq!(to_pixels(1512, 2.0), 3024);
        assert_eq!(to_pixels(913, 2.0), 1826);
        assert_eq!(dpi_for_scale(2.0), 192);
    }

    #[test]
    fn standard_display_keeps_point_size_and_96_dpi() {
        assert_eq!(to_pixels(1280, 1.0), 1280);
        assert_eq!(dpi_for_scale(1.0), 96);
    }

    #[test]
    fn fractional_scales_round_to_whole_pixels() {
        assert_eq!(to_pixels(101, 1.5), 152);
        assert_eq!(dpi_for_scale(1.5), 144);
    }

    #[test]
    fn bad_readings_fall_back_to_point_resolution() {
        for raw in [0.0, -2.0, 0.5, f64::NAN, f64::INFINITY, 64.0] {
            assert_eq!(sanitize_scale(raw), 1.0, "raw {raw}");
            assert_eq!(to_pixels(800, raw), 800, "raw {raw}");
            assert_eq!(dpi_for_scale(raw), 96, "raw {raw}");
        }
    }

    #[test]
    fn zero_length_stays_zero() {
        assert_eq!(to_pixels(0, 2.0), 0);
    }
}
