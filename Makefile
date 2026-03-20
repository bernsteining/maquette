WASM_TARGET = target/wasm32-unknown-unknown/release/maquette.wasm
WASM_OUT = maquette/maquette.wasm
WASM_PKG = $(HOME)/.local/share/typst/packages/local/maquette/0.1.0/maquette.wasm

build:
	cargo build --target wasm32-unknown-unknown --release
	wasm-opt -O3 --enable-simd --enable-bulk-memory --enable-sign-ext --enable-nontrapping-float-to-int --enable-mutable-globals --enable-multivalue --traps-never-happen --fast-math --closed-world --directize --inline-functions-with-loops --converge $(WASM_TARGET) -o $(WASM_OUT)
	cp $(WASM_OUT) $(WASM_PKG)
	@ls -lh $(WASM_OUT)

harness:
	cargo build --release --manifest-path harness/Cargo.toml

doc: build
	typst compile examples/documentation.typ examples/documentation.pdf --root .
