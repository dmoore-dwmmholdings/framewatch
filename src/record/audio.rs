//! Microphone capture via `cpal`, written to a mono i16 WAV (`hound`) at the
//! device's native sample rate. The bundled whisper transcriber resamples to
//! 16 kHz when it reads the WAV; keeping the capture rate native gives the muxed
//! recording decent audio and keeps the realtime path a cheap streaming write.

use crate::audioutil::downmix_to_mono;
use crate::error::RecordError;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::SampleFormat;
use std::path::Path;
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Instant;

/// What the WAV writer reports back when capture stops.
pub(crate) struct AudioStats {
    /// WAV sample rate (the device's native rate).
    pub sample_rate: u32,
    /// WAV channel count (always 1 — we write mono).
    pub channels: u16,
    /// Audio duration in ms.
    pub duration_ms: u64,
    /// When the first sample arrived (for A/V start alignment).
    pub first_sample_at: Option<Instant>,
}

/// A live microphone recording. Dropping/finishing it stops capture.
pub(crate) struct AudioRecorder {
    // cpal's Stream is !Send; AudioRecorder is created and finished on one thread.
    stream: Option<cpal::Stream>,
    writer: Option<JoinHandle<Result<AudioStats, RecordError>>>,
    first_at: Arc<Mutex<Option<Instant>>>,
}

fn mark_first(first: &Arc<Mutex<Option<Instant>>>) {
    let mut g = first.lock().unwrap();
    if g.is_none() {
        *g = Some(Instant::now());
    }
}

impl AudioRecorder {
    /// Start capturing from `device_name` (or the default input) into `out_wav`.
    pub(crate) fn start(device_name: Option<&str>, out_wav: &Path) -> Result<Self, RecordError> {
        let host = cpal::default_host();
        let device = match device_name {
            Some(name) => host
                .input_devices()
                .map_err(|e| RecordError::Audio(e.to_string()))?
                .find(|d| {
                    d.description()
                        .map(|desc| {
                            let n = desc.name();
                            n.eq_ignore_ascii_case(name) || n.contains(name)
                        })
                        .unwrap_or(false)
                })
                .ok_or(RecordError::NoInputDevice)?,
            None => host
                .default_input_device()
                .ok_or(RecordError::NoInputDevice)?,
        };

        let supported = device
            .default_input_config()
            .map_err(|e| RecordError::Audio(e.to_string()))?;
        let sample_format = supported.sample_format();
        let in_rate = supported.sample_rate();
        let in_channels = supported.channels();
        let config: cpal::StreamConfig = supported.into();

        let (tx, rx) = channel::<Vec<f32>>();
        let first_at = Arc::new(Mutex::new(None));

        // WAV writer thread: downmix to mono i16 and stream to disk.
        let out = out_wav.to_path_buf();
        let writer = std::thread::spawn(move || -> Result<AudioStats, RecordError> {
            let spec = hound::WavSpec {
                channels: 1,
                sample_rate: in_rate,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            };
            let mut w = hound::WavWriter::create(&out, spec)
                .map_err(|e| RecordError::Wav(e.to_string()))?;
            let mut written: u64 = 0;
            for buf in rx.iter() {
                for s in downmix_to_mono(&buf, in_channels) {
                    let v = (s.clamp(-1.0, 1.0) * 32767.0) as i16;
                    w.write_sample(v)
                        .map_err(|e| RecordError::Wav(e.to_string()))?;
                    written += 1;
                }
            }
            w.finalize().map_err(|e| RecordError::Wav(e.to_string()))?;
            let duration_ms = if in_rate > 0 {
                written * 1000 / in_rate as u64
            } else {
                0
            };
            Ok(AudioStats {
                sample_rate: in_rate,
                channels: 1,
                duration_ms,
                first_sample_at: None,
            })
        });

        let err_fn = |e| tracing::warn!("framewatch: audio stream error: {e}");
        let stream = build_stream(
            &device,
            &config,
            sample_format,
            tx,
            first_at.clone(),
            err_fn,
        )?;
        stream
            .play()
            .map_err(|e| RecordError::Audio(e.to_string()))?;

        Ok(Self {
            stream: Some(stream),
            writer: Some(writer),
            first_at,
        })
    }

    /// Stop capture and finalize the WAV, returning its stats.
    pub(crate) fn finish(mut self) -> Result<AudioStats, RecordError> {
        // Drop the stream first: stops capture and drops the callback's Sender,
        // so the writer thread's channel disconnects and it finalizes the WAV.
        self.stream.take();
        let join = self
            .writer
            .take()
            .expect("audio writer thread present until finish()");
        let mut stats = join
            .join()
            .map_err(|_| RecordError::Audio("audio writer thread panicked".into()))??;
        stats.first_sample_at = *self.first_at.lock().unwrap();
        Ok(stats)
    }
}

/// Build an input stream for the device's native sample format, forwarding f32
/// sample buffers to the writer thread. The original `tx` is dropped after the
/// stream is built, so the writer ends when the (only remaining) callback Sender
/// is dropped on stop.
fn build_stream(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    sample_format: SampleFormat,
    tx: Sender<Vec<f32>>,
    first_at: Arc<Mutex<Option<Instant>>>,
    err_fn: impl FnMut(cpal::StreamError) + Send + 'static,
) -> Result<cpal::Stream, RecordError> {
    let stream = match sample_format {
        SampleFormat::F32 => {
            let tx = tx.clone();
            device.build_input_stream(
                config,
                move |data: &[f32], _: &cpal::InputCallbackInfo| {
                    mark_first(&first_at);
                    let _ = tx.send(data.to_vec());
                },
                err_fn,
                None,
            )
        }
        SampleFormat::I16 => {
            let tx = tx.clone();
            device.build_input_stream(
                config,
                move |data: &[i16], _: &cpal::InputCallbackInfo| {
                    mark_first(&first_at);
                    let _ = tx.send(data.iter().map(|&s| s as f32 / 32768.0).collect());
                },
                err_fn,
                None,
            )
        }
        SampleFormat::U16 => {
            let tx = tx.clone();
            device.build_input_stream(
                config,
                move |data: &[u16], _: &cpal::InputCallbackInfo| {
                    mark_first(&first_at);
                    let _ = tx.send(
                        data.iter()
                            .map(|&s| (s as f32 - 32768.0) / 32768.0)
                            .collect(),
                    );
                },
                err_fn,
                None,
            )
        }
        _ => return Err(RecordError::UnsupportedSampleFormat),
    };
    // Drop the original Sender so only the callback's clone keeps the channel open.
    drop(tx);
    stream.map_err(|e| RecordError::Audio(e.to_string()))
}
