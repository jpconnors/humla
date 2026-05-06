# STT Adapter Phase 2 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add Deepgram and Groq as Humla STT providers and ship the supporting infrastructure (multi-provider Keychain handling, typed `ProviderConfig` write path from Settings, Word-type unification). All adapters slot into the Phase 1 trait — recording pipeline doesn't change.

**Architecture:** Generalize Phase 1's single-OpenAI-key cache to a `HashMap<&'static str, Option<String>>` keyed by `provider_id`. Add Tauri commands `provider_key_*` for per-provider Keychain access. Add `set_provider_config` so Settings UI writes typed JSON; `read_provider_config` updated to prefer the new key over legacy. Two new adapter files (`stt/deepgram.rs`, `stt/groq.rs`); shared OpenAI-compat code factored into `stt/openai_compat.rs`.

**Tech Stack:** Rust 1.85, Tauri 2, async-trait, serde, parking_lot, reqwest. Frontend: React 19 + Tiptap + Tailwind v4 (per existing pattern).

**Reference docs:** `docs/design/stt-adapter.md` (Phase 2 sections), `docs/superpowers/plans/2026-05-06-stt-adapter-phase1.md` (what shipped).

---

## Background for the implementer

Phase 1 (shipped as v0.21.0) introduced the `BatchSttAdapter` trait, OpenAI/Local adapters, and `read_provider_config` that rebuilds `ProviderConfig` from legacy settings keys on every call. The Settings UI still writes the legacy keys directly; the new path round-trips through `from_legacy_settings` for now.

This phase makes the abstraction pay off:

1. **Two new providers** — Deepgram (better speaker turns, native diarization, ~$0.43/h) and Groq (Whisper Large v3 Turbo at $0.04/h, OpenAI-compatible).
2. **Multi-provider Keychain** — Phase 1 cached one `Option<Option<String>>` slot for OpenAI. Generalize to per-provider; eliminates "second new keychain prompt every time the user picks Deepgram."
3. **`TranscribeCtx` field split** — Phase 1 stuffed user vocabulary AND rolling transcript trail into one `initial_prompt: Option<&str>`. Whisper-shaped providers can absorb both as one prompt; Deepgram cannot (its `keywords` query param is a per-token boost, not a continuation primer; sending transcript text would actively hurt). Splitting into `bias_terms: &[&str]` + `prior_context: Option<&str>` lets each adapter pick what fits.
4. **Typed `ProviderConfig` write path** — Settings UI sends one JSON blob instead of three string writes. Backend writes are atomic; switching provider is a single transaction.
5. **Settings UI migration** — surface for picking Deepgram/Groq, per-provider API key inputs.
6. **Word type unification** — rename `local_whisper::Word` → `stt::Word`, mechanical pass.

**Critical**: this is *not* a forced UI redesign. The existing `Transcription` tab stays as-is. We add provider radio entries and per-provider key inputs to the same component.

---

## File Structure

### New files
| File | Responsibility |
|---|---|
| `src-tauri/src/stt/openai_compat.rs` | Shared multipart-upload + JSON-parse for OpenAI-shaped HTTP transcribers (used by `OpenAiAdapter` and `GroqAdapter`) |
| `src-tauri/src/stt/deepgram.rs` | `DeepgramAdapter` — Deepgram-specific multipart + response parser |
| `src-tauri/src/stt/groq.rs` | `GroqAdapter` — thin wrapper around `openai_compat` with Groq base URL + label |
| `src-tauri/src/stt/keychain.rs` | Generalized provider-keyed Keychain helpers + multi-slot cache |

### Modified files
| File | Change |
|---|---|
| `src-tauri/src/lib.rs` | `api_key_cache` field type changes from `Option<Option<String>>` to `HashMap<&'static str, Option<String>>` |
| `src-tauri/src/commands.rs` | `read_openai_api_key` becomes a thin wrapper. Vocabulary string parsed into `&[&str]` for `bias_terms`. Add `set_provider_config` and `provider_key_*` commands. `read_provider_config` prefers typed key over legacy. |
| `src-tauri/src/stt/adapter.rs` | Replace `initial_prompt: Option<&str>` with `bias_terms: &[&str]` + `prior_context: Option<&str>` on `TranscribeCtx` |
| `src-tauri/src/stt/openai.rs` | Refactor to consume `bias_terms` + `prior_context`; delegate to `openai_compat` |
| `src-tauri/src/stt/local.rs` | Same shape change |
| `src-tauri/src/stt/config.rs` | Add `Deepgram(DeepgramConfig)` and `Groq(GroqConfig)` variants to `ProviderConfig` |
| `src-tauri/src/stt/mod.rs` | Re-export new adapters; update `build_adapter` registry |
| `src-tauri/src/local_whisper.rs` | Drop `Word` struct, re-export `stt::Word` for back-compat |
| `src/lib/ipc.ts` | Add new Tauri command bindings (`set_provider_config`, `provider_key_get`, etc.); deprecate `api_key_get` etc. with a comment |
| `src/pages/settings/tabs/Transcription.tsx` | Add Deepgram + Groq radio options, per-provider API key sections |
| `src/pages/settings/useSettings.ts` | Replace 3-key write fan-out with single `set_provider_config` call when migrating |
| `src/pages/settings/types.ts` | Add `ProviderConfig` type union mirroring backend |

---

## Task 1: Generalize Keychain helpers + multi-provider cache

**Files:**
- Create: `src-tauri/src/stt/keychain.rs`
- Modify: `src-tauri/src/lib.rs` (AppState field type)
- Modify: `src-tauri/src/commands.rs` (existing fns delegate)

- [ ] **Step 1: Define provider keychain registry**

Create `src-tauri/src/stt/keychain.rs`:

```rust
//! Per-provider Keychain access. Phase 1 hard-coded a single OpenAI slot;
//! Phase 2 generalises to one slot per cloud provider, keyed by the
//! `provider_id` that adapters return. Cache lives on AppState as a
//! HashMap so each provider's first read triggers exactly one Keychain
//! prompt, and subsequent reads are free.

use std::collections::HashMap;
use std::sync::Arc;
use parking_lot::Mutex;

use crate::stt::ProviderConfig;

pub const KEYCHAIN_SERVICE: &str = "no.humla.app";

/// Cache type to plug into AppState.
pub type ApiKeyCache = Arc<Mutex<HashMap<&'static str, Option<String>>>>;

pub fn new_cache() -> ApiKeyCache {
    Arc::new(Mutex::new(HashMap::new()))
}

/// Map a provider id to its Keychain account name. We keep this static
/// (no allocation) because adapter ids are `&'static str`.
pub fn keychain_account_for(provider_id: &str) -> Option<&'static str> {
    match provider_id {
        "openai" => Some("openai_api_key"),
        "deepgram" => Some("deepgram_api_key"),
        "groq" => Some("groq_api_key"),
        // "local" doesn't need a key.
        _ => None,
    }
}

/// True if the given config requires an API key. Used by transcribe_chunk
/// to decide whether to look one up.
pub fn requires_api_key(cfg: &ProviderConfig) -> bool {
    matches!(
        cfg,
        ProviderConfig::OpenAi(_) | ProviderConfig::Deepgram(_) | ProviderConfig::Groq(_)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_providers_have_keychain_accounts() {
        assert_eq!(keychain_account_for("openai"), Some("openai_api_key"));
        assert_eq!(keychain_account_for("deepgram"), Some("deepgram_api_key"));
        assert_eq!(keychain_account_for("groq"), Some("groq_api_key"));
        assert_eq!(keychain_account_for("local"), None);
        assert_eq!(keychain_account_for("nonsense"), None);
    }
}
```

- [ ] **Step 2: Re-export from `stt::mod`**

Edit `src-tauri/src/stt/mod.rs`:

```rust
//! STT provider abstraction. See docs/design/stt-adapter.md for rationale.

mod adapter;
mod auth;
mod config;
mod keychain;
mod local;
mod openai;

use std::path::PathBuf;

pub use adapter::{BatchSttAdapter, TranscribeCtx, TranscribeResult, Word};
pub use auth::Auth;
pub use config::{
    from_legacy_settings, LocalWhisperConfig, OpenAiConfig, ProviderConfig,
};
pub use keychain::{new_cache, requires_api_key, ApiKeyCache, KEYCHAIN_SERVICE};
pub use local::LocalWhisperAdapter;
pub use openai::OpenAiAdapter;

// ...rest of build_adapter unchanged for now
```

- [ ] **Step 3: Update `AppState` to use the new cache type**

Edit `src-tauri/src/lib.rs`:

```rust
pub struct AppState {
    pub db: Arc<Mutex<rusqlite::Connection>>,
    pub recording: Arc<Mutex<recording::RecordingSession>>,
    pub whisper: local_whisper::SharedContext,
    pub transcribe_gate: Arc<tokio::sync::Mutex<()>>,
    // Per-provider Keychain cache. Each provider's first read triggers
    // one OS-level Keychain prompt; subsequent reads return from this
    // map. Cleared/updated by `provider_key_set`.
    pub api_key_cache: stt::ApiKeyCache,
}
```

And the constructor in `run()`:

```rust
app.manage(AppState {
    db: Arc::new(Mutex::new(conn)),
    recording: Arc::new(Mutex::new(recording::RecordingSession::default())),
    whisper: local_whisper::new_shared(),
    transcribe_gate: Arc::new(tokio::sync::Mutex::new(())),
    api_key_cache: stt::new_cache(),
});
```

- [ ] **Step 4: Generalize `read_openai_api_key` → generic `read_provider_api_key`**

In `src-tauri/src/commands.rs`, replace the existing keychain helpers:

```rust
// (delete the existing KEYCHAIN_SERVICE and KEYCHAIN_ACCOUNT_OPENAI consts;
// they now live in stt::keychain)

/// Read the API key for the given provider from the macOS Keychain.
/// Returns Ok(None) if no key is stored or the provider doesn't take one
/// (e.g. local Whisper). Cached per-provider on AppState; first call per
/// provider per session triggers one Keychain prompt.
fn read_provider_api_key(
    state: &State<AppState>,
    provider_id: &'static str,
) -> Result<Option<String>, String> {
    if let Some(cached) = state.api_key_cache.lock().get(provider_id).cloned() {
        return Ok(cached);
    }
    let Some(account) = crate::stt::keychain_account_for(provider_id) else {
        return Ok(None);
    };
    let entry = keyring::Entry::new(crate::stt::KEYCHAIN_SERVICE, account)
        .map_err(|e| format!("keychain entry: {e}"))?;
    let result = match entry.get_password() {
        Ok(s) => {
            let t = s.trim().to_string();
            Ok(if t.is_empty() { None } else { Some(t) })
        }
        Err(keyring::Error::NoEntry) => {
            // Only OpenAI has a legacy migration path (the SQLite plaintext
            // row). New providers were never stored anywhere else.
            if provider_id == "openai" {
                migrate_legacy_api_key(state, &entry)
            } else {
                Ok(None)
            }
        }
        Err(e) => Err(format!("keychain read: {e}")),
    };
    if let Ok(value) = &result {
        state
            .api_key_cache
            .lock()
            .insert(provider_id, value.clone());
    }
    result
}

/// Write the API key for the given provider to the macOS Keychain.
/// Empty input deletes the entry. Updates the in-memory cache so reads
/// reflect the new value without a fresh prompt.
fn set_provider_api_key(
    state: &State<AppState>,
    provider_id: &'static str,
    key: &str,
) -> Result<(), String> {
    let trimmed = key.trim();
    let account = crate::stt::keychain_account_for(provider_id)
        .ok_or_else(|| format!("provider {provider_id} has no Keychain slot"))?;
    let entry = keyring::Entry::new(crate::stt::KEYCHAIN_SERVICE, account)
        .map_err(|e| format!("keychain entry: {e}"))?;
    if trimmed.is_empty() {
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => {}
            Err(e) => return Err(format!("keychain delete: {e}")),
        }
    } else {
        entry
            .set_password(trimmed)
            .map_err(|e| format!("keychain write: {e}"))?;
    }
    if provider_id == "openai" {
        // Mirror the Phase 1 behaviour: blank the legacy SQLite row.
        let conn = state.db.lock();
        let _ = db::set_setting(&conn, API_KEY, "");
    }
    state.api_key_cache.lock().insert(
        provider_id,
        if trimmed.is_empty() { None } else { Some(trimmed.to_string()) },
    );
    Ok(())
}

/// Phase-1 compatibility shim. Existing call sites can keep using this
/// name; new sites should call `read_provider_api_key` directly.
fn read_openai_api_key(state: &State<AppState>) -> Result<Option<String>, String> {
    read_provider_api_key(state, "openai")
}
```

- [ ] **Step 5: Verify compile + test**

Run:
```bash
cargo test --manifest-path src-tauri/Cargo.toml stt::keychain stt:: -- --nocapture
```

Expected: 1 keychain test + the Phase-1 stt:: tests all pass (15 total).

```bash
cargo build --manifest-path src-tauri/Cargo.toml --message-format=short
```

Expected: clean. Pre-existing `Auth` "unused" warnings will start clearing as Tasks 3+ wire up adapters.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/stt/keychain.rs src-tauri/src/stt/mod.rs src-tauri/src/lib.rs src-tauri/src/commands.rs
git commit -m "stt: generalize Keychain helpers to per-provider cache"
```

---

## Task 2: Add Deepgram + Groq variants to ProviderConfig

**Files:**
- Modify: `src-tauri/src/stt/config.rs`

- [ ] **Step 1: Extend the tagged union**

Replace the `ProviderConfig` enum and per-provider config structs in `src-tauri/src/stt/config.rs`:

```rust
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "provider")]
pub enum ProviderConfig {
    #[serde(rename = "openai")]
    OpenAi(OpenAiConfig),
    #[serde(rename = "local")]
    Local(LocalWhisperConfig),
    #[serde(rename = "deepgram")]
    Deepgram(DeepgramConfig),
    #[serde(rename = "groq")]
    Groq(GroqConfig),
}

impl ProviderConfig {
    pub fn provider_id(&self) -> &'static str {
        match self {
            ProviderConfig::OpenAi(_) => "openai",
            ProviderConfig::Local(_) => "local",
            ProviderConfig::Deepgram(_) => "deepgram",
            ProviderConfig::Groq(_) => "groq",
        }
    }

    pub fn model(&self) -> &str {
        match self {
            ProviderConfig::OpenAi(c) => &c.model,
            ProviderConfig::Local(c) => &c.model_id,
            ProviderConfig::Deepgram(c) => &c.model,
            ProviderConfig::Groq(c) => &c.model,
        }
    }

    pub fn base_url(&self) -> Option<&str> {
        match self {
            ProviderConfig::OpenAi(c) => c.base_url.as_deref(),
            ProviderConfig::Local(_) => None,
            ProviderConfig::Deepgram(c) => c.base_url.as_deref(),
            ProviderConfig::Groq(_) => None, // Groq has a fixed URL
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeepgramConfig {
    pub model: String,           // e.g. "nova-3", "nova-2", "base"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct GroqConfig {
    pub model: String,           // e.g. "whisper-large-v3-turbo"
}
```

- [ ] **Step 2: Add round-trip tests for the new variants**

Append to the existing `mod tests` block in `config.rs`:

```rust
    #[test]
    fn deepgram_round_trips_through_json() {
        let cfg = ProviderConfig::Deepgram(DeepgramConfig {
            model: "nova-3".to_string(),
            base_url: None,
        });
        let json = serde_json::to_string(&cfg).unwrap();
        let back: ProviderConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(cfg, back);
        assert!(json.contains(r#""provider":"deepgram""#));
        assert_eq!(cfg.model(), "nova-3");
    }

    #[test]
    fn groq_round_trips_through_json() {
        let cfg = ProviderConfig::Groq(GroqConfig {
            model: "whisper-large-v3-turbo".to_string(),
        });
        let json = serde_json::to_string(&cfg).unwrap();
        let back: ProviderConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(cfg, back);
        assert!(json.contains(r#""provider":"groq""#));
    }
```

- [ ] **Step 3: Re-export from `stt::mod`**

Edit `src-tauri/src/stt/mod.rs`:

```rust
pub use config::{
    from_legacy_settings, DeepgramConfig, GroqConfig, LocalWhisperConfig,
    OpenAiConfig, ProviderConfig,
};
```

- [ ] **Step 4: Run tests**

```bash
cargo test --manifest-path src-tauri/Cargo.toml stt::config -- --nocapture
```

Expected: 8 tests pass (the 6 from Phase 1 + 2 new).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/stt/config.rs src-tauri/src/stt/mod.rs
git commit -m "stt: add Deepgram + Groq variants to ProviderConfig"
```

---

## Task 3: Split `TranscribeCtx::initial_prompt` into `bias_terms` + `prior_context`

**Why:** Whisper's `initial_prompt` does double duty — it's both a continuation primer (last 150 transcribed words) and a vocabulary biaser (proper nouns the user added). Deepgram's `keywords` query param only handles the second job; sending transcript text as "keywords" would actively harm decoding by biasing toward whatever happened to be in the previous chunk. Splitting the field lets each adapter use what fits its provider model.

**Files:**
- Modify: `src-tauri/src/stt/adapter.rs` (rename + split the field)
- Modify: `src-tauri/src/stt/openai.rs` (consume both, build Whisper-style prompt)
- Modify: `src-tauri/src/stt/local.rs` (same)
- Modify: `src-tauri/src/commands.rs` (caller — split vocabulary string into `&[&str]`)

- [ ] **Step 1: Update `TranscribeCtx`**

In `src-tauri/src/stt/adapter.rs`, replace the existing struct:

```rust
pub struct TranscribeCtx<'a> {
    pub model: &'a str,
    pub language: &'a str,
    /// User-supplied vocabulary (proper nouns, tech terms). Maps to
    /// Whisper's `initial_prompt` for OpenAI/Local/Groq, and to
    /// Deepgram's `keywords` query param.
    pub bias_terms: &'a [&'a str],
    /// Last ~150 transcribed words from this source's stream. Used by
    /// Whisper-shaped adapters as the trailing portion of `initial_prompt`
    /// to keep cross-chunk continuity. Ignored by Deepgram (no equivalent;
    /// Deepgram's keyword bias would actively hurt if fed transcript
    /// text).
    pub prior_context: Option<&'a str>,
    pub api_key: Option<&'a str>,
    pub base_url: Option<&'a str>,
}
```

- [ ] **Step 2: Update `OpenAiAdapter::transcribe` to assemble its own Whisper prompt**

Edit `src-tauri/src/stt/openai.rs`:

```rust
    async fn transcribe(
        &self,
        ctx: TranscribeCtx<'_>,
        audio: &Path,
    ) -> Result<TranscribeResult> {
        let api_key = ctx
            .api_key
            .ok_or_else(|| anyhow::anyhow!("OpenAI adapter requires api_key"))?;
        let prompt = build_whisper_prompt(ctx.bias_terms, ctx.prior_context);
        let (text, words) = openai::transcribe_file(
            api_key,
            ctx.model,
            Some(ctx.language),
            prompt.as_deref(),
            audio,
        )
        .await?;
        let words = words
            .into_iter()
            .map(|w| Word { text: w.text, start_ms: w.start_ms, end_ms: w.end_ms })
            .collect();
        Ok(TranscribeResult { text, words })
    }
}

/// Glue bias terms + trailing transcript context into Whisper's
/// `initial_prompt` slot. Vocabulary terms come first (Whisper biases
/// toward early prompt tokens), then trail context. Returns None when
/// neither is present so the API call omits the field.
fn build_whisper_prompt(
    bias_terms: &[&str],
    prior_context: Option<&str>,
) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    if !bias_terms.is_empty() {
        parts.push(bias_terms.join(", "));
    }
    if let Some(ctx) = prior_context {
        let trimmed = ctx.trim();
        if !trimmed.is_empty() {
            parts.push(trimmed.to_string());
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(". "))
    }
}
```

(Move this `build_whisper_prompt` to a shared module if you'd rather; one-line `pub fn` import from `local.rs`. Phase-2 implementation note: it's small enough to duplicate at the start; refactor later if a third adapter wants it.)

- [ ] **Step 3: Update `LocalWhisperAdapter::transcribe` similarly**

```rust
    async fn transcribe(
        &self,
        ctx: TranscribeCtx<'_>,
        audio: &Path,
    ) -> Result<TranscribeResult> {
        let prompt = crate::stt::openai::build_whisper_prompt(ctx.bias_terms, ctx.prior_context);
        let (text, words) = local_whisper::transcribe_file_with_words(
            self.shared.clone(),
            self.model_path.clone(),
            self.use_gpu,
            ctx.language,
            prompt.as_deref(),
            self.preset,
            audio,
        )
        .await?;
        let words = words
            .into_iter()
            .map(|w| Word { text: w.text, start_ms: w.start_ms, end_ms: w.end_ms })
            .collect();
        Ok(TranscribeResult { text, words })
    }
```

(Make `build_whisper_prompt` `pub(crate)` in `openai.rs` so `local.rs` can import it. Or duplicate; a 12-line function isn't worth a shared module.)

- [ ] **Step 4: Update `transcribe_chunk` callsite**

In `src-tauri/src/commands.rs`, the existing `let prompt = build_initial_prompt(&vocabulary, trail_snapshot);` line and the subsequent `ctx` construction become:

```rust
    // Vocabulary is stored as a newline-or-comma-separated string. Split
    // into individual terms for the new bias_terms field. Trim, drop
    // empties, drop short tokens (< 3 chars create false positives in
    // every provider's keyword biaser).
    let vocab_terms: Vec<&str> = vocabulary
        .split(|c: char| c == '\n' || c == ',')
        .map(str::trim)
        .filter(|s| s.len() >= 3)
        .collect();

    // build_adapter & local_deps unchanged — same as before
    // ...

    let ctx = crate::stt::TranscribeCtx {
        model: provider_cfg.model(),
        language: &language,
        bias_terms: &vocab_terms,
        prior_context: trail_snapshot.as_deref(),
        api_key: api_key.as_deref(),
        base_url: provider_cfg.base_url(),
    };
```

The old `build_initial_prompt` helper in commands.rs becomes unused — delete it.

- [ ] **Step 5: Compile + run all stt:: tests**

```bash
cargo test --manifest-path src-tauri/Cargo.toml stt:: -- --nocapture
```

Expected: existing 14 Phase-1 tests still pass (the trait surface changed but the test cases use `BatchSttAdapter::provider_id` etc., not the ctx fields).

```bash
cargo build --manifest-path src-tauri/Cargo.toml --message-format=short
```

Expected: clean.

- [ ] **Step 6: Manual smoke test**

```bash
pnpm tauri dev
```

Record ~15s with vocabulary set to "Humla, Tauri" and verify the transcript still respects vocab + cross-chunk continuity. Output should be bit-identical to v0.21.0 because the OpenAI/Local prompt assembly produces the same string the old `build_initial_prompt` did.

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/stt/adapter.rs src-tauri/src/stt/openai.rs src-tauri/src/stt/local.rs src-tauri/src/commands.rs
git commit -m "stt: split TranscribeCtx::initial_prompt into bias_terms + prior_context"
```

---

## Task 4: Factor shared OpenAI-compat HTTP transcriber

**Files:**
- Create: `src-tauri/src/stt/openai_compat.rs`
- Modify: `src-tauri/src/stt/openai.rs` (delegate to it)

- [ ] **Step 1: Build the shared base**

Create `src-tauri/src/stt/openai_compat.rs`:

```rust
//! Shared HTTP transcriber for OpenAI-compatible STT endpoints.
//! OpenAI itself, Groq, and self-hosted Whisper.cpp servers all expose
//! `POST /v1/audio/transcriptions` with the same multipart shape. The
//! per-provider adapter (OpenAiAdapter, GroqAdapter) configures base URL
//! + model + word-timestamp policy and calls into here.

use anyhow::{anyhow, Result};
use serde::Deserialize;
use std::path::Path;

use crate::stt::adapter::Word;

#[derive(Deserialize)]
struct PlainResponse {
    text: String,
}

#[derive(Deserialize)]
struct VerboseResponse {
    text: String,
    #[serde(default)]
    words: Vec<VerboseWord>,
}

#[derive(Deserialize)]
struct VerboseWord {
    word: String,
    start: f64,
    end: f64,
}

/// One transcription against a `/v1/audio/transcriptions` endpoint.
/// `verbose` true requests `verbose_json` + `timestamp_granularities[]=word`
/// (only valid for OpenAI's `whisper-1`; gpt-4o-transcribe family rejects
/// it; Groq's whisper-large-v3-turbo accepts both shapes).
///
/// `bias_terms` and `prior_context` are merged into Whisper's
/// `initial_prompt` slot. Skipped entirely when `skip_prompt_for_model`
/// matches `model` (gpt-4o-transcribe-diarize rejects the field).
pub async fn transcribe(
    base_url: &str,
    api_key: &str,
    model: &str,
    language: Option<&str>,
    bias_terms: &[&str],
    prior_context: Option<&str>,
    audio_path: &Path,
    verbose: bool,
    skip_prompt_for_model: Option<&str>,
) -> Result<(String, Vec<Word>)> {
    let bytes = tokio::fs::read(audio_path).await?;
    let file_name = audio_path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("chunk.wav")
        .to_string();
    let part = reqwest::multipart::Part::bytes(bytes)
        .file_name(file_name)
        .mime_str("audio/wav")?;

    let response_format = if verbose { "verbose_json" } else { "json" };
    let mut form = reqwest::multipart::Form::new()
        .part("file", part)
        .text("model", model.to_string())
        .text("response_format", response_format.to_string())
        .text("temperature", "0".to_string());
    if verbose {
        form = form.text("timestamp_granularities[]", "word".to_string());
    }
    if let Some(l) = language {
        if l != "auto" {
            form = form.text("language", l.to_string());
        }
    }
    if skip_prompt_for_model != Some(model) {
        if let Some(prompt) = build_whisper_prompt(bias_terms, prior_context) {
            form = form.text("prompt", prompt);
        }
    }

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()?;
    let r = client
        .post(format!("{base_url}/audio/transcriptions"))
        .bearer_auth(api_key)
        .multipart(form)
        .send()
        .await?;

    if !r.status().is_success() {
        let s = r.status();
        let body = r.text().await.unwrap_or_default();
        return Err(anyhow!("{base_url} {s}: {body}"));
    }

    if verbose {
        let body: VerboseResponse = r.json().await?;
        let words = body
            .words
            .into_iter()
            .filter_map(|w| {
                let text = w.word.trim().to_string();
                if text.is_empty() {
                    return None;
                }
                let start_ms = (w.start.max(0.0) * 1000.0).round() as u64;
                let end_ms = (w.end.max(0.0) * 1000.0).round() as u64;
                Some(Word {
                    text,
                    start_ms,
                    end_ms: end_ms.max(start_ms),
                })
            })
            .collect();
        Ok((body.text, words))
    } else {
        let body: PlainResponse = r.json().await?;
        Ok((body.text, Vec::new()))
    }
}
```

- [ ] **Step 2: Move `build_whisper_prompt` here from Task 3**

In Task 3 we put `build_whisper_prompt` in `openai.rs` as `pub(crate)`. Now that we have a shared module, move it to `openai_compat.rs` and update the imports in `openai.rs` and `local.rs` to call `crate::stt::openai_compat::build_whisper_prompt`.

```rust
// Moved from src-tauri/src/stt/openai.rs to here
pub fn build_whisper_prompt(
    bias_terms: &[&str],
    prior_context: Option<&str>,
) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    if !bias_terms.is_empty() {
        parts.push(bias_terms.join(", "));
    }
    if let Some(ctx) = prior_context {
        let trimmed = ctx.trim();
        if !trimmed.is_empty() {
            parts.push(trimmed.to_string());
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(". "))
    }
}
```

- [ ] **Step 3: Refactor `OpenAiAdapter` to use the shared transcriber**

Replace the `transcribe` method in `src-tauri/src/stt/openai.rs`:

```rust
    async fn transcribe(
        &self,
        ctx: TranscribeCtx<'_>,
        audio: &Path,
    ) -> Result<TranscribeResult> {
        let api_key = ctx
            .api_key
            .ok_or_else(|| anyhow::anyhow!("OpenAI adapter requires api_key"))?;
        let base_url = ctx.base_url.unwrap_or("https://api.openai.com/v1");
        let verbose = self.supports_word_timestamps(ctx.model);
        let (text, words) = crate::stt::openai_compat::transcribe(
            base_url,
            api_key,
            ctx.model,
            Some(ctx.language),
            ctx.bias_terms,
            ctx.prior_context,
            audio,
            verbose,
            // OpenAI docs: gpt-4o-transcribe-diarize rejects `prompt`.
            Some("gpt-4o-transcribe-diarize"),
        )
        .await?;
        Ok(TranscribeResult { text, words })
    }
```

(Add `mod openai_compat;` to `src-tauri/src/stt/mod.rs`.)

- [ ] **Step 4: Update `LocalWhisperAdapter` to use the shared `build_whisper_prompt`**

In `src-tauri/src/stt/local.rs`, change the import line at the top of the `transcribe` method from `crate::stt::openai::build_whisper_prompt` (set up in Task 3) to `crate::stt::openai_compat::build_whisper_prompt`. Same body, different home.

- [ ] **Step 5: Verify compile + run existing OpenAI smoke**

```bash
cargo test --manifest-path src-tauri/Cargo.toml stt::openai stt::local -- --nocapture
```

Expected: both metadata tests still pass.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/stt/openai_compat.rs src-tauri/src/stt/openai.rs src-tauri/src/stt/local.rs src-tauri/src/stt/mod.rs
git commit -m "stt: factor OpenAI-compat transcriber into shared module"
```

---

## Task 5: Add `GroqAdapter`

**Files:**
- Create: `src-tauri/src/stt/groq.rs`
- Modify: `src-tauri/src/stt/mod.rs`

- [ ] **Step 1: Implement the adapter**

Create `src-tauri/src/stt/groq.rs`:

```rust
//! Groq batch STT adapter. Groq hosts whisper-large-v3-turbo at an
//! OpenAI-compatible endpoint; auth + payload + response shape are
//! identical, only the base URL and rate-limit profile differ.

use anyhow::Result;
use async_trait::async_trait;
use std::path::Path;

use crate::stt::adapter::{BatchSttAdapter, TranscribeCtx, TranscribeResult, Word};

const GROQ_BASE: &str = "https://api.groq.com/openai/v1";

#[derive(Default)]
pub struct GroqAdapter;

impl GroqAdapter {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl BatchSttAdapter for GroqAdapter {
    fn provider_id(&self) -> &'static str {
        "groq"
    }

    fn label(&self) -> &'static str {
        "Groq"
    }

    fn supports_language(&self, _lang: &str) -> bool {
        true
    }

    fn supports_word_timestamps(&self, _model: &str) -> bool {
        // Groq's whisper-large-v3-turbo accepts verbose_json and returns
        // word-level timestamps. Model-agnostic in practice; if Groq adds
        // a non-Whisper STT model later, narrow this.
        true
    }

    async fn transcribe(
        &self,
        ctx: TranscribeCtx<'_>,
        audio: &Path,
    ) -> Result<TranscribeResult> {
        let api_key = ctx
            .api_key
            .ok_or_else(|| anyhow::anyhow!("Groq adapter requires api_key"))?;
        let base_url = ctx.base_url.unwrap_or(GROQ_BASE);
        let (text, words) = crate::stt::openai_compat::transcribe(
            base_url,
            api_key,
            ctx.model,
            Some(ctx.language),
            ctx.bias_terms,
            ctx.prior_context,
            audio,
            true, // always request verbose_json — Groq accepts it
            None,
        )
        .await?;
        Ok(TranscribeResult { text, words })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_is_correct() {
        let a = GroqAdapter::new();
        assert_eq!(a.provider_id(), "groq");
        assert_eq!(a.label(), "Groq");
        assert!(a.supports_word_timestamps("whisper-large-v3-turbo"));
    }
}
```

- [ ] **Step 2: Wire into the registry**

Edit `src-tauri/src/stt/mod.rs`:

```rust
mod adapter;
mod auth;
mod config;
mod groq;
mod keychain;
mod local;
mod openai;
mod openai_compat;

// ...existing pub use statements...
pub use groq::GroqAdapter;

// In build_adapter, add the Groq arm:
pub fn build_adapter(
    cfg: &ProviderConfig,
    local_deps: Option<LocalDeps>,
) -> Box<dyn BatchSttAdapter> {
    match cfg {
        ProviderConfig::OpenAi(_) => Box::new(OpenAiAdapter::new()),
        ProviderConfig::Local(local_cfg) => {
            let deps = local_deps.expect("LocalDeps required for ProviderConfig::Local");
            Box::new(LocalWhisperAdapter::new(
                deps.shared,
                deps.model_path,
                deps.use_gpu,
                local_whisper::Preset::from_setting(&local_cfg.preset),
            ))
        }
        ProviderConfig::Deepgram(_) => Box::new(crate::stt::DeepgramAdapter::new()),
        ProviderConfig::Groq(_) => Box::new(GroqAdapter::new()),
    }
}
```

- [ ] **Step 3: Test (Deepgram registry arm will fail compile — that's Task 6)**

This task expects a compile error referencing `DeepgramAdapter` because we add it in Task 6. The Groq parts are complete. To verify Groq compiles in isolation, temporarily comment the Deepgram arm:

```rust
// ProviderConfig::Deepgram(_) => Box::new(crate::stt::DeepgramAdapter::new()),
ProviderConfig::Deepgram(_) => unreachable!("Deepgram lands in Task 6"),
```

Then:

```bash
cargo test --manifest-path src-tauri/Cargo.toml stt::groq -- --nocapture
```

Expected: 1 test passes.

- [ ] **Step 4: Commit (with the Deepgram placeholder)**

```bash
git add src-tauri/src/stt/groq.rs src-tauri/src/stt/mod.rs
git commit -m "stt: add Groq adapter (whisper-large-v3-turbo via OpenAI-compat)"
```

---

## Task 6: Add `DeepgramAdapter`

**Files:**
- Create: `src-tauri/src/stt/deepgram.rs`
- Modify: `src-tauri/src/stt/mod.rs` (un-stub, register)

- [ ] **Step 1: Implement the adapter**

Create `src-tauri/src/stt/deepgram.rs`:

```rust
//! Deepgram batch STT adapter. Different from OpenAI-compat in three
//! ways: auth uses `Token` not `Bearer`, response is nested under
//! `results.channels[0].alternatives[0]`, and Deepgram supports per-
//! request `keywords` for vocab biasing instead of `prompt`.

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use serde::Deserialize;
use std::path::Path;

use crate::stt::adapter::{BatchSttAdapter, TranscribeCtx, TranscribeResult, Word};

const DEEPGRAM_BASE: &str = "https://api.deepgram.com/v1/listen";

#[derive(Deserialize)]
struct ListenResponse {
    results: ResultsBlock,
}

#[derive(Deserialize)]
struct ResultsBlock {
    channels: Vec<Channel>,
}

#[derive(Deserialize)]
struct Channel {
    alternatives: Vec<Alternative>,
}

#[derive(Deserialize)]
struct Alternative {
    transcript: String,
    #[serde(default)]
    words: Vec<DGWord>,
}

#[derive(Deserialize)]
struct DGWord {
    word: String,
    start: f64,
    end: f64,
}

#[derive(Default)]
pub struct DeepgramAdapter;

impl DeepgramAdapter {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl BatchSttAdapter for DeepgramAdapter {
    fn provider_id(&self) -> &'static str {
        "deepgram"
    }

    fn label(&self) -> &'static str {
        "Deepgram"
    }

    fn supports_language(&self, _lang: &str) -> bool {
        true
    }

    fn supports_word_timestamps(&self, _model: &str) -> bool {
        // Deepgram returns word-level timestamps for every model.
        true
    }

    async fn transcribe(
        &self,
        ctx: TranscribeCtx<'_>,
        audio: &Path,
    ) -> Result<TranscribeResult> {
        let api_key = ctx
            .api_key
            .ok_or_else(|| anyhow!("Deepgram adapter requires api_key"))?;
        let base_url = ctx.base_url.unwrap_or(DEEPGRAM_BASE);
        let bytes = tokio::fs::read(audio).await?;

        let mut req = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()?
            .post(base_url)
            .header("Authorization", format!("Token {api_key}"))
            .header("Content-Type", "audio/wav")
            .query(&[
                ("model", ctx.model),
                ("smart_format", "true"),
                ("punctuate", "true"),
            ]);
        if ctx.language != "auto" {
            req = req.query(&[("language", ctx.language)]);
        }
        // Deepgram's `keywords` is a per-token boost, not a continuation
        // primer. We feed it ONLY the user's vocabulary (proper nouns,
        // tech terms) and explicitly drop `prior_context`: pushing
        // transcript text in here would bias decoding toward whatever
        // was said before, which is exactly the wrong signal.
        //
        // Format: `keywords=Term:1.5` — intensifier 1.5 is a measured
        // boost (higher causes phonetic over-recognition; lower is
        // imperceptible). Deepgram caps at 100 entries.
        let mut keyword_count = 0usize;
        for term in ctx.bias_terms.iter() {
            if keyword_count >= 100 {
                break;
            }
            let cleaned = term.trim_matches(|c: char| !c.is_alphanumeric());
            if cleaned.len() >= 3 {
                req = req.query(&[("keywords", &format!("{cleaned}:1.5"))]);
                keyword_count += 1;
            }
        }
        req = req.body(bytes);

        let r = req.send().await?;
        if !r.status().is_success() {
            let s = r.status();
            let body = r.text().await.unwrap_or_default();
            return Err(anyhow!("Deepgram {s}: {body}"));
        }
        let body: ListenResponse = r.json().await?;
        let alt = body
            .results
            .channels
            .into_iter()
            .next()
            .and_then(|c| c.alternatives.into_iter().next())
            .ok_or_else(|| anyhow!("Deepgram returned no alternatives"))?;

        let words = alt
            .words
            .into_iter()
            .filter_map(|w| {
                let text = w.word.trim().to_string();
                if text.is_empty() {
                    return None;
                }
                let start_ms = (w.start.max(0.0) * 1000.0).round() as u64;
                let end_ms = (w.end.max(0.0) * 1000.0).round() as u64;
                Some(Word {
                    text,
                    start_ms,
                    end_ms: end_ms.max(start_ms),
                })
            })
            .collect();

        Ok(TranscribeResult {
            text: alt.transcript,
            words,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_is_correct() {
        let a = DeepgramAdapter::new();
        assert_eq!(a.provider_id(), "deepgram");
        assert_eq!(a.label(), "Deepgram");
        assert!(a.supports_word_timestamps("nova-3"));
    }

    #[test]
    fn parses_canonical_listen_response() {
        let json = r#"{
          "results": {
            "channels": [{
              "alternatives": [{
                "transcript": "hello world",
                "words": [
                  {"word": "hello", "start": 0.5, "end": 0.9},
                  {"word": "world", "start": 1.0, "end": 1.4}
                ]
              }]
            }]
          }
        }"#;
        let parsed: ListenResponse = serde_json::from_str(json).unwrap();
        let alt = &parsed.results.channels[0].alternatives[0];
        assert_eq!(alt.transcript, "hello world");
        assert_eq!(alt.words.len(), 2);
        assert_eq!(alt.words[0].word, "hello");
        assert!((alt.words[0].start - 0.5).abs() < 1e-6);
    }
}
```

- [ ] **Step 2: Un-stub the registry arm**

Edit `src-tauri/src/stt/mod.rs`:

```rust
mod adapter;
mod auth;
mod config;
mod deepgram;
mod groq;
mod keychain;
mod local;
mod openai;
mod openai_compat;

// ...
pub use deepgram::DeepgramAdapter;
pub use groq::GroqAdapter;
// ...

pub fn build_adapter(
    cfg: &ProviderConfig,
    local_deps: Option<LocalDeps>,
) -> Box<dyn BatchSttAdapter> {
    match cfg {
        ProviderConfig::OpenAi(_) => Box::new(OpenAiAdapter::new()),
        ProviderConfig::Local(local_cfg) => {
            let deps = local_deps.expect("LocalDeps required for ProviderConfig::Local");
            Box::new(LocalWhisperAdapter::new(
                deps.shared,
                deps.model_path,
                deps.use_gpu,
                local_whisper::Preset::from_setting(&local_cfg.preset),
            ))
        }
        ProviderConfig::Deepgram(_) => Box::new(DeepgramAdapter::new()),
        ProviderConfig::Groq(_) => Box::new(GroqAdapter::new()),
    }
}
```

- [ ] **Step 3: Test**

```bash
cargo test --manifest-path src-tauri/Cargo.toml stt::deepgram -- --nocapture
```

Expected: 2 tests pass (metadata + canonical-response parser).

```bash
cargo test --manifest-path src-tauri/Cargo.toml stt:: -- --nocapture
```

Expected: full stt:: suite passes (≈18 tests).

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/stt/deepgram.rs src-tauri/src/stt/mod.rs
git commit -m "stt: add Deepgram adapter with word timestamps + keyword biasing"
```

---

## Task 7: Add `set_provider_config` Tauri command + cached read path

**Files:**
- Modify: `src-tauri/src/commands.rs`
- Modify: `src-tauri/src/lib.rs` (register the new command)

- [ ] **Step 1: Add the write command + provider key commands**

Insert near the existing `api_key_set` command (around line 1041 of commands.rs) — keep the legacy `api_key_*` names as compatibility aliases:

```rust
#[tauri::command]
pub fn set_provider_config(
    state: State<AppState>,
    config: serde_json::Value,
) -> Result<(), String> {
    // Validate by deserialising; refuse to write garbage.
    let cfg: crate::stt::ProviderConfig = serde_json::from_value(config)
        .map_err(|e| format!("invalid provider config: {e}"))?;
    let json = serde_json::to_string(&cfg).map_err(err)?;
    let conn = state.db.lock();
    db::set_setting(&conn, "transcribe_config", &json).map_err(err)?;
    Ok(())
}

#[tauri::command]
pub fn provider_key_get(
    state: State<AppState>,
    provider: String,
) -> Result<Option<String>, String> {
    // Frontend pattern: only "stored" / null. We don't return raw keys.
    let id = canonical_provider_id(&provider).ok_or_else(|| {
        format!("unknown provider: {provider}")
    })?;
    Ok(read_provider_api_key(&state, id)?.map(|_| "stored".to_string()))
}

#[tauri::command]
pub fn provider_key_set(
    state: State<AppState>,
    provider: String,
    key: String,
) -> Result<(), String> {
    let id = canonical_provider_id(&provider)
        .ok_or_else(|| format!("unknown provider: {provider}"))?;
    set_provider_api_key(&state, id, &key)
}

#[tauri::command]
pub async fn provider_key_test(
    state: State<'_, AppState>,
    provider: String,
) -> Result<TestResult, String> {
    let id = canonical_provider_id(&provider)
        .ok_or_else(|| format!("unknown provider: {provider}"))?;
    let key = read_provider_api_key(&state, id)?
        .ok_or_else(|| "No API key stored".to_string())?;

    // Per-provider ping URL.
    let url = match id {
        "openai" => format!("{}/models", openai::BASE),
        "deepgram" => "https://api.deepgram.com/v1/projects".to_string(),
        "groq" => "https://api.groq.com/openai/v1/models".to_string(),
        _ => return Err(format!("provider {id} doesn't support test")),
    };
    let auth_header = match id {
        "deepgram" => format!("Token {key}"),
        _ => format!("Bearer {key}"),
    };
    let r = openai::client()
        .get(url)
        .header("Authorization", auth_header)
        .send()
        .await
        .map_err(|e| format!("network: {e}"))?;
    Ok(TestResult {
        ok: r.status().is_success(),
        status: r.status().as_u16(),
        error: if r.status().is_success() {
            None
        } else {
            Some(r.text().await.unwrap_or_default())
        },
    })
}

/// Map a frontend-supplied provider string to a static id we trust.
/// Rejecting unknown ids prevents the frontend from probing arbitrary
/// Keychain accounts via the Tauri bridge.
fn canonical_provider_id(s: &str) -> Option<&'static str> {
    match s {
        "openai" => Some("openai"),
        "deepgram" => Some("deepgram"),
        "groq" => Some("groq"),
        "local" => Some("local"),
        _ => None,
    }
}
```

- [ ] **Step 2: Update `read_provider_config` to prefer the new key**

Edit the helper to read `transcribe_config` first, fall back to legacy:

```rust
fn read_provider_config(state: &State<AppState>) -> anyhow::Result<crate::stt::ProviderConfig> {
    let conn = state.db.lock();
    if let Some(json) = db::get_setting(&conn, "transcribe_config")? {
        if let Ok(cfg) = serde_json::from_str::<crate::stt::ProviderConfig>(&json) {
            return Ok(cfg);
        }
        // Corrupted JSON — fall through to legacy reconstruction so the
        // user isn't locked out of their app over a malformed cache.
    }
    let provider = db::get_setting(&conn, "transcribe_provider")?;
    let model = db::get_setting(&conn, "transcribe_model")?;
    let whisper_model = db::get_setting(&conn, "local_whisper_model")?;
    let whisper_preset = db::get_setting(&conn, "whisper_preset")?;
    let whisper_use_gpu = db::get_setting(&conn, "local_whisper_use_gpu")?
        .and_then(|v| match v.as_str() {
            "true" => Some(true),
            "false" => Some(false),
            _ => None,
        });
    Ok(crate::stt::from_legacy_settings(
        provider.as_deref(),
        model.as_deref(),
        whisper_model.as_deref(),
        whisper_preset.as_deref(),
        whisper_use_gpu,
    ))
}
```

- [ ] **Step 3: Register the new commands in `lib.rs`**

Find the `tauri::generate_handler!` invocation in `src-tauri/src/lib.rs` and add the three new commands alongside the existing `api_key_get` etc.:

```rust
.invoke_handler(tauri::generate_handler![
    // ...existing handlers...
    commands::api_key_get,
    commands::api_key_set,
    commands::api_key_test,
    commands::set_provider_config,
    commands::provider_key_get,
    commands::provider_key_set,
    commands::provider_key_test,
    // ...
])
```

- [ ] **Step 4: Compile + test**

```bash
cargo build --manifest-path src-tauri/Cargo.toml --message-format=short
```

Expected: clean build (warnings about unused trait surface should be drying up at this point).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/commands.rs src-tauri/src/lib.rs
git commit -m "commands: add set_provider_config + provider_key_* Tauri commands"
```

---

## Task 8: Migrate Settings UI to write `ProviderConfig`

**Files:**
- Modify: `src/lib/ipc.ts` (add new bindings)
- Modify: `src/pages/settings/types.ts` (add `ProviderConfig` type)
- Modify: `src/pages/settings/tabs/Transcription.tsx` (provider radios + key inputs)
- Modify: `src/pages/settings/useSettings.ts` (route writes through `set_provider_config`)

This is the biggest single task in this phase — the others are mostly Rust additions, this one rewires the frontend. Plan ~3-4 hours.

- [ ] **Step 1: Add the new IPC bindings**

In `src/lib/ipc.ts`, add the typed surface for the new commands. Mark the legacy `api_key_*` ones as `// deprecated` in a comment. Approximate addition:

```ts
export type ProviderConfig =
  | { provider: "openai"; model: string; base_url?: string }
  | { provider: "local"; model_id: string; preset: "fast" | "balanced" | "quality"; use_gpu: boolean }
  | { provider: "deepgram"; model: string; base_url?: string }
  | { provider: "groq"; model: string };

export const settings = {
  // ...existing...
  setProviderConfig: (cfg: ProviderConfig) =>
    invoke<void>("set_provider_config", { config: cfg }),
  getProviderKey: (provider: string) =>
    invoke<string | null>("provider_key_get", { provider }),
  setProviderKey: (provider: string, key: string) =>
    invoke<void>("provider_key_set", { provider, key }),
  testProviderKey: (provider: string) =>
    invoke<{ ok: boolean; status: number; error: string | null }>(
      "provider_key_test",
      { provider }
    ),
  // deprecated: use setProviderKey/getProviderKey/testProviderKey instead
  getApiKey: () => invoke<string | null>("api_key_get"),
  setApiKey: (key: string) => invoke<void>("api_key_set", { key }),
  testApiKey: () => invoke<{ ok: boolean; status: number; error: string | null }>("api_key_test"),
};
```

- [ ] **Step 2: Surface Deepgram + Groq in the Transcription tab**

Edit `src/pages/settings/tabs/Transcription.tsx`. The current file has a provider radio group + per-provider model selectors. Add Deepgram and Groq as additional provider options:

```tsx
type Provider = "openai" | "local" | "deepgram" | "groq";

// In the provider radio block (look for "transcribe_provider" — line 51):
<RadioGroup
  value={provider}
  options={[
    { value: "openai", label: "OpenAI (cloud)" },
    { value: "local", label: "Local Whisper" },
    { value: "deepgram", label: "Deepgram (cloud)" },
    { value: "groq", label: "Groq (cloud, Whisper Large v3 Turbo)" },
  ]}
  onChange={(v) => onProviderChange(v as Provider)}
/>
```

Add per-provider model dropdowns (Deepgram: `nova-3`, `nova-2`; Groq: fixed `whisper-large-v3-turbo`) and per-provider API-key inputs. Reuse the same `ApiKeyField` component the OpenAI key uses today; pass the provider id into it.

- [ ] **Step 3: Route writes through `set_provider_config`**

In `useSettings.ts`, replace the multi-key write fan-out with a single call. The current code (around line 51, 67, 87) calls `update("transcribe_provider", ...)` etc. After the migration:

```ts
async function writeProviderConfig(cfg: ProviderConfig) {
  await settings.setProviderConfig(cfg);
  // Mirror to legacy keys for any code still reading them this release
  // (e.g. recording_start prerequisite check). Phase 3 removes these.
  if (cfg.provider === "openai" || cfg.provider === "local") {
    await update("transcribe_provider", cfg.provider);
    if (cfg.provider === "openai") {
      await update("transcribe_model", cfg.model);
    } else {
      await update("local_whisper_model", cfg.model_id);
      await update("whisper_preset", cfg.preset);
      await update("local_whisper_use_gpu", cfg.use_gpu ? "true" : "false");
    }
  } else {
    // For Deepgram/Groq, also stamp transcribe_provider so the
    // recording_start prerequisite check sees a non-local value.
    await update("transcribe_provider", cfg.provider);
  }
}
```

- [ ] **Step 4: Manual smoke test in dev**

```bash
pnpm tauri dev
```

Then in the running app:
1. Open Settings → Transcription. Confirm both your existing OpenAI/Local choices and the new Deepgram/Groq options render.
2. Set provider to Deepgram, paste your test API key, click Test. Expect "ok".
3. Record ~15s, confirm transcript appears with Deepgram-attributed text.
4. Switch to Groq, repeat.
5. Switch back to Local Whisper, confirm no regression.

- [ ] **Step 5: Commit**

```bash
git add src/lib/ipc.ts src/pages/settings/types.ts src/pages/settings/tabs/Transcription.tsx src/pages/settings/useSettings.ts
git commit -m "settings: surface Deepgram + Groq, write typed ProviderConfig"
```

---

## Task 9: Word type unification

**Files:**
- Modify: `src-tauri/src/local_whisper.rs` (drop the local Word, re-export stt::Word)
- Modify: `src-tauri/src/commands.rs` (drop the inline conversion now that types match)
- Modify: `src-tauri/src/recording.rs` if it uses `local_whisper::Word` directly

- [ ] **Step 1: Re-export `stt::Word` from `local_whisper`**

Replace the local `Word` definition in `src-tauri/src/local_whisper.rs` with a re-export:

```rust
// (delete the existing pub struct Word { ... } definition near line 414)
pub use crate::stt::Word;
```

- [ ] **Step 2: Drop the inline conversion in `transcribe_chunk`**

In `commands.rs`, the post-adapter conversion currently looks like:

```rust
let words: Vec<local_whisper::Word> = words
    .into_iter()
    .map(|w| local_whisper::Word { text: w.text, start_ms: w.start_ms, end_ms: w.end_ms })
    .collect();
```

Since `local_whisper::Word` now equals `stt::Word`, just:

```rust
// no conversion needed — words is already Vec<stt::Word>
```

Remove the `let words: ...` line and rename downstream uses if needed. The downstream code expects `words: Vec<crate::recording::ChunkWord>` (note: different type) — that conversion stays.

- [ ] **Step 3: Verify compile**

```bash
cargo build --manifest-path src-tauri/Cargo.toml --message-format=short
```

Expected: clean. If `recording.rs` references `local_whisper::Word`, it'll resolve to the re-export — no change needed.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/local_whisper.rs src-tauri/src/commands.rs
git commit -m "stt: unify Word type — local_whisper::Word now re-exports stt::Word"
```

---

## Task 10: v0.22.0 release

**Files:**
- Modify: `package.json`, `src-tauri/tauri.conf.json`, `src-tauri/Cargo.toml`

- [ ] **Step 1: Bump versions**

All three to `"0.22.0"` (or `"version = "0.22.0""` for Cargo.toml). Verify they match:

```bash
grep -E '"version"|^version' package.json src-tauri/tauri.conf.json src-tauri/Cargo.toml
```

- [ ] **Step 2: Refresh Cargo.lock**

```bash
cargo check --manifest-path src-tauri/Cargo.toml
```

- [ ] **Step 3: Commit version bump**

Commit message captures the user-facing change:

```bash
git add package.json src-tauri/tauri.conf.json src-tauri/Cargo.toml src-tauri/Cargo.lock
git commit -m "v0.22.0: Deepgram + Groq STT providers, per-provider Keychain"
```

- [ ] **Step 4: Run release**

```bash
pnpm release
```

Wait for: build → sign → notarise (Apple, ~3-8 min) → staple → updater sign → tag push → GitHub release. Confirm `latest.json` updates and the tag lands.

- [ ] **Step 5: Smoke-test the released DMG**

Install the new DMG over the existing v0.21.0. Confirm:
1. App launches without TCC re-grant.
2. Settings shows the new providers.
3. Switching to Deepgram and recording a clip transcribes successfully.
4. Existing OpenAI / Local users see no regression.

---

## Open questions to resolve at task time

These don't block the plan but the implementer should think about each:

1. **Per-note provider override UI.** Phase 1's `read_provider_config` already picks per-note language; per-note provider would be one more dropdown in the note's settings sheet. Worth doing in Phase 2, or defer to Phase 3?
2. **Deepgram intensifier 1.5 is a measured guess.** Higher causes phonetic over-recognition (a quiet phoneme that *kind of* sounds like "Humla" gets transcribed as "Humla"). Lower is imperceptible. After shipping, watch reports for over-eager keyword matches; tune to 1.3 if too aggressive or 2.0 if vocabulary terms still get missed.
3. **Nova-3 has a separate `keyterm` slot** (up to 100 multi-word phrases, better than `keywords`). This plan uses `keywords` only because it works on every Deepgram model. If Phase 2 ships and most users land on Nova-3, a Phase 3 follow-up could detect `model == "nova-3"` and prefer `keyterm`.
4. **Groq rate limits** — tighter than OpenAI's. If a user's recording produces >30 chunks/min (unusual but possible on long monologues with no pauses), Groq may 429. The transcribe layer doesn't currently retry; if this happens in practice, add `retry-with-backoff` logic to `openai_compat::transcribe`.
5. **Settings UI legacy mirror in Step 8.3.** This double-write keeps the recording_start prerequisite check (line 1675 of commands.rs) working. Phase 3 should switch that check to read `transcribe_config` and drop the mirror.

## What I'd hold off on

- **Streaming providers.** Plan deferred per the design doc; revisit when batch-only proves limiting.
- **Self-hosted Whisper.cpp via OpenAI-compat.** With the `OpenAiCompatAdapter` factored out in Task 4, this is essentially a 30-line variant. Add it if user demand surfaces; otherwise defer.
- **Removing the legacy settings keys entirely.** Phase 3, after `transcribe_config` has been the canonical source for at least one minor version.

## Self-review

- **Spec coverage:** Tasks 1–9 cover the six Phase 2 goals (multi-provider keychain, `TranscribeCtx` field split, ProviderConfig write path, two new providers, Settings UI migration, Word unification). Task 10 ships them.
- **Type consistency:** `provider_id` returned by adapters (`"openai"`, `"local"`, `"deepgram"`, `"groq"`) matches `canonical_provider_id` in commands and the Settings UI's radio values. `keychain_account_for` covers the same set. `bias_terms: &[&str]` plumbing is consistent across `TranscribeCtx`, `openai_compat::transcribe`, all four adapter implementations, and the `transcribe_chunk` callsite.
- **No placeholders:** every code step has the actual code. Frontend bits intentionally point at file:line and structural changes rather than dictating the full JSX — the implementer needs to read those components in context to integrate cleanly.
- **Test scope:** Task 6's Deepgram parser test uses a real-shape JSON fixture. Task 5's Groq test is metadata-only because Groq's response shape is identical to OpenAI's — already covered. Task 3's `TranscribeCtx` change is verified by manual smoke test — bit-identical Whisper prompt output is the gate (no automated regression test for prompt assembly because we'd need to expose `build_whisper_prompt`'s exact joining behaviour as a stable contract, which we don't want to commit to yet).

## Estimated diff

| Task | Lines added | Lines removed | Time |
|---|---|---|---|
| 1: Multi-provider keychain | ~120 | ~30 | 1.5h |
| 2: Config variants | ~70 | 0 | 30m |
| 3: TranscribeCtx field split | ~50 | ~20 | 1h |
| 4: openai_compat extract | ~150 | ~80 | 1h |
| 5: GroqAdapter | ~70 | 0 | 30m |
| 6: DeepgramAdapter | ~160 | 0 | 1.5h |
| 7: Tauri commands | ~120 | ~5 | 1h |
| 8: Settings UI | ~250 | ~80 | 3-4h |
| 9: Word unification | ~10 | ~30 | 30m |
| 10: Release | ~5 | 0 | 30m + notarise wait |
| **Total** | **~1005** | **~245** | **~11-13h focused** |
