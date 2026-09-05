#!/usr/bin/env node
// Node >= 22, macOS/Linux. Uses a fresh Chrome profile, never an existing session.
import { spawn } from 'node:child_process';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';

export function completedSummary(output) {
  const match = output.match(/test result: (ok|FAILED)\. (\d+) passed; (\d+) failed;/);
  if (!match) return null;
  if (match[1] !== 'ok' || Number(match[2]) !== 2 || Number(match[3]) !== 0) {
    throw new Error(`Browser routing tests did not both pass: ${match[0]}`);
  }
  return match[0];
}

export function validateReports(reports) {
  for (const marker of ['ROUTING_PERFORMANCE', 'ROUTING_DRAG_PERFORMANCE']) {
    const report = reports[marker];
    if (!report || report.reports?.length !== 2) throw new Error(`Missing complete ${marker}`);
    for (const [index, [nodes, connections]] of [[100, 500], [500, 2000]].entries()) {
      const fixture = report.reports[index];
      if (fixture.nodes !== nodes || fixture.connections !== connections) {
        throw new Error(`Unexpected fixture in ${marker}`);
      }
      const timers = marker === 'ROUTING_PERFORMANCE'
        ? ['routing', 'cached_routing', 'hover', 'cpu_frame', 'layout', 'ui', 'tessellation']
        : ['update', 'cold', 'layout'];
      for (const name of timers) {
        const samples = fixture[name]?.samples_ms;
        if (samples?.length !== 20 || samples.some(v => !Number.isFinite(v) || v < 0)) {
          throw new Error(`Expected twenty release samples for ${name} in ${marker}`);
        }
      }
      if (marker === 'ROUTING_DRAG_PERFORMANCE' &&
          (fixture.outcomes?.length !== 21 || !Number.isFinite(fixture.release_ms) || fixture.release_ms < 0)) {
        throw new Error('Missing drag outcomes or release measurement');
      }
    }
  }
}

async function measure(wasm) {
  const seconds = Number(process.env.ROUTING_BROWSER_TIMEOUT_SECONDS ?? 180);
  if (!Number.isFinite(seconds) || seconds <= 0 || seconds > 3600) {
    throw new Error('ROUTING_BROWSER_TIMEOUT_SECONDS must be in (0, 3600]');
  }
  if (!process.env.CHROME_BIN) throw new Error('Set CHROME_BIN to a Chrome executable');
  const profile = await mkdtemp(join(tmpdir(), 'node-graph-browser-'));
  const children = [];
  let socket;
  let deadline;
  const reports = {};
  let fail;
  const failure = new Promise((_, reject) => { fail = reject; });
  const interrupted = signal => fail(new Error(`Browser run interrupted by ${signal}; no baseline accepted`));
  process.on('SIGINT', interrupted);
  process.on('SIGTERM', interrupted);
  const run = async () => {
    const start = (command, args, env = process.env) => {
      const child = spawn(command, args, { env, detached: true, stdio: ['ignore', 'pipe', 'pipe'] });
      children.push(child);
      let diagnostics = '';
      const remember = chunk => { diagnostics = (diagnostics + chunk).slice(-4000); };
      child.stdout.on('data', remember);
      child.stderr.on('data', remember);
      child.once('error', fail);
      child.once('exit', code => fail(new Error(`Benchmark child exited unexpectedly (${code}): ${diagnostics}`)));
      return child;
    };
    const waitForLine = (child, pattern) => new Promise((accept, reject) => {
      let output = '';
      const receive = chunk => {
        output += chunk;
        const match = output.match(pattern);
        if (match) accept(match[1]);
      };
      child.stdout.on('data', receive);
      child.stderr.on('data', receive);
      child.once('error', reject);
      child.once('exit', code => reject(new Error(`Process exited (${code}): ${output.slice(-4000)}`)));
    });
    const runner = start(process.env.WASM_BINDGEN_TEST_RUNNER ?? 'wasm-bindgen-test-runner',
      [resolve(wasm), '_browser', '--nocapture'],
      { ...process.env, NO_HEADLESS: '1', WASM_BINDGEN_TEST_ADDRESS: '127.0.0.1:0' });
    const address = await waitForLine(runner, /available at (http:\/\/127\.0\.0\.1:\d+)/);
    const chrome = start(process.env.CHROME_BIN, [
      '--headless', '--no-first-run', '--no-default-browser-check',
      '--disable-background-networking', '--remote-debugging-address=127.0.0.1',
      '--remote-debugging-port=0', `--user-data-dir=${profile}`, 'about:blank',
    ]);
    const endpoint = await waitForLine(chrome, /DevTools listening on (ws:\/\/127\.0\.0\.1:\d+\/[^\s]+)/);
    socket = new WebSocket(endpoint);
    socket.addEventListener('close', () => fail(new Error('Browser connection closed before completion')));
    socket.addEventListener('error', () => fail(new Error('Browser connection failed')));
    await new Promise((accept, reject) => {
      socket.addEventListener('open', accept, { once: true });
      socket.addEventListener('error', reject, { once: true });
    });
    let sequence = 0;
    const pending = new Map();
    socket.addEventListener('message', event => {
      try {
        const message = JSON.parse(event.data);
        if (message.id) {
          const waiter = pending.get(message.id);
          pending.delete(message.id);
          if (message.error) waiter?.reject(new Error(JSON.stringify(message.error)));
          else waiter?.accept(message.result);
        } else if (message.method === 'Runtime.consoleAPICalled') {
          const line = message.params.args.map(arg => arg.value ?? arg.description).join(' ');
          for (const marker of ['ROUTING_PERFORMANCE', 'ROUTING_DRAG_PERFORMANCE']) {
            if (line.startsWith(`${marker} `)) {
              if (reports[marker]) throw new Error(`Duplicate ${marker}`);
              reports[marker] = JSON.parse(line.slice(marker.length + 1));
            }
          }
          if (line.startsWith('ROUTING_PROGRESS ')) process.stderr.write(`${line}\n`);
        }
      } catch (error) { fail(error); }
    });
    const send = (method, params = {}, sessionId) => new Promise((accept, reject) => {
      const id = ++sequence;
      pending.set(id, { accept, reject });
      socket.send(JSON.stringify({ id, method, params, sessionId }));
    });
    const browser = await send('Browser.getVersion');
    const { targetId } = await send('Target.createTarget', { url: 'about:blank' });
    const { sessionId } = await send('Target.attachToTarget', { targetId, flatten: true });
    await send('Runtime.enable', {}, sessionId);
    await send('Page.navigate', { url: address }, sessionId);
    for (;;) {
      const result = await send('Runtime.evaluate', {
        expression: 'document.getElementById("output")?.textContent ?? ""',
        returnByValue: true,
      }, sessionId);
      const summary = completedSummary(result.result?.value ?? '');
      if (summary) {
        validateReports(reports);
        return { browser, summary, ...reports };
      }
      await new Promise(accept => setTimeout(accept, 250));
    }
  };
  try {
    return await Promise.race([
      run(),
      failure,
      new Promise((_, reject) => {
        deadline = setTimeout(() => reject(new Error(`Browser run exceeded ${seconds}s; no baseline accepted`)), seconds * 1000);
      }),
    ]);
  } finally {
    clearTimeout(deadline);
    socket?.close();
    // Only terminate the process groups created above, including a blocked renderer.
    for (const child of children.reverse()) {
      if (!child.pid) continue;
      try { process.kill(-child.pid, 'SIGTERM'); } catch (error) {
        if (error.code !== 'ESRCH') throw error;
      }
    }
    await new Promise(accept => setTimeout(accept, 500));
    for (const child of children) {
      if (!child.pid) continue;
      try { process.kill(-child.pid, 'SIGKILL'); } catch (error) {
        if (error.code !== 'ESRCH') throw error;
      }
    }
    await rm(profile, { recursive: true, force: true });
    process.off('SIGINT', interrupted);
    process.off('SIGTERM', interrupted);
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  if (process.argv.length !== 3) {
    process.stderr.write('Usage: CHROME_BIN=... WASM_BINDGEN_TEST_RUNNER=... node scripts/measure_node_graph_browser.mjs <release-test.wasm>\n');
    process.exitCode = 1;
  } else {
    try {
      console.log(JSON.stringify(await measure(process.argv[2]), null, 2));
    } catch (error) {
      console.error(error.message);
      process.exitCode = 1;
    }
  }
}
