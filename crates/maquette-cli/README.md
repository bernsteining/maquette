# maquette-cli

The maquette renderer as a native **`maquette`** binary — **STL / OBJ / PLY, glTF (PBR), OpenSCAD** → PNG / SVG, from the command line. Headless, deterministic, **no GL / GPU / display**.

## Install

```sh
make cli                              # → target/release/maquette
# or
cargo install --path crates/maquette-cli
```

Prebuilt binaries + `curl … | sh` / PowerShell installers ship via [dist](https://opensource.axo.dev/cargo-dist/) on tagged releases.

## Use

```sh
maquette bunny.obj  -o bunny.png  --set width=800 --set height=600
maquette part.scad  -o part.png   --fn 64
maquette scene.glb  -o scene.png  --plugin gltf --config camera.json
maquette model.stl  -o model.svg
```

- `--config file.json` — the render-config dict (the same keys the plugins accept).
- `--set key=value` — override one key (value parsed as JSON), repeatable; wins over `--config`.
- Output format follows the `-o` extension (`.png` / `.svg`); `--plugin` overrides the auto-detection; `--fn` sets the OpenSCAD facet count.

> Native and wasm renders are visually equivalent but **not bit-identical** (floating-point evaluation differs).
