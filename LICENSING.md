# Licensing

maquette is split across two licenses depending on which dependencies a crate
links.

## MIT

The core renderer and the STL / OBJ / PLY and glTF plugins depend only on
permissively licensed code and are **MIT** (see [`LICENSE`](LICENSE)):

- `maquette-core`
- `maquette` — Typst package, STL / OBJ / PLY
- `maquette-gltf` — Typst package, glTF 2.0

## GPL-3.0-or-later

`maquette-scad` parses OpenSCAD with [`openscad-rs`](https://crates.io/crates/openscad-rs),
which is GPL-3.0-or-later. Any binary that links it inherits the copyleft, so the
following are **GPL-3.0-or-later** (see [`COPYING`](COPYING)):

- `maquette-scad` — Typst package, OpenSCAD
- `maquette-cli` — the `maquette` command-line tool (bundles scad)
- `maquette-c` — C bindings (bundle scad)
- `maquette-py` — Python bindings (bundle scad)

Rendering a `.scad` file with the maquette-scad plugin does **not** place your
document or its output under the GPL — the license covers the plugin itself, not
its inputs or outputs.

## Third-party notices

`maquette-scad` bundles, via `crates/vendored/manifold-csg-sys`:

- the **Manifold** CSG library — Apache-2.0
- **Clipper2** — Boost Software License 1.0

Their license texts live in that directory (`LICENSE-APACHE`, `LICENSE-MIT`).
Other bundled dependencies are MIT / Apache-2.0 / BSD / Zlib.
