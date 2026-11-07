import React from 'react';
import {AbsoluteFill, Audio, Sequence, staticFile} from 'remotion';
import beats from '../narration/beats.json';
import timing from './timing.json';
import {C} from './theme';
import {BeatProps} from './components/parts';
import {Beat1Hook} from './beats/Beat1Hook';
import {Beat2Turn} from './beats/Beat2Turn';
import {Beat3FunctionIsFile} from './beats/Beat3FunctionIsFile';
import {Beat4Questions} from './beats/Beat4Questions';
import {Beat5Moves} from './beats/Beat5Moves';
import {Beat6Returns} from './beats/Beat6Returns';
import {Beat7Flow} from './beats/Beat7Flow';
import {Beat8Safety} from './beats/Beat8Safety';
import {Beat9Close} from './beats/Beat9Close';

const VISUALS: Record<number, React.FC<BeatProps>> = {
  1: Beat1Hook,
  2: Beat2Turn,
  3: Beat3FunctionIsFile,
  4: Beat4Questions,
  5: Beat5Moves,
  6: Beat6Returns,
  7: Beat7Flow,
  8: Beat8Safety,
  9: Beat9Close,
};

// Each beat lasts exactly as long as its padded narration clip (see scripts/tts.py).
export const TOTAL_FRAMES: number = timing.reduce((sum, t) => sum + t.frames, 0);

export const Concept: React.FC = () => {
  let from = 0;
  return (
    <AbsoluteFill style={{background: C.bg}}>
      {timing.map((t) => {
        const beat = beats.find((b) => b.id === t.id);
        const Visual = VISUALS[t.id];
        if (!beat || !Visual) {
          throw new Error(`No beat or visual for id ${t.id}`);
        }
        const start = from;
        from += t.frames;
        return (
          <Sequence key={t.id} from={start} durationInFrames={t.frames}>
            <Visual frames={t.frames} caption={beat.caption} />
            <Audio src={staticFile(`audio/beat-${t.id}.wav`)} />
          </Sequence>
        );
      })}
    </AbsoluteFill>
  );
};
