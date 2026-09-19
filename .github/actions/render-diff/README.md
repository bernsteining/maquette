# maquette 3D render diff — GitHub Action

On a pull request that changes 3D model files (`.stl` `.obj` `.ply` `.glb` `.gltf` `.scad`), render each one **before** (base) and **after** (head) and post the images as a sticky PR comment — a *visual diff* for geometry, where a text diff is unreadable.

## Usage

```yaml
name: 3D render diff
on:
  pull_request:
    paths: ["**.scad", "**.stl", "**.obj", "**.ply", "**.glb", "**.gltf"]
permissions:
  contents: write        # host rendered images on a render-diff-assets branch
  pull-requests: write   # post the comment
jobs:
  render-diff:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
        with: { fetch-depth: 2 }        # base + head is enough
      - uses: bernsteining/maquette/.github/actions/render-diff@v1
        with:
          config: .github/render-diff.json   # optional render settings
          # cli-version: v0.1.4              # use a prebuilt CLI instead of building
```

## Inputs

| Input | Default | Description |
|---|---|---|
| `github-token` | `${{ github.token }}` | Needs `contents:write` + `pull-requests:write`. |
| `config` | `.github/render-diff.json` | Render settings (flat dict, or `{default, overrides:[{match,config}]}` for per-model). Optional. |
| `args` | `""` | Extra maquette flags applied on top (e.g. `--set antialias=2`). |
| `jobs` | `4` | Parallel render jobs. |
| `cli-version` | `""` | Prebuilt CLI release tag to download; empty = build from source. |

## Render settings

`config` uses the maquette render-config keys (see the schema in the main repo). Per-model example:

```json
{
  "default": { "width": 600, "azimuth": 30, "background": "#182028" },
  "overrides": [
    { "match": "*.glb",        "config": { "up": [0,1,0], "shadows": true } },
    { "match": "brackets/*.scad", "config": { "azimuth": 90 } }
  ]
}
```

Same-repo PRs only (a fork PR gets a read-only token). Images are hosted on a `render-diff-assets` branch in your repo.
