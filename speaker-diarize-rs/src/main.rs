//! speaker-diarize sidecar — Windows port of the macOS Swift sidecar.
//!
//! Wire-protocol contract (must match the Swift sidecar exactly so the
//! Rust backend `src-tauri/src/diarize.rs` can treat both identically):
//!
//! Subcommands:
//!   speaker-diarize <wav-path> [--num-speakers N] [--engine E] [--threshold T]
//!     → stdout: JSON array of {start_ms, end_ms, speaker_id}, exit 0
//!     → stderr on failure: any line beginning with `humla-error: ` is the
//!       user-facing error message; the backend strips this prefix and
//!       toasts the rest.
//!
//!   speaker-diarize status [--engine E]
//!     → stdout: {"downloaded": bool, "sizeBytes": N, "path": "…"}
//!
//!   speaker-diarize download [--engine E]
//!     → stdout: stream of {"event":"progress", "fraction": F, "phase": "…"}
//!       lines, ending with {"event":"done"}, exit 0.
//!
//!   speaker-diarize delete [--engine E]
//!     → stdout: empty, exit 0.

use anyhow::{anyhow, Context, Result};
use clap::Parser;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

mod download;
mod engine;
mod io_wav;

#[derive(Parser, Debug)]
#[command(version, about = "Humla speaker-diarize Windows sidecar")]
struct Cli {
    /// First positional: WAV path for diarize, or one of "status",
    /// "download", "delete".
    target: String,
    #[arg(long, default_value = "community1")]
    engine: String,
    #[arg(long)]
    num_speakers: Option<usize>,
    #[arg(long)]
    threshold: Option<f32>,
    #[arg(long)]
    silence_threshold: Option<f32>,
    #[arg(long)]
    pred_threshold: Option<f32>,
}

#[derive(Debug, Clone, Copy)]
enum Engine {
    Community1,
    Sortformer,
}

impl Engine {
    fn parse(s: &str) -> Engine {
        match s {
            "sortformer" => Engine::Sortformer,
            _ => Engine::Community1,
        }
    }
    fn folder_name(self) -> &'static str {
        match self {
            Engine::Community1 => "community1",
            Engine::Sortformer => "sortformer",
        }
    }
}

#[derive(Serialize, Deserialize, Debug)]
struct Segment {
    start_ms: u64,
    end_ms: u64,
    speaker_id: String,
}

#[derive(Serialize, Debug)]
struct ModelStatus {
    downloaded: bool,
    #[serde(rename = "sizeBytes")]
    size_bytes: u64,
    path: String,
}

fn main() {
    let cli = Cli::parse();
    let engine = Engine::parse(&cli.engine);
    let result = match cli.target.as_str() {
        "status" => run_status(engine),
        "download" => run_download(engine),
        "delete" => run_delete(engine),
        wav_path => run_diarize(
            Path::new(wav_path),
            engine,
            cli.num_speakers,
            cli.threshold,
            cli.silence_threshold,
            cli.pred_threshold,
        ),
    };
    if let Err(e) = result {
        // The backend's diarize.rs strips the `humla-error: ` prefix and
        // shows the rest as a recording_error toast. Any other stderr
        // output is treated as diagnostic noise.
        eprintln!("humla-error: {e:#}");
        std::process::exit(1);
    }
}

fn run_status(engine: Engine) -> Result<()> {
    let dir = engine_dir(engine)?;
    let downloaded = engine::is_downloaded(&dir, engine);
    let size_bytes = if downloaded { engine::dir_size(&dir).unwrap_or(0) } else { 0 };
    let status = ModelStatus {
        downloaded,
        size_bytes,
        path: dir.to_string_lossy().to_string(),
    };
    println!("{}", serde_json::to_string(&status)?);
    Ok(())
}

fn run_download(engine: Engine) -> Result<()> {
    let dir = engine_dir(engine)?;
    std::fs::create_dir_all(&dir)?;
    download::run(engine, &dir)
}

fn run_delete(engine: Engine) -> Result<()> {
    let dir = engine_dir(engine)?;
    if dir.exists() {
        std::fs::remove_dir_all(&dir)
            .with_context(|| format!("remove {}", dir.display()))?;
    }
    Ok(())
}

fn run_diarize(
    wav_path: &Path,
    engine: Engine,
    num_speakers: Option<usize>,
    threshold: Option<f32>,
    silence_threshold: Option<f32>,
    pred_threshold: Option<f32>,
) -> Result<()> {
    if !wav_path.exists() {
        return Err(anyhow!("audio file not found: {}", wav_path.display()));
    }
    let dir = engine_dir(engine)?;
    if !engine::is_downloaded(&dir, engine) {
        return Err(anyhow!(
            "diarization models not downloaded. Open Settings → Diarization to download them first."
        ));
    }
    let samples = io_wav::read_mono_16k(wav_path)
        .with_context(|| format!("read wav {}", wav_path.display()))?;
    let segments = engine::diarize(
        engine,
        &dir,
        &samples,
        num_speakers,
        threshold,
        silence_threshold,
        pred_threshold,
    )?;
    println!("{}", serde_json::to_string(&segments)?);
    Ok(())
}

/// Per-engine model cache directory. Lives under the Humla appdata root so
/// the user can wipe everything in one shot from the OS file manager.
///
/// Windows: %APPDATA%\no.humla.app\models\diarize\<engine>
/// Linux:   ~/.local/share/no.humla.app/models/diarize/<engine>
/// macOS:   ~/Library/Application Support/no.humla.app/models/diarize/<engine>
///         (note: macOS uses the Swift sidecar; this binary only runs on
///          the macOS dev machine for parity testing)
fn engine_dir(engine: Engine) -> Result<PathBuf> {
    let base = dirs::data_dir()
        .ok_or_else(|| anyhow!("no app data dir"))?
        .join("no.humla.app")
        .join("models")
        .join("diarize")
        .join(engine.folder_name());
    Ok(base)
}
