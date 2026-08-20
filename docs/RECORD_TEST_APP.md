# Recording test app

`record-test-app` is a native egui window containing intentional, visually
obvious defects. Use it for a bounded manual test of `framewatch record`; it is
not part of the production binary.

## Prepare from the publishable crate

Build only the test app from the checkout, package the crate, then install that
packaged source into an isolated Cargo root. The recording commands below use
the installed binary, not `target/release/framewatch.exe`:

```powershell
cargo build --release --features gui --example record-test-app
cargo package --allow-dirty

$metadata = cargo metadata --no-deps --format-version 1 | ConvertFrom-Json
$version = ($metadata.packages | Where-Object name -eq "framewatch").version
$installRoot = Join-Path $PWD ".framewatch\install-smoke"
cargo install `
  --path ".\target\package\framewatch-$version" `
  --root $installRoot `
  --locked `
  --features "wgc record"
$framewatch = Join-Path $installRoot "bin\framewatch.exe"
& $framewatch --version
& $framewatch record --help
& $framewatch transcriber setup
```

FFmpeg and FFprobe must be on `PATH`. Framewatch provisions its pinned managed
Whisper runtime/model automatically on the first narrated recording.

## Run the narrated test

In PowerShell, launch the test window and retain its exact PID:

```powershell
$testApp = Start-Process .\target\release\examples\record-test-app.exe -PassThru
```

Then start a bounded recording. The first run downloads and verifies managed
Whisper (~150 MiB); subsequent runs reuse the cache:

```powershell
& $framewatch record `
  --pid $testApp.Id `
  --duration 45 `
  --fps 15 `
  --out .\.fwsmoke
```

While recording, narrate every visible problem you notice. Click `SUBMITT` once
and resize the window so the recording covers motion, interaction, and a size
change. The app requests an intentionally odd 901×651 initial size; Framewatch
pads odd dimensions internally for H.264 compatibility.

To exercise capture without transcription or a download, add `--no-transcribe`.
To override managed Whisper with another local transcriber, use for example:

```powershell
--transcribe-cmd "whisper-cli -m <model.bin> -f {audio} -osrt -of {output}"
```

## Verify the package

Use the package path printed by Framewatch:

```powershell
ffprobe -v error -show_entries format=duration:stream=index,codec_name,width,height,r_frame_rate -of json <package>\recording.mp4
Get-Content -Raw <package>\recording.json | ConvertFrom-Json
```

Expected artifacts are `recording.mp4`, `audio.wav`, `transcript.json`,
`transcript.srt`, `recording.json`, `PROMPT.md`, and `README_FOR_AGENT.md`.

## Planted-issue answer key

Open this section only after making the narration if you want a blind review.

1. “Recording” is misspelled in the heading.
2. “All systems operational” contradicts “3 critical errors.”
3. The progress bar shows 72%, but its text says 42%.
4. `$10.00 + $5.00 = $12.00` is incorrect.
5. `not-an-email` is not a valid contact email address.
6. “Notifications” is misspelled.
7. The security-alert helper text has poor contrast.
8. The destructive action is green and the safe action is red.
9. `SUBMITT` is misspelled.
10. Header version 2.5.0 conflicts with footer version 2.4.0.
