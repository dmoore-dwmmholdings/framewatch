# Task from a screen recording

A human recorded their screen for 19.0s while narrating instructions out loud. The recording is in this package. Read the narration below, then carry out what they asked. The video lets you see exactly what they were pointing at or referring to.

## What you have
- `recording.mp4` — the screen capture of "Settings — MyApp" (myapp.exe), 1920x1080 at 30 fps, 19.0s long. The narration audio is muxed into the video and also available standalone as `audio.wav`.
- The full narration transcript is inline below. Every line is timestamped in mm:ss,mmm from the start of the video, so each spoken instruction maps to a specific moment on screen.

## How to use the video
- If you can ingest video directly, watch `recording.mp4` and follow along with the timestamps below.
- Otherwise, pull a still frame at any timestamp with ffmpeg. `start_ms` is in milliseconds; ffmpeg `-ss` takes seconds, so divide by 1000:

      ffmpeg -ss <seconds> -i recording.mp4 -frames:v 1 frame.png

  Example — to see what was on screen when the narrator spoke at start_ms 12500:

      ffmpeg -ss 12.500 -i recording.mp4 -frames:v 1 frame.png

- Correlate words with actions: when the narration says "open this menu" at a given timestamp, extract the frame at that timestamp to see which menu.

## Narration transcript (timestamps are mm:ss,mmm from video start)
- [00:00:00,800 → 00:00:04,200] Okay, I'm going to show you the bug. First, open the Settings panel from the gear icon in the top right.
- [00:00:04,800 → 00:00:09,000] See this "Sync" toggle here? Turn it on, and watch the status line at the bottom.
- [00:00:09,600 → 00:00:14,500] It flashes "connected" for a second, then flips back to "offline" — that's the bug I want fixed.
- [00:00:15,200 → 00:00:19,000] The relevant code is in sync_manager.rs; the reconnect handler never clears the old socket.

## Your task
Follow the narrated instructions above in order. Where an instruction is visual ("this", "here", "that button"), use the timestamp to locate the on-screen target in the video before acting.
