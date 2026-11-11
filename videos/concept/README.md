# Rein concept video

A ~70 second narrated explainer for the Rein agent-flow language. Abstract animated visuals, no code on screen. Built with Remotion (React to MP4). Narration is local Kokoro v1.0 TTS.

Owned path: `videos/concept/`. Narration text: `script.md` and `narration/beats.json`.

## Output

- `out/rein-concept.mp4` (1920x1080, 30 fps, H.264 + AAC)
- `out/stills/beat-1.png` .. `beat-9.png` (one still per beat, at 92% through each beat)

## Setup

Requires Node 24 and ffmpeg/ffprobe on PATH. The narration step needs a Python that has `kokoro_onnx` and `soundfile`.

```bash
pnpm install
```

Kokoro files are read from the existing local install. Nothing is copied into this repo.

| Env var | Default |
|---|---|
| `KOKORO_PYTHON` | `/Users/ofek/dev/videos/reddit-comment-explainer/.venv/bin/python` |
| `KOKORO_MODEL` | `/Users/ofek/dev/videos/reddit-comment-explainer/tts/kokoro-v1.0.onnx` |
| `KOKORO_VOICES` | `/Users/ofek/dev/videos/reddit-comment-explainer/tts/voices-v1.0.bin` |
| `KOKORO_VOICE` | `am_michael` |

## Re-render

```bash
./render.sh
```

This runs, in order:

1. `scripts/tts.py`: synthesizes `public/audio/beat-N.wav` from `narration/beats.json`, pads and normalizes each clip, and writes `src/timing.json`.
2. `scripts/stills.mjs`: writes `out/stills/beat-N.png`.
3. `pnpm exec remotion render`: writes `out/rein-concept.mp4`.
4. `ffprobe`: prints duration and stream info.

Each beat lasts exactly as long as its narration clip. Editing a line in `narration/beats.json` and re-running `render.sh` updates timing and captions. Visual beats are timed as fractions of their beat length, so re-check the stills after a wording change.

## Layout

- `src/Concept.tsx`: sequences the nine beats, one `<Audio>` per beat.
- `src/beats/Beat1Hook.tsx` .. `Beat9Close.tsx`: one visual per beat.
- `src/components/parts.tsx`: shared cards, pills, icons, caption.
- `src/lib/motion.ts`: easing, spring, path helpers.
- `src/theme.ts`: colors (dark background, one teal accent) and font.
- `narration/beats.json`: speech and caption text per beat.

## Notes

- Cards show only short labels: function, question, if, switch, instruction, next function, main, and the wordmark.
- Hard limits: no syntax, filenames, CLI output or example flows on screen.
