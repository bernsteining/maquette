# maquette-render

3D rendering — **STL / OBJ / PLY, glTF (PBR), OpenSCAD** → raster pixels or SVG — in JavaScript, via the same WebAssembly the [maquette](https://github.com/bernsteining/maquette) Typst plugins ship. **Headless, deterministic, no WebGL/GPU.** Runs in **Node** and the **browser**.

```sh
npm install maquette-render
```

```js
import { createMaquette, toImageData } from "maquette-render";

const mq = createMaquette();

// OpenSCAD → pixels
const img = await mq.renderScad(
  "difference() { cube(20, center=true); sphere(12); }",
  { width: 400, height: 400, azimuth: 30, shading: "gooch" },
  { facets: 64 }
);
// img = { width, height, pixels: Uint8Array (RGBA) }

// In a browser: draw to a canvas
canvas.width = img.width; canvas.height = img.height;
canvas.getContext("2d").putImageData(toImageData(img), 0, 0);

// A mesh → SVG
const svg = await mq.renderObj(objBytes, { azimuth: 180 }, { format: "svg" });

// glTF (PBR)
const helmet = await mq.renderGltf(glbBytes, { camera: [2.5,1.5,2.5], shadows: true });
```

`config` is a plain object of the render-config keys (or a JSON string, or omitted). Raster renders resolve to `{ width, height, pixels }` (RGBA8); `{ format: "svg" }` resolves to an SVG string.

## API

- `renderStl / renderObj / renderPly(data, config?, { format }?)`
- `renderGltf(data, config?)` — PNG raster only
- `renderScad(src, config?, { facets, format }?)` · `compileScad(src, { facets }?)` → PLY
- `infoStl / infoObj / infoPly / infoGltf(data)` → metadata object
- `decodeRaster(bytes)`, `toImageData(raster)` (browser)

`data` may be a `Uint8Array`, `ArrayBuffer`, or string. Bundlers serve `wasm/*.wasm` next to the module; override fetching with `createMaquette({ load })`.

> The demo at [bernsteining.github.io/maquette](https://bernsteining.github.io/maquette/) runs this exact WebAssembly.
