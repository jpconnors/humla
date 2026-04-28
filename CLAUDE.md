# Humla — project notes

## What this app is

**Humla** is a personal macOS meeting-notes app inspired by Granola. You take freeform notes during a meeting; in parallel, the app records mic + system audio, transcribes the call, and produces a structured AI summary that fuses your notes with the transcript. Built for personal/small-team use, not SaaS — your data, your API keys, local SQLite, no backend.

The name is Norwegian for "bumblebee" (think: small, hum, personal).

## Core capabilities

- **Hybrid capture** — mic input + macOS system audio (the other side of the call) recorded simultaneously via a Swift sidecar.
- **Multi-provider transcription** — pick per-note between OpenAI (Whisper / gpt-4o-transcribe / gpt-4o-mini-transcribe / gpt-4o-transcribe-diarize), Speechmatics (multi-region), or **on-device Whisper** via Metal.
- **Two-source summaries** — the model gets `[Notater]` (your typed notes) and `[Transkripsjon]` (the meeting transcript) as separate inputs, with a system prompt that tells it to favour your notes for intent and the transcript for facts.
- **Per-note presets** — Meeting / 1:1 / Lecture / Interview / Brainstorm / Voice memo, each with its own summary prompt. Custom prompts also supported.
- **Custom vocabulary** — a per-user list of names, tech terms, and phrases that fans out as a Whisper `prompt`, a local-model `initial_prompt`, and Speechmatics `additional_vocab`.
- **Folders** — flat folder list, per-note assignment, search across titles/bodies/transcripts/folder names with auto-expand on hits.
- **Editable transcript** — Whisper can mishear; you can correct directly in the transcript pane (edits are blocked while a recording is in flight to avoid clobber).
- **Manual summarise button** — explicit action, not auto-fired on stop.

## Architecture overview

```
┌─────────────────────────────────────────────────────────────┐
│ React + Vite frontend (src/)                                │
│  Tiptap editor · Zustand store · React Router · Tailwind v4 │
└──────────────────────┬──────────────────────────────────────┘
                       │ Tauri IPC (invoke / events)
┌──────────────────────▼──────────────────────────────────────┐
│ Rust backend (src-tauri/src/)                               │
│  commands.rs · db.rs · recording.rs · openai.rs · …         │
│                                                             │
│  ┌────────────────┐  ┌─────────────────┐  ┌──────────────┐  │
│  │ SQLite (rusql) │  │ Swift sidecar   │  │ HTTPS clients│  │
│  │ notes/folders/ │  │ (audio-capture) │  │ OpenAI       │  │
│  │ settings       │  │ AVAudioEngine + │  │ Speechmatics │  │
│  │                │  │ ScreenCaptureKit│  │ HF (model dl)│  │
│  └────────────────┘  └─────────────────┘  └──────────────┘  │
│                                                             │
│  Local Whisper (whisper-rs + Metal, in-process)             │
└─────────────────────────────────────────────────────────────┘
```

### Data flow during a recording

1. User hits Record on a note → `recording_start` spawns the Swift sidecar via `setsid` (sandbox-detached so TCC prompts go to *Humla* itself, not Terminal).
2. Sidecar uses `AVAudioEngine` (mic) + `ScreenCaptureKit` (system audio), writes 5-second WAV chunks to a temp dir, prints chunk paths to stdout.
3. Rust reader thread parses each path, fans the chunk out to the chosen transcription provider (in `tokio::spawn`'d handles tracked in `RecordingSession.inflight`).
4. Each transcribed chunk is **filtered** (silence gate, hallucination heuristics, attribution-tail stripping), then `append_transcript`'d to SQLite and emitted to the frontend as a `transcript` event.
5. On stop: SIGTERM the sidecar → wait 3 s → SIGKILL fallback → drain inflight handles + reader handle → emit `Idle`.
6. On crash: stdout EOF detection resets the session and emits an error toast.
7. Summary is fired manually via `summarize_note`, which reads `note.body` (HTML → plain text) + `note.transcript`, resolves the preset's prompt, appends a language directive, and calls OpenAI.

## Tech stack

### Frontend (`src/`)

- **React 19** + **TypeScript** + **Vite 6**
- **Tauri 2** — `@tauri-apps/api` for `invoke` + event listeners; webview-based UI
- **React Router 7** — note routing (`/note/:id`), settings, home
- **Zustand** — `useNotesStore` (notes/folders) + `useRecordingStore` (status/errors/diagnostics); backend events bound once via `bindBackendListeners`
- **Tiptap v2** — body editor (StarterKit + Placeholder + Suggestion + BubbleMenu); plain `<textarea>` for transcript
- **react-markdown** + **remark-gfm** — summary rendering
- **Tailwind v4** — `@tailwindcss/vite` plugin; design tokens in `src/styles/globals.css`
- **lucide-react** — icon set (replaced original emoji)
- **Nothing-design aesthetic** — Space Grotesk + Space Mono, monochrome palette, system-aware dark/light. Custom utilities: `.nd-chip`, `.nd-action`, `.nd-label`, `.nd-bare`.

### Backend (`src-tauri/src/`)

- **Rust** + **Tauri 2** runtime
- **rusqlite** (`bundled` feature) — single SQLite DB at `~/Library/Application Support/no.humla.app/`. Idempotent ALTER TABLE migrations; index creation runs *after* migrations.
- **reqwest** with `rustls-tls` + `stream` — all HTTPS (OpenAI, Speechmatics, Hugging Face for model download)
- **tokio** — async runtime; `spawn_blocking` wraps the local Whisper inference
- **whisper-rs 0.13** with `metal` feature — bundles whisper.cpp via cmake, runs `large-v3-turbo-q5_0` on Apple Silicon GPUs
- **parking_lot** — mutex for session state
- **serde** / **serde_json** — IPC + provider payloads
- **chrono** — timestamps
- **uuid** — note + folder IDs
- **anyhow** — Rust-side error type; converted to `String` at the IPC boundary

### Module map

| File | Responsibility |
|---|---|
| `lib.rs` | `AppState`, command registration, plugin setup |
| `main.rs` | Tauri entry |
| `commands.rs` | All `#[tauri::command]` fns; recording lifecycle; transcribe fan-out; summary; folders; settings |
| `db.rs` | SQLite schema, migrations, CRUD for notes/folders/settings |
| `recording.rs` | `RecordingSession` (child, temp dir, inflight handles, reader handle) |
| `openai.rs` | Transcription + summary HTTP clients; default summary system prompt |
| `speechmatics.rs` | Batch SaaS client; region routing; `additional_vocab` |
| `local_whisper.rs` | On-device Whisper; `SharedContext`; download via HF; deletion |
| `presets.rs` | Backend mirror of frontend preset prompts; `{LANGUAGE}` substitution |
| `wav.rs` | Proper RIFF chunk walking; RMS for silence gate; mono-16k decoder |

### Sidecar (`audio-capture/`)

- **Swift Package** producing a single binary, built via `scripts/build-sidecar.sh`
- **AVFoundation** for mic, **ScreenCaptureKit** for system audio
- **Hidden from Dock** via `NSApplication.shared.setActivationPolicy(.prohibited)`
- Ad-hoc signed today; binary cached via SHA-256 stamp at `src-tauri/binaries/.audio-capture-<triple>.stamp` to avoid TCC churn on rebuilds (override with `FORCE_SIDECAR_REBUILD=1`)

## macOS specifics

- **Bundle id** `no.humla.app` — TCC keys on this. Pre-rename `com.notes-app.local` permissions don't transfer.
- **Entitlements** (`src-tauri/entitlements.plist`) — mic input, network client, screen capture usage description, no app-sandbox.
- **TCC pain point** — every rebuild that re-signs the sidecar invalidates the trusted-binary entry. Fix: the stamp-based cache in `build-sidecar.sh`, or graduate to notarised builds (see "Deferred: notarised distribution" below).
- **Tauri webview limitation** — `window.prompt` / `confirm` / `alert` are blocked by the Tauri webview to avoid main-thread deadlock. We use inline input UIs everywhere (folder creation in Sidebar + Note's FolderPicker).

## Local data layout

- **DB** — `~/Library/Application Support/no.humla.app/humla.db` (SQLite, WAL mode)
- **Settings** — `settings` table inside the same DB (API keys, language, provider, models, custom vocab, custom prompt)
- **Local Whisper model** — `~/Library/Application Support/no.humla.app/models/ggml-large-v3-turbo-q5_0.bin` (~600 MB, downloaded on demand)
- **Audio temp** — `tempfile::TempDir` per recording session; cleaned on stop

## Build & distribution

| Command | What it does |
|---|---|
| `pnpm dev` | Vite dev server only (frontend) |
| `pnpm tauri dev` | Tauri dev (assumes sidecar already built) |
| `./scripts/build-sidecar.sh` | Build + ad-hoc sign the Swift sidecar (skips if unchanged) |
| `pnpm icon` | Regenerate the macOS app icon from `src-tauri/icons/source.png` |
| `pnpm tauri build` | Production bundle (`.app` + `.dmg`) |
| `pnpm dmg` | Wrapper: builds sidecar, then `pnpm tauri build`; prints final DMG path |

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
