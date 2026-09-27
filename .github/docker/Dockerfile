# CI toolchain image for the maquette plugins. Baking the deps in removes the
# per-job apt install + emsdk download + binaryen fetch from every workflow run.
# Rust version tracks rust-toolchain.toml; binaryen/emsdk track the Makefile.
FROM rust:1.94.1-slim-bookworm

# scad needs cmake + LLVM/clang + libc++ (Manifold C++ CSG kernel via
# wasm-cxx-shim); git/curl/make/xz/python back the rest of the build.
RUN apt-get update && apt-get install -y --no-install-recommends \
      git curl ca-certificates make xz-utils \
      cmake clang lld libc++-dev libc++abi-dev python3 \
    && rm -rf /var/lib/apt/lists/*

RUN rustup target add wasm32-unknown-unknown && rustup component add clippy

# Only wasm-opt (Makefile) and wasm-dis (wasm-invariants.sh) are used; both
# are statically linked, so the rest of the release tarball is skipped.
ARG BINARYEN_VERSION=version_128
RUN curl -fsSL --retry 3 --retry-delay 5 --retry-all-errors \
      "https://github.com/WebAssembly/binaryen/releases/download/${BINARYEN_VERSION}/binaryen-${BINARYEN_VERSION}-x86_64-linux.tar.gz" \
      | tar -xz -C /usr/local/bin --strip-components=2 \
          "binaryen-${BINARYEN_VERSION}/bin/wasm-opt" "binaryen-${BINARYEN_VERSION}/bin/wasm-dis"

# emsdk only supplies the scad LTO toolchain (clang, wasm-ld, llvm-ar/ranlib)
# and its libc++ headers; node, the prebuilt emscripten sysroot cache and its
# binaryen copy are never used, so they are dropped in the same layer.
ARG EMSDK_VERSION=6.0.8
RUN git clone --depth 1 https://github.com/emscripten-core/emsdk.git /opt/emsdk \
    && /opt/emsdk/emsdk install "${EMSDK_VERSION}" \
    && /opt/emsdk/emsdk activate "${EMSDK_VERSION}" \
    && rm -rf /opt/emsdk/.git /opt/emsdk/downloads /opt/emsdk/node \
         /opt/emsdk/upstream/emscripten/cache /opt/emsdk/upstream/emscripten/node_modules \
         /opt/emsdk/upstream/emscripten/test \
    && cd /opt/emsdk/upstream/bin \
    && rm -f clangd clang-scan-deps llvm-dwarfdump llvm-profdata llvm-cov llvm-readobj llvm-dwp \
    && find . -type f \( -name 'wasm*' -o -name 'binaryen*' \) -delete
ENV EMSDK=/opt/emsdk
