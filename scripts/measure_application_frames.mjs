#!/usr/bin/env node
// Node >= 22, macOS/Linux. Only launches and terminates its own profiling process.
import { spawn } from 'node:child_process';
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';

export function validateFrameReport(report) {
  const count = report?.sample_count;
  if (report?.fixture !== 'native-application-ui-frames-v1' ||
      !Number.isInteger(count) || count < 1 || count > 10000 || report.frames?.length !== count ||
      !Number.isInteger(report.warmup) || report.warmup < 1 || report.warmup > 10000) {
    throw new Error('Incomplete application frame report');
  }
  let previous = -1;
  for (const frame of report.frames) {
    if (!Number.isSafeInteger(frame.observed_frame) || frame.observed_frame <= previous ||
        !Number.isFinite(frame.eframe_cpu_ms) || frame.eframe_cpu_ms < 0 ||
        !Number.isFinite(frame.ui_start_interval_ms) || frame.ui_start_interval_ms <= 0) {
      throw new Error('Invalid or repeated application frame sample');
    }
    previous = frame.observed_frame;
  }
  for (const name of ['eframe_cpu', 'ui_start_interval']) {
    const expected = report.frames.map(frame => frame[`${name}_ms`]).sort((a, b) => a - b);
    const stats = report[name];
    if (stats?.samples_ms?.length !== count || stats.samples_ms.some((v, i) => v !== expected[i]) ||
        stats.max_ms !== expected.at(-1)) throw new Error(`Invalid ${name} distribution`);
    for (const percentile of [50, 95, 99]) {
      if (stats[`p${percentile}_ms`] !== expected[Math.ceil(count * percentile / 100) - 1]) {
        throw new Error(`Invalid ${name} percentile`);
      }
    }
  }
  if (!report.adapter?.name || !report.adapter?.backend || !report.adapter?.device_type ||
      !Number.isFinite(report.pixels_per_point) || report.pixels_per_point <= 0 ||
      report.viewport_points?.length !== 2 || report.viewport_points.some(v => !Number.isFinite(v) || v <= 0) ||
      !/^[a-f0-9]{64}$/.test(report.graph_blake3 ?? '')) throw new Error('Missing frame environment metadata');
}

export async function runFrameProfile(binary, args, timeoutSeconds = 45) {
  if (!Number.isFinite(timeoutSeconds) || timeoutSeconds <= 0 || timeoutSeconds > 3600) {
    throw new Error('APP_FRAME_TIMEOUT_SECONDS must be in (0, 3600]');
  }
  const child = spawn(binary, args, { detached: true, stdio: ['ignore', 'pipe', 'pipe'] });
  let output = '';
  let diagnostics = '';
  let timer;
  let fail;
  const interrupted = signal => fail(new Error(`Frame profile interrupted by ${signal}`));
  process.on('SIGINT', interrupted);
  process.on('SIGTERM', interrupted);
  try {
    await new Promise((accept, reject) => {
      fail = reject;
      timer = setTimeout(() => reject(new Error(`Frame profile exceeded ${timeoutSeconds}s; no report accepted`)), timeoutSeconds * 1000);
      child.once('error', reject);
      child.stdout.on('data', chunk => {
        output += chunk;
        if (output.length > 4 * 1024 * 1024) reject(new Error('Frame profile exceeded output limit'));
      });
      child.stderr.on('data', chunk => { diagnostics = (diagnostics + chunk).slice(-4000); });
      child.once('close', code => code === 0 ? accept() : reject(new Error(`Frame profile exited (${code}): ${diagnostics}`)));
    });
    const lines = output.split('\n').filter(line => line.startsWith('APP_FRAME_PERFORMANCE '));
    if (lines.length !== 1) throw new Error('Expected exactly one complete application frame report');
    const report = JSON.parse(lines[0].slice('APP_FRAME_PERFORMANCE '.length));
    validateFrameReport(report);
    return report;
  } finally {
    clearTimeout(timer);
    process.off('SIGINT', interrupted);
    process.off('SIGTERM', interrupted);
    if (child.pid) {
      try { process.kill(-child.pid, 'SIGKILL'); } catch (error) { if (error.code !== 'ESRCH') throw error; }
    }
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  const [binary, graph, ...args] = process.argv.slice(2);
  try {
    if (!binary || !graph) throw new Error('Usage: measure_application_frames.mjs <logic-conduit> <graph.json> [profile-frames options]');
    const report = await runFrameProfile(resolve(binary), ['profile-frames', resolve(graph), ...args], Number(process.env.APP_FRAME_TIMEOUT_SECONDS ?? 45));
    process.stdout.write(`${JSON.stringify(report, null, 2)}\n`);
  } catch (error) {
    process.stderr.write(`${error.message}\n`);
    process.exitCode = 1;
  }
}
