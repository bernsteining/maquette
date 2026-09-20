
const target = process.argv[2];
if (!target) { console.error("usage: webdemo-smoke.mjs <esm-entry>"); process.exit(2); }

const ctx = () => new Proxy(function () {}, {
  get(t, p) {
    if (p in t) return t[p];
    if (p === "style" || p === "dataset") return (t[p] = {});
    if (p === "classList") return { add() {}, remove() {}, toggle() {}, contains: () => false };
    if (p === "children") return [];
    if (p === "getContext") return () => ctx();
    if (p === "getBoundingClientRect") return () => ({ left: 0, top: 0, width: 100, height: 100, right: 100, bottom: 100 });
    if (p === "closest") return () => null;
    if (["append", "appendChild", "setAttribute", "removeAttribute", "addEventListener", "removeEventListener", "remove", "click", "focus", "setPointerCapture", "releasePointerCapture", "toBlob", "replaceChild", "select"].includes(p)) return () => {};
    if (["clientWidth", "clientHeight", "offsetWidth", "offsetHeight", "scrollTop", "scrollLeft", "tabIndex"].includes(p)) return 100;
    if (["value", "textContent", "innerHTML", "title", "className", "id"].includes(p)) return "";
    if (typeof p === "symbol") return undefined;
    return () => ctx();
  },
  set() { return true; },
  apply() { return ctx(); },
});
globalThis.document = {
  getElementById: () => ctx(), createElement: () => ctx(), createTextNode: () => ctx(),
  querySelector: () => ctx(), querySelectorAll: () => [], addEventListener() {}, removeEventListener() {},
  documentElement: ctx(), body: ctx(), activeElement: ctx(), fullscreenEnabled: false,
  webkitFullscreenEnabled: false, fullscreenElement: null, exitFullscreen() {}, execCommand() {}, dispatchEvent() {},
};
globalThis.window = {
  devicePixelRatio: 2, innerWidth: 1200, innerHeight: 800,
  matchMedia: () => ({ matches: false, addEventListener() {} }), addEventListener() {}, removeEventListener() {},
  requestIdleCallback: (cb) => setTimeout(() => cb({ timeRemaining: () => 0 }), 0),
  requestAnimationFrame: (cb) => setTimeout(() => cb(0), 0),
};
globalThis.matchMedia = globalThis.window.matchMedia;
Object.defineProperty(globalThis, "navigator", { value: { clipboard: { writeText: async () => {} }, connection: {}, deviceMemory: 8, userAgent: "node" }, configurable: true });
globalThis.location = { origin: "http://x", pathname: "/", search: "", hash: "", href: "http://x/" };
globalThis.history = { replaceState() {}, pushState() {} };
globalThis.performance = globalThis.performance || { now: () => 0 };
globalThis.requestAnimationFrame = globalThis.window.requestAnimationFrame;
globalThis.requestIdleCallback = globalThis.window.requestIdleCallback;
globalThis.getComputedStyle = () => ({ paddingLeft: "0px", paddingRight: "0px", paddingTop: "0px", paddingBottom: "0px" });
globalThis.Worker = class { constructor() {} postMessage(m) { queueMicrotask(() => { if (this.onmessage) this.onmessage({ data: { id: m && m.id, ok: true, result: new Uint8Array([0, 1, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0]) } }); }); } addEventListener() {} terminate() {} };
globalThis.ResizeObserver = class { observe() {} unobserve() {} disconnect() {} };
globalThis.fetch = async () => ({ ok: true, json: async () => ({ plugins: [], models: [], molecules: {}, defaults: {} }), arrayBuffer: async () => new ArrayBuffer(8), text: async () => "", headers: { get: () => null } });
globalThis.URL = globalThis.URL || {}; globalThis.URL.createObjectURL = () => "blob:x"; globalThis.URL.revokeObjectURL = () => {};
globalThis.Blob = class {}; globalThis.ImageData = class { constructor(a, w, h) { this.data = a; this.width = w; this.height = h; } };
globalThis.Image = class { set src(v) { if (this.onload) setTimeout(() => this.onload(), 0); } };
globalThis.createImageBitmap = async () => ({ close() {}, width: 1, height: 1 });
globalThis.indexedDB = { open: () => ({}) };

let bootErr = null;
process.on("unhandledRejection", (e) => { bootErr = e; });
try {
  await import(target);
  console.log("LOAD OK — module graph evaluated with no import/TDZ/reference error");
} catch (e) {
  console.error("LOAD FAILED:\n" + (e && e.stack || e));
  process.exit(1);
}
await new Promise((r) => setTimeout(r, 300));
if (bootErr) { console.error("BOOT rejected: " + (bootErr.stack || bootErr.message || bootErr)); process.exit(1); }
console.log("BOOT SETTLED — no unhandled rejection");
