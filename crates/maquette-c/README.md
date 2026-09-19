# maquette (C ABI)

Headless, CPU 3D rendering — STL / OBJ / PLY, glTF (PBR), OpenSCAD → PNG / SVG — as a C library. **No GL, no GPU, no display.** This is the universal binding layer: call it from C or C++, or from any FFI-capable language (Python `ctypes`, Ruby FFI, Julia `ccall`, Go `cgo`, C#, Lua, R…).

## Build

```sh
cargo build --release
```

produces `target/release/libmaquette.{so,dylib,dll}` (dynamic) and `libmaquette.a` (static), plus the header `include/maquette.h`.

## Use

```c
#include "maquette.h"

uint8_t *out = NULL; size_t n = 0;
int rc = maquette_render_scad_png(
    "difference(){ cube(20, center=true); sphere(12); }",
    "{\"width\":256,\"height\":256,\"shading\":\"gooch\"}", 48, &out, &n);
if (rc == 0) { fwrite(out, 1, n, fopen("part.png", "wb")); }
else { fprintf(stderr, "%.*s\n", (int)n, out); }  // error message
maquette_free(out, n);
```

```sh
cc render.c -I include -L target/release -lmaquette -Wl,-rpath,target/release -o render
```

See [`examples/render.c`](examples/render.c).

## API

Every function returns **0 on success, 1 on error**. On return, `*out`/`*out_len` is a heap buffer **you own**: the rendered bytes on success (PNG, or SVG/PLY/JSON text), or a UTF-8 error message on error. Free it with `maquette_free(ptr, len)`. `config` is a JSON string of the render-config keys (or `NULL` for defaults).

- `maquette_render_{stl,obj,ply}_{png,svg}(data, len, config, &out, &out_len)`
- `maquette_render_gltf_png(data, len, config, &out, &out_len)`
- `maquette_render_scad_{png,svg}(src, config, facets, &out, &out_len)` · `maquette_compile_scad(src, facets, …)` → PLY
- `maquette_info_{stl,obj,ply,gltf}(data, len, config, &out, &out_len)` → JSON
- `maquette_free(ptr, len)`
