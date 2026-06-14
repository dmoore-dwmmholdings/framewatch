# Changelog

All notable changes to this project are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres
to [Semantic Versioning](https://semver.org/spec/v2.0.0.html) (pre-1.0: breaking
changes bump the minor version).

## [Unreleased]

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

[Unreleased]: https://github.com/dmoore-dwmmholdings/framewatch/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/dmoore-dwmmholdings/framewatch/releases/tag/v0.1.0
