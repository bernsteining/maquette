# Wrapper around wasm-cxx-shim's `wasm_cxx_shim_add_manifold()` helper.
#
# The helper (added in shim v0.3.0) absorbs the high-change-rate parts
# of the integration cocktail: FetchContent of manifold + Clipper2 with
# tested-pin defaults, the three carry-patches (MANIFOLD_NO_IOSTREAM /
# MANIFOLD_NO_FILESYSTEM / CLIPPER2_NO_IOSTREAM), manifold's CMake
# options, and the base compile flags. We're left with the consumer-side
# specifics: the `-isystem` chain (libc++ headers + our `__config_site`
# override + the `<mutex>` stub).
#
# Invoked from manifold-csg-sys/build.rs via:
#
#   cmake -S wasm32-uu/ -B <out>/manifold-build-wasm32-uu \
#       -DCMAKE_TOOLCHAIN_FILE=<shim>/cmake/toolchain-wasm32.cmake \
#       -DWASM_CXX_SHIM_DIR=<shim_src> \
#       -DWASM32_UU_INC_DIR=<wasm32-uu/include> \
#       -DLIBCXX_HEADERS=<llvm>/include/c++/v1
#
# After the configure, `cmake --build` produces:
#   _deps/manifold-build/bindings/c/libmanifoldc.a
#   _deps/manifold-build/src/libmanifold.a
#   _deps/clipper2-build/libClipper2.a
# which build.rs picks up via find_lib_recursive() and emits as
# rustc-link-lib=static.
#
# This `_deps/<name>-build/...` layout is part of the contract between us
# and the helper — it's what cmake's FetchContent module produces and what
# the helper's docstring documents. If the helper or cmake changes that
# shape, build.rs's `find_lib_recursive` will panic with an actionable
# error pointing at the problem.

cmake_minimum_required(VERSION 3.20)
project(manifold-csg-wasm-uu LANGUAGES CXX C)

if(NOT WASM_CXX_SHIM_DIR)
    message(FATAL_ERROR "Set -DWASM_CXX_SHIM_DIR=<path> to the cloned wasm-cxx-shim source tree.")
endif()
if(NOT WASM32_UU_INC_DIR)
    message(FATAL_ERROR "Set -DWASM32_UU_INC_DIR=<path> to crates/manifold-csg-sys/wasm32-uu/include.")
endif()
if(NOT LIBCXX_HEADERS)
    message(FATAL_ERROR "Set -DLIBCXX_HEADERS=<path> to the LLVM libc++ v1 headers directory.")
endif()

include(${WASM_CXX_SHIM_DIR}/cmake/WasmCxxShimManifold.cmake)

# `-isystem` chain — caller responsibility per the helper's docstring.
# Each `-isystem PATH` pair MUST be wrapped in `SHELL:` or CMake's
# compile-flag deduplication collapses repeat `-isystem` flags
# together (only the first path wins; the rest get treated as linker
# inputs and emit confusing "input unused" warnings).
#
# Order is significant: our `__config_site` override + `<mutex>` stub
# must precede LLVM's libc++ headers so the override wins. libm/libc
# come last because they're the lowest-priority fallbacks.
add_compile_options(
    "SHELL:-isystem ${WASM32_UU_INC_DIR}"
    "SHELL:-isystem ${LIBCXX_HEADERS}"
    "SHELL:-isystem ${WASM_CXX_SHIM_DIR}/libm/include"
    "SHELL:-isystem ${WASM_CXX_SHIM_DIR}/libc/include"
    # Defense-in-depth: drop standard system include directories but
    # keep the compiler resource dir so builtin headers like <stdint.h>
    # still resolve. Even if the explicit -isystem chain misbehaves,
    # host C/C++ system paths stay excluded.
    -nostdlibinc
)

# FetchContent + manifold/Clipper2 options + base compile flags.
# Host build.rs passes -DMANIFOLD_GIT_TAG so the wasm-uu lane builds
# against the same manifold commit the host pins. The shim's tested-pin
# default (currently 3ce9622b) is older than typical host pins; without
# this override, the wasm-uu lane would link against an older C API
# surface than our FFI declarations target.
if(MANIFOLD_GIT_TAG)
    wasm_cxx_shim_add_manifold(MANIFOLD_GIT_TAG ${MANIFOLD_GIT_TAG})
else()
    wasm_cxx_shim_add_manifold()
endif()

# Force the manifold + Clipper2 archives to build by default. Without
# this, `cmake --build` only builds targets marked ALL; FetchContent
# subprojects' libraries aren't ALL by default in some configurations.
add_custom_target(_force_manifold_build ALL
    DEPENDS manifoldc manifold Clipper2)
