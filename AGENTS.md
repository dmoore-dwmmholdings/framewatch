# Framewatch repository guidance

## Project purpose

Framewatch is a Rust crate and Windows CLI that captures meaningful window states
for coding agents. Treat the JSON/JSONL artifacts and agent-facing documentation
as public interfaces.

## Repository map

- `src/engine.rs` and `src/detect/`: platform-neutral detection pipeline.
- `src/capture/windows/`: Windows Graphics Capture implementation behind `wgc`.
- `src/bin/framewatch.rs`: CLI commands and lifecycle behavior.
- `src/record/`, `src/recording.rs`, and `src/transcript.rs`: optional recording
  and narration-package support.
- `docs/AGENT_INTEGRATION.md` and `dist/framewatch.json`: human- and
  machine-readable integration contracts.
- `.agents/skills/framewatch/`: Codex workflow for using Framewatch.

## Working agreements

- Keep the core library cross-platform. Gate Windows capture and GUI behavior
  behind the existing features.
- Use `MockBackend` and injected clocks for deterministic engine tests. Do not
  require a live window, GPU, microphone, or network in automated tests.
- When CLI flags, artifact layouts, or schemas change, update the integration
  guide, `dist/framewatch.json`, generated `README_FOR_AGENT.md` templates, and
  relevant tests together. Update the Codex skill when its workflow changes.
- Do not edit files under `target/` or commit generated executables from `dist/`.
- Preserve unrelated worktree changes.

## Verification

Run the narrowest relevant checks while iterating, then use the applicable CI
set before handoff:

```sh
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo test --no-default-features
```

On Windows, changes that touch live capture, the GUI, or recording also require:

```sh
cargo clippy --features "gui wgc record" --all-targets -- -D warnings
cargo test --features "gui wgc record"
```
