# Per-plugin wasmi bench

`bench.sh` measures each plugin's wall-clock, fuel (wasmi instruction count),
output SHA1, and wasm size under the wasmi interpreter — the same runtime
Typst uses.

## Usage

```sh
# Baseline (once per plugin)
./bench.sh gltf     --save
./bench.sh maquette --save
./bench.sh scad     --save

# Bench against the saved baseline
./bench.sh gltf
./bench.sh maquette
./bench.sh scad

# Alternate assets
./bench.sh maquette bunny.obj
./bench.sh scad     gear.scad
./bench.sh gltf     boombox.glb

# All three, defaults
./bench.sh all
```

Baselines live in `/tmp/bench/baseline/` — regenerate whenever you land a
change you want to lock in as the new reference.

## Metrics

- **`min`** — best of 5 (gltf/maquette) or 3 (scad) wall-clock runs.
- **`fuel`** — wasmi instructions executed. Deterministic per input; if this
  moves between runs of the same wasm, something is wrong.
- **`wasm`** — file size in bytes.
- **`cmp`** — SHA1 of the plugin's output bytes vs baseline. `identical`
  means bit-perfect; a hash means the output drifted.

## Notes

- The `scad` bench uses 3-iteration averaging (build_scad is slower + hits
  Manifold's C++ CSG kernel); gltf + maquette use 5.
- Each plugin gets a fixed test config (see `bench.sh` for the JSON).
  A different config would give different numbers — the point is
  repeatability, not comparability across plugins.
