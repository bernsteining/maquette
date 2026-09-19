# Publishing / distribution

How each piece of the maquette family ships. Nothing here publishes automatically without an explicit tag or manual trigger.

## Typst plugins → Typst Universe

`maquette`, `maquette-gltf`, `maquette-scad` are wasm plugins published to the [Typst Universe](https://typst.app/universe/) `@preview` registry. That is a **manual PR** to `typst/packages` (Typst-team review); there is no automated publish. `make build`/`gltf-build`/`scad-build` produce the wasm each package ships.

## Native CLI → GitHub Releases (dist)

`crates/maquette-cli` ships as prebuilt binaries + installers via [dist](https://opensource.axo.dev/cargo-dist/) (`dist-workspace.toml`, `.github/workflows/release.yml`). Cut a release:

```sh
git tag v0.1.4 && git push --tags
```

→ builds `maquette` for linux-gnu / macOS x64+arm / windows-msvc, creates the GitHub Release with `curl … | sh` + PowerShell installers; `build.yml` then attaches the plugin wasm.

## Python bindings → PyPI

`crates/maquette-py` (pyo3/maturin) builds abi3 wheels + sdist via `.github/workflows/pypi.yml` (maturin-action). Wheels are built on every `v*` tag and on manual dispatch; they upload as **artifacts**. To actually publish:

1. Create the PyPI project and configure a [Trusted Publisher](https://docs.pypi.org/trusted-publishers/) for this repo + the `pypi` environment.
2. Push a `v*` tag — the `publish` job then uploads via OIDC (no token stored).

Until step 1, the publish job no-ops/fails harmlessly and you still get downloadable wheels.

## Render-diff → GitHub Actions Marketplace

`.github/actions/render-diff/` is a self-contained composite action. To list it on the Marketplace:

1. It must live at the **root** of a repository (Marketplace requirement). Either move this directory to a dedicated repo (e.g. `bernsteining/maquette-render-diff`) with `action.yml` at the root, or keep it here and reference it as `bernsteining/maquette/.github/actions/render-diff@v1` (works, but not Marketplace-listable).
2. Tag a release (`v1`, plus a moving `v1` major tag) and check "Publish to Marketplace".

The repo can also **dogfood** it by pointing `.github/workflows/render-diff.yml` at `uses: ./.github/actions/render-diff` (removes the duplicated script).

## npm → JavaScript/wasm

`packages/maquette-js` (`maquette-render` — `maquette` on npm is an unrelated vdom lib) packages the plugin wasm + a JS loader (the same protocol the demo worker uses). `npm run build` stages the wasm into `wasm/`; `npm publish` ships it. A CI job could `npm publish --provenance` on a `v*` tag with an `NPM_TOKEN`. No native build needed — it reuses the wasm.

## C ABI → other-language bindings

`crates/maquette-c` builds `libmaquette.{so,dylib,dll,a}` + `include/maquette.h` — the universal binding layer (C, C++, Ruby, Julia, Go/cgo, C#, R…). There is no single registry for C libraries; ship the built libs + header via a GitHub Release (they could ride the dist release), or let downstreams vendor the crate.

## Nix

`flake.nix` builds the CLI **from source, hermetically** (the real Nix way — not a prebuilt blob):

- `nix run github:bernsteining/maquette` / `nix build` — `rustPlatform.buildRustPackage`. Cargo deps are vendored from `Cargo.lock`; the Manifold CSG kernel and its Clipper2 dependency (normally git-cloned by a build script — forbidden in the sandbox) are pre-fetched as fixed-output derivations and injected via `MANIFOLD_SRC` / `CLIPPER2_SRC`, which the vendored `manifold-csg-sys` build script honours to run a fully network-free cmake build.
- `nix develop` — a dev shell (rust, cmake, clang, binaryen, git) to hack on the CLI + wasm.

First build: replace the `lib.fakeHash` placeholders — the two source FODs and the four git deps in `cargoLock.outputHashes` — with the hashes Nix prints. The offline `MANIFOLD_SRC`/`CLIPPER2_SRC` support is validated natively; the Nix derivation itself is unverified until someone runs it (no `nix` in the dev environment here). Builds on any Linux/macOS system (it compiles from source).

## Linux packages (deb / rpm / apk / pacman)

Not set up yet. The handy route is **nFPM** or **GoReleaser**: one config turns the release binary into `.deb`/`.rpm`/`.apk`/pacman packages you host (attach to the GitHub Release, or your own apt/rpm repo). Getting into the *official* Debian/Arch/Void/Fedora repos is a per-distro maintainer submission, not a push.

## crates.io — not a target (blocked)

`cargo publish` does **not** work for this family: the plugins carry a **git** dependency (`gltf` pinned to a rev) and a **path** dependency (the vendored `manifold-csg-sys`), both of which crates.io rejects. `maquette-cli` / `maquette-py` transitively depend on those, so they can't publish either. Distribution goes through Typst Universe (plugins), GitHub Releases (CLI), and PyPI (Python) instead. Publishing to crates.io would require upstreaming a `gltf` release and a `manifold-csg-sys` release first.
