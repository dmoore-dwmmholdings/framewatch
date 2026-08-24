# framewatch — distribution / integration entry point

This folder is the **entry point for another project or agent** to use framewatch.

| File | What it is |
|---|---|
| `framewatch.exe` | Windows release binary (built with `cli` + `wgc` + `gui` + `record`). *Not committed to git; produced by the build below.* |
| GitHub Releases | Versioned ZIPs for Windows and macOS (`aarch64-apple-darwin` and `x86_64-apple-darwin`). macOS builds use `cli` + `macos` + `gui` and require macOS 14+. |
| `framewatch.json` | **Machine-readable manifest** — where the binary is, every command, every flag, and the full output schema. Parse this. |
| `../.agents/skills/framewatch/` | **Codex skill** — native workflow discovery for capture, inspection, and recording tasks. |
| `sample-session/` | A real example of what one capture produces (`timeline.jsonl`, `session.json`, `frames/*.png`, `README_FOR_AGENT.md`). |

Full human/agent guide: [`../docs/AGENT_INTEGRATION.md`](../docs/AGENT_INTEGRATION.md).

## TL;DR

```sh
# discover windows
dist\framewatch.exe windows

# capture one window (blocks; Ctrl+C to stop)
dist\framewatch.exe watch --title "Visual Studio Code" --out ./.framewatch

# then read:  ./.framewatch/<session_id>/timeline.jsonl   (+ session.json, frames/)
# open images only for events with kind == "settled" or "busy_end"
```

## (Re)building the binary

```sh
cargo build --release --features "cli wgc gui record"
# then copy target/release/framewatch.exe -> dist/framewatch.exe

# macOS 14+ (ScreenCaptureKit)
cargo build --release --features "cli macos gui"
```
