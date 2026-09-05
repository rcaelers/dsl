import assert from 'node:assert/strict';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';

import { runFrameProfile, validateFrameReport } from './measure_application_frames.mjs';

function report() {
  const stats = (a, b) => ({ samples_ms: [a, b], p50_ms: a, p95_ms: b, p99_ms: b, max_ms: b });
  return { fixture: 'native-application-ui-frames-v1', sample_count: 2, warmup: 1,
    frames: [{ observed_frame: 2, eframe_cpu_ms: 1, ui_start_interval_ms: 16 },
      { observed_frame: 3, eframe_cpu_ms: 2, ui_start_interval_ms: 17 }],
    eframe_cpu: stats(1, 2), ui_start_interval: stats(16, 17),
    adapter: { name: 'Fixture adapter', backend: 'Metal', device_type: 'IntegratedGpu' },
    pixels_per_point: 2, viewport_points: [1440, 900], graph_blake3: 'a'.repeat(64) };
}

test('complete reports retain exact raw frames and nearest-rank distributions', () => validateFrameReport(report()));

test('partial, repeated, nonfinite, inconsistent and unidentified reports are rejected', () => {
  for (const change of [r => r.frames.pop(), r => { r.frames[1].observed_frame = 2; },
    r => { r.frames[0].eframe_cpu_ms = NaN; }, r => { r.frames[0].ui_start_interval_ms = 0; },
    r => { r.eframe_cpu.samples_ms[0] = 2; }, r => { r.eframe_cpu.p95_ms = 1; },
    r => { r.sample_count = 0; }, r => { r.warmup = 0; }, r => { delete r.adapter; },
    r => { r.graph_blake3 = ''; }, r => { r.viewport_points = [0, 900]; }]) {
    const value = report(); change(value);
    assert.throws(() => validateFrameReport(value));
  }
});

test('process completion is required and duplicate or missing reports are rejected', async () => {
  const line = `APP_FRAME_PERFORMANCE ${JSON.stringify(report())}`;
  assert.deepEqual(await runFrameProfile(process.execPath, ['-e', `console.log(${JSON.stringify(line)})`]), report());
  for (const source of ['process.exit(7)', 'console.log("partial")',
    `console.log(${JSON.stringify(`${line}\n${line}`)})`, `console.log(${JSON.stringify(line)});process.exit(7)`]) {
    await assert.rejects(runFrameProfile(process.execPath, ['-e', source]));
  }
});

test('deadline kills the owned process and its descendant even after a plausible report', { timeout: 10000 }, async () => {
  const directory = await mkdtemp(join(tmpdir(), 'app-frame-runner-test-'));
  const path = join(directory, 'pids');
  try {
    const source = `const {spawn}=require('node:child_process'); const child=spawn(process.execPath,['-e','setInterval(()=>{},1000)']);
      require('node:fs').writeFileSync(${JSON.stringify(path)}, process.pid+' '+child.pid);
      console.log(${JSON.stringify(`APP_FRAME_PERFORMANCE ${JSON.stringify(report())}`)}); setInterval(()=>{},1000);`;
    await assert.rejects(runFrameProfile(process.execPath, ['-e', source], 0.5), /exceeded/);
    // Let the host reap the terminated processes before checking their ids.
    await new Promise(accept => setTimeout(accept, 100));
    for (const id of (await readFile(path, 'utf8')).trim().split(' ')) {
      assert.throws(() => process.kill(Number(id), 0), { code: 'ESRCH' });
    }
  } finally { await rm(directory, { recursive: true, force: true }); }
});
