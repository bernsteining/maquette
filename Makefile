WASM_TARGET = target/wasm32-unknown-unknown/release/maquette.wasm
WASM_OUT = maquette/maquette.wasm
WASM_PKG = $(HOME)/.local/share/typst/packages/local/maquette/0.1.0/maquette.wasm

# Path remaps so no build-machine paths (home, cargo registry, rustup toolchain)
# leak into the wasm. Overridable — CI passes its container-specific prefixes.
# Locally, cargo/rustup/project all live under $HOME, so one remap covers them.
REMAP ?= --remap-path-prefix=$(HOME)=~

# RUSTFLAGS overrides .cargo/config.toml, so re-declare the SIMD target-features
# here (must match config), then append the path remaps.
RUSTFLAGS_WASM = -Ctarget-feature=+simd128,+bulk-memory,+sign-ext,+nontrapping-fptoint,+mutable-globals,+multivalue $(REMAP)

# Build + optimize the wasm into $(WASM_OUT). Single source of truth for the
# build — used by local `build` and by CI (docs/ demo + Pages).
wasm:
	RUSTFLAGS="$(RUSTFLAGS_WASM)" cargo build --target wasm32-unknown-unknown --release
	wasm-opt -O3 --enable-simd --enable-bulk-memory --enable-sign-ext --enable-nontrapping-float-to-int --enable-mutable-globals --enable-multivalue --traps-never-happen --fast-math --closed-world --directize --inline-functions-with-loops --converge $(WASM_TARGET) -o $(WASM_OUT)
	@ls -lh $(WASM_OUT)

# Local: build + install into the typst local package dir.
build: wasm
	mkdir -p $(dir $(WASM_PKG))
	cp $(WASM_OUT) $(WASM_PKG)

harness:
	cargo build --release --manifest-path harness/Cargo.toml

# Documentation. One PDF per plugin, all under docs/ (same dir GitHub Pages
# publishes — the browser demo + the docs sit next to each other, easy to
# cross-link). `make docs` builds all three; each target only depends on the
# wasm the doc's own examples exercise, so a scad-only edit doesn't force a
# maquette-gltf rebuild.
doc-maquette: build
	typst compile docs/maquette-documentation.typ docs/maquette-documentation.pdf --root .

doc-gltf: gltf-build
	typst compile docs/maquette-gltf-documentation.typ docs/maquette-gltf-documentation.pdf --root .

doc-scad: scad-build
	typst compile docs/maquette-scad-documentation.typ docs/maquette-scad-documentation.pdf --root .

docs: doc-maquette doc-gltf doc-scad

# Back-compat: `make doc` still builds the maquette PDF (the historical target).
doc: doc-maquette

# Model files the browser demo ships. Canonical copies live once in
# examples/data/; the demo needs them under docs/ (the only dir Pages
# publishes). Single source of truth for the demo's model set — shared by
# `make demo` (local) and the Pages CI, so neither hard-codes the list.
# Picker models first, then extras only referenced by documentation deep-links.
DEMO_MODELS = bunny.obj teapot.obj crankshaft.obj brain_skull.obj rubi_scan.ply \
              cube.stl colored_cube.stl cube.obj rubi_blender.ply
# glTF demo models — only the Damaged Helmet ships in the demo. Additional
# assets (fox, boombox, toycar, cesiumman) live in examples/data/gltf/ for
# local dev; add them here to include them in the demo's model list.
DEMO_GLTF_MODELS = helmet.blg

# Copy the demo models into docs/ (gitignored there — regenerated, not committed).
demo-assets:
	cp $(addprefix examples/data/,$(DEMO_MODELS)) docs/
	cp $(addprefix examples/data/gltf/,$(DEMO_GLTF_MODELS)) docs/

# Assemble the demo dir locally: fresh wasm + models, ready to serve.
demo: wasm demo-assets scad-wasm gltf-wasm
	cp $(WASM_OUT) docs/maquette.wasm
	cp $(SCAD_WASM_OUT) docs/maquette-scad.wasm
	cp $(GLTF_WASM_OUT) docs/maquette-gltf.wasm
	@echo "docs/ ready — serve with:  python3 -m http.server -d docs"

# --- maquette-scad: OpenSCAD/CSG plugin (workspace member, Manifold kernel) ---
SCAD_WASM_TARGET = target/wasm32-unknown-unknown/release/maquette_scad.wasm
SCAD_WASM_OUT = crates/maquette-scad/maquette-scad.wasm

# Build + optimize the scad plugin wasm. Mirrors the core `wasm` recipe (same
# target features + wasm-opt flags) for consistency.
scad-wasm:
	RUSTFLAGS="$(RUSTFLAGS_WASM)" cargo build --target wasm32-unknown-unknown --release -p maquette-scad
	wasm-opt -O3 --enable-simd --enable-bulk-memory --enable-sign-ext --enable-nontrapping-float-to-int --enable-mutable-globals --enable-multivalue --traps-never-happen --fast-math --closed-world --directize --inline-functions-with-loops --converge $(SCAD_WASM_TARGET) -o $(SCAD_WASM_OUT)
	@ls -lh $(SCAD_WASM_OUT)

# --- maquette-gltf: glTF 2.0 plugin (workspace member, shares maquette-core) ---
GLTF_WASM_TARGET = target/wasm32-unknown-unknown/release/maquette_gltf.wasm
GLTF_WASM_OUT = crates/maquette-gltf/maquette-gltf.wasm
GLTF_WASM_PKG = $(HOME)/.local/share/typst/packages/local/maquette-gltf/0.1.0/maquette-gltf.wasm

# Build + optimize the glTF plugin wasm. Flags mostly mirror the maquette
# core plugin, minus `--converge`: on this codebase, --converge trades ~1%
# speed for a ~0.2 % size shave under wasmi. wasmi is an interpreter, so
# instructions-per-frame beats module bytes; we keep the single-pass -O3
# output. Verified with harness --bench=5 --fuel on helmet.blg.
gltf-wasm:
	RUSTFLAGS="$(RUSTFLAGS_WASM)" cargo build --target wasm32-unknown-unknown --release -p maquette-gltf
	wasm-opt -O3 --enable-simd --enable-bulk-memory --enable-sign-ext --enable-nontrapping-float-to-int --enable-mutable-globals --enable-multivalue --traps-never-happen --fast-math --closed-world --directize --inline-functions-with-loops $(GLTF_WASM_TARGET) -o $(GLTF_WASM_OUT)
	@ls -lh $(GLTF_WASM_OUT)

# Install glTF plugin into the local Typst package dir (mirror of `build`).
gltf-build: gltf-wasm
	mkdir -p $(dir $(GLTF_WASM_PKG))
	cp $(GLTF_WASM_OUT) $(GLTF_WASM_PKG)

# Install SCAD plugin into the local Typst package dir.
SCAD_WASM_PKG = $(HOME)/.local/share/typst/packages/local/maquette-scad/0.1.0/maquette-scad.wasm
scad-build: scad-wasm
	mkdir -p $(dir $(SCAD_WASM_PKG))
	cp $(SCAD_WASM_OUT) $(SCAD_WASM_PKG)

.PHONY: wasm build harness doc doc-maquette doc-gltf doc-scad docs demo-assets demo scad-wasm scad-build gltf-wasm gltf-build
