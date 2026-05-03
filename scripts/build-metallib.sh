#!/usr/bin/env bash
# Prebuild ggml-metal's shader library so whisper.cpp's runtime
# Metal-source compilation never runs.
#
# Background: whisper-rs 0.13's vendored whisper.cpp embeds
# `ggml-metal.metal` as a string and asks the system Metal compiler
# to build it on first GPU init. The embed pipeline relies on a
# `sed`-based `__embed_ggml-common.h__` substitution in cmake that
# silently misfires in some setups (see whisper.cpp#3009 / #2041 /
# llama.cpp#5977), leaving the embedded source missing every
# `block_q*` struct typedef. The compiler then dumps ~50 lines of
# `error: use of undeclared identifier 'block_q5_1'` per chunk and
# whisper.cpp falls back to the BLAS (CPU) backend — losing GPU
# acceleration entirely.
#
# Workaround: build a real `default.metallib` ahead of time.
# whisper.cpp's loader checks `GGML_METAL_PATH_RESOURCES` at runtime
# and prefers a precompiled metallib over compiling source — so this
# script runs once at build time, ships the metallib in the .app's
# Resources/, and the runtime path stays out of trouble entirely.
#
# Outputs `src-tauri/resources/default.metallib`. Caches via SHA-256
# stamp so unchanged inputs don't force a rebuild.

set -euo pipefail

SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
ROOT=$(cd "$SCRIPT_DIR/.." && pwd)

METAL_SRC="$ROOT/src-tauri/metal/ggml-metal.metal"
COMMON_H="$ROOT/src-tauri/metal/ggml-common.h"

if [[ ! -f "$METAL_SRC" ]] || [[ ! -f "$COMMON_H" ]]; then
  echo "build-metallib: missing vendored Metal source under src-tauri/metal/" >&2
  echo "  expected: $METAL_SRC" >&2
  echo "  expected: $COMMON_H" >&2
  exit 1
fi

OUT_DIR="$ROOT/src-tauri/resources"
TARGET="$OUT_DIR/default.metallib"
STAMP="$OUT_DIR/.metallib.stamp"

# Hash the inputs so an unchanged source skips the rebuild — this
# script gets called from the dmg pipeline, no need to recompile
# every time.
CURRENT_HASH=$(cat "$METAL_SRC" "$COMMON_H" | shasum -a 256 | cut -d' ' -f1)
if [[ "${FORCE_METALLIB_REBUILD:-}" != "1" ]] \
   && [[ -f "$STAMP" ]] \
   && [[ -f "$TARGET" ]] \
   && [[ "$(cat "$STAMP")" == "$CURRENT_HASH" ]]; then
  echo "build-metallib: up-to-date ($TARGET)"
  exit 0
fi

mkdir -p "$OUT_DIR"
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

# Copy ggml-common.h next to the metal source so the
# `#include "ggml-common.h"` line in the .metal file resolves
# during xcrun metal compilation. (We don't sed-inline because
# Metal handles `#include` natively when the header is in the
# same directory — and that's exactly the path that breaks
# inside whisper.cpp's cmake embed pipeline.)
cp "$METAL_SRC" "$TMP/ggml-metal.metal"
cp "$COMMON_H" "$TMP/ggml-common.h"

xcrun -sdk macosx metal -O3 -c "$TMP/ggml-metal.metal" -o "$TMP/ggml-metal.air"
xcrun -sdk macosx metallib "$TMP/ggml-metal.air" -o "$TARGET"

echo "$CURRENT_HASH" > "$STAMP"
echo "build-metallib: built $TARGET"
