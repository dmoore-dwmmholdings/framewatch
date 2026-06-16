//! Extra engine-path coverage: transition_start, fps_cap dropping, all-black
//! frames, and the `max_active_ms` keyframe for sustained activity.

mod common;
use common::*;
use framewatch::{Config, Engine, EventKind, SystemClock, Target};

fn cfg() -> Config {
    Config::builder()
        .target(Target::ByExe("x.exe".into()))
        .build()
        .unwrap()
}

#[test]
fn transition_start_emitted_when_enabled() {
    let mut c = cfg();
    c.emit_transition_start = true;
    c.settle_ms = 100;
    let mut e = Engine::new(c, SystemClock);
    let base = base_instant();
    let (f0, t0) = frame_at(solid(0, 0, 0), base, 0);
    e.process(&f0, t0);
    let (f1, t1) = frame_at(solid(255, 255, 255), base, 50); // big meaningful change
    let kinds: Vec<_> = e.process(&f1, t1).iter().map(|x| x.kind()).collect();
    assert!(
        kinds.contains(&EventKind::TransitionStart),
        "expected transition_start: {kinds:?}"
    );
}

#[test]
fn fps_cap_drops_fast_frames() {
    let mut c = cfg();
    c.fps_cap = 5; // 200 ms minimum interval
    let mut e = Engine::new(c, SystemClock);
    let base = base_instant();
    let (f0, t0) = frame_at(solid(0, 0, 0), base, 0);
    e.process(&f0, t0);
    let (f1, t1) = frame_at(solid(255, 255, 255), base, 10); // only 10ms later
    assert!(e.process(&f1, t1).is_empty(), "fast frame is dropped");
    assert_eq!(e.frames_dropped(), 1);
}

#[test]
fn all_black_frame_warns_without_panicking() {
    let mut e = Engine::new(cfg(), SystemClock);
    let base = base_instant();
    let (f0, t0) = frame_at(solid(0, 0, 0), base, 0);
    assert_eq!(e.process(&f0, t0)[0].kind(), EventKind::Initial);
}

#[test]
fn max_active_keyframe_settles_sustained_activity() {
    let mut c = cfg();
    c.settle_ms = 100_000; // never settles via quiescence
    c.max_active_ms = 200; // ...but a keyframe fires after 200ms of activity
    c.fps_cap = 0; // process every frame
    let mut e = Engine::new(c, SystemClock);
    let base = base_instant();
    let (f0, t0) = frame_at(solid(0, 0, 0), base, 0);
    e.process(&f0, t0);
    let mut kinds = Vec::new();
    for i in 1..=10u64 {
        let v = (i * 20) as u8;
        let (f, t) = frame_at(solid(v, v, v), base, i * 50);
        kinds.extend(e.process(&f, t).iter().map(|x| x.kind()));
    }
    assert!(
        kinds.contains(&EventKind::Settled),
        "sustained activity yields a keyframe settle: {kinds:?}"
    );
}
