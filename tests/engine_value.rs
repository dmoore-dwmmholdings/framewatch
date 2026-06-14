//! Volatile-value scenario: a region changing every frame is sampled on a
//! throttle (`value_sample_ms`), not once per frame, and never forces a settle.

mod common;
use common::*;
use framewatch::{Config, Engine, EventKind, SystemClock, Target};

fn value_config() -> Config {
    let mut cfg = Config::builder()
        .target(Target::ByExe("Code.exe".into()))
        .value_sample_ms(100)
        .settle_ms(100)
        .build()
        .unwrap();
    // A volatile counter region in the middle.
    cfg.rois.push(framewatch::RoiHint {
        kind: framewatch::RoiKind::Volatile,
        label: "counter".into(),
        rect_norm: [0.45, 0.45, 0.1, 0.1],
    });
    cfg
}

#[test]
fn volatile_region_is_throttled_not_per_frame() {
    let mut engine = Engine::new(value_config(), SystemClock);
    let base = base_instant();

    let mut value_samples = 0usize;
    let mut settles = 0usize;
    let mut changing_frames = 0usize;

    // Frame 0: initial.
    let (f0, t0) = frame_at(solid(20, 20, 20), base, 0);
    let _ = engine.process(&f0, t0);

    // 30 frames, the volatile region changes every frame (~33 ms apart -> ~1 s).
    for i in 1..=30u64 {
        let mut buf = solid(20, 20, 20);
        let v = (i.wrapping_mul(37) % 240) as u8 + 5;
        paint_rect(&mut buf, 144, 81, 32, 18, v, v, v);
        changing_frames += 1;
        let (f, t) = frame_at(buf, base, i * 33);
        for ev in engine.process(&f, t) {
            match ev.kind() {
                EventKind::ValueSample => {
                    value_samples += 1;
                    assert!(
                        ev.image.is_none(),
                        "value samples are image-less by default"
                    );
                }
                EventKind::Settled => settles += 1,
                _ => {}
            }
        }
    }

    assert!(
        (5..=15).contains(&value_samples),
        "throttled samples expected (~10), got {value_samples}"
    );
    assert!(
        value_samples < changing_frames,
        "must not sample every frame: {value_samples} vs {changing_frames}"
    );
    assert_eq!(settles, 0, "a volatile-only region must not force a settle");
}
