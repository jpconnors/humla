//! audio-capture sidecar — Windows port of the macOS Swift sidecar.
//!
//! Wire-protocol contract this binary satisfies (must match the Swift
//! sidecar exactly so the Rust backend in src-tauri can treat both
//! identically):
//!
//! Stdout (line-delimited JSON):
//!   {"event":"chunk","source":"mic"|"sys","path":"…","start_ms":N}
//!   {"event":"full_recording","source":"mic"|"sys","path":"…","duration_ms":N}
//!   {"event":"heartbeat","mic_frames":…,"sys_frames":…,"chunks":…,
//!    "mic_peak":…,"sys_peak":…}
//!   {"event":"paused"} / {"event":"resumed"} / {"event":"stopped"}
//!   {"event":"error","message":…}
//!
//! Stdin (line-delimited commands):
//!   pause\n   resume\n   stop\n
//!
//! Args:
//!   audio-capture status               — emit {microphone,screen} JSON, exit
//!   audio-capture request-microphone   — re-probe + emit, exit
//!   audio-capture request-screen       — emit {screen:granted}, exit
//!   audio-capture --out <dir>          — recording mode (default)

mod audio;
mod events;
mod mic;
mod resample;
mod stats;

#[cfg(windows)]
mod system_win;

use crate::audio::{ensure_out_dir, ChunkWriter, FullRecordingWriter};
use crate::events::{emit, emit_error, spawn_stdin_reader};
use crate::stats::Stats;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() >= 2 {
        match args[1].as_str() {
            "status" => return run_status(),
            "request-microphone" => return run_request_microphone(),
            "request-screen" => return run_request_screen(),
            _ => {}
        }
    }
    if let Err(e) = run_record(&args) {
        emit_error(format!("fatal: {e}"));
        std::process::exit(1);
    }
}

// -- Permission probes ------------------------------------------------------
//
// Windows has no TCC-equivalent prompt API. Mic privacy is a global Settings
// toggle; if it's off, opening the device fails with E_ACCESSDENIED. There
// is no programmatic "request access" call — only a deep link to
// ms-settings:privacy-microphone (handled on the Tauri side via
// `permissions_open_settings`).
//
// `status` therefore probes by trying to query the default input device's
// default config. The probe never streams audio, so it's cheap and doesn't
// leave the device busy.

fn run_status() {
    let microphone = if can_open_default_mic() { "granted" } else { "denied" };
    // Screen recording on Windows has no permission gate — anyone can
    // capture the desktop without prompting. Always granted so the
    // Settings page in Humla doesn't badger the user about a non-existent
    // toggle.
    emit(serde_json::json!({
        "microphone": microphone,
        "screen": "granted",
    }));
}

fn run_request_microphone() {
    // No request API on Windows — re-probe and emit. The frontend tells the
    // user to flip the toggle in Settings if this still says denied.
    let microphone = if can_open_default_mic() { "granted" } else { "denied" };
    emit(serde_json::json!({"microphone": microphone}));
    if microphone != "granted" {
        std::process::exit(1);
    }
}

fn run_request_screen() {
    emit(serde_json::json!({"screen": "granted"}));
}

fn can_open_default_mic() -> bool {
    use cpal::traits::{DeviceTrait, HostTrait};
    let host = cpal::default_host();
    match host.default_input_device() {
        Some(device) => device.default_input_config().is_ok(),
        None => false,
    }
}

// -- Recording mode ---------------------------------------------------------

fn run_record(args: &[String]) -> anyhow::Result<()> {
    let out_dir = parse_out_dir(args).unwrap_or_else(std::env::temp_dir);
    ensure_out_dir(&out_dir)?;

    let stats = Arc::new(Stats::default());

    // Per-source writer pairs. VAD constants (1.0 / 15.0 / 500 ms) match
    // the Swift sidecar exactly so transcription quality is identical.
    let mic_chunks = Arc::new(ChunkWriter::new(
        "mic",
        out_dir.clone(),
        1.0,
        15.0,
        500.0,
        stats.clone(),
    ));
    let mic_full = Arc::new(FullRecordingWriter::new("mic", out_dir.clone()));
    let sys_chunks = Arc::new(ChunkWriter::new(
        "sys",
        out_dir.clone(),
        1.0,
        15.0,
        500.0,
        stats.clone(),
    ));
    let sys_full = Arc::new(FullRecordingWriter::new("sys", out_dir));

    // Start mic. Failure here is non-fatal — system audio may still be
    // available, which is enough for some recording scenarios. We surface
    // the cause as an error event so the frontend can show a useful toast.
    //
    // The cpal `Stream` inside `MicCapture` is `!Send` on every platform
    // (the audio backend pins to a single thread). We MUST keep it on the
    // main thread for the lifetime of the recording. Pause control is
    // routed via `mic_paused`, the `Send + Sync` atomic the stream's
    // callback checks each tick.
    let (mic_stream_holder, mic_paused) = match mic::start(
        mic_chunks.clone(),
        mic_full.clone(),
        stats.clone(),
    ) {
        Ok(h) => (Some(h.stream), Some(h.paused)),
        Err(e) => {
            emit_error(format!(
                "mic capture failed: {e}. Open Settings → Privacy → Microphone and grant Humla access."
            ));
            (None, None)
        }
    };

    // Start system loopback (Windows only). Non-fatal if it fails — in-person
    // meetings need only mic anyway. The handle is `Send` (it owns its own
    // thread) so we can stash it anywhere; we keep it on main and drop on
    // shutdown.
    #[cfg(windows)]
    let sys_handle = match system_win::start(sys_chunks.clone(), sys_full.clone(), stats.clone()) {
        Ok(h) => Some(h),
        Err(e) => {
            emit_error(format!("system loopback unavailable: {e}"));
            None
        }
    };

    let shutting_down = Arc::new(AtomicBool::new(false));

    // Heartbeat — every 2 s, emit aggregated counters since last beat.
    let stats_for_hb = stats.clone();
    let stop_hb = shutting_down.clone();
    let hb_handle = std::thread::Builder::new()
        .name("heartbeat".into())
        .spawn(move || {
            while !stop_hb.load(Ordering::Acquire) {
                std::thread::sleep(Duration::from_secs(2));
                if stop_hb.load(Ordering::Acquire) {
                    break;
                }
                let snap = stats_for_hb.snapshot();
                emit(serde_json::json!({
                    "event": "heartbeat",
                    "mic_frames": snap.mic_frames,
                    "sys_frames": snap.sys_frames,
                    "chunks": snap.chunks,
                    "mic_peak": snap.mic_peak,
                    "sys_peak": snap.sys_peak,
                }));
            }
        })?;

    // Stdin command IPC. The Rust backend (Windows path) writes
    // "pause"/"resume"/"stop" newline-terminated to our stdin. The reader
    // updates the shared atomics directly; the cpal mic callback and the
    // WASAPI thread already check those atomics each tick, so no bridge
    // thread is needed. The main loop polls `shutting_down` to exit.
    {
        // Clone the pause atomic for each capture source so the reader
        // can flip them in lockstep. Sources where capture failed get a
        // dummy atomic — the toggle is harmless (no callback is reading).
        let mic_p = mic_paused
            .clone()
            .unwrap_or_else(|| Arc::new(AtomicBool::new(false)));
        #[cfg(windows)]
        let sys_pause: Option<Arc<dyn Fn(bool) + Send + Sync>> = {
            // SystemCapture's pause/resume are method calls; wrap them so
            // the closure below stays platform-uniform.
            // We can't move sys_handle here (we need to keep it owned by
            // main for shutdown), so route via a shared atomic the
            // wasapi thread reads — that's already how SystemCapture works
            // internally. Expose its flag via a clone.
            sys_handle.as_ref().map(|h| {
                let f = h.pause_flag();
                let cb: Arc<dyn Fn(bool) + Send + Sync> = Arc::new(move |p: bool| {
                    f.store(p, Ordering::Release);
                });
                cb
            })
        };
        #[cfg(not(windows))]
        let sys_pause: Option<Arc<dyn Fn(bool) + Send + Sync>> = None;

        let shutting_down_t = shutting_down.clone();
        spawn_stdin_reader(move |cmd| match cmd {
            "pause" => {
                mic_p.store(true, Ordering::Release);
                if let Some(f) = sys_pause.as_ref() { f(true); }
                emit(serde_json::json!({"event": "paused"}));
            }
            "resume" => {
                mic_p.store(false, Ordering::Release);
                if let Some(f) = sys_pause.as_ref() { f(false); }
                emit(serde_json::json!({"event": "resumed"}));
            }
            "stop" => {
                shutting_down_t.store(true, Ordering::Release);
            }
            other => emit_error(format!("unknown stdin command: {other}")),
        });
    }

    // Ctrl+C / SIGTERM → graceful shutdown. Mirrors the Swift sidecar's
    // SIGTERM handler. ctrlc::set_handler covers Windows (CTRL+BREAK /
    // CTRL+C) and Unix (SIGTERM/SIGINT).
    {
        let shutting_down_t = shutting_down.clone();
        let _ = ctrlc::set_handler(move || {
            shutting_down_t.store(true, Ordering::Release);
        });
    }

    // Block until shutdown is signalled (stdin "stop", Ctrl+C, or SIGTERM).
    // The cpal Stream stays alive on this (main) thread the whole time —
    // dropping it here at function return is what stops the mic backend.
    while !shutting_down.load(Ordering::Acquire) {
        std::thread::sleep(Duration::from_millis(100));
    }

    // Heartbeat off first so a beat doesn't sneak between writer closes
    // and our final `stopped` event.
    let _ = hb_handle.join();

    // Drop capture handles BEFORE closing the writers so any callback
    // already in flight finishes before flush_current() runs.
    drop(mic_stream_holder);
    #[cfg(windows)]
    drop(sys_handle);

    // Close writers — same order as the Swift sidecar so the parent gets
    // chunk events before full_recording events for each source.
    mic_chunks.close();
    sys_chunks.close();
    mic_full.close();
    sys_full.close();

    emit(serde_json::json!({"event": "stopped"}));
    Ok(())
}

fn parse_out_dir(args: &[String]) -> Option<PathBuf> {
    let mut iter = args.iter();
    while let Some(a) = iter.next() {
        if a == "--out" {
            if let Some(v) = iter.next() {
                return Some(PathBuf::from(v));
            }
        }
    }
    None
}
