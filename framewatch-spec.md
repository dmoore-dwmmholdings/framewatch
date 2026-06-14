# framewatch — Specification & Implementation Guide

**Event-driven, change-triggered window capture that emits timestamped screenshots + metadata so an AI coding agent can reconstruct what happened without a continuous frame stream.**

> Working crate name: `framewatch`. Pick your final name before publishing and verify availability with `cargo search <name>` / on crates.io, then do a single find-and-replace across the repo. Other candidates: `framecue`, `winframe`, `framescribe`, `glance-rs`.

Version of this document: 1.0 — targeted at a single-pass ("one-shot") implementation by a capable Rust agent or developer.

---

## 1. Goals, non-goals, and the core idea

### 1.1 The core idea

Continuous screen capture is wasteful and floods an agent's context with near-duplicate frames. The opposite — on-demand "take a screenshot now" — misses everything that happened between requests. `framewatch` sits in the middle: it watches **one specific window**, and it only writes a frame to disk at **semantically meaningful moments**:

- the window **settles** after a burst of change (the final, important state),
- a **busy/loading indicator starts or stops** (a "spinner" region begins or ends animating),
- a **volatile region** (a counter, progress %, log tail) is **sampled** on a throttle instead of every tick,
- plus the **initial** frame and any **manual** trigger.

Everything else is collapsed. Between saved frames, a lightweight append-only **timeline** records what happened (how many frames were coalesced, how long the window was busy, which regions changed) so the agent can read a compact, timestamped story and only look at the images that matter.

Detection is **heuristic and runs in the hot path with no LLM call** — the user can pre-annotate where spinners and volatile values live via a GUI, which makes detection both faster and more accurate.

### 1.2 Goals

- **Single-window**, change-triggered capture on **Windows** using the Graphics Capture API (frames are delivered only when the window updates).
- **Smart suppression** of animation noise: spinners and rapidly-changing values do not cause a flood of saved frames.
- **Agent-ready artifacts**: PNGs + a machine-readable `timeline.jsonl` + a `session.json` manifest, written into a project directory.
- **Library-first**: a clean, embeddable Rust API (`framewatch::Engine`, `Sink` trait) so another app can drive or consume it. Plus a CLI.
- **GUI picker/annotator** (`framewatch gui`): select the target window, draw spinner/volatile/ignore/watch regions.
- **Pluggable sinks**: default `DirectorySink`; a `Sink` trait for custom destinations; an optional future LLM sink behind a feature flag.
- **Publishable to crates.io** with docs that build on docs.rs.

### 1.3 Non-goals (v0.x)

- No cross-platform capture backend in v0.1 (the *core engine* is platform-agnostic and compiles everywhere, but the capture backend is Windows-only — see §4.1). macOS/Linux backends are a documented future extension.
- No built-in LLM inference in the hot path. (An `llm` feature is reserved as an out-of-band sink.)
- No OCR or DOM/accessibility-tree extraction in v0.1 (listed in §13 as future work).
- Not a general screen recorder; it deliberately drops frames.

---

## 2. Prior art and why this is new

The primitives exist; the assembled product does not.

| Capability | Closest existing thing | Gap |
|---|---|---|
| Per-window capture, frame-on-change | [`windows-capture`](https://crates.io/crates/windows-capture) (Graphics Capture API) | Gives you raw frames; no semantics, no dedup, no artifacts |
| Native dirty-rectangle change data | [`dxgi-capture-rs`](https://crates.io/crates/dxgi-capture-rs) | Per-monitor, low-level; no spinner/value logic, no agent output |
| Cross-platform capture | [`xcap`](https://github.com/nashaofu/xcap), [`scap`](https://crates.io/crates/scap) | No change semantics at all |
| Perceptual-hash dedup | [`image_hasher`](https://crates.io/crates/image_hasher) (aHash/dHash/pHash) | A building block, not a pipeline |
| pHash/dHash GUI-transition dedup | Research (e.g. UI-Oceanus), closed systems (retrace) | Not packaged, not Windows-native, not agent-output-shaped |

No crate combines **change-triggered capture + transient-state (spinner/volatile) awareness + timestamped, agent-readable artifacts** in one importable package. That is what this builds.

---

## 3. High-level architecture

```
                         ┌───────────────────────────────────────────────────────────┐
                         │                       framewatch (lib)                      │
                         │                                                             │
  Target window  ─────►  │  ┌──────────────┐   RawFrame    ┌───────────────────────┐  │
  (Windows HWND)         │  │ CaptureBackend│ ───────────► │        Engine         │  │
                         │  │  (WGC / Mock) │  (BGRA + ts   │  ┌─────────────────┐  │  │
                         │  └──────────────┘   + WindowInfo)│  │ diff (tile SAD) │  │  │
                         │        ▲                         │  ├─────────────────┤  │  │
                         │        │ frame-on-change         │  │ dHash dedup     │  │  │
                         │        │                         │  ├─────────────────┤  │  │
                         │   GUI picks HWND                 │  │ volatility /    │  │  │
                         │   + ROI hints                    │  │ spinner detect  │  │  │
                         │                                  │  ├─────────────────┤  │  │
                         │                                  │  │ state machine + │  │  │
                         │                                  │  │ decision        │  │  │
                         │                                  │  └─────────────────┘  │  │
                         │                                  └──────────┬────────────┘  │
                         │                                             │ CaptureEvent  │
                         │                                  ┌──────────▼────────────┐  │
                         │                                  │   encode (PNG once)   │  │
                         │                                  └──────────┬────────────┘  │
                         │                          ┌──────────────────┼──────────────┐│
                         │                          ▼                  ▼              ▼│
                         │                  DirectorySink        ChannelSink   (custom)│
                         │                  frames/*.png         mpsc to host   Sink   │
                         │                  timeline.jsonl                              │
                         │                  session.json                                │
                         └───────────────────────────────────────────────────────────┘
```

Two hard architectural rules:

1. **The `Engine` is pure and backend-agnostic.** It is a function of `(state, RawFrame, now) → Vec<CaptureEvent>`. It does no I/O, no capture, no timing of its own (the clock is injected). This makes the hard part fully unit-testable on any OS, including Linux CI.
2. **All Windows-specific code is `#[cfg(windows)]`** and behind the `capture::windows` module and the `gui` feature. The core crate compiles and tests on Linux/macOS so docs.rs and CI work, and so a future macOS backend just implements the same `CaptureBackend` trait.

---

## 4. Component design

### 4.1 Capture backend

```rust
/// A source of frames for a single target window.
pub trait CaptureBackend {
    /// Begin capture. `on_frame` is invoked once per delivered frame.
    /// Implementations should deliver frames only when the window content
    /// changes (Windows Graphics Capture does this natively).
    fn run(&mut self, on_frame: &mut dyn FnMut(RawFrame) -> ControlFlow) -> Result<(), CaptureError>;
    fn stop(&mut self);
}

pub enum ControlFlow { Continue, Stop }
```

- **Windows implementation (`capture/windows/wgc.rs`, `#[cfg(windows)]`):** wraps [`windows-capture`](https://crates.io/crates/windows-capture) (current line `1.5`). You implement its `GraphicsCaptureApiHandler` trait; its callback hands you a frame backed by a Direct3D texture mapped to CPU as BGRA. Copy/borrow into a `RawFrame` (see §4.2). WGC delivers a frame only when the window repaints, so an idle window costs nothing.
- **Window enumeration (`capture/windows/enumerate.rs`):** `EnumWindows` + `IsWindowVisible` + `GetWindowTextW` + `GetWindowThreadProcessId` → `QueryFullProcessImageNameW` for the exe; filter out zero-size windows and DWM-cloaked ones (`DwmGetWindowAttribute(DWMWA_CLOAKED)`). Produces the list the GUI/CLI shows and resolves a `Target` to a concrete `HWND`.
- **Mock backend (`capture/mock.rs`, all platforms):** replays a `Vec<RawFrame>` (or a vec of decoded PNGs) at a controllable cadence. This is what CI and the `examples/embed.rs` use.

> **Dependency gotcha:** `windows-capture` pulls in its own version of the `windows` crate. If you also depend on `windows` directly for enumeration, **match its major/minor version** (or use `windows-capture`'s re-exports) to avoid duplicate, incompatible `windows::…` types. Check `cargo tree -p windows` after adding both.

### 4.2 Frame representation

```rust
pub struct RawFrame {
    /// BGRA8, top-down rows. Shared so we can hand it to the encoder without copying.
    pub buffer: std::sync::Arc<[u8]>,
    pub width: u32,
    pub height: u32,
    pub stride: u32,                 // bytes per row; may exceed width*4 (padding)
    pub captured_at: std::time::Instant,   // monotonic; used by the engine for timing
    pub wall_time: chrono::DateTime<chrono::Utc>, // for human/agent-facing timestamps
    pub window: WindowInfo,
}

pub struct WindowInfo {
    pub hwnd: isize,
    pub title: String,
    pub exe: String,                 // basename, e.g. "chrome.exe"
    pub class: String,               // window class, e.g. "Chrome_WidgetWin_1"
    pub rect: Rect,                  // window bounds in screen coords
    pub client_rect: Rect,           // client area; ROIs are stored relative to this
    pub dpi: u32,
    pub foreground: bool,
}

pub struct Rect { pub x: i32, pub y: i32, pub w: u32, pub h: u32 }
```

### 4.3 Detection pipeline

All steps operate on a cheap **working frame**: the full BGRA frame downsampled to a tile grid (default `32 × 18`). Each tile stores one byte of mean luminance. Downsampling is the only per-frame full-buffer pass; everything downstream is `O(tiles)`.

```rust
// detect/diff.rs
pub struct WorkingFrame {
    pub cols: u16,
    pub rows: u16,
    pub luma: Box<[u8]>,   // len = cols*rows, mean luminance per tile
}

impl WorkingFrame {
    pub fn from_raw(frame: &RawFrame, cols: u16, rows: u16) -> Self { /* box-average each tile */ }
}

pub struct TileDiff {
    pub changed: Box<[bool]>,      // per-tile changed mask (len = cols*rows)
    pub changed_count: u32,
    pub area_ratio: f32,           // changed_count / total tiles
    pub bboxes: Vec<Rect>,         // pixel bounding boxes of changed clusters (connected components)
}

/// A tile is "changed" if |luma_now - luma_prev| > tile_change_threshold.
pub fn diff(prev: &WorkingFrame, cur: &WorkingFrame, threshold: u8, ignore: &TileMask) -> TileDiff;
```

**Step A — Tile diff.** Compare current vs previous working frame, skipping tiles inside `Ignore` ROIs. Produce the changed mask, count, area ratio, and pixel bounding boxes (a simple 4-connected component pass over the tile mask, scaled back to pixels).

**Step B — Volatility / spinner detection (`detect/volatility.rs`).** Maintain a per-tile ring buffer of the last `volatility_window` (default 32) change flags. `change_rate[tile] = ones / window`. Classify regions:

- A **spinner / animation** is a *small, spatially-compact* cluster of tiles with **high** `change_rate` while the rest of the frame is static. ROI hints of kind `Spinner` force this classification (and define the region exactly).
- A **volatile value** region is one flagged `Volatile`, or auto-detected as high-rate but text-like/larger; treated as "sample on a throttle," not "save every change."
- `Watch` ROIs lower the change threshold and always count as meaningful.
- `Ignore` ROIs are excluded from diffing entirely (clocks, cursors, ads, the WGC capture border).

```rust
pub struct RegionState { pub label: String, pub kind: RoiKind, pub busy: bool, pub change_rate: f32 }

pub struct Volatility {
    // per-tile ring buffers + per-region rollups
}
impl Volatility {
    pub fn update(&mut self, diff: &TileDiff, rois: &RoiSet) -> Vec<RegionState>;
    /// True when a Spinner region's change_rate crosses busy_rate_threshold upward.
    pub fn busy_rising(&self) -> Vec<&str>;
    pub fn busy_falling(&self) -> Vec<&str>; // region went static for >= settle
}
```

**Step C — Perceptual-hash dedup (`detect/hash.rs`).** Before *saving an image*, compute a 64-bit **dHash** (gradient hash) of the working frame via [`image_hasher`](https://crates.io/crates/image_hasher) (`HashAlg::Gradient`, `hash_size(8,8)`). If `hamming(dhash, last_saved_dhash) <= dedup_hamming` (default 8) and the event is not "forced," **skip the image** (still log a timeline entry, increment `coalesced_frames`).

```rust
pub struct Hasher { /* wraps image_hasher::Hasher */ }
impl Hasher {
    pub fn hash(&self, wf: &WorkingFrame) -> ImgHash;      // 64-bit
}
pub fn hamming(a: &ImgHash, b: &ImgHash) -> u32;
```

### 4.4 State machine and decision logic (`engine.rs`)

```
            meaningful change                 settle_ms with no meaningful change & not busy
   ┌─────┐ ───────────────────►  ┌────────┐ ──────────────────────────────────────► ┌─────┐
   │Idle │                       │ Active │                                          │Idle │  (emits `settled`)
   └─────┘ ◄───────────────────  └────────┘                                          └─────┘
      │  spinner region starts        ▲ │ spinner stops (busy_end) ──► then settle ──► settled
      │  animating (busy_start)       │ │
      ▼                               │ ▼
   ┌──────┐  ──────────────────────────┘
   │ Busy │  (volatile regions sampled every value_sample_ms while here)
   └──────┘
```

The engine is a single method; the host loop calls it per frame:

```rust
impl Engine {
    /// Process one frame. Returns 0..n events to hand to the sink(s).
    /// `now` comes from an injected Clock so tests are deterministic.
    pub fn process(&mut self, frame: &RawFrame, now: Instant) -> smallvec::SmallVec<[CaptureEvent; 2]>;
}
```

Decision rules (in order), each frame:

1. **Initial:** if this is the first frame, emit `Initial` (forced, always an image).
2. Build `WorkingFrame`; compute `TileDiff` (excluding `Ignore`). Update `Volatility`.
3. Compute **meaningful change** = changed tiles *outside* spinner/volatile regions, with area ≥ `meaningful_area_ratio` (default 0.002). `Watch` regions use a lower threshold.
4. **Busy edges:** for each `busy_rising` region → emit `BusyStart` (timeline entry; image optional, default off). For each `busy_falling` → mark; the region is now static.
5. **Transition start (optional):** `Idle → Active` on meaningful change. Emit `TransitionStart` only if `emit_transition_start` (default false).
6. **Volatile sampling:** while any volatile region is active and `now - last_value_sample >= value_sample_ms`, emit `ValueSample` (image optional, default off — usually a timeline note is enough).
7. **Quiescence / settle:** if `Active` and there has been **no meaningful change and no busy** for `settle_ms` (default 350 ms) → transition to `Idle` and emit `Settled` (**forced image** — this is the money frame).
8. **Min interval & dedup:** never save two images closer than `min_emit_interval_ms` apart unless forced; apply dHash dedup (§4.3) to non-forced image emits.

The result: typically **one image when the window settles**, plus tiny begin/end markers around busy periods, plus throttled samples for live values — and a `coalesced_frames` count so the agent knows how much activity each saved frame represents.

### 4.5 Events, metadata, and the agent contract

```rust
pub enum EventKind { Initial, TransitionStart, BusyStart, BusyEnd, ValueSample, Settled, Manual }

pub struct CaptureEvent {
    pub meta: CaptureMeta,
    /// PNG (or chosen format) bytes + dims. None for image-less timeline-only events.
    pub image: Option<EncodedImage>,
}
```

The **metadata is the public contract** an agent reads. One `timeline.jsonl` line per event:

```jsonc
{
  "session_id": "2026-06-13T15-04-05_chrome",
  "seq": 42,
  "id": "f000042",
  "kind": "settled",                         // initial|transition_start|busy_start|busy_end|value_sample|settled|manual
  "wall_time": "2026-06-13T15:05:12.482Z",
  "elapsed_ms": 67482,                        // since session start (monotonic)
  "image": "frames/000042_settled.png",       // null for image-less events
  "window": {
    "title": "Build — myapp — Visual Studio Code",
    "exe": "Code.exe", "class": "Chrome_WidgetWin_1",
    "hwnd": 67890, "rect": [0,0,2560,1440], "dpi": 144, "foreground": true
  },
  "change": {
    "changed_tiles": 37, "tile_grid": [32,18], "area_ratio": 0.064,
    "bboxes": [[120,80,640,48],[300,400,900,520]],
    "dhash": "f0e1c2a39b5d7e60", "hamming_to_prev_emit": 24
  },
  "busy": { "active": false, "regions": [{ "label": "test-runner-spinner", "active": false }] },
  "timing": { "since_prev_emit_ms": 1840, "active_for_ms": 1180, "quiescent_for_ms": 350 },
  "coalesced_frames": 14,                     // frames observed & collapsed since previous emit
  "note": "Settled after 1.18s of activity in 2 regions (top toolbar, main content)."
}
```

And a `session.json` manifest written/updated at start and on shutdown:

```jsonc
{
  "session_id": "2026-06-13T15-04-05_chrome",
  "tool": "framewatch 0.1.0",
  "target": { "title": "...", "exe": "Code.exe", "class": "Chrome_WidgetWin_1", "selected_via": "gui" },
  "started_at": "2026-06-13T15:04:05.001Z",
  "ended_at": "2026-06-13T15:12:41.220Z",
  "config": { "settle_ms": 350, "tile_grid": [32,18], "dedup_hamming": 8, "value_sample_ms": 1000 },
  "roi_hints": [
    { "kind": "spinner", "label": "test-runner-spinner", "rect_norm": [0.02,0.94,0.04,0.05] },
    { "kind": "ignore",  "label": "clock",               "rect_norm": [0.92,0.0,0.08,0.03] }
  ],
  "counts": { "frames_observed": 5123, "images_saved": 64, "events": 92 },
  "timeline": "timeline.jsonl"
}
```

To make consumption foolproof, `DirectorySink` also drops a short **`README_FOR_AGENT.md`** in the session dir explaining: read `session.json`, stream `timeline.jsonl`, and open only the PNGs referenced by `kind:"settled"`/`busy_end` unless you need finer detail. (Template in Appendix E.)

### 4.6 Sinks

```rust
pub trait Sink: Send {
    fn on_event(&mut self, event: &CaptureEvent) -> Result<(), SinkError>;
    fn flush(&mut self) -> Result<(), SinkError> { Ok(()) }
}

pub struct EncodedImage { pub bytes: Vec<u8>, pub format: ImageFormat, pub width: u32, pub height: u32 }
```

- **`DirectorySink`** (`sink/directory.rs`): writes `frames/<seq>_<kind>.<ext>`, appends a line to `timeline.jsonl`, maintains `session.json`, enforces rotation (`max_frames` / `max_bytes`). Default output dir: `./.framewatch/<session_id>/` so it lands inside the project the agent is working in.
- **`ChannelSink`** (`sink/channel.rs`): forwards owned events to an `mpsc`/`crossbeam` `Sender` for an embedding app.
- **`CompositeSink`**: fan-out to a `Vec<Box<dyn Sink>>`.
- **`LlmSink`** (reserved, `feature = "llm"`): out-of-band — pushes images to a vision model on a background thread to produce `note` captions. Never in the capture hot path.

The engine encodes each image **once** (BGRA→RGBA→PNG) and passes `EncodedImage` to all sinks, so multiple sinks never re-encode.

### 4.7 GUI (`feature = "gui"`, `framewatch gui`)

Built with [`eframe`/`egui`](https://crates.io/crates/eframe) (current line `0.34`). One window, three panels:

- **Picker (left):** the enumerated window list (title + exe, refresh button). Selecting one sets the `Target` and starts a low-FPS live preview using the same `CaptureBackend`.
- **Preview + ROI editor (center):** the live frame drawn as an egui texture. The user **drag-draws rectangles** and assigns each a **kind** (`Watch` / `Spinner` / `Volatile` / `Ignore`) and a label. Rectangles are stored in **client-normalized coordinates** (`0.0..1.0` of the client rect) so they survive window resizes and DPI changes.
- **Config + actions (right):** sliders for `settle_ms`, tile sensitivity, `value_sample_ms`; output-dir picker; **Save config & ROIs** (writes `framewatch.toml` + ROIs, keyed by window class/exe so re-runs auto-load them); **Start watching** (spawns the engine in-process, or prints the equivalent `framewatch watch …` command).

The GUI is the fast path to good detection: hand-marking the spinner means zero guesswork and zero model calls.

---

## 5. Public API surface (library)

The embedding contract should be small. Minimum viable public API:

```rust
// Re-exported from lib.rs
pub use config::{Config, ConfigBuilder, Target, RoiHint, RoiKind, ImageOpts, Rotation};
pub use frame::{RawFrame, WindowInfo, Rect};
pub use event::{CaptureEvent, CaptureMeta, EventKind, EncodedImage, ImageFormat};
pub use engine::Engine;
pub use sink::{Sink, SinkError, DirectorySink, ChannelSink, CompositeSink};
pub use capture::{CaptureBackend, CaptureError, ControlFlow, MockBackend, enumerate_windows};
pub use clock::{Clock, SystemClock};
pub use error::Error;

/// One-call convenience: capture `target` into `out_dir` until interrupted.
pub fn watch(config: Config, sink: impl Sink) -> Result<(), Error>;

/// Construct the platform default capture backend for `config`
/// (the Windows Graphics Capture backend on Windows).
pub fn default_backend(config: &Config) -> Result<Box<dyn CaptureBackend>, Error>;
```

Idiomatic embedding example (this is `examples/embed.rs` against the mock backend so it runs in CI):

```rust
use framewatch::{Config, Target, DirectorySink, Engine, CaptureBackend, ControlFlow, SystemClock};

fn main() -> anyhow::Result<()> {
    let config = Config::builder()
        .target(Target::ByTitleRegex("Visual Studio Code".into()))
        .out_dir("./.framewatch")
        .settle_ms(350)
        .spinner_roi("test-runner", [0.02, 0.94, 0.04, 0.05])
        .ignore_roi("clock", [0.92, 0.0, 0.08, 0.03])
        .build()?;

    let mut engine = Engine::new(config.clone(), SystemClock);
    let mut sink = DirectorySink::new(&config)?;

    // This example replays fixture PNGs so it runs on any OS in CI.
    // On Windows, swap this line for `framewatch::default_backend(&config)?`
    // to capture the live target window instead — the loop below is identical.
    let mut backend = framewatch::MockBackend::from_pngs("tests/fixtures/*.png")?;

    backend.run(&mut |frame| {
        for event in engine.process(&frame, frame.captured_at) {
            sink.on_event(&event).ok();
        }
        ControlFlow::Continue
    })?;
    Ok(())
}
```

---

## 6. Configuration schema

```rust
pub struct Config {
    pub target: Target,                  // ByHwnd(isize) | ByTitleRegex(String) | ByExe(String)
    pub out_dir: std::path::PathBuf,     // default "./.framewatch"
    pub fps_cap: u32,                    // max frames/sec processed (default 30)
    pub min_emit_interval_ms: u64,       // floor between saved images (default 200)
    pub settle_ms: u64,                  // quiescence to declare "settled" (default 350)
    pub tile_grid: (u16, u16),           // (cols, rows) (default (32, 18))
    pub tile_change_threshold: u8,       // per-tile luma delta (default 12)
    pub meaningful_area_ratio: f32,      // min changed area to count as activity (default 0.002)
    pub dedup_hamming: u32,              // dHash distance for dedup (default 8)
    pub volatility_window: u16,          // frames in the change-rate window (default 32)
    pub busy_rate_threshold: f32,        // region change-rate to be "busy" (default 0.5)
    pub value_sample_ms: u64,            // throttle for volatile regions (default 1000)
    pub emit_transition_start: bool,     // default false
    pub save_image_for: SaveMask,        // kinds that get images (default: Initial|Settled|BusyEnd|Manual)
    pub image: ImageOpts,                // format (Png default), scale (1.0 default)
    pub rois: Vec<RoiHint>,
    pub rotation: Rotation,              // max_frames / max_bytes; default 5000 frames / 2 GiB
}

pub enum Target { ByHwnd(isize), ByTitleRegex(String), ByExe(String) }
pub enum RoiKind { Watch, Spinner, Volatile, Ignore }
pub struct RoiHint { pub kind: RoiKind, pub label: String, pub rect_norm: [f32; 4] } // x,y,w,h in 0..1 of client
```

Both a `framewatch.toml` (human/GUI-authored) and the builder API map onto `Config`. Defaults are chosen so that, out of the box, a typical app produces a handful of `settled` frames per workflow, not hundreds.

---

## 7. Crate layout, Cargo.toml, and feature flags

```
framewatch/
├─ Cargo.toml
├─ README.md
├─ CHANGELOG.md
├─ LICENSE-MIT
├─ LICENSE-APACHE
├─ .github/workflows/ci.yml
├─ src/
│  ├─ lib.rs              # public re-exports, crate docs, `watch()`, `default_backend()`
│  ├─ config.rs           # Config, ConfigBuilder, Target, RoiHint, serde, TOML load
│  ├─ frame.rs            # RawFrame, WindowInfo, Rect
│  ├─ error.rs            # Error, CaptureError, SinkError (thiserror)
│  ├─ clock.rs            # Clock trait, SystemClock, MockClock (cfg(test) or pub for embedders)
│  ├─ event.rs            # CaptureEvent, CaptureMeta, EventKind, EncodedImage, serde
│  ├─ session.rs          # Session id/paths, manifest read/write
│  ├─ engine.rs           # Engine: state machine + process()
│  ├─ detect/
│  │  ├─ mod.rs
│  │  ├─ diff.rs          # WorkingFrame, TileDiff
│  │  ├─ hash.rs          # dHash wrapper, hamming
│  │  ├─ volatility.rs    # per-tile/region temporal stats, busy edges
│  │  └─ roi.rs           # RoiSet, region<->tile mapping, TileMask
│  ├─ sink/
│  │  ├─ mod.rs           # Sink trait, EncodedImage, encode(), CompositeSink
│  │  ├─ directory.rs     # DirectorySink
│  │  └─ channel.rs       # ChannelSink
│  ├─ capture/
│  │  ├─ mod.rs           # CaptureBackend trait, ControlFlow, Target resolution, enumerate API
│  │  ├─ mock.rs          # MockBackend (all platforms)
│  │  └─ windows/         # #[cfg(windows)]
│  │     ├─ mod.rs
│  │     ├─ wgc.rs        # windows-capture backend
│  │     └─ enumerate.rs  # EnumWindows + window metadata
│  ├─ gui/                # feature "gui"
│  │  ├─ mod.rs
│  │  └─ app.rs           # eframe App: picker, preview, ROI editor
│  └─ bin/
│     └─ framewatch.rs    # feature "cli": clap CLI (subcommands: windows, watch, gui)
├─ examples/
│  └─ embed.rs            # library embedding against MockBackend
└─ tests/
   ├─ engine_static.rs
   ├─ engine_spinner.rs
   ├─ engine_value.rs
   ├─ dedup.rs
   ├─ sink_directory.rs
   └─ metadata_golden.rs
```

`Cargo.toml` (verify each version against crates.io at implementation time — the lines below reflect current releases as of June 2026):

```toml
[package]
name = "framewatch"
version = "0.1.0"
edition = "2021"
rust-version = "1.78"
license = "MIT OR Apache-2.0"
description = "Event-driven, change-triggered window capture that emits timestamped screenshots + metadata for AI agents."
repository = "https://github.com/youruser/framewatch"
readme = "README.md"
keywords = ["screen-capture", "screenshot", "windows", "agent", "automation"]
categories = ["multimedia", "os::windows-apis", "computer-vision"]

[features]
default = ["cli"]
cli = ["dep:clap", "dep:anyhow", "dep:tracing-subscriber"]
gui = ["dep:eframe", "dep:egui"]
jpeg = ["image/jpeg"]
webp = ["dep:webp"]
llm = []  # reserved: out-of-band vision-caption sink

[dependencies]
serde = { version = "1", features = ["derive"] }
serde_json = "1"
toml = "0.8"
image = { version = "0.25", default-features = false, features = ["png"] }
image_hasher = "3"
thiserror = "2"
tracing = "0.1"
chrono = { version = "0.4", features = ["serde"] }
crossbeam-channel = "0.5"
smallvec = "1"
regex = "1"
# CLI (optional)
clap = { version = "4", features = ["derive"], optional = true }
anyhow = { version = "1", optional = true }
tracing-subscriber = { version = "0.3", features = ["env-filter"], optional = true }
# GUI (optional)
eframe = { version = "0.34", optional = true }
egui = { version = "0.34", optional = true }
# Encoders (optional)
webp = { version = "0.3", optional = true }

[target.'cfg(windows)'.dependencies]
windows-capture = "1.5"
windows = { version = "0.58", features = [
  "Win32_Foundation",
  "Win32_UI_WindowsAndMessaging",
  "Win32_Graphics_Dwm",
  "Win32_Graphics_Gdi",
  "Win32_System_Threading",
] }

[dev-dependencies]
tempfile = "3"
# (MockClock and MockBackend live in-crate)

[package.metadata.docs.rs]
default-target = "x86_64-pc-windows-msvc"
targets = ["x86_64-pc-windows-msvc"]
features = ["gui"]
```

Feature design rationale: the **core library has no required Windows or GUI deps** — they're all `optional` or under `cfg(windows)`. An embedding app does `framewatch = { version = "0.1", default-features = false }` to get just the engine + sinks + WGC backend (no clap/egui). `cargo install framewatch` gets the CLI; `--features gui` adds the picker. docs.rs builds the Windows target so the WGC backend and GUI are documented.

---

## 8. Threading and performance

- **Capture thread** (owned by the WGC backend): receives a frame on repaint, builds the `RawFrame` (`Arc<[u8]>` BGRA), and calls the engine's `process`. Downsampling to the tile grid is the only full-buffer pass and is cheap (sample/average; consider SIMD later). Detection is `O(tiles)` — sub-millisecond for a 32×18 grid.
- **Encode/sink thread**: the engine sends `(Arc<RawFrame>, CaptureMeta, force)` over a **bounded** `crossbeam-channel` (cap ~8). PNG encoding (the expensive part) happens here, off the capture path. On backpressure, **coalesce**: drop pending non-forced frames, keep only the latest `settled`. Dropping is correct behavior here, not a failure.
- **Idle cost is ~zero** because WGC delivers no frames for a static window.
- **`fps_cap`** clamps processing rate for pathological high-FPS windows (videos/games) by skipping frames between the cap interval — the volatility detector will also tag such regions as volatile and stop them from forcing saves.
- Keep the last full-res `Arc<[u8]>` so a `Settled` decision can encode the exact frame that settled without re-capturing.

Target budget: < 2% CPU watching a normal IDE/browser workflow; a single saved PNG per settle costs one encode (~a few ms at 1080p).

---

## 9. Step-by-step implementation plan (milestones)

Ordered so each milestone is independently testable and the risky Windows/GUI parts come **after** the fully-testable core. An agent can execute these top to bottom.

### M0 — Scaffolding
- `cargo new --lib framewatch`; set up `Cargo.toml` (§7), dual licenses, `README.md` skeleton, `CHANGELOG.md`.
- CI (`.github/workflows/ci.yml`): jobs on `ubuntu-latest` (build/test/clippy/fmt of the **core**, default-features and `--no-default-features`) and `windows-latest` (build with `--features gui` + the WGC backend). Deny warnings.
- **Acceptance:** `cargo build` and `cargo clippy` pass on Linux with an empty lib.

### M1 — Core types (cross-platform)
- Implement `frame.rs`, `event.rs` (+ serde), `config.rs` (+ builder, TOML, defaults), `error.rs`, `clock.rs` (`Clock` trait, `SystemClock`, `MockClock`), `session.rs` (id format `"%Y-%m-%dT%H-%M-%S_<exe-stem>"`, path helpers).
- **Acceptance:** serde round-trip tests for `CaptureMeta`, `Config`, session manifest; `cargo test` green on Linux.

### M2 — Detection engine (cross-platform, headless) — the heart
- Implement `detect/diff.rs`, `detect/hash.rs`, `detect/roi.rs`, `detect/volatility.rs`, then `engine.rs` (state machine + `process`).
- Drive everything off the injected `Clock`.
- **Tests (synthetic `RawFrame`s built with the `image` crate + `MockClock`):**
  - *static*: N identical frames → exactly one `Initial`, nothing else.
  - *single change then static*: → exactly one `Settled` at `settle_ms` after the change.
  - *spinner*: a small oscillating region while the rest is static → `BusyStart` then `BusyEnd`+`Settled`; **no** per-frame settles.
  - *volatile value*: a region incrementing every frame → throttled `ValueSample`s at `value_sample_ms`, **not** one per frame.
  - *dedup*: visually-identical frames after activity → image suppressed, `coalesced_frames` increments.
- **Acceptance:** all scenario tests green on Linux CI; this proves the core logic without a screen.

### M3 — Sinks
- `sink/mod.rs` (`Sink`, `EncodedImage`, `encode()` BGRA→PNG via `image`, `CompositeSink`), `sink/directory.rs`, `sink/channel.rs`.
- **Tests:** drive the engine over a synthetic sequence into a `DirectorySink` pointed at a `tempfile::tempdir()`; assert the PNGs exist, `timeline.jsonl` parses line-by-line, `session.json` counts are correct.
- **Acceptance:** golden test of one full session's `timeline.jsonl` (normalize timestamps).

### M4 — Mock backend + embedding example
- `capture/mod.rs` (`CaptureBackend`, `ControlFlow`, `Target`, `enumerate_windows` trait surface), `capture/mock.rs` (replays decoded PNGs as frames), `examples/embed.rs`.
- **Acceptance:** `cargo run --example embed` produces a session dir from bundled fixture PNGs on Linux.

### M5 — Windows capture backend (`#[cfg(windows)]`)
- `capture/windows/enumerate.rs` (window list + metadata, cloaked/zero-size filtering, DPI, exe).
- `capture/windows/wgc.rs` (implement `GraphicsCaptureApiHandler` from `windows-capture`; map frame → `RawFrame`; resolve `Target` → `HWND`; handle window-closed/stop).
- Reconcile the `windows` crate version with `windows-capture` (`cargo tree -p windows`).
- **Acceptance (manual, on Windows):** `framewatch watch --title "Notepad"` saves a `settled` frame after you type and pause; saves only one frame while a spinner animates in a browser tab.

### M6 — CLI (`feature = "cli"`)
- `src/bin/framewatch.rs` with clap subcommands:
  - `framewatch windows` — list capturable windows (title, exe, hwnd).
  - `framewatch watch [--title <re> | --exe <name> | --hwnd <id>] [--config framewatch.toml] [--out <dir>] [knobs…]`.
  - `framewatch gui` — launches the GUI (errors with a helpful message if built without `--features gui`).
- Wire `tracing-subscriber` with `RUST_LOG`/`--verbose`.
- **Acceptance:** `framewatch windows` and `framewatch watch` work end-to-end on Windows; `--help` is clean.

### M7 — GUI (`feature = "gui"`)
- `gui/app.rs` eframe app: window picker (from M5 enumerate), live preview texture, ROI rectangle editor with kind+label, config sliders, save `framewatch.toml`+ROIs (keyed by class/exe), Start watching.
- Store/auto-load ROIs from a per-user config dir (`directories`/`dirs` crate or `%APPDATA%\framewatch\rois\<key>.json`).
- **Acceptance (manual):** select a running app, draw a spinner box + an ignore box over its clock, start watching, confirm the spinner no longer floods captures and the ignored region never triggers.

### M8 — Docs & polish
- `#![warn(missing_docs)]`; rustdoc on every public item; runnable doc examples (gate Windows-only ones with `# #[cfg(windows)]`).
- README: what/why, the prior-art gap, a quickstart, an animated GIF, the agent-consumption contract, and the embedding snippet.
- `CHANGELOG.md`, `deny(warnings)` in CI, `cargo deny`/`cargo audit` (optional) for license/advisory hygiene.
- **Acceptance:** `cargo doc --no-deps` clean; `cargo test --doc` green.

### M9 — Publish to crates.io
- `cargo publish --dry-run`; fix any packaging issues (ensure `examples/`, fixtures, and licenses are included; exclude large test assets via `exclude`).
- Confirm docs.rs metadata (§7) so the Windows backend + GUI render.
- Tag `v0.1.0`, `cargo publish`. Verify the docs.rs build succeeds for `x86_64-pc-windows-msvc`.
- **Acceptance:** crate installs (`cargo add framewatch`), embeds with `default-features = false`, and docs.rs shows the full API.

---

## 10. Testing strategy (why this can be one-shot)

The whole point of the core/backend split is that **the hard logic is testable without a GPU, a screen, or Windows**:

- **Deterministic time** via the `Clock` trait + `MockClock` — settle/throttle behavior is tested by advancing a fake clock, not by sleeping.
- **Synthetic frames** built with the `image` crate: helpers like `solid(w,h,color)`, `with_rect(base, rect, color)`, `spinner_frames(n)`, `counter_frames(n)` generate the BGRA buffers the engine consumes.
- **Scenario tests** (M2) encode the spec's behavior as assertions — these are the executable definition of "correct."
- **Golden metadata** tests pin the JSON contract so accidental schema drift fails CI.
- **MockBackend** integration test runs a full capture→engine→DirectorySink pipeline on Linux CI.
- **Windows backend & GUI**: covered by manual acceptance checks (M5/M7) plus a `windows-latest` CI job that at least compiles them.

Recommended extras: `proptest` for the diff/hamming functions; `criterion` benches for `WorkingFrame::from_raw` and `diff` to keep the hot path honest.

---

## 11. crates.io publishing checklist

- [ ] Final crate name confirmed available; repo find-and-replaced.
- [ ] `description`, `keywords` (≤5), `categories`, `repository`, `readme`, `license = "MIT OR Apache-2.0"`, both `LICENSE-*` files present.
- [ ] `rust-version` (MSRV) set and tested in CI.
- [ ] `default-features = false` path verified for embedders (no clap/egui pulled).
- [ ] `[package.metadata.docs.rs]` builds the Windows target with `gui`.
- [ ] `cargo publish --dry-run` clean; package size sane (`exclude` big fixtures).
- [ ] `cargo test`, `cargo test --doc`, `cargo clippy -- -D warnings`, `cargo fmt --check` all green.
- [ ] SemVer discipline: pre-1.0, breaking changes bump the minor; document in `CHANGELOG.md`.
- [ ] Tag and GitHub release; verify docs.rs build after publish.

---

## 12. Risks and mitigations

| Risk | Impact | Mitigation |
|---|---|---|
| `windows` vs `windows-capture` version skew (duplicate types) | Won't compile | Pin to the same major; prefer `windows-capture` re-exports; check `cargo tree`. |
| WGC capture border / privacy indicator | Yellow border appears in frames | Add it as a default `Ignore` ROI; on Win11 disable via API where allowed; document. |
| Minimized window doesn't render | No frames | Detect minimized/occluded via window state; warn the user; recommend keeping the window visible. |
| DRM/protected content | Black frames | Document as an OS limitation; surface a warning when frames are all-black. |
| ROI drift on resize/DPI change | Hints land on wrong pixels | Store ROIs in **client-normalized** coords; recompute tile masks on `client_rect` change. |
| High-FPS windows (video/games) | CPU/encoder pressure | `fps_cap` + volatility tagging suppress saves; bounded channel coalesces. |
| Spinner auto-detection false positives | Missed real changes | Default to **hinted** spinners (GUI); keep auto-detect conservative/opt-in. |
| Encoder backpressure | Memory growth | Bounded channel + coalesce-to-latest; never block the capture thread. |

---

## 13. Future extensions (post-1.0, keep the seams)

- **macOS backend** via `ScreenCaptureKit` and **Linux** via PipeWire/`scap`, implementing the same `CaptureBackend` — the engine is already portable.
- **OCR / accessibility tree** enrichment (UIAutomation on Windows) to put text, not just pixels, in the timeline.
- **`llm` sink** that captions `settled` frames out-of-band.
- **Native dirty-rects** path using `dxgi-capture-rs` to skip the diff entirely on monitors.
- **Video segments** (short clips) for busy periods instead of begin/end markers.
- **MCP server** wrapper so an agent can subscribe to the timeline live.

---

## 14. Appendices

### Appendix A — Engine decision pseudocode

```text
fn process(frame, now):
    events = []
    wf = WorkingFrame::from_raw(frame, grid)
    if first_frame:
        emit(Initial, force_image=true); prev = wf; return events

    diff = diff(prev, wf, threshold, ignore_mask)
    regions = volatility.update(diff, rois)

    for r in volatility.busy_rising():  emit(BusyStart, image=cfg.save_image_for)
    for r in volatility.busy_falling(): mark_region_static(r)

    meaningful = changed_tiles_outside(spinner|volatile) with area >= meaningful_area_ratio
                 (Watch regions use lower threshold)

    if state == Idle and meaningful:
        state = Active; active_start = now
        if cfg.emit_transition_start: emit(TransitionStart)

    if any_volatile_active and now - last_value_sample >= value_sample_ms:
        emit(ValueSample); last_value_sample = now

    if state == Active and no_meaningful_change_for(settle_ms) and not any_busy:
        state = Idle
        emit(Settled, force_image=true)   // the money frame

    prev = wf
    return events

fn emit(kind, force_image=false):
    save = cfg.save_image_for.contains(kind) or force_image
    if save and not force_image:
        if now - last_emit < min_emit_interval_ms: save = false
        if hamming(dhash(wf), last_saved_dhash) <= dedup_hamming: save = false; coalesced += 1
    meta = build_meta(kind, frame, diff, regions, timing, coalesced)
    image = if save { Some(encode(frame)) } else { None }
    if save { last_saved_dhash = dhash(wf); last_emit = now; coalesced = 0 }
    events.push(CaptureEvent { meta, image })
```

### Appendix B — Example `timeline.jsonl` (one workflow)

```jsonc
{"seq":0,"kind":"initial","elapsed_ms":0,"image":"frames/000000_initial.png","note":"Session start."}
{"seq":1,"kind":"busy_start","elapsed_ms":1200,"image":null,"busy":{"active":true,"regions":[{"label":"test-runner","active":true}]},"note":"Test runner started (spinner active)."}
{"seq":2,"kind":"busy_end","elapsed_ms":4830,"image":"frames/000002_busy_end.png","coalesced_frames":71,"note":"Spinner stopped after 3.63s; 71 animation frames collapsed."}
{"seq":3,"kind":"settled","elapsed_ms":5180,"image":"frames/000003_settled.png","change":{"area_ratio":0.21,"hamming_to_prev_emit":29},"note":"Settled: test results rendered in main panel."}
```

An agent reads this as: *started → ran tests for 3.6s (one image of the result) → UI settled (one image)* — four entries, two images, instead of ~75 screenshots.

### Appendix C — Example CLI session

```bash
# 1. See what can be captured
framewatch windows

# 2. Pick the window + mark spinner/ignore regions visually
framewatch gui            # select app, draw boxes, Save & Start

# 3. Or run headless against a saved config
framewatch watch --config framewatch.toml --out ./.framewatch

# 4. Or one-liner by title, default detection
framewatch watch --title "Visual Studio Code"
```

### Appendix D — Minimal `framewatch.toml`

```toml
target  = { title = "Visual Studio Code" }
out_dir = "./.framewatch"
settle_ms = 350
value_sample_ms = 1000

[[rois]]
kind = "spinner"
label = "test-runner-spinner"
rect_norm = [0.02, 0.94, 0.04, 0.05]

[[rois]]
kind = "ignore"
label = "clock"
rect_norm = [0.92, 0.0, 0.08, 0.03]
```

### Appendix E — `README_FOR_AGENT.md` (dropped into each session dir)

```md
# framewatch session

This directory is an automatically-captured, de-duplicated visual log of a single
application window. To understand what happened:

1. Read `session.json` — target app, time range, config, and region hints.
2. Stream `timeline.jsonl` (one JSON event per line, chronological). Each event has a
   `kind`, `elapsed_ms`, an optional `image` path, and a human `note`.
3. You usually only need to open images for events with `kind` = "settled" or "busy_end";
   those are stable, meaningful states. `coalesced_frames` tells you how much activity
   each image represents. Use "value_sample"/"busy_start" notes for timing without images.

Frames are PNGs under `frames/`. There is intentionally no continuous stream — the gaps
are quiescent or were collapsed as animation/noise.
```

---

*End of specification. Build M0→M9 in order; the core (M1–M4) is fully testable on any OS before you touch a single Windows API.*
