// One still per beat, taken at 92% through each beat. Bundles once for all nine.
import {mkdirSync, readFileSync} from 'node:fs';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
import {bundle} from '@remotion/bundler';
import {renderStill, selectComposition} from '@remotion/renderer';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const outDir = path.join(root, 'out', 'stills');
mkdirSync(outDir, {recursive: true});

const timing = JSON.parse(readFileSync(path.join(root, 'src', 'timing.json'), 'utf8'));
const serveUrl = await bundle({entryPoint: path.join(root, 'src', 'index.ts')});
const composition = await selectComposition({serveUrl, id: 'Concept'});

let start = 0;
for (const beat of timing) {
  const frame = start + Math.floor(beat.frames * 0.92);
  const output = path.join(outDir, `beat-${beat.id}.png`);
  await renderStill({composition, serveUrl, output, frame, imageFormat: 'png'});
  console.log(`beat ${beat.id} frame ${frame} -> ${output}`);
  start += beat.frames;
}
