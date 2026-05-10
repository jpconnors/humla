//! Microphone capture via cpal. Cross-platform (Windows / macOS / Linux);
//! the macOS Swift sidecar is the canonical implementation there, but we
//! keep cpal's mic path enabled on every target for parity testing.
//!
//! Format negotiation:
//!   - We ask cpal for the default input device's default config (typically
//!     44.1 kHz or 48 kHz, mono or stereo Float32). Any other sample format
//!     (i16 / u16) is converted on the fly inside the stream callback.
//!   - Output is mono Float32 at 16 kHz, mixed + resampled by `Resampler`.
//!
//! Pause / resume:
//!   - When `paused` is set, the callback discards audio and stops feeding
//!     the writers. The cpal stream itself stays alive — pausing/resuming
//!     a cpal stream re-opens the device on some backends, which can race
//!     with WASAPI's exclusive-mode locks and lose the device hand-off.

use crate::audio::{ChunkWriter, FullRecordingWriter};
use crate::events::emit_error;
use crate::resample::{Resampler, TARGET_RATE};
use crate::stats::Stats;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Sample, SampleFormat, Stream, StreamConfig};
use parking_lot::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Returned from `start()`. The `Stream` is `!Send` (cpal restriction on
/// every platform — the underlying audio backend pins to a single thread)
/// so the caller MUST keep it on the thread that called `start`. The
/// `paused` flag is `Send + Sync` and can move into the stdin-IPC reader so
/// `pause`/`resume` commands can flip it without touching the Stream.
pub struct MicCapture {
    /// Holding the Stream alive keeps the audio callback running. Dropping
    /// it stops capture cleanly.
    pub stream: Stream,
    /// Set true to make the audio callback discard incoming buffers without
    /// writing them. Acquire/Release ordering matches the callback's load.
    pub paused: Arc<AtomicBool>,
}

pub fn start(
    chunks: Arc<ChunkWriter>,
    full: Arc<FullRecordingWriter>,
    stats: Arc<Stats>,
) -> anyhow::Result<MicCapture> {
    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .ok_or_else(|| anyhow::anyhow!("no default input device"))?;
    let supported = device.default_input_config()?;
    let in_rate = supported.sample_rate().0;
    let channels = supported.channels() as usize;
    let format = supported.sample_format();
    let stream_config: StreamConfig = supported.into();

    eprintln!(
        "audio-capture: mic device='{}' rate={} channels={} format={:?}",
        device.name().unwrap_or_else(|_| "<unknown>".into()),
        in_rate,
        channels,
        format
    );

    let paused = Arc::new(AtomicBool::new(false));
    let resampler = Arc::new(Mutex::new(Resampler::new(in_rate, TARGET_RATE)));

    let err_fn = |e: cpal::StreamError| {
        emit_error(format!("mic stream: {e}"));
    };

    let stream = match format {
        SampleFormat::F32 => {
            let p = paused.clone();
            let r = resampler.clone();
            let c = chunks.clone();
            let f = full.clone();
            let s = stats.clone();
            device.build_input_stream(
                &stream_config,
                move |data: &[f32], _| handle(data, channels, &p, &r, &c, &f, &s),
                err_fn,
                None,
            )?
        }
        SampleFormat::I16 => {
            let p = paused.clone();
            let r = resampler.clone();
            let c = chunks.clone();
            let f = full.clone();
            let s = stats.clone();
            device.build_input_stream(
                &stream_config,
                move |data: &[i16], _| {
                    let f32_buf: Vec<f32> = data.iter().map(|x| x.to_float_sample()).collect();
                    handle(&f32_buf, channels, &p, &r, &c, &f, &s);
                },
                err_fn,
                None,
            )?
        }
        SampleFormat::U16 => {
            let p = paused.clone();
            let r = resampler.clone();
            let c = chunks.clone();
            let f = full.clone();
            let s = stats.clone();
            device.build_input_stream(
                &stream_config,
                move |data: &[u16], _| {
                    let f32_buf: Vec<f32> = data.iter().map(|x| x.to_float_sample()).collect();
                    handle(&f32_buf, channels, &p, &r, &c, &f, &s);
                },
                err_fn,
                None,
            )?
        }
        other => {
            return Err(anyhow::anyhow!("unsupported mic sample format: {other:?}"));
        }
    };

    stream.play()?;

    Ok(MicCapture { stream, paused })
}

fn handle(
    data: &[f32],
    channels: usize,
    paused: &AtomicBool,
    resampler: &Mutex<Resampler>,
    chunks: &ChunkWriter,
    full: &FullRecordingWriter,
    stats: &Stats,
) {
    if paused.load(Ordering::Acquire) {
        return;
    }
    if data.is_empty() {
        return;
    }
    let mono16k = resampler.lock().process_f32(data, channels);
    if mono16k.is_empty() {
        return;
    }
    stats.add_mic(&mono16k);
    chunks.write(&mono16k);
    full.write(&mono16k);
}
