import React from 'react';
import {AbsoluteFill, useCurrentFrame, useVideoConfig} from 'remotion';
import {C} from '../theme';
import {arrowPoints, pop, ramp} from '../lib/motion';
import {Background, Bar, BeatProps, Caption, drawProps, FileCard, Pill} from '../components/parts';

// Body of a function: one path, a split on if, a switch, then a loop that is crossed out.
export const Beat5Moves: React.FC<BeatProps> = ({frames, caption}) => {
  const frame = useCurrentFrame();
  const {fps} = useVideoConfig();
  const at = (p: number) => p * frames;

  const loopEnd: [number, number] = [1150, 300];
  const crossOn = pop(frame, fps, at(0.86));

  return (
    <AbsoluteFill>
      <Background />
      <svg width={1920} height={1080} style={{position: 'absolute', left: 0, top: 0}}>
        <path d="M 960 348 L 960 404" stroke={C.line} strokeWidth={3} fill="none" {...drawProps(ramp(frame, at(0.12), at(0.2)))} />
        <path d="M 960 476 C 960 520, 620 500, 620 555" stroke={C.line} strokeWidth={3} fill="none" {...drawProps(ramp(frame, at(0.24), at(0.34)))} />
        <path d="M 960 476 C 960 520, 1300 500, 1300 584" stroke={C.line} strokeWidth={3} fill="none" {...drawProps(ramp(frame, at(0.4), at(0.47)))} />
        <path d="M 1300 656 C 1300 700, 1080 690, 1080 730" stroke={C.line} strokeWidth={3} fill="none" {...drawProps(ramp(frame, at(0.54), at(0.58)))} />
        <path d="M 1300 656 L 1300 730" stroke={C.line} strokeWidth={3} fill="none" {...drawProps(ramp(frame, at(0.56), at(0.6)))} />
        <path d="M 1300 656 C 1300 700, 1520 690, 1520 730" stroke={C.line} strokeWidth={3} fill="none" {...drawProps(ramp(frame, at(0.58), at(0.62)))} />
        <path
          d="M 1595 770 C 1720 800, 1760 300, 1150 300"
          stroke={C.muted}
          strokeWidth={3}
          strokeLinecap="round"
          fill="none"
          {...drawProps(ramp(frame, at(0.7), at(0.84)))}
        />
        <polygon points={arrowPoints(loopEnd, [1760, 300])} fill={C.muted} opacity={ramp(frame, at(0.82), at(0.84))} />
        <g opacity={crossOn} transform={`translate(1648 546) scale(${0.6 + 0.4 * crossOn}) translate(-1648 -546)`}>
          <line x1={1620} y1={518} x2={1676} y2={574} stroke={C.danger} strokeWidth={7} strokeLinecap="round" />
          <line x1={1676} y1={518} x2={1620} y2={574} stroke={C.danger} strokeWidth={7} strokeLinecap="round" />
        </g>
      </svg>

      <Pill x={770} y={252} w={380} h={96} text="function" radius={22} scale={pop(frame, fps, at(0.04))} opacity={pop(frame, fps, at(0.04))} fontSize={36} />
      <Pill x={885} y={404} w={150} h={72} text="if" color={C.accent} scale={pop(frame, fps, at(0.2))} opacity={pop(frame, fps, at(0.2))} fontSize={36} />

      <FileCard x={480} y={555} w={280} h={130} opacity={pop(frame, fps, at(0.36))}>
        <Bar x={36} y={40} w={200} />
        <Bar x={36} y={70} w={160} />
        <Bar x={36} y={100} w={220} />
      </FileCard>

      <Pill x={1210} y={584} w={180} h={72} text="switch" color={C.accent} scale={pop(frame, fps, at(0.46))} opacity={pop(frame, fps, at(0.46))} fontSize={36} />

      {[1080, 1300, 1520].map((cx, i) => (
        <FileCard key={cx} x={cx - 75} y={730} w={150} h={80} opacity={pop(frame, fps, at(0.58 + i * 0.03))}>
          <Bar x={30} y={34} w={90} h={12} />
        </FileCard>
      ))}

      <Caption text={caption} frames={frames} />
    </AbsoluteFill>
  );
};
