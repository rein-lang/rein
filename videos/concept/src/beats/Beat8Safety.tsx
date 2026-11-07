import React from 'react';
import {AbsoluteFill, useCurrentFrame, useVideoConfig} from 'remotion';
import {C} from '../theme';
import {mixHex, pop, ramp} from '../lib/motion';
import {Background, Bar, BeatProps, Caption, CheckIcon, CrossIcon, drawProps, FileCard, Pill} from '../components/parts';

const ROWS = [300, 440, 580, 720];
const BROKEN = 2;
const PANEL = {x: 1000, y: 250};
const LINE_WIDTHS = [440, 320, 380, 260, 400, 300];
const LINE_INDENT = [0, 40, 40, 0, 0, 0];

// Four paths, each checked. One pointer is broken; it turns red, then is fixed.
// The editor on the right shows the same problem as a squiggle while you type.
export const Beat8Safety: React.FC<BeatProps> = ({frames, caption}) => {
  const frame = useCurrentFrame();
  const {fps} = useVideoConfig();
  const at = (p: number) => p * frames;

  const fix = ramp(frame, at(0.6), at(0.7));
  const lineColor = mixHex(C.danger, C.accent, fix);
  const squiggleColor = mixHex(C.danger, C.accent, fix);

  const zig: string[] = [];
  for (let i = 0; i <= 19; i++) {
    zig.push(`${40 + i * 13},${284 + (i % 2 === 0 ? 0 : 6)}`);
  }

  return (
    <AbsoluteFill>
      <Background />
      <svg width={1920} height={1080} style={{position: 'absolute', left: 0, top: 0}}>
        {ROWS.map((cy, i) => {
          const isBroken = i === BROKEN;
          const start = at(0.08 + i * 0.05);
          const draw = ramp(frame, start, start + 0.12 * frames);
          return (
            <line
              key={i}
              x1={300}
              y1={cy}
              x2={762}
              y2={cy}
              stroke={isBroken ? lineColor : C.accent}
              strokeWidth={4}
              strokeLinecap="round"
              {...drawProps(draw)}
            />
          );
        })}
      </svg>

      {ROWS.map((cy, i) => {
        const isBroken = i === BROKEN;
        const ok = pop(frame, fps, at(0.22 + i * 0.05));
        return (
          <React.Fragment key={i}>
            <Pill x={150} y={cy - 38} w={150} h={76} radius={18} color={C.faint} opacity={ok} scale={0.9 + 0.1 * ok} />
            <div style={{position: 'absolute', left: 762, top: cy - 38, opacity: ok}}>
              {isBroken ? (
                <>
                  <div style={{opacity: 1 - fix}}>
                    <CrossIcon size={76} color={C.danger} />
                  </div>
                  <div style={{position: 'absolute', left: 0, top: 0, opacity: fix}}>
                    <CheckIcon size={76} />
                  </div>
                </>
              ) : (
                <CheckIcon size={76} />
              )}
            </div>
          </React.Fragment>
        );
      })}

      <FileCard x={PANEL.x} y={PANEL.y} w={760} h={520}>
        <svg width={760} height={520} style={{position: 'absolute', left: 0, top: 0}}>
          <circle cx={32} cy={30} r={7} fill={C.faint} />
          <circle cx={56} cy={30} r={7} fill={C.faint} />
          <circle cx={80} cy={30} r={7} fill={C.faint} />
          <polyline
            points={zig.join(' ')}
            fill="none"
            stroke={squiggleColor}
            strokeWidth={3}
            strokeLinejoin="round"
            opacity={ramp(frame, at(0.4), at(0.46))}
          />
          <rect
            x={40 + LINE_WIDTHS[4] + 12}
            y={90 + 4 * 56 - 6}
            width={4}
            height={28}
            fill={C.accent}
            opacity={pop(frame, fps, at(0.5)) * (frame % 24 < 14 ? 1 : 0.15)}
          />
        </svg>
        {LINE_WIDTHS.map((w, j) => (
          <Bar key={j} x={40 + LINE_INDENT[j]} y={90 + j * 56} w={w} h={14} opacity={0.5} />
        ))}
        <div style={{position: 'absolute', left: 40, top: 440, opacity: ramp(frame, at(0.72), at(0.8))}}>
          <CheckIcon size={28} />
        </div>
        <Bar x={84} y={447} w={300} h={12} opacity={0.4} />
      </FileCard>

      <Caption text={caption} frames={frames} />
    </AbsoluteFill>
  );
};
