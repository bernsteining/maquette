#!/usr/bin/env bash
# Collect host + toolchain info for a wasm32-unknown-unknown build of
# manifold-csg. Run from the repo root and attach the output to a bug
# report:
#
#   bash crates/manifold-csg-sys/wasm32-uu/diagnose.sh > bugreport.txt 2>&1
#
# Read-only and idempotent. Sections mirror the LLVM probe ladder in
# build.rs (find_llvm / candidate_llvm_bin_dirs) so the report says what
# build.rs would see.

set -u

# Resolve repo root from this script's location so users can run it
# from anywhere (./.cargo/config.toml, target/..., etc. below are
# resolved relative to the repo, not pwd).
script_dir="$(cd "$(dirname "$0")" && pwd)"
repo_root="$(cd "$script_dir/../../.." && pwd)"
cd "$repo_root"

section() {
    printf '\n=== %s ===\n' "$1"
}

# Run a command if available; otherwise print "(not installed)".
safe() {
    if command -v "$1" >/dev/null 2>&1; then
        "$@" 2>&1 || printf '(exit %d)\n' "$?"
    else
        printf '(%s not installed)\n' "$1"
    fi
}

# Print the contents of a file if it exists, else a marker line.
dump_file() {
    if [ -f "$1" ]; then
        cat "$1"
    else
        printf '(missing: %s)\n' "$1"
    fi
}

# Tail a file with a header, capped to N lines, if it exists.
tail_file() {
    local path="$1" lines="$2"
    if [ -f "$path" ]; then
        printf '\n--- %s (last %s lines) ---\n' "$path" "$lines"
        tail -n "$lines" "$path"
    fi
}

section "Bug report header"
printf 'date:        %s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
printf 'pwd:         %s\n' "$(pwd)"
if command -v git >/dev/null 2>&1 && git rev-parse --git-dir >/dev/null 2>&1; then
    printf 'git sha:     %s\n' "$(git rev-parse HEAD)"
    printf 'git branch:  %s\n' "$(git rev-parse --abbrev-ref HEAD)"
    printf 'dirty files: %s\n' "$(git status --porcelain | wc -l | tr -d ' ')"
else
    printf 'git:         (not a git checkout)\n'
fi

section "Host"
printf 'uname:       %s\n' "$(uname -a)"
if [ -f /etc/os-release ]; then
    printf -- '--- /etc/os-release ---\n'
    cat /etc/os-release
fi
if command -v sw_vers >/dev/null 2>&1; then
    printf -- '--- sw_vers ---\n'
    sw_vers
fi

section "Rust toolchain"
safe rustc -vV
printf -- '--- cargo ---\n'
safe cargo -vV
printf -- '--- rustup show active-toolchain ---\n'
safe rustup show active-toolchain
printf -- '--- rustup target list (wasm32) ---\n'
if command -v rustup >/dev/null 2>&1; then
    rustup target list --installed 2>&1 | grep -i wasm32 || printf '(no wasm32 targets installed)\n'
else
    printf '(rustup not installed)\n'
fi

section "Build tools"
safe cmake --version
printf -- '--- make ---\n'
if command -v make >/dev/null 2>&1; then
    make --version 2>&1 | head -1
else
    printf '(make not installed)\n'
fi
printf -- '--- ninja ---\n'
safe ninja --version
printf -- '--- git ---\n'
safe git --version
printf -- '--- node ---\n'
safe node --version

section "LLVM probe ladder"
# Mirrors candidate_llvm_bin_dirs() in build.rs. For each candidate:
# does clang++ exist there, what does --version say, and are the
# libc++ headers we'd consume present?
probe_llvm_dir() {
    local bin_dir="$1"
    local clangpp="$bin_dir/clang++"
    if [ ! -x "$clangpp" ]; then
        printf '%-50s (no clang++)\n' "$bin_dir"
        return
    fi
    local ver
    ver=$("$clangpp" --version 2>&1 | head -1)
    printf '%-50s %s\n' "$bin_dir" "$ver"
    local llvm_root="${bin_dir%/bin}"
    local found=""
    for rel in include/c++/v1 lib/c++/v1; do
        if [ -f "$llvm_root/$rel/vector" ]; then
            found="$llvm_root/$rel"
            break
        fi
    done
    if [ -n "$found" ]; then
        printf '  libc++ headers: %s\n' "$found"
    else
        printf '  libc++ headers: (NOT FOUND under %s)\n' "$llvm_root"
    fi
}

# Same order as candidate_llvm_bin_dirs() in build.rs.
if [ -n "${WASM_CXX_SHIM_LLVM_BIN_DIR-}" ]; then
    printf 'WASM_CXX_SHIM_LLVM_BIN_DIR is set:\n'
    probe_llvm_dir "$WASM_CXX_SHIM_LLVM_BIN_DIR"
    printf '\n'
fi
for v in 22 21 20 19 18; do
    probe_llvm_dir "/opt/homebrew/opt/llvm@${v}/bin"
    probe_llvm_dir "/usr/local/opt/llvm@${v}/bin"
    probe_llvm_dir "/usr/lib/llvm-${v}/bin"
done
probe_llvm_dir "/opt/homebrew/opt/llvm/bin"
probe_llvm_dir "/usr/local/opt/llvm/bin"
printf '\n--- which -a clang++ ---\n'
if command -v clang++ >/dev/null 2>&1; then
    # `which -a` is non-portable; fall back to a manual PATH walk.
    type -a clang++ 2>/dev/null || command -v clang++
    clang++ --version 2>&1 | head -1
else
    printf '(no clang++ on PATH)\n'
fi

section "System libc++ shadow check"
# JaminKoke's failure mode: system libc++ at /usr/include/c++/v1
# leaked into the build instead of the LLVM-versioned one. If this
# directory exists, it's a candidate culprit.
for path in /usr/include/c++/v1 /usr/include/c++/v1/__configuration/hardening.h; do
    if [ -e "$path" ]; then
        printf 'FOUND: %s\n' "$path"
    else
        printf 'absent: %s\n' "$path"
    fi
done

section "Environment variables"
for var in WASM_CXX_SHIM_LLVM_BIN_DIR WASM_CXX_SHIM_LIBCXX_HEADERS \
           CC CXX CFLAGS CXXFLAGS LDFLAGS \
           RUSTFLAGS CARGO_BUILD_TARGET \
           CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_LINKER \
           CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUSTFLAGS \
           CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER \
           CMAKE_GENERATOR CMAKE_TOOLCHAIN_FILE; do
    if [ -n "${!var-}" ]; then
        printf '%s=%s\n' "$var" "${!var}"
    else
        printf '%s=<unset>\n' "$var"
    fi
done
printf -- '\n--- ./.cargo/config.toml ---\n'
dump_file ./.cargo/config.toml
printf -- '\n--- ~/.cargo/config.toml ---\n'
dump_file "$HOME/.cargo/config.toml"

section "Cached build state"
# Walk target/ for the most recent manifold-csg-sys build dir and
# report on the cached wasm-cxx-shim / Clipper2 / manifold artifacts
# that build.rs leaves there. Useful for diagnosing stale-cache issues
# and for confirming what the build was configured against.
shopt -s nullglob
build_dirs=()
for d in target/*/build/manifold-csg-sys-*/out target/wasm32-unknown-unknown/*/build/manifold-csg-sys-*/out; do
    [ -d "$d" ] && build_dirs+=("$d")
done
if [ "${#build_dirs[@]}" -eq 0 ]; then
    printf '(no manifold-csg-sys build dirs under target/ — nothing cached yet)\n'
else
    # Pick the most recently modified one. `ls -td` is portable enough.
    out_dir=$(ls -td "${build_dirs[@]}" 2>/dev/null | head -1)
    printf 'most recent: %s\n' "$out_dir"
    for stamp in .shim-version-stamp .clipper2-version-stamp .manifold-wasm32-uu-version-stamp; do
        if [ -f "$out_dir/$stamp" ]; then
            printf '%s: %s\n' "$stamp" "$(cat "$out_dir/$stamp")"
        else
            printf '%s: (missing)\n' "$stamp"
        fi
    done
    for lib in wasm-cxx-shim-build/libc/libwasm-cxx-shim-libc.a \
               wasm-cxx-shim-build/libcxx/libwasm-cxx-shim-libcxx.a \
               wasm-cxx-shim-build/libm/libwasm-cxx-shim-libm.a \
               manifold-build-wasm32-uu/src/libmanifold.a \
               manifold-build-wasm32-uu/bindings/c/libmanifoldc.a \
               manifold-build-wasm32-uu/_deps/clipper2-build/libClipper2.a; do
        if [ -f "$out_dir/$lib" ]; then
            size=$(wc -c < "$out_dir/$lib" | tr -d ' ')
            printf '  %s (%s bytes)\n' "$lib" "$size"
        else
            printf '  %s (missing)\n' "$lib"
        fi
    done
    # Pull a few interesting lines out of CMakeCache.txt — the full
    # file is huge and noisy; these tell us what compiler + flags
    # cmake locked in.
    for cache in wasm-cxx-shim-build/CMakeCache.txt manifold-build-wasm32-uu/CMakeCache.txt; do
        path="$out_dir/$cache"
        if [ -f "$path" ]; then
            printf '\n--- %s (selected lines) ---\n' "$cache"
            grep -E '^(CMAKE_(C|CXX)_COMPILER|CMAKE_TOOLCHAIN_FILE|CMAKE_(C|CXX)_FLAGS_INIT|CMAKE_BUILD_TYPE|FETCHCONTENT_SOURCE_DIR_CLIPPER2):' "$path" || \
                printf '(no matching lines)\n'
        fi
    done
    # CMake's own diagnostic logs — when configure fails these are
    # the gold-standard artifacts.
    for log in wasm-cxx-shim-build/CMakeFiles/CMakeError.log \
               wasm-cxx-shim-build/CMakeFiles/CMakeOutput.log \
               manifold-build-wasm32-uu/CMakeFiles/CMakeError.log \
               manifold-build-wasm32-uu/CMakeFiles/CMakeOutput.log; do
        tail_file "$out_dir/$log" 200
    done
fi

section "Footer"
cat <<'EOF'
What to attach: the entire output of this script (stdout+stderr).

If you have a partial build failure log from `cargo build`, attach
that too — the cmake/make output streams live and isn't captured
here.

Re-run before posting if you've changed env vars or installed new
toolchain components.
EOF
