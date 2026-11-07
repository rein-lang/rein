import {Easing, interpolate, spring} from 'remotion';

export type Pt = [number, number];

// Eased 0 -> 1 between two frames. Clamped outside the window.
export const ramp = (frame: number, start: number, end: number): number =>
  interpolate(frame, [start, Math.max(end, start + 1)], [0, 1], {
    extrapolateLeft: 'clamp',
    extrapolateRight: 'clamp',
    easing: Easing.inOut(Easing.cubic),
  });

// Critically damped entrance (no overshoot). Holds 0 before the delay.
export const pop = (frame: number, fps: number, delay = 0): number =>
  spring({frame: Math.max(0, frame - delay), fps, config: {damping: 200, stiffness: 140}});

export const lerp = (a: number, b: number, t: number): number => a + (b - a) * t;

export const mixHex = (a: string, b: string, t: number): string => {
  const ch = (h: string, i: number) => parseInt(h.slice(i, i + 2), 16);
  const k = Math.min(1, Math.max(0, t));
  const r = Math.round(lerp(ch(a, 1), ch(b, 1), k));
  const g = Math.round(lerp(ch(a, 3), ch(b, 3), k));
  const bl = Math.round(lerp(ch(a, 5), ch(b, 5), k));
  return `rgb(${r},${g},${bl})`;
};

export const polyLength = (pts: Pt[]): number => {
  let total = 0;
  for (let i = 1; i < pts.length; i++) {
    total += Math.hypot(pts[i][0] - pts[i - 1][0], pts[i][1] - pts[i - 1][1]);
  }
  return total;
};

// Point at arc-length fraction s (0..1) along a polyline.
export const alongPolyline = (pts: Pt[], s: number): Pt => {
  let d = Math.min(1, Math.max(0, s)) * polyLength(pts);
  for (let i = 1; i < pts.length; i++) {
    const [ax, ay] = pts[i - 1];
    const [bx, by] = pts[i];
    const seg = Math.hypot(bx - ax, by - ay);
    if (d <= seg || i === pts.length - 1) {
      const t = seg === 0 ? 0 : Math.min(1, d / seg);
      return [ax + (bx - ax) * t, ay + (by - ay) * t];
    }
    d -= seg;
  }
  return pts[pts.length - 1];
};

export const polyPath = (pts: Pt[]): string =>
  'M ' + pts.map(([x, y]) => `${x} ${y}`).join(' L ');

// Filled triangle whose tip sits on `tip`, pointing away from `from`.
export const arrowPoints = (tip: Pt, from: Pt, size = 16): string => {
  const dx = tip[0] - from[0];
  const dy = tip[1] - from[1];
  const len = Math.hypot(dx, dy) || 1;
  const ux = dx / len;
  const uy = dy / len;
  const bx = tip[0] - ux * size;
  const by = tip[1] - uy * size;
  const px = -uy * size * 0.6;
  const py = ux * size * 0.6;
  return `${tip[0]},${tip[1]} ${bx + px},${by + py} ${bx - px},${by - py}`;
};
