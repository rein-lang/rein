import React from 'react';
import {AbsoluteFill, useCurrentFrame, useVideoConfig} from 'remotion';
import {C} from '../theme';
import {mixHex, pop, ramp} from '../lib/motion';
import {Background, Bar, BeatProps, Caption, FileCard} from '../components/parts';

const BODY_WIDTHS = [480, 420, 460, 300, 400, 340];
const CARD_W = 560;
const CARD_H = 420;

// A stack of function files. Each file opens with a description bar.
// The front file's description bar lights up as the narration names it.
export const Beat3FunctionIsFile: React.FC<BeatProps> = ({frames, caption}) => {
  const frame = useCurrentFrame();
  const {fps} = useVideoConfig();
  const at = (p: number) => p * frames;
  const glow = ramp(frame, at(0.55), at(0.7));
  const layers = [3, 2, 1, 0]; // back to front

  return (
    <AbsoluteFill>
      <Background />
      {layers.map((k) => {
        const enter = pop(frame, fps, at(0.04) + (3 - k) * 7);
        const front = k === 0;
        const x = 600 + 40 * k + (1 - enter) * 240;
        const y = 260 - 28 * k;
        return (
          <FileCard
            key={k}
            x={x}
            y={y}
            w={CARD_W}
            h={CARD_H}
            opacity={enter}
            label={front ? 'function' : undefined}
            glow={front ? glow * 0.6 : 0}
          >
            <div
              style={{
                position: 'absolute',
                left: 40,
                top: 64,
                width: 480,
                height: 30,
                borderRadius: 15,
                background: front ? mixHex('#22303b', C.accent, glow * 0.7) : '#22303b',
              }}
            />
            {BODY_WIDTHS.map((w, j) => (
              <Bar key={j} x={40} y={120 + j * 40} w={w} />
            ))}
          </FileCard>
        );
      })}
      <Caption text={caption} frames={frames} />
    </AbsoluteFill>
  );
};
