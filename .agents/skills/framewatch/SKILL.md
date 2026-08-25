---
name: framewatch
description: Capture and inspect meaningful states from Windows, macOS, or Linux X11 application windows with the framewatch CLI. Use when Codex needs screenshots after a UI settles, a compact timeline of UI transitions, repeatable visual checks while developing or testing a desktop app, or a narrated recording package. Do not use for browser pages that existing browser tooling can inspect semantically.
---

# Framewatch

Use Framewatch as a bounded visual-observation tool. Prefer deterministic output
paths and inspect only the meaningful frames recorded in its timeline.

## Resolve the executable

1. Prefer `framewatch` when it is on `PATH`.
2. In the Framewatch source checkout, try `target/release/framewatch` on macOS
   or `target/release/framewatch.exe` on Windows, then the equivalent `dist/`
   path.
3. Run the candidate's `--version` and top-level `--help`. When the checkout has
   `Cargo.toml` or `dist/framewatch.json`, require the binary version to match
   the package version. Also confirm the requested subcommand appears in help.
4. If no current candidate exists, install the Windows CLI with
   `cargo install framewatch --features wgc`, or the macOS 14+ CLI with
   `cargo install framewatch --features macos`, or the Linux X11 CLI with
   `cargo install framewatch --features linux-x11`. Use `cargo install framewatch
   --features "wgc record"` when Windows recording is needed, or
   `--features "macos record"` for macOS recording, or
   `--features "linux-x11 record"` for Linux X11 recording. In a source
   checkout where building is in scope, use `cargo build --release --features
   "cli wgc"` on Windows, `cargo build --release --features "cli macos"` on
   macOS, or `cargo build --release --features "cli linux-x11"` on Linux X11;
   add `gui` or `record` for the requested workflow.

Do not silently use a stale binary. Live capture requires Windows with `wgc`,
macOS 14+ with `macos`, or Linux X11 with `linux-x11` and `DISPLAY` set. Wayland
is not supported yet because it needs an XDG Desktop Portal capture flow.
macOS requires Screen Recording permission for the terminal or app running
Framewatch, plus Microphone permission for narrated recordings.

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

## Labelling frames from the app under test

A capture says *when* the window changed. It cannot say what the app thought it
was doing. Marks close that gap, in the same `timeline.jsonl`, so nothing has to
align two clocks afterwards.

**One label, from anywhere** — safe while `watch` is running:

```bash
framewatch mark --label "before-checkout"
framewatch mark --label "signed-in" --json '{"user":"agent.buyer.01"}'
```

With no `--session` it targets the newest session under `--out` (default
`./.framewatch`). `mark` appends the line itself, so the label survives even with
no watcher running.

**A stream of labels, from the app** — no process per label:

```bash
framewatch watch --title "My App" --labels-file "$TEMP/console.log" --duration 60
```

Each line becomes one mark. Plain text is the label; a JSON object is kept whole
in `data`, with its `note`, `label`, or `kind` field used as the label — so an app
can append the events it already logs. In Playwright the bridge is one line:

```js
page.on('console', (m) => appendFileSync(logPath, `${m.text()}\n`))
```

**Timing.** Followed files are read at the moment a frame is captured, so a label
written before a frame lands on *that* frame, not the one after it. Labels
arriving after the last frame are still written when the session closes.

**What you get back.** The next captured frame carries
`marks_since_last_frame: ["before-checkout"]`, and when exactly one mark preceded
it the image is named after the label:

```
frames/000004_settled_before-checkout.png
```

Several marks before one frame all appear in the array, but the frame keeps its
plain name — one label is a name, several are ambiguous.

## Making a small change trip capture

The defaults ignore noise, which also means a thin strip — a 24 px status banner
across the top of a browser window — can be coalesced away.

Try `--roi` first: cropping to the strip makes it 100 % of the frame and needs no
tuning. When you need the whole window *and* the small change:

| Flag | Default | Use it when |
|---|---|---|
| `--min-area-ratio <r>` | `0.002` | the change is a small fraction of the frame |
| `--tile-change-threshold <n>` | `12` | the change is low-contrast |
| `--tile-grid <COLSxROWS>` | `32x18` | a finer grid makes the change a bigger share of one tile |

```bash
framewatch watch --title "My App" --min-area-ratio 0.0005 --tile-change-threshold 8
```

## Recipe: a web app with agent-sandbox

Driving a Firebase app through `@agent-sandbox/client`, whose SDK writes
`[agent-sandbox] #N …` console lines and puts the session id in the window title:

```bash
# 1. Mint a session (MCP `create_session`, or the CLI) and open the URL in Chrome.
# 2. Bridge the page console to a file (Playwright, one line — see above).
# 3. Watch that lane's window, tailing the same file:
framewatch watch --title "sbx buyer.01.k3f9" --labels-file "$TEMP/console.log" \
  --out "$TEMP/fw/buyer.01.k3f9" --settle-ms 250 --duration 300
```

The window title carries the sandbox id, so several lanes can run at once and
each watcher finds its own. Frames come back named after the SDK's own events —
`000004_settled_route.png`, `000006_settled_error.png` — and the timeline holds
both the labels and the captures in one file.

Keep the window in the **foreground**: Chrome stops painting an occluded window,
so framewatch sees no change and captures nothing after the first frame.
