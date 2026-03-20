# Harness

A wasmi-based test harness for running the maquette WASM plugin outside of Typst.

## Build

```sh
make harness
```

## Usage

```
harness [--fuel] [--bench=N] <wasm> <func> <file1> <file2>
```

- `<wasm>` — path to compiled `.wasm` binary
- `<func>` — exported function name
- `<file1>` — model file (STL/OBJ/PLY)
- `<file2>` — JSON config file
- `--fuel` — count WASM instructions
- `--bench=N` — run N iterations, report avg/min time

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
