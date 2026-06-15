//! End-to-end test of the recording-package writer and the external
//! `--transcribe-cmd` path. Pure and cross-platform — it runs on Linux CI with
//! default features (no whisper.cpp, no Windows capture). The "external
//! transcriber" is this very test binary, re-invoked: portable, no shell or
//! interpreter required.

use chrono::Utc;
use framewatch::recording::{files, AudioMeta, PackageWriter, RecordingManifest, VideoMeta};
use framewatch::{Target, Transcriber};

const FAKE_JSON: &str = r#"{"language":"en","duration_ms":4800,"segments":[{"start_ms":1250,"end_ms":4800,"text":"open the settings panel"}]}"#;

/// Acts as the fake `--transcribe-cmd` when this binary is re-invoked with the
/// framewatch `{output}` base path (`.../transcript_raw`) as an argument: it
/// writes a fixed framewatch-JSON transcript to `<base>.json`. During a normal
/// test run no such argument is present, so this is a no-op.
#[test]
fn zz_fake_transcriber_helper() {
    if let Some(base) = std::env::args().find(|a| a.ends_with("transcript_raw")) {
        std::fs::write(format!("{base}.json"), FAKE_JSON).expect("write fake transcript");
    }
}

fn write_dummy_media(dir: &std::path::Path) {
    // The runtime writes these; simulate them so we can assert the full package.
    std::fs::write(dir.join(files::VIDEO), b"\x00\x00\x00\x18ftypmp42").unwrap();
    std::fs::write(dir.join(files::AUDIO), b"RIFF\x00\x00\x00\x00WAVE").unwrap();
}

fn fake_transcribe_cmd_template() -> String {
    let exe = std::env::current_exe().expect("current exe");
    // Quote the exe path (may contain spaces); pass the {output} placeholder so
    // the helper learns where to write.
    format!(
        "\"{}\" --exact zz_fake_transcriber_helper {{audio}} {{output}}",
        exe.display()
    )
}

#[test]
fn transcribe_cmd_writes_full_package() {
    let tmp = tempfile::tempdir().unwrap();
    let writer = PackageWriter::new(tmp.path(), Utc::now(), "Game.exe").unwrap();
    let dir = writer.recording().dir.clone();

    // Pretend the runtime already produced the media + a wav to transcribe.
    write_dummy_media(&dir);
    let audio = writer.recording().audio_path();

    let transcriber = Transcriber::Command {
        template: fake_transcribe_cmd_template(),
    };
    let transcript = transcriber.transcribe(&audio, &dir).unwrap();
    assert_eq!(transcript.segments.len(), 1);
    assert_eq!(transcript.segments[0].text, "open the settings panel");
    assert_eq!(transcript.segments[0].start_ms, 1250);

    writer.write_transcript(&transcript).unwrap();
    let (engine, model) = transcriber.engine_meta();
    assert_eq!(engine, "command");
    let manifest = RecordingManifest::new(
        writer.recording(),
        &Target::ByExe("Game.exe".into()),
        "cli",
        VideoMeta {
            path: files::VIDEO.into(),
            container: "mp4".into(),
            codec: "h264".into(),
            fps: 30.0,
            width: 1920,
            height: 1080,
            duration_ms: 4800,
        },
        Some(AudioMeta {
            path: files::AUDIO.into(),
            sample_rate: 48_000,
            channels: 1,
            duration_ms: 4800,
        }),
        &transcript,
        engine,
        model,
        Utc::now(),
    );
    writer.finalize(&manifest, &transcript).unwrap();

    // All seven artifacts present.
    for name in [
        files::VIDEO,
        files::AUDIO,
        files::TRANSCRIPT_JSON,
        files::TRANSCRIPT_SRT,
        files::MANIFEST,
        files::PROMPT,
        files::README,
    ] {
        assert!(dir.join(name).exists(), "missing artifact: {name}");
    }

    // transcript.srt matches the canonical rendering.
    let srt = std::fs::read_to_string(dir.join(files::TRANSCRIPT_SRT)).unwrap();
    assert_eq!(srt, transcript.to_srt());
    assert!(srt.contains("00:00:01,250 --> 00:00:04,800"));

    // recording.json parses back and records the engine.
    let manifest_txt = std::fs::read_to_string(dir.join(files::MANIFEST)).unwrap();
    let parsed: RecordingManifest = serde_json::from_str(&manifest_txt).unwrap();
    assert_eq!(parsed.kind, "recording");
    assert_eq!(parsed.transcript.engine, "command");
    assert_eq!(parsed.transcript.segment_count, 1);

    // PROMPT.md embeds the transcript line and the ffmpeg recipe.
    let prompt = std::fs::read_to_string(dir.join(files::PROMPT)).unwrap();
    assert!(prompt.contains("[00:00:01,250 → 00:00:04,800] open the settings panel"));
    assert!(prompt.contains("ffmpeg -ss 12.500 -i recording.mp4 -frames:v 1 frame.png"));
}

#[test]
fn no_transcribe_writes_empty_package() {
    let tmp = tempfile::tempdir().unwrap();
    let writer = PackageWriter::new(tmp.path(), Utc::now(), "Game.exe").unwrap();
    let dir = writer.recording().dir.clone();
    write_dummy_media(&dir);

    let transcript = Transcriber::Disabled
        .transcribe(&writer.recording().audio_path(), &dir)
        .unwrap();
    assert!(transcript.is_empty());

    writer.write_transcript(&transcript).unwrap();
    let manifest = RecordingManifest::new(
        writer.recording(),
        &Target::ByExe("Game.exe".into()),
        "cli",
        VideoMeta {
            path: files::VIDEO.into(),
            container: "mp4".into(),
            codec: "h264".into(),
            fps: 30.0,
            width: 1280,
            height: 720,
            duration_ms: 0,
        },
        Some(AudioMeta {
            path: files::AUDIO.into(),
            sample_rate: 48_000,
            channels: 1,
            duration_ms: 0,
        }),
        &transcript,
        "none",
        None,
        Utc::now(),
    );
    writer.finalize(&manifest, &transcript).unwrap();

    let prompt = std::fs::read_to_string(dir.join(files::PROMPT)).unwrap();
    assert!(prompt.contains("no narration transcript"));
    let json = std::fs::read_to_string(dir.join(files::TRANSCRIPT_JSON)).unwrap();
    assert!(json.contains("\"segments\": []"));
}

#[test]
fn video_only_package_omits_audio() {
    let tmp = tempfile::tempdir().unwrap();
    let writer = PackageWriter::new(tmp.path(), Utc::now(), "Game.exe").unwrap();
    let dir = writer.recording().dir.clone();
    // Only the video exists — no audio.wav.
    std::fs::write(dir.join(files::VIDEO), b"\x00\x00\x00\x18ftypmp42").unwrap();

    let transcript = framewatch::Transcript::default();
    writer.write_transcript(&transcript).unwrap();
    let manifest = RecordingManifest::new(
        writer.recording(),
        &Target::ByExe("Game.exe".into()),
        "cli",
        VideoMeta {
            path: files::VIDEO.into(),
            container: "mp4".into(),
            codec: "h264".into(),
            fps: 30.0,
            width: 1280,
            height: 720,
            duration_ms: 5000,
        },
        None, // no microphone captured
        &transcript,
        "none",
        None,
        Utc::now(),
    );
    writer.finalize(&manifest, &transcript).unwrap();

    // audio.wav is neither listed nor required.
    assert!(!manifest.artifacts.iter().any(|a| a == files::AUDIO));
    let manifest_txt = std::fs::read_to_string(dir.join(files::MANIFEST)).unwrap();
    assert!(!manifest_txt.contains("\"audio\""));
    let parsed: RecordingManifest = serde_json::from_str(&manifest_txt).unwrap();
    assert!(parsed.audio.is_none());
    // The prompt tells the model there is no audio track.
    let prompt = std::fs::read_to_string(dir.join(files::PROMPT)).unwrap();
    assert!(prompt.contains("no audio track"));
}
