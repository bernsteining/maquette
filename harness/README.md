# Harness

A wasmi-based test harness for running the maquette WASM plugin outside of Typst.

## Build

```sh
make harness
```

## Usage

```
harness [--fuel] [--bench=N] [--repeat=N] <wasm> <func> <file1> [<file2>]
```

- `<wasm>` — path to compiled `.wasm` binary
- `<func>` — exported function name
- `<file1>` — model file (STL/OBJ/PLY)
- `<file2>` — JSON config file (omit for one-argument functions such as `prepare_obj`)
- `--fuel` — count WASM instructions
- `--bench=N` — run N iterations (fresh instance each), report avg/min time and module load time
- `--repeat=N` — call the function N times on the same instance and time the last call: the warm,
  cache-hit path a Typst document hits when it renders an already-parsed model again

Result bytes are written to stdout; diagnostics to stderr.

## Examples

Render teapot to PNG:
```sh
echo '{}' > /tmp/config.json
./harness/target/release/harness maquette/maquette.wasm render_obj_png examples/data/teapot.obj /tmp/config.json > teapot.png
```

Benchmark with instruction counting:
```sh
./harness/target/release/harness --fuel --bench=5 maquette/maquette.wasm render_obj_png examples/data/teapot.obj /tmp/config.json > /dev/null
```

Get model info:
```sh
./harness/target/release/harness maquette/maquette.wasm get_obj_info examples/data/teapot.obj /tmp/config.json
```

## Available functions

| Function | Format | Output |
|---|---|---|
| `render_stl` | STL | SVG |
| `render_stl_png` | STL | PNG |
| `render_obj` | OBJ | SVG |
| `render_obj_png` | OBJ | PNG |
| `render_ply` | PLY | SVG |
| `render_ply_png` | PLY | PNG |
| `get_stl_info` | STL | JSON |
| `get_obj_info` | OBJ | JSON |
| `get_ply_info` | PLY | JSON |

## Measuring: prefer wall-clock over fuel

wasmi charges fuel when it enters a function, a `loop` iteration or an `if`/`else` arm — for every
instruction in that frame, including `match` arms lowered to `block`/`br_table` that never execute.
Fuel therefore over-bills branchy code (a single-pass OBJ parser rewrite scored +0.4% fuel yet ran
7–14% faster), and Typst doesn't meter fuel at all. Use fuel as a quick filter; decide on wall-clock,
alternating the two builds within one session and taking the minimum of several runs.

## Stage profiling

Build the plugin with `--features prof` (e.g. `cargo rustc ... -p maquette --features prof`). Each
`crate::prof::mark(id)` in `render_raster` then calls the host, which records the elapsed time (or
the fuel counter with `--fuel`); the harness prints one `prof: <id> <delta> <cumulative> n=<count>`
line per mark id, summing repeated marks. Without the feature the marks compile to nothing.

| id | stage ending at the mark |
|---|---|
| 10 | entry → `render_raster` setup |
| 11 | preprocess (clone, color map, clip, explode, bbox) |
| 12 | smooth vertex normals |
| 13 | lights + shadow data |
| 14 | projection + per-vertex shading |
| 15 | depth sort |
| 16 | framebuffer allocation |
| 17 | rasterization |
| 18 | strokes, wireframe, outline, SSAO, bloom, glow, sharpen |
| 19 | FXAA |

The first call of a large function also pays wasmi's lazy translation, so profile warm calls
(`--bench=2`) when comparing stages.
