let runtime = null;
const cancelled = new Set();
const active = new Set();

function sequenceOf(message) {
  return String(message.sequence);
}

async function initialize(message) {
  try {
    const module = await import(message.moduleUrl);
    await module.default({ module_or_path: message.wasmUrl });
    runtime = module;
    self.postMessage({ kind: "ready" });
  } catch (error) {
    self.postMessage({
      kind: "worker_failed",
      message: error instanceof Error ? error.message : String(error),
    });
  }
}

function run(message) {
  const sequence = sequenceOf(message);
  active.add(sequence);
  try {
    self.postMessage({ kind: "progress", sequence, completed: "0", total: "1" });
    const input = new Uint8Array(message.payload);
    const output = runtime.executePortableWorkerOperation(message.operation, input);
    if (cancelled.delete(sequence)) {
      return;
    }
    const payload = output.byteOffset === 0 && output.byteLength === output.buffer.byteLength
      ? output.buffer
      : output.buffer.slice(output.byteOffset, output.byteOffset + output.byteLength);
    self.postMessage({ kind: "progress", sequence, completed: "1", total: "1" });
    self.postMessage({ kind: "complete", sequence, payload }, [payload]);
  } catch (error) {
    if (!cancelled.delete(sequence)) {
      self.postMessage({
        kind: "failed",
        sequence,
        message: error instanceof Error ? error.message : String(error),
      });
    }
  } finally {
    active.delete(sequence);
    cancelled.delete(sequence);
  }
}

self.onmessage = (event) => {
  const message = event.data;
  switch (message.kind) {
    case "initialize":
      void initialize(message);
      break;
    case "run":
      if (runtime === null) {
        self.postMessage({
          kind: "failed",
          sequence: sequenceOf(message),
          message: "worker runtime is not initialized",
        });
      } else {
        run(message);
      }
      break;
    case "cancel":
      if (active.has(sequenceOf(message))) {
        cancelled.add(sequenceOf(message));
      }
      break;
  }
};
