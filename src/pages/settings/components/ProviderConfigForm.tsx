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
import { whisperAcceleratorLabel } from "../../../lib/platform";

type LocalModelSummary = {
  id: string;
  label: string;
  kind: "multilingual" | "language_specific";
  specificLanguage: string | null;
  downloaded: boolean;
};

// Reusable provider+model picker. Used by the Default provider section
// and the per-language override editor. Keeps the four provider variants'
// divergent fields (OpenAI: model, Local: model_id+preset+gpu, Deepgram:
// model, Groq: model) in one place so the two callers stay in lockstep.
//
// `localModels` is the live list of downloaded local models (from
// `useSettings.local.models`). Used to:
//   1. Hide the Local option when nothing is downloaded.
//   2. Pre-select the first downloaded multilingual model when
//      switching to Local.
//   3. Filter the model_id picker to actually-downloaded files.
//
// `filterLocalToLanguage` (when set) restricts the local model picker to
// `LanguageSpecific` models matching the language PLUS multilingual
// fallbacks. Used by the per-language override form so "Norwegian → Local
// → ?" only offers NB Whisper or multilingual options.
export function ProviderConfigForm({
  value,
  onChange,
  localModels,
  filterLocalToLanguage,
  hideLocal = false,
}: {
  value: ProviderConfig;
  onChange: (next: ProviderConfig) => void;
  localModels: LocalModelSummary[];
  filterLocalToLanguage?: string;
  hideLocal?: boolean;
}) {
  const provider = value.provider;
  const localAvailable = !hideLocal && localModels.some((m) => m.downloaded);

  const localModelOptions = localModels
    .filter((m) => m.downloaded)
    .filter((m) => {
      if (!filterLocalToLanguage) return true;
      // Multilingual models always usable; language-specific must match.
      return (
        m.kind === "multilingual" || m.specificLanguage === filterLocalToLanguage
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
            {whisperAcceleratorLabel}
          </label>
        </>
      )}
    </div>
  );
}
