#!/bin/sh
# Build the maquette-scad Typst plugin (wasm32-unknown-unknown) and copy it next
# to maquette-scad.typ. The Manifold C++ kernel is compiled into the module via
# wasm-cxx-shim, so the result is self-contained (0 host imports) and runs under
# Typst's wasmi.
#
# Requires a wasm-capable LLVM 20+ WITH llvm-ar and libc++ headers. This machine
# has no system LLVM binutils, so we borrow emsdk's bundled LLVM toolchain.
# Override EMSDK if yours lives elsewhere.
set -e
cd "$(dirname "$0")"

EMSDK="${EMSDK:-$HOME/.local/share/emsdk}"
EMB="$EMSDK/upstream/bin"
LIBCXX="$EMSDK/upstream/emscripten/cache/sysroot/include/c++/v1"

if [ ! -x "$EMB/llvm-ar" ] || [ ! -e "$LIBCXX/vector" ]; then
  echo "error: need emsdk LLVM at $EMB (llvm-ar) and libc++ headers at $LIBCXX" >&2
  echo "       set EMSDK to your emscripten SDK root, or install clang+lld+libc++ for wasm." >&2
  exit 1
fi

export PATH="$EMB:$PATH"
export WASM_CXX_SHIM_LLVM_BIN_DIR="$EMB"
export WASM_CXX_SHIM_LIBCXX_HEADERS="$LIBCXX"

# manifold-csg's wasm features are selected by the target-specific dependency
# table in Cargo.toml, so no --features flag is needed here.
cargo build --release --target wasm32-unknown-unknown

cp target/wasm32-unknown-unknown/release/maquette_scad.wasm maquette-scad.wasm

# Post-optimize with wasm-opt if present (~11% smaller). The module carries wasm
# SIMD (clang auto-vectorized Manifold's C++) + bulk-memory etc., so those features
# must be enabled for the validator. Skipped gracefully if wasm-opt isn't installed.
if command -v wasm-opt >/dev/null 2>&1; then
  wasm-opt -O2 \
    --enable-simd --enable-bulk-memory --enable-sign-ext \
    --enable-mutable-globals --enable-nontrapping-float-to-int --enable-multivalue \
    maquette-scad.wasm -o maquette-scad.wasm.opt && mv maquette-scad.wasm.opt maquette-scad.wasm
  echo "wasm-opt: applied"
else
  echo "wasm-opt: not found, shipping unoptimized wasm"
fi
echo "built maquette-scad.wasm ($(wc -c < maquette-scad.wasm) bytes)"
