# framewatch — distribution / integration entry point

This folder is the **entry point for another project or agent** to use framewatch.

| File | What it is |
|---|---|
| `framewatch.exe` | The release binary (built with `cli` + `wgc` + `gui`). *Not committed to git; produced by the build below.* |
| `framewatch.json` | **Machine-readable manifest** — where the binary is, every command, every flag, and the full output schema. Parse this. |
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
cargo build --release --features "cli wgc gui"
# then copy target/release/framewatch.exe -> dist/framewatch.exe
```
