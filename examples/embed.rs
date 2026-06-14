//! Library embedding example, driven by the [`MockBackend`] so it runs on any OS
//! in CI.
//!
//! On Windows (built with `--features wgc`) you would swap the mock backend for
//! `framewatch::default_backend(&config)?` to capture a live window — the loop
//! below is identical.
//!
//! Run with: `cargo run --example embed`

use framewatch::{
    CaptureBackend, Config, ControlFlow, DirectorySink, Engine, MockBackend, Sink, SystemClock,
    Target, WindowInfo,
};
use std::time::Duration;

const W: u32 = 320;
const H: u32 = 180;

fn solid(b: u8, g: u8, r: u8) -> Vec<u8> {
    let mut buf = vec![0u8; (W * H * 4) as usize];
    for px in buf.chunks_exact_mut(4) {
        px[0] = b;
        px[1] = g;
        px[2] = r;
        px[3] = 255;
    }
    buf
}

#[allow(clippy::too_many_arguments)]
fn paint(buf: &mut [u8], x: u32, y: u32, w: u32, h: u32, b: u8, g: u8, r: u8) {
    for yy in y..(y + h).min(H) {
        for xx in x..(x + w).min(W) {
            let off = ((yy * W + xx) * 4) as usize;
            buf[off] = b;
            buf[off + 1] = g;
            buf[off + 2] = r;
            buf[off + 3] = 255;
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = Config::builder()
        .target(Target::ByTitleRegex("Visual Studio Code".into()))
        .out_dir("./.framewatch")
        .settle_ms(150)
        .spinner_roi("test-runner", [0.02, 0.94, 0.04, 0.05])
        .ignore_roi("clock", [0.92, 0.0, 0.08, 0.03])
        .build()?;

    let mut engine = Engine::new(config.clone(), SystemClock);
    let mut sink = DirectorySink::new(&config)?;
    let session_dir = sink.session().dir.clone();

    // Build a small synthetic scenario: idle → a change → settle.
    let window = WindowInfo::synthetic("Visual Studio Code", W, H);
    let mut frames: Vec<(u32, u32, Vec<u8>)> = Vec::new();
    frames.push((W, H, solid(30, 30, 30))); // initial
    let mut changed = solid(30, 30, 30);
    paint(&mut changed, 40, 30, 220, 90, 40, 180, 80);
    for _ in 0..10 {
        frames.push((W, H, changed.clone())); // a change, then held static -> settles
    }

    let mut backend = MockBackend::from_bgra_frames(frames, Duration::from_millis(33), window);

    // On Windows: `let mut backend = framewatch::default_backend(&config)?;`
    backend.run(&mut |frame| {
        for event in engine.process(&frame, frame.captured_at) {
            sink.on_event(&event).ok();
        }
        ControlFlow::Continue
    })?;
    sink.flush()?;

    println!("framewatch session written to: {}", session_dir.display());
    Ok(())
}
