# maquette (Python)

Deterministic, **headless**, **CPU** 3D rendering from Python — **no GL, no GPU, no display**. Render STL / OBJ / PLY, glTF (PBR), and OpenSCAD to PNG or SVG. Built on the [maquette](https://github.com/bernsteining/maquette) renderer (from-scratch rasterizer, Cook-Torrance PBR, shadows, SSAO) via [PyO3](https://pyo3.rs).

Ideal where spinning up an OpenGL/EGL context is a pain: CI, GPU-less servers, notebooks, batch thumbnailing, code-CAD (CadQuery/build123d), mesh QA (trimesh/Open3D), scientific figures.

```python
import maquette

# OpenSCAD source → PNG
png = maquette.render_scad(
    "difference() { cube(20, center=true); sphere(12); }",
    {"azimuth": 30, "elevation": 20, "shading": "gooch"},
    facets=64,
)
open("part.png", "wb").write(png)

# A mesh file → PNG (dispatches on extension)
png = maquette.render_file("bunny.obj", {"width": 800, "height": 600})

# glTF (PBR) → PNG
png = maquette.render_gltf(open("helmet.glb", "rb").read(),
                           {"camera": [2.5, 1.5, 2.5], "shadows": True})

# Metadata
print(maquette.info_obj(open("bunny.obj", "rb").read()))
```

`config` is a plain dict of the render-config keys (or a JSON string, or `None`). Every renderer returns `bytes` (PNG by default, `fmt="svg"` for vector where supported).

## Functions

- `render_stl / render_obj / render_ply(data, config=None, fmt="png")`
- `render_gltf(data, config=None)` — PNG only
- `render_scad(src, config=None, facets=32, fmt="png")` — compile + render
- `compile_scad(src, facets=32)` — OpenSCAD → PLY bytes
- `render_file(path, config=None, fmt="png", facets=32)` — by extension
- `info_stl / info_obj / info_ply / info_gltf(data)` — metadata dict

## Install

```sh
pip install maquette
```

Wheels bundle the renderer (and the Manifold CSG kernel for OpenSCAD) — no system dependencies at runtime.
