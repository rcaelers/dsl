const ROOT_NAME = "logic-conduit-artifacts-v1";
const POINTER_VERSION = 1;

let root = null;
let commandChain = Promise.resolve();

function text(error) {
  return error instanceof Error ? error.message : String(error);
}

function errorCode(error) {
  switch (error && error.name) {
    case "QuotaExceededError":
      return "quota";
    case "NotAllowedError":
    case "SecurityError":
      return "permission";
    case "NotFoundError":
      return "site_data_lost";
    default:
      return "io";
  }
}

function encodeNamespace(value) {
  return Array.from(new TextEncoder().encode(value), byte => byte.toString(16).padStart(2, "0")).join("");
}

function decodeNamespace(value) {
  if (value.length % 2 !== 0 || !/^[0-9a-f]*$/.test(value)) {
    throw new Error("invalid encoded artifact namespace");
  }
  const bytes = new Uint8Array(value.length / 2);
  for (let index = 0; index < bytes.length; index += 1) {
    bytes[index] = Number.parseInt(value.slice(index * 2, index * 2 + 2), 16);
  }
  return new TextDecoder("utf-8", { fatal: true }).decode(bytes);
}

function validIdentity(value) {
  return typeof value === "string" && /^[0-9a-f]{64}$/.test(value);
}

function generation() {
  if (typeof crypto.randomUUID === "function") {
    return crypto.randomUUID().replaceAll("-", "");
  }
  return `${Date.now().toString(16)}${Math.random().toString(16).slice(2)}`;
}

async function storageRoot() {
  if (root !== null) {
    return root;
  }
  if (!self.navigator.storage || typeof self.navigator.storage.getDirectory !== "function") {
    throw new DOMException("OPFS is unavailable", "NotSupportedError");
  }
  const origin = await self.navigator.storage.getDirectory();
  root = await origin.getDirectoryHandle(ROOT_NAME, { create: true });
  return root;
}

async function namespaceDirectory(namespace, create) {
  return (await storageRoot()).getDirectoryHandle(`n-${encodeNamespace(namespace)}`, { create });
}

async function writeFile(directory, name, bytes) {
  const handle = await directory.getFileHandle(name, { create: true });
  const writable = await handle.createWritable({ keepExistingData: false });
  try {
    await writable.write(bytes);
    await writable.close();
  } catch (error) {
    await writable.abort().catch(() => {});
    throw error;
  }
}

async function removeIfPresent(directory, name) {
  await directory.removeEntry(name).catch(error => {
    if (!error || error.name !== "NotFoundError") {
      throw error;
    }
  });
}

async function records() {
  const result = [];
  const storage = await storageRoot();
  for await (const [directoryName, directory] of storage.entries()) {
    if (directory.kind !== "directory" || !directoryName.startsWith("n-")) {
      continue;
    }
    let namespace;
    try {
      namespace = decodeNamespace(directoryName.slice(2));
    } catch (_error) {
      continue;
    }
    const dataNames = [];
    const referencedData = new Set();
    for await (const [pointerName, pointerHandle] of directory.entries()) {
      if (pointerHandle.kind === "file" && pointerName.startsWith("d-") && pointerName.endsWith(".bin")) {
        dataNames.push(pointerName);
        continue;
      }
      if (pointerHandle.kind !== "file" || !pointerName.startsWith("p-") || !pointerName.endsWith(".json")) {
        continue;
      }
      const identity = pointerName.slice(2, -5);
      if (!validIdentity(identity)) {
        continue;
      }
      try {
        const pointer = JSON.parse(await (await pointerHandle.getFile()).text());
        if (pointer.version !== POINTER_VERSION || typeof pointer.generation !== "string") {
          throw new Error("unsupported OPFS pointer");
        }
        const dataName = `d-${identity}-${pointer.generation}.bin`;
        const dataHandle = await directory.getFileHandle(dataName);
        const file = await dataHandle.getFile();
        if (!Number.isSafeInteger(pointer.length) || pointer.length !== file.size) {
          throw new Error("OPFS artifact length does not match its pointer");
        }
        referencedData.add(dataName);
        result.push({
          namespace,
          identity,
          directory,
          pointerName,
          dataName,
          length: file.size,
          accessed: Number.isFinite(pointer.accessed) ? pointer.accessed : 0,
          file,
        });
      } catch (_error) {
        await removeIfPresent(directory, pointerName).catch(() => {});
      }
    }
    for (const dataName of dataNames) {
      if (!referencedData.has(dataName)) {
        await removeIfPresent(directory, dataName).catch(() => {});
      }
    }
  }
  return result;
}

async function removeRecord(record) {
  await removeIfPresent(record.directory, record.pointerName);
  await removeIfPresent(record.directory, record.dataName);
}

async function loadEntries(maxBytes) {
  const available = await records();
  available.sort((left, right) => right.accessed - left.accessed || left.identity.localeCompare(right.identity));
  const entries = [];
  const transfers = [];
  let used = 0;
  let evicted = 0;
  for (const record of available) {
    if (record.length > maxBytes - used) {
      await removeRecord(record).catch(() => {});
      evicted += 1;
      continue;
    }
    const bytes = await record.file.arrayBuffer();
    entries.push({ namespace: record.namespace, identity: record.identity, bytes });
    transfers.push(bytes);
    used += record.length;
  }
  return { entries, transfers, evicted };
}

async function estimate() {
  if (!self.navigator.storage || typeof self.navigator.storage.estimate !== "function") {
    return { quota: null, usage: null };
  }
  const value = await self.navigator.storage.estimate();
  return {
    quota: Number.isSafeInteger(value.quota) ? String(value.quota) : null,
    usage: Number.isSafeInteger(value.usage) ? String(value.usage) : null,
  };
}

async function persistenceGranted() {
  if (!self.navigator.storage) {
    return false;
  }
  if (typeof self.navigator.storage.persisted === "function" && await self.navigator.storage.persisted()) {
    return true;
  }
  return typeof self.navigator.storage.persist === "function" && await self.navigator.storage.persist();
}

async function initialize(message) {
  try {
    await storageRoot();
    const durable = await persistenceGranted().catch(() => false);
    const loaded = await loadEntries(Number(message.maxBytes));
    const storage = await estimate().catch(() => ({ quota: null, usage: null }));
    self.postMessage({
      kind: "ready",
      durable,
      quota: storage.quota,
      usage: storage.usage,
      evicted: String(loaded.evicted),
      entries: loaded.entries,
    }, loaded.transfers);
  } catch (error) {
    self.postMessage({ kind: "unavailable", code: errorCode(error), message: text(error) });
  }
}

async function cleanupOldData(directory, identity, keepName) {
  const prefix = `d-${identity}-`;
  for await (const [name, handle] of directory.entries()) {
    if (handle.kind === "file" && name.startsWith(prefix) && name !== keepName) {
      await removeIfPresent(directory, name).catch(() => {});
    }
  }
}

async function publishOnce(message) {
  const directory = await namespaceDirectory(message.namespace, true);
  const nextGeneration = generation();
  const dataName = `d-${message.identity}-${nextGeneration}.bin`;
  const pointerName = `p-${message.identity}.json`;
  const bytes = new Uint8Array(message.bytes);
  await writeFile(directory, dataName, bytes);
  const pointer = new TextEncoder().encode(JSON.stringify({
    version: POINTER_VERSION,
    generation: nextGeneration,
    length: bytes.byteLength,
    accessed: Date.now(),
  }));
  try {
    await writeFile(directory, pointerName, pointer);
  } catch (error) {
    await removeIfPresent(directory, dataName).catch(() => {});
    throw error;
  }
  await cleanupOldData(directory, message.identity, dataName);
}

async function evictOldest(excludedNamespace, excludedIdentity) {
  const available = (await records())
    .filter(record => record.namespace !== excludedNamespace || record.identity !== excludedIdentity)
    .sort((left, right) => left.accessed - right.accessed || left.identity.localeCompare(right.identity));
  if (available.length === 0) {
    return false;
  }
  await removeRecord(available[0]);
  return true;
}

async function publish(message) {
  while (true) {
    try {
      await publishOnce(message);
      return;
    } catch (error) {
      if (errorCode(error) !== "quota" || !(await evictOldest(message.namespace, message.identity))) {
        throw error;
      }
    }
  }
}

async function remove(message) {
  let directory;
  try {
    directory = await namespaceDirectory(message.namespace, false);
  } catch (error) {
    if (error && error.name === "NotFoundError") {
      return;
    }
    throw error;
  }
  await removeIfPresent(directory, `p-${message.identity}.json`);
  await cleanupOldData(directory, message.identity, "");
}

async function execute(message) {
  try {
    if (!validIdentity(message.identity)) {
      throw new Error("invalid artifact identity");
    }
    await executeCommand(message);
    const storage = await estimate().catch(() => ({ quota: null, usage: null }));
    self.postMessage({ kind: "complete", sequence: String(message.sequence), quota: storage.quota, usage: storage.usage });
  } catch (error) {
    if (errorCode(error) === "site_data_lost") {
      root = null;
      try {
        await executeCommand(message);
        const storage = await estimate().catch(() => ({ quota: null, usage: null }));
        self.postMessage({ kind: "complete", sequence: String(message.sequence), quota: storage.quota, usage: storage.usage });
        return;
      } catch (retryError) {
        error = retryError;
      }
    }
    self.postMessage({
      kind: "failed",
      sequence: String(message.sequence),
      code: errorCode(error),
      message: text(error),
    });
  }
}

async function executeCommand(message) {
  if (message.kind === "publish") {
    await publish(message);
  } else if (message.kind === "remove") {
    await remove(message);
  } else {
    throw new Error("unknown OPFS command");
  }
}

self.onmessage = event => {
  const message = event.data;
  if (message.kind === "initialize") {
    void initialize(message);
    return;
  }
  commandChain = commandChain.then(() => execute(message), () => execute(message));
};
