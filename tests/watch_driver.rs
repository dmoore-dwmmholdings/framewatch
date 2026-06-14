//! Driver-level tests for the one-shot / auto-exit conditions, using the mock
//! backend so they run on any OS.

mod common;
use common::*;
use framewatch::{ChannelSink, Config, EventKind, MockBackend, Target};
use std::time::Duration;

fn cfg() -> Config {
    Config::builder()
        .target(Target::ByExe("game.exe".into()))
        .settle_ms(100)
        .fps_cap(0)
        .build()
        .unwrap()
}

#[test]
fn stop_after_images_one_shot() {
    let mut c = cfg();
    c.stop_after_images = 1;

    // Several frames available, but we should stop right after the first image.
    let frames: Vec<(u32, u32, Vec<u8>)> = (0..6).map(|_| (W, H, solid(128, 128, 128))).collect();
    let backend = MockBackend::from_bgra_frames(frames, Duration::from_millis(33), window_info());

    let (sink, rx) = ChannelSink::unbounded();
    framewatch::watch_with(c, backend, sink).unwrap();

    let events: Vec<_> = rx.try_iter().collect();
    let with_images = events.iter().filter(|e| e.image.is_some()).count();
    assert_eq!(with_images, 1, "exactly one image saved: {events:?}");
    assert_eq!(events[0].kind(), EventKind::Initial);
}

#[test]
fn stop_after_settled_one_shot() {
    let mut c = cfg();
    c.stop_after_settled = true;

    // initial, one change, then static -> settles; then more frames we must NOT reach.
    let mut frames: Vec<(u32, u32, Vec<u8>)> = vec![(W, H, solid(128, 128, 128))];
    let mut changed = solid(128, 128, 128);
    paint_rect(&mut changed, 40, 40, 200, 100, 10, 200, 10);
    for _ in 0..20 {
        frames.push((W, H, changed.clone()));
    }
    let backend = MockBackend::from_bgra_frames(frames, Duration::from_millis(33), window_info());

    let (sink, rx) = ChannelSink::unbounded();
    framewatch::watch_with(c, backend, sink).unwrap();

    let events: Vec<_> = rx.try_iter().collect();
    let settles = events
        .iter()
        .filter(|e| e.kind() == EventKind::Settled)
        .count();
    assert_eq!(settles, 1, "stops after the first settle: {events:?}");
    assert_eq!(
        events.last().unwrap().kind(),
        EventKind::Settled,
        "settled is the last event before stopping"
    );
}
