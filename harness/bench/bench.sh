#!/usr/bin/env bash
# Per-plugin wasmi bench + baseline drift check.
#
#   ./bench.sh <plugin> [asset] [--save]
#
# <plugin>: gltf | scad | maquette
# [asset]:  optional asset name; defaults per-plugin.
# --save:   write baseline snapshot (output blob + fuel + wall) for
#           later drift comparison. Without --save, we bench current
#           build against the saved baseline.
#
# Prints a one-liner:
#   [plugin/asset  tag]  min=…  fuel=…  wasm=…B (±%)  cmp=…
set -euo pipefail

REPO=$(cd "$(dirname "$0")/../.." && pwd)
HARNESS=$REPO/harness/target/release/harness
BENCH_DIR=/tmp/bench
BASELINE_DIR=$BENCH_DIR/baseline
mkdir -p $BASELINE_DIR

plugin=${1:?usage: bench.sh <gltf|scad|maquette> [asset] [--save]}
asset=${2:-}
save=false
[ "${3:-}" = "--save" ] && save=true
[ "$asset" = "--save" ] && { save=true; asset=; }

# ── plugin-specific config ──────────────────────────────────────────────
case "$plugin" in
  gltf)
    wasm=$REPO/crates/maquette-gltf/maquette-gltf.wasm
    func=render_gltf
    asset_default=helmet.blg
    asset_path=$REPO/examples/data/gltf/${asset:-$asset_default}
    cfg=/tmp/bench/gltf-cfg.json
    cat > $cfg <<'EOF'
{"width": 512, "height": 512, "camera": [2.5, 1.5, 2.5], "center": [0, 0, 0],
 "up": [0, 1, 0], "fov": 40, "background": "#181820",
 "ibl": {"intensity": 1.2}, "ssao": {"samples": 16, "radius": 0.4}}
EOF
    ;;
  scad)
    wasm=$REPO/docs/maquette-scad.wasm
    func=build_scad
    asset_default=nut.scad
    asset_path=$REPO/examples/scad/data/${asset:-$asset_default}
    cfg=/tmp/bench/scad-cfg.json
    echo '{"fn": 32}' > $cfg
    ;;
  maquette)
    wasm=$REPO/maquette/maquette.wasm
    asset_default=teapot.obj
    asset_name=${asset:-$asset_default}
    asset_path=$REPO/examples/data/$asset_name
    # Dispatch fn by extension.
    case "$asset_name" in
      *.obj) func=render_obj_png ;;
      *.stl) func=render_stl_png ;;
      *.ply) func=render_ply_png ;;
      *) echo "unknown maquette asset ext: $asset_name" >&2; exit 1 ;;
    esac
    cfg=/tmp/bench/maquette-cfg.json
    cat > $cfg <<'EOF'
{"width": 512, "height": 512, "azimuth": 0, "elevation": 0,
 "background": "#f0f0f0", "antialias": 1}
EOF
    ;;
  all)
    for p in maquette gltf scad; do "$0" $p; done
    exit 0
    ;;
  *) echo "unknown plugin: $plugin" >&2; exit 1 ;;
esac

asset_tag=$(basename "$asset_path")
key=${plugin}-${asset_tag}
out=$BENCH_DIR/${key}.out
stats=$BENCH_DIR/${key}.stats
base_out=$BASELINE_DIR/${key}.out
base_stats=$BASELINE_DIR/${key}.stats

# ── run harness (bench=5, fuel=on) ──────────────────────────────────────
[ -x "$HARNESS" ] || (cd $REPO && cargo build --release --manifest-path harness/Cargo.toml)
[ -f "$wasm" ]     || { echo "wasm missing: $wasm" >&2; exit 1; }
[ -f "$asset_path" ] || { echo "asset missing: $asset_path" >&2; exit 1; }

# scad's build_scad takes 4 args (src, config, params, extras); other funcs take 2
extra_args=()
if [ "$plugin" = "scad" ]; then
  # 3rd arg = params dict (already in cfg); 4th arg = extras bundle (empty)
  extras=/tmp/bench/scad-extras.bin
  : > $extras
  extra_args=(--bench=3 --fuel $wasm $func $asset_path $cfg /tmp/bench/scad-params.bin $extras)
  echo '{}' > /tmp/bench/scad-params.bin
else
  extra_args=(--bench=5 --fuel $wasm $func $asset_path $cfg)
fi
$HARNESS "${extra_args[@]}" > $out 2> $stats

# ── extract metrics ─────────────────────────────────────────────────────
wasm_size=$(stat -c%s $wasm)
min=$(awk '/^min:/{print $2}' $stats)
fuel=$(awk '/^fuel:/{print $2}' $stats)

if $save; then
  cp $out   $base_out
  cp $stats $base_stats
  echo "[$key saved]  min=$min  fuel=$fuel  wasm=$wasm_size B"
  exit 0
fi

if [ ! -f "$base_out" ]; then
  echo "[$key first run — bench only, no baseline yet]  min=$min  fuel=$fuel  wasm=$wasm_size B"
  echo "                (rerun with --save to snapshot)"
  exit 0
fi

# ── compare vs baseline ─────────────────────────────────────────────────
base_size=$(awk '/^wasm=/{print $2}' <<<"$(grep -oE 'wasm=[0-9]+' $base_stats 2>/dev/null || echo)")
base_min_raw=$(awk '/^min:/{print $2}' $base_stats)
base_fuel=$(awk '/^fuel:/{print $2}' $base_stats)
sha_a=$(sha1sum $base_out | cut -c1-12)
sha_b=$(sha1sum $out      | cut -c1-12)
cmp_status=$([ "$sha_a" = "$sha_b" ] && echo "identical" || echo "$sha_a→$sha_b")

# Times come out as "5.044s", "41.308ms", "10.611µs" — normalise to ms for math.
to_ms() { python3 -c "
import re, sys
s = sys.argv[1]
m = re.match(r'([0-9.]+)(µs|ms|s|us)?', s)
v = float(m.group(1)); u = m.group(2) or 's'
print({'s': v*1000, 'ms': v, 'us': v/1000, 'µs': v/1000}[u])
" "$1"; }
cur_ms=$(to_ms "$min")
base_ms=$(to_ms "$base_min_raw")
pct_min=$(python3 -c "print(f'{100*($cur_ms - $base_ms)/$base_ms:+.1f}%')")
pct_fuel=$(python3 -c "print(f'{100*($fuel - $base_fuel)/$base_fuel:+.2f}%')")
printf '[%s]  min=%s (%s)  fuel=%s (%s)  wasm=%d B  cmp=%s\n' \
  "$key" "$min" "$pct_min" "$fuel" "$pct_fuel" "$wasm_size" "$cmp_status"
