// ESLint flat config — a module-graph safety net for the webdemo JS.
//
// `no-undef` flags any identifier used but never declared or imported: exactly
// the class of bug a module split can introduce (a function moved into one
// module still referencing a symbol that lives in another, without an import).
// esbuild bundling and a load-harness both MISS this — they treat an unresolved
// name as an assumed runtime global — so this lint is the reliable guard.
//
// Globals are listed inline so the config needs no `globals` npm dependency.
// Scoped to the demo's JS; run with `make lint-js` (CI + pre-commit).

const BROWSER = [
  "window", "document", "navigator", "location", "history", "performance", "console",
  "fetch", "Worker", "Blob", "Image", "ImageData", "URL", "URLSearchParams",
  "createImageBitmap", "requestAnimationFrame", "cancelAnimationFrame",
  "requestIdleCallback", "cancelIdleCallback", "ResizeObserver", "MutationObserver",
  "matchMedia", "getComputedStyle", "getSelection", "indexedDB", "localStorage",
  "sessionStorage", "structuredClone", "setTimeout", "clearTimeout", "setInterval",
  "clearInterval", "queueMicrotask", "btoa", "atob", "TextEncoder", "TextDecoder",
  "CompressionStream", "DecompressionStream", "Response", "Request", "Headers",
  "AbortController", "WebAssembly", "self", "screen", "devicePixelRatio",
  "innerWidth", "innerHeight", "crypto", "alert", "confirm", "prompt",
  "CustomEvent", "Event", "EventTarget", "Node", "Element", "HTMLElement",
  "DOMParser", "FileReader", "File", "FormData", "DataTransfer", "postMessage",
];
const ECMA = [
  "Uint8Array", "Uint8ClampedArray", "Uint16Array", "Uint32Array", "Int8Array",
  "Int16Array", "Int32Array", "Float32Array", "Float64Array", "DataView",
  "ArrayBuffer", "SharedArrayBuffer", "Math", "JSON", "Object", "Array", "Set",
  "Map", "WeakMap", "WeakSet", "Promise", "Number", "String", "Boolean", "Date",
  "RegExp", "Error", "TypeError", "RangeError", "SyntaxError", "Symbol", "Reflect",
  "Proxy", "Intl", "isNaN", "isFinite", "parseInt", "parseFloat", "Infinity",
  "NaN", "undefined", "globalThis", "Function", "BigInt",
];
const globals = Object.fromEntries([...BROWSER, ...ECMA].map((n) => [n, "readonly"]));

export default [
  {
    files: ["docs/js/**/*.js", "docs/worker.js"],
    languageOptions: { ecmaVersion: "latest", sourceType: "module", globals },
    rules: { "no-undef": "error" },
  },
];
