WASM_TARGET = target/wasm32-unknown-unknown/release/maquette.wasm
WASM_OUT = crates/maquette/maquette.wasm
WASM_PKG = $(HOME)/.local/share/typst/packages/local/maquette/0.1.3/maquette.wasm

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
	RUSTFLAGS="$(RUSTFLAGS_WASM)" cargo build --target wasm32-unknown-unknown --release -p maquette
	wasm-opt -O3 --enable-simd --enable-bulk-memory --enable-sign-ext --enable-nontrapping-float-to-int --enable-mutable-globals --enable-multivalue --traps-never-happen --fast-math --closed-world --directize --inline-functions-with-loops --converge $(WASM_TARGET) -o $(WASM_OUT)
	@ls -lh $(WASM_OUT)

# Local: build + install into the typst local package dir. Copies the
# whole package (wasm + typst.toml + .typ) so `@local/maquette:0.1.0`
# resolves — otherwise a bare wasm sits there without the manifest and
# typst can't find the entry .typ.
build: wasm
	mkdir -p $(dir $(WASM_PKG))
	cp $(WASM_OUT) $(WASM_PKG)
	cp crates/maquette/maquette/maquette.typ crates/maquette/maquette/typst.toml $(dir $(WASM_PKG))

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
# glTF demo models — Littlest Tokyo (animated, Draco-compressed) is our sole
# glTF showcase in the picker. Damaged Helmet (helmet.blg) still lives in
# examples/data/gltf/ for the PDF documentation build + local dev; add it
# back here to reinclude it in the demo's model list. Other assets (fox,
# boombox, toycar, cesiumman, potofcoals) are dev-only in the same way.
DEMO_GLTF_MODELS = tokyo.glb
DEMO_MOL_MODELS = lsd.pdb aspirin.xyz crambin.bcif 9R1O.pdb
DEMO_SCAD_MODELS = gear.scad

# Copy the demo models into docs/ (gitignored there — regenerated, not committed).
demo-assets:
	cp $(addprefix examples/data/,$(DEMO_MODELS)) docs/
	cp $(addprefix examples/data/gltf/,$(DEMO_GLTF_MODELS)) docs/
	mkdir -p docs/molecules docs/scad
	cp $(addprefix examples/data/molecules/,$(DEMO_MOL_MODELS)) docs/molecules/
	cp $(addprefix examples/data/scad/,$(DEMO_SCAD_MODELS)) docs/scad/

# Assemble the demo dir locally: fresh wasm + models, ready to serve.
demo: wasm demo-assets scad-wasm gltf-wasm
	cp $(WASM_OUT) docs/maquette.wasm
	cp $(SCAD_WASM_OUT) docs/maquette-scad.wasm
	cp $(GLTF_WASM_OUT) docs/maquette-gltf.wasm
	@echo "docs/ ready — serve with:  python3 -m http.server -d docs"

# --- maquette-scad: OpenSCAD/CSG plugin (workspace member, Manifold kernel) ---
SCAD_WASM_TARGET = target/wasm32-unknown-unknown/release/maquette_scad.wasm
SCAD_WASM_OUT = crates/maquette-scad/maquette-scad.wasm

# Manifold + Clipper2 are built via cmake against the wasm-cxx-shim
# toolchain (see crates/vendored/manifold-csg-sys/). Two flags govern
# the perf/reproducibility contract for scad-wasm:
#
#   MANIFOLD_WASM_CXX_FLAGS  extra C/C++ flags for the manifold build.
#                            `-flto` enables cross-TU LTO across manifold
#                            and Clipper2, worth 2–5% wall-clock on the
#                            wasmi interpreter (measured on nut/gear/menger).
#
#   RUSTFLAGS_SCAD_LINK      swaps rust-lld for emsdk's wasm-ld so LTO can
#                            actually run: rust-lld's LLVM 21 can't read
#                            manifold's LLVM 23 bitcode (emsdk clang);
#                            emsdk's wasm-ld (LLVM 23) reads both.
#                            Requires $EMSDK to be set at build time.
#
# Ship both — no override defaults, but if EMSDK is not set we fall back
# to the non-LTO path (rust-lld). CI runners without emsdk still work.
#
# When EMSDK is set, we ALSO point WASM_CXX_SHIM_LIBCXX_HEADERS at emsdk's
# bundled libc++ headers. Without this, the shim falls through to system
# libc++ (Debian bookworm's is too old for shim v0.5.0's __config_site
# override — errors like `use of undeclared identifier 'wcschr'`). Emsdk's
# headers are guaranteed compatible with the clang++ we're pairing with.
EMSDK_LLVM    := $(if $(EMSDK),$(EMSDK)/upstream/bin,)
EMSDK_WASMLD  := $(if $(EMSDK),$(EMSDK)/upstream/bin/wasm-ld,)
EMSDK_LIBCXX  := $(if $(EMSDK),$(EMSDK)/upstream/emscripten/system/lib/libcxx/include,)
ifeq ($(strip $(EMSDK_WASMLD)),)
SCAD_CXX_FLAGS =
SCAD_RUSTFLAGS_EXTRA =
SCAD_ENV =
else
SCAD_CXX_FLAGS = -flto
SCAD_RUSTFLAGS_EXTRA = -Clinker=$(EMSDK_WASMLD) -Clink-arg=--lto-O3
# Pin BOTH clang++ and libc++ to emsdk so the libcxx-extras compile (in
# build.rs) and the manifold+Clipper2 compile (via the shim's cmake) use
# the exact same toolchain. Without WASM_CXX_SHIM_LLVM_BIN_DIR the shim
# probes /usr/lib/llvm-N/bin first — on Debian bookworm that resolves to
# clang-14, whose builtin headers collide with emsdk's newer libc++ src.
SCAD_ENV = WASM_CXX_SHIM_LLVM_BIN_DIR=$(EMSDK_LLVM) WASM_CXX_SHIM_LIBCXX_HEADERS=$(EMSDK_LIBCXX)
endif

# Build + optimize the scad plugin wasm. Mirrors the core `wasm` recipe (same
# target features + wasm-opt flags) for consistency; adds the LTO recipe above.
scad-wasm:
	$(SCAD_ENV) \
	MANIFOLD_WASM_CXX_FLAGS="$(SCAD_CXX_FLAGS)" \
	RUSTFLAGS="$(RUSTFLAGS_WASM) $(SCAD_RUSTFLAGS_EXTRA)" \
	  cargo build --target wasm32-unknown-unknown --release -p maquette-scad
	wasm-opt -O3 --enable-simd --enable-bulk-memory --enable-sign-ext --enable-nontrapping-float-to-int --enable-mutable-globals --enable-multivalue --traps-never-happen --fast-math --closed-world --directize --inline-functions-with-loops --converge $(SCAD_WASM_TARGET) -o $(SCAD_WASM_OUT)
	@ls -lh $(SCAD_WASM_OUT)
	@[ -n "$(SCAD_CXX_FLAGS)" ] && echo "  (built with $(SCAD_CXX_FLAGS) via emsdk wasm-ld)" || echo "  (built without LTO — set EMSDK to enable)"

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
	cp crates/maquette-scad/maquette-scad.typ crates/maquette-scad/typst.toml $(dir $(SCAD_WASM_PKG))

.PHONY: wasm build harness doc doc-maquette doc-gltf doc-scad docs demo-assets demo scad-wasm scad-build gltf-wasm gltf-build
