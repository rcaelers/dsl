#!/usr/bin/env node
// macOS, Node 22+, Xcode Instruments. Attaches only to the process launched here.
import { spawn } from 'node:child_process';
import { access } from 'node:fs/promises';
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';

import { validateFrameReport } from './measure_application_frames.mjs';

export function metalProfileArgs(graph, args = []) {
  const defaults = [['--warmup', '30'], ['--samples', '120'], ['--minimum-seconds', '20']];
  return ['profile-frames', graph, ...defaults.filter(([flag]) =>
    !args.some(arg => arg === flag || arg.startsWith(`${flag}=`))).flat(), ...args];
}

export async function runMetalProfile(binary, args, tracePath, options = {}) {
  const { timeoutSeconds = 45, attachDelayMs = 3000,
    traceCommand = pid => ['/usr/bin/xcrun', ['xctrace', 'record', '--template', 'Metal System Trace',
      '--attach', String(pid), '--time-limit', '5s', '--no-prompt', '--output', tracePath]] } = options;
  if (!Number.isFinite(timeoutSeconds) || timeoutSeconds <= 0 || timeoutSeconds > 3600 ||
      !Number.isFinite(attachDelayMs) || attachDelayMs < 0 || attachDelayMs >= timeoutSeconds * 1000) {
    throw new Error('Invalid trace deadline or attachment delay');
  }
  // A trace directory may contain a large capture; never append to or overwrite one.
  await access(tracePath).then(() => { throw new Error('Trace output already exists'); }, error => {
    if (error.code !== 'ENOENT') throw error;
  });
  const owned = [];
  let timer;
  let delay;
  let fail;
  let phase = 'application startup';
  const failure = new Promise((_, reject) => { fail = reject; });
  const interrupted = signal => fail(new Error(`Metal profile interrupted by ${signal}`));
  const launch = (command, argv, onOutput = () => {}) => {
    const child = spawn(command, argv, { detached: true, stdio: ['ignore', 'pipe', 'pipe'] });
    owned.push(child);
    let output = '';
    let diagnostics = '';
    const done = new Promise(accept => {
      child.once('error', fail);
      child.stdout.on('data', data => {
        output += data;
        onOutput(output);
        if (output.length > 4 * 1024 * 1024) fail(new Error('Metal profile exceeded output limit'));
      });
      child.stderr.on('data', data => { diagnostics = (diagnostics + data).slice(-4000); });
      child.once('close', code => {
        if (code !== 0) fail(new Error(`Profile process exited (${code}): ${diagnostics}`));
        accept({ code, output, diagnostics });
      });
    });
    return { child, done };
  };
  process.on('SIGINT', interrupted);
  process.on('SIGTERM', interrupted);
  try {
    return await Promise.race([
      (async () => {
        const app = launch(binary, args);
        let appClosed = false;
        app.child.once('close', () => { appClosed = true; });
        await new Promise(accept => { delay = setTimeout(accept, attachDelayMs); });
        if (appClosed) throw new Error('Application ended before trace attachment');
        const [command, argv] = traceCommand(app.child.pid);
        phase = 'trace recording and save';
        let recordedWithLiveApplication = false;
        const trace = launch(command, argv, output => {
          if (!appClosed && output.includes('Reached specified time limit, ending recording...')) {
            recordedWithLiveApplication = true;
          }
        });
        const traceResult = await trace.done;
        // Saving can outlive the target. Require the target through recording,
        // not through trace serialization; a successful CPU report alone is insufficient.
        if (!recordedWithLiveApplication) throw new Error('Trace did not reach its time limit with a live application');
        await access(tracePath);
        phase = 'application completion';
        const appResult = await app.done;
        const lines = appResult.output.split('\n').filter(line => line.startsWith('APP_FRAME_PERFORMANCE '));
        if (lines.length !== 1) throw new Error('Expected exactly one complete application report');
        const report = JSON.parse(lines[0].slice('APP_FRAME_PERFORMANCE '.length));
        validateFrameReport(report);
        return { fixture: 'native-application-metal-capture-v1', application_pid: app.child.pid,
          attach_delay_ms: attachDelayMs, trace: tracePath, trace_output: traceResult.output,
          trace_diagnostics: traceResult.diagnostics, application: report,
          scope: 'Five-second process-targeted Metal trace; application CPU observations overlap instrumentation and are not an uninstrumented baseline. Raw trace may contain system-wide metadata; export and retain only target-process records.' };
      })(),
      failure,
      new Promise((_, reject) => {
        timer = setTimeout(() => reject(new Error(`Metal profile deadline exceeded during ${phase}; no capture accepted`)), timeoutSeconds * 1000);
      }),
    ]);
  } finally {
    clearTimeout(timer);
    clearTimeout(delay);
    process.off('SIGINT', interrupted);
    process.off('SIGTERM', interrupted);
    for (const child of owned.reverse()) if (child.pid) {
      try { process.kill(-child.pid, 'SIGKILL'); } catch (error) { if (error.code !== 'ESRCH') throw error; }
    }
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  const [binary, graph, trace, ...args] = process.argv.slice(2);
  try {
    if (!binary || !graph || !trace) throw new Error('Usage: measure_application_metal.mjs <logic-conduit> <graph.json> <new.trace> [profile-frames options]');
    const report = await runMetalProfile(resolve(binary), metalProfileArgs(resolve(graph), args), resolve(trace),
    { timeoutSeconds: Number(process.env.APP_FRAME_TIMEOUT_SECONDS ?? 45) });
    console.log(JSON.stringify(report, null, 2));
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
