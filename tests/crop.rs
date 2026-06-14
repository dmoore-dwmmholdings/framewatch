//! Tests for the `--roi`/crop feature: `RawFrame::crop` and end-to-end cropping
//! through the watch driver.

mod common;
use common::*;
use framewatch::{ChannelSink, Config, MockBackend, Rect, Target};
use std::time::Duration;

#[test]
fn raw_frame_crop_dims_and_pixels() {
    let mut buf = solid(0, 0, 0);
    paint_rect(&mut buf, 100, 50, 40, 30, 10, 20, 200); // BGRA block
    let (frame, _t) = frame_at(buf, base_instant(), 0);

    let cropped = frame.crop(Rect::new(100, 50, 40, 30));
    assert_eq!((cropped.width, cropped.height), (40, 30));
    assert_eq!(cropped.stride, 40 * 4);
    // Top-left of the crop is the painted block color.
    let (b, g, r, _a) = cropped.pixel(0, 0);
    assert_eq!((b, g, r), (10, 20, 200));
}

#[test]
fn crop_clamps_and_handles_out_of_bounds() {
    let (frame, _t) = frame_at(solid(50, 50, 50), base_instant(), 0);

    // Extends past the edge -> clamped to the remaining area.
    let c = frame.crop(Rect::new(300, 170, 1000, 1000));
    assert_eq!((c.width, c.height), (W - 300, H - 170));

    // Fully outside -> returned unchanged.
    let c2 = frame.crop(Rect::new(10_000, 10_000, 10, 10));
    assert_eq!((c2.width, c2.height), (W, H));
}

#[test]
fn watch_with_crop_produces_cropped_images() {
    let mut c = Config::builder()
        .target(Target::ByExe("game.exe".into()))
        .fps_cap(0)
        .build()
        .unwrap();
    c.stop_after_images = 1;
    c.crop = Some(Rect::new(20, 10, 120, 80));

    let frames: Vec<(u32, u32, Vec<u8>)> = (0..3).map(|_| (W, H, solid(128, 128, 128))).collect();
    let backend = MockBackend::from_bgra_frames(frames, Duration::from_millis(33), window_info());

    let (sink, rx) = ChannelSink::unbounded();
    framewatch::watch_with(c, backend, sink).unwrap();

    let events: Vec<_> = rx.try_iter().collect();
    let img = events[0].image.as_ref().expect("initial image present");
    assert_eq!(
        (img.width, img.height),
        (120, 80),
        "saved image is cropped to the ROI"
    );
}
