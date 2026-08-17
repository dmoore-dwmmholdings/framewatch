---
name: framewatch
description: Capture and inspect meaningful states from Windows application windows with the framewatch CLI. Use when Codex needs screenshots after a UI settles, a compact timeline of UI transitions, repeatable visual checks while developing or testing a desktop app, or a narrated recording package. Do not use for browser pages that existing browser tooling can inspect semantically or for non-Windows live capture.
---

# Framewatch

Use Framewatch as a bounded visual-observation tool. Prefer deterministic output
paths and inspect only the meaningful frames recorded in its timeline.

## Resolve the executable

1. Prefer `framewatch` when it is on `PATH`.
2. In the Framewatch source checkout, try `target/release/framewatch.exe`, then
   `dist/framewatch.exe`.
3. Run the candidate's `--version` and top-level `--help`. When the checkout has
   `Cargo.toml` or `dist/framewatch.json`, require the binary version to match
   the package version. Also confirm the requested subcommand appears in help.
4. If no current candidate exists, install the published Windows CLI with
   `cargo install framewatch --features wgc`; use
   `cargo install framewatch --features "wgc record"` when recording is needed.
   In a source checkout where building is in scope, use
   `cargo build --release --features "cli wgc"`; add `gui` or `record` for the
   requested workflow and use `target/release/framewatch.exe`.

Do not silently use a stale binary. Live capture requires Windows and the `wgc`
feature.

For exact flags or schemas, read `docs/AGENT_INTEGRATION.md` and
`dist/framewatch.json` when they are present in the checkout. Otherwise use the
CLI `--help` output.

## Choose a capture mode

- Use `shot` for one stable image at a known path. Prefer this for visual
  verification after launching or changing an app.
- Use `watch` when transitions, spinners, or several settled states matter.
- Use `record` only when the user explicitly asks to record a window or narrate a
  workflow. Microphone capture and transcription are optional and may expose
  sensitive content. Managed whisper.cpp is the default transcriber; first use
  downloads about 150 MiB into the user cache. Use `--no-transcribe` when
  narration does not need to be interpreted.
- Use `windows` to discover a target when its title, executable, PID, or HWND is
  unknown.
- Use `gui` only when a human needs to pick a window or draw regions of interest.

Prefer `--pid` for an already-running process and `--launch` for an app that
Framewatch should start and tear down. Title and executable matching can select a
stale window from an earlier run.

## Capture one settled state

Create the output directory first when the chosen `--out-file` has a parent.
Then run a bounded command such as:

```powershell
framewatch shot --title "My App" --out-file .framewatch/latest.png --timeout 20 --settle-best-effort
```

Use `--launch "app.exe <args>"` when Framewatch should own the app lifecycle.
Use `--roi X,Y,W,H` to exclude host chrome or irrelevant animation. Open the PNG
with the available image-viewing tool and report the visual evidence.

## Capture a transition timeline

Keep agent-run watches bounded:

```powershell
framewatch watch --title "My App" --wait 15 --until-settled --duration 8 --out ./.framewatch
```

After the command exits:

1. Read `session.json`.
2. Read `timeline.jsonl` chronologically.
3. Open images referenced by `settled` and `busy_end` events. Also inspect
   `initial` when the target was already static and emitted no `settled` event.
4. Use `coalesced_frames` and event notes to summarize activity without opening
   every image.

Do not leave an unbounded `watch` process running after the task.

## Consume a recording package

For an explicitly requested recording, prefer a fixed `--duration`. Use
`--no-audio` when narration is unnecessary. Prefer managed Whisper for narrated
recordings; run `framewatch transcriber setup` before a time-sensitive first
recording so the download is complete. Use `--transcribe-cmd` only for an
approved external override.

Consume the result in this order:

1. Read `PROMPT.md` for the timestamped transcript and task.
2. Read `recording.json` for paths and media metadata.
3. Inspect `recording.mp4` directly when supported, or extract only the frames at
   transcript timestamps that matter.

## Safety and cleanup

- Capture only windows and audio within the user's task scope.
- Treat screenshots, timelines, video, audio, and transcripts as potentially
  sensitive local artifacts.
- Store routine output under the ignored `.framewatch/` directory.
- Stop launched or watched processes on completion. Preserve requested evidence;
  remove scratch captures only when they are no longer needed and deletion is
  authorized.
