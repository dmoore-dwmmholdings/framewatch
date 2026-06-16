//! Opt-in automatic spinner detection (H1): with `auto_detect_spinners`, a small
//! oscillating region with *no* ROI hint collapses to `BusyStart` → `BusyEnd` +
//! `Settled`, just like a hinted spinner. With the flag off (default) it does not.

mod common;
use common::*;
use framewatch::{Config, Engine, EventKind, SystemClock, Target};

fn base_config(auto: bool) -> Config {
    let mut cfg = Config::builder()
        .target(Target::ByExe("Code.exe".into()))
        .settle_ms(100)
        .auto_detect_spinners(auto)
        .build()
        .unwrap();
    cfg.volatility_window = 8;
    cfg.busy_rate_threshold = 0.5;
    cfg
}

/// Drive: initial, 14 frames oscillating a small top-left block, then 16 static.
fn run(cfg: Config) -> Vec<EventKind> {
    let mut engine = Engine::new(cfg, SystemClock);
    let base = base_instant();
    let mut kinds = Vec::new();

    let (f0, t0) = frame_at(solid(128, 128, 128), base, 0);
    kinds.extend(engine.process(&f0, t0).iter().map(|e| e.kind()));

    for i in 1..=14u64 {
        let mut buf = solid(128, 128, 128);
        let v = if i % 2 == 1 { 255 } else { 0 };
        paint_rect(&mut buf, 0, 0, 20, 20, v, v, v); // ~2x2 tiles of a 32x18 grid
        let (f, t) = frame_at(buf, base, i * 33);
        kinds.extend(engine.process(&f, t).iter().map(|e| e.kind()));
    }
    for i in 15..=30u64 {
        let (f, t) = frame_at(solid(128, 128, 128), base, i * 33);
        kinds.extend(engine.process(&f, t).iter().map(|e| e.kind()));
    }
    kinds
}

#[test]
fn auto_spinner_collapses_to_busy_then_settled() {
    let kinds = run(base_config(true));
    let busy_start = kinds.iter().filter(|k| **k == EventKind::BusyStart).count();
    let busy_end = kinds.iter().filter(|k| **k == EventKind::BusyEnd).count();
    let settled = kinds.iter().filter(|k| **k == EventKind::Settled).count();

    assert_eq!(busy_start, 1, "exactly one busy_start: {kinds:?}");
    assert_eq!(busy_end, 1, "exactly one busy_end: {kinds:?}");
    assert!(settled >= 1, "settles after the spinner stops: {kinds:?}");

    let pos = |k: EventKind| kinds.iter().position(|x| *x == k).unwrap();
    assert_eq!(kinds[0], EventKind::Initial);
    assert!(pos(EventKind::BusyStart) < pos(EventKind::BusyEnd));
    assert!(pos(EventKind::BusyEnd) <= pos(EventKind::Settled));
}

#[test]
fn without_auto_detection_no_busy_edges() {
    let kinds = run(base_config(false));
    let busy = kinds
        .iter()
        .filter(|k| matches!(k, EventKind::BusyStart | EventKind::BusyEnd))
        .count();
    assert_eq!(
        busy, 0,
        "no auto busy edges when the flag is off: {kinds:?}"
    );
}
