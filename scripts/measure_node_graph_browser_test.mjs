import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';

import { completedSummary, validateReports } from './measure_node_graph_browser.mjs';

test('completion requires both passing tests, not progress or a single test', () => {
  assert.equal(completedSummary('running 2 tests\nROUTING_PROGRESS {}'), null);
  assert.ok(completedSummary('test result: ok. 2 passed; 0 failed; 0 ignored;'));
  for (const output of ['ok. 1 passed; 0 failed;', 'ok. 0 passed; 0 failed;', 'FAILED. 1 passed; 1 failed;']) {
    assert.throws(() => completedSummary(`test result: ${output}`));
  }
});

function reports() {
  const timer = () => ({ samples_ms: Array(20).fill(1) });
  const fixtures = [[100, 500], [500, 2000]];
  return {
    ROUTING_PERFORMANCE: { reports: fixtures.map(([nodes, connections]) => ({
      nodes, connections,
      ...Object.fromEntries(['routing', 'cached_routing', 'hover', 'cpu_frame', 'layout', 'ui', 'tessellation']
        .map(name => [name, timer()])),
    })) },
    ROUTING_DRAG_PERFORMANCE: { reports: fixtures.map(([nodes, connections]) => ({
      nodes, connections, update: timer(), cold: timer(), layout: timer(),
      release_ms: 1, outcomes: Array(21).fill({}),
    })) },
  };
}

test('complete release samples are accepted', () => validateReports(reports()));

test('partial, debug, mismatched, nonfinite and incomplete drag reports are rejected', () => {
  for (const mutate of [
    value => { delete value.ROUTING_DRAG_PERFORMANCE; },
    value => { value.ROUTING_PERFORMANCE.reports.pop(); },
    value => { value.ROUTING_PERFORMANCE.reports[0].nodes = 99; },
    value => { value.ROUTING_PERFORMANCE.reports[0].routing.samples_ms = [1]; },
    value => { value.ROUTING_PERFORMANCE.reports[0].ui.samples_ms[0] = NaN; },
    value => { value.ROUTING_DRAG_PERFORMANCE.reports[1].outcomes.pop(); },
    value => { delete value.ROUTING_DRAG_PERFORMANCE.reports[0].release_ms; },
  ]) {
    const value = reports();
    mutate(value);
    assert.throws(() => validateReports(value));
  }
});

async function runFixture(source, seconds) {
  const directory = await mkdtemp(join(tmpdir(), 'browser-runner-test-'));
  const fixture = join(directory, 'fixture.mjs');
  const pids = join(directory, 'pids');
  await writeFile(fixture, source);
  const start = performance.now();
  try {
    const child = spawn(process.execPath, [fileURLToPath(new URL('./measure_node_graph_browser.mjs', import.meta.url)), fixture], {
      env: { ...process.env, CHROME_BIN: process.execPath, WASM_BINDGEN_TEST_RUNNER: process.execPath,
        ROUTING_BROWSER_TIMEOUT_SECONDS: String(seconds), FIXTURE_PIDS: pids },
      stdio: ['ignore', 'pipe', 'pipe'],
    });
    let stdout = '';
    let stderr = '';
    child.stdout.on('data', chunk => { stdout += chunk; });
    child.stderr.on('data', chunk => { stderr += chunk; });
    const code = await new Promise((accept, reject) => {
      child.once('error', reject);
      child.once('exit', accept);
    });
    assert.equal(code, 1);
    assert.equal(stdout, '', 'failed execution must not emit an accepted baseline');
    assert.ok(performance.now() - start < 5000, 'runner must terminate promptly');
    const ids = await readFile(pids, 'utf8').catch(() => '');
    for (const id of ids.trim().split(/\s+/).filter(Boolean)) {
      assert.throws(() => process.kill(Number(id), 0), { code: 'ESRCH' });
    }
    return stderr;
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
}

test('timeout kills its stalled runner and descendant without accepting partial output', { timeout: 10000 }, async () => {
  const stderr = await runFixture(`
    import { spawn } from 'node:child_process';
    import { writeFileSync } from 'node:fs';
    const child = spawn(process.execPath, ['-e', 'setInterval(() => {}, 1000)']);
    writeFileSync(process.env.FIXTURE_PIDS, process.pid + '\\n' + child.pid);
    setInterval(() => {}, 1000);
  `, 1);
  assert.match(stderr, /exceeded 1s; no baseline accepted/);
});

test('early runner exit fails promptly', { timeout: 10000 }, async () => {
  const stderr = await runFixture('process.exit(7);', 10);
  assert.match(stderr, /exited.*\(7\)/);
});
