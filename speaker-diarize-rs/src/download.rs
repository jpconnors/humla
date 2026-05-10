//! Model download for the speaker-diarize sidecar.
//!
//! Mirrors the Swift sidecar's three-phase progress events
//! (`listing` / `downloading` / `compiling`) — we only really do
//! `downloading` here (sherpa-onnx ONNX models don't need a separate
//! compile step like CoreML), but emitting the same phase strings keeps
//! the frontend code path identical across platforms.
//!
//! Models come from the official sherpa-onnx HuggingFace mirror:
//! https://huggingface.co/csukuangfj/sherpa-onnx-pyannote-segmentation-3-0
//! https://huggingface.co/csukuangfj/sherpa-onnx-3dspeaker
//! Sortformer model URL is a placeholder until upstream publishes one;
//! the current implementation rejects the sortformer engine until then.

use crate::Engine;
use anyhow::{anyhow, Context, Result};
use serde_json::json;
use std::path::{Path, PathBuf};

struct ModelFile {
    url: &'static str,
    rel_path: &'static str,
}

fn community1_files() -> &'static [ModelFile] {
    &[
        ModelFile {
            // pyannote segmentation-3.0 (ONNX export)
            url: "https://github.com/k2-fsa/sherpa-onnx/releases/download/speaker-segmentation-models/sherpa-onnx-pyannote-segmentation-3-0.tar.bz2",
            rel_path: "segmentation.onnx",
        },
        ModelFile {
            // 3D-Speaker xvector embedding
            url: "https://github.com/k2-fsa/sherpa-onnx/releases/download/speaker-recongition-models/3dspeaker_speech_eres2net_base_sv_zh-cn_3dspeaker_16k.onnx",
            rel_path: "embedding.onnx",
        },
    ]
}

fn sortformer_files() -> &'static [ModelFile] {
    // Placeholder URL — pin to the real sherpa-onnx sortformer release
    // once it's published. The Swift sidecar uses the FluidAudio CoreML
    // build which has no ONNX equivalent yet.
    &[ModelFile {
        url: "https://github.com/k2-fsa/sherpa-onnx/releases/download/sortformer-models/PLACEHOLDER",
        rel_path: "sortformer.onnx",
    }]
}

pub fn expected_paths(engine: Engine, dir: &Path) -> Vec<PathBuf> {
    let files = match engine {
        Engine::Community1 => community1_files(),
        Engine::Sortformer => sortformer_files(),
    };
    files.iter().map(|f| dir.join(f.rel_path)).collect()
}

pub fn run(engine: Engine, dir: &Path) -> Result<()> {
    if matches!(engine, Engine::Sortformer) {
        // Be honest rather than silently 404 the user. They can switch to
        // community1 in Settings — same data path, slightly different
        // accuracy/perf trade-off.
        emit_progress(0.0, "listing", engine);
        return Err(anyhow!(
            "sortformer ONNX models are not yet published for Windows. Use community1 for now."
        ));
    }

    let files = community1_files();
    emit_progress(0.0, "listing", engine);

    let total: f64 = files.len() as f64;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("build tokio runtime")?;

    runtime.block_on(async {
        let client = reqwest::Client::builder()
            .build()
            .context("build http client")?;
        for (i, f) in files.iter().enumerate() {
            let dest = dir.join(f.rel_path);
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent)?;
            }
            download_one(&client, f.url, &dest, i, files.len(), engine).await?;
            let frac = ((i + 1) as f64) / total;
            emit_progress(frac, "downloading", engine);
        }
        anyhow::Ok(())
    })?;

    // Marker file the status command checks. Written last so a
    // partially-downloaded directory doesn't masquerade as ready.
    std::fs::write(dir.join(".downloaded"), b"v1")?;
    println!("{}", json!({"event": "done"}));
    Ok(())
}

async fn download_one(
    client: &reqwest::Client,
    url: &str,
    dest: &Path,
    index: usize,
    total: usize,
    engine: Engine,
) -> Result<()> {
    use futures::StreamExt;
    use tokio::io::AsyncWriteExt;

    let resp = client
        .get(url)
        .send()
        .await
        .with_context(|| format!("GET {url}"))?
        .error_for_status()
        .with_context(|| format!("HTTP error from {url}"))?;
    let total_bytes = resp.content_length().unwrap_or(0);
    let tmp = dest.with_extension("part");
    let mut file = tokio::fs::File::create(&tmp)
        .await
        .with_context(|| format!("create {}", tmp.display()))?;
    let mut downloaded: u64 = 0;
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.with_context(|| format!("read body of {url}"))?;
        file.write_all(&chunk).await?;
        downloaded += chunk.len() as u64;
        if total_bytes > 0 {
            let local = downloaded as f64 / total_bytes as f64;
            let global = (index as f64 + local) / total as f64;
            emit_progress(global, "downloading", engine);
        }
    }
    file.flush().await?;
    drop(file);
    tokio::fs::rename(&tmp, dest).await?;
    Ok(())
}

fn emit_progress(fraction: f64, phase: &str, engine: Engine) {
    let payload = json!({
        "event": "progress",
        "fraction": fraction,
        "phase": phase,
        "engine": engine.folder_name(),
    });
    println!("{payload}");
}
