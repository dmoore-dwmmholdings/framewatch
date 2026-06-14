//! Output sinks: the [`Sink`] trait, BGRA→PNG encoding, and composition.

pub mod channel;
pub mod directory;

pub use channel::ChannelSink;
pub use directory::DirectorySink;

use crate::config::ImageOpts;
use crate::error::SinkError;
use crate::event::{CaptureEvent, EncodedImage, ImageFormat};
use crate::frame::RawFrame;
use std::io::Cursor;

/// A destination for [`CaptureEvent`]s.
pub trait Sink: Send {
    /// Handle one event (write image + metadata, forward it, etc.).
    fn on_event(&mut self, event: &CaptureEvent) -> Result<(), SinkError>;

    /// Flush any buffered state. Called on shutdown.
    fn flush(&mut self) -> Result<(), SinkError> {
        Ok(())
    }
}

/// Fan-out sink: forwards every event to each child sink.
#[derive(Default)]
pub struct CompositeSink {
    sinks: Vec<Box<dyn Sink>>,
}

impl CompositeSink {
    /// An empty composite.
    pub fn new() -> Self {
        Self { sinks: Vec::new() }
    }

    /// Add a sink.
    pub fn push(&mut self, sink: Box<dyn Sink>) {
        self.sinks.push(sink);
    }

    /// Builder-style add.
    pub fn with(mut self, sink: Box<dyn Sink>) -> Self {
        self.sinks.push(sink);
        self
    }
}

impl Sink for CompositeSink {
    fn on_event(&mut self, event: &CaptureEvent) -> Result<(), SinkError> {
        let mut first_err = None;
        for s in &mut self.sinks {
            if let Err(e) = s.on_event(event) {
                if first_err.is_none() {
                    first_err = Some(e);
                }
            }
        }
        match first_err {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }

    fn flush(&mut self) -> Result<(), SinkError> {
        let mut first_err = None;
        for s in &mut self.sinks {
            if let Err(e) = s.flush() {
                if first_err.is_none() {
                    first_err = Some(e);
                }
            }
        }
        match first_err {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }
}

/// Encode a raw BGRA frame into an [`EncodedImage`] (the engine calls this once
/// per saved image so multiple sinks never re-encode).
pub fn encode(frame: &RawFrame, opts: &ImageOpts) -> Result<EncodedImage, SinkError> {
    // BGRA (with stride) -> tightly packed RGBA.
    let w = frame.width;
    let h = frame.height;
    // usize math avoids u32 overflow on very large (e.g. multi-4K) frames.
    let mut rgba = Vec::with_capacity(w as usize * h as usize * 4);
    for y in 0..h {
        let row = y as usize * frame.stride as usize;
        for x in 0..w {
            let off = row + (x * 4) as usize;
            let b = frame.buffer.get(off).copied().unwrap_or(0);
            let g = frame.buffer.get(off + 1).copied().unwrap_or(0);
            let r = frame.buffer.get(off + 2).copied().unwrap_or(0);
            let a = frame.buffer.get(off + 3).copied().unwrap_or(255);
            rgba.push(r);
            rgba.push(g);
            rgba.push(b);
            rgba.push(a);
        }
    }

    let buf = image::RgbaImage::from_raw(w, h, rgba)
        .ok_or_else(|| SinkError::Encode("frame buffer too small for dimensions".into()))?;
    let mut dynimg = image::DynamicImage::ImageRgba8(buf);

    if opts.scale > 0.0 && (opts.scale - 1.0).abs() > f32::EPSILON {
        let nw = ((w as f32) * opts.scale).round().max(1.0) as u32;
        let nh = ((h as f32) * opts.scale).round().max(1.0) as u32;
        dynimg = dynimg.resize_exact(nw, nh, image::imageops::FilterType::Triangle);
    }

    let out_w = dynimg.width();
    let out_h = dynimg.height();

    let bytes = match opts.format {
        ImageFormat::Webp => encode_webp(&dynimg)?,
        fmt => {
            let target = match fmt {
                ImageFormat::Png => image::ImageFormat::Png,
                ImageFormat::Jpeg => image::ImageFormat::Jpeg,
                ImageFormat::Webp => unreachable!(),
            };
            let mut out = Cursor::new(Vec::new());
            // JPEG has no alpha; drop it for that path.
            let to_write = if matches!(fmt, ImageFormat::Jpeg) {
                image::DynamicImage::ImageRgb8(dynimg.to_rgb8())
            } else {
                dynimg
            };
            to_write
                .write_to(&mut out, target)
                .map_err(|e| SinkError::Encode(e.to_string()))?;
            out.into_inner()
        }
    };

    Ok(EncodedImage {
        bytes,
        format: opts.format,
        width: out_w,
        height: out_h,
    })
}

#[cfg(feature = "webp")]
fn encode_webp(img: &image::DynamicImage) -> Result<Vec<u8>, SinkError> {
    let rgba = img.to_rgba8();
    let encoder = webp::Encoder::from_rgba(&rgba, img.width(), img.height());
    let mem = encoder.encode(80.0);
    Ok(mem.to_vec())
}

#[cfg(not(feature = "webp"))]
fn encode_webp(_img: &image::DynamicImage) -> Result<Vec<u8>, SinkError> {
    Err(SinkError::Encode(
        "webp output requires the `webp` feature".into(),
    ))
}
