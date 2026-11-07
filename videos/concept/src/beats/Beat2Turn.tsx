import React from 'react';
import {AbsoluteFill, useCurrentFrame} from 'remotion';
import {C} from '../theme';
import {lerp, mixHex, ramp} from '../lib/motion';
import {Background, BeatProps, Caption} from '../components/parts';

// Same scatter as the tangle, so the morph starts from the same picture.
const START: [number, number][] = [
  [560, 330], [720, 470], [610, 640], [860, 260], [930, 600], [1080, 420],
  [1220, 300], [1180, 620], [1360, 500], [780, 780], [1460, 720],
];
// Three clean cards. Nodes 0..2 become them; the rest collapse into the middle.
const CARD_CENTERS: [number, number][] = [[470, 540], [960, 540], [1450, 540]];
const EDGES: [number, number][] = [
  [0, 1], [1, 2], [0, 3], [1, 4], [2, 4], [3, 5], [4, 5], [5, 6], [4, 7],
  [6, 8], [7, 8], [2, 9], [9, 4], [8, 10], [7, 10], [5, 8], [3, 6],
];

export const Beat2Turn: React.FC<BeatProps> = ({frames, caption}) => {
  const frame = useCurrentFrame();
  const at = (p: number) => p * frames;
  const morph = ramp(frame, at(0.3), at(0.72));
  const fade = ramp(frame, at(0.5), at(0.78));
  const wires = 1 - ramp(frame, at(0.14), at(0.46));

  const target = (i: number): [number, number] => (i < 3 ? CARD_CENTERS[i] : [960, 540]);
  const centre = (i: number): [number, number] => {
    const [tx, ty] = target(i);
    return [lerp(START[i][0], tx, morph), lerp(START[i][1], ty, morph)];
  };

  return (
    <AbsoluteFill>
      <Background />
      <svg width={1920} height={1080} style={{position: 'absolute', left: 0, top: 0}}>
        {EDGES.map(([a, b], k) => {
          const [ax, ay] = centre(a);
          const [bx, by] = centre(b);
          return (
            <line
              key={k}
              x1={ax}
              y1={ay}
              x2={bx}
              y2={by}
              stroke={C.line}
              strokeWidth={3}
              strokeLinecap="round"
              opacity={wires}
            />
          );
        })}
      </svg>
      {START.map((_, i) => {
        const isCard = i < 3;
        const [cx, cy] = centre(i);
        const w = isCard ? lerp(64, 300, morph) : lerp(64, 0, morph);
        const h = isCard ? lerp(64, 220, morph) : lerp(64, 0, morph);
        return (
          <div
            key={i}
            style={{
              position: 'absolute',
              left: cx - w / 2,
              top: cy - h / 2,
              width: w,
              height: h,
              boxSizing: 'border-box',
              borderRadius: lerp(16, 22, morph),
              background: C.card,
              border: `3px solid ${mixHex(C.faint, C.border, morph)}`,
              opacity: isCard ? 1 : 1 - fade,
            }}
          />
        );
      })}
      <Caption text={caption} frames={frames} />
    </AbsoluteFill>
  );
};
