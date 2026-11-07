import React from 'react';
import {AbsoluteFill, useCurrentFrame, useVideoConfig} from 'remotion';
import {C, FONT} from '../theme';
import {lerp, pop, ramp} from '../lib/motion';
import {Background, BeatProps, Caption, Pill} from '../components/parts';

// Title card: wordmark, accent rule, alpha status.
export const Beat9Close: React.FC<BeatProps> = ({frames, caption}) => {
  const frame = useCurrentFrame();
  const {fps} = useVideoConfig();
  const at = (p: number) => p * frames;

  const word = pop(frame, fps, at(0.02));
  const rule = ramp(frame, at(0.3), at(0.5));
  const status = pop(frame, fps, at(0.5));

  return (
    <AbsoluteFill>
      <Background />
      <div
        style={{
          position: 'absolute',
          left: 0,
          right: 0,
          top: 300,
          textAlign: 'center',
          fontFamily: FONT,
          fontSize: 230,
          fontWeight: 800,
          letterSpacing: -6,
          color: C.text,
          opacity: word,
          transform: `translateY(${(1 - word) * 30}px)`,
        }}
      >
        Rein
      </div>
      <div
        style={{
          position: 'absolute',
          left: 960 - 110,
          top: 600,
          width: lerp(0, 220, rule),
          height: 8,
          borderRadius: 4,
          background: C.accent,
        }}
      />
      <Pill x={960 - 190} y={660} w={380} h={76} text="Alpha, coming soon" color={C.accent} fontSize={34} textColor={C.accent} scale={status} opacity={status} />
      <Caption text={caption} frames={frames} />
    </AbsoluteFill>
  );
};
