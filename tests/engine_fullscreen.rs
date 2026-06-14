//! Sustained-activity ("fullscreen video") scenarios: a frame that changes on
//! every tick must still produce periodic captures (not zero, not one-per-frame),
//! and `fps_cap` must rate-limit processing.

mod common;
use common::*;
use framewatch::{Config, Engine, EventKind, SystemClock, Target};

/// A whole-frame change every tick never quiesces — without the keyframe it would
/// settle exactly zero times. With `max_active_ms` it produces periodic keyframes.
#[test]
fn sustained_change_produces_periodic_keyframes() {
    let cfg = Config::builder()
        .target(Target::ByExe("game.exe".into()))
        .settle_ms(100_000) // effectively never settles via quiescence
        .max_active_ms(150) // keyframe every ~150ms of sustained activity
        .fps_cap(0) // process every frame in this deterministic test
        .build()
        .unwrap();
    let mut engine = Engine::new(cfg, SystemClock);
    let base = base_instant();

    let mut settled = 0usize;
    let mut frames = 0usize;
    for i in 0..20u64 {
        // Repaint a large region with a per-frame color -> meaningful every frame.
        let mut buf = solid(10, 10, 10);
        let v = (i.wrapping_mul(53) % 240) as u8 + 5;
        paint_rect(&mut buf, 20, 20, 280, 140, v, v / 2, 255 - v);
        let (f, t) = frame_at(buf, base, i * 50);
        frames += 1;
        for ev in engine.process(&f, t) {
            if ev.kind() == EventKind::Settled {
                settled += 1;
            }
        }
    }

    assert!(
        settled >= 2,
        "sustained activity must yield periodic keyframes, got {settled}"
    );
    assert!(
        settled < frames,
        "must not capture every frame: {settled} settles vs {frames} frames"
    );
}

/// With the keyframe disabled, the same input settles zero times — this pins the
/// exact bug that was reported for fullscreen.
#[test]
fn sustained_change_without_keyframe_never_settles() {
    let cfg = Config::builder()
        .target(Target::ByExe("game.exe".into()))
        .settle_ms(100_000)
        .max_active_ms(0) // disabled -> reproduces the old behavior
        .fps_cap(0)
        .build()
        .unwrap();
    let mut engine = Engine::new(cfg, SystemClock);
    let base = base_instant();

    let mut settled = 0usize;
    for i in 0..20u64 {
        let mut buf = solid(10, 10, 10);
        let v = (i.wrapping_mul(53) % 240) as u8 + 5;
        paint_rect(&mut buf, 20, 20, 280, 140, v, v / 2, 255 - v);
        let (f, t) = frame_at(buf, base, i * 50);
        for ev in engine.process(&f, t) {
            if ev.kind() == EventKind::Settled {
                settled += 1;
            }
        }
    }
    assert_eq!(
        settled, 0,
        "without keyframe, constant change never settles"
    );
}

/// `fps_cap` drops frames arriving faster than the cap interval.
#[test]
fn fps_cap_rate_limits_processing() {
    let cfg = Config::builder()
        .target(Target::ByExe("game.exe".into()))
        .fps_cap(30) // ~33 ms minimum interval
        .build()
        .unwrap();
    let mut engine = Engine::new(cfg, SystemClock);
    let base = base_instant();

    // 30 frames at 10 ms apart -> well above 30 fps, so most must be dropped.
    for i in 0..30u64 {
        let (f, t) = frame_at(solid(50, 50, 50), base, i * 10);
        let _ = engine.process(&f, t);
    }

    assert!(
        engine.frames_dropped() >= 10,
        "fps_cap should drop fast frames, dropped {}",
        engine.frames_dropped()
    );
}
