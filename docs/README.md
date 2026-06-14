# framewatch documentation

Documentation hub for **framewatch** — event-driven, change-triggered window
capture that emits timestamped screenshots + metadata for AI agents.

## Start here

| Document | Read it when you want to… |
|---|---|
| [Project README](../README.md) | Understand what framewatch is, why it exists, install it, and see the feature matrix. |
| [Agent Integration Guide](AGENT_INTEGRATION.md) | Have another project or AI agent **call** framewatch — exact commands, flags, the output/JSON contract, and the embedding API. |
| [`dist/framewatch.json`](../dist/framewatch.json) | Parse a **machine-readable** manifest of the binary path, commands, and output schema. |
| [`dist/sample-session/`](../dist/sample-session) | See a **real example** of one capture's output (`timeline.jsonl`, `session.json`, frames). |
| [Specification](../framewatch-spec.md) | Understand the design, architecture, and rationale in depth. |
| [CHANGELOG](../CHANGELOG.md) | See what changed between versions. |

## API docs

The full Rust API reference is generated from the source:

```sh
cargo doc --open                 # core API
cargo doc --features "wgc gui" --open   # include the Windows backend + GUI
```

Once published, the same docs render at <https://docs.rs/framewatch>.

## Quick map of the codebase

| Area | Module |
|---|---|
| Pure detection engine (state machine) | [`src/engine.rs`](../src/engine.rs) |
| Detection pipeline (diff, hash, ROI, volatility) | [`src/detect/`](../src/detect) |
| Output sinks (directory, channel, composite) | [`src/sink/`](../src/sink) |
| Capture backends (mock + Windows WGC) | [`src/capture/`](../src/capture) |
| Config / events / frames | [`src/config.rs`](../src/config.rs), [`src/event.rs`](../src/event.rs), [`src/frame.rs`](../src/frame.rs) |
| CLI / GUI | [`src/bin/framewatch.rs`](../src/bin/framewatch.rs), [`src/gui/`](../src/gui) |
