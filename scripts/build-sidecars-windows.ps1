# Build the Rust audio-capture + speaker-diarize sidecars for Windows and
# copy them into src-tauri/binaries/ with the triple suffix Tauri's bundler
# expects.
#
# Usage:
#   pwsh -File scripts/build-sidecars-windows.ps1
#
# Skips the per-sidecar rebuild when nothing in its source tree has changed
# (SHA-256 stamp at src-tauri/binaries/.<name>-<triple>.stamp), mirroring
# the macOS build-sidecar.sh behaviour. Override with FORCE_SIDECAR_REBUILD=1.

$ErrorActionPreference = "Stop"
Set-Location -Path (Join-Path $PSScriptRoot "..")

# Detect target triple from rustc. Tauri's externalBin contract is that
# `binaries/<name>` resolves to `<name>-<triple>.exe` on Windows.
$rustVersion = & rustc -vV
$hostLine = $rustVersion | Select-String -Pattern "^host: " | Select-Object -First 1
if (-not $hostLine) {
    throw "Could not parse rustc -vV output for target triple"
}
$triple = ($hostLine -replace "^host: ", "").Trim()
if (-not $triple.EndsWith("pc-windows-msvc") -and -not $triple.EndsWith("pc-windows-gnu")) {
    Write-Warning "Unexpected target triple '$triple' — Tauri expects a Windows triple. Continuing anyway."
}

$binariesDir = "src-tauri/binaries"
New-Item -ItemType Directory -Force -Path $binariesDir | Out-Null

$force = $env:FORCE_SIDECAR_REBUILD -eq "1"

function Build-Sidecar {
    param(
        [string]$SourceDir,
        [string]$BinName     # name of the [[bin]] in the crate's Cargo.toml
    )

    $dest = Join-Path $binariesDir "$BinName-$triple.exe"
    $stamp = Join-Path $binariesDir ".$BinName-$triple.stamp"

    # Source hash: every .rs file under src/, plus Cargo.toml + Cargo.lock if
    # present. Skips target/ to avoid rebuild loops.
    $srcFiles = Get-ChildItem -Path (Join-Path $SourceDir "src") -Recurse -File `
                | Where-Object { $_.FullName -notmatch "\\target\\" }
    $cargoToml = Join-Path $SourceDir "Cargo.toml"
    $cargoLock = Join-Path $SourceDir "Cargo.lock"
    $hashInput = ($srcFiles + @(Get-Item $cargoToml)) | ForEach-Object { Get-FileHash -Algorithm SHA256 $_.FullName } `
                | ForEach-Object { "$($_.Hash)  $($_.Path)" }
    if (Test-Path $cargoLock) {
        $hashInput += "$((Get-FileHash -Algorithm SHA256 $cargoLock).Hash)  Cargo.lock"
    }
    $combined = ($hashInput -join "`n")
    $sha = [System.BitConverter]::ToString([System.Security.Cryptography.SHA256]::Create().ComputeHash([System.Text.Encoding]::UTF8.GetBytes($combined))) -replace "-",""
    $sha = $sha.ToLower()

    if (-not $force -and (Test-Path $dest) -and (Test-Path $stamp) -and ((Get-Content $stamp -Raw).Trim() -eq $sha)) {
        Write-Host "$BinName unchanged, skipping rebuild ($dest)"
        return
    }

    Push-Location $SourceDir
    try {
        & cargo build --release
        if ($LASTEXITCODE -ne 0) { throw "$BinName cargo build failed" }
    } finally {
        Pop-Location
    }

    $built = Join-Path $SourceDir "target/release/$BinName.exe"
    if (-not (Test-Path $built)) {
        throw "Build succeeded but binary not found at $built"
    }
    Copy-Item -Force $built $dest
    Set-Content -Path $stamp -Value $sha -NoNewline
    Write-Host "built: $dest"
}

Build-Sidecar -SourceDir "audio-capture-rs"   -BinName "audio-capture"
Build-Sidecar -SourceDir "speaker-diarize-rs" -BinName "speaker-diarize"

Write-Host ""
Write-Host "Sidecars ready in $binariesDir/. Next: pnpm tauri build"
