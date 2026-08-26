// test/manifold-link/libcxx-extras.cpp
//
// Provides the libc++ source-file symbols that manifold pulls in but
// the main libcxx component intentionally doesn't ship:
//   * std::__1::__shared_count / __shared_weak_count out-of-line
//     methods (from libc++'s src/memory.cpp)
//   * std::nothrow global, std::__throw_bad_alloc (from libc++'s
//     src/new_helpers.cpp)
//   * std::bad_weak_ptr key functions
//   * std::align helper
//
// These are documented in libcxx/README.md as "consumers will surface
// link errors and need to compile [the upstream libc++ source files]
// themselves." This file IS that compilation, scoped to the manifold-
// link smoke. It's NOT part of the main libcxx component because it
// requires `#include <memory>` / `<new>` which we deliberately keep
// out of the main libcxx stubs (insulation from libc++ version
// drift). Including those headers couples this TU to libc++'s
// `__shared_count` / `__shared_weak_count` class layout — if libc++
// changes those layouts in a future release, this TU silently breaks
// (the manifold-tests run will catch it). We accept that coupling
// because the alternative — redeclaring the classes locally with
// matching layouts — is fragile in a different way.
//
// Runtime semantics: refcount manipulation is non-atomic (correct
// for our single-threaded wasm), and the destructor + __release_weak
// dispatch through the vtable to derived-class
// `__on_zero_shared_weak()` implementations — which DO free the
// controlled object. Lifetime correctness holds for single-threaded
// use; do not assume it generalizes to a future threaded build.
//
// References (the upstream files we're emulating):
//   https://github.com/llvm/llvm-project/blob/release/20.x/libcxx/src/memory.cpp
//   https://github.com/llvm/llvm-project/blob/release/20.x/libcxx/src/new_helpers.cpp

#include <memory>
#include <new>
#include <typeinfo>

// ---- std::nothrow + __throw_bad_alloc (from new_helpers.cpp) ----

namespace std {  // intentionally NOT versioned (matches libc++ upstream)

const nothrow_t nothrow{};

[[noreturn]] void __throw_bad_alloc() {
    __builtin_trap();
}

}  // namespace std

// ---- bad_weak_ptr key functions + shared_count/__shared_weak_count
//      out-of-line methods (from memory.cpp) ----

namespace std { inline namespace __1 {

bad_weak_ptr::~bad_weak_ptr() noexcept {}
const char* bad_weak_ptr::what() const noexcept { return "bad_weak_ptr"; }

__shared_count::~__shared_count() {}
__shared_weak_count::~__shared_weak_count() {}

void __shared_weak_count::__release_weak() noexcept {
    if (--__shared_weak_owners_ == -1) {
        __on_zero_shared_weak();
    }
}

__shared_weak_count* __shared_weak_count::lock() noexcept {
    long object_owners = __shared_owners_;
    while (object_owners != -1) {
        // single-threaded: just bump and return
        __shared_owners_ = object_owners + 1;
        return this;
    }
    return nullptr;
}

const void* __shared_weak_count::__get_deleter(const type_info&) const noexcept {
    return nullptr;
}

// std::align (rare-use helper, vendored from upstream verbatim)
void* align(size_t alignment, size_t size, void*& ptr, size_t& space) {
    void* r = nullptr;
    if (size <= space) {
        char* p1 = static_cast<char*>(ptr);
        char* p2 = reinterpret_cast<char*>(
            reinterpret_cast<__UINTPTR_TYPE__>(p1 + (alignment - 1)) & -alignment);
        size_t d = static_cast<size_t>(p2 - p1);
        if (d <= space - size) {
            r = p2;
            ptr = r;
            space -= d;
        }
    }
    return r;
}

// std::__hash_memory — libc++'s out-of-line memory hash, used by every
// hash-container instantiation (unordered_map, unordered_set, ...).
// Under LLVM 20 the symbol was elsewhere or inlined; LLVM 21 makes it
// an undefined extern that libc++ expects the runtime to provide. The
// shim doesn't ship libc++.a/.dylib, so we provide it here. Quality
// (avalanche, distribution) doesn't have to match libc++'s upstream
// implementation byte-for-byte — hash containers stay correct as long
// as the function is deterministic and a function of all input bytes.
// FNV-1a fits the bill in ~10 lines. Uses 32-bit constants because
// size_t is 32-bit on wasm32.
//
// LLVM 21 declares `__hash_memory` in `<__functional/hash.h>` with a
// `_LIBCPP_NOESCAPE` parameter attribute (expands to
// `[[_Clang::__noescape__]]`). Definition must match the declaration's
// parameter attributes or clang flags "conflicting types". We pull in
// the declaration if it exists (LLVM 21+) and use `_LIBCPP_NOESCAPE`
// in the definition (macro is defined unconditionally in `<__config>`
// across LLVM 20+, expanding to either the attribute or nothing).
}}  // close std::__1 to include libc++ headers at translation-unit scope

#if __has_include(<__functional/hash.h>)
#  include <__functional/hash.h>
#endif

namespace std { inline namespace __1 {

_LIBCPP_EXPORTED_FROM_ABI size_t __hash_memory(_LIBCPP_NOESCAPE const void* key, size_t length) noexcept {
    constexpr size_t fnv_offset_basis = 2166136261u;
    constexpr size_t fnv_prime        = 16777619u;
    size_t h = fnv_offset_basis;
    const unsigned char* p = static_cast<const unsigned char*>(key);
    for (size_t i = 0; i < length; ++i) {
        h ^= static_cast<size_t>(p[i]);
        h *= fnv_prime;
    }
    return h;
}

}}  // namespace std::__1
