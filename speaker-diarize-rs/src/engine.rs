//! Diarization engine wrappers.
//!
//! **Status: scaffold.** The `diarize()` function returns a clear
//! "not yet wired" error rather than fake segments. The download +
//! status + delete subcommands are fully functional, so the user can
//! download the models from Settings, see them on disk, and delete
//! them — all that's missing is the inference call.
//!
//! To finish the port, swap the `not_implemented_error` block at the
//! bottom of `diarize()` for a real ONNX Runtime call. The simplest
//! path is to add `sherpa-rs = "0.6"` (which bundles ONNX Runtime and
//! a working pyannote pipeline) and call its
//! `SpeakerDiarization::process(samples)` — returning the produced
//! segments in the same `Vec<Segment>` shape this stub already builds
//! up.
//!
//! See `download.rs::expected_paths` for which model files are on disk
//! at the time `diarize()` is called.

use crate::{download, Engine, Segment};
use anyhow::{anyhow, Context, Result};
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
///
/// **Stub today.** Returns `Err(...)` until the ONNX Runtime call is
/// wired in. See module docs.
pub fn diarize(
    engine: Engine,
    dir: &Path,
    samples: &[f32],
    num_speakers: Option<usize>,
    threshold: Option<f32>,
    silence_threshold: Option<f32>,
    pred_threshold: Option<f32>,
) -> Result<Vec<Segment>> {
    // Verify model files are on disk before claiming we can't run them.
    // This converts "model missing" into a re-download hint instead of
    // the generic "not implemented" message.
    let paths = download::expected_paths(engine, dir);
    for p in paths.iter() {
        if !p.exists() {
            return Err(anyhow!(
                "missing diarization model file: {}. Re-run download from Settings → Diarization.",
                p.display()
            ));
        }
    }

    // Threshold knobs aren't used yet — preserve the names so call-sites
    // don't need to change once inference is wired up.
    let _ = (samples, num_speakers, threshold, silence_threshold, pred_threshold);

    Err(anyhow!(
        "Windows diarization is not yet wired up to ONNX Runtime. \
         Open speaker-diarize-rs/src/engine.rs and replace this stub with a \
         real sherpa-rs (or ort) inference call — see module docs for the steps. \
         All other subcommands (status / download / delete) are fully functional."
    ))
}
