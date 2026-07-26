# maquette — browser demo

A fully client-side demo of maquette that runs the **exact same `maquette.wasm`**
published to Typst Universe — no server, no build step, no bindings. The page
drives the plugin's `wasm-minimal-protocol` ABI directly from ~40 lines of JS
(`app.js`), so what you see here is what the Typst plugin produces.

## Files

| File | What it is |
|------|------------|
| `index.html` | UI (self-contained, inline CSS) |
| `app.js` | WASM shim + form/render/export logic |
| `maquette.wasm` | copy of `../maquette/maquette.wasm` (keep in sync on release) |
| `bunny.obj` | default embedded model |

## Run locally

`fetch()` needs HTTP (not `file://`), so serve the folder:

```sh
cd docs && python3 -m http.server 8000
# open http://localhost:8000
```

## Publish on GitHub Pages

Settings → Pages → Source: **Deploy from a branch** → branch `master`, folder
`/docs`. The demo will be live at `https://<user>.github.io/maquette/`.

## Keeping the WASM current

`maquette.wasm` here is a copy. After rebuilding the plugin, refresh it:

```sh
cp maquette/maquette.wasm docs/maquette.wasm
```

(Or make it a symlink / add a line to the build script.)

## Notes

- Requires a browser with **WASM SIMD** (Chrome/Edge/Firefox, Safari 16.4+).
- Files are read as raw bytes (`file.arrayBuffer()`), so binary STL/PLY load
  without the UTF-8 pitfall that `read()` has in Typst.
- The "Typst code" panel exports a minimal snippet (only the options you
  changed) using `read(..., encoding: none)`.
