//! Diarization engine wrappers.
//!
//! Inference runs through `sherpa-onnx` (offline pyannote-segmentation-3.0
//! + 3D-Speaker xvector + fast clustering). `download.rs` lays the two
//! ONNX files into `dir`; we point sherpa-onnx at them and run.
//!
//! Sortformer is a placeholder — `download::run` rejects it because no
//! ONNX export is published upstream yet. We bail loudly here too in
//! case a stale `.downloaded` marker sneaks past that check.

use crate::{download, Engine, Segment};
use anyhow::{anyhow, Context, Result};
use sherpa_onnx::{
    FastClusteringConfig, OfflineSpeakerDiarization, OfflineSpeakerDiarizationConfig,
    OfflineSpeakerSegmentationModelConfig, OfflineSpeakerSegmentationPyannoteModelConfig,
    SpeakerEmbeddingExtractorConfig,
};
use std::path::{Path, PathBuf};

/// Was the per-engine model set successfully downloaded? Cheap presence
/// check — verifies the marker file the download writes on completion. We
/// use a marker rather than checking individual model files so a partially-
/// downloaded engine reports as not-downloaded and the user re-runs.
pub fn is_downloaded(dir: &Path, _engine: Engine) -> bool {
    dir.join(".downloaded").exists()
}

pub fn dir_size(dir: &Path) -> Result<u64> {
    let mut total: u64 = 0;
    for entry in walkdir(dir)? {
        if entry.is_file() {
            total += std::fs::metadata(&entry)?.len();
        }
    }
    Ok(total)
}

fn walkdir(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(p) = stack.pop() {
        if !p.exists() {
            continue;
        }
        if p.is_dir() {
            for entry in std::fs::read_dir(&p)
                .with_context(|| format!("read_dir {}", p.display()))?
            {
                let entry = entry?;
                stack.push(entry.path());
            }
        } else {
            out.push(p);
        }
    }
    Ok(out)
}

/// Run diarization. Returns `Vec<Segment>` in `start_ms` order, with
/// stable `speaker_id` strings ("speaker_0", "speaker_1", …) — the Rust
/// backend's `assign_speaker` only cares about identity-equality, not
/// the specific labels.
pub fn diarize(
    engine: Engine,
    dir: &Path,
    samples: &[f32],
    num_speakers: Option<usize>,
    threshold: Option<f32>,
    silence_threshold: Option<f32>,
    pred_threshold: Option<f32>,
) -> Result<Vec<Segment>> {
    if matches!(engine, Engine::Sortformer) {
        return Err(anyhow!(
            "sortformer is not yet supported on Windows. Switch to community1 in Settings → Diarization."
        ));
    }

    // Sortformer-only knobs; kept in the signature so the dispatch layer
    // can stay engine-agnostic.
    let _ = (silence_threshold, pred_threshold);

    let paths = download::expected_paths(engine, dir);
    for p in paths.iter() {
        if !p.exists() {
            return Err(anyhow!(
                "missing diarization model file: {}. Re-run download from Settings → Diarization.",
                p.display()
            ));
        }
    }

    let seg_path = dir.join("segmentation.onnx");
    let emb_path = dir.join("embedding.onnx");

    let config = OfflineSpeakerDiarizationConfig {
        segmentation: OfflineSpeakerSegmentationModelConfig {
            pyannote: OfflineSpeakerSegmentationPyannoteModelConfig {
                model: Some(seg_path.to_string_lossy().into_owned()),
            },
            ..Default::default()
        },
        embedding: SpeakerEmbeddingExtractorConfig {
            model: Some(emb_path.to_string_lossy().into_owned()),
            ..Default::default()
        },
        clustering: FastClusteringConfig {
            // num_clusters == 0 → estimate from threshold; > 0 → fixed.
            // Matches FluidAudio community-1's behaviour when the user
            // has set `expected_speakers`.
            num_clusters: num_speakers.map(|n| n as i32).unwrap_or(0),
            // 0.5 mirrors the macOS community1 setting; only consulted
            // when num_clusters == 0.
            threshold: threshold.unwrap_or(0.5),
        },
        ..Default::default()
    };

    let sd = OfflineSpeakerDiarization::create(&config).ok_or_else(|| {
        anyhow!(
            "sherpa-onnx failed to load diarization models from {}",
            dir.display()
        )
    })?;

    // io_wav::read_mono_16k already enforces 16 kHz; this trips loudly
    // if a future model bundle disagrees instead of silently producing
    // junk segments.
    if sd.sample_rate() != 16_000 {
        return Err(anyhow!(
            "diarizer expects {} Hz; supplied samples are 16 kHz",
            sd.sample_rate()
        ));
    }

    let result = sd
        .process(samples)
        .ok_or_else(|| anyhow!("sherpa-onnx process() returned None — empty or malformed audio"))?;

    let segments = result
        .sort_by_start_time()
        .into_iter()
        .map(|s| Segment {
            start_ms: (s.start.max(0.0) * 1000.0).round() as u64,
            end_ms: (s.end.max(0.0) * 1000.0).round() as u64,
            speaker_id: format!("speaker_{}", s.speaker),
        })
        .collect();

    Ok(segments)
}
