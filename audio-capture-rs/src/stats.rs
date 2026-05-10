//! Aggregated diagnostic counters drained periodically into `heartbeat`
//! events. Mirrors the Swift sidecar's `Stats` class.

use parking_lot::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Default)]
pub struct Stats {
    mic_frames: AtomicU64,
    sys_frames: AtomicU64,
    chunks: AtomicU64,
    /// Peaks are protected by a mutex (not atomic floats) because we need
    /// to read-and-reset them as a pair in the heartbeat tick. Atomic f32
    /// isn't in std and bringing in a crate for it would be overkill.
    peaks: Mutex<Peaks>,
}

#[derive(Default)]
struct Peaks {
    mic: f32,
    sys: f32,
}

impl Stats {
    pub fn add_mic(&self, samples: &[f32]) {
        self.mic_frames
            .fetch_add(samples.len() as u64, Ordering::Relaxed);
        let peak = samples.iter().fold(0.0f32, |acc, s| acc.max(s.abs()));
        let mut g = self.peaks.lock();
        if peak > g.mic {
            g.mic = peak;
        }
    }

    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn add_sys(&self, samples: &[f32]) {
        self.sys_frames
            .fetch_add(samples.len() as u64, Ordering::Relaxed);
        let peak = samples.iter().fold(0.0f32, |acc, s| acc.max(s.abs()));
        let mut g = self.peaks.lock();
        if peak > g.sys {
            g.sys = peak;
        }
    }

    pub fn inc_chunks(&self) {
        self.chunks.fetch_add(1, Ordering::Relaxed);
    }

    /// Snapshot frame counts (cumulative) and peaks (resets to zero on
    /// each call so each heartbeat reports the peak observed since the
    /// last beat — matches the Swift sidecar's behaviour).
    pub fn snapshot(&self) -> StatsSnapshot {
        let mut g = self.peaks.lock();
        let mic_peak = g.mic;
        let sys_peak = g.sys;
        g.mic = 0.0;
        g.sys = 0.0;
        StatsSnapshot {
            mic_frames: self.mic_frames.load(Ordering::Relaxed),
            sys_frames: self.sys_frames.load(Ordering::Relaxed),
            chunks: self.chunks.load(Ordering::Relaxed),
            mic_peak,
            sys_peak,
        }
    }
}

pub struct StatsSnapshot {
    pub mic_frames: u64,
    pub sys_frames: u64,
    pub chunks: u64,
    pub mic_peak: f32,
    pub sys_peak: f32,
}
