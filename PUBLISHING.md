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

## crates.io — not a target (blocked)

`cargo publish` does **not** work for this family: the plugins carry a **git** dependency (`gltf` pinned to a rev) and a **path** dependency (the vendored `manifold-csg-sys`), both of which crates.io rejects. `maquette-cli` / `maquette-py` transitively depend on those, so they can't publish either. Distribution goes through Typst Universe (plugins), GitHub Releases (CLI), and PyPI (Python) instead. Publishing to crates.io would require upstreaming a `gltf` release and a `manifold-csg-sys` release first.
