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

(i.e. `<repo-root>\dist\framewatch.exe` — resolve `dist/framewatch.exe` against
wherever you cloned/built this repository.)

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
| `--pid <id>` | match the window owned by this **process id** — exact, so back-to-back captures never latch onto a *stale* window from an earlier run of the same exe |

> **Doing repeated captures of the same exe?** Match by `--pid` (or use `shot
> --launch`, §1.3), not `--exe` — otherwise a previous still-alive window can win.

Common knobs (all optional; sensible defaults):

| Flag | Default | Meaning |
|---|---|---|
| `--out <dir>` | `./.framewatch` | parent output directory |
| `--settle-ms <n>` | `350` | quiescence before declaring "settled" |
| `--value-sample-ms <n>` | `1000` | throttle for volatile-region samples |
| `--roi <X,Y,W,H>` | — | **crop** capture + detection + output to a pixel region (clip host chrome / titlebar / menu bar). Coords are relative to the captured frame's top-left. |
| `--config <file>` | — | load a `framewatch.toml` base config (flags override it) |
| `-v`, `-vv` | — | log verbosity (or set `RUST_LOG`) |

`--roi` crops everything downstream: the saved PNGs are just that region, and
change-detection ignores motion outside it (so e.g. host window chrome won't
trigger captures). `window.rect` in the timeline still reports the full source
window. Example — clip a 1920×1040 guest area below a host titlebar/menu:

```sh
dist\framewatch.exe watch --title "QEMU" --roi 0,52,1920,1040 --until-settled --out ./.framewatch
```

**Lifecycle flags — use these to avoid coordinating two processes:**

| Flag | Meaning |
|---|---|
| `--wait <secs>` | poll/retry for the target window to **appear** instead of failing instantly — so it doesn't matter whether you launch the app or `watch` first |
| `--until-settled` | exit after the **first settled frame** (deterministic one-shot: wait for the UI to settle, grab it, done) |
| `--duration <secs>` | exit after N seconds (time-bounded one-shot; also stops an idle window) |
| `--frames <n>` | exit after N images have been saved |

**Behavior:** by default `watch` **blocks** until **Ctrl+C** or the **target
window closes**, writing files incrementally (each event flushed immediately) so a
session is readable while running. With any lifecycle flag it becomes a bounded /
one-shot run. Recommended agent one-liner (no launch-order races, exits on its own):

```sh
dist\framewatch.exe watch --title "My App" --wait 15 --until-settled --out ./.framewatch
```

> **Already-static screens:** a target with no motion (e.g. a quiescent login
> screen) produces only the `initial` frame and never a `settled` event — there's
> nothing to settle from, and that initial frame *is* the stable capture. So pair
> `--until-settled` with `--duration <secs>` as a fallback bound:
> `--wait 15 --until-settled --duration 8` exits on settle if the UI animates, or
> after 8s with the stable `initial` frame if it's already static.

On start it prints (to stdout):

```
framewatch: writing session to ./.framewatch/2026-06-14T06-22-17_Code
framewatch: press Ctrl+C to stop.
```

> **Parse that first line** to learn the session directory — or compute it
> yourself (see §2.1), or just watch `<out>/` for a new subdirectory.

### 1.3 One-shot to a single file (`shot`) — best for scripted/batch capture

`shot` collapses launch → wait-for-window → one settled frame → teardown into a
single command, writes the PNG to a **path you choose**, prints that path on
stdout, and uses the **exit code** to signal success. No session directory, no
timestamped glob, no two-process orchestration.

```sh
# Launch a held program, capture its window (matched by the launched PID), kill it:
dist\framewatch.exe shot --launch "game.exe --freecam --pos 1,2,3" --out-file shot.png --timeout 25

# Or against an already-running window (no launch):
dist\framewatch.exe shot --pid 41234 --out-file shot.png
dist\framewatch.exe shot --title "QEMU" --roi 0,52,1920,1040 --out-file guest.png
```

| Flag | Meaning |
|---|---|
| `--launch "<cmd>"` | spawn this program, capture **its** window (by PID), then kill it on exit. Whitespace-split; use `"..."` to group an argument. |
| `--out-file <path>` | exact PNG path to write (required, deterministic) |
| `--title/--exe/--hwnd/--pid` | selector when not using `--launch` |
| `--timeout <secs>` | overall budget to wait for the window + a settled frame (default 20) |
| `--settle-ms <n>` | quiescence to declare "settled" |
| `--roi <X,Y,W,H>` | crop (clip host chrome) |
| `--settle-best-effort` | if nothing settles before the timeout, write the latest frame anyway instead of failing |

**Contract:** on success `shot` prints the written path to **stdout** and exits
`0`. If no frame settles before `--timeout` (and `--settle-best-effort` is not
set), it writes nothing and exits **non-zero** (3) — so a script can branch on the
exit code instead of globbing. Capturing by the launched PID means repeated
captures never pick up a stale window.

> **`--launch` caveat:** the captured window must be **owned by the launched
> process**. A normal `game.exe` qualifies; "relauncher" apps that hand off to a
> separate process (e.g. Win11 Notepad/Calculator as Store apps) do not — for
> those, launch the app yourself and pass `--pid`. Statically-quiescent targets
> need `--settle-best-effort` (only `initial` is produced — see §2.2).

```sh
# scripted use:
if path=$(framewatch shot --launch "game.exe --scene city" --out-file city.png --timeout 30); then
  echo "captured $path"
else
  echo "never settled" >&2
fi
```

### 1.4 GUI picker / ROI editor (for humans)

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
    // rect is [x, y, WIDTH, HEIGHT] (NOT [left, top, right, bottom]); virtual-desktop
    // pixels, so x/y can be negative or large on multi-monitor setups.
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
# Path is relative to the consuming crate (adjust to where this repo lives),
# or use a git dependency once published.
framewatch = { path = "../framewatch", default-features = false }

# For live Windows capture, also enable wgc:
# framewatch = { path = "../framewatch", default-features = false, features = ["wgc"] }
# Once on crates.io:
# framewatch = { version = "0.1", default-features = false, features = ["wgc"] }
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

# one-shot, no launch-order coordination needed:
dist\framewatch.exe watch --title "My App" --wait 15 --until-settled --out ./.framewatch
dist\framewatch.exe watch --title "My App" --wait 15 --duration 8     --out ./.framewatch

# crop to a region (clip host chrome) — e.g. a guest area below a titlebar/menu:
dist\framewatch.exe watch --title "QEMU" --roi 0,52,1920,1040 --wait 15 --until-settled --duration 8 --out ./.framewatch

# one settled frame to a chosen file (launch + capture + kill; prints path, exit code):
dist\framewatch.exe shot --launch "game.exe --freecam" --out-file shot.png --timeout 25
dist\framewatch.exe shot --pid 41234 --out-file shot.png      # exact window, no stale match

# record a window to video while narrating, then bundle an LLM package (see §6):
dist\framewatch.exe record --title "My Game" --duration 60 --transcribe-cmd "whisper-cli -m m.bin -f {audio} -osrt -of {output}"

# then read:  <out>/<session_id>/timeline.jsonl   (+ session.json, frames/*.png)
# open images only for kind == "settled" | "busy_end"
# (for `record`: read <out>/<session_id>/PROMPT.md — see §6)
```

---

## 6. Recording packages (`record`)

`record` is the opposite of `watch`: instead of a deduped story of "money frames",
it **continuously records** one window to video while the user narrates into the
microphone, then transcribes the narration locally and writes a package an LLM
can act on. Build with `--features "wgc record"`; **`ffmpeg` must be on PATH**.

```sh
# Record until Ctrl+C (or --duration), transcribing with a local transcriber
# ({audio}/{output} are substituted):
dist\framewatch.exe record --title "My Game" --duration 60 \
  --transcribe-cmd "whisper-cli -m ggml-base.en.bin -f {audio} -osrt -of {output}" --out ./.framewatch

# Or skip transcription (video + audio only):
dist\framewatch.exe record --pid 41234 --no-transcribe
```

Selectors (`--title/--exe/--hwnd/--pid`), `--launch`, `--out`, `--roi`, `--wait`,
and `--duration` behave exactly as in `watch`/`shot`. Extra options: `--fps`
(default 30), `--mic <name>` (default input device), `--no-audio` (record
video-only), and the transcription choices above. Stop with **Ctrl+C** (the mp4
is finalized cleanly) or `--duration`. If no microphone is available, recording
falls back to video-only automatically.

### 6.1 Package layout

```
<out_dir>/<session_id>/
├─ PROMPT.md            # START HERE — the task prompt, transcript inline
├─ recording.mp4        # the window video (narration muxed in)
├─ audio.wav            # the raw microphone narration
├─ transcript.json      # { language?, duration_ms, segments: [{start_ms,end_ms,text}] }
├─ transcript.srt       # the same transcript as SubRip subtitles
├─ recording.json       # manifest (see below)
└─ README_FOR_AGENT.md
```

`session_id` has the same `%Y-%m-%dT%H-%M-%S_<exe-stem>` format as a capture session.

### 6.2 How to consume

1. **Read `PROMPT.md`.** It is self-contained: it embeds the full timestamped
   transcript inline, so a text-only model can follow the instructions with no
   tools.
2. **If you can see video,** also use `recording.mp4`. Every transcript segment's
   `start_ms`/`end_ms` is measured **from the start of the video**, so a spoken
   instruction maps to a specific on-screen moment. Ingest the mp4 directly, or
   pull a still frame at a timestamp (`-ss` is in **seconds** = `start_ms` / 1000):

   ```sh
   ffmpeg -ss 12.500 -i recording.mp4 -frames:v 1 frame.png   # the moment at start_ms 12500
   ```

### 6.3 `transcript.json` schema

```jsonc
{
  "language": "en",            // omitted if unknown
  "duration_ms": 64200,
  "segments": [
    { "start_ms": 1250, "end_ms": 4800, "text": "First, open the settings panel." },
    { "start_ms": 5000, "end_ms": 8200, "text": "See this dropdown — set it to manual." }
  ]
}
```

### 6.4 `recording.json` — manifest

```jsonc
{
  "session_id": "2026-06-15T00-30-31_game",
  "tool": "framewatch 0.4.0",
  "kind": "recording",                         // distinguishes this from a capture session.json
  "target": { "title": "My Game", "exe": "game.exe", "selected_via": "cli" },
  "started_at": "...Z", "ended_at": "...Z",
  "video": { "path": "recording.mp4", "container": "mp4", "codec": "h264",
             "fps": 30.0, "width": 1920, "height": 1080, "duration_ms": 64200 },
  // "audio" is omitted entirely for a video-only recording (no microphone):
  "audio": { "path": "audio.wav", "sample_rate": 48000, "channels": 1, "duration_ms": 64300 },
  "transcript": { "path": "transcript.json", "srt": "transcript.srt",
                  "engine": "command",         // "command" | "none"
                  "model": "whisper-cli -m … -f {audio} …", "segment_count": 2, "language": "en" },
  "artifacts": ["recording.mp4","audio.wav","transcript.json","transcript.srt",
                "recording.json","PROMPT.md","README_FOR_AGENT.md"]
}
```

### 6.5 Transcription engines

| Choice | Flag | Needs |
|---|---|---|
| External command | `--transcribe-cmd "<cmd>"` | any local transcriber on PATH (e.g. whisper.cpp's prebuilt `whisper-cli`) |
| None | `--no-transcribe` | — (empty transcript; video + audio only) |

framewatch bundles no speech-to-text engine — transcription is always done by
shelling out via `--transcribe-cmd`, so there's nothing to compile and any
transcriber works.

There is no microphone, or you don't want one? Recording is video-only then: it
warns and writes a package with no `audio.wav` and an empty transcript (and the
manifest omits the `audio` block). `--no-audio` opts out of mic capture explicitly.

The `--transcribe-cmd` template is whitespace-split (quotes group args). `{audio}`
is replaced with the WAV path and `{output}` with a framewatch-chosen output base
path; if neither placeholder appears, the WAV path is appended and framewatch reads
the command's **stdout**. The command must emit **framewatch transcript JSON**
(`{ "segments": [...] }`) or **SubRip (SRT)** — framewatch detects which.

A live recording (real window + mic + ffmpeg) is Windows-only; the transcript /
package / prompt code is pure and runs everywhere.
