//! Spinner scenario: a small oscillating region while the rest is static produces
//! `BusyStart` then `BusyEnd` + `Settled` — and *no* per-frame settles.

mod common;
use common::*;
use framewatch::{Config, Engine, EventKind, SystemClock, Target};

fn spinner_config() -> Config {
    let mut cfg = Config::builder()
        .target(Target::ByExe("Code.exe".into()))
        .settle_ms(100)
        // Small ring so busy edges are reached quickly in the test.
        .build()
        .unwrap();
    cfg.volatility_window = 8;
    cfg.busy_rate_threshold = 0.5;
    // A 2x2-tile spinner in the top-left corner.
    cfg.rois.push(framewatch::RoiHint {
        kind: framewatch::RoiKind::Spinner,
        label: "test-runner-spinner".into(),
        rect_norm: [0.0, 0.0, 0.0625, 0.111],
    });
    cfg
}

#[test]
fn spinner_collapses_to_busy_then_settled() {
    let mut engine = Engine::new(spinner_config(), SystemClock);
    let base = base_instant();
    let mut kinds: Vec<EventKind> = Vec::new();

    // Frame 0: initial, spinner "off" == background gray.
    let (f0, t0) = frame_at(solid(128, 128, 128), base, 0);
    kinds.extend(engine.process(&f0, t0).iter().map(|e| e.kind()));

    // Frames 1..=14: oscillate the spinner region only.
    for i in 1..=14u64 {
        let mut buf = solid(128, 128, 128);
        let v = if i % 2 == 1 { 255 } else { 0 };
        paint_rect(&mut buf, 0, 0, 20, 20, v, v, v);
        let (f, t) = frame_at(buf, base, i * 33);
        kinds.extend(engine.process(&f, t).iter().map(|e| e.kind()));
    }

    // Frames 15..=30: fully static background (spinner stopped).
    for i in 15..=30u64 {
        let (f, t) = frame_at(solid(128, 128, 128), base, i * 33);
        kinds.extend(engine.process(&f, t).iter().map(|e| e.kind()));
    }

    let busy_start = kinds.iter().filter(|k| **k == EventKind::BusyStart).count();
    let busy_end = kinds.iter().filter(|k| **k == EventKind::BusyEnd).count();
    let settled = kinds.iter().filter(|k| **k == EventKind::Settled).count();

    assert_eq!(busy_start, 1, "exactly one busy_start: {kinds:?}");
    assert_eq!(busy_end, 1, "exactly one busy_end: {kinds:?}");
    assert_eq!(settled, 1, "exactly one settle after busy: {kinds:?}");

    // Ordering: initial, then busy_start before busy_end before settled.
    let pos = |k: EventKind| kinds.iter().position(|x| *x == k).unwrap();
    assert_eq!(kinds[0], EventKind::Initial);
    assert!(pos(EventKind::BusyStart) < pos(EventKind::BusyEnd));
    assert!(pos(EventKind::BusyEnd) <= pos(EventKind::Settled));
}
