# Changelog

All notable changes to this project are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres
to [Semantic Versioning](https://semver.org/spec/v2.0.0.html) (pre-1.0: breaking
changes bump the minor version).

## [Unreleased]

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
