//! The `framewatch` CLI: `windows`, `watch`, `shot`, `record`, and `gui` subcommands.

use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand};
use framewatch::{
    CaptureEvent, ChannelSink, Config, DirectorySink, EncodedImage, EventKind, Target,
};
use std::path::PathBuf;

#[cfg(feature = "record")]
const DEFAULT_RECORD_LAUNCH_WAIT_MS: u64 = 15_000;

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
    /// Install and inspect managed transcription dependencies.
    Transcriber(TranscriberArgs),
    /// Launch the GUI picker / ROI editor.
    Gui(GuiArgs),
    /// Append an application label to a session's timeline, so the next frame
    /// is captioned with it.
    Mark(MarkArgs),
}

#[derive(Args)]
struct MarkArgs {
    /// The label, e.g. "before-checkout".
    #[arg(long, short)]
    label: String,
    /// Session directory. Defaults to the most recent session under `--out`.
    #[arg(long)]
    session: Option<PathBuf>,
    /// Where sessions live, when `--session` is not given (default ./.framewatch).
    #[arg(long)]
    out: Option<PathBuf>,
    /// A JSON blob to carry alongside the label.
    #[arg(long, value_name = "JSON")]
    json: Option<String>,
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
    /// Tail this newline-delimited file; each line becomes a `mark` event and
    /// captions the next frame. A line may be plain text or a JSON object.
    #[arg(long, value_name = "PATH")]
    labels_file: Option<PathBuf>,
    /// Per-tile luma delta needed to count a tile as changed (0-255, default 12).
    /// Lower it to notice small, low-contrast changes.
    #[arg(long, value_name = "N")]
    tile_change_threshold: Option<u8>,
    /// Fraction of the frame that must change to count as meaningful activity
    /// (default 0.002). Lower it for a thin strip like a status banner.
    #[arg(long, value_name = "RATIO")]
    min_area_ratio: Option<f32>,
    /// Tile grid as `COLSxROWS` (default 32x18). A finer grid makes a small
    /// change a larger fraction of one tile, so it survives thresholding.
    #[arg(long, value_name = "COLSxROWS")]
    tile_grid: Option<String>,
}

/// Parse a `COLSxROWS` tile grid.
fn parse_tile_grid(spec: &str) -> Result<(u16, u16)> {
    let (cols, rows) = spec
        .split_once(['x', 'X'])
        .with_context(|| format!("--tile-grid must be COLSxROWS, got: {spec:?}"))?;
    Ok((
        cols.trim().parse().context("--tile-grid COLS")?,
        rows.trim().parse().context("--tile-grid ROWS")?,
    ))
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
    /// `--launch` defaults to 15 seconds when no config wait is set.
    #[arg(long)]
    wait: Option<u64>,
    /// Auto-stop N seconds after encoder startup (otherwise record until Ctrl+C).
    #[arg(long, value_parser = clap::value_parser!(u64).range(1..))]
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
    /// Override managed Whisper by shelling out to another local transcriber.
    /// `{audio}` and `{output}` are substituted; the command must emit
    /// framewatch transcript JSON or SRT.
    #[arg(long, value_name = "CMD")]
    transcribe_cmd: Option<String>,
    /// Skip transcription and managed Whisper setup (record video + audio only).
    #[arg(long, conflicts_with = "transcribe_cmd")]
    no_transcribe: bool,
    /// Load a base config from this TOML file (for `out`/`target`/`roi` defaults).
    #[arg(long)]
    config: Option<PathBuf>,
}

#[derive(Args)]
struct TranscriberArgs {
    #[command(subcommand)]
    command: TranscriberCommand,
}

#[derive(Subcommand)]
enum TranscriberCommand {
    /// Download, verify, and validate managed whisper.cpp and its model.
    Setup,
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
        Command::Transcriber(args) => cmd_transcriber(args),
        Command::Gui(args) => cmd_gui(args),
        Command::Mark(args) => cmd_mark(args),
    }
}

/// The newest session directory under `root`, by directory name.
///
/// Session ids start with a sortable timestamp, so lexical order is
/// chronological — and reading the name beats reading mtimes, which a running
/// watcher keeps touching for every session it has ever written to.
fn latest_session(root: &std::path::Path) -> Result<PathBuf> {
    let mut best: Option<(String, PathBuf)> = None;
    let entries = std::fs::read_dir(root)
        .with_context(|| format!("reading sessions under {}", root.display()))?;
    for entry in entries.flatten() {
        if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        // Only a real session — a stray directory would give a confusing error
        // later, when the mark went somewhere nothing reads.
        if !entry.path().join("session.json").exists() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if best.as_ref().is_none_or(|(current, _)| name > *current) {
            best = Some((name, entry.path()));
        }
    }
    match best {
        Some((_, path)) => Ok(path),
        None => anyhow::bail!(
            "no framewatch session found under {}. Pass --session, or start `framewatch watch` first.",
            root.display()
        ),
    }
}

fn cmd_mark(args: MarkArgs) -> Result<()> {
    let session_dir = match args.session {
        Some(dir) => dir,
        None => {
            let root = args.out.unwrap_or_else(|| PathBuf::from(".framewatch"));
            latest_session(&root)?
        }
    };
    if !session_dir.join("session.json").exists() {
        anyhow::bail!(
            "{} does not look like a framewatch session (no session.json)",
            session_dir.display()
        );
    }

    let started_at = session_started_at(&session_dir);
    let mut record = framewatch::MarkRecord::new(args.label);
    if let Some(raw) = args.json.as_deref() {
        let value: serde_json::Value =
            serde_json::from_str(raw).context("--json must be valid JSON")?;
        record = record.with_data(value);
    }
    if let Some((id, started)) = started_at {
        record = record.in_session(&id, started);
    }

    // The timeline line is written here, not by the watcher: a mark has to
    // survive whether or not one is running.
    framewatch::mark::append_to_timeline(&session_dir, &record)
        .context("appending to timeline.jsonl")?;
    // Best effort: only a live watcher reads this, and its absence just means
    // the next frame is not captioned.
    let _ = framewatch::mark::notify_pending(&session_dir, &record);

    println!("{}", session_dir.join("timeline.jsonl").display());
    Ok(())
}

/// Read the session id and start time out of `session.json`.
fn session_started_at(dir: &std::path::Path) -> Option<(String, chrono::DateTime<chrono::Utc>)> {
    let raw = std::fs::read_to_string(dir.join("session.json")).ok()?;
    let value: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let id = value
        .get("session_id")
        .and_then(|v| v.as_str())?
        .to_string();
    let started = value.get("started_at").and_then(|v| v.as_str())?;
    let parsed = chrono::DateTime::parse_from_rfc3339(started).ok()?;
    Some((id, parsed.with_timezone(&chrono::Utc)))
}

#[cfg(feature = "record")]
fn cmd_transcriber(args: TranscriberArgs) -> Result<()> {
    match args.command {
        TranscriberCommand::Setup => {
            eprintln!(
                "framewatch: installing managed whisper.cpp {} / {} (~150 MiB on first use)",
                framewatch::WHISPER_VERSION,
                framewatch::WHISPER_MODEL
            );
            let managed = framewatch::ensure_managed_whisper()
                .context("preparing managed Whisper transcription")?;
            println!("whisper_cli={}", managed.executable.display());
            println!("model={}", managed.model.display());
            Ok(())
        }
    }
}

#[cfg(not(feature = "record"))]
fn cmd_transcriber(_args: TranscriberArgs) -> Result<()> {
    anyhow::bail!(
        "managed transcription requires the `record` feature. Reinstall with \
         `cargo install framewatch --features \"wgc record\"`."
    )
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
    if let Some(threshold) = args.tile_change_threshold {
        config.tile_change_threshold = threshold;
    }
    if let Some(ratio) = args.min_area_ratio {
        config.meaningful_area_ratio = ratio;
    }
    if let Some(spec) = args.tile_grid.as_deref() {
        config.tile_grid = parse_tile_grid(spec)?;
    }

    config.validate().context("invalid configuration")?;

    let mut sink = DirectorySink::new(&config).context("creating output sink")?;
    let dir = sink.session().dir.clone();

    // The sink already follows this session's `marks.pending`, so a
    // `framewatch mark` in another process needs no wiring here.
    if let Some(path) = args.labels_file.clone() {
        println!("framewatch: tailing labels from {}", path.display());
        sink.tail_labels(path);
    }

    println!("framewatch: writing session to {}", dir.display());
    println!("framewatch: press Ctrl+C to stop.");

    framewatch::watch(config, sink).context("capture loop")?;
    Ok(())
}

fn cmd_shot(args: ShotArgs) -> Result<()> {
    // 1. Optionally launch the target program; we capture its window by PID. The
    //    guard tears the child down on *every* exit path, including a config or
    //    `--roi` error below (so a bad flag can't orphan the launched program).
    let child = match &args.launch {
        Some(cmd) => Some(ChildGuard::new(
            spawn_launch(cmd).context("launching --launch command")?,
        )),
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

    // 4. Tear down the launched process before reporting (the guard also covers
    //    any earlier error path).
    drop(child);
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

/// Kills (and reaps) a `--launch`ed child on drop, so the launched program is
/// never orphaned if a later step (config load, `--roi` parse, validation, …)
/// fails between spawning it and the explicit teardown.
struct ChildGuard(Option<std::process::Child>);

impl ChildGuard {
    fn new(child: std::process::Child) -> Self {
        Self(Some(child))
    }

    /// The launched process id (used to target capture by PID).
    fn id(&self) -> u32 {
        self.0.as_ref().map(|c| c.id()).unwrap_or(0)
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if let Some(mut c) = self.0.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

#[cfg(feature = "record")]
fn effective_record_wait_ms(
    explicit_wait_seconds: Option<u64>,
    config_wait_ms: u64,
    launched: bool,
) -> u64 {
    explicit_wait_seconds
        .map(|seconds| seconds.saturating_mul(1000))
        .unwrap_or_else(|| {
            if launched && config_wait_ms == 0 {
                DEFAULT_RECORD_LAUNCH_WAIT_MS
            } else {
                config_wait_ms
            }
        })
}

#[cfg(feature = "record")]
fn cmd_record(args: RecordArgs) -> Result<()> {
    use framewatch::recording::{files, AudioMeta, VideoMeta};
    use framewatch::{
        record, record_with_duration, PackageWriter, RecordConfig, RecordingManifest, Transcriber,
    };
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    if args.duration == Some(0) {
        anyhow::bail!("--duration must be at least 1 second");
    }

    // Base config supplies out_dir / target / crop defaults.
    let base = match &args.config {
        Some(path) => Config::from_toml_path(path).context("loading config")?,
        None => Config::default(),
    };

    // 1. Optionally launch the target; capture its window by PID. The guard tears
    //    the child down on every exit path (incl. a parse/setup error below).
    let child = match &args.launch {
        Some(cmd) => Some(ChildGuard::new(
            spawn_launch(cmd).context("launching --launch command")?,
        )),
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
    //    (there's nothing to transcribe). With no override, provision the pinned
    //    managed whisper.cpp runtime/model before launching capture resources.
    let transcriber = if args.no_transcribe || args.no_audio {
        Transcriber::Disabled
    } else if let Some(cmd) = args.transcribe_cmd.clone() {
        Transcriber::Command { template: cmd }
    } else {
        eprintln!(
            "framewatch: preparing managed Whisper transcription (first use downloads ~150 MiB; \
             pass --no-transcribe to opt out)."
        );
        let managed = framewatch::ensure_managed_whisper()
            .context("preparing managed Whisper transcription")?;
        eprintln!(
            "framewatch: using whisper.cpp {} with model {}",
            framewatch::WHISPER_VERSION,
            framewatch::WHISPER_MODEL
        );
        Transcriber::ManagedWhisper {
            executable: managed.executable,
            model: managed.model,
        }
    };

    // 3. Install the stop handler before creating output or starting resources.
    let stop = Arc::new(AtomicBool::new(false));
    {
        let stop = stop.clone();
        ctrlc::set_handler(move || stop.store(true, Ordering::SeqCst))
            .context("installing Ctrl+C handler")?;
    }

    // 4. Output package directory.
    let out_dir = args.out.unwrap_or(base.out_dir);
    let crop = match args.roi.as_deref() {
        Some(spec) => Some(parse_roi(spec)?),
        None => base.crop,
    };
    let started_at = chrono::Utc::now();
    let hint = framewatch::session::target_hint(&target);
    let writer = PackageWriter::new(&out_dir, started_at, &hint).context("creating package dir")?;
    let dir = writer.recording().dir.clone();

    // 5. Record until stopped. A bounded duration begins after the first frame
    //    and encoder startup, rather than consuming target/window wait time.
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
        wait_ms: effective_record_wait_ms(args.wait, base.wait_ms, child.is_some()),
        stop,
    };
    let outcome = match args.duration {
        Some(secs) => record_with_duration(rcfg, std::time::Duration::from_secs(secs)),
        None => record(rcfg),
    };

    // Tear down the launched child before reporting (the guard also covers any
    // earlier error path).
    drop(child);
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

    #[test]
    fn record_duration_rejects_zero() {
        assert!(Cli::try_parse_from([
            "framewatch",
            "record",
            "--title",
            "test",
            "--duration",
            "0",
        ])
        .is_err());
        assert!(Cli::try_parse_from([
            "framewatch",
            "record",
            "--title",
            "test",
            "--duration",
            "1",
        ])
        .is_ok());
    }

    #[cfg(feature = "record")]
    #[test]
    fn record_launch_waits_for_the_child_window_by_default() {
        assert_eq!(effective_record_wait_ms(None, 0, true), 15_000);
        assert_eq!(effective_record_wait_ms(None, 2_500, true), 2_500);
        assert_eq!(effective_record_wait_ms(Some(7), 2_500, true), 7_000);
        assert_eq!(effective_record_wait_ms(None, 0, false), 0);
    }

    #[test]
    fn managed_transcriber_setup_command_parses() {
        assert!(Cli::try_parse_from(["framewatch", "transcriber", "setup"]).is_ok());
    }

    /// A portable long-running child so we can prove the guard kills it.
    #[cfg(windows)]
    fn sleeper() -> std::process::Command {
        let mut c = std::process::Command::new("ping");
        c.args(["-n", "30", "127.0.0.1"])
            .stdout(std::process::Stdio::null());
        c
    }
    #[cfg(not(windows))]
    fn sleeper() -> std::process::Command {
        let mut c = std::process::Command::new("sleep");
        c.arg("30");
        c
    }

    #[test]
    fn child_guard_kills_on_drop() {
        let start = std::time::Instant::now();
        {
            let guard = ChildGuard::new(sleeper().spawn().expect("spawn sleeper"));
            assert!(guard.id() > 0);
        } // Drop kills + reaps; must not block for the child's full lifetime.
        assert!(
            start.elapsed().as_secs() < 5,
            "ChildGuard::drop should kill the child promptly"
        );
    }
}
