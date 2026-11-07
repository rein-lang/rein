import React from 'react';
import {AbsoluteFill, useCurrentFrame, useVideoConfig} from 'remotion';
import {C} from '../theme';
import {pop, ramp} from '../lib/motion';
import {Background, BeatProps, Caption, drawProps} from '../components/parts';

// Fixed scatter: every render draws the same tangle.
const NODES: [number, number][] = [
  [560, 330], [720, 470], [610, 640], [860, 260], [930, 600], [1080, 420],
  [1220, 300], [1180, 620], [1360, 500], [780, 780], [1460, 720],
];
const EDGES: [number, number][] = [
  [0, 1], [1, 2], [0, 3], [1, 4], [2, 4], [3, 5], [4, 5], [5, 6], [4, 7],
  [6, 8], [7, 8], [2, 9], [9, 4], [8, 10], [7, 10], [5, 8], [3, 6],
];
const NODE = 64;

const wobble = (i: number, frame: number): [number, number] => [
  Math.sin(frame / 22 + i * 1.7) * 10,
  Math.cos(frame / 19 + i * 1.3) * 10,
];

// Tangle: nodes pop in, wires draw and keep shifting, so the graph reads as messy.
export const Beat1Hook: React.FC<BeatProps> = ({frames, caption}) => {
  const frame = useCurrentFrame();
  const {fps} = useVideoConfig();
  const at = (p: number) => p * frames;

  const center = (i: number): [number, number] => {
    const [dx, dy] = wobble(i, frame);
    return [NODES[i][0] + dx, NODES[i][1] + dy];
  };

  return (
    <AbsoluteFill>
      <Background />
      <svg width={1920} height={1080} style={{position: 'absolute', left: 0, top: 0}}>
        {EDGES.map(([a, b], k) => {
          const [ax, ay] = center(a);
          const [bx, by] = center(b);
          const len = Math.hypot(bx - ax, by - ay) || 1;
          const bend = Math.sin(frame / 25 + k) * 70;
          const cx = (ax + bx) / 2 - ((by - ay) / len) * bend;
          const cy = (ay + by) / 2 + ((bx - ax) / len) * bend;
          const grow = ramp(frame, at(0.04) + k * 4, at(0.04) + k * 4 + 24);
          return (
            <path
              key={k}
              d={`M ${ax} ${ay} Q ${cx} ${cy} ${bx} ${by}`}
              fill="none"
              stroke={C.line}
              strokeWidth={3}
              strokeLinecap="round"
              {...drawProps(grow)}
            />
          );
        })}
      </svg>
      {NODES.map((_, i) => {
        const p = pop(frame, fps, at(0.02) + i * 4);
        const [x, y] = center(i);
        return (
          <div
            key={i}
            style={{
              position: 'absolute',
              left: x - NODE / 2,
              top: y - NODE / 2,
              width: NODE,
              height: NODE,
              boxSizing: 'border-box',
              borderRadius: 16,
              background: C.card,
              border: `3px solid ${C.faint}`,
              opacity: p,
              transform: `scale(${0.6 + 0.4 * p})`,
            }}
          />
        );
      })}
      <Caption text={caption} frames={frames} />
    </AbsoluteFill>
  );
};
