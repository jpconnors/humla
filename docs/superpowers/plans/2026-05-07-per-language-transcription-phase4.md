# Per-language Transcription Routing (Phase 4) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let users route different recording languages to different STT providers + models — e.g. Norwegian → local NB Whisper, English → Deepgram Nova-3, fallback → OpenAI whisper-1. Ship as v0.24.0.

**Architecture:** Wrap today's bare `ProviderConfig` JSON (stored in `transcribe_config` settings row) inside a new `TranscribeConfig { default, per_language: BTreeMap<String, ProviderConfig> }` shape. A pure `resolve(&self, language)` method picks the per-language entry if present, else default. The current `addon_for_language` auto-routing inside `local_whisper` is **dropped**; downloaded language-specific models no longer auto-apply — instead, Settings surfaces a one-click "Add as <language> override?" suggestion. The `Primary` / `LanguageAddon` enum variants are renamed `Multilingual` / `LanguageSpecific` to drop the misleading "add-on stacks on top of a primary" mental model. UI gets a new "Per-language overrides" section in the Transcription tab, plus a refactored Local-models list with explicit language tags.

**Tech Stack:** Rust 1.85, Tauri 2, async-trait, serde, parking_lot, rusqlite. Frontend: React 19 + TypeScript + Vite (existing settings hook pattern).

**Reference docs:** `docs/design/per-language-transcription.md` (the design doc this plan implements; user has approved Q1=drop auto-apply, Q3=drop `addon_for_language`, recommendations stand for Q2/Q4/Q5/Q6/Q7).

---

## Background for the implementer

Phase 3 (shipped as v0.23.0) made `transcribe_config` the single source of truth for the active STT provider, with a one-shot DB migration retiring all legacy flat keys. The stored shape is currently a bare `ProviderConfig`:

```jsonc
{ "provider": "deepgram", "model": "nova-3" }
```

Phase 4 wraps that shape into a `TranscribeConfig` and adds a per-language map:

```jsonc
{
  "default": { "provider": "openai", "model": "whisper-1" },
  "per_language": {
    "no": { "provider": "local", "model_id": "nb-whisper-large-q5", "preset": "quality", "use_gpu": true },
    "en": { "provider": "deepgram", "model": "nova-3" }
  }
}
```

A startup migration (`migrate_per_language_v4`) wraps the bare-`ProviderConfig` row into the new shape; idempotent via parse-as-`TranscribeConfig` check (no flag row needed because the parse itself decides whether work is required).

Two tightly-related design decisions, both confirmed:

1. **Auto-apply for downloaded language-specific local models is dropped.** Today, downloading NB Whisper makes Norwegian recordings automatically route to it via `local_whisper::addon_for_language`. After Phase 4, no implicit routing exists — the user must add an explicit override (or accept the post-download suggestion).
2. **`addon_for_language` is dropped entirely.** Resolution becomes a single rule: `per_language.get(language).unwrap_or(&default)`. The `LanguageAddon` enum variant is renamed `LanguageSpecific` to drop the stacking implication; the UI badge "NO auto" becomes "Norwegian".

The implementation falls into three layers:
- **Schema + resolver** (Tasks 1–4): `TranscribeConfig`, `resolve()`, callers updated to take a language arg.
- **Migration + naming** (Tasks 5–6): DB migration, `local_whisper` enum rename, drop `addon_for_language`.
- **UI** (Tasks 7–11): `useSettings` refactor, new `PerLanguageOverrides` section, refactored `LocalModelManager`, factored `<ProviderConfigForm>` reused by default + override-add.

Then Tasks 12–14 are smoke test + version bump + release.

---

## File Structure

### New files

| File | Responsibility |
|---|---|
| `src/pages/settings/components/ProviderConfigForm.tsx` | Reusable form for picking provider + model + (for local) preset + GPU. Used by both the default-provider section AND the per-language override add/edit form. |
| `src/pages/settings/components/PerLanguageOverrides.tsx` | The new "Per-language overrides" section — list of entries + add form. |

### Modified files

| File | Change |
|---|---|
| `src-tauri/src/stt/config.rs` | Add `TranscribeConfig` struct + `resolve()` method + round-trip tests. Re-export from `stt::mod`. |
| `src-tauri/src/stt/mod.rs` | Re-export `TranscribeConfig`. |
| `src-tauri/src/local_whisper.rs` | Rename `Primary` → `Multilingual`, `LanguageAddon { language }` → `LanguageSpecific { language }`. Drop `addon_for_language()`. Update copy on the NB Whisper entry. |
| `src-tauri/src/db.rs` | Add `migrate_per_language_v4` with tests. |
| `src-tauri/src/lib.rs` | Wire migration. Rename Tauri commands `get_provider_config` → `get_transcribe_config`, `set_provider_config` → `set_transcribe_config`. |
| `src-tauri/src/commands.rs` | Replace `read_provider_config` with `read_transcribe_config`. Update `recording_start` prereq, prewarm, and `transcribe_chunk` to call `cfg.resolve(&language)`. Update `local_model_path` to no longer special-case the language addon. Rename the Tauri commands. |
| `src/lib/ipc.ts` | Add `TranscribeConfig` type. Replace `getProviderConfig` / `setProviderConfig` with `getTranscribeConfig` / `setTranscribeConfig`. |
| `src/pages/settings/useSettings.ts` | Replace `providerConfig` state with `transcribeConfig`. Helpers: `defaultConfig` getter, `setDefaultConfig`, `addLanguageOverride`, `removeLanguageOverride`. |
| `src/pages/settings/tabs/Transcription.tsx` | "Default provider" section uses `<ProviderConfigForm>`. New "Per-language overrides" section. Removes the inline provider/model JSX. |
| `src/pages/settings/tabs/ApiKeys.tsx` | No change (already uses `saveProviderKey/testProviderKey`). |
| `src/pages/settings/components/LocalModelManager.tsx` | Drop "addon" badge wording. Render language tag ("Norwegian", "Multilingual"). Radio button only enabled for `Multilingual` entries. After download of a `LanguageSpecific` model, surface a "Add as <language> override?" toast. |
| `src/pages/settings/types.ts` | Drop the `Provider` re-export usage if it became unused (TypeScript compile will tell us). |
| `src/pages/Settings.tsx` | Pass new `useSettings` return-shape props to TranscriptionTab. |
| `package.json`, `src-tauri/tauri.conf.json`, `src-tauri/Cargo.toml` | Bump to 0.24.0. |

---

## Task 1: Add `TranscribeConfig` + `resolve()` with tests

**Files:**
- Modify: `src-tauri/src/stt/config.rs`
- Modify: `src-tauri/src/stt/mod.rs`

- [ ] **Step 1: Define the new struct**

Append to `src-tauri/src/stt/config.rs` (after the existing `ProviderConfig` impl):

```rust
use std::collections::BTreeMap;

/// Top-level transcription configuration. Wraps a default `ProviderConfig`
/// plus a map of per-language overrides keyed by ISO 639-1 code (matching
/// the `language` field on Note and the global `language` setting).
///
/// Resolution order at recording time:
///   1. If the recording's language matches a `per_language` key, use that.
///   2. Otherwise use `default`.
///
/// The "auto" pseudo-language never matches a `per_language` entry —
/// resolves to `default`. This mirrors today's `addon_for_language`
/// behaviour, which returned None for "auto".
///
/// `BTreeMap` (not `HashMap`) is intentional: stable JSON key order makes
/// settings diffs readable.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TranscribeConfig {
    pub default: ProviderConfig,
    #[serde(default)]
    pub per_language: BTreeMap<String, ProviderConfig>,
}

impl TranscribeConfig {
    /// Pick the active `ProviderConfig` for the given recording language.
    /// `language` should be an ISO 639-1 code or "auto"; anything else
    /// just falls through to the default (no error case worth surfacing).
    pub fn resolve(&self, language: &str) -> &ProviderConfig {
        if language == "auto" {
            return &self.default;
        }
        self.per_language.get(language).unwrap_or(&self.default)
    }

    /// Sensible default for fresh installs and for recovering from a
    /// corrupt `transcribe_config` row. Matches the bare-default
    /// produced by `from_legacy_settings(None, …)` in v0.23, wrapped.
    pub fn default_fallback() -> Self {
        Self {
            default: from_legacy_settings(None, None, None, None, None),
            per_language: BTreeMap::new(),
        }
    }
}
```

- [ ] **Step 2: Add round-trip + resolve tests**

Append to the existing `mod tests` block in `src-tauri/src/stt/config.rs`:

```rust
    #[test]
    fn transcribe_config_round_trips_through_json() {
        let mut per = BTreeMap::new();
        per.insert(
            "no".to_string(),
            ProviderConfig::Local(LocalWhisperConfig {
                model_id: "nb-whisper-large-q5".to_string(),
                preset: "quality".to_string(),
                use_gpu: true,
            }),
        );
        per.insert(
            "en".to_string(),
            ProviderConfig::Deepgram(DeepgramConfig {
                model: "nova-3".to_string(),
                base_url: None,
            }),
        );
        let cfg = TranscribeConfig {
            default: ProviderConfig::OpenAi(OpenAiConfig {
                model: "whisper-1".to_string(),
                base_url: None,
            }),
            per_language: per,
        };
        let json = serde_json::to_string(&cfg).unwrap();
        let back: TranscribeConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(cfg, back);
        // BTreeMap key order in JSON: "en" before "no" alphabetically.
        let en_pos = json.find(r#""en":"#).unwrap();
        let no_pos = json.find(r#""no":"#).unwrap();
        assert!(en_pos < no_pos, "BTreeMap should serialise keys in order");
    }

    #[test]
    fn transcribe_config_with_empty_per_language_round_trips() {
        let cfg = TranscribeConfig {
            default: ProviderConfig::OpenAi(OpenAiConfig {
                model: "whisper-1".to_string(),
                base_url: None,
            }),
            per_language: BTreeMap::new(),
        };
        let json = serde_json::to_string(&cfg).unwrap();
        let back: TranscribeConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(cfg, back);
    }

    #[test]
    fn resolve_returns_per_language_match() {
        let mut per = BTreeMap::new();
        per.insert(
            "no".to_string(),
            ProviderConfig::Local(LocalWhisperConfig {
                model_id: "nb-whisper-large-q5".to_string(),
                preset: "quality".to_string(),
                use_gpu: true,
            }),
        );
        let cfg = TranscribeConfig {
            default: ProviderConfig::OpenAi(OpenAiConfig {
                model: "whisper-1".to_string(),
                base_url: None,
            }),
            per_language: per,
        };
        assert_eq!(cfg.resolve("no").provider_id(), "local");
    }

    #[test]
    fn resolve_falls_back_to_default_for_unmapped_language() {
        let cfg = TranscribeConfig {
            default: ProviderConfig::OpenAi(OpenAiConfig {
                model: "whisper-1".to_string(),
                base_url: None,
            }),
            per_language: BTreeMap::new(),
        };
        assert_eq!(cfg.resolve("de").provider_id(), "openai");
    }

    #[test]
    fn resolve_treats_auto_as_default_even_with_overrides_present() {
        let mut per = BTreeMap::new();
        per.insert(
            "no".to_string(),
            ProviderConfig::Local(LocalWhisperConfig {
                model_id: "nb-whisper-large-q5".to_string(),
                preset: "quality".to_string(),
                use_gpu: true,
            }),
        );
        let cfg = TranscribeConfig {
            default: ProviderConfig::OpenAi(OpenAiConfig {
                model: "whisper-1".to_string(),
                base_url: None,
            }),
            per_language: per,
        };
        // "auto" is never a real ISO language code — never matches an
        // override. Same semantics as the old addon_for_language guard.
        assert_eq!(cfg.resolve("auto").provider_id(), "openai");
    }

    #[test]
    fn legacy_provider_config_does_not_parse_as_transcribe_config() {
        // Migration logic (Task 5) relies on this asymmetry: a stored
        // bare ProviderConfig must FAIL to deserialise into a
        // TranscribeConfig so the migration knows to wrap.
        let legacy = r#"{"provider":"openai","model":"whisper-1"}"#;
        assert!(serde_json::from_str::<TranscribeConfig>(legacy).is_err());
    }
```

- [ ] **Step 3: Re-export from `stt::mod`**

Edit `src-tauri/src/stt/mod.rs`. Find the existing config re-export (`pub use config::{...}`) and add `TranscribeConfig`:

```rust
pub use config::{
    from_legacy_settings, DeepgramConfig, GroqConfig, LocalWhisperConfig,
    OpenAiConfig, ProviderConfig, TranscribeConfig,
};
```

- [ ] **Step 4: Run tests**

```bash
cargo test --manifest-path src-tauri/Cargo.toml stt::config -- --nocapture
```

Expected: 14 tests pass (the 8 from earlier phases + 6 new).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/stt/config.rs src-tauri/src/stt/mod.rs
git commit -m "stt: add TranscribeConfig with resolve() + per-language map"
```

---

## Task 2: Backend — `read_transcribe_config` + thread language through callers

**Files:**
- Modify: `src-tauri/src/commands.rs`

- [ ] **Step 1: Replace `read_provider_config` with `read_transcribe_config`**

Find `commands.rs` `fn read_provider_config(...)` (the version Phase 3 left at ~line 1490). Replace the entire function with:

```rust
/// Read the active transcription config (default + per-language
/// overrides) from the typed `transcribe_config` JSON. Falls back to a
/// hardcoded default when the row is absent or corrupt — defensive only;
/// `db::migrate_per_language_v4` ensures the row is always present after
/// one launch under v0.24+.
fn read_transcribe_config(
    state: &State<AppState>,
) -> anyhow::Result<crate::stt::TranscribeConfig> {
    let conn = state.db.lock();
    if let Some(json) = db::get_setting(&conn, "transcribe_config")? {
        if let Ok(cfg) = serde_json::from_str::<crate::stt::TranscribeConfig>(&json) {
            return Ok(cfg);
        }
        // Corrupted JSON — fall through to the default rather than locking
        // the user out over a malformed cache. Settings UI overwrites
        // this when the user opens the Transcription tab.
    }
    Ok(crate::stt::TranscribeConfig::default_fallback())
}
```

- [ ] **Step 2: Update `recording_start` prereq + prewarm**

Find the Phase 3 prereq block (the `let provider_cfg = read_provider_config(&state)…` line, around `commands.rs:1771`). Change the call:

```rust
// Resolve the per-language override (if any) up front. Both the
// prereq check and the prewarm path use the resolved provider —
// otherwise a user with a Norwegian override (Local) and an English
// default (Deepgram) would prewarm the wrong model when starting an
// English meeting.
let transcribe_cfg = read_transcribe_config(&state)
    .map_err(|e| e.to_string())?;
let language = {
    let conn = state.db.lock();
    let global = db::get_setting(&conn, "language")
        .map_err(err)?
        .unwrap_or_else(|| DEFAULT_LANGUAGE.to_string());
    let note_lang = db::get_note(&conn, &note_id)
        .map(|n| n.language)
        .unwrap_or_default();
    if note_lang.trim().is_empty() { global } else { note_lang }
};
let provider_cfg = transcribe_cfg.resolve(&language).clone();
```

The rest of the prereq + prewarm block (the `match &provider_cfg { … }` and the `if let crate::stt::ProviderConfig::Local(local_cfg) = &provider_cfg { … }`) keeps working unchanged — `provider_cfg` is still a `ProviderConfig`, just resolved instead of read directly.

- [ ] **Step 3: Update `transcribe_chunk`**

Find the Phase 3 chunk-dispatch block (around `commands.rs:3617`). Replace `read_provider_config` with the resolve flow:

```rust
let provider_cfg = {
    let state: State<AppState> = app.state();
    read_transcribe_config(&state)?.resolve(&language).clone()
};
```

The `(language, vocabulary)` block already runs first and resolves the per-note-or-global language; that order is correct because resolve needs language. Move the `read_transcribe_config(&state)?` call to AFTER the `let (language, vocabulary) = { … }` block (the existing flow has provider_cfg first; we swap them so language comes first).

The rest of `transcribe_chunk` keeps working — `provider_cfg.provider_id()`, `provider_cfg.model()`, `provider_cfg.base_url()`, the `matches!(provider_cfg, …Local(_))` check all still apply unchanged.

- [ ] **Step 4: Compile + tests**

```bash
cargo build --manifest-path src-tauri/Cargo.toml --message-format=short
```

Expected: clean.

```bash
cargo test --manifest-path src-tauri/Cargo.toml -- --nocapture
```

Expected: 116 tests pass (115 from Phase 3 + 6 new schema tests, minus 5 phase-3 migration tests if they renamed — they didn't, so plus 6 = 121 minus any other diffs; the exact count isn't load-bearing as long as nothing fails).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/commands.rs
git commit -m "commands: read_transcribe_config + resolve via recording language"
```

---

## Task 3: Drop `local_whisper::addon_for_language` + rename enum variants

**Files:**
- Modify: `src-tauri/src/local_whisper.rs`
- Modify: `src-tauri/src/commands.rs` (call site in `local_model_path`)

**Why this is one task, not two:** the rename and the drop are mechanically intertwined — `local_model_path` calls `addon_for_language`, the enum is referenced by both the rename and the drop. Splitting them would leave the tree red between commits without a clear rollback story.

- [ ] **Step 1: Rename + simplify the enum + registry**

In `src-tauri/src/local_whisper.rs`, find the `pub enum ModelKind` block (around `local_whisper.rs:29`). Replace with:

```rust
// Each model declares its language scope:
//   - Multilingual: handles any language — these are the user-pickable
//     general-purpose models, one of which is always the active default.
//   - LanguageSpecific { language }: specialised for one ISO 639-1
//     language. Never the default; selected via per-language override
//     in transcribe_config.per_language. NB Whisper Large is finetuned
//     by Nasjonalbiblioteket on Norwegian and produces noticeably worse
//     output on other languages — that's why it's tagged here instead
//     of being a general option in the picker.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModelKind {
    Multilingual,
    LanguageSpecific { language: &'static str },
}
```

Update each `ModelInfo` entry's `kind`:
- All four general models: `ModelKind::Primary` → `ModelKind::Multilingual`.
- The NB Whisper entry: `ModelKind::LanguageAddon { language: "no" }` → `ModelKind::LanguageSpecific { language: "no" }`.

Also update the NB Whisper `label` and `description` to drop the "add-on" wording:

```rust
ModelInfo {
    id: "nb-whisper-large-q5",
    label: "NB Whisper Large",
    filename: "nb-whisper-large-q5_0.bin",
    url: "https://huggingface.co/NbAiLab/nb-whisper-large/resolve/main/ggml-model-q5_0.bin",
    size_bytes_hint: 1_159_237_632,
    description: "Norwegian-finetuned by Nasjonalbiblioteket. Pick this as a per-language override for Norwegian recordings — produces noticeably worse output on English or other languages.",
    kind: ModelKind::LanguageSpecific { language: "no" },
},
```

- [ ] **Step 2: Delete `addon_for_language`**

Still in `local_whisper.rs`, delete the function around `local_whisper.rs:108–120`:

```rust
pub fn addon_for_language(language: &str) -> Option<&'static ModelInfo> {
    if language == "auto" {
        return None;
    }
    MODELS.iter().find(|m| match m.kind {
        ModelKind::LanguageAddon { language: addon_lang } => addon_lang == language,
        _ => false,
    })
}
```

- [ ] **Step 3: Update `local_model_path` to drop the addon special-case**

In `src-tauri/src/commands.rs`, find `local_model_path` (the function refactored in Phase 3 to take `model_id: &str`). The current body has an early `if let Some(addon) = local_whisper::addon_for_language(language) {…}` block — delete it. The simplified function becomes:

```rust
fn local_model_path(
    app: &AppHandle,
    _language: &str,
    model_id: &str,
) -> Result<PathBuf, String> {
    let dir = local_model_dir(app)?;
    let info = local_whisper::find_model(model_id)
        .filter(|m| m.kind == local_whisper::ModelKind::Multilingual
            || matches!(m.kind, local_whisper::ModelKind::LanguageSpecific { .. }))
        .unwrap_or_else(local_whisper::default_model);
    let path = dir.join(info.filename);
    if path.exists() {
        return Ok(path);
    }
    Ok(dir.join(local_whisper::default_model().filename))
}
```

The `language` argument is now unused; prefix with `_` so the compiler doesn't warn. Keeping the parameter (rather than removing) avoids cascading signature changes — there are 3 call sites, all in commands.rs. The argument lives in case some future addon-like behaviour wants it back.

The filter relaxation (now accepts both `Multilingual` and `LanguageSpecific`) is intentional: a `LanguageSpecific` entry is now legitimately user-selectable via a per-language override's `model_id`, so we shouldn't strip it like the old `Primary`-only filter did.

- [ ] **Step 4: Update the `local_whisper_models` Tauri command's `kind` payload**

Find the `LocalWhisperModelStatus` struct + the `local_whisper_models` Tauri command (around `commands.rs:1500`). The struct has a `kind: &'static str` field; the populating line currently writes `"primary"` or `"addon"`. Replace those literals with `"multilingual"` / `"language_specific"`.

```rust
let (kind_str, addon_lang) = match info.kind {
    local_whisper::ModelKind::Multilingual => ("multilingual", None),
    local_whisper::ModelKind::LanguageSpecific { language } => {
        ("language_specific", Some(language.to_string()))
    }
};
```

The `LocalWhisperModelStatus.kind` doc comment also needs updating — change to:

```rust
/// "multilingual" — selectable as the default transcription model.
/// "language_specific" — usable only as the model behind a per-language
/// override in transcribe_config.per_language. Never the default.
kind: &'static str,
```

- [ ] **Step 5: Update frontend type to match**

In `src/lib/ipc.ts`, find `LocalWhisperModelStatus`. Change:

```ts
kind: "primary" | "addon";
addonLanguage: string | null;
```

to:

```ts
kind: "multilingual" | "language_specific";
specificLanguage: string | null;
```

Then update consumers — `grep -rn 'kind === "primary"\|kind === "addon"\|addonLanguage' src` and rename references. Two files use these:
- `src/pages/settings/useSettings.ts` lines `~196`, `~225` — change `m.kind === "primary"` to `m.kind === "multilingual"`.
- `src/pages/settings/components/LocalModelManager.tsx` — uses `addonLanguage` for the badge. Rename to `specificLanguage` (Task 8 will rewrite the rendering logic anyway, so just rename the field for now).

- [ ] **Step 6: Compile + test**

```bash
cargo build --manifest-path src-tauri/Cargo.toml --message-format=short
```

Expected: clean. Should NOT see any "function `addon_for_language` is never used" warning — the only caller is gone.

```bash
cargo test --manifest-path src-tauri/Cargo.toml -- --nocapture
```

Expected: all green.

```bash
pnpm tsc -b
```

Expected: clean.

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/local_whisper.rs src-tauri/src/commands.rs src/lib/ipc.ts src/pages/settings/useSettings.ts src/pages/settings/components/LocalModelManager.tsx
git commit -m "stt: rename ModelKind variants; drop addon_for_language auto-routing"
```

---

## Task 4: Rename `get_provider_config` → `get_transcribe_config`, add the new shape

**Files:**
- Modify: `src-tauri/src/commands.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src/lib/ipc.ts`

- [ ] **Step 1: Rename the Tauri commands in `commands.rs`**

Find `get_provider_config` (Phase 3) and `set_provider_config` (Phase 2) — they're back-to-back in `commands.rs`. Rename + retype:

```rust
/// Read the active transcription config (default + per-language
/// overrides). The Settings UI calls this on mount as the source of
/// truth for the Transcription tab and uses it to drive both the
/// Default provider section and the Per-language overrides list.
#[tauri::command]
pub fn get_transcribe_config(
    state: State<AppState>,
) -> Result<crate::stt::TranscribeConfig, String> {
    read_transcribe_config(&state).map_err(|e| e.to_string())
}

/// Persist a typed `TranscribeConfig` to settings. Frontend writes the
/// whole shape on every change so the choice (default + every per-
/// language entry) is atomic — no partial drift from one path
/// half-updating.
#[tauri::command]
pub fn set_transcribe_config(
    state: State<AppState>,
    config: crate::stt::TranscribeConfig,
) -> Result<(), String> {
    let json = serde_json::to_string(&config).map_err(err)?;
    let conn = state.db.lock();
    db::set_setting(&conn, "transcribe_config", &json).map_err(err)
}
```

- [ ] **Step 2: Update the Tauri command registry**

In `src-tauri/src/lib.rs`, find the lines:

```rust
commands::get_provider_config,
commands::set_provider_config,
```

Replace with:

```rust
commands::get_transcribe_config,
commands::set_transcribe_config,
```

- [ ] **Step 3: Update the frontend bindings**

In `src/lib/ipc.ts`, find:

```ts
getProviderConfig: () => invoke<ProviderConfig>("get_provider_config"),
setProviderConfig: (config: ProviderConfig) =>
  invoke<void>("set_provider_config", { config }),
```

Replace with:

```ts
getTranscribeConfig: () => invoke<TranscribeConfig>("get_transcribe_config"),
setTranscribeConfig: (config: TranscribeConfig) =>
  invoke<void>("set_transcribe_config", { config }),
```

Also add the new type definition next to `ProviderConfig`:

```ts
/// Mirror of the Rust `crate::stt::TranscribeConfig`. Wraps a default
/// provider config plus a map of per-language overrides keyed by ISO
/// 639-1 code (matching Note.language and the global `language` setting).
export type TranscribeConfig = {
  default: ProviderConfig;
  per_language: Record<string, ProviderConfig>;
};
```

(`Record<string, ProviderConfig>` is the natural TypeScript representation of a Rust `BTreeMap<String, ProviderConfig>` — serde serialises it as a plain JSON object, which JS reads as a key-keyed dictionary.)

- [ ] **Step 4: Compile**

```bash
cargo build --manifest-path src-tauri/Cargo.toml --message-format=short
```

Expected: clean.

```bash
pnpm tsc -b
```

Expected: errors in `useSettings.ts` (still uses `getProviderConfig` / `setProviderConfig`). Task 6 fixes those.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/commands.rs src-tauri/src/lib.rs src/lib/ipc.ts
git commit -m "commands: rename to get/set_transcribe_config; add TranscribeConfig type"
```

---

## Task 5: DB migration `migrate_per_language_v4`

**Files:**
- Modify: `src-tauri/src/db.rs`
- Modify: `src-tauri/src/lib.rs`

- [ ] **Step 1: Add the migration function**

In `src-tauri/src/db.rs`, after `migrate_transcribe_config` (the v3 migration from Phase 3), add:

```rust
/// One-shot v0.24 migration: wrap a bare `ProviderConfig` JSON in
/// `transcribe_config` into the new `TranscribeConfig { default,
/// per_language }` shape. Idempotent via the parse-as-TranscribeConfig
/// check — running twice is a no-op because the second pass parses
/// successfully and bails.
///
/// Unlike `migrate_transcribe_config`, this migration doesn't need a
/// flag row: the parse outcome itself encodes whether work is needed.
/// (Phase 3 needed a flag because it deleted seven other rows whose
/// absence couldn't reliably distinguish "fresh install" from
/// "already migrated".)
pub fn migrate_per_language_v4(conn: &Connection) -> Result<()> {
    let Some(raw) = get_setting(conn, "transcribe_config")? else {
        // No transcribe_config row at all — fresh install or v0.21
        // user who hasn't been touched by migrate_transcribe_config
        // yet (it runs first). Either way, nothing to wrap. The
        // read_transcribe_config fallback covers this user when the
        // app reads.
        return Ok(());
    };
    if serde_json::from_str::<crate::stt::TranscribeConfig>(&raw).is_ok() {
        // Already in the new shape — second-or-later run, no-op.
        return Ok(());
    }
    let Ok(legacy) = serde_json::from_str::<crate::stt::ProviderConfig>(&raw) else {
        // Row is neither a TranscribeConfig nor a bare ProviderConfig.
        // Probably a corrupt write from some intermediate dev state.
        // Don't touch it — leave the read_transcribe_config fallback
        // to recover. Logging via the caller's eprintln is fine.
        return Err(anyhow::anyhow!(
            "transcribe_config row is neither TranscribeConfig nor ProviderConfig — leaving untouched"
        ));
    };
    let wrapped = crate::stt::TranscribeConfig {
        default: legacy,
        per_language: std::collections::BTreeMap::new(),
    };
    let json = serde_json::to_string(&wrapped)
        .map_err(|e| anyhow::anyhow!("serialize wrapped TranscribeConfig: {e}"))?;
    set_setting(conn, "transcribe_config", &json)?;
    Ok(())
}
```

- [ ] **Step 2: Add tests**

In the existing `mod tests` block in `db.rs`, add:

```rust
    #[test]
    fn migrate_per_language_v4_wraps_bare_provider_config() {
        // v0.23 user upgrading: typed transcribe_config exists as a
        // bare ProviderConfig. Migration wraps into TranscribeConfig.
        let conn = settings_only_conn();
        set_setting(
            &conn,
            "transcribe_config",
            r#"{"provider":"deepgram","model":"nova-3"}"#,
        )
        .unwrap();
        migrate_per_language_v4(&conn).unwrap();
        let after = get_setting(&conn, "transcribe_config").unwrap().unwrap();
        let parsed: crate::stt::TranscribeConfig = serde_json::from_str(&after).unwrap();
        assert_eq!(parsed.default.provider_id(), "deepgram");
        assert!(parsed.per_language.is_empty());
    }

    #[test]
    fn migrate_per_language_v4_is_idempotent() {
        let conn = settings_only_conn();
        set_setting(
            &conn,
            "transcribe_config",
            r#"{"provider":"openai","model":"whisper-1"}"#,
        )
        .unwrap();
        migrate_per_language_v4(&conn).unwrap();
        let after_first = get_setting(&conn, "transcribe_config").unwrap();
        migrate_per_language_v4(&conn).unwrap();
        let after_second = get_setting(&conn, "transcribe_config").unwrap();
        assert_eq!(after_first, after_second, "second run must be a no-op");
    }

    #[test]
    fn migrate_per_language_v4_skips_when_row_absent() {
        // Fresh install: no transcribe_config yet. Migration runs but
        // finds nothing to wrap. The runtime fallback in
        // read_transcribe_config handles this user.
        let conn = settings_only_conn();
        migrate_per_language_v4(&conn).unwrap();
        assert!(get_setting(&conn, "transcribe_config").unwrap().is_none());
    }

    #[test]
    fn migrate_per_language_v4_preserves_existing_overrides_on_rerun() {
        // v0.24 user re-runs the migration (every launch). The row
        // already has `per_language` entries; they must survive.
        let conn = settings_only_conn();
        set_setting(
            &conn,
            "transcribe_config",
            r#"{"default":{"provider":"openai","model":"whisper-1"},"per_language":{"no":{"provider":"local","model_id":"nb-whisper-large-q5","preset":"quality","use_gpu":true}}}"#,
        )
        .unwrap();
        migrate_per_language_v4(&conn).unwrap();
        let after = get_setting(&conn, "transcribe_config").unwrap().unwrap();
        let parsed: crate::stt::TranscribeConfig = serde_json::from_str(&after).unwrap();
        assert_eq!(parsed.per_language.len(), 1);
        assert_eq!(parsed.per_language.get("no").unwrap().provider_id(), "local");
    }

    #[test]
    fn migrate_per_language_v4_errors_on_garbage_row() {
        let conn = settings_only_conn();
        set_setting(&conn, "transcribe_config", r#"{"bogus":true}"#).unwrap();
        // Not a fatal failure for the user — caller logs and falls
        // through; read_transcribe_config recovers via its own
        // fallback. We assert the error type only to document
        // behaviour, not to require the caller to surface it.
        assert!(migrate_per_language_v4(&conn).is_err());
    }
```

- [ ] **Step 3: Wire migration into startup**

In `src-tauri/src/lib.rs`, find the `migrate_transcribe_config` block (Phase 3, around `lib.rs:135`). Add a parallel block immediately after:

```rust
// v0.24 — wrap the v0.23 bare-ProviderConfig transcribe_config row
// into the new TranscribeConfig { default, per_language } shape.
// Idempotent via parse check; runs after migrate_transcribe_config so
// v0.21 → v0.24 users get both legacy collapse AND wrap in one launch.
{
    let state: tauri::State<AppState> = app.state();
    let conn = state.db.lock();
    if let Err(e) = db::migrate_per_language_v4(&conn) {
        eprintln!("migrate_per_language_v4: {e}");
    }
}
```

- [ ] **Step 4: Run tests**

```bash
cargo test --manifest-path src-tauri/Cargo.toml db::tests -- --nocapture
```

Expected: 10 tests pass (the 5 v3 tests + 5 new v4 tests).

```bash
cargo build --manifest-path src-tauri/Cargo.toml --message-format=short
```

Expected: clean.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/db.rs src-tauri/src/lib.rs
git commit -m "db: migrate_per_language_v4 wraps bare ProviderConfig into TranscribeConfig"
```

---

## Task 6: `useSettings` — `transcribeConfig` as state, drop `providerConfig`

**Files:**
- Modify: `src/pages/settings/useSettings.ts`

- [ ] **Step 1: Replace state + initial load**

Find the `providerConfig` state declaration (`useSettings.ts:~40`) and the `useEffect` that loads it. Replace:

```ts
const [providerConfig, setProviderConfig] = useState<ProviderConfig>({
  provider: "openai",
  model: "whisper-1",
});
```

with:

```ts
const [transcribeConfig, setTranscribeConfig] = useState<TranscribeConfig>({
  default: { provider: "openai", model: "whisper-1" },
  per_language: {},
});
```

In the load `useEffect`, change:

```ts
ipc.getProviderConfig().catch(() => null),
```

to:

```ts
ipc.getTranscribeConfig().catch(() => null),
```

And the consumer:

```ts
if (cfg) setProviderConfig(cfg);
```

to:

```ts
if (cfg) setTranscribeConfig(cfg);
```

The variable destructure `const [k1, kdg, kgrq, models, ds, ss, cfg] = …` keeps `cfg` as the name; just the type changes.

- [ ] **Step 2: Replace `updateProviderConfig` with the new helpers**

Find `async function updateProviderConfig(cfg: ProviderConfig)` (the helper Phase 3 added). Replace with:

```ts
async function updateTranscribeConfig(cfg: TranscribeConfig) {
  setTranscribeConfig(cfg);
  try {
    await ipc.setTranscribeConfig(cfg);
  } catch (e) {
    console.warn("[settings] setTranscribeConfig failed:", e);
  }
}

/// Convenience for the Default provider section. Only mutates the
/// `default` slot; per-language overrides untouched.
async function setDefaultConfig(cfg: ProviderConfig) {
  await updateTranscribeConfig({ ...transcribeConfig, default: cfg });
}

/// Add or replace a per-language override.
async function setLanguageOverride(language: string, cfg: ProviderConfig) {
  await updateTranscribeConfig({
    ...transcribeConfig,
    per_language: { ...transcribeConfig.per_language, [language]: cfg },
  });
}

/// Remove a per-language override entirely. No-op if the language
/// isn't currently mapped.
async function removeLanguageOverride(language: string) {
  if (!(language in transcribeConfig.per_language)) return;
  const next = { ...transcribeConfig.per_language };
  delete next[language];
  await updateTranscribeConfig({ ...transcribeConfig, per_language: next });
}
```

- [ ] **Step 3: Update `downloadModel` and `deleteModel` to use the new helpers**

The Phase 3 versions write `local_whisper_model` via `updateProviderConfig`. Update them to use `transcribeConfig.default`:

```ts
// In downloadModel, around the auto-pick-first-downloaded branch:
if (
  downloadedInfo?.kind === "multilingual" &&
  models.filter((m) => m.kind === "multilingual" && m.downloaded).length === 1 &&
  transcribeConfig.default.provider === "local"
) {
  await setDefaultConfig({
    ...transcribeConfig.default,
    model_id: modelId,
  });
}
```

```ts
// In deleteModel, around the active-model fallback branch:
if (
  transcribeConfig.default.provider === "local" &&
  transcribeConfig.default.model_id === modelId
) {
  const fallback =
    models.find((m) => m.kind === "multilingual" && m.downloaded)?.id ??
    "large-v3-turbo-q5";
  await setDefaultConfig({ ...transcribeConfig.default, model_id: fallback });
}
```

The `kind === "multilingual"` filter (replacing `=== "primary"`) flowed in from Task 3.

- [ ] **Step 4: Update the return shape**

Find the `return { … }` at the bottom of `useSettings`. Replace `providerConfig`/`updateProviderConfig` lines with:

```ts
transcribeConfig,
updateTranscribeConfig,
setDefaultConfig,
setLanguageOverride,
removeLanguageOverride,
```

- [ ] **Step 5: Drop the now-unused `ProviderConfig` import (TS will surface this) — keep `TranscribeConfig`**

Find the imports at the top of the file:

```ts
import {
  ipc,
  onDiarizeDownloadProgress,
  onLocalWhisperProgress,
  type ProviderConfig,
  type TranscribeProvider,
} from "../../lib/ipc";
```

`ProviderConfig` is still referenced by `setDefaultConfig(cfg: ProviderConfig)` and `setLanguageOverride(_, cfg: ProviderConfig)` — keep it. Add `TranscribeConfig`:

```ts
import {
  ipc,
  onDiarizeDownloadProgress,
  onLocalWhisperProgress,
  type ProviderConfig,
  type TranscribeConfig,
  type TranscribeProvider,
} from "../../lib/ipc";
```

- [ ] **Step 6: TypeScript check**

```bash
pnpm tsc -b
```

Expected: errors only in `Transcription.tsx` and `Settings.tsx` (still pass `providerConfig` / `updateProviderConfig`). Tasks 8 + 10 fix those.

- [ ] **Step 7: Commit**

```bash
git add src/pages/settings/useSettings.ts
git commit -m "settings: transcribeConfig as state with default + per-language helpers"
```

---

## Task 7: New reusable `<ProviderConfigForm>` component

**Files:**
- Create: `src/pages/settings/components/ProviderConfigForm.tsx`

This is the form for picking provider + model (+ for Local: preset + GPU). Used by both:
1. The Default provider section in Transcription.tsx (Task 9 wires it).
2. The "Add language override" form in PerLanguageOverrides.tsx (Task 9 too).

Factoring it out is worth it — duplicating ~80 lines of conditional JSX in two places would be the alternative.

- [ ] **Step 1: Create the component**

Create `src/pages/settings/components/ProviderConfigForm.tsx`:

```tsx
import type { ProviderConfig } from "../../../lib/ipc";
import {
  DEEPGRAM_MODELS,
  GROQ_MODELS,
  LOCAL_PROVIDER,
  PROVIDERS_BASE,
  TRANSCRIBE_MODELS,
  WHISPER_PRESETS,
  type Provider,
} from "../types";
import { Select } from "./Select";

/// Reusable provider+model picker. Used by the Default provider section
/// and the per-language override editor. Keeps the four provider
/// variants' divergent fields (OpenAI: model, Local: model_id+preset+gpu,
/// Deepgram: model, Groq: model) in one place so the two callers stay
/// in lockstep.
///
/// `localModels` is the live list of downloaded local models (from
/// `useSettings.local.models`). Used to:
///   1. Hide the Local option when nothing is downloaded.
///   2. Pre-select the first downloaded multilingual model when
///      switching to Local.
///   3. Filter the model_id picker to actually-downloaded files.
export function ProviderConfigForm({
  value,
  onChange,
  localModels,
  /// Restrict the local model picker to a specific language. When set,
  /// the picker shows only `LanguageSpecific` models matching the language
  /// PLUS multilingual models. Used by the per-language override form so
  /// "Norwegian override → Local → ?" only offers NB Whisper or
  /// multilingual fallbacks.
  filterLocalToLanguage,
  /// Hide the "Local" option entirely. Some callers want to constrain
  /// the choice (currently unused; reserved for future polish).
  hideLocal = false,
}: {
  value: ProviderConfig;
  onChange: (next: ProviderConfig) => void;
  localModels: Array<{
    id: string;
    label: string;
    kind: "multilingual" | "language_specific";
    specificLanguage: string | null;
    downloaded: boolean;
  }>;
  filterLocalToLanguage?: string;
  hideLocal?: boolean;
}) {
  const provider = value.provider;
  const localAvailable =
    !hideLocal && localModels.some((m) => m.downloaded);

  const localModelOptions = localModels
    .filter((m) => m.downloaded)
    .filter((m) => {
      if (!filterLocalToLanguage) return true;
      // Multilingual models always usable; language-specific must match.
      return (
        m.kind === "multilingual" ||
        m.specificLanguage === filterLocalToLanguage
      );
    })
    .map((m) => ({ value: m.id, label: m.label }));

  return (
    <div className="flex flex-col gap-3">
      <Select
        value={provider}
        onChange={(v) => {
          const p = v as Provider;
          if (p === "openai") {
            onChange({ provider: "openai", model: "whisper-1" });
          } else if (p === "local") {
            const first = localModels.find(
              (m) =>
                m.downloaded &&
                (filterLocalToLanguage
                  ? m.kind === "multilingual" ||
                    m.specificLanguage === filterLocalToLanguage
                  : m.kind === "multilingual"),
            );
            onChange({
              provider: "local",
              model_id: first?.id ?? "large-v3-turbo-q5",
              preset: "quality",
              use_gpu: true,
            });
          } else if (p === "deepgram") {
            onChange({ provider: "deepgram", model: "nova-3" });
          } else if (p === "groq") {
            onChange({ provider: "groq", model: "whisper-large-v3-turbo" });
          }
        }}
        options={
          localAvailable ? [...PROVIDERS_BASE, LOCAL_PROVIDER] : PROVIDERS_BASE
        }
      />

      {value.provider === "openai" && (
        <Select
          value={value.model}
          onChange={(v) => onChange({ provider: "openai", model: v })}
          options={TRANSCRIBE_MODELS.map((m) => ({ value: m, label: m }))}
        />
      )}

      {value.provider === "deepgram" && (
        <Select
          value={value.model}
          onChange={(v) => onChange({ provider: "deepgram", model: v })}
          options={DEEPGRAM_MODELS.map((m) => ({ value: m, label: m }))}
        />
      )}

      {value.provider === "groq" && (
        <Select
          value={value.model}
          onChange={(v) => onChange({ provider: "groq", model: v })}
          options={GROQ_MODELS.map((m) => ({ value: m, label: m }))}
        />
      )}

      {value.provider === "local" && (
        <>
          <Select
            value={value.model_id}
            onChange={(v) => onChange({ ...value, model_id: v })}
            options={
              localModelOptions.length > 0
                ? localModelOptions
                : [
                    {
                      value: value.model_id,
                      label: `${value.model_id} (not downloaded)`,
                    },
                  ]
            }
          />
          <Select
            value={value.preset}
            onChange={(v) => onChange({ ...value, preset: v })}
            options={WHISPER_PRESETS}
          />
          <label className="flex items-center gap-2 cursor-pointer text-sm">
            <input
              type="checkbox"
              checked={value.use_gpu}
              onChange={(e) =>
                onChange({ ...value, use_gpu: e.target.checked })
              }
            />
            Use Metal (Apple GPU) for Whisper inference
          </label>
        </>
      )}
    </div>
  );
}
```

- [ ] **Step 2: Compile**

```bash
pnpm tsc -b
```

Expected: no new errors. (Task 6's existing errors haven't been fixed yet — those land in Tasks 8/9/10.)

- [ ] **Step 3: Commit**

```bash
git add src/pages/settings/components/ProviderConfigForm.tsx
git commit -m "settings: factor reusable ProviderConfigForm component"
```

---

## Task 8: Refactor `LocalModelManager` — language tags, drop "addon" wording

**Files:**
- Modify: `src/pages/settings/components/LocalModelManager.tsx`

The current LocalModelManager renders a "NO auto" badge for addon models and segregates Primary vs Addon visually. After Phase 4, the segregation goes away (all models in one flat list) and the badges declare language scope.

- [ ] **Step 1: Read the current component**

```bash
wc -l src/pages/settings/components/LocalModelManager.tsx
```

Approximate size: ~140 lines. The bulk of the change is replacing the badge logic and the section split (if any).

- [ ] **Step 2: Update the row component**

Find the row rendering JSX. The current block (around `LocalModelManager.tsx:127–140`):

```tsx
<div className="flex items-center gap-2 text-sm">
  <span className="font-medium">{model.label}</span>
  {model.kind === "addon" && model.addonLanguage && (
    <span className="text-xs px-1.5 py-0.5 rounded bg-[var(--color-pill-hover)] text-[var(--color-text-muted)]">
      {model.addonLanguage} auto
    </span>
  )}
  …
</div>
```

Replace with:

```tsx
<div className="flex items-center gap-2 text-sm">
  <span className="font-medium">{model.label}</span>
  <span className="text-xs px-1.5 py-0.5 rounded bg-[var(--color-pill-hover)] text-[var(--color-text-muted)]">
    {model.kind === "multilingual"
      ? "Multilingual"
      : languageLabel(model.specificLanguage)}
  </span>
  …
</div>
```

Add a small `languageLabel` helper at the top of the file (or imported from `lib/languages`):

```tsx
import { LANGUAGES } from "../../../lib/languages";

function languageLabel(code: string | null): string {
  if (!code) return "Unknown";
  const found = LANGUAGES.find((l) => l.value === code);
  return found?.label ?? code;
}
```

- [ ] **Step 3: Update the radio button gating**

The radio button currently shows for `Primary`. It should now show for `multilingual` only — `language_specific` models are picked via per-language overrides, never via the radio.

Find `showRadio`:

```tsx
showRadio={model.kind === "primary"}
```

(or whatever the prop is — adjust as the file dictates). Change to:

```tsx
showRadio={model.kind === "multilingual"}
```

If the parent decides this rather than the row component, do the equivalent edit at the parent scope.

- [ ] **Step 4: Drop section splits**

If the file currently splits Multilingual vs Addon into two visual groups (with `Primary models` / `Specialised models` headings), collapse into one list. The badges now carry the distinction.

- [ ] **Step 5: Add the post-download suggestion (deferred)**

The "Add as Norwegian override?" suggestion after downloading a `language_specific` model is a v0.24 polish item. Implement as a flash message inside `useSettings.downloadModel` that the LocalState's `flash` field already supports — but with action-button content. To keep this task scoped, **defer the action-button affordance to Task 11 (UI polish)**: for now, the existing `flashLocal` toast just says "NB Whisper Large downloaded" without the override prompt. Note this in the commit message so it's traceable.

- [ ] **Step 6: Compile**

```bash
pnpm tsc -b
```

Expected: errors still in `Transcription.tsx` (Task 9). Other files green.

- [ ] **Step 7: Commit**

```bash
git add src/pages/settings/components/LocalModelManager.tsx
git commit -m "settings: LocalModelManager — language tags, drop 'addon' badge wording"
```

---

## Task 9: New `PerLanguageOverrides` section + Transcription tab rewire

**Files:**
- Create: `src/pages/settings/components/PerLanguageOverrides.tsx`
- Modify: `src/pages/settings/tabs/Transcription.tsx`

- [ ] **Step 1: Create the section component**

Create `src/pages/settings/components/PerLanguageOverrides.tsx`:

```tsx
import { useState } from "react";
import { LANGUAGES, languageOptionLabel } from "../../../lib/languages";
import type { ProviderConfig, TranscribeConfig } from "../../../lib/ipc";
import { ProviderConfigForm } from "./ProviderConfigForm";
import { Select } from "./Select";
import type { LocalState } from "../types";

/// Section that lists existing per-language overrides and offers a
/// + Add form to create new ones. Each entry is rendered as a card with
/// a one-line summary plus a [×] delete button. No inline edit — to
/// change an existing override, the user deletes and re-adds. Keeps
/// the UI simple; revisit if friction surfaces.
export function PerLanguageOverrides({
  config,
  setLanguageOverride,
  removeLanguageOverride,
  local,
}: {
  config: TranscribeConfig;
  setLanguageOverride: (language: string, cfg: ProviderConfig) => Promise<void>;
  removeLanguageOverride: (language: string) => Promise<void>;
  local: LocalState;
}) {
  const [showAddForm, setShowAddForm] = useState(false);
  const entries = Object.entries(config.per_language).sort(([a], [b]) =>
    a.localeCompare(b),
  );

  return (
    <div className="flex flex-col gap-3">
      <p className="text-xs text-[var(--color-text-muted)]">
        Override the default provider for specific recording languages.
        Useful when one model is materially better for a given language —
        e.g. NB Whisper for Norwegian, Deepgram Nova-3 for English.
      </p>

      {entries.length > 0 ? (
        <div className="flex flex-col gap-2">
          {entries.map(([language, cfg]) => (
            <OverrideRow
              key={language}
              language={language}
              cfg={cfg}
              onDelete={() => removeLanguageOverride(language)}
            />
          ))}
        </div>
      ) : (
        <p className="text-xs text-[var(--color-text-muted)] italic">
          No overrides — every recording uses the default provider.
        </p>
      )}

      {!showAddForm && (
        <button
          type="button"
          onClick={() => setShowAddForm(true)}
          className="self-start text-sm px-3 py-1.5 rounded-md border border-[var(--color-line)] hover:bg-[var(--color-pill-hover)]"
        >
          + Add language override
        </button>
      )}
      {showAddForm && (
        <AddOverrideForm
          existingLanguages={Object.keys(config.per_language)}
          local={local}
          onCancel={() => setShowAddForm(false)}
          onAdd={async (language, cfg) => {
            await setLanguageOverride(language, cfg);
            setShowAddForm(false);
          }}
        />
      )}
    </div>
  );
}

function OverrideRow({
  language,
  cfg,
  onDelete,
}: {
  language: string;
  cfg: ProviderConfig;
  onDelete: () => void;
}) {
  return (
    <div className="flex items-start gap-3 px-3 py-2 rounded-md border border-[var(--color-line)]">
      <div className="flex-1 flex flex-col gap-0.5">
        <div className="text-sm font-medium">
          {languageOptionLabel(
            LANGUAGES.find((l) => l.value === language) ?? {
              value: language,
              label: language,
            },
          )}
        </div>
        <div className="text-xs text-[var(--color-text-muted)]">
          {summariseProvider(cfg)}
        </div>
      </div>
      <button
        type="button"
        onClick={onDelete}
        aria-label={`Remove ${language} override`}
        className="text-sm px-2 py-1 rounded hover:bg-[var(--color-pill-hover)]"
      >
        ×
      </button>
    </div>
  );
}

function summariseProvider(cfg: ProviderConfig): string {
  switch (cfg.provider) {
    case "openai":
      return `OpenAI · ${cfg.model}`;
    case "deepgram":
      return `Deepgram · ${cfg.model}`;
    case "groq":
      return `Groq · ${cfg.model}`;
    case "local":
      return `Local · ${cfg.model_id} · ${cfg.preset} preset · GPU ${cfg.use_gpu ? "on" : "off"}`;
  }
}

function AddOverrideForm({
  existingLanguages,
  local,
  onCancel,
  onAdd,
}: {
  existingLanguages: string[];
  local: LocalState;
  onCancel: () => void;
  onAdd: (language: string, cfg: ProviderConfig) => Promise<void>;
}) {
  // Filter the language picker to languages NOT already overridden.
  const available = LANGUAGES.filter((l) => !existingLanguages.includes(l.value));
  const initialLanguage = available[0]?.value ?? "no";
  const [language, setLanguage] = useState(initialLanguage);
  const [cfg, setCfg] = useState<ProviderConfig>({
    provider: "openai",
    model: "whisper-1",
  });

  return (
    <div className="flex flex-col gap-3 px-3 py-3 rounded-md border border-[var(--color-line)] bg-[var(--color-canvas)]">
      <div className="text-xs uppercase tracking-wide text-[var(--color-text-muted)]">
        New override
      </div>
      <Select
        value={language}
        onChange={setLanguage}
        options={available.map((l) => ({
          value: l.value,
          label: languageOptionLabel(l),
        }))}
      />
      <ProviderConfigForm
        value={cfg}
        onChange={setCfg}
        localModels={local.models}
        filterLocalToLanguage={language}
      />
      <div className="flex items-center gap-2">
        <button
          type="button"
          onClick={onCancel}
          className="text-sm px-3 py-1.5 rounded-md border border-[var(--color-line)] hover:bg-[var(--color-pill-hover)]"
        >
          Cancel
        </button>
        <button
          type="button"
          onClick={() => onAdd(language, cfg)}
          className="text-sm px-3 py-1.5 rounded-md bg-[var(--color-text)] text-[var(--color-canvas)]"
        >
          Add
        </button>
      </div>
    </div>
  );
}
```

(`Select` imports from `./Select` — adjust the import path if Select lives at a different relative location; check `LocalModelManager.tsx`'s Select import to mirror.)

- [ ] **Step 2: Rewire `Transcription.tsx`**

Replace the entire Transcription tab body. The new structure has three sections (Default provider, Per-language overrides, Local models) plus the existing Speaker diarization / Audio retention / Vocabulary sections (unchanged).

Open `src/pages/settings/tabs/Transcription.tsx` and replace the props destructure + body. The full new file:

```tsx
import { DiarizeModelManager } from "../components/DiarizeModelManager";
import { LocalModelManager } from "../components/LocalModelManager";
import { PerLanguageOverrides } from "../components/PerLanguageOverrides";
import { ProviderConfigForm } from "../components/ProviderConfigForm";
import { Row, Section } from "../components/Section";
import { useDeveloperMode } from "../../../lib/useDeveloperMode";
import { inputClass } from "../types";
import type { SettingsHook } from "../useSettings";

export function TranscriptionTab({
  s,
  update,
  transcribeConfig,
  setDefaultConfig,
  setLanguageOverride,
  removeLanguageOverride,
  local,
  downloadModel,
  deleteModel,
  diarize,
  downloadDiarize,
  deleteDiarize,
  sortformer,
  downloadSortformer,
  deleteSortformer,
}: Pick<
  SettingsHook,
  | "s"
  | "update"
  | "transcribeConfig"
  | "setDefaultConfig"
  | "setLanguageOverride"
  | "removeLanguageOverride"
  | "local"
  | "downloadModel"
  | "deleteModel"
  | "diarize"
  | "downloadDiarize"
  | "deleteDiarize"
  | "sortformer"
  | "downloadSortformer"
  | "deleteSortformer"
>) {
  const devMode = useDeveloperMode();

  return (
    <>
      <Section title="Default provider">
        <Row label="Active">
          <ProviderConfigForm
            value={transcribeConfig.default}
            onChange={setDefaultConfig}
            localModels={local.models}
          />
          {transcribeConfig.default.provider === "local" &&
            !local.models.some((m) => m.downloaded) && (
              <p className="text-xs text-red-600 dark:text-red-400 mt-2">
                No local model is downloaded. Download one below before recording.
              </p>
            )}
          {transcribeConfig.default.provider === "openai" &&
            transcribeConfig.default.model === "gpt-4o-transcribe-diarize" && (
              <p className="text-xs text-[var(--color-text-muted)] mt-2">
                Note: <code>gpt-4o-transcribe-diarize</code> treats the
                language setting as a hint and does not accept a biasing
                prompt. For strict language locking, use{" "}
                <code>whisper-1</code> or <code>gpt-4o-transcribe</code>.
              </p>
            )}
        </Row>
      </Section>

      <Section title="Per-language overrides">
        <Row label="Overrides">
          <PerLanguageOverrides
            config={transcribeConfig}
            setLanguageOverride={setLanguageOverride}
            removeLanguageOverride={removeLanguageOverride}
            local={local}
          />
        </Row>
      </Section>

      <Section title="Local models">
        <LocalModelManager
          state={local}
          activeId={
            transcribeConfig.default.provider === "local"
              ? transcribeConfig.default.model_id
              : ""
          }
          language={s.language}
          onDownload={downloadModel}
          onDelete={deleteModel}
          onSelect={(id) => {
            // Selecting a local model from the manager pins it as the
            // default's model_id (only meaningful when default is Local).
            // If the user is currently on a non-local default, switch
            // them to Local with this model — matches the v0.23 implicit
            // behaviour of the radio button.
            if (transcribeConfig.default.provider === "local") {
              setDefaultConfig({ ...transcribeConfig.default, model_id: id });
            } else {
              setDefaultConfig({
                provider: "local",
                model_id: id,
                preset: "quality",
                use_gpu: true,
              });
            }
          }}
        />
      </Section>

      <Section title="Speaker diarization">
        {/* …existing body unchanged… */}
      </Section>

      <Section title="Audio retention">
        {/* …existing body unchanged… */}
      </Section>

      <Section title="Vocabulary">
        {/* …existing body unchanged… */}
      </Section>
    </>
  );
}
```

(Copy the bodies of "Speaker diarization", "Audio retention", and "Vocabulary" from the current file unchanged. Don't try to rewrite those sections — they're not part of Phase 4.)

- [ ] **Step 3: Update `Settings.tsx` props**

`src/pages/Settings.tsx` currently passes `providerConfig` / `updateProviderConfig` to TranscriptionTab. Replace with the new props:

```tsx
<TranscriptionTab
  s={settings.s}
  update={settings.update}
  transcribeConfig={settings.transcribeConfig}
  setDefaultConfig={settings.setDefaultConfig}
  setLanguageOverride={settings.setLanguageOverride}
  removeLanguageOverride={settings.removeLanguageOverride}
  local={settings.local}
  downloadModel={settings.downloadModel}
  deleteModel={settings.deleteModel}
  diarize={settings.diarize}
  downloadDiarize={settings.downloadDiarize}
  deleteDiarize={settings.deleteDiarize}
  sortformer={settings.sortformer}
  downloadSortformer={settings.downloadSortformer}
  deleteSortformer={settings.deleteSortformer}
/>
```

- [ ] **Step 4: Compile**

```bash
pnpm tsc -b
```

Expected: clean.

```bash
pnpm build
```

Expected: clean.

- [ ] **Step 5: Commit**

```bash
git add src/pages/settings/components/PerLanguageOverrides.tsx src/pages/settings/tabs/Transcription.tsx src/pages/Settings.tsx
git commit -m "settings: per-language overrides UI; default uses ProviderConfigForm"
```

---

## Task 10: Post-download suggestion for `language_specific` models

**Files:**
- Modify: `src/pages/settings/useSettings.ts` (extend `flashLocal` with action callback)
- Modify: `src/pages/settings/types.ts` (extend `LocalState.flash` shape)
- Modify: `src/pages/settings/components/LocalModelManager.tsx` (render the action when present)

After downloading NB Whisper, the user sees:

> ✓ NB Whisper Large downloaded.
> [Add as Norwegian override?]

Clicking calls `setLanguageOverride("no", { provider: "local", model_id: "nb-whisper-large-q5", preset: "quality", use_gpu: true })`. Dismissing just clears the toast.

- [ ] **Step 1: Extend `LocalState.flash`**

In `src/pages/settings/types.ts`, find `LocalState`:

```ts
export type LocalState = {
  models: LocalWhisperModelStatus[];
  downloading: Record<string, { received: number; total: number | null }>;
  error: string | null;
  flash: string | null;
};
```

Replace `flash: string | null` with:

```ts
flash:
  | null
  | { kind: "info"; message: string }
  | {
      kind: "suggest_language_override";
      message: string;
      language: string;
      modelId: string;
    };
```

Update `EMPTY_LOCAL_STATE` accordingly:

```ts
export const EMPTY_LOCAL_STATE: LocalState = {
  models: [],
  downloading: {},
  error: null,
  flash: null,
};
```

(No change — `null` is already valid.)

- [ ] **Step 2: Extend `flashLocal` in `useSettings.ts`**

Replace the `flashLocal` helper:

```ts
function flashLocal(flash: NonNullable<LocalState["flash"]>) {
  setLocal((p) => ({ ...p, flash }));
  window.setTimeout(() => {
    setLocal((p) => (p.flash === flash ? { ...p, flash: null } : p));
  }, 8000);
}
```

(8 s instead of 4 s — gives the user time to act on the suggestion.)

Update existing `flashLocal("…")` callers in `downloadModel` and `deleteModel` to use the new shape:

```ts
flashLocal({ kind: "info", message: `${label} downloaded` });
```
```ts
flashLocal({ kind: "info", message: before ? `Deleted ${before.label}` : "Whisper model deleted" });
```

- [ ] **Step 3: Surface the override suggestion after download**

In `downloadModel`, after the existing `flashLocal({ kind: "info", … })` call, branch on the model's kind:

```ts
const downloadedInfo = models.find((m) => m.id === modelId);
const label = downloadedInfo?.label ?? modelId;
if (
  downloadedInfo?.kind === "language_specific" &&
  downloadedInfo.specificLanguage &&
  !(downloadedInfo.specificLanguage in transcribeConfig.per_language)
) {
  flashLocal({
    kind: "suggest_language_override",
    message: `${label} downloaded.`,
    language: downloadedInfo.specificLanguage,
    modelId,
  });
} else {
  flashLocal({ kind: "info", message: `${label} downloaded` });
}
```

(Replace the existing `flashLocal(…)` line in `downloadModel` with this branched version.)

- [ ] **Step 4: Render the action button in LocalModelManager**

The flash is displayed somewhere in `LocalModelManager.tsx` (or its parent). Find the line that renders `local.flash` (likely a `<p>` showing the message) and replace with:

```tsx
{local.flash && (
  <div className="flex items-center gap-3 px-3 py-2 rounded-md bg-[var(--color-pill-hover)] text-sm">
    <span>{local.flash.message}</span>
    {local.flash.kind === "suggest_language_override" && (
      <button
        type="button"
        onClick={() => {
          if (local.flash?.kind !== "suggest_language_override") return;
          setLanguageOverride(local.flash.language, {
            provider: "local",
            model_id: local.flash.modelId,
            preset: "quality",
            use_gpu: true,
          });
        }}
        className="ml-auto text-sm px-2 py-1 rounded border border-[var(--color-line)] hover:bg-[var(--color-canvas)]"
      >
        Add as {languageLabel(local.flash.language)} override
      </button>
    )}
  </div>
)}
```

This requires `setLanguageOverride` as a prop on `LocalModelManager`. Add it to the prop type + the call site in Transcription.tsx.

- [ ] **Step 5: Compile**

```bash
pnpm tsc -b
pnpm build
```

Expected: clean.

- [ ] **Step 6: Commit**

```bash
git add src/pages/settings/useSettings.ts src/pages/settings/types.ts src/pages/settings/components/LocalModelManager.tsx src/pages/settings/tabs/Transcription.tsx
git commit -m "settings: post-download suggest 'Add as <language> override?' toast"
```

---

## Task 11: Smoke tests + manual verification

**Files:** none (verification only).

- [ ] **Step 1: Cargo + frontend build sanity**

```bash
cargo test --manifest-path src-tauri/Cargo.toml -- --nocapture
pnpm tsc -b
pnpm build
```

All three must be clean. Expected total cargo tests: 121 (115 from v0.23 + 6 new).

- [ ] **Step 2: Inspect current DB to verify migration plan**

```bash
sqlite3 ~/Library/Application\ Support/no.humla.app/notes.sqlite \
  "SELECT value FROM settings WHERE key = 'transcribe_config';"
```

Expected: a bare `{"provider":"…","model":"…"}` (the v0.23 shape). After v0.24 launches, this becomes `{"default":…,"per_language":{}}`.

- [ ] **Step 3: Optional — dev launch test**

If you want to verify the actual app behaviour before release, run `pnpm tauri dev` and:
1. Open Settings → Transcription. Expect the new Default + Per-language overrides + Local models layout.
2. Add a Norwegian override pointing at OpenAI (or any provider). Confirm it persists across restart.
3. Set the global language to "no" and start a quick recording. Confirm it routes to the override (check the network tab / log output for which API gets hit).
4. Delete the override. Confirm the recording falls back to the default.
5. Download NB Whisper (if not already). Confirm the "Add as Norwegian override?" suggestion appears.

This step is optional; the v0.24.0 release smoke test in Task 13 covers production verification.

- [ ] **Step 4: Mark complete**

No commit — verification only.

---

## Task 12: v0.24.0 release

**Files:**
- Modify: `package.json`, `src-tauri/tauri.conf.json`, `src-tauri/Cargo.toml`

- [ ] **Step 1: Bump versions to 0.24.0**

```bash
grep -E '"version"|^version' package.json src-tauri/tauri.conf.json src-tauri/Cargo.toml
```

Edit each to `0.24.0`. Verify they match.

- [ ] **Step 2: Refresh Cargo.lock**

```bash
cargo check --manifest-path src-tauri/Cargo.toml
```

- [ ] **Step 3: Commit version bump**

```bash
git add package.json src-tauri/tauri.conf.json src-tauri/Cargo.toml src-tauri/Cargo.lock
git commit -m "v0.24.0: per-language transcription routing"
```

- [ ] **Step 4: Run release**

```bash
pnpm release
```

(Same flow as v0.23.0: build → sign → notarise → staple → updater sign → tag push → GitHub release.)

- [ ] **Step 5: Post-release smoke test on the DMG**

Install the released DMG over v0.23.0. Confirm:

1. App launches without TCC re-grant.
2. Migration runs silently — no error toast on first launch. Settings shows the same active provider as v0.23.0 (now in the "Default provider" section).
3. The per-language overrides section is empty.
4. Adding a Norwegian override and recording a Norwegian note transcribes via the override. (Verify by switching the override's provider mid-test and seeing the transcript style change.)

---

## Open questions to resolve at task time

These are smaller design judgements that need a one-line call during execution; none block the plan.

1. **Per-language entry editing.** Currently delete + re-add. If the user's first reaction is "where's the Edit button?", add an inline edit affordance (~30 lines of UI in `OverrideRow`). Watch for it during smoke test; punt unless friction.
2. **Suggestion toast cadence.** 8 s might not be enough to read + click. If feedback says it disappears too fast, bump to 12 s or make it persistent until dismissed.
3. **Override scope leak through to summary.** This plan only routes transcription. The summary provider config (separate setting) doesn't know about the per-language map. If a user wants Norwegian summaries to use a specific local LLM, that's a future feature — out of scope.

## What I'd hold off on

- **Per-note provider override.** Phase 2 § Open Q1. The new schema makes it trivial to extend, but it's a separate feature.
- **Range/group support** ("Norwegian and Swedish both → NB Whisper"). Two entries cover this; revisit only if many users complain.
- **Auto-detect language driving routing**. Adds latency + complexity. Not justified without telemetry.

---

## Self-review

**Spec coverage** (against the design doc § "Phase 4 implementation outline"):

- ✅ Schema + serde (Task 1).
- ✅ `local_whisper` rename + drop `addon_for_language` (Task 3).
- ✅ Resolver + commands threading language (Task 2).
- ✅ DB migration (Task 5).
- ✅ Tauri command rename (Task 4).
- ✅ Settings UI: Default + Per-language overrides + Local models (Tasks 7, 8, 9).
- ✅ `useSettings` refactor (Task 6).
- ✅ Post-download suggestion (Task 10).
- ✅ Release (Task 12).

**Type consistency:**

- `TranscribeConfig` Rust shape matches the `ipc.ts` type field-for-field (`default`, `per_language` with snake_case in JSON; `BTreeMap<String, ProviderConfig>` ↔ `Record<string, ProviderConfig>`).
- `ModelKind` Rust enum (`Multilingual` / `LanguageSpecific`) maps to the IPC string values `"multilingual"` / `"language_specific"`, mirrored in `ipc.ts` `LocalWhisperModelStatus.kind` union.
- `addonLanguage` (old field name) renamed `specificLanguage` in both Rust serde-rename and TS — must rename consistently or one side will silently get null.
- `flash` shape's discriminator field name (`kind`) is the same idiom as `ProviderConfig`'s tagged union — consistent code style.

**Placeholder scan:** no `TODO`, `TBD`, "implement later". Every code step has the actual code. Task 8 explicitly defers the suggestion-button affordance to Task 10 with a forward reference (not a placeholder). Task 9's `Section` bodies for Speaker diarization / Audio retention / Vocabulary explicitly say "copy unchanged from current file" rather than re-listing them — that's a directive, not a placeholder, and it bounds blast radius (don't accidentally rewrite those).

**Risks left to attend at execution:**

- Task 3 renaming touches both backend and frontend. If a TS file references `kind === "primary"` or `addonLanguage` somewhere I didn't grep, the build will surface it. Treat any TS error there as "another reference to rename, not a logic bug."
- Task 10's flash shape change (`string | null` → discriminated union) is a breaking type change for any future caller that destructures `local.flash` directly. Grep for `local.flash` after Task 10 lands; only the LocalModelManager render path should reference it.
- Task 12's release pipeline already has a known requirement for `pnpm release` after explicit user authorization (per Phase 3's pattern). Do not run unprompted.

## Estimated diff

| Task | Lines added | Lines removed | Time |
|---|---|---|---|
| 1: TranscribeConfig + tests | ~140 | 0 | 1h |
| 2: read_transcribe_config + callers | ~30 | ~30 | 1h |
| 3: rename + drop addon_for_language | ~20 | ~30 | 1.5h |
| 4: rename Tauri commands | ~20 | ~10 | 30m |
| 5: migration v4 + tests | ~140 | 0 | 1h |
| 6: useSettings refactor | ~80 | ~30 | 1.5h |
| 7: ProviderConfigForm | ~150 | 0 | 1h |
| 8: LocalModelManager refactor | ~30 | ~20 | 45m |
| 9: PerLanguageOverrides + Transcription rewire | ~250 | ~140 | 2.5h |
| 10: post-download suggestion | ~70 | ~10 | 1h |
| 11: smoke test | 0 | 0 | 30m |
| 12: release | ~5 | 0 | 30m + notarise wait |
| **Total** | **~935** | **~270** | **~12h focused** |
