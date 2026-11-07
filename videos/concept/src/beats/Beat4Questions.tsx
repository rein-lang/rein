import React from 'react';
import {AbsoluteFill, interpolate, useCurrentFrame, useVideoConfig} from 'remotion';
import {C} from '../theme';
import {lerp, mixHex, pop, ramp} from '../lib/motion';
import {Background, Bar, BeatProps, Caption, CheckIcon, CrossIcon, drawProps, FileCard, QuestionIcon} from '../components/parts';

const CHIP_CY = [345, 510, 675];
const AVATAR = {x: 1040, y: 510, r: 92};
const ANSWER_X = 1200;

// A function card sends out questions. The agent answers each one:
// a check, one option of a few, or a no.
export const Beat4Questions: React.FC<BeatProps> = ({frames, caption}) => {
  const frame = useCurrentFrame();
  const {fps} = useVideoConfig();
  const at = (p: number) => p * frames;

  const chipPop = [0.12, 0.2, 0.28].map((p) => pop(frame, fps, at(p)));
  const eyeDy = interpolate(
    frame,
    [at(0.1), at(0.22), at(0.36), at(0.5), at(0.66), at(0.78)],
    [0, -10, -10, 10, 10, 0],
    {extrapolateLeft: 'clamp', extrapolateRight: 'clamp'},
  );
  const glow = ramp(frame, at(0.04), at(0.12)) * 0.6;

  const answers = [
    {y: 345, delay: 0.38, dot: 0.4},
    {y: 510, delay: 0.52, dot: 0.54},
    {y: 675, delay: 0.66, dot: 0.68},
  ];

  return (
    <AbsoluteFill>
      <Background />
      <svg width={1920} height={1080} style={{position: 'absolute', left: 0, top: 0}}>
        {CHIP_CY.map((cy, i) => (
          <g key={i}>
            <path
              d={`M 540 510 L 660 ${cy}`}
              stroke={C.line}
              strokeWidth={3}
              strokeLinecap="round"
              fill="none"
              {...drawProps(ramp(frame, at(0.1 + i * 0.06), at(0.22 + i * 0.06)))}
            />
            <path
              d={`M 830 ${cy} L ${AVATAR.x - AVATAR.r} ${AVATAR.y}`}
              stroke={C.line}
              strokeWidth={3}
              strokeLinecap="round"
              fill="none"
              {...drawProps(ramp(frame, at(0.2 + i * 0.06), at(0.3 + i * 0.06)))}
            />
          </g>
        ))}
        <circle cx={AVATAR.x} cy={AVATAR.y} r={AVATAR.r} fill={C.cardHi} stroke={C.accent} strokeWidth={3} />
        <circle cx={AVATAR.x - 28} cy={AVATAR.y - 6 + eyeDy} r={11} fill={C.text} />
        <circle cx={AVATAR.x + 28} cy={AVATAR.y - 6 + eyeDy} r={11} fill={C.text} />
        {answers.map((a, i) => {
          const p = ramp(frame, at(a.dot), at(a.dot + 0.08));
          const fadeOut = 1 - ramp(frame, at(a.dot + 0.08), at(a.dot + 0.11));
          const x = lerp(AVATAR.x + AVATAR.r, ANSWER_X, p);
          const y = lerp(AVATAR.y, a.y, p);
          return <circle key={i} cx={x} cy={y} r={9} fill={C.accent} opacity={p > 0 ? fadeOut : 0} />;
        })}
      </svg>

      <FileCard x={160} y={300} w={380} h={420} label="function" glow={glow}>
        <div
          style={{
            position: 'absolute',
            left: 36,
            top: 64,
            width: 308,
            height: 30,
            borderRadius: 15,
            background: '#22303b',
          }}
        />
        {[308, 250, 290, 200].map((w, j) => (
          <Bar key={j} x={36} y={124 + j * 40} w={w} />
        ))}
      </FileCard>

      {CHIP_CY.map((cy, i) => (
        <div
          key={`chip-${i}`}
          style={{
            position: 'absolute',
            left: 660,
            top: cy - 33,
            width: 170,
            height: 66,
            boxSizing: 'border-box',
            borderRadius: 33,
            border: `2px solid ${mixHex(C.border, C.accent, chipPop[i])}`,
            background: C.card,
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'center',
            opacity: chipPop[i],
            transform: `scale(${0.8 + 0.2 * chipPop[i]})`,
          }}
        >
          <QuestionIcon size={34} />
        </div>
      ))}

      {answers.map((a, i) => {
        const p = pop(frame, fps, at(a.delay));
        const common = {
          position: 'absolute' as const,
          left: ANSWER_X,
          top: a.y - 33,
          width: 260,
          height: 66,
          boxSizing: 'border-box' as const,
          borderRadius: 33,
          border: `2px solid ${C.border}`,
          background: C.card,
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'center',
          opacity: p,
          transform: `scale(${0.8 + 0.2 * p})`,
        };
        if (i === 1) {
          const pick = pop(frame, fps, at(0.6));
          return (
            <div key="opt" style={{...common, gap: 26}}>
              {[0, 1, 2].map((k) => (
                <div
                  key={k}
                  style={{
                    width: 32,
                    height: 32,
                    boxSizing: 'border-box',
                    borderRadius: 16,
                    border: `2px solid ${k === 1 ? C.accent : C.faint}`,
                    background: k === 1 ? mixHex(C.card, C.accent, pick) : C.card,
                  }}
                />
              ))}
            </div>
          );
        }
        return (
          <div key={i} style={common}>
            {i === 0 ? <CheckIcon size={40} /> : <CrossIcon size={40} color={C.muted} />}
          </div>
        );
      })}

      <Caption text={caption} frames={frames} />
    </AbsoluteFill>
  );
};
