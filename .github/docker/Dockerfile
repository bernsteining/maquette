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

RUN rustup target add wasm32-unknown-unknown

ARG BINARYEN_VERSION=version_128
RUN curl -fsSL --retry 3 --retry-delay 5 --retry-all-errors \
      "https://github.com/WebAssembly/binaryen/releases/download/${BINARYEN_VERSION}/binaryen-${BINARYEN_VERSION}-x86_64-linux.tar.gz" \
      | tar -xz -C /opt \
    && ln -s /opt/binaryen-${BINARYEN_VERSION}/bin/* /usr/local/bin/

ARG EMSDK_VERSION=6.0.8
RUN git clone --depth 1 https://github.com/emscripten-core/emsdk.git /opt/emsdk \
    && /opt/emsdk/emsdk install "${EMSDK_VERSION}" \
    && /opt/emsdk/emsdk activate "${EMSDK_VERSION}" \
    && rm -rf /opt/emsdk/.git /opt/emsdk/downloads
ENV EMSDK=/opt/emsdk
