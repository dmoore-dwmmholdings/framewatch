//! The `framewatch` CLI: `windows`, `watch`, and `gui` subcommands.

use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand};
use framewatch::{Config, DirectorySink, Target};
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
