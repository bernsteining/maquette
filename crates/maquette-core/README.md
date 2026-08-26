# maquette-core

Format-agnostic render primitives shared by the [maquette](../../maquette/README.md) plugin family. Not a plugin itself — this crate is a Rust **rlib** that the plugin cdylibs statically link.

Nothing in here knows about STL, OBJ, PLY, glTF, or OpenSCAD. It knows about triangles, textures, shadow maps, and how to turn all of that into an RGBA framebuffer.

## What's in the box

| Module | What it does |
|---|---|
| [`math`](src/math.rs) | `Vec3`, `Mat4`, quaternion helpers — f64 for scene setup, f32 for the hot path |
| [`rasterizer`](src/rasterizer.rs) | SIMD scanline triangle fill, z-buffer, WBOIT translucency, per-pixel shader trait |
| [`texture`](src/texture.rs) | Mipmapped 2D textures, `Wrap` / `Filter` samplers, SIMD bilinear |
| [`texture_decode`](src/texture_decode.rs) | PNG + JPEG + WebP → RGBA8 |
| [`color`](src/color.rs) | sRGB ↔ linear LUTs, hex parsing |
| [`ibl`](src/ibl.rs) | HDR (Radiance / RGBE) environment loader, octahedral encoding, diffuse irradiance prefilter, specular mip chain |
| [`rgbe`](src/rgbe.rs) | Radiance .hdr decoder |
| [`shadow`](src/shadow.rs) | Format-agnostic shadow-map builder (takes any caster-triangle list + `PunctualLight` set) |
| [`light`](src/light.rs) | `PunctualLight` + `LightKind` — directional / point / spot |
| [`ssao`](src/ssao.rs) | Screen-space ambient occlusion (golden-angle sunflower sample distribution, bilateral blur) |
| [`fxaa`](src/fxaa.rs) | Fast approximate anti-aliasing (post-process) |

## Design constraints

- **`wasm32-unknown-unknown` targetable.** No filesystem, no threads, no OS syscalls. Every dep is either pure-Rust or has a working wasm path.
- **SIMD (`v128`, `f32x4_*`) throughout the hot loops** — bilinear tap unpacks, IBL polynomial approximations, SSAO sample gathers, RGB→RGBA output.
- **Format-agnostic.** Callers hand primitives (`Triangle`, `PunctualLight`, `IblEnvironment`) — the crate doesn't own scene traversal or asset parsing. This keeps `maquette-core` linkable from every plugin without dragging their format-specific deps.
- **Static / reusable buffers** for the SSAO/AO scratch to avoid per-frame allocs on animation scrubs.

## Who uses it

- [`maquette`](../../maquette/README.md) — STL/OBJ/PLY plugin. Uses `rasterizer`, `texture`, `color`, `fxaa`, `ssao`.
- [`maquette-gltf`](../maquette-gltf/README.md) — glTF 2.0 plugin. Uses the whole surface, especially `ibl` (HDR environments), `shadow` (per-light PCF/PCSS), and the SIMD rasterizer + texture sampler.
- [`maquette-scad`](../maquette-scad/README.md) — OpenSCAD plugin. Uses none of it directly (it emits geometry for the other two).

## Not a public API

`maquette-core` is versioned only in the workspace sense. Its Rust surface is a private contract between the plugins. If you want the rendering primitives standalone, vendor the source.

## License

MIT.
