const WORKER_URL = new URL("../worker.js" + new URL(import.meta.url).search, import.meta.url);

let worker = null;
let generation = 0;
let seq = 0;
const pending = new Map();
let canvasSource = null;
let workerPaints = false;

function crash(error) {
  if (!worker) return;
  worker.terminate();
  worker = null;
  workerPaints = false;
  generation++;
  for (const p of pending.values()) p.reject(error);
  pending.clear();
}

function getWorker() {
  if (worker) return worker;
  worker = new Worker(WORKER_URL);
  worker.onmessage = (e) => {
    const p = pending.get(e.data.id);
    if (!p) return;
    pending.delete(e.data.id);
    if (e.data.ok) p.resolve(e.data); else p.reject(new Error(e.data.error));
  };
  worker.onerror = (e) => { e.preventDefault?.(); crash(new Error("render worker crashed" + (e.message ? ": " + e.message : ""))); };
  worker.onmessageerror = () => crash(new Error("render worker sent an unreadable message"));
  const canvas = canvasSource && canvasSource();
  if (canvas && canvas.transferControlToOffscreen) {
    const offscreen = canvas.transferControlToOffscreen();
    worker.postMessage({ id: 0, kind: "canvas", canvas: offscreen }, [offscreen]);
    workerPaints = true;
  }
  return worker;
}

function request(msg) {
  const w = getWorker();
  return new Promise((resolve, reject) => {
    const id = ++seq;
    pending.set(id, { resolve, reject });
    w.postMessage({ id, ...msg });
  });
}

function makeClient(plugin) {
  let synced = -1;
  let known = new Set();
  let active = null;
  const sendBind = (key, bytes, activate) =>
    request({ kind: "model", plugin, key, bytes, activate }).then(() => { if (key) known.add(key); });
  async function sync() {
    if (synced === generation) return;
    synced = generation;
    known = new Set();
    client.ready = false;
    if (active) await sendBind(active.key, active.bytes, true);
  }
  const client = {
    ready: false,
    async ensure() {
      await sync();
      if (client.ready) return;
      await request({ kind: "ensure", plugin });
      client.ready = true;
    },
    async bind(bytes, key = null) {
      active = { key, bytes };
      await sync();
      await sendBind(key, key && known.has(key) ? null : bytes, true);
    },
    async prefetch(key, bytes) {
      await sync();
      if (!known.has(key)) await sendBind(key, bytes, false);
    },
    async call(fn, ...args) {
      await sync();
      return (await request({ kind: "call", plugin, fn, args })).result;
    },
    async callWithModel(fn, ...args) {
      await sync();
      return (await request({ kind: "call", plugin, fn, args, withModel: true })).result;
    },
    async render(fn, ...args) {
      await sync();
      return request({ kind: "call", plugin, fn, args, withModel: true, raster: workerPaints });
    },
  };
  return client;
}

const plugins = {
  maquette: makeClient("maquette"),
  scad: makeClient("maquette-scad"),
  gltf: makeClient("maquette-gltf"),
  molfig: makeClient("molfig"),
};

function paintInWorker(source) {
  canvasSource = source;
  getWorker();
}

async function snapshotPng() {
  return (await request({ kind: "snapshot" })).blob;
}

export { plugins, paintInWorker, snapshotPng };
