//! L1 (`dedup_forced`) and L2 (`Engine::set_session_id`) behaviours.

mod common;
use common::*;
use framewatch::{Config, Engine, SystemClock, Target};

fn cfg(dedup_forced: bool) -> Config {
    let mut c = Config::builder()
        .target(Target::ByExe("x.exe".into()))
        .build()
        .unwrap();
    c.dedup_forced = dedup_forced;
    c
}

#[test]
fn dedup_forced_suppresses_identical_money_frame() {
    let base = base_instant();
    let (f0, t0) = frame_at(solid(128, 128, 128), base, 0);
    let (f1, t1) = frame_at(solid(128, 128, 128), base, 100); // byte-identical

    // With dedup_forced, the second identical forced (Manual) emit saves no image
    // (the first is always kept).
    let mut e = Engine::new(cfg(true), SystemClock);
    let m0 = e.manual(&f0, t0);
    let m1 = e.manual(&f1, t1);
    assert!(m0.image.is_some(), "first forced frame is always saved");
    assert!(m1.image.is_none(), "identical forced frame is deduped");

    // Default (false) preserves the prior behaviour: both saved.
    let mut e = Engine::new(cfg(false), SystemClock);
    assert!(e.manual(&f0, t0).image.is_some());
    assert!(
        e.manual(&f1, t1).image.is_some(),
        "without dedup_forced both money-frames are saved"
    );
}

#[test]
fn engine_stamps_session_id_into_meta() {
    let base = base_instant();
    let (f0, t0) = frame_at(solid(10, 10, 10), base, 0);
    let mut e = Engine::new(cfg(false), SystemClock);
    e.set_session_id("sess-abc");
    let evs = e.process(&f0, t0);
    assert_eq!(evs[0].meta.session_id, "sess-abc");

    // Default engine leaves it empty (sink-populated).
    let mut e = Engine::new(cfg(false), SystemClock);
    let evs = e.process(&f0, t0);
    assert_eq!(evs[0].meta.session_id, "");
}
