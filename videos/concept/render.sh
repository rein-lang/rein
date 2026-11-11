#!/usr/bin/env bash
# Regenerate narration (Kokoro v1.0, local), stills and the MP4. Run from anywhere.
# Env overrides: KOKORO_PYTHON, KOKORO_MODEL, KOKORO_VOICES, KOKORO_VOICE. See README.md.
set -euo pipefail
cd "$(dirname "$0")"

KOKORO_DIR=/Users/ofek/dev/videos/reddit-comment-explainer
export KOKORO_PYTHON="${KOKORO_PYTHON:-$KOKORO_DIR/.venv/bin/python}"
export KOKORO_MODEL="${KOKORO_MODEL:-$KOKORO_DIR/tts/kokoro-v1.0.onnx}"
export KOKORO_VOICES="${KOKORO_VOICES:-$KOKORO_DIR/tts/voices-v1.0.bin}"
[ -x "$KOKORO_PYTHON" ] || { echo "KOKORO_PYTHON not executable: $KOKORO_PYTHON"; exit 1; }
[ -f "$KOKORO_MODEL" ] || { echo "Missing KOKORO_MODEL: $KOKORO_MODEL"; exit 1; }
[ -f "$KOKORO_VOICES" ] || { echo "Missing KOKORO_VOICES: $KOKORO_VOICES"; exit 1; }
[ -d node_modules ] || pnpm install

"$KOKORO_PYTHON" scripts/tts.py
node scripts/stills.mjs
pnpm exec remotion render src/index.ts Concept out/rein-concept.mp4 --codec=h264

ffprobe -v error -show_entries format=duration:stream=codec_type,width,height -of compact out/rein-concept.mp4
