//! The `framewatch` CLI: `windows`, `watch`, `shot`, `record`, and `gui` subcommands.

use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand};
use framewatch::{
    CaptureEvent, ChannelSink, Config, DirectorySink, EncodedImage, EventKind, Target,
};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "framewatch",
    version,
    about = "Event-driven, change-triggered window capture for AI agents."
)]
struct Cli {
    /// Increase log verbosity (-v, -vv). Overridden by RUST_LOG.
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    verbose: u8,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
// The `Watch` variant carries many CLI flags; size doesn't matter for a
// parsed-once command enum.
#[allow(clippy::large_enum_variant)]
enum Command {
    /// List capturable windows (title, exe, hwnd).
    Windows,
    /// Watch a window and write a framewatch session.
    Watch(WatchArgs),
    /// One-shot: (optionally launch a program,) capture one settled frame to a
    /// single file, print its path, and exit. Ideal for scripted/batch capture.
    Shot(ShotArgs),
    /// Record a window to video while narrating into the mic, then write an
    /// LLM-ready package (video + timestamped transcript + prompt).
    Record(RecordArgs),
    /// Launch the GUI picker / ROI editor.
    Gui(GuiArgs),
}

#[derive(Args)]
struct WatchArgs {
    /// Match the window title by a case-insensitive substring.
    #[arg(long, group = "target")]
    title: Option<String>,
    /// Match by executable basename, e.g. "Code.exe".
    #[arg(long, group = "target")]
    exe: Option<String>,
    /// Match by native window handle.
    #[arg(long, group = "target")]
    hwnd: Option<isize>,
    /// Match the window owned by this process id (exact — avoids latching onto a
    /// stale window from an earlier run of the same exe).
    #[arg(long, group = "target")]
    pid: Option<u32>,
    /// Load a base config from this TOML file.
    #[arg(long)]
    config: Option<PathBuf>,
    /// Output directory (default ./.framewatch).
    #[arg(long)]
    out: Option<PathBuf>,
    /// Quiescence (ms) to declare "settled".
    #[arg(long)]
    settle_ms: Option<u64>,
    /// Throttle (ms) for volatile-region samples.
    #[arg(long)]
    value_sample_ms: Option<u64>,
    /// Wait up to N seconds for the target window to appear before failing.
    #[arg(long)]
    wait: Option<u64>,
    /// Auto-stop capture after N seconds (one-shot with a time bound).
    #[arg(long)]
    duration: Option<u64>,
    /// Auto-stop after N images have been saved.
    #[arg(long)]
    frames: Option<u64>,
    /// Auto-stop after the first settled frame (deterministic one-shot).
    #[arg(long)]
    until_settled: bool,
    /// Crop capture + detection + output to a pixel region: `X,Y,W,H`
    /// (e.g. to clip host window chrome / titlebar). Coords are relative to the
    /// captured frame's top-left.
    #[arg(long, value_name = "X,Y,W,H")]
    roi: Option<String>,
}

/// Parse an `X,Y,W,H` ROI spec into a [`framewatch::Rect`].
fn parse_roi(spec: &str) -> Result<framewatch::Rect> {
    let parts: Vec<&str> = spec.split(',').map(|s| s.trim()).collect();
    if parts.len() != 4 {
        anyhow::bail!("--roi must be X,Y,W,H (4 comma-separated integers), got: {spec:?}");
    }
    let x = parts[0].parse().context("--roi X")?;
    let y = parts[1].parse().context("--roi Y")?;
    let w = parts[2].parse().context("--roi W")?;
    let h = parts[3].parse().context("--roi H")?;
    Ok(framewatch::Rect::new(x, y, w, h))
}

#[derive(Args)]
struct ShotArgs {
    /// Match the window title by a case-insensitive substring.
    #[arg(long, group = "target")]
    title: Option<String>,
    /// Match by executable basename, e.g. "Code.exe".
    #[arg(long, group = "target")]
    exe: Option<String>,
    /// Match by native window handle.
    #[arg(long, group = "target")]
    hwnd: Option<isize>,
    /// Match the window owned by this process id.
    #[arg(long, group = "target")]
    pid: Option<u32>,
    /// Launch this command, capture its window (by its PID), then kill it.
    /// The launch string is whitespace-split (use `"..."` to group an argument).
    #[arg(long)]
    launch: Option<String>,
    /// Write the single settled PNG to this exact path (deterministic).
    #[arg(long)]
    out_file: PathBuf,
    /// Overall budget in seconds (wait for the window + for it to settle).
    #[arg(long, default_value_t = 20)]
    timeout: u64,
    /// Quiescence (ms) to declare "settled".
    #[arg(long)]
    settle_ms: Option<u64>,
    /// Crop to a pixel region X,Y,W,H (clips host chrome).
    #[arg(long, value_name = "X,Y,W,H")]
    roi: Option<String>,
    /// If nothing fully settles before the timeout, write the latest frame anyway
    /// (instead of exiting non-zero).
    #[arg(long)]
    settle_best_effort: bool,
    /// Load a base config from this TOML file.
    #[arg(long)]
    config: Option<PathBuf>,
}

#[derive(Args)]
struct RecordArgs {
    /// Match the window title by a case-insensitive substring.
    #[arg(long, group = "target")]
    title: Option<String>,
    /// Match by executable basename, e.g. "Code.exe".
    #[arg(long, group = "target")]
    exe: Option<String>,
    /// Match by native window handle.
    #[arg(long, group = "target")]
    hwnd: Option<isize>,
    /// Match the window owned by this process id.
    #[arg(long, group = "target")]
    pid: Option<u32>,
    /// Launch this command, record its window (by PID), then kill it on stop.
    /// The launch string is whitespace-split (use `"..."` to group an argument).
    #[arg(long)]
    launch: Option<String>,
    /// Parent output directory (a per-recording subdir is created inside it).
    #[arg(long)]
    out: Option<PathBuf>,
    /// Crop the recorded region to a pixel rect `X,Y,W,H` (clips host chrome).
    #[arg(long, value_name = "X,Y,W,H")]
    roi: Option<String>,
    /// Wait up to N seconds for the target window to appear before failing.
    #[arg(long)]
    wait: Option<u64>,
    /// Auto-stop after N seconds (otherwise record until Ctrl+C).
    #[arg(long)]
    duration: Option<u64>,
    /// Target video frames per second (1..=60).
    #[arg(long, default_value_t = 30)]
    fps: u32,
    /// Microphone input device name (substring match; default: system default).
    #[arg(long)]
    mic: Option<String>,
    /// Don't capture the microphone — record video only (also skips transcription).
    #[arg(long)]
    no_audio: bool,
    /// Transcribe the narration by shelling out to a local transcriber (e.g.
    /// whisper.cpp's `whisper-cli`). `{audio}` and `{output}` are substituted;
    /// the command must emit framewatch transcript JSON or SRT.
    #[arg(long, value_name = "CMD")]
    transcribe_cmd: Option<String>,
    /// Skip transcription (record video + audio only).
    #[arg(long, conflicts_with = "transcribe_cmd")]
    no_transcribe: bool,
    /// Load a base config from this TOML file (for `out`/`target`/`roi` defaults).
    #[arg(long)]
    config: Option<PathBuf>,
}

#[derive(Args)]
struct GuiArgs {
    /// Load a base config from this TOML file.
    #[arg(long)]
    config: Option<PathBuf>,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    init_tracing(cli.verbose);

    match cli.command {
        Command::Windows => cmd_windows(),
        Command::Watch(args) => cmd_watch(args),
        Command::Shot(args) => cmd_shot(args),
        Command::Record(args) => cmd_record(args),
        Command::Gui(args) => cmd_gui(args),
    }
}

fn init_tracing(verbose: u8) {
    use tracing_subscriber::EnvFilter;
    let default = match verbose {
        0 => "warn",
        1 => "info",
        2 => "debug",
        _ => "trace",
    };
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .try_init();
}

fn cmd_windows() -> Result<()> {
    let windows = framewatch::enumerate_windows().context("enumerating windows")?;
    if windows.is_empty() {
        println!("(no capturable windows found)");
        return Ok(());
    }
    #[allow(clippy::print_literal)]
    {
        println!("{:<10} {:<24} {}", "HWND", "EXE", "TITLE");
    }
    for w in windows {
        let title: String = w.title.chars().take(70).collect();
        println!("{:<10} {:<24} {}", w.hwnd, w.exe, title);
    }
    Ok(())
}

fn cmd_watch(args: WatchArgs) -> Result<()> {
    let mut config = match &args.config {
        Some(path) => Config::from_toml_path(path).context("loading config")?,
        None => Config::default(),
    };

    if let Some(t) = args.title {
        config.target = Target::ByTitleRegex(t);
    } else if let Some(e) = args.exe {
        config.target = Target::ByExe(e);
    } else if let Some(h) = args.hwnd {
        config.target = Target::ByHwnd(h);
    } else if let Some(p) = args.pid {
        config.target = Target::ByPid(p);
    }
    if let Some(out) = args.out {
        config.out_dir = out;
    }
    if let Some(ms) = args.settle_ms {
        config.settle_ms = ms;
    }
    if let Some(ms) = args.value_sample_ms {
        config.value_sample_ms = ms;
    }
    if let Some(secs) = args.wait {
        config.wait_ms = secs.saturating_mul(1000);
    }
    if let Some(secs) = args.duration {
        config.stop_after_ms = secs.saturating_mul(1000);
    }
    if let Some(n) = args.frames {
        config.stop_after_images = n;
    }
    if args.until_settled {
        config.stop_after_settled = true;
    }
    if let Some(spec) = args.roi.as_deref() {
        config.crop = Some(parse_roi(spec)?);
    }

    config.validate().context("invalid configuration")?;

    let sink = DirectorySink::new(&config).context("creating output sink")?;
    let dir = sink.session().dir.clone();
    println!("framewatch: writing session to {}", dir.display());
    println!("framewatch: press Ctrl+C to stop.");

    framewatch::watch(config, sink).context("capture loop")?;
    Ok(())
}

fn cmd_shot(args: ShotArgs) -> Result<()> {
    // 1. Optionally launch the target program; we capture its window by PID and
    //    tear it down afterwards.
    let mut child = match &args.launch {
        Some(cmd) => Some(spawn_launch(cmd).context("launching --launch command")?),
        None => None,
    };

    // 2. Build a config that captures exactly one settled frame, bounded by --timeout.
    let mut config = match &args.config {
        Some(path) => Config::from_toml_path(path).context("loading config")?,
        None => Config::default(),
    };
    if let Some(c) = &child {
        config.target = Target::ByPid(c.id());
    } else if let Some(p) = args.pid {
        config.target = Target::ByPid(p);
    } else if let Some(t) = args.title {
        config.target = Target::ByTitleRegex(t);
    } else if let Some(e) = args.exe {
        config.target = Target::ByExe(e);
    } else if let Some(h) = args.hwnd {
        config.target = Target::ByHwnd(h);
    } else {
        anyhow::bail!("provide a selector (--title/--exe/--hwnd/--pid) or --launch");
    }
    let budget_ms = args.timeout.saturating_mul(1000);
    config.wait_ms = budget_ms; // wait for the window to appear
    config.stop_after_ms = budget_ms; // ...and for it to settle
    config.stop_after_settled = true;
    if let Some(ms) = args.settle_ms {
        config.settle_ms = ms;
    }
    if let Some(spec) = args.roi.as_deref() {
        config.crop = Some(parse_roi(spec)?);
    }
    config.validate().context("invalid configuration")?;

    // 3. Capture into a channel (no session directory — just frames in memory).
    let (sink, rx) = ChannelSink::unbounded();
    let capture = framewatch::watch(config, sink);

    // 4. Always tear down the launched process before reporting.
    if let Some(c) = child.as_mut() {
        let _ = c.kill();
        let _ = c.wait();
    }
    capture.context("capture")?;

    // 5. Pick the frame and write it to the requested path.
    let events: Vec<CaptureEvent> = rx.try_iter().collect();
    match select_shot_frame(&events, args.settle_best_effort) {
        Some(img) => {
            std::fs::write(&args.out_file, &img.bytes)
                .with_context(|| format!("writing {}", args.out_file.display()))?;
            // The chosen frame path on stdout (machine-readable for scripts).
            println!("{}", args.out_file.display());
            Ok(())
        }
        None => {
            eprintln!(
                "framewatch: no settled frame within {}s (use --settle-best-effort to write the latest frame anyway)",
                args.timeout
            );
            std::process::exit(3);
        }
    }
}

/// Choose which captured frame to save: the last settled frame, or (if
/// `best_effort`) the last image-bearing frame, else nothing.
fn select_shot_frame(events: &[CaptureEvent], best_effort: bool) -> Option<&EncodedImage> {
    if let Some(ev) = events
        .iter()
        .rev()
        .find(|e| e.kind() == EventKind::Settled && e.image.is_some())
    {
        return ev.image.as_ref();
    }
    if best_effort {
        if let Some(ev) = events.iter().rev().find(|e| e.image.is_some()) {
            return ev.image.as_ref();
        }
    }
    None
}

/// Spawn a process from a whitespace-split command string (double quotes group).
fn spawn_launch(cmd: &str) -> Result<std::process::Child> {
    let tokens = framewatch::tokenize(cmd);
    let (program, rest) = tokens
        .split_first()
        .ok_or_else(|| anyhow::anyhow!("--launch is empty"))?;
    std::process::Command::new(program)
        .args(rest)
        .spawn()
        .map_err(Into::into)
}

#[cfg(feature = "record")]
fn cmd_record(args: RecordArgs) -> Result<()> {
    use framewatch::recording::{files, AudioMeta, VideoMeta};
    use framewatch::{record, PackageWriter, RecordConfig, RecordingManifest, Transcriber};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    // Base config supplies out_dir / target / crop defaults.
    let base = match &args.config {
        Some(path) => Config::from_toml_path(path).context("loading config")?,
        None => Config::default(),
    };

    // 1. Optionally launch the target; then capture its window by PID.
    let mut child = match &args.launch {
        Some(cmd) => Some(spawn_launch(cmd).context("launching --launch command")?),
        None => None,
    };
    let target = if let Some(c) = &child {
        Target::ByPid(c.id())
    } else if let Some(t) = args.title {
        Target::ByTitleRegex(t)
    } else if let Some(e) = args.exe {
        Target::ByExe(e)
    } else if let Some(h) = args.hwnd {
        Target::ByHwnd(h)
    } else if let Some(p) = args.pid {
        Target::ByPid(p)
    } else {
        base.target.clone()
    };
    if matches!(&target, Target::ByTitleRegex(s) | Target::ByExe(s) if s.is_empty()) {
        anyhow::bail!("provide a selector (--title/--exe/--hwnd/--pid) or --launch");
    }

    // 2. Choose the transcriber up front. `--no-audio` implies no transcription
    //    (there's nothing to transcribe).
    let transcriber = if args.no_transcribe || args.no_audio {
        Transcriber::Disabled
    } else if let Some(cmd) = args.transcribe_cmd.clone() {
        Transcriber::Command { template: cmd }
    } else {
        eprintln!(
            "framewatch: no transcription requested — recording video + audio only \
             (pass --transcribe-cmd \"<transcriber>\"; --no-transcribe silences this)."
        );
        Transcriber::Disabled
    };

    // 3. Output package directory.
    let out_dir = args.out.unwrap_or(base.out_dir);
    let crop = match args.roi.as_deref() {
        Some(spec) => Some(parse_roi(spec)?),
        None => base.crop,
    };
    let started_at = chrono::Utc::now();
    let hint = framewatch::session::target_hint(&target);
    let writer = PackageWriter::new(&out_dir, started_at, &hint).context("creating package dir")?;
    let dir = writer.recording().dir.clone();

    // 4. Stop on Ctrl+C or after --duration.
    let stop = Arc::new(AtomicBool::new(false));
    {
        let stop = stop.clone();
        let _ = ctrlc::set_handler(move || stop.store(true, Ordering::SeqCst));
    }
    if let Some(secs) = args.duration {
        let stop = stop.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(secs.saturating_mul(1000)));
            stop.store(true, Ordering::SeqCst);
        });
    }

    // 5. Record until stopped.
    println!("framewatch: recording to {}", dir.display());
    match args.duration {
        Some(secs) => println!("framewatch: will stop after {secs}s (or Ctrl+C)."),
        None => println!("framewatch: press Ctrl+C to stop."),
    }
    let rcfg = RecordConfig {
        target: target.clone(),
        crop,
        fps: args.fps,
        mic: args.mic.clone(),
        capture_audio: !args.no_audio,
        video_out: writer.recording().video_path(),
        audio_out: writer.recording().audio_path(),
        work_dir: dir.clone(),
        wait_ms: args
            .wait
            .map(|s| s.saturating_mul(1000))
            .unwrap_or(base.wait_ms),
        stop,
    };
    let outcome = record(rcfg);

    // Always tear down a launched child before reporting.
    if let Some(c) = child.as_mut() {
        let _ = c.kill();
        let _ = c.wait();
    }
    let outcome = outcome.context("recording")?;

    // 6. Transcribe — only if audio was actually captured. A failure is
    //    non-fatal: keep the captured media and write a package without a
    //    transcript.
    let (transcript, engine, model) = match &outcome.audio {
        Some(_) => match transcriber.transcribe(&writer.recording().audio_path(), &dir) {
            Ok(t) => {
                let (engine, model) = transcriber.engine_meta();
                (t, engine, model)
            }
            Err(e) => {
                eprintln!(
                    "framewatch: transcription failed ({e}); writing package without a transcript."
                );
                (framewatch::Transcript::default(), "none", None)
            }
        },
        None => {
            eprintln!("framewatch: no audio was recorded; the package is video-only.");
            (framewatch::Transcript::default(), "none", None)
        }
    };

    // 7. Assemble + write the package.
    writer
        .write_transcript(&transcript)
        .context("writing transcript")?;
    let audio_meta = outcome.audio.as_ref().map(|a| AudioMeta {
        path: files::AUDIO.to_string(),
        sample_rate: a.sample_rate,
        channels: a.channels,
        duration_ms: a.duration_ms,
    });
    let mut manifest = RecordingManifest::new(
        writer.recording(),
        &target,
        "cli",
        VideoMeta {
            path: files::VIDEO.to_string(),
            container: outcome.container.clone(),
            codec: outcome.codec.clone(),
            fps: outcome.fps,
            width: outcome.width,
            height: outcome.height,
            duration_ms: outcome.video_duration_ms,
        },
        audio_meta,
        &transcript,
        engine,
        model,
        outcome.ended_at,
    );
    // Enrich the target descriptor with the resolved window's real title/exe.
    if !outcome.window_title.is_empty() {
        manifest.target.title = Some(outcome.window_title.clone());
    }
    if !outcome.window_exe.is_empty() {
        manifest.target.exe = Some(outcome.window_exe.clone());
    }
    writer
        .finalize(&manifest, &transcript)
        .context("writing package")?;

    println!(
        "framewatch: wrote recording package to {} ({} transcript segment(s))",
        dir.display(),
        transcript.segments.len()
    );
    // The prompt path on its own line, machine-readable for scripts.
    println!("{}", writer.recording().prompt_path().display());
    Ok(())
}

#[cfg(not(feature = "record"))]
fn cmd_record(_args: RecordArgs) -> Result<()> {
    anyhow::bail!(
        "this build has no recording support. Reinstall with \
         `cargo install framewatch --features \"cli wgc record\"` (Windows; needs ffmpeg on PATH)."
    )
}

#[cfg(feature = "gui")]
fn cmd_gui(args: GuiArgs) -> Result<()> {
    let config = match &args.config {
        Some(path) => Some(Config::from_toml_path(path).context("loading config")?),
        None => None,
    };
    framewatch::gui::run(config).context("running gui")?;
    Ok(())
}

#[cfg(not(feature = "gui"))]
fn cmd_gui(_args: GuiArgs) -> Result<()> {
    anyhow::bail!(
        "this build has no GUI. Reinstall with `cargo install framewatch --features gui` \
         (and `wgc` on Windows for live capture)."
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_roi_ok_and_errors() {
        let r = parse_roi("10,20,300,200").unwrap();
        assert_eq!((r.x, r.y, r.w, r.h), (10, 20, 300, 200));
        assert!(parse_roi("1,2,3").is_err());
        assert!(parse_roi("a,b,c,d").is_err());
    }
}
