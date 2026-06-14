//! Static scenario: N identical frames produce exactly one `Initial` event.

mod common;
use common::*;
use framewatch::{Config, Engine, EventKind, SystemClock, Target};

#[test]
fn static_window_emits_only_initial() {
    let cfg = Config::builder()
        .target(Target::ByExe("Code.exe".into()))
        .build()
        .unwrap();
    let mut engine = Engine::new(cfg, SystemClock);

    let base = base_instant();
    let mut all = Vec::new();
    for i in 0..10u64 {
        let (frame, now) = frame_at(solid(128, 128, 128), base, i * 33);
        for ev in engine.process(&frame, now) {
            all.push(ev.kind());
        }
    }

    assert_eq!(all, [EventKind::Initial], "only the initial frame saves");
}

#[test]
fn single_change_then_static_settles_once() {
    let cfg = Config::builder()
        .target(Target::ByExe("Code.exe".into()))
        .settle_ms(100)
        .build()
        .unwrap();
    let mut engine = Engine::new(cfg, SystemClock);

    let base = base_instant();
    let mut kinds = Vec::new();

    // Frame 0: initial background.
    let (f0, t0) = frame_at(solid(128, 128, 128), base, 0);
    kinds.extend(engine.process(&f0, t0).iter().map(|e| e.kind()));

    // Frame 1: a large change (meaningful).
    let mut changed = solid(128, 128, 128);
    paint_rect(&mut changed, 40, 40, 200, 100, 10, 200, 10);
    let (f1, t1) = frame_at(changed.clone(), base, 33);
    kinds.extend(engine.process(&f1, t1).iter().map(|e| e.kind()));

    // Frames 2..: identical to frame 1 (static) → should settle after settle_ms.
    for i in 2..10u64 {
        let (f, t) = frame_at(changed.clone(), base, i * 33);
        kinds.extend(engine.process(&f, t).iter().map(|e| e.kind()));
    }

    assert_eq!(kinds.first(), Some(&EventKind::Initial));
    let settles = kinds.iter().filter(|k| **k == EventKind::Settled).count();
    assert_eq!(settles, 1, "exactly one settle: got {kinds:?}");
}
