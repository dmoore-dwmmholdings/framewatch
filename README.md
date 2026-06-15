# framewatch

**Event-driven, change-triggered window capture that emits timestamped screenshots + metadata so an AI coding agent can reconstruct what happened — without a continuous frame stream.**

[![CI](https://github.com/dmoore-dwmmholdings/framewatch/actions/workflows/ci.yml/badge.svg)](https://github.com/dmoore-dwmmholdings/framewatch/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/framewatch.svg)](https://crates.io/crates/framewatch)
[![docs.rs](https://img.shields.io/docsrs/framewatch)](https://docs.rs/framewatch)

---

## Why

Continuous screen capture floods an agent's context with near-duplicate frames.
On-demand "screenshot now" misses everything in between. `framewatch` sits in the
middle: it watches **one window** and only writes a frame to disk at
**semantically meaningful moments**:

- the window **settles** after a burst of change (the final, important state),
- a **spinner / busy indicator** starts or stops animating,
- a **volatile region** (counter, progress %, log tail) is **sampled on a throttle**,
- plus the **initial** frame and any **manual** trigger.

Everything else is collapsed. Between saved frames an append-only **timeline**
records what happened (how many frames were coalesced, how long the window was
busy, which regions changed), so an agent reads a compact, timestamped story and
opens only the images that matter.

Detection is **heuristic and runs in the hot path with no LLM call**. You can
pre-annotate where spinners and volatile values live via the GUI, which makes
detection both faster and more accurate.

## What makes it new

The primitives exist; the assembled product did not. No crate combined
**change-triggered capture + transient-state (spinner/volatile) awareness +
timestamped, agent-readable artifacts** in one importable package:

| Capability | Closest existing thing | Gap framewatch fills |
|---|---|---|
| Per-window capture, frame-on-change | [`windows-capture`](https://crates.io/crates/windows-capture) | Raw frames only; no semantics, dedup, or artifacts |
| Native dirty-rectangle data | [`dxgi-capture-rs`](https://crates.io/crates/dxgi-capture-rs) | Per-monitor, low-level; no spinner/value logic |
| Cross-platform capture | [`xcap`](https://github.com/nashaofu/xcap), [`scap`](https://crates.io/crates/scap) | No change semantics |
| Perceptual-hash dedup | [`image_hasher`](https://crates.io/crates/image_hasher) | A building block, not a pipeline |

> **Using framewatch from another project or agent?** See
> [`docs/AGENT_INTEGRATION.md`](docs/AGENT_INTEGRATION.md) for the exact
> where/how-to-call contract, and [`dist/framewatch.json`](dist/framewatch.json)
> for a machine-readable manifest.

## Install

```sh
# CLI (Windows live capture):
cargo install framewatch --features wgc        # add gui for the picker: --features "wgc gui"

# As a library (engine + sinks only, no clap/egui):
cargo add framewatch --no-default-features
```

> **Platforms.** The detection engine is platform-agnostic and compiles/tests
> everywhere. The live capture backend is **Windows-only** (Graphics Capture API)
> and is enabled with the `wgc` feature. macOS/Linux backends are a documented
> future extension — they just implement the same `CaptureBackend` trait.

## Quickstart (CLI)

```sh
# 1. See what can be captured
framewatch windows

# 2. Pick a window + mark spinner/ignore regions visually
framewatch gui

# 3. Or run headless by title
framewatch watch --title "Visual Studio Code" --out ./.framewatch

# 4. Or against a saved config
framewatch watch --config framewatch.toml
```

## Record & narrate → an LLM package (V4)

Sometimes you don't want a deduped story — you want to *show and tell*. The
`record` subcommand (build with `--features "wgc record"`, needs `ffmpeg` on
PATH) **continuously** records one window to video while you narrate into the
microphone, then transcribes the narration locally and bundles everything an LLM
needs to act on it:

```sh
# Record a window for 60s (or stop early with Ctrl+C) while you talk:
framewatch record --title "My Game" --duration 60 \
    --transcribe-cmd "whisper-cli -m ggml-base.en.bin -f {audio} -osrt -of {output}"
# Or with bundled whisper (build --features whisper):
framewatch record --title "My Game" --whisper-model ggml-base.en.bin
```

It writes a package directory:

```
recording.mp4         # the window video (your narration muxed in)
audio.wav             # the raw mic narration
transcript.json/.srt  # segments with start_ms/end_ms from video start
recording.json        # manifest
PROMPT.md             # the prompt to hand the model (transcript inline)
README_FOR_AGENT.md
```

Because every transcript segment is timestamped from the start of the video, a
model can correlate "click *this*" with the exact on-screen moment — ingesting
`recording.mp4` directly or pulling a frame with
`ffmpeg -ss <seconds> -i recording.mp4 -frames:v 1 frame.png`. See the
[recording-package contract](docs/AGENT_INTEGRATION.md#6-recording-packages-record).

> **No microphone?** Recording degrades gracefully — it warns and produces a
> **video-only** package (no transcript). Pass `--no-audio` to opt out of mic
> capture explicitly.
>
> **Transcription on Windows.** Use `--transcribe-cmd` with whisper.cpp's
> prebuilt `whisper-cli` (or `faster-whisper` / `openai-whisper`) — **no
> compilation needed**. The bundled `--features whisper` engine builds on
> **Linux/macOS**; on Windows it's currently blocked by an upstream
> [`whisper-rs`](https://crates.io/crates/whisper-rs) build bug (it passes the
> MSVC-only `/utf-8` flag to GNU toolchains, and its log-level enum mismatches
> bindgen's output under MSVC), so prefer `--transcribe-cmd` there.

## The agent-consumption contract

A session directory (`./.framewatch/<session_id>/`) contains:

```
frames/000000_initial.png
frames/000003_settled.png
timeline.jsonl          # one JSON event per line, chronological
session.json            # manifest: target, time range, config, ROI hints, counts
README_FOR_AGENT.md     # how to read this directory
```

An agent should: read `session.json`, stream `timeline.jsonl`, and open only the
PNGs referenced by `kind:"settled"` / `kind:"busy_end"` unless it needs finer
detail. `coalesced_frames` tells it how much activity each saved image represents.

A whole "run tests" workflow collapses to ~4 timeline entries and 2 images
instead of ~75 screenshots:

```jsonc
{"seq":0,"kind":"initial","elapsed_ms":0,"image":"frames/000000_initial.png","note":"Session start."}
{"seq":1,"kind":"busy_start","elapsed_ms":1200,"image":null,"note":"Test runner started (spinner active)."}
{"seq":2,"kind":"busy_end","elapsed_ms":4830,"image":"frames/000002_busy_end.png","coalesced_frames":71,"note":"Spinner stopped after 3.63s; 71 animation frames collapsed."}
{"seq":3,"kind":"settled","elapsed_ms":5180,"image":"frames/000003_settled.png","note":"Settled: test results rendered."}
```

## Embedding

```rust
use framewatch::{Config, Target, DirectorySink, Engine, CaptureBackend, ControlFlow, Sink, SystemClock};

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

    // On Windows (built with `--features wgc`), use the live backend:
    // let mut backend = framewatch::default_backend(&config)?;
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

The `Engine` is pure: `(state, RawFrame, now) -> Vec<CaptureEvent>`. It does no
I/O, no capture, and takes its clock by injection — which is why the whole
detection pipeline is unit-tested without a GPU, screen, or Windows.

## Cargo features

| Feature | Default | Adds |
|---|---|---|
| `cli` | ✅ | the `framewatch` binary (clap) |
| `wgc` | | Windows Graphics Capture backend + window enumeration |
| `gui` | | eframe/egui window picker & ROI editor |
| `record` | | `record` subcommand: window video (via `ffmpeg`) + mic (`cpal`) → LLM package |
| `whisper` | | bundled local transcription (whisper.cpp via `whisper-rs`) |
| `jpeg` / `webp` | | extra image encoders |
| `llm` | | reserved: out-of-band vision-caption sink |

The core library pulls **no** Windows or GUI deps unless you opt in.

## License

Licensed under either of [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE) at
your option.
