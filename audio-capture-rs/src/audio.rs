//! WAV writers shared between the mic and system streams.
//!
//! Mirrors the Swift sidecar's `ChunkWriter` + `FullRecordingWriter`:
//! - `ChunkWriter` rotates on natural speech pauses (VAD: 1.0–15.0 s with
//!   500 ms silence trigger). Drops chunks whose peak fell below the
//!   silence threshold so quiet tails don't waste a Whisper invocation.
//! - `FullRecordingWriter` captures every received frame into a single WAV
//!   for the duration of the recording. The post-stop diarize pass reads
//!   it; the playback view also reads it (when `keep_audio` is on).
//!
//! Both writers serialise on an internal mutex so the audio thread can
//! call `write` without coordination from outside.

use crate::events::{emit, emit_error};
use crate::resample::TARGET_RATE;
use crate::stats::Stats;
use hound::{SampleFormat, WavSpec, WavWriter};
use parking_lot::Mutex;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

const TARGET_RATE_F: f32 = TARGET_RATE as f32;

fn wav_spec() -> WavSpec {
    // 16-bit PCM mono at TARGET_RATE — same shape Whisper-rs expects after
    // we hand it the audio, and what the macOS sidecar writes.
    WavSpec {
        channels: 1,
        sample_rate: TARGET_RATE,
        bits_per_sample: 16,
        sample_format: SampleFormat::Int,
    }
}

fn f32_to_i16(s: f32) -> i16 {
    let clamped = s.clamp(-1.0, 1.0);
    (clamped * i16::MAX as f32) as i16
}

/// VAD-bounded chunk writer. One instance per source ("mic" / "sys").
pub struct ChunkWriter {
    source: &'static str,
    dir: PathBuf,
    min_frames: u32,
    max_frames: u32,
    vad_silence_frames: u32,
    silence_threshold: f32, // chunk-level: drop if peak below
    vad_frame_threshold: f32, // per-buffer peak: above this counts as voice
    inner: Mutex<ChunkWriterInner>,
    stats: Arc<Stats>,
}

struct ChunkWriterInner {
    index: u32,
    file: Option<WavWriter<std::io::BufWriter<std::fs::File>>>,
    url: Option<PathBuf>,
    written: u32,
    chunk_peak: f32,
    silent_run: u32,
    /// Total frames written across ALL chunks since this writer opened.
    /// Each chunk's `start_ms` is computed from the value at the moment
    /// the chunk's first sample landed — i.e. the cumulative offset
    /// before this chunk started.
    total_frames_written: u64,
    chunk_start_frames: u64,
}

impl ChunkWriter {
    pub fn new(
        source: &'static str,
        dir: PathBuf,
        min_seconds: f32,
        max_seconds: f32,
        vad_silence_ms: f32,
        stats: Arc<Stats>,
    ) -> Self {
        Self {
            source,
            dir,
            min_frames: (min_seconds * TARGET_RATE_F) as u32,
            max_frames: (max_seconds * TARGET_RATE_F) as u32,
            vad_silence_frames: ((vad_silence_ms / 1000.0) * TARGET_RATE_F) as u32,
            silence_threshold: 0.005,
            vad_frame_threshold: 0.008,
            inner: Mutex::new(ChunkWriterInner {
                index: 0,
                file: None,
                url: None,
                written: 0,
                chunk_peak: 0.0,
                silent_run: 0,
                total_frames_written: 0,
                chunk_start_frames: 0,
            }),
            stats,
        }
    }

    pub fn write(&self, samples: &[f32]) {
        if samples.is_empty() {
            return;
        }
        let mut g = self.inner.lock();
        if g.file.is_none() {
            if let Err(e) = self.open_next(&mut g) {
                emit_error(format!("{} open: {e}", self.source));
                return;
            }
        }
        let file = g.file.as_mut().expect("file just opened");
        // Per-buffer peak feeds chunk_peak (chunk-level silence drop) and
        // silent_run (VAD rotation trigger).
        let mut buf_peak = 0.0f32;
        for &s in samples {
            let v = s.abs();
            if v > buf_peak {
                buf_peak = v;
            }
            if let Err(e) = file.write_sample(f32_to_i16(s)) {
                emit_error(format!("{} write_sample: {e}", self.source));
                break;
            }
        }
        let n = samples.len() as u32;
        g.written += n;
        g.total_frames_written += n as u64;
        if buf_peak > g.chunk_peak {
            g.chunk_peak = buf_peak;
        }
        if buf_peak < self.vad_frame_threshold {
            g.silent_run = g.silent_run.saturating_add(n);
        } else {
            g.silent_run = 0;
        }

        // Rotate on:
        //   - VAD pause (silent_run >= threshold) but only past min_frames.
        //   - Hard cap (max_frames) so a continuous monologue still emits.
        let vad_rotate = g.written >= self.min_frames && g.silent_run >= self.vad_silence_frames;
        if g.written >= self.max_frames || vad_rotate {
            if let Err(e) = self.rotate(&mut g) {
                emit_error(format!("{} rotate: {e}", self.source));
            }
        }
    }

    pub fn close(&self) {
        let mut g = self.inner.lock();
        if g.file.is_some() {
            if let Err(e) = self.flush_current(&mut g) {
                emit_error(format!("{} close: {e}", self.source));
            }
        }
    }

    fn open_next(&self, g: &mut ChunkWriterInner) -> anyhow::Result<()> {
        g.index += 1;
        let name = format!("{}-chunk-{:04}.wav", self.source, g.index);
        let path = self.dir.join(&name);
        let writer = WavWriter::create(&path, wav_spec())?;
        g.file = Some(writer);
        g.url = Some(path);
        g.written = 0;
        g.chunk_peak = 0.0;
        g.silent_run = 0;
        g.chunk_start_frames = g.total_frames_written;
        Ok(())
    }

    fn rotate(&self, g: &mut ChunkWriterInner) -> anyhow::Result<()> {
        self.flush_current(g)?;
        self.open_next(g)?;
        Ok(())
    }

    /// Finalize the current chunk file. Either emits a `chunk` event (if
    /// the chunk's peak passes the silence threshold) or deletes the file.
    fn flush_current(&self, g: &mut ChunkWriterInner) -> anyhow::Result<()> {
        let file = g.file.take();
        if let Some(writer) = file {
            writer.finalize()?;
        }
        let url = g.url.take();
        if let Some(path) = url {
            if g.written > 0 && g.chunk_peak >= self.silence_threshold {
                let start_ms = (g.chunk_start_frames as f64 / TARGET_RATE as f64 * 1000.0) as u64;
                emit(serde_json::json!({
                    "event": "chunk",
                    "source": self.source,
                    "path": path.to_string_lossy(),
                    "start_ms": start_ms,
                }));
                self.stats.inc_chunks();
            } else {
                let _ = fs::remove_file(&path);
            }
        }
        g.written = 0;
        g.chunk_peak = 0.0;
        g.silent_run = 0;
        Ok(())
    }
}

/// Per-source full recording writer. Captures every frame into a single
/// `<source>-full.wav`. Emits a `full_recording` event on close.
pub struct FullRecordingWriter {
    source: &'static str,
    dir: PathBuf,
    inner: Mutex<FullInner>,
}

struct FullInner {
    file: Option<WavWriter<std::io::BufWriter<std::fs::File>>>,
    url: Option<PathBuf>,
    written: u64,
}

impl FullRecordingWriter {
    pub fn new(source: &'static str, dir: PathBuf) -> Self {
        Self {
            source,
            dir,
            inner: Mutex::new(FullInner {
                file: None,
                url: None,
                written: 0,
            }),
        }
    }

    pub fn write(&self, samples: &[f32]) {
        if samples.is_empty() {
            return;
        }
        let mut g = self.inner.lock();
        if g.file.is_none() {
            let path = self.dir.join(format!("{}-full.wav", self.source));
            match WavWriter::create(&path, wav_spec()) {
                Ok(w) => {
                    g.file = Some(w);
                    g.url = Some(path);
                }
                Err(e) => {
                    emit_error(format!("{} full open: {e}", self.source));
                    return;
                }
            }
        }
        let file = g.file.as_mut().expect("file just opened");
        for &s in samples {
            if let Err(e) = file.write_sample(f32_to_i16(s)) {
                emit_error(format!("{} full write: {e}", self.source));
                break;
            }
        }
        g.written += samples.len() as u64;
    }

    pub fn close(&self) {
        let mut g = self.inner.lock();
        if let Some(writer) = g.file.take() {
            if let Err(e) = writer.finalize() {
                emit_error(format!("{} full finalize: {e}", self.source));
            }
        }
        if let Some(path) = g.url.take() {
            if g.written > 0 {
                let duration_ms = (g.written as f64 / TARGET_RATE as f64 * 1000.0) as u64;
                emit(serde_json::json!({
                    "event": "full_recording",
                    "source": self.source,
                    "path": path.to_string_lossy(),
                    "duration_ms": duration_ms,
                }));
            } else {
                // Empty WAV — no audio ever arrived from this source. Drop
                // the zero-byte file so the backend doesn't waste a
                // diarize call on it.
                let _ = fs::remove_file(&path);
            }
        }
    }
}

/// Ensure the output directory exists and is writable.
pub fn ensure_out_dir(dir: &Path) -> anyhow::Result<()> {
    fs::create_dir_all(dir)?;
    Ok(())
}
