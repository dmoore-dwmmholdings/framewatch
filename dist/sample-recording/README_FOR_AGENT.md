# framewatch recording package

This directory is a single screen recording of one application window with a
synchronized voice narration and its transcript. A human recorded their screen
while speaking instructions; your job is to follow those instructions, using the
video to see exactly what they pointed at.

Files:
1. `PROMPT.md`        — START HERE. The task prompt with the full transcript inline.
2. `recording.mp4`    — the screen recording (the narration is also muxed in).
3. `audio.wav`        — the raw microphone narration (PCM).
4. `transcript.json`  — machine-readable transcript: segments with `start_ms`/`end_ms`/`text`.
5. `transcript.srt`   — the same transcript as SubRip subtitles (HH:MM:SS,mmm).
6. `recording.json`   — manifest: target window, time range, video/audio/transcript meta.

How to consume:
- A text-only model can work entirely from `PROMPT.md` — the transcript is inline.
- A multimodal model SHOULD also look at the video. Each transcript segment's
  `start_ms`/`end_ms` is measured from the start of `recording.mp4`, so when the
  narration says "click *this*", seek the video to that timestamp to see what
  "this" was. Extract a frame at a timestamp with ffmpeg (`-ss` is in seconds):

      ffmpeg -ss 12.500 -i recording.mp4 -frames:v 1 frame.png

  (start_ms 12500 -> -ss 12.500)
