"""Synthesize one padded, normalized narration WAV per beat with Kokoro v1.0 (local ONNX).

Reads narration/beats.json (single source of truth for speech).
Writes public/audio/beat-N.wav and src/timing.json (frame counts at 30 fps).

Env (defaults point at the local reddit-comment-explainer install; nothing is copied into the repo):
  KOKORO_MODEL   path to kokoro-v1.0.onnx
  KOKORO_VOICES  path to voices-v1.0.bin
  KOKORO_VOICE   voice id (default am_michael)
Run with the interpreter that has kokoro_onnx: KOKORO_PYTHON (see render.sh).
"""
import json
import os
import subprocess
from pathlib import Path

import numpy as np
import soundfile as sf
from kokoro_onnx import Kokoro

ROOT = Path(__file__).resolve().parent.parent
AUDIO_DIR = ROOT / "public" / "audio"
TMP_DIR = AUDIO_DIR / "_raw"
TIMING_FILE = ROOT / "src" / "timing.json"
DEFAULT_DIR = Path("/Users/ofek/dev/videos/reddit-comment-explainer/tts")
MODEL = os.environ.get("KOKORO_MODEL", str(DEFAULT_DIR / "kokoro-v1.0.onnx"))
VOICES = os.environ.get("KOKORO_VOICES", str(DEFAULT_DIR / "voices-v1.0.bin"))
VOICE = os.environ.get("KOKORO_VOICE", "am_michael")
SPEED = 1.0
SR_OUT = 48000
FPS = 30
LEAD_S = 0.4   # silence before speech
TAIL_S = 0.8   # silence after speech
TAPER_S = 0.012


def main() -> None:
    for p in (MODEL, VOICES):
        assert Path(p).exists(), f"missing Kokoro file: {p}"
    beats = json.loads((ROOT / "narration" / "beats.json").read_text())
    AUDIO_DIR.mkdir(parents=True, exist_ok=True)
    TMP_DIR.mkdir(parents=True, exist_ok=True)

    kokoro = Kokoro(MODEL, VOICES)
    timing = []
    for beat in beats:
        samples, sr = kokoro.create(beat["speech"], voice=VOICE, speed=SPEED, lang="en-us")
        audio = np.asarray(samples, dtype=np.float32)
        taper = int(TAPER_S * sr)
        ramp = np.linspace(0.0, 1.0, taper, dtype=np.float32)
        audio[:taper] *= ramp
        audio[-taper:] *= ramp[::-1]
        padded = np.concatenate([
            np.zeros(int(LEAD_S * sr), dtype=np.float32),
            audio,
            np.zeros(int(TAIL_S * sr), dtype=np.float32),
        ])
        raw = TMP_DIR / f"padded-{beat['id']}.wav"
        sf.write(str(raw), padded, sr)
        out = AUDIO_DIR / f"beat-{beat['id']}.wav"
        # Loudness-normalize per beat so scenes sit together; resample to 48 kHz for Remotion.
        subprocess.run(
            ["ffmpeg", "-y", "-hide_banner", "-loglevel", "error", "-i", str(raw),
             "-af", "loudnorm=I=-19:TP=-2", "-ar", str(SR_OUT), "-ac", "1", str(out)],
            check=True,
        )
        seconds = len(padded) / sr
        timing.append({"id": beat["id"], "name": beat["name"], "seconds": round(seconds, 3),
                       "frames": round(seconds * FPS)})
        print(f"beat {beat['id']} {beat['name']}: {seconds:.2f}s (speech {len(audio)/sr:.2f}s)")

    TIMING_FILE.write_text(json.dumps(timing, indent=2) + "\n")
    print(f"engine=kokoro-v1.0 voice={VOICE} wrote {TIMING_FILE} total={sum(t['frames'] for t in timing)} frames")


if __name__ == "__main__":
    main()
