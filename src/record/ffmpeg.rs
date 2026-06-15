//! ffmpeg subprocess helpers: building the encode/mux argument vectors, probing
//! for ffmpeg on PATH, and driving the encode child over stdin.
//!
//! framewatch never links ffmpeg — it shells out to the `ffmpeg` binary. The
//! window video is piped in as raw BGRA and encoded to H.264/mp4; a second pass
//! muxes that with the recorded WAV, applying the measured A/V start offset.

use crate::error::RecordError;
use std::io::Write;
use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};

/// Spawn ffmpeg in its own process group so a console Ctrl+C (which Windows
/// delivers to the whole group) doesn't kill it mid-write. We stop it cleanly by
/// closing stdin instead, which lets it finalize the mp4 (moov atom).
const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;

/// Build an `ffmpeg` command that won't receive the console's Ctrl+C.
fn ffmpeg_command() -> Command {
    let mut cmd = Command::new("ffmpeg");
    cmd.creation_flags(CREATE_NEW_PROCESS_GROUP);
    cmd
}

/// Build the args for the video-encode pass: raw BGRA frames on stdin (constant
/// `fps`, locked `width`x`height`) → H.264/mp4 at `out`.
pub(crate) fn encode_args(width: u32, height: u32, fps: u32, out: &str) -> Vec<String> {
    vec![
        "-hide_banner".into(),
        "-loglevel".into(),
        "warning".into(),
        "-y".into(),
        // Input: raw BGRA, constant frame rate matching the pacing clock.
        "-f".into(),
        "rawvideo".into(),
        "-pixel_format".into(),
        "bgra".into(),
        "-video_size".into(),
        format!("{width}x{height}"),
        "-framerate".into(),
        fps.to_string(),
        "-i".into(),
        "pipe:0".into(),
        // No audio in this pass (muxed later).
        "-an".into(),
        // yuv420p is required for broad mp4/H.264 player compatibility.
        "-c:v".into(),
        "libx264".into(),
        "-preset".into(),
        "veryfast".into(),
        "-pix_fmt".into(),
        "yuv420p".into(),
        "-movflags".into(),
        "+faststart".into(),
        out.into(),
    ]
}

/// Build the args for the mux pass: encoded video + WAV → final mp4, shifting
/// the audio by `audio_offset_s` (positive delays audio) to align starts.
pub(crate) fn mux_args(audio: &str, video: &str, audio_offset_s: f64, out: &str) -> Vec<String> {
    vec![
        "-hide_banner".into(),
        "-loglevel".into(),
        "warning".into(),
        "-y".into(),
        // Audio is input 0 (offset applies to the input it precedes).
        "-itsoffset".into(),
        format!("{audio_offset_s:.3}"),
        "-i".into(),
        audio.into(),
        "-i".into(),
        video.into(),
        // Take video from input 1, audio from input 0.
        "-map".into(),
        "1:v:0".into(),
        "-map".into(),
        "0:a:0".into(),
        "-c:v".into(),
        "copy".into(),
        "-c:a".into(),
        "aac".into(),
        "-b:a".into(),
        "128k".into(),
        "-movflags".into(),
        "+faststart".into(),
        "-shortest".into(),
        out.into(),
    ]
}

/// Whether an `ffmpeg` binary is reachable on PATH.
pub(crate) fn ffmpeg_available() -> bool {
    Command::new("ffmpeg")
        .arg("-version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn spawn_err(e: std::io::Error) -> RecordError {
    if e.kind() == std::io::ErrorKind::NotFound {
        RecordError::FfmpegNotFound
    } else {
        RecordError::Io(e)
    }
}

/// A running ffmpeg encode process fed raw BGRA frames over stdin.
pub(crate) struct VideoEncoder {
    child: Child,
    stdin: Option<ChildStdin>,
}

impl VideoEncoder {
    /// Spawn `ffmpeg` to encode `width`x`height` BGRA frames at `fps` to `out`.
    pub(crate) fn spawn(
        width: u32,
        height: u32,
        fps: u32,
        out: &Path,
    ) -> Result<Self, RecordError> {
        let out = out.to_string_lossy().into_owned();
        let mut child = ffmpeg_command()
            .args(encode_args(width, height, fps, &out))
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            // Let ffmpeg's warnings/errors reach the user's terminal directly —
            // this also avoids a stderr pipe filling up and blocking our writes.
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(spawn_err)?;
        let stdin = child.stdin.take();
        Ok(Self { child, stdin })
    }

    /// Write one frame's worth of BGRA bytes to ffmpeg's stdin.
    pub(crate) fn write_frame(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        match self.stdin.as_mut() {
            Some(w) => w.write_all(bytes),
            None => Ok(()),
        }
    }

    /// Close stdin (EOF) and wait for ffmpeg to finalize the file. Must be called
    /// on a clean stop — killing the process instead would leave the mp4 without
    /// its moov atom (unplayable).
    pub(crate) fn finish(mut self) -> Result<(), RecordError> {
        drop(self.stdin.take());
        let status = self.child.wait()?;
        if status.success() {
            Ok(())
        } else {
            Err(RecordError::FfmpegFailed(status))
        }
    }
}

/// Mux `video` + `audio` into `out`, shifting audio by `audio_offset_s`.
pub(crate) fn run_mux(
    audio: &Path,
    video: &Path,
    audio_offset_s: f64,
    out: &Path,
) -> Result<(), RecordError> {
    let status = ffmpeg_command()
        .args(mux_args(
            &audio.to_string_lossy(),
            &video.to_string_lossy(),
            audio_offset_s,
            &out.to_string_lossy(),
        ))
        .stdin(Stdio::null())
        .stderr(Stdio::inherit())
        .status()
        .map_err(spawn_err)?;
    if status.success() {
        Ok(())
    } else {
        Err(RecordError::FfmpegFailed(status))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_args_declare_cfr_bgra_input() {
        let a = encode_args(1920, 1080, 30, "out.mp4");
        // input framerate/size declared before -i, on the input side.
        let i = a.iter().position(|x| x == "-i").unwrap();
        assert!(a[..i].contains(&"rawvideo".to_string()));
        assert!(a[..i].contains(&"bgra".to_string()));
        assert!(a[..i].contains(&"1920x1080".to_string()));
        assert!(a[..i].contains(&"30".to_string()));
        // output uses h264 + yuv420p + faststart.
        assert!(a.contains(&"libx264".to_string()));
        assert!(a.contains(&"yuv420p".to_string()));
        assert!(a.contains(&"+faststart".to_string()));
        assert_eq!(a.last().unwrap(), "out.mp4");
    }

    #[test]
    fn mux_args_apply_offset_and_copy_video() {
        let a = mux_args("a.wav", "v.mp4", 0.25, "final.mp4");
        let off = a.iter().position(|x| x == "-itsoffset").unwrap();
        assert_eq!(a[off + 1], "0.250");
        // audio (input 0) precedes video (input 1); video stream is copied.
        assert!(a.windows(2).any(|w| w == ["-map", "1:v:0"]));
        assert!(a.windows(2).any(|w| w == ["-map", "0:a:0"]));
        assert!(a.windows(2).any(|w| w == ["-c:v", "copy"]));
        assert!(a.contains(&"-shortest".to_string()));
        assert_eq!(a.last().unwrap(), "final.mp4");
    }

    #[test]
    fn mux_args_format_negative_offset() {
        let a = mux_args("a.wav", "v.mp4", -0.02, "final.mp4");
        let off = a.iter().position(|x| x == "-itsoffset").unwrap();
        assert_eq!(a[off + 1], "-0.020");
    }
}
