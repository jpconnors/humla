//! WAV input for the diarizer.
//!
//! The Rust audio-capture sidecar writes 16-bit PCM mono at 16 kHz, and
//! the macOS Swift sidecar writes the same format. So we only need to
//! handle the one shape — but we still read defensively (channel-count
//! and sample-rate checks) so a wrong-shaped WAV produces a clear error
//! rather than nonsense diarization.

use anyhow::{anyhow, Context, Result};
use std::path::Path;

/// Read a 16 kHz mono WAV into Float32 samples normalised to [-1, 1].
/// Sherpa-onnx expects exactly this format for its diarization pipeline.
pub fn read_mono_16k(path: &Path) -> Result<Vec<f32>> {
    let mut reader = hound::WavReader::open(path)
        .with_context(|| format!("open {}", path.display()))?;
    let spec = reader.spec();
    if spec.sample_rate != 16_000 {
        return Err(anyhow!(
            "expected 16 kHz WAV, got {} Hz at {}",
            spec.sample_rate,
            path.display()
        ));
    }
    if spec.channels != 1 {
        return Err(anyhow!(
            "expected mono WAV, got {} channels at {}",
            spec.channels,
            path.display()
        ));
    }
    match (spec.sample_format, spec.bits_per_sample) {
        (hound::SampleFormat::Int, 16) => {
            let samples: Result<Vec<f32>, _> = reader
                .samples::<i16>()
                .map(|s| s.map(|v| v as f32 / i16::MAX as f32))
                .collect();
            Ok(samples?)
        }
        (hound::SampleFormat::Float, 32) => {
            let samples: Result<Vec<f32>, _> = reader.samples::<f32>().collect();
            Ok(samples?)
        }
        (fmt, bits) => Err(anyhow!(
            "unsupported WAV sample format: {:?} {}-bit",
            fmt,
            bits
        )),
    }
}
