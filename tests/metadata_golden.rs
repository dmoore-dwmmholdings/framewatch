//! Golden metadata test: pins the `timeline.jsonl` JSON contract so accidental
//! schema drift fails CI. Volatile fields (timestamps, hashes) are normalized.

mod common;
use common::*;
use framewatch::{Config, DirectorySink, Engine, Sink, SystemClock, Target};
use serde_json::{json, Value};

/// Replace fields that legitimately vary run-to-run (timestamps, hashes, exact
/// frame counts, prose) with stable placeholders, so the test pins the *schema*.
fn normalize(mut v: Value) -> Value {
    let obj = v.as_object_mut().unwrap();
    obj.insert("session_id".into(), json!("<sid>"));
    obj.insert("wall_time".into(), json!("<wall>"));
    obj.insert("elapsed_ms".into(), json!(0));
    obj.insert("timing".into(), json!("<timing>"));
    obj.insert("coalesced_frames".into(), json!(0));
    obj.insert("note".into(), json!("<note>"));
    if let Some(change) = obj.get_mut("change").and_then(|c| c.as_object_mut()) {
        if change.contains_key("dhash") {
            change.insert("dhash".into(), json!("<dhash>"));
        }
        if change.contains_key("hamming_to_prev_emit") {
            change.insert("hamming_to_prev_emit".into(), json!(0));
        }
        // area_ratio/bboxes/changed_tiles are content-derived and stable, keep them.
    }
    v
}

#[test]
fn timeline_schema_is_stable() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = Config::builder()
        .target(Target::ByExe("Code.exe".into()))
        .out_dir(tmp.path())
        .settle_ms(100)
        .build()
        .unwrap();

    let mut sink = DirectorySink::with_options(&cfg, base_wall(), "config").unwrap();
    let session_dir = sink.session().dir.clone();
    let mut engine = Engine::new(cfg, SystemClock);
    let base = base_instant();

    let (f0, t0) = frame_at(solid(128, 128, 128), base, 0);
    for ev in engine.process(&f0, t0) {
        sink.on_event(&ev).unwrap();
    }
    let mut changed = solid(128, 128, 128);
    paint_rect(&mut changed, 40, 40, 200, 100, 10, 200, 10);
    for i in 1..12u64 {
        let (f, t) = frame_at(changed.clone(), base, i * 33);
        for ev in engine.process(&f, t) {
            sink.on_event(&ev).unwrap();
        }
    }
    sink.flush().unwrap();

    let timeline = std::fs::read_to_string(session_dir.join("timeline.jsonl")).unwrap();
    let normalized: Vec<Value> = timeline
        .lines()
        .filter(|l| !l.is_empty())
        .map(|l| normalize(serde_json::from_str(l).unwrap()))
        .collect();

    let expected = [
        json!({
            "session_id": "<sid>",
            "seq": 0,
            "id": "f000000",
            "kind": "initial",
            "wall_time": "<wall>",
            "elapsed_ms": 0,
            "image": "frames/000000_initial.png",
            "window": {
                "title": "Build — myapp — Visual Studio Code",
                "exe": "Code.exe",
                "class": "Chrome_WidgetWin_1",
                "hwnd": 67890,
                "rect": [0, 0, 320, 180],
                "dpi": 96,
                "foreground": true
            },
            "change": {
                "changed_tiles": 0,
                "tile_grid": [32, 18],
                "area_ratio": 0.0,
                "bboxes": [],
                "dhash": "<dhash>"
            },
            "busy": { "active": false, "regions": [] },
            "timing": "<timing>",
            "coalesced_frames": 0,
            "note": "<note>"
        }),
        json!({
            "session_id": "<sid>",
            "seq": 1,
            "id": "f000001",
            "kind": "settled",
            "wall_time": "<wall>",
            "elapsed_ms": 0,
            "image": "frames/000001_settled.png",
            "window": {
                "title": "Build — myapp — Visual Studio Code",
                "exe": "Code.exe",
                "class": "Chrome_WidgetWin_1",
                "hwnd": 67890,
                "rect": [0, 0, 320, 180],
                "dpi": 96,
                "foreground": true
            },
            "change": {
                "changed_tiles": 0,
                "tile_grid": [32, 18],
                "area_ratio": 0.0,
                "bboxes": [],
                "dhash": "<dhash>",
                "hamming_to_prev_emit": 0
            },
            "busy": { "active": false, "regions": [] },
            "timing": "<timing>",
            "coalesced_frames": 0,
            "note": "<note>"
        }),
    ];

    assert_eq!(
        normalized.len(),
        expected.len(),
        "event count: {normalized:#?}"
    );
    for (got, want) in normalized.iter().zip(expected.iter()) {
        assert_eq!(got, want);
    }
}
