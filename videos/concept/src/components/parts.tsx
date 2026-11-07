import React from 'react';
import {AbsoluteFill, interpolate, useCurrentFrame, useVideoConfig} from 'remotion';
import {C, FONT} from '../theme';
import {mixHex, pop} from '../lib/motion';

export type BeatProps = {frames: number; caption: string};

export const Background: React.FC = () => (
  <AbsoluteFill
    style={{
      background: C.bg,
      backgroundImage: 'radial-gradient(rgba(139,148,158,0.13) 1.5px, transparent 1.5px)',
      backgroundSize: '48px 48px',
    }}
  />
);

// Spread onto an SVG <path> to draw it from 0 to p (0..1).
export const drawProps = (p: number) => ({
  pathLength: 1,
  strokeDasharray: 1,
  strokeDashoffset: 1 - p,
  opacity: p > 0 ? 1 : 0,
});

// Burned-in caption, bottom band, inside the 96 px safe margin.
export const Caption: React.FC<{text: string; frames: number}> = ({text, frames}) => {
  const frame = useCurrentFrame();
  const {fps} = useVideoConfig();
  const enter = pop(frame, fps, 6);
  const exit = interpolate(frame, [frames - 12, frames - 2], [1, 0], {
    extrapolateLeft: 'clamp',
    extrapolateRight: 'clamp',
  });
  return (
    <div
      style={{
        position: 'absolute',
        left: 0,
        right: 0,
        bottom: 96,
        display: 'flex',
        justifyContent: 'center',
        opacity: enter * exit,
        transform: `translateY(${(1 - enter) * 20}px)`,
      }}
    >
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 24,
          maxWidth: 1500,
          padding: '20px 44px',
          background: 'rgba(13,17,23,0.9)',
          border: `2px solid ${C.border}`,
          borderRadius: 20,
          boxShadow: '0 18px 40px rgba(0,0,0,0.45)',
        }}
      >
        <div style={{width: 8, height: 46, borderRadius: 4, background: C.accent, flexShrink: 0}} />
        <div style={{fontFamily: FONT, fontSize: 54, fontWeight: 650, color: C.text, lineHeight: 1.2}}>
          {text}
        </div>
      </div>
    </div>
  );
};

// The file-shaped "function" card. Children are positioned relative to the card.
export const FileCard: React.FC<{
  x: number;
  y: number;
  w: number;
  h: number;
  label?: string;
  labelColor?: string;
  glow?: number;
  opacity?: number;
  children?: React.ReactNode;
}> = ({x, y, w, h, label, labelColor, glow = 0, opacity = 1, children}) => (
  <div
    style={{
      position: 'absolute',
      left: x,
      top: y,
      width: w,
      height: h,
      boxSizing: 'border-box',
      borderRadius: 22,
      background: C.card,
      border: `2px solid ${mixHex(C.border, C.accent, glow)}`,
      boxShadow: `0 22px 44px rgba(0,0,0,0.5), 0 0 ${Math.round(glow * 40)}px rgba(45,212,191,${(glow * 0.35).toFixed(3)})`,
      opacity,
    }}
  >
    {label ? (
      <div
        style={{
          position: 'absolute',
          left: 28,
          top: 20,
          fontFamily: FONT,
          fontSize: 24,
          fontWeight: 600,
          letterSpacing: 0.4,
          color: labelColor ?? C.muted,
        }}
      >
        {label}
      </div>
    ) : null}
    {children}
  </div>
);

// Neutral placeholder line. Stands in for text, never for code.
export const Bar: React.FC<{x: number; y: number; w: number; h?: number; color?: string; opacity?: number}> = ({
  x,
  y,
  w,
  h = 14,
  color = C.faint,
  opacity = 0.55,
}) => (
  <div
    style={{
      position: 'absolute',
      left: x,
      top: y,
      width: w,
      height: h,
      borderRadius: h / 2,
      background: color,
      opacity,
    }}
  />
);

export const Pill: React.FC<{
  x: number;
  y: number;
  w: number;
  h: number;
  text?: string;
  color?: string;
  fill?: string;
  scale?: number;
  opacity?: number;
  fontSize?: number;
  textColor?: string;
  radius?: number;
}> = ({x, y, w, h, text, color = C.border, fill = C.card, scale = 1, opacity = 1, fontSize = 34, textColor, radius}) => (
  <div
    style={{
      position: 'absolute',
      left: x,
      top: y,
      width: w,
      height: h,
      boxSizing: 'border-box',
      borderRadius: radius ?? h / 2,
      border: `3px solid ${color}`,
      background: fill,
      display: 'flex',
      alignItems: 'center',
      justifyContent: 'center',
      fontFamily: FONT,
      fontSize,
      fontWeight: 650,
      color: textColor ?? C.text,
      opacity,
      transform: `scale(${scale})`,
    }}
  >
    {text}
  </div>
);

export const CheckIcon: React.FC<{size: number; color?: string}> = ({size, color = C.accent}) => (
  <svg width={size} height={size} viewBox="0 0 24 24">
    <circle cx="12" cy="12" r="12" fill={color} />
    <path
      d="M6.8 12.6 L10.3 16 L17.4 8.8"
      fill="none"
      stroke={C.bg}
      strokeWidth={2.6}
      strokeLinecap="round"
      strokeLinejoin="round"
    />
  </svg>
);

export const CrossIcon: React.FC<{size: number; color?: string}> = ({size, color = C.muted}) => (
  <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke={color} strokeWidth={2.2} strokeLinecap="round">
    <circle cx="12" cy="12" r="10.5" />
    <path d="M8.5 8.5 L15.5 15.5 M15.5 8.5 L8.5 15.5" />
  </svg>
);

export const QuestionIcon: React.FC<{size: number; color?: string}> = ({size, color = C.accent}) => (
  <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke={color} strokeWidth={2.2} strokeLinecap="round" strokeLinejoin="round">
    <circle cx="12" cy="12" r="10" />
    <path d="M9.2 9.3a2.8 2.8 0 1 1 3.9 2.6c-.8.4-1.1.9-1.1 1.7v.4" />
    <circle cx="12" cy="17.2" r="0.7" fill={color} />
  </svg>
);
