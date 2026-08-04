let runtime = null;
const cancelled = new Set();
const active = new Set();
const captureFiles = new Map();
const cancelledCaptures = new Set();
const pendingCaptureAttachments = [];
const captureImportChunkBytes = 4 * 1024 * 1024;
let captureAdvanceScheduled = false;
let graphAdvanceScheduled = false;

function workerFailureMessage(error) {
  return typeof globalThis.logicConduitWorkerPanic === "string"
    ? globalThis.logicConduitWorkerPanic
    : error instanceof Error
      ? error.message
      : String(error);
}

globalThis.logicConduitReadCaptureRange = (reference, offset, length) => {
  const file = captureFiles.get(reference);
  if (file === undefined) {
    throw new Error(`browser capture '${reference}' is not attached to this worker`);
  }
  const reader = new FileReaderSync();
  return new Uint8Array(reader.readAsArrayBuffer(file.slice(offset, offset + length)));
};

function sequenceOf(message) {
  return String(message.sequence);
}

async function initialize(message) {
  try {
    const module = await import(message.moduleUrl);
    await module.default({ module_or_path: message.wasmUrl });
    module.initializeWorkerHost();
    runtime = module;
    self.postMessage({ kind: "ready" });
    for (const attachment of pendingCaptureAttachments.splice(0)) {
      void attachCapture(attachment);
    }
  } catch (error) {
    self.postMessage({
      kind: "worker_failed",
      message: workerFailureMessage(error),
    });
  }
}

function transferableBuffer(bytes) {
  return bytes.byteOffset === 0 && bytes.byteLength === bytes.buffer.byteLength
    ? bytes.buffer
    : bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength);
}

async function attachCapture(message) {
  const reference = String(message.reference);
  const file = message.file;
  captureFiles.set(reference, file);
  let identityHandle = null;
  try {
    identityHandle = runtime.beginCaptureIdentity();
    let completed = 0;
    while (completed < file.size) {
      if (cancelledCaptures.has(reference)) {
        runtime.cancelCaptureIdentity(identityHandle);
        identityHandle = null;
        captureFiles.delete(reference);
        return;
      }
      const end = Math.min(completed + captureImportChunkBytes, file.size);
      const bytes = new Uint8Array(await file.slice(completed, end).arrayBuffer());
      runtime.updateCaptureIdentity(identityHandle, bytes);
      completed = end;
      self.postMessage({
        kind: "capture_attach_progress",
        reference,
        completed: String(completed),
        total: String(file.size),
      });
    }
    const identity = runtime.finishCaptureIdentity(identityHandle);
    identityHandle = null;
    if (cancelledCaptures.has(reference)) {
      captureFiles.delete(reference);
      return;
    }
    const metadata = runtime.inspectCaptureFile(
      reference,
      String(message.displayName),
      identity,
      file.size,
    );
    const identityBuffer = transferableBuffer(identity);
    const metadataBuffer = transferableBuffer(metadata);
    self.postMessage(
      {
        kind: "capture_attached",
        reference,
        identity: identityBuffer,
        metadata: metadataBuffer,
      },
      [identityBuffer, metadataBuffer],
    );
  } catch (error) {
    if (identityHandle !== null) {
      runtime.cancelCaptureIdentity(identityHandle);
    }
    captureFiles.delete(reference);
    if (!cancelledCaptures.has(reference)) {
      self.postMessage({
        kind: "capture_attach_failed",
        reference,
        message: workerFailureMessage(error),
      });
    }
  } finally {
    cancelledCaptures.delete(reference);
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
    const payload = transferableBuffer(output);
    self.postMessage({ kind: "progress", sequence, completed: "1", total: "1" });
    self.postMessage({ kind: "complete", sequence, payload }, [payload]);
  } catch (error) {
    if (!cancelled.delete(sequence)) {
      self.postMessage({
        kind: "failed",
        sequence,
        message: workerFailureMessage(error),
      });
    }
  } finally {
    active.delete(sequence);
    cancelled.delete(sequence);
  }
}

function runCapture(message) {
  try {
    const input = new Uint8Array(message.payload);
    const pending = runtime.executeCaptureWorkerRequest(input, publishCaptureOutput);
    if (pending) {
      scheduleCaptureAdvance();
    }
  } catch (error) {
    self.postMessage({
      kind: "worker_failed",
      message: workerFailureMessage(error),
    });
  }
}

function publishCaptureOutput(output) {
  const payload = transferableBuffer(output);
  self.postMessage({ kind: "capture_messages", payload }, [payload]);
}

function scheduleCaptureAdvance() {
  if (captureAdvanceScheduled) {
    return;
  }
  captureAdvanceScheduled = true;
  setTimeout(() => {
    captureAdvanceScheduled = false;
    try {
      if (runtime.advanceCaptureWorkerPreparation(publishCaptureOutput)) {
        scheduleCaptureAdvance();
      }
    } catch (error) {
      self.postMessage({
        kind: "worker_failed",
        message: workerFailureMessage(error),
      });
    }
  }, 0);
}

function runGraph(message) {
  try {
    const input = new Uint8Array(message.payload);
    const active = runtime.executeGraphWorkerRequest(input, publishGraphOutput);
    if (active) {
      scheduleGraphAdvance();
    } else {
      publishBrowserOutputs();
    }
  } catch (error) {
    self.postMessage({
      kind: "worker_failed",
      message: workerFailureMessage(error),
    });
  }
}

function publishGraphOutput(output) {
  const payload = transferableBuffer(output);
  self.postMessage({ kind: "graph_messages", payload }, [payload]);
}

function scheduleGraphAdvance() {
  if (graphAdvanceScheduled) {
    return;
  }
  graphAdvanceScheduled = true;
  setTimeout(() => {
    graphAdvanceScheduled = false;
    try {
      if (runtime.advanceGraphWorkerRun(publishGraphOutput)) {
        scheduleGraphAdvance();
      } else {
        publishBrowserOutputs();
      }
    } catch (error) {
      self.postMessage({
        kind: "worker_failed",
        message: workerFailureMessage(error),
      });
    }
  }, 0);
}

function publishBrowserOutputs() {
  const payload = runtime.takeBrowserOutputFiles();
  if (payload.byteLength === 2) {
    return;
  }
  const buffer = transferableBuffer(payload);
  self.postMessage({ kind: "graph_output_files", payload: buffer }, [buffer]);
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
    case "capture_run":
      if (runtime === null) {
        self.postMessage({
          kind: "worker_failed",
          message: "capture worker runtime is not initialized",
        });
      } else {
        runCapture(message);
      }
      break;
    case "graph_run":
      if (runtime === null) {
        self.postMessage({
          kind: "worker_failed",
          message: "capture worker runtime is not initialized",
        });
      } else {
        runGraph(message);
      }
      break;
    case "capture_attach":
      if (runtime === null) {
        pendingCaptureAttachments.push(message);
      } else {
        void attachCapture(message);
      }
      break;
    case "capture_detach":
      cancelledCaptures.add(String(message.reference));
      captureFiles.delete(String(message.reference));
      break;
  }
};
