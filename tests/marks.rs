//! Marks: application labels landing in the timeline and captioning frames.

mod common;
use common::*;
use framewatch::mark::{append_to_timeline, notify_pending, MarkRecord, PENDING_FILE};
use framewatch::{Config, DirectorySink, Engine, Sink, SystemClock, Target};
use serde_json::Value;

fn config(out: &std::path::Path) -> Config {
    Config::builder()
        .target(Target::ByExe("Code.exe".into()))
        .out_dir(out)
        .settle_ms(100)
        .build()
        .unwrap()
}

/// Drive one initial frame, then a change that settles.
fn run(engine: &mut Engine, sink: &mut DirectorySink, base: std::time::Instant) {
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
}

fn timeline(dir: &std::path::Path) -> Vec<Value> {
    std::fs::read_to_string(dir.join("timeline.jsonl"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).expect("every timeline line is valid JSON"))
        .collect()
}

fn frame_names(dir: &std::path::Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir.join("frames"))
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn a_queued_mark_is_written_and_captions_the_next_frame() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = config(tmp.path());
    let mut sink = DirectorySink::with_options(&cfg, base_wall(), "config").unwrap();
    let dir = sink.session().dir.clone();
    let inbox = sink.marks();
    let mut engine = Engine::new(cfg, SystemClock);

    inbox.push(MarkRecord::new("before-checkout"));
    run(&mut engine, &mut sink, base_instant());
    sink.flush().unwrap();

    let lines = timeline(&dir);
    let marks: Vec<&Value> = lines.iter().filter(|v| v["kind"] == "mark").collect();
    assert_eq!(marks.len(), 1, "one mark line");
    assert_eq!(marks[0]["note"], "before-checkout");
    assert!(
        !marks[0]["session_id"].as_str().unwrap().is_empty(),
        "the sink stamps the session id"
    );

    // The mark line comes before the frame it captions.
    let mark_at = lines.iter().position(|v| v["kind"] == "mark").unwrap();
    assert_eq!(mark_at, 0);

    let first_frame = lines.iter().find(|v| v["kind"] != "mark").unwrap();
    assert_eq!(
        first_frame["marks_since_last_frame"],
        serde_json::json!(["before-checkout"])
    );

    assert!(
        frame_names(&dir)[0].contains("before-checkout"),
        "frames: {:?}",
        frame_names(&dir)
    );
}

#[test]
fn a_frame_with_no_mark_keeps_its_plain_name_and_omits_the_field() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = config(tmp.path());
    let mut sink = DirectorySink::with_options(&cfg, base_wall(), "config").unwrap();
    let dir = sink.session().dir.clone();
    let mut engine = Engine::new(cfg, SystemClock);

    run(&mut engine, &mut sink, base_instant());
    sink.flush().unwrap();

    for name in frame_names(&dir) {
        assert!(
            name.ends_with("_initial.png") || name.ends_with("_settled.png"),
            "unexpected frame name {name}"
        );
    }
    for line in timeline(&dir) {
        assert!(line.get("marks_since_last_frame").is_none());
    }
}

#[test]
fn several_marks_before_one_frame_all_land_but_do_not_name_it() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = config(tmp.path());
    let mut sink = DirectorySink::with_options(&cfg, base_wall(), "config").unwrap();
    let dir = sink.session().dir.clone();
    let inbox = sink.marks();
    let mut engine = Engine::new(cfg, SystemClock);

    inbox.push(MarkRecord::new("one"));
    inbox.push(MarkRecord::new("two"));
    run(&mut engine, &mut sink, base_instant());
    sink.flush().unwrap();

    let lines = timeline(&dir);
    assert_eq!(lines.iter().filter(|v| v["kind"] == "mark").count(), 2);

    let first_frame = lines.iter().find(|v| v["kind"] != "mark").unwrap();
    assert_eq!(
        first_frame["marks_since_last_frame"],
        serde_json::json!(["one", "two"])
    );
    // Ambiguous, so the frame keeps its plain name; the labels are in the JSON.
    assert!(frame_names(&dir)[0].ends_with("_initial.png"));
}

#[test]
fn a_mark_another_process_already_wrote_is_not_written_twice() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = config(tmp.path());
    let mut sink = DirectorySink::with_options(&cfg, base_wall(), "config").unwrap();
    let dir = sink.session().dir.clone();
    let mut engine = Engine::new(cfg, SystemClock);

    // Exactly what `framewatch mark` does from another process: append the
    // line, then notify. Nothing pushes to the inbox — the sink reads the
    // notification file itself.
    let record = MarkRecord::new("out-of-process");
    append_to_timeline(&dir, &record).unwrap();
    notify_pending(&dir, &record).unwrap();

    run(&mut engine, &mut sink, base_instant());
    sink.flush().unwrap();

    let lines = timeline(&dir);
    assert_eq!(
        lines.iter().filter(|v| v["kind"] == "mark").count(),
        1,
        "exactly one line for one mark"
    );
    let first_frame = lines.iter().find(|v| v["kind"] != "mark").unwrap();
    assert_eq!(
        first_frame["marks_since_last_frame"],
        serde_json::json!(["out-of-process"])
    );
    assert!(dir.join(PENDING_FILE).exists());
}

#[test]
fn appending_a_mark_while_the_sink_is_writing_keeps_every_line_readable() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = config(tmp.path());
    let mut sink = DirectorySink::with_options(&cfg, base_wall(), "config").unwrap();
    let dir = sink.session().dir.clone();
    let mut engine = Engine::new(cfg, SystemClock);

    // The sink holds `timeline.jsonl` open; another process appends to the same
    // file between frames. Every line must still parse.
    let base = base_instant();
    let (f0, t0) = frame_at(solid(128, 128, 128), base, 0);
    for ev in engine.process(&f0, t0) {
        sink.on_event(&ev).unwrap();
    }
    for i in 0..20 {
        append_to_timeline(&dir, &MarkRecord::new(format!("mark-{i}"))).unwrap();
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

    let lines = timeline(&dir);
    assert_eq!(lines.iter().filter(|v| v["kind"] == "mark").count(), 20);
    assert!(lines.iter().any(|v| v["kind"] == "settled"));
    for line in &lines {
        assert!(line["kind"].is_string(), "every line has a kind");
    }
}

#[test]
fn a_mark_that_slugs_to_nothing_leaves_the_frame_name_alone() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = config(tmp.path());
    let mut sink = DirectorySink::with_options(&cfg, base_wall(), "config").unwrap();
    let dir = sink.session().dir.clone();
    let inbox = sink.marks();
    let mut engine = Engine::new(cfg, SystemClock);

    inbox.push(MarkRecord::new("///"));
    run(&mut engine, &mut sink, base_instant());
    sink.flush().unwrap();

    assert!(frame_names(&dir)[0].ends_with("_initial.png"));
    let lines = timeline(&dir);
    let first_frame = lines.iter().find(|v| v["kind"] != "mark").unwrap();
    assert_eq!(
        first_frame["marks_since_last_frame"],
        serde_json::json!(["///"]),
        "the label is still recorded, only the filename declines it"
    );
}

#[test]
fn marks_queued_after_the_last_frame_are_still_written() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = config(tmp.path());
    let mut sink = DirectorySink::with_options(&cfg, base_wall(), "config").unwrap();
    let dir = sink.session().dir.clone();
    let inbox = sink.marks();
    let mut engine = Engine::new(cfg, SystemClock);

    run(&mut engine, &mut sink, base_instant());
    // The window never changes again — which is exactly when an app tends to
    // report the error that ended the run.
    inbox.push(MarkRecord::new("permission-denied"));
    sink.flush().unwrap();

    let lines = timeline(&dir);
    let last = lines.last().unwrap();
    assert_eq!(last["kind"], "mark");
    assert_eq!(last["note"], "permission-denied");
}

/// The race this design exists to remove: a mark written microseconds before a
/// frame must land on *that* frame, not the next one. A background poll could
/// not promise this; reading the file at frame time can.
#[test]
fn a_mark_written_immediately_before_a_frame_lands_on_that_frame() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = config(tmp.path());
    let mut sink = DirectorySink::with_options(&cfg, base_wall(), "config").unwrap();
    let dir = sink.session().dir.clone();
    let mut engine = Engine::new(cfg, SystemClock);
    let base = base_instant();

    // No sleep anywhere: write, then immediately capture.
    let record = MarkRecord::new("before-orders");
    append_to_timeline(&dir, &record).unwrap();
    notify_pending(&dir, &record).unwrap();

    let (f0, t0) = frame_at(solid(128, 128, 128), base, 0);
    for ev in engine.process(&f0, t0) {
        sink.on_event(&ev).unwrap();
    }
    sink.flush().unwrap();

    let lines = timeline(&dir);
    assert_eq!(
        lines.iter().filter(|v| v["kind"] == "mark").count(),
        1,
        "the notification must not become a second timeline line"
    );
    let frame = lines.iter().find(|v| v["kind"] != "mark").unwrap();
    assert_eq!(
        frame["marks_since_last_frame"],
        serde_json::json!(["before-orders"])
    );
    assert!(frame_names(&dir)[0].ends_with("_initial_before-orders.png"));
}

#[test]
fn a_labels_file_is_read_at_frame_time_too() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = config(tmp.path());
    let mut sink = DirectorySink::with_options(&cfg, base_wall(), "config").unwrap();
    let dir = sink.session().dir.clone();
    let labels = tmp.path().join("console.log");
    sink.tail_labels(&labels);
    let mut engine = Engine::new(cfg, SystemClock);
    let base = base_instant();

    // A plain line and one of the app's own JSON events.
    framewatch::mark::append_line(&labels, "ready").unwrap();
    let (f0, t0) = frame_at(solid(128, 128, 128), base, 0);
    for ev in engine.process(&f0, t0) {
        sink.on_event(&ev).unwrap();
    }

    framewatch::mark::append_line(&labels, r#"{"kind":"route","route":"/orders"}"#).unwrap();
    let mut changed = solid(128, 128, 128);
    paint_rect(&mut changed, 40, 40, 200, 100, 10, 200, 10);
    for i in 1..12u64 {
        let (f, t) = frame_at(changed.clone(), base, i * 33);
        for ev in engine.process(&f, t) {
            sink.on_event(&ev).unwrap();
        }
    }
    sink.flush().unwrap();

    let names = frame_names(&dir);
    assert!(names[0].ends_with("_initial_ready.png"), "{names:?}");
    assert!(names[1].ends_with("_settled_route.png"), "{names:?}");

    // The watcher owes these lines, and the JSON payload is kept whole.
    let lines = timeline(&dir);
    let marks: Vec<&Value> = lines.iter().filter(|v| v["kind"] == "mark").collect();
    assert_eq!(marks.len(), 2);
    assert_eq!(marks[1]["data"]["route"], "/orders");
    assert!(marks[1]["elapsed_ms"].is_number());
}

#[test]
fn a_labels_file_that_appears_after_the_watcher_started_is_still_followed() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = config(tmp.path());
    let mut sink = DirectorySink::with_options(&cfg, base_wall(), "config").unwrap();
    let dir = sink.session().dir.clone();
    // Nothing at this path yet — the app has not started writing.
    let labels = tmp.path().join("later.log");
    sink.tail_labels(&labels);
    let mut engine = Engine::new(cfg, SystemClock);

    framewatch::mark::append_line(&labels, "arrived-late").unwrap();
    run(&mut engine, &mut sink, base_instant());
    sink.flush().unwrap();

    assert!(timeline(&dir)
        .iter()
        .any(|v| v["kind"] == "mark" && v["note"] == "arrived-late"));
}

#[test]
fn a_half_written_line_waits_for_its_newline() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = config(tmp.path());
    let mut sink = DirectorySink::with_options(&cfg, base_wall(), "config").unwrap();
    let dir = sink.session().dir.clone();
    let labels = tmp.path().join("console.log");
    sink.tail_labels(&labels);
    let mut engine = Engine::new(cfg, SystemClock);
    let base = base_instant();

    // A writer caught mid-line.
    std::fs::write(&labels, "before-che").unwrap();
    let (f0, t0) = frame_at(solid(128, 128, 128), base, 0);
    for ev in engine.process(&f0, t0) {
        sink.on_event(&ev).unwrap();
    }
    assert!(
        timeline(&dir).iter().all(|v| v["kind"] != "mark"),
        "a fragment must not become a label"
    );

    // The rest arrives.
    std::fs::write(&labels, "before-checkout\n").unwrap();
    let mut changed = solid(128, 128, 128);
    paint_rect(&mut changed, 40, 40, 200, 100, 10, 200, 10);
    for i in 1..12u64 {
        let (f, t) = frame_at(changed.clone(), base, i * 33);
        for ev in engine.process(&f, t) {
            sink.on_event(&ev).unwrap();
        }
    }
    sink.flush().unwrap();

    let lines = timeline(&dir);
    let marks: Vec<&Value> = lines.iter().filter(|v| v["kind"] == "mark").collect();
    assert_eq!(marks.len(), 1);
    assert_eq!(marks[0]["note"], "before-checkout");
}
