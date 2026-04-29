# VAD Chunking + Trailing-Context Whisper Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Three coordinated changes that make local-Whisper transcription feel closer to live captioning: (1) remove Speechmatics end-to-end, (2) replace fixed 20s chunks in the Swift sidecar with VAD-detected natural-boundary chunks, (3) carry the last ~150 words of committed transcript + custom vocabulary into each Whisper call as `initial_prompt` so context is preserved across chunks.

**Architecture:** Sidecar emits chunks at silence boundaries (with min/max length caps) instead of a fixed 20s timer, so each chunk arrives at a natural pause. A new per-recording-session `TranscriptTrail` ring buffer (Rust) holds committed transcript words; on every new chunk the buffer is rendered into a prompt prefix combined with the user's custom vocabulary and passed to whisper-rs / OpenAI as `initial_prompt`. This anchors decoding to the actual conversation context, dampens hallucinations on near-silent chunks, and stabilises proper-noun spelling across the transcript. Speechmatics is fully removed — no provider, no UI, no IPC, no settings.

**Tech Stack:** Swift 5 + AVAudioEngine + ScreenCaptureKit (sidecar VAD), Rust + tokio + parking_lot + whisper-rs (transcript trail + prompt builder), TypeScript + React 19 (settings UI cleanup).

---

## File Structure

**Modified:**
- `src-tauri/src/lib.rs` — drop `mod speechmatics`, drop 3 command registrations
- `src-tauri/src/commands.rs` — drop Speechmatics constants/commands/branches; add `build_initial_prompt`; clear/extend trail on commit
- `src-tauri/src/recording.rs` — add `TranscriptTrail` struct + field on `RecordingSession`
- `src-tauri/src/local_whisper.rs` — no signature change (already takes `initial_prompt`)
- `src-tauri/src/openai.rs` — no change (already takes `prompt`)
- `src/lib/ipc.ts` — drop Speechmatics types and methods
- `src/pages/Settings.tsx` — drop Speechmatics provider option, key field, region/op selectors
- `audio-capture/Sources/audio-capture/main.swift` — VAD-driven `ChunkWriter` rotation
- `README.md` and `CLAUDE.md` — drop Speechmatics references (light touch)

**Deleted:**
- `src-tauri/src/speechmatics.rs`

---

## Task 1: Remove Speechmatics from the backend

**Files:**
- Delete: `src-tauri/src/speechmatics.rs`
- Modify: `src-tauri/src/lib.rs:3` and `src-tauri/src/lib.rs:67-69`
- Modify: `src-tauri/src/commands.rs` — multiple regions

- [ ] **Step 1: Delete the speechmatics module file**

```bash
rm src-tauri/src/speechmatics.rs
```

- [ ] **Step 2: Drop the module + command registrations in `lib.rs`**

In `src-tauri/src/lib.rs:3`, remove the line:

```rust
mod speechmatics;
```

In `src-tauri/src/lib.rs:67-69`, remove the three `commands::speechmatics_*` entries from the `tauri::generate_handler!` list. The remaining list keeps `commands::api_key_*` and `commands::local_whisper_*` intact.

- [ ] **Step 3: Drop Speechmatics imports and constants in `commands.rs`**

In `src-tauri/src/commands.rs:3` remove `use crate::speechmatics;`.

In `src-tauri/src/commands.rs:20-21` and `:25` remove these three constants:

```rust
const DEFAULT_SPEECHMATICS_OP: &str = "enhanced";
const DEFAULT_SPEECHMATICS_REGION: &str = "eu1";
// ...
const SPEECHMATICS_API_KEY: &str = "__speechmatics_api_key__";
```

- [ ] **Step 4: Delete the three Speechmatics tauri commands**

In `src-tauri/src/commands.rs:161-197` remove the entire block of three functions `speechmatics_api_key_get`, `speechmatics_api_key_set`, and `speechmatics_api_key_test`.

- [ ] **Step 5: Drop the Speechmatics branch in the recording prerequisite check**

In `src-tauri/src/commands.rs:429-442`, the `let pre_err = match provider.as_str()` becomes:

```rust
let pre_err = match provider.as_str() {
    "local" => {
        let p = local_model_path(&app).map_err(|e| e.to_string())?;
        (!p.exists()).then_some(
            "Local Whisper model not downloaded. Download it in Settings → Transcription.",
        )
    }
    _ => read_secret(&state, API_KEY)?
        .is_none()
        .then_some("OpenAI API key not set. Add one in Settings → API keys."),
};
```

- [ ] **Step 6: Drop Speechmatics from `transcribe_chunk` config + dispatch**

In `src-tauri/src/commands.rs:684-694`, the api_key match becomes:

```rust
let api_key = match provider.as_str() {
    "local" => String::new(),
    _ => db::get_setting(&conn, API_KEY)?
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow::anyhow!("no OpenAI API key"))?,
};
```

In `src-tauri/src/commands.rs:697-700` remove the two `speechmatics_op` and `speechmatics_region` `db::get_setting` lines.

In `src-tauri/src/commands.rs:703-711` (the `TranscribeCfg { ... }` literal) remove the `speechmatics_op` and `speechmatics_region` fields.

In `src-tauri/src/commands.rs:730` remove `let vocab_list = vocabulary_list(&cfg.vocabulary);` — Speechmatics was the only caller.

In `src-tauri/src/commands.rs:732-768` (the dispatch `match`), remove the `"speechmatics" => { ... }` arm. Resulting structure:

```rust
let text = match cfg.provider.as_str() {
    "local" => {
        let model_path = local_model_path(&app).map_err(|e| anyhow::anyhow!(e))?;
        let shared = {
            let state: State<AppState> = app.state();
            state.whisper.clone()
        };
        local_whisper::transcribe_file(
            shared,
            model_path,
            &cfg.language,
            vocab_prompt.as_deref(),
            &path,
        )
        .await?
    }
    _ => {
        openai::transcribe_file(
            &cfg.api_key,
            &cfg.openai_model,
            Some(&cfg.language),
            vocab_prompt.as_deref(),
            &path,
        )
        .await?
    }
};
```

(Task 4 will replace `vocab_prompt.as_deref()` with the richer `build_initial_prompt(...)` output. Leave the call shape as `vocab_prompt` here so this commit compiles cleanly.)

- [ ] **Step 7: Drop the now-unused `TranscribeCfg` Speechmatics fields**

In `src-tauri/src/commands.rs:792-800` (struct `TranscribeCfg`), remove the `speechmatics_op` and `speechmatics_region` fields.

- [ ] **Step 8: Drop `vocabulary_list`**

In `src-tauri/src/commands.rs:982-990` delete the `fn vocabulary_list` function and its doc comment. `vocabulary_prompt` stays — it splits and joins for itself.

Update `vocabulary_prompt`'s body so it no longer delegates to the deleted helper:

```rust
fn vocabulary_prompt(raw: &str) -> Option<String> {
    let items: Vec<&str> = raw
        .split(|c: char| c == ',' || c == '\n')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect();
    if items.is_empty() {
        return None;
    }
    Some(items.join(", "))
}
```

- [ ] **Step 9: Verify the backend compiles**

Run: `cd src-tauri && cargo check`
Expected: PASS with zero warnings about unused Speechmatics symbols.

- [ ] **Step 10: Commit**

```bash
git add src-tauri/src/
git commit -m "Drop Speechmatics provider from backend"
```

---

## Task 2: Remove Speechmatics from the frontend

**Files:**
- Modify: `src/lib/ipc.ts:24-35` and `:68-71`
- Modify: `src/pages/Settings.tsx` — multiple regions

- [ ] **Step 1: Drop Speechmatics types and IPC methods in `ipc.ts`**

In `src/lib/ipc.ts:24-33`, the `SettingsKey` union becomes:

```ts
export type SettingsKey =
  | "language"
  | "transcribe_provider"
  | "transcribe_model"
  | "custom_vocabulary"
  | "summary_model"
  | "summary_prompt"
  | "theme";
```

In `src/lib/ipc.ts:35`, the `TranscribeProvider` becomes:

```ts
export type TranscribeProvider = "openai" | "local";
```

In `src/lib/ipc.ts:68-71`, delete `getSpeechmaticsKey`, `setSpeechmaticsKey`, and `testSpeechmaticsKey`.

- [ ] **Step 2: Drop Speechmatics constants and provider option in `Settings.tsx`**

In `src/pages/Settings.tsx:18-27`, the `DEFAULTS` becomes:

```ts
const DEFAULTS: Record<EditableKey, string> = {
  language: "no",
  transcribe_provider: "openai",
  transcribe_model: "whisper-1",
  custom_vocabulary: "",
  summary_model: "gpt-5.4-mini",
  summary_prompt: SUMMARY_PRESETS[0].prompt_no,
};
```

In `src/pages/Settings.tsx:29-46`, replace the providers + Speechmatics ops/regions block with:

```ts
const PROVIDERS_BASE = [
  { value: "openai", label: "OpenAI" },
];
const LOCAL_PROVIDER = { value: "local", label: "Local (Whisper turbo, on-device)" };
```

(Delete `SPEECHMATICS_OPS` and `SPEECHMATICS_REGIONS` entirely.)

- [ ] **Step 3: Narrow the `Provider` type and delete `smKey` state**

In `src/pages/Settings.tsx:82` change:

```ts
type Provider = "openai" | "local";
```

In `src/pages/Settings.tsx:110` delete the `smKey` state line and its setter usage.

- [ ] **Step 4: Update the initial-load effect**

In `src/pages/Settings.tsx:116-131`, the effect becomes:

```ts
useEffect(() => {
  (async () => {
    const [k1, lw] = await Promise.all([
      ipc.getApiKey(),
      ipc.localWhisperStatus(),
    ]);
    setOpenaiKey((p) => ({ ...p, hasKey: !!k1 }));
    setLocal((p) => ({ ...p, status: lw }));
    const entries = await Promise.all(
      (Object.keys(DEFAULTS) as EditableKey[]).map(
        async (key) => [key, (await ipc.getSetting(key)) ?? DEFAULTS[key]] as const
      )
    );
    setS(Object.fromEntries(entries) as Record<EditableKey, string>);
  })();
}, []);
```

- [ ] **Step 5: Simplify `saveKey` and `testKey`**

In `src/pages/Settings.tsx:168-189`, replace both functions with:

```ts
async function saveKey() {
  if (!openaiKey.draft.trim()) return;
  await ipc.setApiKey(openaiKey.draft.trim());
  setOpenaiKey({ draft: "", hasKey: true, testing: false, result: null });
}

async function testKey() {
  setOpenaiKey((p) => ({ ...p, testing: true }));
  try {
    const r = await ipc.testApiKey();
    const result = r.ok
      ? ({ ok: true } as const)
      : ({ ok: false, message: `${r.status}: ${r.error ?? "unknown error"}` } as const);
    setOpenaiKey((p) => ({ ...p, testing: false, result }));
  } catch (e) {
    setOpenaiKey((p) => ({ ...p, testing: false, result: { ok: false, message: String(e) } }));
  }
}
```

- [ ] **Step 6: Drop the Speechmatics API key field**

In `src/pages/Settings.tsx:233-241`, delete the `<Row label="Speechmatics">` block. The OpenAI row at `:224-232` updates its `onSave`/`onTest` props to drop the argument:

```tsx
<ApiKeyField
  state={openaiKey}
  setState={setOpenaiKey}
  placeholder="sk-…"
  onSave={saveKey}
  onTest={testKey}
/>
```

- [ ] **Step 7: Drop the Speechmatics conditional UI**

In `src/pages/Settings.tsx:281-306`, delete the entire `{provider === "speechmatics" && (...)}` block.

- [ ] **Step 8: Update vocabulary helper text**

In `src/pages/Settings.tsx:319` (placeholder) drop `Speechmatics, ` from the placeholder string, leaving `"Tauri, Humla, ScreenCaptureKit, Granola"`.

In `src/pages/Settings.tsx:323-329` (helper paragraph) becomes:

```tsx
<p className="text-xs text-[var(--color-text-muted)] mt-2">
  Comma- or newline-separated. Names, jargon, and uncommon
  spellings — biases the transcriber toward these tokens.
  <code> gpt-4o-transcribe-diarize </code> ignores it.
</p>
```

- [ ] **Step 9: Verify TS + Vite build is clean**

Run: `pnpm tsc --noEmit && pnpm vite build`
Expected: PASS with no unused-import or missing-property errors.

- [ ] **Step 10: Commit**

```bash
git add src/
git commit -m "Drop Speechmatics provider from frontend"
```

---

## Task 3: VAD-based natural-boundary chunking in the Swift sidecar

**Files:**
- Modify: `audio-capture/Sources/audio-capture/main.swift:124-205` (`ChunkWriter`) and `:431-442` (drain handler)

**Approach:** The drain timer fires every 200ms and pushes up to 1s of mixed Float32 samples into `ChunkWriter`. We extend `ChunkWriter` with a frame-level VAD: track consecutive silent frames (peak below threshold). Once the chunk has crossed `minChunkFrames`, the *next* time we see ≥`vadSilenceFrames` of consecutive silence, we rotate. As a safety cap, also rotate at `maxChunkFrames` regardless. The existing `silenceThreshold` per-chunk drop behaviour (delete chunks with no real speech) is preserved.

- [ ] **Step 1: Replace the `ChunkWriter` definition with VAD-aware version**

In `audio-capture/Sources/audio-capture/main.swift:124-203`, replace the entire `final class ChunkWriter { ... }` block with:

```swift
final class ChunkWriter {
    private let dir: URL
    private let minFrames: AVAudioFrameCount
    private let maxFrames: AVAudioFrameCount
    private let vadSilenceFrames: AVAudioFrameCount
    private let silenceThreshold: Float = 0.005   // chunk-level: below this we drop
    private let vadFrameThreshold: Float = 0.008  // per-buffer peak: above this = voice
    private var index: Int = 0
    private var file: AVAudioFile?
    private var url: URL?
    private var written: AVAudioFrameCount = 0
    private var chunkPeak: Float = 0
    private var silentRun: AVAudioFrameCount = 0   // consecutive silent frames
    private let queue = DispatchQueue(label: "chunk.writer")

    init(dir: URL, minSeconds: Double, maxSeconds: Double, vadSilenceMs: Double) {
        self.dir = dir
        self.minFrames = AVAudioFrameCount(minSeconds * targetSampleRate)
        self.maxFrames = AVAudioFrameCount(maxSeconds * targetSampleRate)
        self.vadSilenceFrames = AVAudioFrameCount((vadSilenceMs / 1000.0) * targetSampleRate)
    }

    func write(_ buffer: AVAudioPCMBuffer) {
        queue.sync {
            do {
                if file == nil { try openNext() }
                try file!.write(from: buffer)
                written += buffer.frameLength

                // Per-buffer peak drives both chunk-level peak (for the silence
                // drop on close) and per-chunk silent-run tracking (for VAD).
                var bufPeak: Float = 0
                if let chans = buffer.floatChannelData {
                    let n = Int(buffer.frameLength)
                    for i in 0..<n {
                        let v = abs(chans[0][i])
                        if v > bufPeak { bufPeak = v }
                    }
                }
                if bufPeak > chunkPeak { chunkPeak = bufPeak }

                if bufPeak < vadFrameThreshold {
                    silentRun += buffer.frameLength
                } else {
                    silentRun = 0
                }

                // Rotate on whichever fires first:
                //  - hard cap (maxFrames) so a continuous monologue still gets
                //    transcribed periodically and the prompt-context buffer
                //    stays fresh
                //  - VAD pause detected, but only after the chunk has reached
                //    minFrames so we don't emit micro-chunks
                let vadRotate = written >= minFrames && silentRun >= vadSilenceFrames
                if written >= maxFrames || vadRotate {
                    try rotate()
                }
            } catch {
                emitError("write: \(error.localizedDescription)")
            }
        }
    }

    func close() {
        queue.sync {
            if let u = url, written > 0 {
                file = nil
                if chunkPeak >= silenceThreshold {
                    emit(["event": "chunk", "path": u.path])
                    stats.lock.lock(); stats.chunks += 1; stats.lock.unlock()
                } else {
                    try? FileManager.default.removeItem(at: u)
                }
            }
            file = nil
            url = nil
            written = 0
            chunkPeak = 0
            silentRun = 0
            emit(["event": "stopped"])
        }
    }

    private func openNext() throws {
        index += 1
        let u = dir.appendingPathComponent(String(format: "chunk-%04d.wav", index))
        url = u
        file = try AVAudioFile(forWriting: u, settings: writeSettings)
        written = 0
    }

    private func rotate() throws {
        guard let u = url else { return }
        file = nil
        if chunkPeak >= silenceThreshold {
            emit(["event": "chunk", "path": u.path])
            stats.lock.lock(); stats.chunks += 1; stats.lock.unlock()
        } else {
            try? FileManager.default.removeItem(at: u)
        }
        chunkPeak = 0
        silentRun = 0
        try openNext()
    }
}
```

- [ ] **Step 2: Update the `ChunkWriter` instantiation**

In `audio-capture/Sources/audio-capture/main.swift:205`, replace:

```swift
let writer = ChunkWriter(dir: outDir, chunkSeconds: 20.0)
```

with:

```swift
// Min 1.5s prevents micro-chunks (model context loss). Max 12s caps so a
// monologue with no pauses still gets transcribed periodically and the
// trailing-context prompt stays fresh. 600ms silence ≈ a normal sentence
// pause; tighter values trigger on word-internal stops, looser values
// delay transcription past natural breakpoints.
let writer = ChunkWriter(dir: outDir, minSeconds: 1.5, maxSeconds: 12.0, vadSilenceMs: 600.0)
```

- [ ] **Step 3: Build the sidecar**

Run: `./scripts/build-sidecar.sh`
Expected: success; `src-tauri/binaries/audio-capture-aarch64-apple-darwin` updated.

- [ ] **Step 4: Manual smoke-test the sidecar standalone**

```bash
mkdir -p /tmp/humla-vad-test
./src-tauri/binaries/audio-capture-aarch64-apple-darwin --out /tmp/humla-vad-test &
PID=$!
# Speak into the mic for ~10 seconds with deliberate pauses, then:
sleep 12 && kill -TERM $PID
ls -la /tmp/humla-vad-test/
```

Expected: multiple `chunk-NNNN.wav` files, each between ~1.5s and ~12s of audio (use `afinfo /tmp/humla-vad-test/chunk-0001.wav` to check duration). Boundaries should land on the pauses you made, not at fixed 20s.

- [ ] **Step 5: Commit**

```bash
git add audio-capture/Sources/audio-capture/main.swift
git commit -m "Replace fixed 20s chunks with VAD-detected natural boundaries"
```

---

## Task 4: Trailing transcript context for Whisper prompts

**Files:**
- Modify: `src-tauri/src/recording.rs` — add `TranscriptTrail`
- Modify: `src-tauri/src/commands.rs` — `build_initial_prompt`, mutate trail on commit, clear on start

- [ ] **Step 1: Write the failing test for `TranscriptTrail`**

In `src-tauri/src/recording.rs`, append at the bottom of the file:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trail_keeps_last_n_words() {
        let mut t = TranscriptTrail::new(5);
        t.push("one two three");
        t.push("four five six");
        // capacity 5, total seen 6 words → drops "one"
        assert_eq!(t.as_prompt(), Some("two three four five six".to_string()));
    }

    #[test]
    fn trail_returns_none_when_empty() {
        let t = TranscriptTrail::new(10);
        assert_eq!(t.as_prompt(), None);
    }

    #[test]
    fn trail_caps_at_max_when_pushing_long_text() {
        let mut t = TranscriptTrail::new(3);
        t.push("alpha beta gamma delta epsilon");
        assert_eq!(t.as_prompt(), Some("gamma delta epsilon".to_string()));
    }

    #[test]
    fn trail_clear_drops_history() {
        let mut t = TranscriptTrail::new(5);
        t.push("hello world");
        t.clear();
        assert_eq!(t.as_prompt(), None);
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cd src-tauri && cargo test trail_ -- --nocapture`
Expected: FAIL — `TranscriptTrail` does not exist.

- [ ] **Step 3: Add the `TranscriptTrail` struct and field**

In `src-tauri/src/recording.rs`, add at the top below the imports:

```rust
/// Bounded ring of recent transcript words. Used as Whisper's `initial_prompt`
/// so each chunk decodes with knowledge of what was just said — sentence
/// continuity, proper-noun spelling, and a real prior context that suppresses
/// silence-driven hallucinations like "Thanks for watching".
pub struct TranscriptTrail {
    words: std::collections::VecDeque<String>,
    capacity: usize,
}

impl TranscriptTrail {
    pub fn new(capacity: usize) -> Self {
        Self {
            words: std::collections::VecDeque::with_capacity(capacity),
            capacity,
        }
    }

    pub fn push(&mut self, text: &str) {
        for w in text.split_whitespace() {
            if self.words.len() == self.capacity {
                self.words.pop_front();
            }
            self.words.push_back(w.to_string());
        }
    }

    pub fn as_prompt(&self) -> Option<String> {
        if self.words.is_empty() {
            None
        } else {
            Some(
                self.words
                    .iter()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(" "),
            )
        }
    }

    pub fn clear(&mut self) {
        self.words.clear();
    }
}
```

In `RecordingSession` (currently `src-tauri/src/recording.rs:11-23`), add a field:

```rust
#[derive(Default)]
pub struct RecordingSession {
    pub note_id: Option<String>,
    pub child: Option<Child>,
    pub temp_dir: Option<PathBuf>,
    pub stop_tx: Option<mpsc::Sender<()>>,
    pub inflight: Inflight,
    pub reader: Option<JoinHandle<()>>,
    /// Rolling context window of the last ~150 committed words. Fed to
    /// Whisper's `initial_prompt` for every chunk so decoding stays anchored
    /// to the conversation rather than treating each chunk as a cold start.
    pub trail: Arc<Mutex<TranscriptTrail>>,
}
```

`TranscriptTrail` doesn't have `Default`, but `RecordingSession` derives `Default` — fix by replacing the derive with a manual impl OR by adding `Default` for `TranscriptTrail`. Add this below `TranscriptTrail`:

```rust
impl Default for TranscriptTrail {
    fn default() -> Self {
        // 150 words ≈ ~200 Whisper tokens, which fits inside the 224-token
        // prompt budget alongside ~50 tokens of custom vocabulary.
        Self::new(150)
    }
}
```

- [ ] **Step 4: Run tests and verify they pass**

Run: `cd src-tauri && cargo test trail_`
Expected: 4 passed.

- [ ] **Step 5: Add `build_initial_prompt` helper in `commands.rs`**

In `src-tauri/src/commands.rs`, just below the `vocabulary_prompt` function, add:

```rust
// Whisper's decoder accepts an `initial_prompt` that conditions the next
// decode as if it were the previous segment. We compose two parts: the user's
// custom vocabulary (proper-noun bias) and the tail of what's already been
// committed in this session. Combined, this carries sentence continuity,
// proper-noun spelling, and a non-empty prior — which is the single best
// mitigation for Whisper's silence/short-clip hallucinations.
//
// Budget note: Whisper's prompt context is ~224 tokens. Vocabulary is
// typically <50 tokens; the trail is bounded to 150 words (~200 tokens).
// We accept slight overflow — whisper.cpp truncates internally.
fn build_initial_prompt(vocabulary: &str, trail: Option<String>) -> Option<String> {
    let vocab = vocabulary_prompt(vocabulary);
    match (vocab, trail) {
        (None, None) => None,
        (Some(v), None) => Some(v),
        (None, Some(t)) => Some(t),
        (Some(v), Some(t)) => Some(format!("{v}\n\n{t}")),
    }
}
```

- [ ] **Step 6: Wire the trail into `transcribe_chunk`**

In `src-tauri/src/commands.rs:732-768` (the dispatch `match`), replace `vocab_prompt.as_deref()` in both arms with a snapshot of the combined prompt taken before the dispatch. Insert above the match:

```rust
let trail_snapshot = {
    let state: State<AppState> = app.state();
    let session = state.recording.lock();
    session.trail.lock().as_prompt()
};
let prompt = build_initial_prompt(&cfg.vocabulary, trail_snapshot);
```

Then both arms use `prompt.as_deref()` instead of `vocab_prompt.as_deref()`. Also remove the now-unused `vocab_prompt` line at `:729`.

- [ ] **Step 7: Append committed text to the trail**

In `src-tauri/src/commands.rs:778-788` (where `trimmed` is appended to the DB), add the trail update right after `db::append_transcript`:

```rust
if !trimmed.is_empty() {
    let state: State<AppState> = app.state();
    {
        let conn = state.db.lock();
        db::append_transcript(&conn, &note_id, &trimmed)?;
    }
    {
        let session = state.recording.lock();
        session.trail.lock().push(&trimmed);
    }
    let _ = app.emit("transcript_appended", TranscriptPayload {
        note_id: note_id.clone(),
        text: trimmed,
    });
}
```

- [ ] **Step 8: Clear the trail when a new recording starts**

In `src-tauri/src/commands.rs:512-519` (where `state.recording.lock()` is updated with a fresh inflight list at session start), add:

```rust
let inflight: Inflight = Arc::new(parking_lot::Mutex::new(Vec::new()));
{
    let mut s = state.recording.lock();
    s.note_id = Some(note_id.clone());
    s.child = Some(child);
    s.temp_dir = Some(temp_dir);
    s.inflight = inflight.clone();
    s.trail.lock().clear();
}
```

- [ ] **Step 9: Verify the backend compiles + tests pass**

Run: `cd src-tauri && cargo check && cargo test`
Expected: PASS, 4 trail tests green.

- [ ] **Step 10: Commit**

```bash
git add src-tauri/src/
git commit -m "Feed trailing transcript + vocabulary as Whisper initial_prompt"
```

---

## Task 5: Documentation cleanup

**Files:**
- Modify: `README.md`
- Modify: `CLAUDE.md`

- [ ] **Step 1: Drop Speechmatics from `README.md`**

Open `README.md`, search for every occurrence of "Speechmatics" (per the recon: lines 3, 9, 13, 64, 67–68) and remove the references. Adjust surrounding prose so the multi-provider language reads naturally as "OpenAI or on-device Whisper". Don't rewrite sections that aren't about transcription.

- [ ] **Step 2: Drop Speechmatics from `CLAUDE.md`**

Open `CLAUDE.md`, find the Speechmatics mentions (per the recon: lines 12, 15, 35, 71, 90). For each, either delete or rewrite the bullet so the remaining provider list is "OpenAI (Whisper / gpt-4o-transcribe / gpt-4o-mini-transcribe / gpt-4o-transcribe-diarize) or **on-device Whisper** via Metal." Leave the architecture diagram intact except the HTTPS clients box: drop "Speechmatics" from the listed services.

In the same pass, add a one-line note in the architecture overview describing the trailing-context behaviour, e.g. under "Two-source summaries" add a new bullet:

> **Trailing transcript context** — every chunk's transcription receives the last ~150 committed words plus the custom vocabulary as Whisper's `initial_prompt`, so decoding stays anchored to the conversation and silence-driven hallucinations are largely suppressed.

And update the chunking sentence under "Data flow during a recording" — `5-second WAV chunks` → `VAD-bounded WAV chunks (1.5–12s, rotated at silence pauses)`.

- [ ] **Step 3: Commit**

```bash
git add README.md CLAUDE.md
git commit -m "Update docs for VAD chunking + Speechmatics removal"
```

---

## Task 6: End-to-end smoke test

**Goal:** Verify the new pipeline works with a real recording.

- [ ] **Step 1: Build sidecar + run app**

```bash
./scripts/build-sidecar.sh
pnpm tauri dev
```

- [ ] **Step 2: Smoke a local-Whisper recording**

In the running app:
1. Settings → ensure Provider = "Local (Whisper turbo, on-device)" and the model is downloaded.
2. Settings → Custom vocabulary → enter a few proper nouns including one tricky spelling (e.g. "Humla, Speechmatics, ScreenCaptureKit").
3. Open a fresh note. Hit Record. Speak in clear sentences with deliberate pauses, including one of the proper-noun terms early on. Speak for ~30s.
4. Stop. Trigger summary.

Expected:
- Transcript chunks arrive at sentence boundaries (you can see them appear when you pause speaking, not at fixed 20s ticks).
- The proper-noun term, if Whisper got it right early, stays consistent in later chunks.
- The "Settings → Provider" dropdown shows only "OpenAI" and "Local"; no Speechmatics.

- [ ] **Step 3: Smoke an OpenAI recording**

Repeat the recording with Provider = "OpenAI", model = `whisper-1`. Verify the same continuity behaviour (each chunk's prompt now contains the trail from the previous chunks).

- [ ] **Step 4: Verify Speechmatics settings in the DB are inert (not deleted)**

Old Speechmatics keys remain in the `settings` table but nothing reads them. Confirm with:

```bash
sqlite3 ~/Library/Application\ Support/no.humla.app/notes.sqlite \
  "SELECT key FROM settings WHERE key LIKE '%speechmatics%';"
```

Expected: zero or more rows; the app does not touch them. (Optional cleanup on a future migration — out of scope here.)

- [ ] **Step 5: If anything is off, fix and commit. Otherwise wrap.**

If the transcript trail visibly hurts accuracy (e.g. error propagation), the most likely cause is `is_likely_hallucination` letting filler past on short chunks; tighten that filter rather than ripping out the trail.

If chunk boundaries land at awkward points (mid-word), increase `vadSilenceMs` from 600 → 800 in `main.swift`.

If a continuous monologue doesn't surface for ~12s, that's expected (the max-cap kicks in); lower `maxSeconds` to 8.0 if it feels too long.

---

## Out of scope (intentional non-goals)

- **Two-pass commit logic.** The plan feeds back *every* committed chunk's text into the trail; the production-grade version of streaming Whisper only commits text after two consecutive passes confirm it. We're betting that the existing silence gate + hallucination filter + attribution-tail strip are tight enough that bad text rarely reaches the commit. If that turns out wrong, add two-pass commit as a follow-up.
- **Overlapping audio windows.** Sliding-window here means *prompt* context (last 150 committed words), not *audio* context (overlapping windows). Audio overlap is harder to dedupe and we already have natural-boundary chunking from VAD.
- **Removing the Speechmatics rows from the `settings` table.** They're inert; a future migration can sweep them.
- **Live "tentative tail" UI treatment.** The current per-chunk transcript display works fine with VAD-bounded chunks. Designing a dedicated streaming UI is a separate piece of work.
