# maquette-gltf

A [Typst](https://typst.app) plugin that renders **glTF 2.0** assets (`.glb` / `.gltf`) directly into your documents. Software rasterizer, PBR shader, full IBL, ships as a single WebAssembly module.

Sibling to [`maquette`](../../maquette/README.md) (STL/OBJ/PLY) and [`maquette-scad`](../maquette-scad/README.md) (OpenSCAD → mesh). All three share the same render primitives via [`maquette-core`](../maquette-core/README.md).

## Usage

```typst
#import "@preview/maquette-gltf:0.1.0": render-gltf, get-gltf-info

// GLB (single file) or fully-embedded glTF — pass bytes.
#render-gltf(read("scene.glb", encoding: none))

// Split .gltf (external .bin + textures) — pass the path plus an inline
// `read` lambda. The wrapper walks the JSON, discovers every external URI,
// reads each one through your lambda, and packs them into a sidecar bundle
// for the wasm. You write zero filenames.
#render-gltf("assets/scene.gltf", read: p => read(p, encoding: none))

// Metadata (triangle count, bbox, max animation time) — same shape.
#let info = get-gltf-info(read("scene.glb", encoding: none))
```

The `read:` lambda handshake is required for split `.gltf` because Typst
packages can't reach the caller's project directory — `read()` inside a
package resolves against the package's own path, not the user's `.typ`.
Passing an inline lambda gives the wrapper a filesystem handle scoped to
your file. See the wrapper's module header for why bare `read` references
don't work (they stay bound to the package's path context).

## Config

Every knob is a named parameter. Defaults kick in when omitted.

```typst
#render-gltf(read("scene.glb", encoding: none),
  width: 512, height: 512,
  camera: (2.5, 1.5, 2.5), center: (0, 0, 0), up: (0, 1, 0), fov: 40,
  background: "#181820",
  ibl: (intensity: 1.2, hdr: read("studio.hdr", encoding: none)),
  shadows: (resolution: 2048, softness: 1, pcss_light_size: 0.5),
  ground: (color: "#282838", size_scale: 3.0, roughness: 0.9),
  ssao: (samples: 16, radius: 0.4, strength: 1.0),
  tone_mapping: (method: "aces", exposure: 1.2),
  antialias: 2,       // 0=off, 1=FXAA, 2=SSAA 2×, 4=SSAA 4×
  time: 0.0,          // animation time in seconds
)
```

Full field list lives in [`src/config.rs`](src/config.rs).

## Supported

**Core spec:**
- glTF 2.0 JSON + GLB (single-file and split with external `.bin`/textures)
- Meshes (POINTS / LINES / TRIANGLES modes)
- Skinning (JOINTS_0 + JOINTS_1, WEIGHTS_0 + WEIGHTS_1, up to 8 influences per vertex)
- Morph targets
- Animations — TRS + morph weights, LINEAR / STEP / CUBICSPLINE interpolation
- Multiple UV sets (TEXCOORD_0, TEXCOORD_1, TEXCOORD_2 — N ≥ 3 falls back to slot 2)
- Vertex colors (COLOR_0)
- Cameras (perspective + orthographic; auto-picks the first authored camera when the caller doesn't specify)
- Multiple scenes (via `scene_index` config)
- Multiple animations (via `animation_index` config)
- Textures with samplers (wrap / mag+min filter / mipmap; PNG + JPEG + WebP decoders)
- Sparse accessors
- `KHR_mesh_quantization` — quantized positions (i8/u8/i16/u16, normalized or raw)
- LOD-scale correction for KHR_texture_transform + quantized UVs

**Rendering:**
- Cook-Torrance GGX PBR with anisotropic-ready split
- Image-based lighting from HDR (Radiance / RGBE), octahedral encoding
- Shadow maps — PCF + PCSS soft shadows
- Alpha modes — OPAQUE, MASK, BLEND via WBOIT (order-independent)
- Double-sided normal flip
- SSAO, FXAA, SSAA ×2 / ×4

**Extensions:**
- `KHR_lights_punctual` (directional / point / spot)
- `KHR_materials_unlit`
- `KHR_materials_transmission` (refractive, thin-wall)
- `KHR_materials_ior`
- `KHR_materials_specular`
- `KHR_materials_emissive_strength`
- `KHR_materials_volume` (Beer-Lambert attenuation)
- `KHR_materials_clearcoat`
- `KHR_materials_sheen`
- `KHR_materials_iridescence` (Belcour-Barla thin-film Fresnel)
- `KHR_materials_anisotropy`
- `KHR_materials_dispersion`
- `KHR_materials_diffuse_transmission` (matte back-lit lambertian, direct + IBL contributions, per-pixel texture modulation)
- `KHR_texture_transform`
- `KHR_materials_pbrSpecularGlossiness` (legacy — converted to metallic-roughness at load)
- `KHR_materials_variants`
- `EXT_texture_webp`
- `EXT_meshopt_compression`
- `KHR_draco_mesh_compression` (via `draco-oxide`, pure-Rust decoder)

## Not supported

The plugin's graceful-degradation policy: unsupported inputs render with a **placeholder white texture** (for the format-decode gaps) or **silently ignore** the feature (for the animation-pointer / N ≥ 3 UV gaps). Nothing crashes.

### Texture formats
- **`KHR_texture_basisu` (KTX2 / Basis Universal)** — the dominant production texture format (gltfpack default, PlayCanvas, Babylon.js, model-viewer, Cesium 3D Tiles). Falls back to placeholder white. Full support would need a Basis Universal transcoder (~500 KB–1 MB wasm on top of Zstd supercompression). Deferred as a size-vs-value tradeoff — you can pre-transcode a KTX2 asset to WebP or PNG via [`gltf-transform`](https://gltf-transform.dev) as a workaround.
- **`EXT_texture_avif` (AVIF)** — needs libavif / dav1d (very large). Rare in the wild (no Khronos samples exist). Falls back to placeholder white.

### Animation
- **`KHR_animation_pointer`** — animation channels targeting arbitrary properties (materials, lights, cameras) via JSON pointer are silently ignored. Full support needs a JSON-pointer resolver, per-property mutable state animation, and scene re-flatten per frame to invalidate the baked material precomputes. Rare — most animations target node TRS + morph weights, both fully supported.

### Attributes
- **TEXCOORD_N for N ≥ 3** — the vertex carries three UV slots (0, 1, 2). Materials referencing TEXCOORD_3+ collapse to slot 2. Practically never occurs in real assets.
- **Sparse + `KHR_mesh_quantization` combo** — sparse-accessor deltas + quantization together aren't supported. Very rare (sparse is mostly used for morph targets, which typically stay f32).

### Extensions we don't parse
- Any KHR/EXT extension not listed under "Supported". The `extensionsRequired` whitelist is deliberately bypassed (`Gltf::from_slice_without_validation`) so unknown extensions render best-effort rather than a hard error. A stricter mode could be added as a config flag.

## Tested assets

Every render below is reproducible from `examples/data/gltf/`. All pass without crash on the current wasm; the ones that need decoders we don't ship render with the placeholder-white fallback.

| Asset | Extensions exercised | Result |
|---|---|---|
| `helmet.blg` (Damaged Helmet) | PBR, IBL, tangents, all materials factors | ✅ Full render |
| `boombox.glb` | PBR, IBL, emissive | ✅ |
| `fox.glb` | Skinning, animation (walk/idle), morph weights | ✅ |
| `cesiumman.glb` | Skinning, animation, textures | ✅ |
| `toycar.glb` | PBR, IBL, transmission | ✅ |
| `Duck/` (split .gltf) | External `.bin` + PNG via sidecar bundle | ✅ |
| `Duck-quantized/` | `KHR_mesh_quantization` + `KHR_texture_transform` | ✅ Pixel-identical to non-quantized |
| `Duck-webp/` | `EXT_texture_webp` | ✅ Full WebP decode |
| `Duck-ktx2/` | `KHR_texture_basisu` (KTX2) | ⚠ Placeholder white — no KTX2 decoder |
| `Box-draco/` | `KHR_draco_mesh_compression` | ✅ |
| `DiffuseTransmissionTest/` | `KHR_materials_diffuse_transmission` | ✅ |

## Building from source

```sh
make gltf-wasm    # release wasm into crates/maquette-gltf/maquette-gltf.wasm
make gltf-build   # + install into your local Typst package dir
```

Uses `cargo build --release --target wasm32-unknown-unknown -p maquette-gltf` followed by `wasm-opt -O3` with the SIMD / bulk-memory / sign-ext / nontrapping-fptoint feature set enabled. The wasm ends up at roughly 1.6 MB after wasm-opt.

## Dependencies

- [`gltf-rs`](https://github.com/gltf-rs/gltf) — pinned to a specific `main` rev because the crates.io release lags several extensions we need.
- [`draco-oxide-decoder`](https://crates.io/crates/draco-oxide-decoder) — pure-Rust Draco decoder.
- [`meshopt-rs`](https://crates.io/crates/meshopt-rs) — `EXT_meshopt_compression`.
- [`mikktspace`](https://crates.io/crates/mikktspace) — consistent per-vertex tangents when TANGENT is missing.

## License

MIT.
