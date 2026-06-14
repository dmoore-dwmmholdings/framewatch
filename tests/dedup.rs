//! Dedup scenario: a non-forced image emit whose frame is visually identical to
//! the previously-saved frame is suppressed, and `coalesced_frames` increments.

mod common;
use common::*;
use framewatch::{Config, Engine, EventKind, SystemClock, Target};

#[test]
fn identical_busy_end_frame_is_deduped() {
    let mut cfg = Config::builder()
        .target(Target::ByExe("Code.exe".into()))
        .settle_ms(100)
        .min_emit_interval_ms(50)
        .dedup_hamming(8)
        .build()
        .unwrap();
    cfg.volatility_window = 8;
    cfg.busy_rate_threshold = 0.5;
    cfg.rois.push(framewatch::RoiHint {
        kind: framewatch::RoiKind::Spinner,
        label: "spinner".into(),
        rect_norm: [0.0, 0.0, 0.0625, 0.111],
    });

    let mut engine = Engine::new(cfg, SystemClock);
    let base = base_instant();

    // Initial: pure background (spinner off == gray).
    let (f0, t0) = frame_at(solid(128, 128, 128), base, 0);
    let _ = engine.process(&f0, t0);

    // Oscillate the spinner (well past min_emit_interval so dedup, not the
    // interval floor, is what suppresses the BusyEnd image).
    for i in 1..=12u64 {
        let mut buf = solid(128, 128, 128);
        let v = if i % 2 == 1 { 255 } else { 0 };
        paint_rect(&mut buf, 0, 0, 20, 20, v, v, v);
        let (f, t) = frame_at(buf, base, i * 33);
        let _ = engine.process(&f, t);
    }

    // Spinner stops: pure background again (identical to the saved initial).
    let mut busy_end_meta = None;
    for i in 13..=24u64 {
        let (f, t) = frame_at(solid(128, 128, 128), base, i * 33);
        for ev in engine.process(&f, t) {
            if ev.kind() == EventKind::BusyEnd {
                busy_end_meta = Some((ev.image.is_some(), ev.meta.clone()));
            }
        }
    }

    let (had_image, meta) = busy_end_meta.expect("a busy_end event should be emitted");
    assert!(!had_image, "the busy_end image must be deduped away");
    assert!(
        meta.coalesced_frames > 0,
        "coalesced_frames should count collapsed animation frames, got {}",
        meta.coalesced_frames
    );
    let hamming = meta.change.hamming_to_prev_emit.expect("hamming recorded");
    assert!(hamming <= 8, "deduped because hamming {hamming} <= 8");
}
