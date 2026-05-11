import { ApiKeyField } from "../components/ApiKeyField";
import { Row, Section } from "../components/Section";
import type { SettingsHook } from "../useSettings";
import { credentialStoreName } from "../../../lib/platform";

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
            locally in the {credentialStoreName}; not sent anywhere except OpenAI.
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
            Stored in the {credentialStoreName}; sent only to api.deepgram.com.
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
            of OpenAI Whisper. Stored in the {credentialStoreName}; sent only
            to api.groq.com.
          </p>
        </Row>
      </Section>
    </>
  );
}
