import { DiarizeModelManager } from "../components/DiarizeModelManager";
import { LocalModelManager } from "../components/LocalModelManager";
import { Row, Section } from "../components/Section";
import { Select } from "../components/Select";
import {
  LOCAL_PROVIDER,
  PROVIDERS_BASE,
  TRANSCRIBE_MODELS,
  WHISPER_PRESETS,
  inputClass,
  type Provider,
} from "../types";
import type { SettingsHook } from "../useSettings";

export function TranscriptionTab({
  s,
  update,
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
  const provider: Provider = (s.transcribe_provider as Provider) ?? "openai";

  return (
    <>
      <Section title="Provider">
        <Row label="Source">
          <Select
            value={provider}
            onChange={(v) => update("transcribe_provider", v)}
            options={
              local.models.some((m) => m.downloaded)
                ? [...PROVIDERS_BASE, LOCAL_PROVIDER]
                : PROVIDERS_BASE
            }
          />
          {provider === "local" && !local.models.some((m) => m.downloaded) && (
            <p className="text-xs text-red-600 dark:text-red-400 mt-2">
              No local model is downloaded. Download one below before recording.
            </p>
          )}
        </Row>
        {provider === "openai" && (
          <Row label="Model">
            <Select
              value={s.transcribe_model}
              onChange={(v) => update("transcribe_model", v)}
              options={TRANSCRIBE_MODELS.map((m) => ({ value: m, label: m }))}
            />
            {s.transcribe_model === "gpt-4o-transcribe-diarize" && (
              <p className="text-xs text-[var(--color-text-muted)] mt-2">
                Note: <code>gpt-4o-transcribe-diarize</code> treats the
                language setting as a hint and does not accept a biasing
                prompt. For strict language locking, use{" "}
                <code>whisper-1</code> or <code>gpt-4o-transcribe</code>.
              </p>
            )}
          </Row>
        )}
      </Section>

      {provider === "local" && (
        <Section title="Local model behaviour">
          <Row label="Quality preset">
            <Select
              value={s.whisper_preset}
              onChange={(v) => update("whisper_preset", v)}
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
                checked={s.local_whisper_use_gpu !== "false"}
                onChange={(e) =>
                  update("local_whisper_use_gpu", e.target.checked ? "true" : "false")
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
          <Row label="Final pass">
            <label className="flex items-center gap-2 cursor-pointer text-sm">
              <input
                type="checkbox"
                checked={s.final_pass === "true"}
                onChange={(e) =>
                  update("final_pass", e.target.checked ? "true" : "false")
                }
              />
              Re-transcribe the full audio after recording stops
            </label>
            <p className="text-xs text-[var(--color-text-muted)] mt-2">
              When the recording ends, runs Whisper once over the saved
              audio with its native 30-second sliding window. Removes
              chunk-boundary cuts and breaks repetition loops that
              contaminated the live transcript. Local model only — adds
              roughly 1 minute of post-stop processing per 10 minutes of
              recording.
            </p>
          </Row>
        </Section>
      )}

      <Section title="Local models">
        <LocalModelManager
          state={local}
          activeId={s.local_whisper_model}
          language={s.language}
          onDownload={downloadModel}
          onDelete={deleteModel}
          onSelect={(id) => update("local_whisper_model", id)}
        />
      </Section>

      <Section title="Speaker diarization">
        <p className="text-xs text-[var(--color-text-muted)]">
          When downloaded and active, every recording is automatically
          tagged with <code>Speaker 1:</code> / <code>Speaker 2:</code>
          labels before polishing. Both engines run locally via CoreML /
          Apple Neural Engine; pick whichever works better for your
          recordings.
        </p>
        <Row label="Community-1 (clustering)">
          <label className="flex items-center gap-2 cursor-pointer text-sm mb-2">
            <input
              type="radio"
              name="diarize_model"
              checked={s.diarize_model === "community1"}
              disabled={!diarize.status?.downloaded}
              onChange={() => update("diarize_model", "community1")}
            />
            Use Community-1 for new recordings
          </label>
          <DiarizeModelManager
            state={diarize}
            onDownload={downloadDiarize}
            onDelete={deleteDiarize}
          />
          <p className="text-xs text-[var(--color-text-muted)] mt-2">
            Pyannote community-1 segmentation + WeSpeaker embeddings + VBx
            clustering. Strong baseline; auto-detects speaker count;
            occasionally collapses on rapid back-and-forth in the same
            channel.
          </p>
        </Row>
        <Row label="Sortformer (end-to-end)">
          <label className="flex items-center gap-2 cursor-pointer text-sm mb-2">
            <input
              type="radio"
              name="diarize_model"
              checked={s.diarize_model === "sortformer"}
              disabled={!sortformer.status?.downloaded}
              onChange={() => update("diarize_model", "sortformer")}
            />
            Use Sortformer for new recordings
          </label>
          <DiarizeModelManager
            state={sortformer}
            onDownload={downloadSortformer}
            onDelete={deleteSortformer}
          />
          <p className="text-xs text-[var(--color-text-muted)] mt-2">
            NVIDIA Sortformer running in batch over the saved WAV. Fixed
            4-speaker cap, no count hint. Designed to handle rapid speaker
            changes that the clustering approach struggles with — the
            architectural answer if Community-1 keeps confusing your
            speakers.
          </p>
        </Row>
      </Section>

      <Section title="Vocabulary">
        <Row label="Custom terms">
          <textarea
            value={s.custom_vocabulary}
            onChange={(e) => update("custom_vocabulary", e.target.value)}
            rows={3}
            placeholder="Tauri, Humla, ScreenCaptureKit, Granola"
            className={inputClass + " leading-relaxed"}
            style={{ fontFamily: "var(--font-mono)" }}
          />
          <p className="text-xs text-[var(--color-text-muted)] mt-2">
            Comma- or newline-separated. Names, jargon, and uncommon
            spellings — biases the transcriber toward these tokens.
            <code> gpt-4o-transcribe-diarize </code> ignores it.
          </p>
        </Row>
      </Section>
    </>
  );
}
