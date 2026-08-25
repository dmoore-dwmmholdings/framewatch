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
detection both faster and more accurate — or set `auto_detect_spinners = true`
to have a small, compact loading animation detected and collapsed automatically,
no ROI hint required.

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

### Codex integration

This repository includes both Codex-native discovery layers:

- [`AGENTS.md`](AGENTS.md) gives Codex the project architecture, public-contract
  rules, and verification commands when it works on Framewatch itself.
- [`.agents/skills/framewatch/SKILL.md`](.agents/skills/framewatch/SKILL.md)
  teaches Codex when to use `shot`, `watch`, or `record`, and how to consume the
  resulting artifacts. Codex discovers the skill automatically in this checkout;
  invoke it explicitly with `$framewatch` or let it trigger for desktop UI
  capture tasks.

To use the skill from another repository, ensure a current `framewatch.exe` is
on `PATH` and copy the `framewatch` skill directory into that repository's
`.agents/skills/` directory (or into your user-level `~/.agents/skills/`). See
the [Codex skill documentation](https://developers.openai.com/codex/skills) for
skill discovery details.

## Install

```sh
# CLI (Windows live capture):
cargo install framewatch --features wgc        # add gui for the picker: --features "wgc gui"

# CLI (macOS 14+ live capture; grant Screen Recording permission when prompted):
cargo install framewatch --features macos      # add gui for the picker: --features "macos gui"

# CLI (Linux X11 session; DISPLAY must be set):
cargo install framewatch --features linux-x11

# CLI with narrated recording (requires ffmpeg on PATH):
# Windows: --features "wgc record"; macOS 14+: --features "macos record";
# Linux X11: --features "linux-x11 record"
cargo install framewatch --features "wgc record"
framewatch transcriber setup   # optional preflight; record also runs this automatically

# As a library (engine + sinks only, no clap/egui):
cargo add framewatch --no-default-features
```

> **Platforms.** The detection engine is platform-agnostic and compiles/tests
> everywhere. Live capture is available on **Windows** with `wgc` (Windows
> Graphics Capture), **macOS 14+** with `macos` (ScreenCaptureKit), and an
> **X11 Linux** session with `linux-x11`. macOS requires Screen Recording
> permission; `record` with narration also requires Microphone permission.
> Wayland is not supported yet: its security model requires a separately
> implemented, user-approved XDG Desktop Portal capture flow.
> Linux narrated recording also needs `ffmpeg` and the OpenMP runtime provided
> by the distribution (for example, `libgomp1` on Debian/Ubuntu).

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
`record` subcommand (install with `--features "wgc record"` on Windows,
`--features "macos record"` on macOS, or `--features "linux-x11 record"` on
Linux X11; needs `ffmpeg` on PATH) **continuously**
records one window to video while you narrate into the
microphone, then transcribes the narration locally and bundles everything an LLM
needs to act on it. On first use, Framewatch automatically downloads a pinned,
checksum-verified whisper.cpp runtime and `base.en` model (~150 MiB) into the
user cache; later recordings reuse it:

```sh
# Record a window for 60s (or stop early with Ctrl+C) while you talk.
framewatch record --title "My Game" --duration 60
```

It writes a package directory:

```text
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

> **Transcription** uses managed whisper.cpp by default. Use `--no-transcribe`
> for an audio-only package with no model download. Advanced users can override
> the engine with `--transcribe-cmd`; `{audio}` and `{output}` are substituted
> and the command writes framewatch transcript JSON or SRT. Set
> `FRAMEWATCH_WHISPER_DIR` to change the managed cache parent. The runtime and
> model are downloaded rather than embedded in the crate, keeping installation
> small while making the default workflow self-configuring.
>
> **No microphone?** Recording degrades gracefully — it warns and produces a
> **video-only** package (no transcript). Pass `--no-audio` to opt out of mic
> capture explicitly.

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
    // Off-Windows / in tests, drive the engine with your own frames:
    let mut backend = framewatch::MockBackend::new(vec![/* RawFrames */]);

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
| `macos` | | macOS 14+ ScreenCaptureKit backend + window enumeration |
| `linux-x11` | | Linux X11 per-window capture + window enumeration |
| `gui` | | eframe/egui window picker & ROI editor |
| `record` | | `record` subcommand: window video (via `ffmpeg`) + mic (`cpal`) → LLM package |
| `jpeg` / `webp` | | extra image encoders |
| `llm` | | reserved: out-of-band vision-caption sink |

The core library pulls **no** platform capture or GUI deps unless you opt in.

## License

Licensed under either of [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE) at
your option.
