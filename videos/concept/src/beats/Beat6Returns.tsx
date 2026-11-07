import React from 'react';
import {AbsoluteFill, useCurrentFrame, useVideoConfig} from 'remotion';
import {C} from '../theme';
import {arrowPoints, pop} from '../lib/motion';
import {Background, Bar, BeatProps, Caption, drawProps, FileCard, Pill} from '../components/parts';

// Every path ends at one of two things: an instruction bubble, or a pointer that jumps to another function.
export const Beat6Returns: React.FC<BeatProps> = ({frames, caption}) => {
  const frame = useCurrentFrame();
  const {fps} = useVideoConfig();
  const at = (p: number) => p * frames;
  const jumpTip: [number, number] = [1500, 596];

  return (
    <AbsoluteFill>
      <Background />
      <svg width={1920} height={1080} style={{position: 'absolute', left: 0, top: 0}}>
        <path d="M 460 540 C 700 540, 760 340, 1000 340" stroke={C.line} strokeWidth={3} fill="none" {...drawProps(pop(frame, fps, at(0.14)))} />
        <path d="M 460 540 C 700 540, 760 700, 1000 700" stroke={C.line} strokeWidth={3} fill="none" pathLength={1} strokeDasharray={1} strokeDashoffset={1 - pop(frame, fps, at(0.3))} />
        <path
          d="M 1150 662 Q 1280 470 1500 596"
          stroke={C.accent}
          strokeWidth={4}
          strokeLinecap="round"
          fill="none"
          pathLength={1}
          strokeDasharray={1}
          strokeDashoffset={1 - pop(frame, fps, at(0.7))}
        />
        <polygon points={arrowPoints(jumpTip, [1280, 470])} fill={C.accent} opacity={pop(frame, fps, at(0.82))} />
        <polygon points="1050,438 1032,482 1092,438" fill={C.card} stroke={C.border} strokeWidth={2} strokeLinejoin="round" opacity={pop(frame, fps, at(0.4))} />
      </svg>

      <FileCard x={160} y={445} w={300} h={190} label="function" opacity={pop(frame, fps, at(0.04))}>
        <Bar x={36} y={70} w={220} />
        <Bar x={36} y={100} w={240} />
        <Bar x={36} y={130} w={180} />
      </FileCard>

      <div
        style={{
          position: 'absolute',
          left: 1000,
          top: 240,
          width: 560,
          height: 200,
          boxSizing: 'border-box',
          borderRadius: 26,
          border: `2px solid ${C.border}`,
          background: C.card,
          opacity: pop(frame, fps, at(0.4)),
          transform: `scale(${0.9 + 0.1 * pop(frame, fps, at(0.4))})`,
          transformOrigin: 'left bottom',
        }}
      >
        <div style={{position: 'absolute', left: 30, top: 22, fontFamily: '"Helvetica Neue", Helvetica, Arial, sans-serif', fontSize: 24, fontWeight: 600, color: C.accent}}>
          instruction
        </div>
        <Bar x={30} y={88} w={480} />
        <Bar x={30} y={122} w={420} />
        <Bar x={30} y={156} w={300} />
      </div>

      <Pill x={1000} y={665} w={300} h={70} text="next function" color={C.accent} fontSize={30} scale={pop(frame, fps, at(0.56))} opacity={pop(frame, fps, at(0.56))} />

      <FileCard x={1450} y={600} w={300} h={190} label="function" opacity={pop(frame, fps, at(0.78))}>
        <Bar x={36} y={70} w={220} />
        <Bar x={36} y={100} w={160} />
      </FileCard>

      <Caption text={caption} frames={frames} />
    </AbsoluteFill>
  );
};
