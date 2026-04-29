# Humla — project notes

## What this app is

**Humla** is a personal macOS meeting-notes app inspired by Granola. You take freeform notes during a meeting; in parallel, the app records mic + system audio, transcribes the call, and produces a structured AI summary that fuses your notes with the transcript. Built for personal/small-team use, not SaaS — your data, your API keys, local SQLite, no backend.

The name is Norwegian for "bumblebee" (think: small, hum, personal).

## Core capabilities

- **Hybrid capture** — mic input + macOS system audio (the other side of the call) recorded simultaneously via a Swift sidecar.
- **Two transcription providers** — pick per-note between OpenAI (Whisper / gpt-4o-transcribe / gpt-4o-mini-transcribe / gpt-4o-transcribe-diarize) or **on-device Whisper** via Metal.
- **Whisper quality preset** — `Fast` (greedy, snappy) / `Balanced` (beam=3) / `Quality` (beam=5, low no_speech threshold) for the local provider; bundles sampling strategy + confidence thresholds together so the user picks one knob.
- **Per-note transcription language** — global Settings → Language is the default for new notes; each note has its own language chip that overrides for that note.
- **Live speaker diarization** — a second Swift sidecar (`speaker-diarize`, FluidAudio CoreML) runs continuously while a recording is in flight, classifying each Whisper chunk against persistent speaker memory. Transcripts get `Speaker 1: …` / `Speaker 2: …` tags inline as they're appended. Sidecar prewarms at app launch (or right after model download) so chunk 1 lands tagged.
- **Speaker rename + colour-coded pills** — each unique speaker gets one of four semantic colours from the design tokens (interactive blue, success green, warning gold, accent red, cycling for 5+). A chip strip above the transcript lets the user click any speaker to rename inline; rename is a regex line-anchored rewrite of the transcript text — no separate metadata table.
- **Auto-polish on stop** — every recording goes through an LLM cleanup pass after stop (configured `summary_model`, defaults to `gpt-5.4-mini`). Conservative prompt: only fixes typos / chunk-boundary cuts / missing punctuation; preserves line structure, filler words, and `Speaker N:` labels exactly. Bottom-right toast surfaces the active phase.
- **Two-source summaries** — the model gets `[Notater]` (your typed notes) and `[Transkripsjon]` (the meeting transcript) as separate inputs, with a system prompt that tells it to favour your notes for intent and the transcript for facts.
- **Per-note presets** — Meeting / 1:1 / Lecture / Interview / Brainstorm / Voice memo, each with its own summary prompt. Custom prompts also supported.
- **Custom vocabulary** — a per-user list of names, tech terms, and phrases sent as part of Whisper's `initial_prompt` to bias decoding toward those tokens.
- **Trailing transcript context** — every chunk's transcription receives the last ~150 committed words alongside the custom vocabulary as Whisper's `initial_prompt`, so decoding stays anchored to the conversation rather than treating each chunk as a cold start. Single biggest mitigation against silence-driven hallucinations and proper-noun drift across the meeting.
- **VAD-bounded chunks** — the audio-capture sidecar rotates each chunk at natural speech pauses (min 1.0 s / max 15 s / 500 ms silence trigger) instead of a fixed timer, so chunk boundaries land mid-pause rather than mid-word.
- **Reasoning-model temperature handling** — gpt-5.x and o-series models reject custom temperature; `openai::summarize` detects them via `is_reasoning_model()` and omits the parameter, while keeping `temperature=0.2` for traditional chat models.
- **Folders** — flat folder list, per-note assignment, search across titles/bodies/transcripts/folder names with auto-expand on hits.
- **Click-to-edit transcript** — styled view by default with coloured pills + plain text; clicking enters a textarea for edits. Locked while a recording is in flight to avoid clobber.

## Architecture overview

```
┌─────────────────────────────────────────────────────────────┐
│ React + Vite frontend (src/)                                │
│  Tiptap editor · Zustand store · React Router · Tailwind v4 │
└──────────────────────┬──────────────────────────────────────┘
                       │ Tauri IPC (invoke / events)
┌──────────────────────▼──────────────────────────────────────┐
│ Rust backend (src-tauri/src/)                               │
│  commands.rs · db.rs · recording.rs · openai.rs ·           │
│  diarize.rs · local_whisper.rs · presets.rs · wav.rs        │
│                                                             │
│  ┌─────────────┐  ┌─────────────────┐  ┌─────────────────┐  │
│  │SQLite(rusql)│  │ audio-capture   │  │ speaker-diarize │  │
│  │ notes /     │  │ sidecar (Swift) │  │ sidecar (Swift) │  │
│  │ folders /   │  │ AVAudioEngine + │  │ FluidAudio      │  │
│  │ settings    │  │ ScreenCaptureKit│  │ (CoreML / ANE)  │  │
│  └─────────────┘  └─────────────────┘  └─────────────────┘  │
│                                                             │
│  ┌─────────────────────────────────┐  ┌─────────────────┐   │
│  │ HTTPS clients                   │  │ Local Whisper   │   │
│  │ OpenAI · HuggingFace (model dl) │  │ whisper-rs/Metal│   │
│  └─────────────────────────────────┘  └─────────────────┘   │
└─────────────────────────────────────────────────────────────┘
```

### Data flow during a recording

1. **App launch (background)** — if the diarization model is on disk, spawn the `speaker-diarize` sidecar in streaming mode and store its handle on `AppState.diarize_stream`. This warms the CoreML model + ANE compile early so the first recording starts with a tagged chunk 1 rather than a 1–30 s warmup window.
2. **`recording_start`** spawns the `audio-capture` sidecar via `setsid` (sandbox-detached so TCC prompts go to *Humla* itself, not Terminal). If the streaming diarize sidecar is running, send a `{cmd:"reset"}` line to wipe its `SpeakerManager` so the new recording doesn't match against the previous meeting's voices. If it's not running, kick off `ensure_streaming_running` in the background.
3. **Sidecar capture** — `AVAudioEngine` (mic) + `ScreenCaptureKit` (system audio) → mixer → VAD-bounded WAV chunks (1.0–15 s; rotates on 500 ms silence once min length reached, hard cap on max). Sidecar emits `{event:"chunk", path, start_ms}` events on stdout. A parallel `FullRecordingWriter` writes everything to one `full.wav` for the duration of the recording (currently kept around but unused; reserved for future hybrid-finalize).
4. **Rust reader thread** parses each `chunk` event and spawns `transcribe_chunk` on a tokio task tracked in `RecordingSession.inflight`. Concurrent chunks serialise on `transcribe_gate` so each one's `initial_prompt` sees a fresh trail snapshot.
5. **`transcribe_chunk`**:
   1. Read provider config + per-note language (`note.language || global`).
   2. Skip near-silent chunks via `wav::rms` gate.
   3. Acquire `transcribe_gate`. Build initial prompt from custom vocab + `TranscriptTrail` snapshot.
   4. Call provider (local whisper-rs or OpenAI multipart).
   5. Run `is_likely_hallucination` + `strip_attribution_tail`.
   6. **Diarize**: if streaming sidecar is up, send `{path}` and read the response — get a `speaker_id` (or `null`).
   7. Format with prefix: speaker change → `\nSpeaker N: <text>`; same speaker → ` <text>`. Display number `N` is assigned 1-indexed in first-encounter order via `RecordingSession.speaker_display`.
   8. `db::append_transcript(text, separator)` — caller controls whitespace exactly.
   9. Push the *raw* text into `TranscriptTrail` for the next chunk's prompt context.
   10. Emit `transcript_replaced` with the full new transcript (previously `transcript_appended` with auto-space, but speaker prefixes need newline control).
6. **Frontend live update** — `useRecordingStore` listens for `transcript_replaced` and updates the note's transcript in `useNotesStore`. The Note view's transcript card re-derives speaker labels from the text on every render and renders coloured pills inline.
7. **`recording_stop`** — SIGTERM the audio-capture sidecar → 3 s grace → SIGKILL fallback → drain inflight handles + reader handle. The `speaker-diarize` sidecar is **NOT** torn down — it lives across recordings, reset by the next `recording_start`.
8. **Auto-polish (background task)** — after drain, spawn `polish_transcript` which fetches the configured `summary_model`, sends transcript + notes + custom vocab to chat completions with a strict "preserve line structure and speaker labels" prompt, and replaces the transcript with the polished version. Bottom-right toast shows `Polishing…` while it runs. Settles to `Phase::Idle` when done.
9. **Crash recovery** — sidecar stdout EOF detection resets the session and emits an error toast. The audio-capture sidecar polls its PPID every 2 s and self-exits if it sees PID 1 (parent died), so dev-reload zombies clean themselves up.
10. **Summary** is fired manually via `summarize_note`, which reads `note.body` (HTML → plain text) + `note.transcript`, resolves the preset's prompt, appends a language directive, and calls OpenAI. Same provider/key/model as polish; reasoning models (gpt-5.x / o-series) get `temperature` omitted automatically.

## Tech stack

### Frontend (`src/`)

- **React 19** + **TypeScript** + **Vite 6**
- **Tauri 2** — `@tauri-apps/api` for `invoke` + event listeners; webview-based UI
- **React Router 7** — note routing (`/note/:id`), settings, home
- **Zustand** — `useNotesStore` (notes/folders) + `useRecordingStore` (status/errors/diagnostics); backend events bound once via `bindBackendListeners`. Listens for `transcript_appended`, `transcript_replaced`, `summary_ready`, `recording_status`, `recording_error`, `recording_diagnostic`, `local_whisper_progress`, `diarize_download_progress`.
- **Tiptap v2** — body editor (StarterKit + Placeholder + Suggestion + BubbleMenu).
- **Transcript view** — styled-by-default with `white-space: pre-wrap` so its rendered height matches the textarea exactly (no per-line margin → no page-jump on click-to-edit). Speaker labels rendered as inline `nd-speaker-pill` chips; rest of line is plain text.
- **`SpeakerLabels` chip strip** — derives unique speaker labels from the transcript on every render; one chip per speaker; click to inline-rename. Rename rewrites the transcript via line-anchored regex (`/^Speaker N: /gm` → `/^Michael: /gm`).
- **`PolishToast`** — bottom-right global toast that surfaces `Phase::Diarizing` and `Phase::Polishing` with phase-specific copy.
- **`Updater`** — Tauri auto-update flow; polls `latest.json` from GitHub releases on launch.
- **react-markdown** + **remark-gfm** — summary rendering
- **Tailwind v4** — `@tailwindcss/vite` plugin; design tokens in `src/styles/globals.css`. Base resets are wrapped in `@layer base` so utility classes can override them via cascade — see the v0.3.1 commit for context (the "Install & Restart button is invisible" bug).
- **lucide-react** — icon set
- **Nothing-design aesthetic** — Space Grotesk + Space Mono, monochrome palette, system-aware dark/light. Custom utilities: `.nd-chip`, `.nd-speaker-pill`, `.nd-action`, `.nd-label`, `.nd-bare`. Speaker pill colours come from `--color-interactive` / `--color-success` / `--color-warning` / `--color-accent`, assigned in first-encounter order, cycling for 5+ speakers.

### Backend (`src-tauri/src/`)

- **Rust** + **Tauri 2** runtime (rust-version 1.85 because `fluidaudio-rs` was briefly a candidate dep with that MSRV; the bump survived even though we ended up using a Swift sidecar instead).
- **rusqlite** (`bundled` feature) — single SQLite DB at `~/Library/Application Support/no.humla.app/`. Idempotent ALTER TABLE migrations; index creation runs *after* migrations.
- **reqwest** with `rustls-tls` + `stream` — all HTTPS (OpenAI, Hugging Face for model download).
- **tokio** — async runtime. `spawn_blocking` wraps local Whisper inference. Use `tauri::async_runtime::spawn` (NOT `tokio::spawn`) anywhere that runs from Tauri's `setup` closure — the setup callback runs on the main thread before tokio's runtime is attached, and a bare `tokio::spawn` panics with "no current Tokio runtime", which propagates through the AppKit FFI as `panic_cannot_unwind` and `abort()`s the app on launch (seen in v0.6.1).
- **whisper-rs 0.13** with `metal` feature — bundles whisper.cpp via cmake, runs `large-v3-turbo-q5_0` (~547 MB) on Apple Silicon GPUs.
- **parking_lot** — synchronous mutex for session state. NEVER hold a `parking_lot` guard across an `.await` — the future becomes non-Send and Tauri command futures must be Send. Use `tokio::sync::Mutex` for state that's accessed across await points (e.g. `transcribe_gate`, `diarize_stream`).
- **serde** / **serde_json** — IPC + provider payloads + sidecar JSON streams.
- **chrono** — timestamps.
- **uuid** — note + folder IDs.
- **anyhow** — Rust-side error type; converted to `String` at the IPC boundary.

### Module map

| File | Responsibility |
|---|---|
| `lib.rs` | `AppState`, command registration, plugin setup, app-launch prewarm of the streaming diarize sidecar |
| `main.rs` | Tauri entry |
| `commands.rs` | All `#[tauri::command]` fns; recording lifecycle; transcribe fan-out; auto-polish; summary; folders; settings; diarize model lifecycle |
| `db.rs` | SQLite schema, migrations, CRUD for notes/folders/settings. `append_transcript(text, separator)` lets the caller control the join character (space for same-speaker, newline for speaker-change) |
| `recording.rs` | `RecordingSession` (child handles, inflight tasks, reader handle, `chunk_log`, `full_wav_path`, `speaker_display` map, `last_speaker`); `TranscriptTrail` (rolling 150-word window fed to Whisper as `initial_prompt`); `Phase` enum (`Idle` / `Starting` / `Recording` / `Paused` / `Stopping` / `Diarizing` / `Polishing` / `Summarizing`) |
| `openai.rs` | Transcription + summary HTTP clients; default summary system prompt; `is_reasoning_model()` for temperature handling |
| `local_whisper.rs` | On-device Whisper; `SharedContext` (lazy-loaded model, reused across chunks); `prewarm()` fires on `recording_start`; `Preset` enum (Fast/Balanced/Quality) bundling sampling strategy + `no_speech_thold` |
| `diarize.rs` | Speaker-diarize sidecar wrapper. Three modes: one-shot `diarize_file` (offline, currently unused), `StreamingDiarizer` (long-running classifier with `start` / `classify` / `reset` / `shutdown`), and model lifecycle (`status` / `download` / `delete`). `ensure_streaming_running` is idempotent — used by app launch + post-download to spin up the sidecar exactly once |
| `presets.rs` | Backend mirror of frontend preset prompts; `{LANGUAGE}` substitution |
| `wav.rs` | Proper RIFF chunk walking; RMS for silence gate; mono-16k decoder |

### Sidecars

Two Swift Package binaries that run alongside the Tauri main process. Both are bundled via `tauri.conf.json`'s `bundle.macOS.externalBin` and signed with the same Developer ID.

#### `audio-capture/` — recording

- **AVFoundation** for mic, **ScreenCaptureKit** for system audio.
- **Hidden from Dock** via `NSApplication.shared.setActivationPolicy(.prohibited)`.
- Built via `scripts/build-sidecar.sh`. Binary cached via SHA-256 stamp at `src-tauri/binaries/.audio-capture-<triple>.stamp` (override with `FORCE_SIDECAR_REBUILD=1`).
- **Parent-death watchdog** — polls `getppid()` every 2 s; exits if it sees PID 1 (reparented to launchd). Combined with the `setsid` detach in `recording_start`, this prevents zombie sidecars after dev reloads / app crashes.
- Emits these stdout events: `chunk` (with `path` + `start_ms`), `full_recording` (final `path` + `duration_ms`), `stopped`, `paused`, `resumed`, `heartbeat` (frame counts + peaks), `error`.
- Writes a parallel `full.wav` for the entire recording in addition to per-chunk WAVs (used by the offline diarize path; currently dead code but plumbing stays).

#### `speaker-diarize/` — speaker classification

- **FluidAudio Swift package** (depends on `FluidInference/FluidAudio`, Apache 2.0). Uses the streaming `DiarizerManager` (pyannote 3.1 CoreML) with `clusteringThreshold: 0.5` for aggressive separation on system-audio captures where voices share a downstream codec.
- Built via `scripts/build-diarize.sh` — same Developer ID + hardened runtime as audio-capture, but no entitlements file (it just reads a WAV and runs CoreML inference; no mic / screen capture needed).
- Subcommand-style CLI:
  - `speaker-diarize <wav>` — one-shot offline diarization (returns segment array).
  - `speaker-diarize streaming` — long-running classifier; reads `{cmd:"classify", path}` or `{cmd:"reset"}` JSON lines on stdin, writes `{path, speaker_id}` or `{event:"reset_done"}` on stdout. Stays alive until stdin closes.
  - `speaker-diarize status` — checks model presence on disk; emits `{downloaded, sizeBytes, path}` JSON.
  - `speaker-diarize download` — fetches + compiles model; streams `{event:"progress", fraction, phase}` updates (phase ∈ `listing` / `downloading` / `compiling`) followed by `{event:"done"}`.
  - `speaker-diarize delete` — wipes the cache directory.
- Streaming sidecar lifecycle: spawned at app launch (via `lib.rs`'s `tauri::async_runtime::spawn`) if model is downloaded, OR on first `recording_start` if it isn't. `recording_start` sends a `reset` to wipe SpeakerManager between recordings. `recording_stop` does NOT shut down the sidecar — it lives across recordings. Only torn down on `diarize_delete` or app quit.

## macOS specifics

- **Bundle id** `no.humla.app` — TCC keys on this. Pre-rename `com.notes-app.local` permissions don't transfer.
- **Entitlements** (`src-tauri/entitlements.plist`) — mic input, network client, screen capture usage description, no app-sandbox.
- **TCC pain point** — every rebuild that re-signs the sidecar invalidates the trusted-binary entry. Fix: the stamp-based cache in `build-sidecar.sh`, or graduate to notarised builds (see "Deferred: notarised distribution" below).
- **Tauri webview limitation** — `window.prompt` / `confirm` / `alert` are blocked by the Tauri webview to avoid main-thread deadlock. We use inline input UIs everywhere (folder creation in Sidebar + Note's FolderPicker).

## Local data layout

- **DB** — `~/Library/Application Support/no.humla.app/notes.sqlite` (SQLite, WAL mode). Schema: `notes` (with `language`, `summary_preset`, `folder_id` columns), `folders`, `settings`.
- **Settings** — `settings` table inside the same DB. Keys: `language`, `transcribe_provider`, `transcribe_model`, `whisper_preset`, `custom_vocabulary`, `summary_model`, `summary_prompt`, `theme`, plus opaque secret rows `__openai_api_key__`.
- **Local Whisper model** — `~/Library/Application Support/no.humla.app/models/ggml-large-v3-turbo-q5_0.bin` (~547 MB, downloaded on demand from HuggingFace).
- **FluidAudio diarization model** — `~/Library/Application Support/no.humla.app/FluidAudio/Models/` (~15 MB total of `.mlmodelc` directories, downloaded on demand from HuggingFace + compiled for Apple Neural Engine on first use).
- **Audio temp** — `tempfile::TempDir` per recording session; cleaned 30 s after stop. Contains per-chunk WAVs + the full-recording `full.wav`.

## Build & distribution

| Command | What it does |
|---|---|
| `pnpm dev` | Vite dev server only (frontend) |
| `pnpm tauri dev` | Tauri dev (assumes sidecars already built) |
| `./scripts/build-sidecar.sh` | Build + Developer ID sign the audio-capture Swift sidecar (skips if unchanged) |
| `./scripts/build-diarize.sh` | Build + Developer ID sign the speaker-diarize Swift sidecar (skips if unchanged) |
| `pnpm icon` | Regenerate the macOS app icon from `src-tauri/icons/source.png` |
| `pnpm tauri build` | Production bundle (`.app` + `.dmg`) — calls both sidecar build scripts via `beforeBuildCommand` chain |
| `pnpm dmg` | Wrapper: builds both sidecars, then `pnpm tauri build`; prints final DMG path |
| `pnpm release` | Full release pipeline: build + notarise + staple + sign updater payload + tag + push + GitHub release |

DMG output lands in `src-tauri/target/release/bundle/dmg/`. Currently ad-hoc signed only — see distribution notes below.

## Distribution & signing

Builds are signed with the **Developer ID Application: MICHAEL MEHLUM WILHELMSEN (NBUP88JQ35)** identity (configured in `src-tauri/tauri.conf.json` under `bundle.macOS.signingIdentity`). The Swift sidecar gets the same Developer ID + hardened runtime + `src-tauri/sidecar.entitlements` (mic input).

Stable Developer ID signature means **TCC permissions persist across rebuilds** — Microphone / Screen Recording grants stay valid as long as the cert is the same.

### Notarisation

Notarytool credentials live in `.env.notarise` (gitignored) at the repo root:

```
export APPLE_API_KEY=<10-char Key ID>
export APPLE_API_ISSUER=<Issuer UUID>
export APPLE_API_KEY_PATH=/Users/michaelwilhelmsen/.private_keys/AuthKey_<Key ID>.p8
```

`scripts/build-dmg.sh` sources this file before invoking `pnpm tauri build`. Tauri's bundler detects the env vars and runs `xcrun notarytool submit --wait` + stapler automatically.

If `.env.notarise` is absent, the build is still Developer ID signed but not notarised — first launch needs right-click → Open.

### Verifying a release

```
spctl --assess -vv /Applications/Humla.app
# expect: accepted, source=Notarized Developer ID
```

### Reading notarisation failure logs

```
xcrun notarytool log <submission-id> \
  --key $APPLE_API_KEY_PATH \
  --key-id $APPLE_API_KEY \
  --issuer $APPLE_API_ISSUER \
  | jq
```

Common failure causes: nested binary missing hardened runtime, missing entitlement, wrong identifier on a Framework, executable bit lost during copy.

## Releases

Run `pnpm release` to ship a new version. The script builds a notarised + stapled DMG, signs an updater manifest, creates a GitHub release, and uploads all assets so existing installs see the update.

**Before each release, bump the version number in three places** (they must match exactly, or auto-update will misbehave):

1. `package.json` → `"version": "0.1.X"`
2. `src-tauri/tauri.conf.json` → `"version": "0.1.X"`
3. `src-tauri/Cargo.toml` → `version = "0.1.X"`

Convention: semver. Bug fix → patch (`0.1.0` → `0.1.1`). New feature → minor (`0.1.0` → `0.2.0`). Breaking schema change → major (rare for us).

Then:

```
pnpm release
```

The script:
1. Refuses to run if the working tree is dirty or the version isn't bumped beyond the latest GitHub release.
2. Builds the DMG (`pnpm dmg`), which signs + notarises + staples + produces a `.sig` file via the Tauri updater key.
3. Generates `latest.json` with version, signature, and the GitHub download URL.
4. Tags the commit `v<version>`, pushes the tag, creates a GitHub release, uploads `.dmg` + `.sig` + `latest.json` as assets.

All existing Humla installs poll the updater endpoint at startup and prompt to install when a new version lands.

### Updater signing key

Tauri's auto-updater uses a separate Ed25519 keypair from the Apple Developer ID — it signs the **update payload** so the app can verify the DMG hasn't been tampered with before installing.

- **Private key**: `~/.private_keys/humla-updater.key` (passwordless, ~700 perms). Treat with the same care as the notarisation `.p8`. Losing it means you can't ship updates that existing installs will accept — you'd have to publish a new app with a new public key.
- **Public key**: lives in `src-tauri/tauri.conf.json` under `plugins.updater.pubkey`. Bundled into every build. Don't change it once you've shipped or every existing install stops accepting updates.
- The build script reads the private key path from `.env.notarise` (env var `TAURI_SIGNING_PRIVATE_KEY`).

### Verifying a release

```
spctl --assess -vv /Applications/Humla.app
# expect: accepted, source=Notarized Developer ID
```

### Reading notarisation failure logs

```
xcrun notarytool log <submission-id> \
  --key $APPLE_API_KEY_PATH \
  --key-id $APPLE_API_KEY \
  --issuer $APPLE_API_ISSUER \
  | jq
```

Common failure causes: nested binary missing hardened runtime, missing entitlement, wrong identifier on a Framework, executable bit lost during copy.
