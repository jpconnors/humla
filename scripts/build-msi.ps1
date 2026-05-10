# Build a distributable Humla installer (.exe via NSIS) for Windows.
# Output lands in src-tauri/target/release/bundle/nsis/.
#
# Mirrors scripts/build-dmg.sh on macOS. Authenticode signing is left as a
# follow-up — Tauri's bundler reads the cert from %TAURI_PFX_PATH% +
# %TAURI_PFX_PASSWORD% if set, otherwise produces an unsigned installer
# (recipients see the SmartScreen warning on first install).
#
# Usage:
#   pwsh -File scripts/build-msi.ps1
#
# Or via npm script:
#   pnpm bundle:windows

$ErrorActionPreference = "Stop"
Set-Location -Path (Join-Path $PSScriptRoot "..")

# 1. Build both sidecars (cached: skipped if sources unchanged).
& pwsh -File scripts/build-sidecars-windows.ps1
if ($LASTEXITCODE -ne 0) { throw "sidecar build failed" }

# 2. Build + bundle the Tauri app. Uses --bundles nsis to skip the macOS-only
#    app/dmg targets the conf still lists for parity. The resulting .exe is
#    a per-user installer (no admin prompt) by default — see
#    tauri.conf.json bundle.windows.nsis.installMode.
& pnpm tauri build --bundles nsis
if ($LASTEXITCODE -ne 0) { throw "tauri build failed" }

# 3. Surface the artifact path so the release script can find it.
$nsisDir = "src-tauri/target/release/bundle/nsis"
$installer = Get-ChildItem -Path $nsisDir -Filter "*.exe" -ErrorAction SilentlyContinue `
             | Sort-Object LastWriteTime -Descending | Select-Object -First 1
if (-not $installer) {
    throw "build finished but no installer found in $nsisDir"
}

Write-Host ""
Write-Host "Installer ready: $($installer.FullName)"
if (-not $env:TAURI_PFX_PATH) {
    Write-Host "Unsigned. Recipients will see SmartScreen warning on first install."
    Write-Host "To sign, set TAURI_PFX_PATH + TAURI_PFX_PASSWORD env vars before re-running."
} else {
    Write-Host "Signed with cert at $env:TAURI_PFX_PATH."
}
