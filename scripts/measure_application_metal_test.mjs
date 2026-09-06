import assert from 'node:assert/strict';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';

import { metalProfileArgs, runMetalProfile } from './measure_application_metal.mjs';

function report() {
  const stats = value => ({ samples_ms: [value], p50_ms: value, p95_ms: value, p99_ms: value, max_ms: value });
  return { fixture: 'native-application-ui-frames-v1', sample_count: 1, warmup: 1,
    frames: [{ observed_frame: 2, eframe_cpu_ms: 1, ui_start_interval_ms: 16 }],
    eframe_cpu: stats(1), ui_start_interval: stats(16),
    adapter: { name: 'Fixture', backend: 'Metal', device_type: 'IntegratedGpu' },
    pixels_per_point: 2, viewport_points: [1440, 900], graph_blake3: 'a'.repeat(64) };
}

const line = `APP_FRAME_PERFORMANCE ${JSON.stringify(report())}`;
const app = `console.log(${JSON.stringify(line)}); setTimeout(()=>{},500);`;

test('CLI defaults permit explicit profiling overrides without duplicate flags', () => {
  assert.deepEqual(metalProfileArgs('graph'), ['profile-frames', 'graph', '--warmup', '30',
    '--samples', '120', '--minimum-seconds', '20']);
  assert.deepEqual(metalProfileArgs('graph', ['--samples=500', '--minimum-seconds', '30']),
    ['profile-frames', 'graph', '--warmup', '30', '--samples=500', '--minimum-seconds', '30']);
});

test('completed trace and validated application report are both required', async () => {
  const dir = await mkdtemp(join(tmpdir(), 'metal-runner-test-'));
  try {
    const path = join(dir, 'new.trace');
    const options = { attachDelayMs: 10, traceCommand: pid => {
      assert.ok(Number.isInteger(pid) && pid > 0);
      return [process.execPath, ['-e', `require('node:fs').mkdirSync(${JSON.stringify(path)});console.log('Reached specified time limit, ending recording...')`]];
    } };
    const value = await runMetalProfile(process.execPath, ['-e', app], path, options);
    assert.deepEqual(value.application, report());
    await assert.rejects(runMetalProfile(process.execPath, ['-e', app], path, options), /already exists/);
    for (const [source, traceSource] of [
      [app, 'process.exit(7)'],
      [app, ''], // A zero exit without a trace is insufficient.
      ['process.exit(7)', ''],
      ['console.log("partial");setTimeout(()=>{},500)', 'mkdir'],
      [`console.log(${JSON.stringify(`${line}\n${line}`)});setTimeout(()=>{},500)`, 'mkdir'],
    ]) {
      const target = join(dir, `case-${Math.random()}.trace`);
      await assert.rejects(runMetalProfile(process.execPath, ['-e', source], target, {
        attachDelayMs: 10, traceCommand: () => [process.execPath, ['-e', traceSource === 'mkdir'
          ? `require('node:fs').mkdirSync(${JSON.stringify(target)});console.log('Reached specified time limit, ending recording...')` : traceSource]],
      }));
    }
    await assert.rejects(runMetalProfile(process.execPath, ['-e', ''], join(dir, 'early.trace'),
      { attachDelayMs: 200 }), /ended before trace attachment/);
    await assert.rejects(runMetalProfile(process.execPath, ['-e', app], join(dir, 'late.trace'),
      { attachDelayMs: 10, traceCommand: () => [process.execPath, ['-e', "setTimeout(()=>console.log('Reached specified time limit, ending recording...'),800)"]] }), /live application/);
    const saved = join(dir, 'slow-save.trace');
    const slowSave = await runMetalProfile(process.execPath, ['-e', app], saved,
      { attachDelayMs: 10, traceCommand: () => [process.execPath, ['-e',
        `require('node:fs').mkdirSync(${JSON.stringify(saved)});console.log('Reached specified time limit, ending recording...');setTimeout(()=>{},800)`]] });
    assert.deepEqual(slowSave.application, report());
  } finally { await rm(dir, { recursive: true, force: true }); }
});

test('deadline cleans up both owned groups and their descendants', { timeout: 10000 }, async () => {
  const dir = await mkdtemp(join(tmpdir(), 'metal-runner-timeout-'));
  const source = path => `const {spawn}=require('node:child_process');const c=spawn(process.execPath,['-e','setInterval(()=>{},1000)']);
    require('node:fs').writeFileSync(${JSON.stringify(path)},process.pid+' '+c.pid);setInterval(()=>{},1000);`;
  try {
    await assert.rejects(runMetalProfile(process.execPath, ['-e', source(join(dir, 'app-pids'))], join(dir, 'new.trace'), {
      attachDelayMs: 50, timeoutSeconds: 0.7,
      traceCommand: () => [process.execPath, ['-e', source(join(dir, 'trace-pids'))]],
    }), /deadline exceeded/);
    await new Promise(accept => setTimeout(accept, 100));
    for (const name of ['app-pids', 'trace-pids']) for (const id of (await readFile(join(dir, name), 'utf8')).split(' ')) {
      assert.throws(() => process.kill(Number(id), 0), { code: 'ESRCH' });
    }
    for (const options of [{ timeoutSeconds: NaN }, { timeoutSeconds: 0 }, { attachDelayMs: -1 },
      { timeoutSeconds: 1, attachDelayMs: 1000 }]) {
      await assert.rejects(runMetalProfile('', [], join(dir, 'invalid.trace'), options), /Invalid trace/);
    }
  } finally { await rm(dir, { recursive: true, force: true }); }
});
