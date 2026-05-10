//! Windows system audio capture via WASAPI loopback.
//!
//! WASAPI loopback runs the render endpoint in capture mode — it gives us
//! the post-mix audio that's about to leave the speakers, which is exactly
//! what the user hears (Zoom, browser audio, music, system sounds). This
//! is the Windows analogue of macOS ScreenCaptureKit's audio output.
//!
//! Format: WASAPI hands us the device's *mix format* — almost always
//! 48 kHz Float32 stereo on modern Windows. We mix to mono and resample
//! to 16 kHz inside the capture loop, then push to the same writers the
//! mic side uses.
//!
//! Threading: WASAPI's `capture_client.read_from_device` is blocking, so
//! we run it on a dedicated OS thread. Pause/resume is a flag check at
//! the top of each loop iteration; stop drops the loop and lets the
//! thread join.

#![cfg(windows)]

use crate::audio::{ChunkWriter, FullRecordingWriter};
use crate::events::emit_error;
use crate::resample::{Resampler, TARGET_RATE};
use crate::stats::Stats;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;
use wasapi::{initialize_mta, AudioClient, Direction, StreamMode, WaveFormat};

pub struct SystemCapture {
    paused: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl SystemCapture {
    pub fn pause(&self) {
        self.paused.store(true, Ordering::Release);
    }
    pub fn resume(&self) {
        self.paused.store(false, Ordering::Release);
    }
    /// Hand out a clone of the pause atomic so callers can flip it without
    /// going through `pause()`/`resume()` indirection. Used by main's
    /// stdin-IPC reader to dispatch pause across mic + sys uniformly.
    pub fn pause_flag(&self) -> Arc<AtomicBool> {
        self.paused.clone()
    }
    pub fn stop(mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

impl Drop for SystemCapture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

pub fn start(
    chunks: Arc<ChunkWriter>,
    full: Arc<FullRecordingWriter>,
    stats: Arc<Stats>,
) -> anyhow::Result<SystemCapture> {
    let paused = Arc::new(AtomicBool::new(false));
    let stop = Arc::new(AtomicBool::new(false));
    let paused_t = paused.clone();
    let stop_t = stop.clone();

    let handle = std::thread::Builder::new()
        .name("wasapi-loopback".into())
        .spawn(move || {
            if let Err(e) = run_loopback(paused_t, stop_t, chunks, full, stats) {
                emit_error(format!("system loopback: {e}"));
            }
        })?;

    Ok(SystemCapture {
        paused,
        stop,
        handle: Some(handle),
    })
}

fn run_loopback(
    paused: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    chunks: Arc<ChunkWriter>,
    full: Arc<FullRecordingWriter>,
    stats: Arc<Stats>,
) -> anyhow::Result<()> {
    // COM init for this thread. MTA matches WASAPI's expectation for shared
    // capture; the wasapi crate handles the marshalling internals.
    initialize_mta().ok()?;

    // Default render endpoint, captured in loopback mode. `Direction::Render`
    // + `StreamMode::Loopback` is the WASAPI idiom for "give me what's
    // playing back."
    let device = wasapi::get_default_device(&Direction::Render)?;
    let mut audio_client: AudioClient = device.get_iaudioclient()?;

    let mix_format: WaveFormat = audio_client.get_mixformat()?;
    let in_rate = mix_format.get_samplespersec();
    let channels = mix_format.get_nchannels() as usize;
    let bits = mix_format.get_bitspersample();
    let sub = mix_format.get_subformat()?;

    eprintln!(
        "audio-capture: system loopback rate={} channels={} bits={} subformat={:?}",
        in_rate, channels, bits, sub
    );

    // 100 ms buffer is the WASAPI sweet spot for shared mode — small enough
    // for low latency, large enough to avoid underruns under typical scheduler
    // jitter. autoconvert=true lets WASAPI resample if the device's mix format
    // disagrees with our requested format; we feed it the device's own format
    // so it's a no-op in practice but harmless to enable.
    audio_client.initialize_client(
        &mix_format,
        &Direction::Capture,
        &StreamMode::EventsShared {
            autoconvert: true,
            buffer_duration_hns: 1_000_000,
        },
    )?;
    let h_event = audio_client.set_get_eventhandle()?;
    let capture_client = audio_client.get_audiocaptureclient()?;
    let buffer_frame_count = audio_client.get_bufferframecount()?;

    audio_client.start_stream()?;

    let mut resampler = Resampler::new(in_rate, TARGET_RATE);
    // Scratch buffer for one device-period's worth of frames. Pre-allocated
    // so the per-iteration path doesn't touch the allocator.
    let bytes_per_frame = (bits as usize / 8) * channels;
    let mut scratch: Vec<u8> = Vec::with_capacity(buffer_frame_count as usize * bytes_per_frame * 2);

    while !stop.load(Ordering::Acquire) {
        // Wait up to the device period for new audio. If it times out we
        // fall through and re-check the stop flag — keeps shutdown
        // responsive even when no audio is playing.
        if h_event.wait_for_event(150).is_err() {
            continue;
        }

        // Drain the capture buffer in this tick. read_from_device reads
        // exactly one packet (one device period); loop until empty.
        loop {
            let next_packet_frames = match capture_client.get_next_packet_size() {
                Ok(Some(n)) => n,
                Ok(None) => break, // no more packets queued
                Err(e) => {
                    emit_error(format!("get_next_packet_size: {e}"));
                    break;
                }
            };
            if next_packet_frames == 0 {
                break;
            }
            scratch.clear();
            scratch.resize(next_packet_frames as usize * bytes_per_frame, 0);
            let (frames_read, _flags) =
                capture_client.read_from_device(&mut scratch)?;
            if frames_read == 0 {
                continue;
            }
            if paused.load(Ordering::Acquire) {
                continue;
            }
            // Convert raw bytes → Float32. Mix format is virtually always
            // float32 in modern Windows; if the device reports int16/int24
            // the user can still speak (mic), they just won't get system
            // audio for this session.
            let n_floats = frames_read as usize * channels;
            let mut as_f32: Vec<f32> = Vec::with_capacity(n_floats);
            if bits == 32 {
                // Float32 IEEE 754 little-endian
                for chunk in scratch.chunks_exact(4).take(n_floats) {
                    as_f32.push(f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]));
                }
            } else if bits == 16 {
                for chunk in scratch.chunks_exact(2).take(n_floats) {
                    let s = i16::from_le_bytes([chunk[0], chunk[1]]);
                    as_f32.push(s as f32 / i16::MAX as f32);
                }
            } else {
                emit_error(format!(
                    "unsupported loopback bit depth: {bits}"
                ));
                stop.store(true, Ordering::Release);
                break;
            }

            let mono16k = resampler.process_f32(&as_f32, channels);
            if mono16k.is_empty() {
                continue;
            }
            stats.add_sys(&mono16k);
            chunks.write(&mono16k);
            full.write(&mono16k);
        }
    }

    let _ = audio_client.stop_stream();
    Ok(())
}

// On non-Windows we provide an empty SystemCapture stub so main.rs can stay
// platform-agnostic at the call site. The build-script gate (`#![cfg(windows)]`
// at the top of this file) keeps the real impl Windows-only; main.rs gates
// the `mod system_win` declaration accordingly.
