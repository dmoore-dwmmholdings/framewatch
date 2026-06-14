# framewatch — Integration Guide for Agents & Other Projects

This document tells another program (or AI agent) **exactly where to call
framewatch, how to call it, and how to read what it produces.**

There are two ways to use framewatch:

1. **As a CLI tool** — run the prebuilt binary, point it at a window, read the
   files it writes. *(Easiest; language-agnostic.)*
2. **As a Rust library** — embed the engine/sinks directly in a Rust app.

A machine-readable version of everything below lives in
[`dist/framewatch.json`](../dist/framewatch.json). A real example of the output
lives in [`dist/sample-session/`](../dist/sample-session).

---

## 0. Where the binary is

After a release build (`cargo build --release --features "cli wgc gui"`), the
binary is copied to:

```
dist/framewatch.exe
```

Absolute path on this machine:

```
C:\Coding\random-ideas\framewatch\dist\framewatch.exe
```

Verify it:

```sh
dist\framewatch.exe --version      # -> framewatch 0.1.0
```

> **Platform.** Live capture is Windows-only (Graphics Capture API), compiled in
> via the `wgc` feature. `windows` and `watch` need a real window to capture.
> On non-Windows builds these commands return a clear "requires the wgc feature"
> error; the library engine still works everywhere.

---

## 1. CLI usage

### 1.1 List capturable windows

```sh
dist\framewatch.exe windows
```

Prints a table to **stdout**: `HWND`, `EXE`, `TITLE`. Use it to discover a target
(by title text, exe name, or the numeric HWND).

```
HWND       EXE                      TITLE
1647775574 WindowsTerminal.exe      Windows PowerShell
134338     Claude.exe               Claude
...
```

### 1.2 Watch a window (the main command)

```sh
dist\framewatch.exe watch --title "Visual Studio Code" --out ./.framewatch
```

Selectors (choose one):

| Flag | Meaning |
|---|---|
| `--title <regex>` | match window title against a regular expression |
| `--exe <name>` | match executable basename, e.g. `Code.exe` (case-insensitive) |
| `--hwnd <id>` | match the exact numeric window handle |

Common knobs (all optional; sensible defaults):

| Flag | Default | Meaning |
|---|---|---|
| `--out <dir>` | `./.framewatch` | parent output directory |
| `--settle-ms <n>` | `350` | quiescence before declaring "settled" |
| `--value-sample-ms <n>` | `1000` | throttle for volatile-region samples |
| `--config <file>` | — | load a `framewatch.toml` base config (flags override it) |
| `-v`, `-vv` | — | log verbosity (or set `RUST_LOG`) |

**Behavior:** `watch` **blocks**. It prints the session directory it is writing to,
then runs until **Ctrl+C** or until the **target window closes**. It writes files
incrementally (each event is flushed immediately), so a session is readable while
it is still running.

On start it prints (to stdout):

```
framewatch: writing session to ./.framewatch/2026-06-14T06-22-17_Code
framewatch: press Ctrl+C to stop.
```

> **Parse that first line** to learn the session directory — or compute it
> yourself (see §2.1), or just watch `<out>/` for a new subdirectory.

### 1.3 GUI picker / ROI editor (for humans)

```sh
dist\framewatch.exe gui
```

Pick a window, drag-draw `Spinner` / `Volatile` / `Watch` / `Ignore` regions,
tune sliders, and **Save config & ROIs** (writes `framewatch.toml`). Hand-marking
the spinner means zero guesswork in detection. Then feed that `framewatch.toml`
to `watch --config`.

---

## 2. The output contract (what an agent reads)

A single run produces one **session directory**:

```
<out_dir>/<session_id>/
├─ session.json            # manifest: target, time range, config, ROI hints, counts
├─ timeline.jsonl          # one JSON event per line, chronological  ← the main feed
├─ README_FOR_AGENT.md     # this contract, in brief
└─ frames/
   ├─ 000000_initial.png
   ├─ 000003_settled.png
   └─ ...
```

### 2.1 `session_id`

Format: `"%Y-%m-%dT%H-%M-%S_<exe-stem>"`, e.g. `2026-06-14T06-22-17_Code`.

### 2.2 How to consume (recommended algorithm)

1. Read `session.json` for the target app, time range, config, and ROI hints.
2. Stream `timeline.jsonl` line by line (each line is a complete JSON object).
3. For most purposes, **only open images** for events with
   `kind == "settled"` or `kind == "busy_end"` — those are stable, meaningful
   states. Use the `note` and `coalesced_frames` of `busy_start` / `value_sample`
   events for timing context without opening images.
4. `coalesced_frames` tells you how much activity each saved image represents.

A whole "run the tests" workflow collapses to ~4 lines / 2 images instead of ~75
screenshots.

### 2.3 `timeline.jsonl` — event schema

One object per line. Example (`settled` event):

```jsonc
{
  "session_id": "2026-06-14T06-22-17_Code",
  "seq": 1,                                  // monotonic event index
  "id": "f000001",                           // stable id = "f" + zero-padded seq
  "kind": "settled",                         // see kinds below
  "wall_time": "2026-06-14T06:22:17.826Z",   // UTC timestamp of the frame
  "elapsed_ms": 198,                         // ms since session start
  "image": "frames/000001_settled.png",      // path relative to session dir, or null
  "window": {
    "title": "...", "exe": "Code.exe", "class": "Chrome_WidgetWin_1",
    "hwnd": 67890, "rect": [x, y, w, h], "dpi": 96, "foreground": true
  },
  "change": {
    "changed_tiles": 0, "tile_grid": [32, 18], "area_ratio": 0.0,
    "bboxes": [[x, y, w, h], ...],           // pixel boxes of changed clusters
    "dhash": "c02b2b2b2b23c001",             // present only when an image was saved
    "hamming_to_prev_emit": 24               // dHash distance to previous saved image
  },
  "busy": { "active": false, "regions": [{ "label": "test-runner", "active": false }] },
  "timing": { "since_prev_emit_ms": 198, "active_for_ms": 165, "quiescent_for_ms": 165 },
  "coalesced_frames": 5,                      // frames observed & collapsed since prev image
  "note": "Settled after 0.17s of activity in 0 region(s)."  // human-readable
}
```

Fields that are absent when not applicable are simply omitted (`image` is `null`
for image-less events; `dhash` / `hamming_to_prev_emit` / `timing.*` appear only
when known).

### 2.4 Event kinds

| `kind` | Image saved by default? | Meaning — what an agent should infer |
|---|---|---|
| `initial` | ✅ | First frame of the session (the starting state). |
| `busy_start` | — | A spinner/animation region started. Activity is underway. |
| `busy_end` | ✅ | The spinner stopped. `coalesced_frames` = frames collapsed. **Open this image.** |
| `settled` | ✅ | The window went quiet after activity — the "money frame". **Open this image.** |
| `value_sample` | — | A throttled sample of a volatile region (counter/progress). Usually note-only. |
| `transition_start` | — | Activity began (off by default). |
| `manual` | ✅ | A user/host-requested capture. |

### 2.5 `session.json` — manifest

```jsonc
{
  "session_id": "...",
  "tool": "framewatch 0.1.0",
  "target": { "title": "...", "exe": "...", "selected_via": "cli" },
  "started_at": "...Z",
  "ended_at": "...Z",                 // set on clean shutdown
  "config": { "settle_ms": 350, "tile_grid": [32,18], "dedup_hamming": 8, "value_sample_ms": 1000 },
  "roi_hints": [ { "kind": "spinner", "label": "...", "rect_norm": [x,y,w,h] } ],
  "counts": { "frames_observed": 5123, "images_saved": 64, "events": 92 },
  "timeline": "timeline.jsonl"
}
```

---

## 3. Optional: pre-annotate regions with `framewatch.toml`

Hints make detection faster and more accurate. ROIs are in **client-normalized**
coordinates (`0.0..1.0`), so they survive resizes/DPI changes.

```toml
target  = { title = "Visual Studio Code" }
out_dir = "./.framewatch"
settle_ms = 350
value_sample_ms = 1000

[[rois]]
kind = "spinner"            # spinner | volatile | watch | ignore
label = "test-runner-spinner"
rect_norm = [0.02, 0.94, 0.04, 0.05]

[[rois]]
kind = "ignore"
label = "clock"
rect_norm = [0.92, 0.0, 0.08, 0.03]
```

- `spinner` — busy indicator; its changes never trigger a save (only busy edges).
- `volatile` — fast-changing value; sampled on a throttle, not saved per change.
- `watch` — lowered threshold; always counts as meaningful.
- `ignore` — excluded from diffing entirely (clocks, cursors, the capture border).

Run it: `framewatch.exe watch --config framewatch.toml`.

---

## 4. Embedding as a Rust library

Add framewatch as a path/git dependency. The **core engine + sinks pull no
Windows or GUI deps**; opt into live capture with the `wgc` feature.

```toml
# Cargo.toml of the consuming project
[dependencies]
framewatch = { path = "C:/Coding/random-ideas/framewatch", default-features = false }

# For live Windows capture, also enable wgc:
# framewatch = { path = "...", default-features = false, features = ["wgc"] }
```

### 4.1 One-call convenience

```rust
use framewatch::{Config, Target, DirectorySink};

let config = Config::builder()
    .target(Target::ByTitleRegex("Visual Studio Code".into()))
    .out_dir("./.framewatch")
    .settle_ms(350)
    .build()?;

let sink = DirectorySink::new(&config)?;     // writes the session directory
framewatch::watch(config, sink)?;            // blocks until window closes (needs `wgc`)
```

### 4.2 Drive the loop yourself / use a custom sink

```rust
use framewatch::{Config, Target, Engine, CaptureBackend, ControlFlow, Sink, ChannelSink, SystemClock};

let config = Config::builder().target(Target::ByExe("Code.exe".into())).build()?;
let mut engine  = Engine::new(config.clone(), SystemClock);
let mut backend = framewatch::default_backend(&config)?;   // WGC on Windows + wgc
let (mut sink, rx) = ChannelSink::unbounded();              // receive events in your app

backend.run(&mut |frame| {
    for event in engine.process(&frame, frame.captured_at) {
        sink.on_event(&event).ok();      // or inspect `event.meta` / `event.image` directly
    }
    ControlFlow::Continue
})?;
```

### 4.3 Key public API

| Item | Purpose |
|---|---|
| `framewatch::enumerate_windows()` | list capturable windows (`Vec<WindowInfo>`) |
| `framewatch::watch(config, sink)` | capture target into a sink until interrupted |
| `framewatch::default_backend(&config)` | platform capture backend (`Box<dyn CaptureBackend>`) |
| `framewatch::Engine` | pure `(state, RawFrame, now) -> events` state machine |
| `framewatch::Config` / `ConfigBuilder` | configuration (also `Config::from_toml_path`) |
| `framewatch::DirectorySink` / `ChannelSink` / `CompositeSink` | outputs |
| `framewatch::CaptureEvent` / `CaptureMeta` | the event + its serializable metadata |
| `framewatch::MockBackend` | replay frames/PNGs (tests, CI, demos) |

Full API docs: `cargo doc --open` (or docs.rs once published).

---

## 5. Quick reference card

```sh
# discover
dist\framewatch.exe windows

# capture (blocks; Ctrl+C to stop) — pick ONE selector
dist\framewatch.exe watch --title "<regex>" --out ./.framewatch
dist\framewatch.exe watch --exe   "Code.exe"
dist\framewatch.exe watch --hwnd  67890
dist\framewatch.exe watch --config framewatch.toml

# then read:  <out>/<session_id>/timeline.jsonl   (+ session.json, frames/*.png)
# open images only for kind == "settled" | "busy_end"
```
