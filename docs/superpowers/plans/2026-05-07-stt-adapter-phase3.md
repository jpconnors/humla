# STT Adapter Phase 3 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Retire the legacy flat transcription settings keys and their associated shims so `transcribe_config` is the single source of truth, then ship as v0.23.0.

**Architecture:** The Phase 2 release (v0.22.x) already double-wrote both legacy keys (`transcribe_provider`, `transcribe_model`, `whisper_preset`, `local_whisper_model`, `local_whisper_use_gpu`, `deepgram_model`, `groq_model`) AND the typed JSON `transcribe_config`. Phase 3 cuts the legacy reads/writes — a startup migration synthesises `transcribe_config` from legacy keys (covers v0.21 holdouts who never opened Settings under v0.22) then deletes the orphan rows. Frontend reads `ProviderConfig` from the backend on mount and writes through `set_provider_config`. The deprecated `api_key_*` Tauri commands and the `read_openai_api_key` / `set_openai_api_key` shims are deleted; `provider_key_*` and `read_provider_api_key` are the only paths.

**Tech Stack:** Rust 1.85, Tauri 2, async-trait, serde, parking_lot, rusqlite. Frontend: React 19 + TypeScript + Vite (existing settings hook pattern).

**Reference docs:** `docs/design/stt-adapter.md`, `docs/superpowers/plans/2026-05-06-stt-adapter-phase1.md`, `docs/superpowers/plans/2026-05-06-stt-adapter-phase2.md` (Open questions §5 specifically calls out this cleanup).

---

## Background for the implementer

Phase 2 left behind a deliberate transitional shape:

- **Legacy flat keys** (`transcribe_provider` / `transcribe_model` / `whisper_preset` / `local_whisper_model` / `local_whisper_use_gpu` / `deepgram_model` / `groq_model`) are still being **written** by Settings (`useSettings.ts:344–363`) alongside the typed `transcribe_config`.
- **Legacy fallback** in `read_provider_config` (`commands.rs:1493–1509`) reads those keys when the typed JSON is absent — load-bearing for v0.21 users who upgraded without opening Settings.
- **Two prerequisite checks** still read `transcribe_provider` directly: `recording_start` (`commands.rs:1817`) and `local_whisper_use_gpu_setting` (`commands.rs:1444`). They take a different path than `transcribe_chunk`, which already routes through `read_provider_config`.
- **Two key-management surfaces:** `api_key_*` Tauri commands (legacy, OpenAI-only) AND `provider_key_*` (Phase 2). Frontend's `getApiKey/setApiKey/testApiKey` (`ipc.ts:198–200`) wrap the legacy commands; `getProviderKey` etc. wrap the new ones. Backend's `read_openai_api_key` / `set_openai_api_key` (`commands.rs:174–181`) are thin shims over `read_provider_api_key(state, "openai")`.
- **`run_summary`** (`commands.rs:3993`) still calls `read_openai_api_key`; needs to inline through `read_provider_api_key`.

**What we keep**:
- `from_legacy_settings` (`stt/config.rs:84`) — used by the startup migration. Keeps the v0.21 → v0.23 path safe. Could be removed in a future v0.24 once telemetry suggests no v0.21 users remain, but for v0.23 this is the migration's plumbing — load-bearing.
- The Phase 2 keychain cache and `provider_key_*` commands — already the canonical surface.

**Migration risk surface**: a user upgrading **v0.21 → v0.23** (skipping v0.22) only has the legacy keys. The startup migration runs `from_legacy_settings`, writes `transcribe_config`, then deletes legacy rows. After this one-shot, `read_provider_config` will always find typed config or fall back to a hardcoded default (whisper-1 OpenAI), which matches what `from_legacy_settings(None, None, …)` produces.

---

## File Structure

### Modified files

| File | Change |
|---|---|
| `src-tauri/src/db.rs` | Add `migrate_transcribe_config(...)` (idempotent, flag-gated) and a `delete_setting` helper. |
| `src-tauri/src/lib.rs` | Hook the migration into `setup`. Remove `api_key_get/set/test` from `invoke_handler`; add `get_provider_config`. |
| `src-tauri/src/commands.rs` | Delete `read_openai_api_key`, `set_openai_api_key`, and the `api_key_*` Tauri commands. Refactor `local_whisper_use_gpu_setting` to take a `&LocalWhisperConfig`. Refactor `local_model_path` to take `model_id: &str`. Replace the prereq check's direct `transcribe_provider` read with `read_provider_config(&state)`. Drop the legacy fallback inside `read_provider_config` (still defaults via `from_legacy_settings(None, …)` if `transcribe_config` is missing). Inline `run_summary`'s key lookup. Add `get_provider_config` command. |
| `src/lib/ipc.ts` | Drop `transcribe_provider` / `transcribe_model` / `whisper_preset` / `local_whisper_model` / `local_whisper_use_gpu` / `deepgram_model` / `groq_model` from `SettingsKey`. Drop `getApiKey/setApiKey/testApiKey`. Add `getProviderConfig`. |
| `src/pages/settings/types.ts` | Drop those keys' entries from `DEFAULTS`. |
| `src/pages/settings/useSettings.ts` | New `providerConfig` state (loaded via `getProviderConfig`, written via `setProviderConfig`). Drop `buildProviderConfig`, `TRANSCRIBE_KEYS`, the legacy mirror branch in `update`, the `saveKey`/`testKey` openai-only shims (`saveProviderKey("openai")` is already the path the ApiKeys tab uses). The first-load `getApiKey()` becomes `getProviderKey("openai")`. |
| `src/pages/settings/tabs/Transcription.tsx` | Read provider/model fields from `providerConfig`, not `s.<legacy>`. Writes via `updateProviderConfig(cfg)`. |
| `src/pages/settings/tabs/ApiKeys.tsx` | The OpenAI Section uses `saveProviderKey("openai")` / `testProviderKey("openai")` instead of the now-deleted `saveKey` / `testKey` props. |
| `src/pages/settings/components/LocalModelManager.tsx` | The radio `name="local_whisper_model"` is cosmetic — keep as-is (it's a same-group identifier, not a settings key). |
| `package.json`, `src-tauri/tauri.conf.json`, `src-tauri/Cargo.toml` | Bump to `0.23.0`. |

### New files

None. Phase 3 is reductive.

---

## Task 1: Add `delete_setting` + migration helpers in `db.rs`

**Files:**
- Modify: `src-tauri/src/db.rs`

- [ ] **Step 1: Add the `delete_setting` helper**

After `set_setting` (around `db.rs:347`), add:

```rust
pub fn delete_setting(conn: &Connection, key: &str) -> Result<()> {
    let mut stmt = conn.prepare_cached("DELETE FROM settings WHERE key = ?1")?;
    stmt.execute(params![key])?;
    Ok(())
}
```

- [ ] **Step 2: Add the migration function**

After the existing `migrate_summary_prompts` (look for `pub fn migrate_summary_prompts` near `db.rs:432`), add:

```rust
/// One-shot v0.23 migration: ensure `transcribe_config` is present (build
/// from legacy flat keys if missing) and then delete those legacy rows
/// so they can't drift out of sync with the typed config.
///
/// Idempotent — guarded by a flag in the settings table so re-running the
/// app doesn't re-process already-clean state. The flag is the migration's
/// own row (`migrated_transcribe_config_v3 = "true"`); a future schema
/// change that needs to revisit this can use a fresh flag name.
pub fn migrate_transcribe_config(conn: &Connection) -> Result<()> {
    const FLAG: &str = "migrated_transcribe_config_v3";
    if get_setting(conn, FLAG)?.as_deref() == Some("true") {
        return Ok(());
    }

    // If transcribe_config is absent, synthesise it from whatever legacy
    // keys exist. This is the v0.21 → v0.23 path; v0.22 users already
    // have transcribe_config because Settings was double-writing.
    if get_setting(conn, "transcribe_config")?.is_none() {
        let provider = get_setting(conn, "transcribe_provider")?;
        let model = get_setting(conn, "transcribe_model")?;
        let whisper_model = get_setting(conn, "local_whisper_model")?;
        let whisper_preset = get_setting(conn, "whisper_preset")?;
        let whisper_use_gpu = get_setting(conn, "local_whisper_use_gpu")?
            .and_then(|v| match v.as_str() {
                "true" => Some(true),
                "false" => Some(false),
                _ => None,
            });
        let cfg = crate::stt::from_legacy_settings(
            provider.as_deref(),
            model.as_deref(),
            whisper_model.as_deref(),
            whisper_preset.as_deref(),
            whisper_use_gpu,
        );
        let json = serde_json::to_string(&cfg)
            .map_err(|e| anyhow::anyhow!("serialize transcribe_config: {e}"))?;
        set_setting(conn, "transcribe_config", &json)?;
    }

    for key in [
        "transcribe_provider",
        "transcribe_model",
        "whisper_preset",
        "local_whisper_model",
        "local_whisper_use_gpu",
        "deepgram_model",
        "groq_model",
    ] {
        delete_setting(conn, key)?;
    }
    set_setting(conn, FLAG, "true")?;
    Ok(())
}
```

- [ ] **Step 3: Compile**

```bash
cargo build --manifest-path src-tauri/Cargo.toml --message-format=short
```

Expected: clean (the function isn't wired in yet — no warnings beyond the normal unused-while-not-yet-called).

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/db.rs
git commit -m "db: add delete_setting + migrate_transcribe_config helpers"
```

---

## Task 2: Wire the migration into app startup

**Files:**
- Modify: `src-tauri/src/lib.rs`

- [ ] **Step 1: Call the migration alongside `migrate_summary_prompts`**

Find the existing summary-prompts migration block in `src-tauri/src/lib.rs` (around `lib.rs:127–133`):

```rust
// One-shot migration of the legacy single-custom-prompt setting
// into the summary_prompts table. Same flag-guarded shape as the
// diarize cleanup above.
{
    let state: tauri::State<AppState> = app.state();
    let conn = state.db.lock();
    if let Err(e) = db::migrate_summary_prompts(&conn) {
        eprintln!("migrate_summary_prompts: {e}");
    }
}
```

Add the new migration immediately after that block:

```rust
// v0.23 — collapse the legacy flat transcription settings keys into
// the typed `transcribe_config` JSON and delete the orphan rows.
// Same flag-guarded shape as the migrations above. Safe to leave
// shipping forever; the flag check makes it a no-op after first run.
{
    let state: tauri::State<AppState> = app.state();
    let conn = state.db.lock();
    if let Err(e) = db::migrate_transcribe_config(&conn) {
        eprintln!("migrate_transcribe_config: {e}");
    }
}
```

- [ ] **Step 2: Compile + start the app once**

```bash
cargo build --manifest-path src-tauri/Cargo.toml --message-format=short
```

Expected: clean.

```bash
pnpm tauri dev
```

Expected: app opens normally; no panics on startup. Confirm the migration ran by inspecting the SQLite DB while the app is running:

```bash
sqlite3 ~/Library/Application\ Support/no.humla.app/notes.sqlite \
  "SELECT key, value FROM settings WHERE key IN ('migrated_transcribe_config_v3', 'transcribe_config', 'transcribe_provider', 'transcribe_model', 'whisper_preset', 'local_whisper_model', 'local_whisper_use_gpu', 'deepgram_model', 'groq_model');"
```

Expected output:
- `migrated_transcribe_config_v3|true` is present.
- `transcribe_config|{"provider":"…",…}` is present.
- None of the seven legacy keys appear.

Stop the dev app once verified.

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/lib.rs
git commit -m "lib: run transcribe_config migration on startup"
```

---

## Task 3: Inline OpenAI key shims and delete the wrappers

**Files:**
- Modify: `src-tauri/src/commands.rs`

The `read_openai_api_key` / `set_openai_api_key` shims add nothing — they always call `read_provider_api_key(state, "openai")` / `set_provider_api_key(state, "openai", key)`. There are exactly four call sites left (`api_key_get`, `api_key_set`, `api_key_test`, `run_summary`); the first three get deleted in Task 4. This task just inlines the `run_summary` call so we can drop the shims now.

- [ ] **Step 1: Inline the call in `run_summary`**

Find `commands.rs:3993`:

```rust
let openai_api_key = read_openai_api_key(&state)
    .map_err(|e| anyhow::anyhow!("{e}"))?;
```

Replace with:

```rust
let openai_api_key = read_provider_api_key(&state, "openai")
    .map_err(|e| anyhow::anyhow!("{e}"))?;
```

- [ ] **Step 2: Delete the shims**

Delete the two functions at `commands.rs:172–181`:

```rust
/// Phase-1 compatibility shim. Keep existing call sites working; new
/// sites should call `read_provider_api_key` directly.
fn read_openai_api_key(state: &State<AppState>) -> Result<Option<String>, String> {
    read_provider_api_key(state, "openai")
}

/// Phase-1 compatibility shim. Keep existing call sites working.
fn set_openai_api_key(state: &State<AppState>, key: &str) -> Result<(), String> {
    set_provider_api_key(state, "openai", key)
}
```

The build will now fail because `api_key_get/set/test` still reference these names. That's intentional — Task 4 deletes those commands, but to keep this task self-contained we'll temporarily inline their calls too.

- [ ] **Step 3: Inline the `api_key_*` callers (still gets deleted in Task 4 — bridge build)**

Find `commands.rs:1082–1115`:

```rust
#[tauri::command]
pub fn api_key_get(state: State<AppState>) -> Result<Option<String>, String> {
    Ok(read_openai_api_key(&state)?.map(|_| "stored".to_string()))
}

#[tauri::command]
pub fn api_key_set(state: State<AppState>, key: String) -> Result<(), String> {
    set_openai_api_key(&state, &key)
}

#[tauri::command]
pub async fn api_key_test(state: State<'_, AppState>) -> Result<TestResult, String> {
    let key = read_openai_api_key(&state)?.ok_or_else(|| "No API key stored".to_string())?;
    // …rest unchanged…
}
```

Replace each `read_openai_api_key(&state)` with `read_provider_api_key(&state, "openai")` and the single `set_openai_api_key(&state, &key)` with `set_provider_api_key(&state, "openai", &key)`. The bodies otherwise stay identical. (Task 4 deletes them entirely.)

- [ ] **Step 4: Compile**

```bash
cargo build --manifest-path src-tauri/Cargo.toml --message-format=short
```

Expected: clean.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/commands.rs
git commit -m "commands: inline read_openai_api_key/set_openai_api_key shims"
```

---

## Task 4: Delete the deprecated `api_key_*` Tauri commands

**Files:**
- Modify: `src-tauri/src/commands.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src/lib/ipc.ts`

Frontend uses `getApiKey/setApiKey/testApiKey` only via `useSettings.ts`'s OpenAI key load and the `saveKey`/`testKey` shims (which Task 7 deletes). Removing them now means the frontend won't compile until Task 7 lands — but TypeScript will surface the dangling references precisely. We update `ipc.ts` and `useSettings.ts` together here to keep the tree green.

- [ ] **Step 1: Delete the three Tauri commands**

In `src-tauri/src/commands.rs`, delete the entire blocks at `commands.rs:1082–1115` (`api_key_get`, `api_key_set`, `api_key_test`).

The `TestResult` struct used by `api_key_test` is also used by `provider_key_test` — keep it.

- [ ] **Step 2: Remove from the Tauri command registry**

In `src-tauri/src/lib.rs`, remove these three lines (currently `lib.rs:166–168`):

```rust
commands::api_key_get,
commands::api_key_set,
commands::api_key_test,
```

- [ ] **Step 3: Remove the deprecated frontend bindings**

In `src/lib/ipc.ts`, delete lines 198–200:

```ts
getApiKey: () => invoke<string | null>("api_key_get"),
setApiKey: (key: string) => invoke<void>("api_key_set", { key }),
testApiKey: () => invoke<{ ok: boolean; status: number; error: string | null }>("api_key_test"),
```

Also delete the comment block that introduces them (around `ipc.ts:201–202`):

```ts
// Phase-2 generic surface. Use these for new providers; the api_key_*
// shims above stay for compat but resolve to the same Keychain slot.
```

- [ ] **Step 4: Update `useSettings.ts` to load the OpenAI key via `getProviderKey`**

In `src/pages/settings/useSettings.ts:46`, change:

```ts
const [k1, kdg, kgrq, models, ds, ss] = await Promise.all([
  ipc.getApiKey(),
```

to:

```ts
const [k1, kdg, kgrq, models, ds, ss] = await Promise.all([
  ipc.getProviderKey("openai").catch(() => null),
```

Matches how `kdg` / `kgrq` are already loaded (`useSettings.ts:47–48`).

- [ ] **Step 5: Compile both sides**

```bash
cargo build --manifest-path src-tauri/Cargo.toml --message-format=short
```

Expected: clean.

```bash
pnpm tsc -b
```

Expected: clean. (`saveKey` / `testKey` are still defined in `useSettings.ts:429–435`; their bodies are unaffected by this task.)

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/commands.rs src-tauri/src/lib.rs src/lib/ipc.ts src/pages/settings/useSettings.ts
git commit -m "commands: drop deprecated api_key_* Tauri commands and frontend wrappers"
```

---

## Task 5: Refactor `local_model_path` to take an explicit `model_id`

**Files:**
- Modify: `src-tauri/src/commands.rs`

Today `local_model_path` reads `local_whisper_model` directly from settings (`commands.rs:1463`). After Task 2's migration that key is gone — `local_model_path` would fall through to `default_model()` for everyone, breaking users who picked a non-default model. Refactor it to take `model_id: &str` so callers can resolve it from the typed `LocalWhisperConfig`.

- [ ] **Step 1: Change the function signature**

Find `commands.rs:1453–1475`:

```rust
fn local_model_path(app: &AppHandle, language: &str) -> Result<PathBuf, String> {
    let dir = local_model_dir(app)?;
    if let Some(addon) = local_whisper::addon_for_language(language) {
        let p = dir.join(addon.filename);
        if p.exists() {
            return Ok(p);
        }
    }
    let state: State<AppState> = app.state();
    let conn = state.db.lock();
    let id = db::get_setting(&conn, "local_whisper_model")
        .map_err(err)?
        .unwrap_or_default();
    drop(conn);
    let info = local_whisper::find_model(&id)
        .filter(|m| m.kind == local_whisper::ModelKind::Primary)
        .unwrap_or_else(local_whisper::default_model);
    let path = dir.join(info.filename);
    if path.exists() {
        return Ok(path);
    }
    Ok(dir.join(local_whisper::default_model().filename))
}
```

Replace with:

```rust
fn local_model_path(
    app: &AppHandle,
    language: &str,
    model_id: &str,
) -> Result<PathBuf, String> {
    let dir = local_model_dir(app)?;
    if let Some(addon) = local_whisper::addon_for_language(language) {
        let p = dir.join(addon.filename);
        if p.exists() {
            return Ok(p);
        }
    }
    let info = local_whisper::find_model(model_id)
        .filter(|m| m.kind == local_whisper::ModelKind::Primary)
        .unwrap_or_else(local_whisper::default_model);
    let path = dir.join(info.filename);
    if path.exists() {
        return Ok(path);
    }
    Ok(dir.join(local_whisper::default_model().filename))
}
```

- [ ] **Step 2: Update the three callers to pass `model_id`**

There are three call sites — find them with:

```bash
grep -n "local_model_path(" src-tauri/src/commands.rs
```

Expected matches: line ~1835 (`recording_start` prereq), line ~1879 (`recording_start` prewarm), line ~3724 (`transcribe_chunk`). The next two tasks rewrite the surrounding code in each; for now just thread a placeholder `""` into each call so this task compiles. Each caller's surrounding block is rewritten in Task 6 to resolve the model_id from the typed config.

Replace each `local_model_path(&app, &language)` with `local_model_path(&app, &language, "")`. The empty string falls through to `default_model()` via `find_model`, matching today's behaviour when `local_whisper_model` is unset — same fall-back semantics.

- [ ] **Step 3: Compile**

```bash
cargo build --manifest-path src-tauri/Cargo.toml --message-format=short
```

Expected: clean.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/commands.rs
git commit -m "commands: refactor local_model_path to take model_id parameter"
```

---

## Task 6: Route prereq check + transcribe_chunk + prewarm through `read_provider_config`

**Files:**
- Modify: `src-tauri/src/commands.rs`

The prereq check at `commands.rs:1815–1828` reads `transcribe_provider` directly from the settings table; after Task 2's migration that key is gone. Same for the prewarm at `commands.rs:1880` (`local_whisper_use_gpu_setting`) and the `transcribe_chunk` use_gpu read at `commands.rs:3728`. Route all three through `read_provider_config`.

- [ ] **Step 1: Replace the prereq-check provider lookup**

Find `commands.rs:1811–1888` — the entire block from "Pre-check the configured provider's prerequisites" through the prewarm `tokio::spawn`. The current shape is:

```rust
let (provider, language) = {
    let conn = state.db.lock();
    let p = db::get_setting(&conn, "transcribe_provider")
        .map_err(err)?
        .unwrap_or_else(|| DEFAULT_TRANSCRIBE_PROVIDER.to_string());
    let global = db::get_setting(&conn, "language")
        .map_err(err)?
        .unwrap_or_else(|| DEFAULT_LANGUAGE.to_string());
    let note_lang = db::get_note(&conn, &note_id)
        .map(|n| n.language)
        .unwrap_or_default();
    let l = if note_lang.trim().is_empty() { global } else { note_lang };
    (p, l)
};
let pre_err: Option<String> = match provider.as_str() {
    "local" => {
        let p = local_model_path(&app, &language).map_err(|e| e.to_string())?;
        // …unchanged body…
    }
    other => {
        let provider_id = crate::stt::keychain_account_for(other)
            .and_then(|_| match other {
                "openai" => Some("openai"),
                "deepgram" => Some("deepgram"),
                "groq" => Some("groq"),
                _ => None,
            })
            .unwrap_or("openai");
        // …unchanged body…
    }
};
```

Replace the whole `let (provider, language) = …` block + the `match` arms + the prewarm block with:

```rust
let provider_cfg = read_provider_config(&state)
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
let pre_err: Option<String> = match &provider_cfg {
    crate::stt::ProviderConfig::Local(local_cfg) => {
        let p = local_model_path(&app, &language, &local_cfg.model_id)
            .map_err(|e| e.to_string())?;
        if p.exists() {
            None
        } else {
            Some(
                "Local Whisper model not downloaded. Download it in Settings → Transcription."
                    .to_string(),
            )
        }
    }
    other => {
        let provider_id = other.provider_id();
        if read_provider_api_key(&state, provider_id)?.is_none() {
            let label = match provider_id {
                "openai" => "OpenAI",
                "deepgram" => "Deepgram",
                "groq" => "Groq",
                _ => "the selected provider",
            };
            Some(format!(
                "{label} API key not set. Add one in Settings → API keys."
            ))
        } else {
            None
        }
    }
};
if let Some(ref msg) = pre_err {
    emit_error(&app, Some(&note_id), msg);
    return Err(msg.clone());
}

// Race a Whisper model load against the sidecar startup so the first
// chunk doesn't pay the cold-start tax (~1–2 s on Apple Silicon). Fire
// and forget — by the time VAD rotates the first chunk, the model is
// already in Metal memory and inference is fast.
if let crate::stt::ProviderConfig::Local(local_cfg) = &provider_cfg {
    let model_id = local_cfg.model_id.clone();
    let use_gpu = local_cfg.use_gpu;
    if let Ok(model_path) = local_model_path(&app, &language, &model_id) {
        let shared = state.whisper.clone();
        tokio::spawn(async move {
            if let Err(e) = local_whisper::prewarm(shared, model_path, use_gpu).await {
                eprintln!("whisper prewarm: {e}");
            }
        });
    }
}
```

This consolidates four scattered settings reads (`transcribe_provider`, `local_whisper_model`, `local_whisper_use_gpu`, plus the keychain id derivation) into one `read_provider_config` call.

- [ ] **Step 2: Update `transcribe_chunk` to use `LocalWhisperConfig.use_gpu` directly**

Find `commands.rs:3722–3733` (the `local_deps` resolution):

```rust
let local_deps = if matches!(provider_cfg, crate::stt::ProviderConfig::Local(_)) {
    let model_path = local_model_path(&app, &language)
        .map_err(|e| anyhow::anyhow!(e))?;
    let (shared, use_gpu) = {
        let state: State<AppState> = app.state();
        (state.whisper.clone(), local_whisper_use_gpu_setting(&state))
    };
    Some(crate::stt::LocalDeps { shared, model_path, use_gpu })
} else {
    None
};
```

Replace with:

```rust
let local_deps = if let crate::stt::ProviderConfig::Local(local_cfg) = &provider_cfg {
    let model_path = local_model_path(&app, &language, &local_cfg.model_id)
        .map_err(|e| anyhow::anyhow!(e))?;
    let shared = {
        let state: State<AppState> = app.state();
        state.whisper.clone()
    };
    Some(crate::stt::LocalDeps {
        shared,
        model_path,
        use_gpu: local_cfg.use_gpu,
    })
} else {
    None
};
```

- [ ] **Step 3: Delete `local_whisper_use_gpu_setting`**

It's now unused. Find `commands.rs:1444–1451` and delete the function:

```rust
fn local_whisper_use_gpu_setting(state: &State<AppState>) -> bool {
    let conn = state.db.lock();
    db::get_setting(&conn, "local_whisper_use_gpu")
        .ok()
        .flatten()
        .unwrap_or_else(|| DEFAULT_LOCAL_WHISPER_USE_GPU.to_string())
        != "false"
}
```

Also delete the `DEFAULT_LOCAL_WHISPER_USE_GPU` constant at `commands.rs:61` — no other reference (verify with `grep -n DEFAULT_LOCAL_WHISPER_USE_GPU src-tauri/src/commands.rs`; should be empty after deletion).

While here, also delete the now-unused legacy default constants `DEFAULT_TRANSCRIBE_PROVIDER` (`commands.rs:19`) and `DEFAULT_TRANSCRIBE_MODEL` (`commands.rs:20`) — they're only referenced by the prereq block we just replaced. Verify with grep first:

```bash
grep -n "DEFAULT_TRANSCRIBE_PROVIDER\|DEFAULT_TRANSCRIBE_MODEL\|DEFAULT_WHISPER_PRESET\|DEFAULT_LOCAL_WHISPER_USE_GPU" src-tauri/src/commands.rs
```

Expected: no matches. Delete the constants.

- [ ] **Step 4: Compile**

```bash
cargo build --manifest-path src-tauri/Cargo.toml --message-format=short
```

Expected: clean.

- [ ] **Step 5: Run cargo tests**

```bash
cargo test --manifest-path src-tauri/Cargo.toml --message-format=short -- --nocapture
```

Expected: all tests pass.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/commands.rs
git commit -m "commands: route prereq + prewarm + transcribe_chunk through read_provider_config"
```

---

## Task 7: Drop legacy fallback inside `read_provider_config` + add `get_provider_config` Tauri command

**Files:**
- Modify: `src-tauri/src/commands.rs`
- Modify: `src-tauri/src/lib.rs`

After Tasks 1–2 the legacy keys are deleted at startup; the `read_provider_config` legacy fallback is now dead code. Replace it with a hardcoded default. Also add `get_provider_config` so the Settings UI can load the typed config on mount.

- [ ] **Step 1: Simplify `read_provider_config`**

Find `commands.rs:1484–1510` (the function as it stands after Phase 2). Replace it with:

```rust
/// Read the active STT provider config from the typed `transcribe_config`
/// JSON. Falls back to a hardcoded OpenAI / whisper-1 default when the
/// row is absent or corrupt — same shape that `from_legacy_settings`
/// produces with all-None inputs, so behaviour matches a fresh install.
fn read_provider_config(state: &State<AppState>) -> anyhow::Result<crate::stt::ProviderConfig> {
    let conn = state.db.lock();
    if let Some(json) = db::get_setting(&conn, "transcribe_config")? {
        if let Ok(cfg) = serde_json::from_str::<crate::stt::ProviderConfig>(&json) {
            return Ok(cfg);
        }
        // Corrupted JSON — fall through to the default rather than locking
        // the user out over a malformed cache. Settings UI will overwrite
        // it when the user opens the Transcription tab.
    }
    Ok(crate::stt::from_legacy_settings(None, None, None, None, None))
}
```

- [ ] **Step 2: Add the `get_provider_config` Tauri command**

Insert near the existing `set_provider_config` command (around `commands.rs:1118`):

```rust
/// Read the active STT provider config. The Settings UI uses this on
/// mount as the single source of truth instead of reading the legacy
/// flat keys (which v0.23 deleted).
#[tauri::command]
pub fn get_provider_config(
    state: State<AppState>,
) -> Result<crate::stt::ProviderConfig, String> {
    read_provider_config(&state).map_err(|e| e.to_string())
}
```

- [ ] **Step 3: Register the new command in `lib.rs`**

In `src-tauri/src/lib.rs`, find the `commands::set_provider_config` line in `invoke_handler` (around `lib.rs:169`) and add `get_provider_config` directly above it:

```rust
commands::get_provider_config,
commands::set_provider_config,
```

- [ ] **Step 4: Compile**

```bash
cargo build --manifest-path src-tauri/Cargo.toml --message-format=short
```

Expected: clean.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/commands.rs src-tauri/src/lib.rs
git commit -m "commands: simplify read_provider_config; add get_provider_config command"
```

---

## Task 8: Frontend IPC — drop legacy `SettingsKey` entries, add `getProviderConfig`

**Files:**
- Modify: `src/lib/ipc.ts`
- Modify: `src/pages/settings/types.ts`

- [ ] **Step 1: Drop legacy entries from `SettingsKey`**

In `src/lib/ipc.ts:36–60`, remove these lines from the `SettingsKey` union:

```ts
| "transcribe_provider"
| "transcribe_model"
| "whisper_preset"
| "local_whisper_model"
| "local_whisper_use_gpu"
| "deepgram_model"
| "groq_model"
```

Remaining `SettingsKey` should look like:

```ts
export type SettingsKey =
  | "language"
  | "default_summary_preset"
  | "diarize_model"
  | "community1_threshold"
  | "sortformer_silence_threshold"
  | "sortformer_pred_threshold"
  | "keep_audio"
  | "custom_vocabulary"
  | "summary_model"
  | "summary_prompt"
  | "summary_provider"
  | "local_llm_base_url"
  | "local_llm_model"
  | "local_llm_think"
  | "theme"
  | "developer_mode"
  | "silence_rms_threshold";
```

- [ ] **Step 2: Add `getProviderConfig`**

In `src/lib/ipc.ts`, find the `setProviderConfig` line (around `ipc.ts:211–212`) and add `getProviderConfig` next to it:

```ts
getProviderConfig: () => invoke<ProviderConfig>("get_provider_config"),
setProviderConfig: (config: ProviderConfig) =>
  invoke<void>("set_provider_config", { config }),
```

- [ ] **Step 3: Drop legacy entries from `DEFAULTS`**

In `src/pages/settings/types.ts:16–40`, remove these entries:

```ts
transcribe_provider: "openai",
transcribe_model: "whisper-1",
whisper_preset: "quality",
local_whisper_model: "large-v3-turbo-q5",
local_whisper_use_gpu: "true",
deepgram_model: "nova-3",
groq_model: "whisper-large-v3-turbo",
```

The remaining `DEFAULTS` should match the slimmed `EditableKey` union (TypeScript will surface any drift).

- [ ] **Step 4: Compile**

```bash
pnpm tsc -b
```

Expected: errors in `useSettings.ts` and `Transcription.tsx` referencing the dropped keys. Tasks 9 and 10 fix those.

- [ ] **Step 5: Commit**

```bash
git add src/lib/ipc.ts src/pages/settings/types.ts
git commit -m "ipc: drop legacy SettingsKey entries; add getProviderConfig"
```

(Yes, this commit leaves the tree red — Tasks 9 and 10 are fixups. We commit anyway to keep blast radius bounded; it's fine because all three commits land together before any test/dev run. If you'd rather batch, hold the commit until Task 10.)

---

## Task 9: Frontend `useSettings` — `providerConfig` as state; drop legacy mirror

**Files:**
- Modify: `src/pages/settings/useSettings.ts`

- [ ] **Step 1: Add `providerConfig` state and load it**

At the top of the `useSettings` hook (around `useSettings.ts:27–40`), add a new piece of state:

```ts
const [providerConfig, setProviderConfig] = useState<ProviderConfig>({
  provider: "openai",
  model: "whisper-1",
});
```

Then in the first `useEffect` block (the one that loads initial state from the backend, `useSettings.ts:42–71`), expand the `Promise.all` to also fetch `getProviderConfig`:

```ts
const [k1, kdg, kgrq, models, ds, ss, cfg] = await Promise.all([
  ipc.getProviderKey("openai").catch(() => null),
  ipc.getProviderKey("deepgram").catch(() => null),
  ipc.getProviderKey("groq").catch(() => null),
  ipc.localWhisperModels(),
  ipc.diarizeStatus("community1").catch(() => null),
  ipc.diarizeStatus("sortformer").catch(() => null),
  ipc.getProviderConfig().catch(() => null),
]);
if (cancelled) return;
setOpenaiKey((p) => ({ ...p, hasKey: !!k1 }));
setDeepgramKey((p) => ({ ...p, hasKey: !!kdg }));
setGroqKey((p) => ({ ...p, hasKey: !!kgrq }));
setLocal((p) => ({ ...p, models }));
setDiarize((p) => ({ ...p, status: ds }));
setSortformer((p) => ({ ...p, status: ss }));
if (cfg) setProviderConfig(cfg);
```

- [ ] **Step 2: Add `updateProviderConfig`**

After the existing `update` function (around `useSettings.ts:344–363`), add:

```ts
async function updateProviderConfig(cfg: ProviderConfig) {
  setProviderConfig(cfg);
  try {
    await ipc.setProviderConfig(cfg);
  } catch (e) {
    console.warn("[settings] setProviderConfig failed:", e);
  }
}
```

- [ ] **Step 3: Strip the legacy-mirror branch out of `update`**

In `update` (`useSettings.ts:344–363`), remove the `if (TRANSCRIBE_KEYS.has(key)) { … }` block. The function becomes:

```ts
async function update(key: EditableKey, value: string) {
  setS((prev) => ({ ...prev, [key]: value }));
  await ipc.setSetting(key, value);
}
```

- [ ] **Step 4: Delete `buildProviderConfig` and `TRANSCRIBE_KEYS`**

Delete the helper at `useSettings.ts:367–386` (`function buildProviderConfig(...)`).

Delete the constant at the bottom of the file (`useSettings.ts:464–475`):

```ts
const TRANSCRIBE_KEYS = new Set<EditableKey>([
  "transcribe_provider",
  "transcribe_model",
  "whisper_preset",
  "local_whisper_model",
  "local_whisper_use_gpu",
  "deepgram_model",
  "groq_model",
]);
```

Also delete the `// Settings keys that, when changed…` comment block above it.

- [ ] **Step 5: Update `downloadModel` and `deleteModel` to set the active model via `updateProviderConfig`**

These two functions still call `update("local_whisper_model", …)`. After dropping that key from `SettingsKey`, those calls won't typecheck.

In `downloadModel` (around `useSettings.ts:194–202`), find:

```ts
if (
  downloadedInfo?.kind === "primary" &&
  models.filter((m) => m.kind === "primary" && m.downloaded).length === 1
) {
  await update("local_whisper_model", modelId);
}
```

Replace with:

```ts
if (
  downloadedInfo?.kind === "primary" &&
  models.filter((m) => m.kind === "primary" && m.downloaded).length === 1 &&
  providerConfig.provider === "local"
) {
  await updateProviderConfig({ ...providerConfig, model_id: modelId });
}
```

In `deleteModel` (around `useSettings.ts:222–228`), find:

```ts
if (s.local_whisper_model === modelId) {
  const fallback =
    models.find((m) => m.kind === "primary" && m.downloaded)?.id ??
    DEFAULTS.local_whisper_model;
  await update("local_whisper_model", fallback);
}
```

Replace with:

```ts
if (
  providerConfig.provider === "local" &&
  providerConfig.model_id === modelId
) {
  const fallback =
    models.find((m) => m.kind === "primary" && m.downloaded)?.id ??
    "large-v3-turbo-q5";
  await updateProviderConfig({ ...providerConfig, model_id: fallback });
}
```

(The hardcoded `"large-v3-turbo-q5"` matches the historic `DEFAULTS.local_whisper_model` value. We don't keep a constant for it because it has exactly one caller now.)

- [ ] **Step 6: Delete `saveKey` / `testKey` shims and stop returning them**

Delete the two compat shims at `useSettings.ts:427–435`:

```ts
async function saveKey() {
  await saveProviderKey("openai");
}

async function testKey() {
  await testProviderKey("openai");
}
```

In the `return { ... }` block (`useSettings.ts:437–461`), remove the `saveKey,` and `testKey,` lines.

Also add `providerConfig` and `updateProviderConfig` to the returned object:

```ts
return {
  s,
  update,
  providerConfig,
  updateProviderConfig,
  openaiKey,
  setOpenaiKey,
  deepgramKey,
  setDeepgramKey,
  groqKey,
  setGroqKey,
  saveProviderKey,
  testProviderKey,
  local,
  downloadModel,
  deleteModel,
  diarize,
  downloadDiarize,
  deleteDiarize,
  sortformer,
  downloadSortformer,
  deleteSortformer,
  llmModels,
  refreshLlmModels,
};
```

- [ ] **Step 7: Compile**

```bash
pnpm tsc -b
```

Expected: errors only in `Transcription.tsx` and `ApiKeys.tsx` — Tasks 10 and 11 fix those.

- [ ] **Step 8: Commit**

```bash
git add src/pages/settings/useSettings.ts
git commit -m "settings: providerConfig as first-class state; drop legacy mirror"
```

---

## Task 10: Frontend `Transcription.tsx` — read/write via `providerConfig`

**Files:**
- Modify: `src/pages/settings/tabs/Transcription.tsx`

- [ ] **Step 1: Take `providerConfig` and `updateProviderConfig` from props**

At the top of `TranscriptionTab` (around `Transcription.tsx:18–43`), add the two props:

```tsx
export function TranscriptionTab({
  s,
  update,
  providerConfig,
  updateProviderConfig,
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
  | "providerConfig"
  | "updateProviderConfig"
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
```

Then replace the `const provider: Provider = (s.transcribe_provider as Provider) ?? "openai";` line (`Transcription.tsx:44`) with:

```tsx
const provider = providerConfig.provider;
```

- [ ] **Step 2: Rewrite the provider radio onChange**

Find the `<Select value={provider} onChange={…}>` block (`Transcription.tsx:51–58`):

```tsx
<Select
  value={provider}
  onChange={(v) => update("transcribe_provider", v)}
  options={
    local.models.some((m) => m.downloaded)
      ? [...PROVIDERS_BASE, LOCAL_PROVIDER]
      : PROVIDERS_BASE
  }
/>
```

Replace with:

```tsx
<Select
  value={provider}
  onChange={(v) => {
    const p = v as Provider;
    if (p === "openai") {
      updateProviderConfig({ provider: "openai", model: "whisper-1" });
    } else if (p === "local") {
      updateProviderConfig({
        provider: "local",
        model_id:
          local.models.find((m) => m.kind === "primary" && m.downloaded)?.id ??
          "large-v3-turbo-q5",
        preset: "quality",
        use_gpu: true,
      });
    } else if (p === "deepgram") {
      updateProviderConfig({ provider: "deepgram", model: "nova-3" });
    } else if (p === "groq") {
      updateProviderConfig({ provider: "groq", model: "whisper-large-v3-turbo" });
    }
  }}
  options={
    local.models.some((m) => m.downloaded)
      ? [...PROVIDERS_BASE, LOCAL_PROVIDER]
      : PROVIDERS_BASE
  }
/>
```

(When the user switches providers, we reset to a sensible default model rather than carrying over the prior provider's model. The Phase 2 mirror used to leave both the prior and new model in `s.*`; carrying over made no sense for cross-provider switches and the typed config makes it impossible by construction.)

- [ ] **Step 3: Rewrite the per-provider Model rows**

Find the OpenAI Model block (`Transcription.tsx:66–82`):

```tsx
{provider === "openai" && (
  <Row label="Model">
    <Select
      value={s.transcribe_model}
      onChange={(v) => update("transcribe_model", v)}
      options={TRANSCRIBE_MODELS.map((m) => ({ value: m, label: m }))}
    />
    {s.transcribe_model === "gpt-4o-transcribe-diarize" && (
      …
    )}
  </Row>
)}
```

Replace with:

```tsx
{providerConfig.provider === "openai" && (
  <Row label="Model">
    <Select
      value={providerConfig.model}
      onChange={(v) =>
        updateProviderConfig({ provider: "openai", model: v })
      }
      options={TRANSCRIBE_MODELS.map((m) => ({ value: m, label: m }))}
    />
    {providerConfig.model === "gpt-4o-transcribe-diarize" && (
      <p className="text-xs text-[var(--color-text-muted)] mt-2">
        Note: <code>gpt-4o-transcribe-diarize</code> treats the
        language setting as a hint and does not accept a biasing
        prompt. For strict language locking, use{" "}
        <code>whisper-1</code> or <code>gpt-4o-transcribe</code>.
      </p>
    )}
  </Row>
)}
```

Find the Deepgram Model block (`Transcription.tsx:83–98`) and replace with:

```tsx
{providerConfig.provider === "deepgram" && (
  <Row label="Model">
    <Select
      value={providerConfig.model}
      onChange={(v) =>
        updateProviderConfig({ provider: "deepgram", model: v })
      }
      options={DEEPGRAM_MODELS.map((m) => ({ value: m, label: m }))}
    />
    <p className="text-xs text-[var(--color-text-muted)] mt-2">
      <code>nova-3</code> is the current best for English; falls
      back gracefully to other languages. Word timestamps and
      vocabulary biasing (via <code>keywords</code> param) work on
      every model. Add your Deepgram API key under Settings → API
      keys.
    </p>
  </Row>
)}
```

Find the Groq Model block (`Transcription.tsx:99–113`) and replace with:

```tsx
{providerConfig.provider === "groq" && (
  <Row label="Model">
    <Select
      value={providerConfig.model}
      onChange={(v) =>
        updateProviderConfig({ provider: "groq", model: v })
      }
      options={GROQ_MODELS.map((m) => ({ value: m, label: m }))}
    />
    <p className="text-xs text-[var(--color-text-muted)] mt-2">
      Groq hosts <code>whisper-large-v3-turbo</code> at OpenAI-
      compatible endpoints — same Whisper quality, ~10× cheaper
      and faster than OpenAI's hosted Whisper. Add your Groq API
      key under Settings → API keys.
    </p>
  </Row>
)}
```

- [ ] **Step 4: Rewrite the local-provider rows**

Find the local Section block (`Transcription.tsx:116–162`). Replace `provider === "local"` with `providerConfig.provider === "local"` everywhere, and route the preset / GPU / model writes through `updateProviderConfig`. The whole block becomes:

```tsx
{providerConfig.provider === "local" && (
  <Section title="Local model behaviour">
    <Row label="Quality preset">
      <Select
        value={providerConfig.preset}
        onChange={(v) =>
          updateProviderConfig({ ...providerConfig, preset: v })
        }
        options={WHISPER_PRESETS}
      />
      <p className="text-xs text-[var(--color-text-muted)] mt-2">
        Trades latency for accuracy. Quality runs beam search with
        an aggressive no-speech threshold so almost no segments are
        silently dropped — best for meetings and dense speech. Fast
        falls back to greedy decoding for live-caption snappiness.
      </p>
    </Row>
    <Row label="GPU acceleration">
      <label className="flex items-center gap-2 cursor-pointer text-sm">
        <input
          type="checkbox"
          checked={providerConfig.use_gpu}
          onChange={(e) =>
            updateProviderConfig({
              ...providerConfig,
              use_gpu: e.target.checked,
            })
          }
        />
        Use Metal (Apple GPU) for Whisper inference
      </label>
      <p className="text-xs text-[var(--color-text-muted)] mt-2">
        On by default — gives ~10× speedup over CPU on Apple
        Silicon. Turn off if Whisper logs Metal compile errors
        like <code>ggml_backend_metal_init: failed to allocate
        context</code>; the app falls back to CPU/BLAS, which is
        slower but reliable.
      </p>
    </Row>
  </Section>
)}

<Section title="Local models">
  <LocalModelManager
    state={local}
    activeId={
      providerConfig.provider === "local" ? providerConfig.model_id : ""
    }
    language={s.language}
    onDownload={downloadModel}
    onDelete={deleteModel}
    onSelect={(id) => {
      if (providerConfig.provider === "local") {
        updateProviderConfig({ ...providerConfig, model_id: id });
      } else {
        updateProviderConfig({
          provider: "local",
          model_id: id,
          preset: "quality",
          use_gpu: true,
        });
      }
    }}
  />
</Section>
```

(Selecting a local model from the manager when the user is currently on a non-local provider switches them to local — matches the historical behaviour where `update("local_whisper_model", id)` would write while the user is on OpenAI; the next recording would still pick OpenAI but the local list would show the chosen radio. Now it explicitly switches them, which is more intentional.)

- [ ] **Step 5: Compile**

```bash
pnpm tsc -b
```

Expected: only errors in `ApiKeys.tsx` (Task 11). All `Transcription.tsx` errors resolved.

- [ ] **Step 6: Commit**

```bash
git add src/pages/settings/tabs/Transcription.tsx
git commit -m "settings: Transcription tab reads/writes via providerConfig"
```

---

## Task 11: Frontend `ApiKeys.tsx` — drop the OpenAI saveKey/testKey props

**Files:**
- Modify: `src/pages/settings/tabs/ApiKeys.tsx`
- Modify: `src/pages/settings/SettingsPage.tsx` (or wherever the tabs are wired)

`ApiKeys.tsx` still takes `saveKey` and `testKey` props (the openai shims we deleted from `useSettings.ts`). Convert it to use `saveProviderKey("openai")` / `testProviderKey("openai")` like Deepgram and Groq do already.

- [ ] **Step 1: Update `ApiKeys.tsx`**

Find `src/pages/settings/tabs/ApiKeys.tsx`. Replace the props destructure and the OpenAI section:

```tsx
import { ApiKeyField } from "../components/ApiKeyField";
import { Row, Section } from "../components/Section";
import type { SettingsHook } from "../useSettings";

export function ApiKeysTab({
  openaiKey,
  setOpenaiKey,
  deepgramKey,
  setDeepgramKey,
  groqKey,
  setGroqKey,
  saveProviderKey,
  testProviderKey,
}: Pick<
  SettingsHook,
  | "openaiKey"
  | "setOpenaiKey"
  | "deepgramKey"
  | "setDeepgramKey"
  | "groqKey"
  | "setGroqKey"
  | "saveProviderKey"
  | "testProviderKey"
>) {
  return (
    <>
      <Section title="OpenAI">
        <Row label="API key">
          <ApiKeyField
            state={openaiKey}
            setState={setOpenaiKey}
            placeholder="sk-…"
            onSave={() => saveProviderKey("openai")}
            onTest={() => testProviderKey("openai")}
          />
          <p className="text-xs text-[var(--color-text-muted)] mt-2">
            Used for cloud transcription (Whisper / gpt-4o-transcribe) and
            cloud summarization when those providers are selected. Stored
            locally in the macOS Keychain; not sent anywhere except OpenAI.
          </p>
        </Row>
      </Section>

      <Section title="Deepgram">
        <Row label="API key">
          <ApiKeyField
            state={deepgramKey}
            setState={setDeepgramKey}
            placeholder="dg-…"
            onSave={() => saveProviderKey("deepgram")}
            onTest={() => testProviderKey("deepgram")}
          />
          <p className="text-xs text-[var(--color-text-muted)] mt-2">
            Required when Transcription provider is set to Deepgram.
            Stored in the macOS Keychain; sent only to api.deepgram.com.
          </p>
        </Row>
      </Section>

      <Section title="Groq">
        <Row label="API key">
          <ApiKeyField
            state={groqKey}
            setState={setGroqKey}
            placeholder="gsk_…"
            onSave={() => saveProviderKey("groq")}
            onTest={() => testProviderKey("groq")}
          />
          <p className="text-xs text-[var(--color-text-muted)] mt-2">
            Required when Transcription provider is set to Groq. Hosts
            <code> whisper-large-v3-turbo </code> at roughly 10× the speed
            of OpenAI Whisper. Stored in the macOS Keychain; sent only to
            api.groq.com.
          </p>
        </Row>
      </Section>
    </>
  );
}
```

- [ ] **Step 2: Update SettingsPage to drop the now-deleted props**

Find where `ApiKeysTab` is rendered (likely in `src/pages/settings/SettingsPage.tsx` or similar — confirm with):

```bash
grep -rn "ApiKeysTab" src/pages/settings
```

Open the file. The render call probably spreads `settingsHook` or passes individual props including `saveKey` and `testKey`. Remove those two props (TypeScript compile will guide you if you miss any).

Similarly, find and update where `TranscriptionTab` is rendered — add `providerConfig` and `updateProviderConfig` to its props:

```bash
grep -rn "TranscriptionTab" src/pages/settings
```

The render call likely already uses the spread pattern, so adding the two props to `useSettings.ts`'s return (Task 9) means they flow through automatically. If the parent destructures explicitly, add the two names to the destructure and the JSX call.

- [ ] **Step 3: Compile**

```bash
pnpm tsc -b
```

Expected: clean.

- [ ] **Step 4: Commit**

```bash
git add src/pages/settings/tabs/ApiKeys.tsx src/pages/settings/SettingsPage.tsx
git commit -m "settings: ApiKeys tab uses saveProviderKey/testProviderKey directly"
```

(Adjust the file list in `git add` based on what `grep` actually surfaced.)

---

## Task 12: End-to-end smoke test

**Files:** none (manual verification).

- [ ] **Step 1: Reset to a known-clean state**

Backup your DB:

```bash
cp ~/Library/Application\ Support/no.humla.app/notes.sqlite \
   ~/Library/Application\ Support/no.humla.app/notes.sqlite.phase3-backup
```

- [ ] **Step 2: Build dev**

```bash
pnpm tauri dev
```

Confirm:
1. App launches without panics or migration errors.
2. Settings → Transcription opens with the current provider/model selected.
3. Switching to Deepgram → record 15 s → transcript appears with Deepgram-attributed text (assumes Deepgram key is stored).
4. Switching to Groq → record 15 s → transcript appears.
5. Switching to OpenAI → record 15 s → transcript appears.
6. Switching to Local Whisper (if a model is downloaded) → record 15 s → transcript appears.
7. Settings → API keys → all three providers show "stored ✓" or empty correctly.

- [ ] **Step 3: Verify the migration ran cleanly**

```bash
sqlite3 ~/Library/Application\ Support/no.humla.app/notes.sqlite \
  "SELECT key FROM settings WHERE key IN ('transcribe_provider', 'transcribe_model', 'whisper_preset', 'local_whisper_model', 'local_whisper_use_gpu', 'deepgram_model', 'groq_model');"
```

Expected: empty.

```bash
sqlite3 ~/Library/Application\ Support/no.humla.app/notes.sqlite \
  "SELECT value FROM settings WHERE key = 'transcribe_config';"
```

Expected: a JSON blob matching your active provider.

- [ ] **Step 4: v0.21 → v0.23 simulated upgrade test**

Stop the app. Reset to a v0.21-shaped DB:

```bash
sqlite3 ~/Library/Application\ Support/no.humla.app/notes.sqlite \
  "DELETE FROM settings WHERE key IN ('transcribe_config', 'migrated_transcribe_config_v3');" \
  "INSERT INTO settings (key, value) VALUES ('transcribe_provider', 'local'), ('local_whisper_model', 'large-v3-turbo-q5'), ('whisper_preset', 'balanced'), ('local_whisper_use_gpu', 'false');"
```

(That mimics a v0.21 user who set Local with the balanced preset and GPU off.)

```bash
pnpm tauri dev
```

After launch, confirm the migration ran:

```bash
sqlite3 ~/Library/Application\ Support/no.humla.app/notes.sqlite \
  "SELECT value FROM settings WHERE key = 'transcribe_config';"
```

Expected: `{"provider":"local","model_id":"large-v3-turbo-q5","preset":"balanced","use_gpu":false}` (key order may differ).

```bash
sqlite3 ~/Library/Application\ Support/no.humla.app/notes.sqlite \
  "SELECT key FROM settings WHERE key LIKE 'transcribe_%' OR key LIKE 'whisper_%' OR key LIKE 'local_whisper_%' OR key IN ('deepgram_model','groq_model');"
```

Expected: `transcribe_config` and `migrated_transcribe_config_v3` only.

Confirm the app's Settings → Transcription shows Local / large-v3-turbo-q5 / Balanced / GPU off.

Restore your backup:

```bash
mv ~/Library/Application\ Support/no.humla.app/notes.sqlite.phase3-backup \
   ~/Library/Application\ Support/no.humla.app/notes.sqlite
```

- [ ] **Step 5: Commit nothing** — this task is verification only.

---

## Task 13: v0.23.0 release

**Files:**
- Modify: `package.json`, `src-tauri/tauri.conf.json`, `src-tauri/Cargo.toml`

- [ ] **Step 1: Bump versions**

All three to `0.23.0`. Verify:

```bash
grep -E '"version"|^version' package.json src-tauri/tauri.conf.json src-tauri/Cargo.toml
```

Expected: three lines, all showing `0.23.0`.

- [ ] **Step 2: Refresh Cargo.lock**

```bash
cargo check --manifest-path src-tauri/Cargo.toml
```

- [ ] **Step 3: Commit version bump**

```bash
git add package.json src-tauri/tauri.conf.json src-tauri/Cargo.toml src-tauri/Cargo.lock
git commit -m "v0.23.0: drop legacy transcription settings keys, single-source ProviderConfig"
```

- [ ] **Step 4: Run release**

```bash
pnpm release
```

Wait for: build → sign → notarise → staple → updater sign → tag push → GitHub release.

- [ ] **Step 5: Smoke-test the released DMG**

Install the new DMG over v0.22.2. Confirm:

1. App launches without TCC re-grant.
2. Migration runs silently — no error toast on first launch.
3. Settings shows the same provider/model that was active in v0.22.2.
4. Recording with the chosen provider transcribes successfully.

---

## Open questions deferred from this phase

These were considered for inclusion but are tracked separately so Phase 3 stays focused on cleanup:

1. **Per-note provider override UI.** Notes already have a `language` per-note override; per-provider would extend the same dropdown pattern. Touches `notes` schema (new column), `transcribe_chunk` (per-note override resolution), and the Note view's settings sheet. Worth a dedicated phase — call it Phase 4 if user demand surfaces.
2. **Nova-3 multi-word `keyterm`.** Current Deepgram adapter sends `keyterm=Term` (single word) when `model.startsWith("nova-3")`. Multi-word phrases (e.g. "Hugging Face") would require splitting bias terms differently and possibly a UI hint. Two-day task; skip until users ask.
3. **Groq retry/backoff on 429.** Deferred per the Phase 2 plan note; revisit if a user reports rate-limit errors during long recordings.
4. **Removing `from_legacy_settings`.** The migration calls it once and frontend never touches it. Phase 4 candidate after v0.23 has been out for ≥1 minor without anyone reporting migration issues.

---

## Self-review

**Spec coverage** (against the scope agreed in the chat):

- ✅ Backend: `read_provider_config` writes back is replaced by an explicit one-shot DB migration (Task 1+2) — same outcome, cleaner separation. Recording_start prereq, prewarm, transcribe_chunk's use_gpu read all route through `read_provider_config` (Task 6).
- ✅ Backend: drop `read_openai_api_key` shim and `api_key_*` Tauri commands (Tasks 3+4).
- ✅ Backend: `local_whisper_use_gpu_setting` deleted; `local_model_path` takes explicit `model_id` (Tasks 5+6).
- ✅ Frontend: read `ProviderConfig` on mount via new `getProviderConfig` (Tasks 7+9). Drop legacy keys from `SettingsKey` + `DEFAULTS` (Task 8). Drop `getApiKey/setApiKey/testApiKey` (Task 4). `Transcription.tsx` + `ApiKeys.tsx` rewired (Tasks 10+11).
- ✅ DB: one-shot migration deletes orphan rows once `transcribe_config` is present (Task 1+2).
- ✅ Release v0.23.0 (Task 13).

**Type consistency:**

- `ProviderConfig` shape in `ipc.ts:69–73` matches the Rust `crate::stt::ProviderConfig` tag `provider` field exactly (`openai` / `local` / `deepgram` / `groq`).
- `LocalWhisperConfig.model_id` / `.preset` / `.use_gpu` field names match what `Transcription.tsx` reads in Task 10's local section.
- Provider radio values (`PROVIDERS_BASE` + `LOCAL_PROVIDER`) emit the same four strings the `ProviderConfig` discriminator accepts.
- Backend's `get_provider_config` returns `Result<crate::stt::ProviderConfig, String>` which serde serialises to the JSON shape `getProviderConfig` deserialises in `ipc.ts`.

**Placeholder scan:**

- No `TODO`, `TBD`, "implement later".
- No "similar to Task X" — code is repeated where needed.
- No "add appropriate error handling" hand-waves; Task 1's migration handles serialise errors, Tasks 6's prereq paths preserve the existing error shape.
- Every TypeScript snippet is complete and self-contained; every Rust snippet is the actual code to paste.

**Risks left to attend at execution:**

- Task 4 + Task 5 leave the tree red between commits (TypeScript) and red until Task 5 inlines the placeholders. Both are fixed within ~10 minutes of cumulative work; if a reviewer balks, fold Tasks 4+5 into a single commit and Tasks 9+10+11 into a single commit. The plan keeps them separate to bound blast radius per commit.
- Task 12 step 4 (v0.21-shaped DB) requires a clean SQLite shell command. If Cmd+Q during the dev run leaves a write-ahead log, the `INSERT` may race the WAL checkpoint — close the app cleanly first.

## Estimated diff

| Task | Lines added | Lines removed | Time |
|---|---|---|---|
| 1: db helpers + migration | ~60 | 0 | 30m |
| 2: wire migration | ~10 | 0 | 15m |
| 3: inline OpenAI shims | ~5 | ~12 | 15m |
| 4: drop api_key_* commands | ~3 | ~30 | 30m |
| 5: local_model_path signature | ~10 | ~5 | 30m |
| 6: prereq + prewarm + transcribe_chunk | ~50 | ~80 | 1.5h |
| 7: simplify read_provider_config + add get_provider_config | ~20 | ~20 | 30m |
| 8: ipc.ts + types.ts cleanup | ~3 | ~12 | 15m |
| 9: useSettings refactor | ~30 | ~50 | 2h |
| 10: Transcription.tsx | ~120 | ~50 | 2h |
| 11: ApiKeys.tsx | ~5 | ~5 | 20m |
| 12: smoke test | 0 | 0 | 1h |
| 13: release | ~5 | 0 | 30m + notarise wait |
| **Total** | **~321** | **~264** | **~10h focused** |
