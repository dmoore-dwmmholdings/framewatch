//! The `framewatch` CLI: `windows`, `watch`, `shot`, and `gui` subcommands.

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
    /// Launch the GUI picker / ROI editor.
    Gui(GuiArgs),
}

#[derive(Args)]
struct WatchArgs {
    /// Match the window title against this regex.
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
    /// Match the window title against this regex.
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
    let tokens = tokenize(cmd);
    let (program, rest) = tokens
        .split_first()
        .ok_or_else(|| anyhow::anyhow!("--launch is empty"))?;
    std::process::Command::new(program)
        .args(rest)
        .spawn()
        .map_err(Into::into)
}

/// Whitespace-split a command, respecting `"double quotes"`.
fn tokenize(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_quote = false;
    for ch in s.chars() {
        match ch {
            '"' => in_quote = !in_quote,
            c if c.is_whitespace() && !in_quote => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
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
    fn tokenize_respects_quotes_and_whitespace() {
        assert_eq!(
            tokenize("game.exe --freecam --pos \"1 2 3\""),
            vec!["game.exe", "--freecam", "--pos", "1 2 3"]
        );
        assert_eq!(tokenize("   a   b  "), vec!["a", "b"]);
        assert!(tokenize("   ").is_empty());
    }

    #[test]
    fn parse_roi_ok_and_errors() {
        let r = parse_roi("10,20,300,200").unwrap();
        assert_eq!((r.x, r.y, r.w, r.h), (10, 20, 300, 200));
        assert!(parse_roi("1,2,3").is_err());
        assert!(parse_roi("a,b,c,d").is_err());
    }
}
