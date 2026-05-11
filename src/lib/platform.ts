// Cheap platform detection for UI strings that need to differ between
// macOS and Windows/Linux. We don't await Tauri's async `platform()` API
// here because every consumer is a render-time string — userAgent gives
// the right answer synchronously on first paint, no flicker.

const IS_MAC =
  typeof navigator !== "undefined" && /Mac|iPhone|iPad/.test(navigator.userAgent);

export { IS_MAC };

/** "⌘" on macOS, "Ctrl" elsewhere — for shortcut tooltips. */
export const cmdKey = IS_MAC ? "⌘" : "Ctrl+";

/** OS file manager name — used in "Open in Finder/Explorer" labels. */
export const fileManagerName = IS_MAC ? "Finder" : "Explorer";

/** Where API keys are stored — surfaces in the API Keys tab. */
export const credentialStoreName = IS_MAC
  ? "macOS Keychain"
  : "Windows Credential Manager";

/** Generic name for the user's machine — used in "stays on your <X>" copy. */
export const deviceName = IS_MAC ? "Mac" : "PC";

/** Display name for the GPU/accelerator running on-device inference. */
export const acceleratorName = IS_MAC ? "Apple Neural Engine" : "GPU (Vulkan)";

/** Whisper inference acceleration — shows up in the Language settings. */
export const whisperAcceleratorLabel = IS_MAC
  ? "Use Metal (Apple GPU) for Whisper inference"
  : "Use Vulkan (GPU) for Whisper inference";
