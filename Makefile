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
	cp $(WASM_OUT) $(WASM_PKG)

harness:
	cargo build --release --manifest-path harness/Cargo.toml

doc: build
	typst compile examples/documentation.typ examples/documentation.pdf --root .

# Model files the browser demo ships. Canonical copies live once in
# examples/data/; the demo needs them under docs/ (the only dir Pages
# publishes). Single source of truth for the demo's model set — shared by
# `make demo` (local) and the Pages CI, so neither hard-codes the list.
DEMO_MODELS = bunny.obj teapot.obj crankshaft.obj brain_skull.obj rubi_scan.ply

# Copy the demo models into docs/ (gitignored there — regenerated, not committed).
demo-assets:
	cp $(addprefix examples/data/,$(DEMO_MODELS)) docs/

# Assemble the demo dir locally: fresh wasm + models, ready to serve.
demo: wasm demo-assets
	cp $(WASM_OUT) docs/maquette.wasm
	@echo "docs/ ready — serve with:  python3 -m http.server -d docs"

.PHONY: wasm build harness doc demo-assets demo
