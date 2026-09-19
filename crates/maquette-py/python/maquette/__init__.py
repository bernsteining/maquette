"""maquette — deterministic, headless, CPU 3D rendering (no GL/GPU).

Render STL/OBJ/PLY, glTF (PBR) and OpenSCAD to PNG or SVG from Python. The
``config`` argument is a plain dict of the same keys the renderer accepts
(see render-config.schema.json), or a JSON string, or None.

>>> import maquette
>>> png = maquette.render_scad("difference(){cube(20,center=true);sphere(12);}",
...                            {"azimuth": 30, "shading": "gooch"}, facets=64)
>>> open("part.png", "wb").write(png)
"""

from ._maquette import (
    render_stl,
    render_obj,
    render_ply,
    render_gltf,
    render_scad,
    compile_scad,
    info_stl,
    info_obj,
    info_ply,
    info_gltf,
    __version__,
)

__all__ = [
    "render_stl",
    "render_obj",
    "render_ply",
    "render_gltf",
    "render_scad",
    "compile_scad",
    "info_stl",
    "info_obj",
    "info_ply",
    "info_gltf",
    "render_file",
    "__version__",
]


def render_file(path, config=None, fmt="png", facets=32):
    """Render a model file by path, dispatching on its extension.

    Supports .stl/.obj/.ply/.glb/.gltf/.scad. Returns PNG (default) or SVG bytes.
    glTF renders to PNG only.
    """
    import os

    ext = os.path.splitext(path)[1].lower().lstrip(".")
    if ext == "scad":
        with open(path, "r") as f:
            return render_scad(f.read(), config, facets, fmt)
    with open(path, "rb") as f:
        data = f.read()
    if ext == "stl":
        return render_stl(data, config, fmt)
    if ext == "obj":
        return render_obj(data, config, fmt)
    if ext == "ply":
        return render_ply(data, config, fmt)
    if ext in ("glb", "gltf"):
        return render_gltf(data, config)
    raise ValueError(f"unsupported extension: {ext!r}")
