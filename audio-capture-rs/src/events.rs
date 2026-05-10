//! Stdout JSON emission + stdin command IPC.
//!
//! Stdout: line-delimited JSON events the Rust backend's `recording.rs`
//! `SidecarEvent` enum knows how to deserialise. Same wire format as the
//! Swift sidecar — see audio-capture/Sources/audio-capture/main.swift.
//!
//! Stdin: line-delimited commands. The Rust backend writes `pause\n` /
//! `resume\n` / `stop\n` to our stdin pipe instead of sending POSIX signals
//! (Windows has no SIGUSR1/SIGUSR2 equivalent that can be raised by an
//! unrelated process without elevated privilege). On Unix builds of this
//! sidecar the same channel works — the backend keeps using signals there
//! for parity with the Swift sidecar, but we accept stdin commands too as
//! a fallback.

use parking_lot::Mutex;
use serde_json::Value;
use std::io::{self, BufRead, Write};
use std::sync::OnceLock;

/// Single global stdout lock so concurrent emit() calls from the mic and
/// system writer threads can't interleave bytes mid-line.
fn stdout_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

/// Emit a single JSON line on stdout and flush. Writes to a Vec first so a
/// serialisation panic can't leave a partial line in the pipe.
pub fn emit(value: Value) {
    let line = match serde_json::to_string(&value) {
        Ok(s) => s,
        Err(e) => {
            // Last-ditch: write a known-good error line instead of crashing.
            // Errors here would mean a non-serialisable Value, which would
            // be a programming bug — but the sidecar's job is to keep
            // streaming, so we degrade rather than abort.
            format!("{{\"event\":\"error\",\"message\":\"emit serialise: {e}\"}}")
        }
    };
    let _g = stdout_lock().lock();
    let mut out = io::stdout().lock();
    let _ = writeln!(out, "{line}");
    let _ = out.flush();
}

/// Convenience for the common `{"event":"error","message":…}` shape.
pub fn emit_error(msg: impl Into<String>) {
    emit(serde_json::json!({"event": "error", "message": msg.into()}));
}

/// Spawn a background thread that reads line-delimited commands from stdin
/// and dispatches each to `handler`. Returns immediately. The thread exits
/// cleanly when stdin closes (parent dropped its write end → EOF).
pub fn spawn_stdin_reader<F>(mut handler: F)
where
    F: FnMut(&str) + Send + 'static,
{
    std::thread::Builder::new()
        .name("stdin-ipc".into())
        .spawn(move || {
            let stdin = io::stdin();
            for line in stdin.lock().lines() {
                let Ok(line) = line else {
                    // EOF or read error → parent gone, sidecar should exit
                    // soon via its other shutdown paths (SIGTERM / Ctrl+C
                    // handler / parent-death detection on the actual write
                    // attempts that fail with EPIPE).
                    break;
                };
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                handler(trimmed);
            }
        })
        .expect("spawn stdin-ipc thread");
}
