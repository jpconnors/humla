import { useRecordingStore } from "../lib/store";

// Bottom-right toast that appears whenever a recording-stop has triggered
// the polish pass. The recording status store already carries the
// "polishing" phase from the backend; we just render a discrete card while
// it's active. Mounted globally so it stays visible if the user navigates
// away from the note that's being polished.
export function PolishToast() {
  const phase = useRecordingStore((s) => s.status.phase);
  if (phase !== "polishing") return null;

  return (
    <div className="no-drag fixed bottom-6 right-6 z-50 max-w-sm">
      <div className="px-4 py-3 rounded-lg bg-[var(--color-surface)] border border-[var(--color-line)] shadow-md text-sm flex items-center gap-3">
        <Spinner />
        <div>
          <div className="font-medium">Polishing transcript…</div>
          <div className="text-[var(--color-text-muted)] text-xs">
            Cleaning up typos and chunk-boundary artifacts.
          </div>
        </div>
      </div>
    </div>
  );
}

function Spinner() {
  return (
    <span
      className="inline-block w-3.5 h-3.5 rounded-full border-2 border-[var(--color-line-visible)] border-t-[var(--color-text)] animate-spin"
      aria-hidden
    />
  );
}
