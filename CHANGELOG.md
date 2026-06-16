# Changelog

All notable changes to this project are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres
to [Semantic Versioning](https://semver.org/spec/v2.0.0.html) (pre-1.0: breaking
changes bump the minor version).

## [Unreleased]

## [0.5.0] - 2026-06-16

This release closes the gaps from the v0.4.1 functional assessment and expands the
test suite (the pure, cross-platform core is now ~100% covered).

### Added

- **Opt-in automatic spinner detection (H1).** `watch`/`shot` previously needed a
  hand-drawn `Spinner` ROI to suppress a loading animation; with the new
  `auto_detect_spinners` config flag (default off), a small, compact cluster of
  high-change-rate tiles is detected automatically — its churn no longer counts as
  meaningful change and it drives `busy_start`/`busy_end`, so the window can still
  settle. Tunable via `auto_spinner_max_area` (default `0.05`). Automatic
  *volatile-value* detection still requires an explicit `Volatile` ROI (false
  positives there would suppress real frames).
- **`dedup_forced` config (L1).** Optionally apply dHash dedup to forced
  `Settled`/`Manual` emits so a workflow that settles repeatedly to the same view
  doesn't save byte-identical money-frames (the first `Initial` frame is always
  kept). Default off (prior behaviour).
- **`Engine::set_session_id` (L2).** Embedders consuming events directly (e.g. via
  `ChannelSink`) can stamp the `session_id` that `DirectorySink` would otherwise
  fill; the `session_id`/`image` metadata fields are now documented as
  sink-populated.

### Fixed

- **`--launch` child is no longer orphaned (N1).** A config/`--roi` error after
  the program was launched left it running; `shot`/`record` now wrap the child in
  a kill-on-drop guard so every exit path tears it down.
- **Config validation hardened (M3).** `validate()` now rejects a non-positive
  `image.scale`, an oversized `tile_grid` (which could exhaust memory via the
  volatility ring), out-of-range ROI rects, an out-of-range `auto_spinner_max_area`,
  and a zero-size `crop`; `watch_with` now calls `validate()` so embedders get the
  same checks.
- **`record`: stray temp video on mux failure (N2).** The intermediate
  pre-mux video is now removed on the mux-error path too.
- **`PROMPT.md` timestamp label (N3).** Said `mm:ss,mmm`; now correctly
  `HH:MM:SS,mmm` (matching the rendered values).
- **GUI capture-thread lifecycle (L4).** Each preview worker gets its own stop
  flag + join handle (the prior one is stopped and joined before a new selection
  starts), and `Start watching` is bounded to one concurrent session.

### Changed

- **MSRV is now `1.85`** — the floor the locked dependency tree actually requires
  (edition2024), replacing the untested `1.78` claim. A CI job builds on exactly
  the declared MSRV so it can't drift (M1).
- **Packaging/docs hygiene (M2).** The published crate now ships
  `dist/framewatch.json` and the sample packages the agent docs link to (only the
  large prebuilt binary is excluded); stale `framewatch 0.1.0` version strings and
  the fixture-dependent README embed example were corrected.
- `--title` matching documented as a case-insensitive literal substring; the
  `tokenize` (launch/transcribe) limitations (no escaping; backslashes literal)
  are now documented (N4).

## [0.4.1] - 2026-06-15

### Fixed

- **`record`: Ctrl+C now finalizes the recording instead of erroring.** On
  Windows the console delivers Ctrl+C to the whole process group, so the child
  `ffmpeg` was being killed mid-write and finalize failed with an ffmpeg error
  (no package produced). ffmpeg is now spawned in its own process group
  (`CREATE_NEW_PROCESS_GROUP`); on stop we close its stdin so it finalizes the
  mp4 cleanly. (`--duration` stops were unaffected.)
- **`--title` now matches a case-insensitive literal substring instead of a
  regex.** Window titles routinely contain regex-special characters (Windows
  paths with `\`, `(beta)`, `.`), so a copied-and-quoted title would silently
  fail to match (or error on an invalid escape). Matching as plain text is what
  users expect — `--title "discord"` matches "Discord", and titles with spaces or
  backslashes work as typed. Applies to `watch` / `shot` / `record`.

## [0.4.0] - 2026-06-15

### Added

- **`record` subcommand + recording packages (V4).** A new mode that is the
  deliberate opposite of `watch`: it *continuously* records one window to video
  while you narrate into the microphone, then locally transcribes the narration
  and emits an LLM-ready **package**. `framewatch record --title "My Game"
  --duration 60` (stop early with Ctrl+C) writes a directory containing:
  - `recording.mp4` — the window video (H.264) with the narration muxed in,
  - `audio.wav` — the raw microphone narration,
  - `transcript.json` / `transcript.srt` — segments with `start_ms`/`end_ms`
    measured from video start, so each spoken instruction maps to a moment on
    screen,
  - `recording.json` — the package manifest,
  - `PROMPT.md` — a generated prompt that embeds the timestamped transcript inline
    and explains how to ingest the video or pull a frame at a timestamp with
    `ffmpeg -ss`,
  - `README_FOR_AGENT.md` — how to consume the package.

  Selectors and `--launch` / `--out` / `--roi` / `--wait` / `--duration` mirror
  `watch`/`shot`; plus `--fps`, `--mic <device>`, `--no-audio`.
- **Graceful video-only fallback.** If no microphone is available (or with
  `--no-audio`), `record` warns and produces a valid video-only package instead
  of failing — the manifest omits the `audio` block and the transcript is empty.
- **Local transcription via `--transcribe-cmd`.** framewatch bundles no
  speech-to-text engine; it shells out to a local transcriber you have (e.g.
  whisper.cpp's prebuilt `whisper-cli`, `faster-whisper`, `openai-whisper`).
  `{audio}` / `{output}` are substituted; the command emits framewatch transcript
  JSON or SubRip (SRT), which framewatch reads back. `--no-transcribe` records
  video + audio only. This keeps the crate light, publishable, and dependency-free
  for transcription.
- New public API: `framewatch::{record, RecordConfig, RecordOutcome}` (the
  `record` feature), `Transcript` / `TranscriptSegment` / `Transcriber`,
  `Recording` / `RecordingManifest` / `PackageWriter`, and `tokenize`.

### Changed

- Video encoding shells out to `ffmpeg` (must be on PATH); microphone capture
  uses the pure-Rust `cpal` crate. Both are behind the optional `record` feature,
  so default and library builds are unaffected.

### Internal

- Extracted `ManifestTarget::from_target` (shared by the session and recording
  manifests) and moved the launch-string `tokenize` into the library.

## [0.3.0] - 2026-06-14

### Added
- **`shot` subcommand** (from agent feedback): one-shot capture of a single
  settled frame to a chosen file. Optionally `--launch "<cmd>"` to spawn a
  program, capture *its* window (matched by PID), and kill it on exit. Writes to
  `--out-file`, prints the path on stdout, and exits non-zero if nothing settled
  before `--timeout` (`--settle-best-effort` writes the latest frame instead).
  Collapses launch → wait → capture → teardown into one command, with no session
  directory or timestamped glob.
- **Exact `--pid` window matching** (on `watch` and `shot`) and a `Target::ByPid`
  variant — avoids latching onto a stale window from an earlier run of the same
  exe on back-to-back captures.
- **Headless `--roi <X,Y,W,H>` crop**: capture, change detection, and saved
  images are all clipped to a pixel region — e.g. to drop host window chrome
  (titlebar / menu bar) around a captured app, without round-tripping through the
  GUI ROI editor. Backed by `Config::crop`, the `crop` / `crop_xywh` builder
  methods, and a public `RawFrame::crop`.

### Changed
- `Config` and `Target` are now `#[non_exhaustive]`; construct `Config` via
  `Config::builder()` / `Config::default()` (reading/writing existing fields and
  constructing `Target` variants are unaffected). This lets future config knobs
  and target kinds be added as non-breaking patch releases.

### Fixed
- CI: use `checked_div` instead of a manual `if count == 0` guard in
  `WorkingFrame::from_raw`, satisfying clippy's `manual_checked_ops` lint (Rust 1.96).
- Docs: clarified that `window.rect` is `[x, y, width, height]` (not
  `[left, top, right, bottom]`) in virtual-desktop pixels, and that a perfectly
  static target yields only the `initial` frame (pair `--until-settled` with
  `--duration` as a fallback bound).

### Internal
- CI pins the Rust toolchain (1.96.0) instead of tracking `@stable`, so a new
  compiler/clippy release can't turn CI red without a code change.

## [0.2.0] - 2026-06-14

### Fixed
- **Fullscreen / sustained-activity captures.** A surface that changes on every
  frame (e.g. a fullscreen video or game) never quiesced, so after the initial
  frame *no images were ever saved*. Added a `max_active_ms` keyframe (default
  5000 ms) so sustained activity still yields periodic captures.
- Implemented `fps_cap` (it was a documented-but-unused config knob): frames
  arriving faster than the cap are now dropped before the downsample pass.
- Windows backend: window geometry (`rect`/`client_rect`/`dpi`/`foreground`) in
  the timeline metadata is now refreshed during capture instead of being frozen
  at start, so it stays correct across resizes / fullscreen transitions.

### Added
- **Lifecycle flags for frictionless agent use** (from real agent feedback):
  `--wait <secs>` polls for the target window to appear (no launch-order race),
  and `--until-settled` / `--duration <secs>` / `--frames <n>` make `watch` a
  bounded one-shot that exits on its own. Backed by `Config::{wait_ms,
  stop_after_ms, stop_after_images, stop_after_settled}` and a new
  `watch_with(config, backend, sink)` for embedding with a custom backend.
- All-black frame detection: logs a one-time warning when the target is likely in
  exclusive fullscreen or showing DRM-protected content (which WGC renders black).
- `Engine::frames_dropped()` and the `max_active_ms` config / builder option.

### Hardened
- Buffer-size arithmetic in `encode` and `WorkingFrame::from_raw` now uses `usize`
  math to avoid `u32` overflow on very large (multi-4K) frames.
- GUI preview no longer panics if the frame mutex is poisoned.

## [0.1.0] - 2026-06-14

Initial release.

### Added
- Pure, backend-agnostic detection `Engine`: tile diffing, dHash dedup,
  per-tile/region volatility tracking, and an Idle/Active/Busy state machine.
- Event model + JSON contract: `CaptureEvent` / `CaptureMeta`, `timeline.jsonl`,
  and the `session.json` manifest.
- Sinks: `DirectorySink` (PNG + timeline + manifest + `README_FOR_AGENT.md`,
  with rotation), `ChannelSink`, and `CompositeSink`.
- Cross-platform `MockBackend` (replays in-memory frames or decoded PNGs).
- Windows Graphics Capture backend and window enumeration behind the `wgc`
  feature (`#[cfg(windows)]`), wrapping `windows-capture` 2.x.
- `framewatch` CLI (`windows`, `watch`, `gui`) behind the `cli` feature.
- eframe/egui GUI (window picker, live preview, ROI editor) behind the `gui`
  feature.
- Configuration via builder API and `framewatch.toml`.
- Scenario + golden tests covering static, spinner, volatile, dedup, and the
  full directory-sink pipeline.

[Unreleased]: https://github.com/dmoore-dwmmholdings/framewatch/compare/v0.5.0...HEAD
[0.5.0]: https://github.com/dmoore-dwmmholdings/framewatch/compare/v0.4.1...v0.5.0
[0.4.1]: https://github.com/dmoore-dwmmholdings/framewatch/compare/v0.4.0...v0.4.1
[0.4.0]: https://github.com/dmoore-dwmmholdings/framewatch/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/dmoore-dwmmholdings/framewatch/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/dmoore-dwmmholdings/framewatch/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/dmoore-dwmmholdings/framewatch/releases/tag/v0.1.0
