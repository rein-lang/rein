import React from 'react';
import {AbsoluteFill, useCurrentFrame, useVideoConfig} from 'remotion';
import {C} from '../theme';
import {alongPolyline, lerp, mixHex, Pt, polyPath, pop, ramp} from '../lib/motion';
import {Background, Bar, BeatProps, Caption, drawProps, FileCard, QuestionIcon} from '../components/parts';

// Route: main -> function -> function -> function -> instruction.
const CARDS = [
  {key: 'main', x: 120, w: 250, label: 'main', lit: [true, true], thr: 0},
  {key: 'a', x: 470, w: 240, label: 'function', lit: [true, false], thr: 0.25},
  {key: 'b', x: 830, w: 240, label: 'function', lit: [false, true], thr: 0.5},
  {key: 'c', x: 1190, w: 240, label: 'function', lit: [true, false], thr: 0.75},
] as const;
const BUBBLE = {x: 1520, w: 260, h: 170, y: 455};
const ROUTE: Pt[] = [[245, 540], [590, 540], [950, 540], [1310, 540], [1650, 540]];

// The agent starts at main, answers only the questions it meets, and follows pointers.
export const Beat7Flow: React.FC<BeatProps> = ({frames, caption}) => {
  const frame = useCurrentFrame();
  const {fps} = useVideoConfig();
  const at = (p: number) => p * frames;

  const s = ramp(frame, at(0.3), at(0.86));
  const [dx, dy] = alongPolyline(ROUTE, s);
  const mainGlow = ramp(frame, at(0.04), at(0.12));
  const bubbleGlow = ramp(frame, at(0.9), at(0.95));
  const bubbleIn = pop(frame, fps, at(0.84));

  const cardGlow = (thr: number) => ramp(s, thr - 0.02, thr + 0.03);

  return (
    <AbsoluteFill>
      <Background />
      <svg width={1920} height={1080} style={{position: 'absolute', left: 0, top: 0}}>
        <path d={polyPath(ROUTE)} stroke={C.line} strokeWidth={3} fill="none" {...drawProps(1)} />
        <path
          d={polyPath(ROUTE)}
          stroke={C.accent}
          strokeWidth={4}
          strokeLinecap="round"
          fill="none"
          {...drawProps(s)}
        />
      </svg>

      {CARDS.map((card, i) => {
        const glow = card.key === 'main' ? mainGlow : cardGlow(card.thr);
        const enter = pop(frame, fps, at(0.02) + i * 0.02 * frames);
        const cardY = 440;
        return (
          <React.Fragment key={card.key}>
            <FileCard
              x={card.x}
              y={cardY}
              w={card.w}
              h={200}
              label={card.label}
              labelColor={card.key === 'main' ? C.accent : C.muted}
              glow={glow * 0.8}
              opacity={enter}
            >
              <Bar x={28} y={80} w={card.w - 56} h={14} />
              <Bar x={28} y={110} w={card.w - 110} h={14} />
            </FileCard>
            {card.lit.map((isLit, k) => {
              const chipX = card.x + 24 + k * 96;
              const litAmount = card.key === 'main' ? mainGlow : cardGlow(card.thr);
              const on = isLit ? litAmount : 0;
              return (
                <div
                  key={k}
                  style={{
                    position: 'absolute',
                    left: chipX,
                    top: 364,
                    width: 84,
                    height: 56,
                    boxSizing: 'border-box',
                    borderRadius: 28,
                    border: `2px solid ${mixHex(C.faint, C.accent, on)}`,
                    background: mixHex(C.card, '#15332f', on),
                    display: 'flex',
                    alignItems: 'center',
                    justifyContent: 'center',
                    opacity: enter * (isLit ? 1 : 0.5),
                  }}
                >
                  <QuestionIcon size={28} color={mixHex(C.faint, C.accent, on)} />
                </div>
              );
            })}
          </React.Fragment>
        );
      })}

      <FileCard
        x={BUBBLE.x}
        y={BUBBLE.y}
        w={BUBBLE.w}
        h={BUBBLE.h}
        label="instruction"
        labelColor={C.accent}
        glow={bubbleGlow * 0.8}
        opacity={bubbleIn}
      >
        <Bar x={28} y={70} w={200} h={14} />
        <Bar x={28} y={100} w={180} h={14} />
      </FileCard>

      <div
        style={{
          position: 'absolute',
          left: dx - 16,
          top: dy - 16,
          width: 32,
          height: 32,
          borderRadius: 16,
          background: C.accent,
          boxShadow: `0 0 ${lerp(10, 30, s)}px rgba(45,212,191,0.8)`,
          opacity: ramp(frame, at(0.28), at(0.32)) * (1 - ramp(frame, at(0.9), at(0.94))),
        }}
      />

      <Caption text={caption} frames={frames} />
    </AbsoluteFill>
  );
};
