//! Cheap mono-mixdown + linear resample to the target rate (16 kHz mono).
//!
//! Whisper expects 16 kHz mono Float32. Mic capture (cpal) is typically
//! 44.1/48 kHz, often stereo. WASAPI loopback delivers the mix format —
//! almost always 48 kHz stereo Float32 on modern Windows. We don't need
//! studio-quality resampling for speech-to-text, so this stays in the
//! crate to avoid pulling in `rubato` or similar (build size + compile
//! time matter for a sidecar that should start within a few hundred ms).
//!
//! Mix algorithm:
//!   1. Channel sum → mono Float32 average.
//!   2. Linear interpolation between adjacent input samples to land on
//!      the target sample-rate grid.
//!
//! Linear is audibly poor for music but fine for the speech band Whisper
//! cares about. The chunks themselves get gain-checked (silence drop) on
//! the writer side so resampling artefacts in silence stretches don't
//! survive into transcription.

pub const TARGET_RATE: u32 = 16_000;

/// Stateful linear-interpolation resampler. Holds the last sample from the
/// previous buffer so we can interpolate cleanly across buffer boundaries
/// — without that, every callback would seam-artefact at its first sample.
pub struct Resampler {
    in_rate: u32,
    out_rate: u32,
    /// Fractional position into the current input buffer at the start of
    /// the next call. Carries between buffers so output spacing is exact
    /// regardless of how the input is chunked.
    phase: f64,
    /// Last input sample from the previous buffer (mono). Used as the
    /// interpolation anchor for the first output samples of the next
    /// buffer.
    last: f32,
    /// True until the first input sample arrives; suppresses interpolation
    /// against an undefined `last`.
    primed: bool,
}

impl Resampler {
    pub fn new(in_rate: u32, out_rate: u32) -> Self {
        Self {
            in_rate,
            out_rate,
            phase: 0.0,
            last: 0.0,
            primed: false,
        }
    }

    /// Mix `frames` (interleaved, `channels`-wide, Float32) down to mono
    /// and resample to `out_rate`. Returns mono Float32 at `out_rate`.
    pub fn process_f32(&mut self, frames: &[f32], channels: usize) -> Vec<f32> {
        if frames.is_empty() || channels == 0 {
            return Vec::new();
        }
        let in_n = frames.len() / channels;
        let mut mono = Vec::with_capacity(in_n);
        let scale = 1.0 / channels as f32;
        for i in 0..in_n {
            let mut sum = 0.0f32;
            for c in 0..channels {
                sum += frames[i * channels + c];
            }
            mono.push(sum * scale);
        }

        let ratio = self.in_rate as f64 / self.out_rate as f64;
        // Output count: how many out-rate ticks fit before we'd need an
        // input sample we don't have yet. We always need the input to the
        // RIGHT of `phase` for interpolation; cap the output to leave one
        // input sample worth of headroom past the last produced tick so
        // the next call starts cleanly.
        let mut out = Vec::with_capacity(((in_n as f64 / ratio) as usize) + 4);

        let mut phase = self.phase;
        loop {
            let i_floor = phase.floor() as isize;
            let frac = (phase - phase.floor()) as f32;
            // Need samples at i_floor and i_floor+1. i_floor==-1 means the
            // sample to the left lives in the previous buffer (use `last`).
            let next_idx = i_floor + 1;
            if next_idx >= in_n as isize {
                break;
            }
            let left = if i_floor < 0 {
                if !self.primed {
                    // Edge case: very first call, before any audio. Treat
                    // as zero — it'll be silent anyway.
                    0.0
                } else {
                    self.last
                }
            } else {
                mono[i_floor as usize]
            };
            let right = mono[next_idx as usize];
            out.push(left + (right - left) * frac);
            phase += ratio;
        }

        // Carry: subtract the consumed integer portion of phase so it stays
        // in [0, 1) relative to the *next* buffer's index 0. Any sample we
        // didn't consume from this buffer effectively gets re-anchored as
        // a negative index of the next buffer (handled via `last`).
        if !mono.is_empty() {
            self.last = *mono.last().unwrap();
            self.primed = true;
        }
        self.phase = phase - in_n as f64;

        out
    }

    /// Same idea for Int16 PCM input, used by some cpal stream configs.
    #[allow(dead_code)]
    pub fn process_i16(&mut self, frames: &[i16], channels: usize) -> Vec<f32> {
        let f: Vec<f32> = frames.iter().map(|s| *s as f32 / i16::MAX as f32).collect();
        self.process_f32(&f, channels)
    }
}
