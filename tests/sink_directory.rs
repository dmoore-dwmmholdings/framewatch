//! Full pipeline test: drive the engine into a `DirectorySink` and check the
//! on-disk artifacts (PNGs, `timeline.jsonl`, `session.json`).

mod common;
use common::*;
use framewatch::{Config, DirectorySink, Engine, Sink, SystemClock, Target};

#[test]
fn directory_sink_writes_expected_artifacts() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = Config::builder()
        .target(Target::ByExe("Code.exe".into()))
        .out_dir(tmp.path())
        .settle_ms(100)
        .build()
        .unwrap();

    let started = base_wall();
    let mut sink = DirectorySink::with_options(&cfg, started, "config").unwrap();
    let session_dir = sink.session().dir.clone();
    let mut engine = Engine::new(cfg, SystemClock);

    let base = base_instant();

    // initial
    let (f0, t0) = frame_at(solid(128, 128, 128), base, 0);
    for ev in engine.process(&f0, t0) {
        sink.on_event(&ev).unwrap();
    }
    // meaningful change
    let mut changed = solid(128, 128, 128);
    paint_rect(&mut changed, 40, 40, 200, 100, 10, 200, 10);
    let (f1, t1) = frame_at(changed.clone(), base, 33);
    for ev in engine.process(&f1, t1) {
        sink.on_event(&ev).unwrap();
    }
    // settle
    for i in 2..12u64 {
        let (f, t) = frame_at(changed.clone(), base, i * 33);
        for ev in engine.process(&f, t) {
            sink.on_event(&ev).unwrap();
        }
    }
    sink.flush().unwrap();

    // Files exist.
    assert!(session_dir.join("README_FOR_AGENT.md").exists());
    assert!(session_dir.join("timeline.jsonl").exists());
    assert!(session_dir.join("session.json").exists());

    // Count saved PNGs.
    let frames_dir = session_dir.join("frames");
    let png_count = std::fs::read_dir(&frames_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().map(|x| x == "png").unwrap_or(false))
        .count();
    assert_eq!(png_count, 2, "initial + settled PNGs");

    // Timeline parses line-by-line.
    let timeline = std::fs::read_to_string(session_dir.join("timeline.jsonl")).unwrap();
    let lines: Vec<&str> = timeline.lines().filter(|l| !l.is_empty()).collect();
    assert_eq!(lines.len(), 2);
    for line in &lines {
        let meta: framewatch::CaptureMeta = serde_json::from_str(line).unwrap();
        assert!(!meta.session_id.is_empty());
        assert_eq!(meta.change.tile_grid, [32, 18]);
        assert!(meta.image.is_some(), "both events saved an image");
        let img_rel = meta.image.unwrap();
        assert!(session_dir.join(&img_rel).exists(), "{img_rel} exists");
    }

    // Manifest counts.
    let manifest_txt = std::fs::read_to_string(session_dir.join("session.json")).unwrap();
    let manifest: framewatch::session::SessionManifest =
        serde_json::from_str(&manifest_txt).unwrap();
    assert_eq!(manifest.counts.images_saved, 2);
    assert_eq!(manifest.counts.events, 2);
    assert!(manifest.ended_at.is_some());
    assert_eq!(manifest.config.tile_grid, [32, 18]);
}
