# framewatch session

This directory is an automatically-captured, de-duplicated visual log of a single
application window. To understand what happened:

1. Read `session.json` — target app, time range, config, and region hints.
2. Stream `timeline.jsonl` (one JSON event per line, chronological). Each event has a
   `kind`, `elapsed_ms`, an optional `image` path, and a human `note`.
3. You usually only need to open images for events with `kind` = "settled" or "busy_end";
   those are stable, meaningful states. `coalesced_frames` tells you how much activity
   each image represents. Use "value_sample"/"busy_start" notes for timing without images.

Frames are PNGs under `frames/`. There is intentionally no continuous stream — the gaps
are quiescent or were collapsed as animation/noise.
