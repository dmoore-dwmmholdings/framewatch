//! What the engine emits while a window never stops changing.
//!
//! `max_active_ms` exists so that sustained activity still yields frames
//! instead of none, and nothing pinned it. A consumer that waits for `Settled`
//! needs two guarantees from it, and both were only implied: that a window
//! changing continuously keeps producing events at that interval, and that the
//! events it produces can be told apart from a window that actually stopped.
//!
//! docwalk read the second one wrong and called a moving window still for five
//! seconds at a time (its DOCW-98). The distinguishing signal is already on the
//! event — `timing.quiescent_for_ms` — so these lock it in.

mod common;
use common::*;
use framewatch::{Config, Engine, EventKind, SystemClock, Target};

/// A block that alternates hard in one fixed place: an in-place animation,
/// which is also what a spinner and a video look like to a tile differ.
fn counter(t: u64) -> Vec<u8> {
    let mut buf = solid(128, 128, 128);
    let (b, g, r) = if (t / 33).is_multiple_of(2) {
        (0, 0, 0)
    } else {
        (255, 255, 255)
    };
    paint_rect(&mut buf, 140, 80, 40, 20, b, g, r);
    buf
}

/// Run 12 s of 30 fps frames through an engine and collect the settles.
fn settles(max_active_ms: u64, paint: impl Fn(u64) -> Vec<u8>) -> Vec<(u64, Option<u64>)> {
    let cfg = Config::builder()
        .target(Target::ByExe("Code.exe".into()))
        .settle_ms(600)
        .max_active_ms(max_active_ms)
        .build()
        .unwrap();
    let mut engine = Engine::new(cfg, SystemClock);
    let base = base_instant();
    let mut out = Vec::new();
    for i in 0..360u64 {
        let t = i * 33;
        let (frame, now) = frame_at(paint(t), base, t);
        for ev in engine.process(&frame, now) {
            if ev.kind() == EventKind::Settled {
                out.push((t, ev.meta.timing.quiescent_for_ms));
            }
        }
    }
    out
}

#[test]
fn sustained_motion_keeps_producing_settles_at_the_keyframe_interval() {
    for max_active_ms in [300u64, 500, 5000] {
        let settles = settles(max_active_ms, counter);
        assert!(
            settles.len() >= 2,
            "max_active_ms={max_active_ms} produced {} settles in 12 s; \
             a consumer waiting on these would see silence: {settles:?}",
            settles.len()
        );
        // Never longer than the interval plus a frame, so a consumer can put a
        // silence threshold above it and have silence mean something.
        let ceiling = max_active_ms + 100;
        let times: Vec<u64> = settles.iter().map(|(t, _)| *t).collect();
        for gap in times.windows(2) {
            assert!(
                gap[1] - gap[0] <= ceiling,
                "max_active_ms={max_active_ms} went {} ms without a settle: {times:?}",
                gap[1] - gap[0]
            );
        }
    }
}

#[test]
fn a_forced_keyframe_reports_no_quiet_time_and_a_real_settle_reports_one() {
    // The two reasons for a `Settled` share a kind, so the quiet time is the
    // only thing that separates them. While the window is moving it is zero.
    let moving = settles(300, counter);
    assert!(!moving.is_empty(), "no settles to check");
    for (t, quiescent) in &moving {
        assert_eq!(
            *quiescent,
            Some(0),
            "the window was moving at {t} ms, so this settle was forced and \
             must not look like a window that went quiet"
        );
    }

    // A window that goes quiet and stays quiet reports the quiet time it
    // earned, and then stops producing settles at all.
    let stopping = settles(300, |t| if t < 2000 { counter(t) } else { counter(1980) });
    let last = stopping.last().copied().expect("a final settle");
    assert!(
        last.0 >= 2000 && last.1.unwrap_or(0) > 0,
        "the settle after motion stopped should carry a quiet time: {stopping:?}"
    );
    assert!(
        stopping.iter().all(|(t, _)| *t <= last.0),
        "nothing should follow a window that stopped"
    );
}
