#!/usr/bin/env bash
# Render the 3D model files changed on this branch, before (base) and after
# (head), and emit a Markdown comment body with a before/after table per file.
# Image URLs are the token %%IMGBASE%% for the caller to substitute once the
# PNGs are hosted. Rendering runs in parallel (JOBS); the comment is assembled
# in order afterwards. Local use: build the CLI, then
#   .github/scripts/render-diff.sh <base-ref> [out-dir]
set -euo pipefail

BASE_REF="${1:?usage: render-diff.sh <base-ref> [out-dir]}"
OUT="${2:-render-diff-out}"
BIN="${MAQUETTE_BIN:-target/release/maquette}"
MAX_FILES="${MAX_FILES:-12}"
JOBS="${JOBS:-4}"

# Render settings are user-configurable via a repo config file (the same keys
# the plugins accept — see render-config.schema.json). Two shapes:
#   * flat:       a plain render-config dict, applied to every model.
#   * per-model:  { "default": {...}, "overrides": [ {"match":"*.glb","config":{...}}, ... ] }
#                 each model gets `default` deep-merged with the first override
#                 whose glob matches its path (first match wins).
# MAQUETTE_ARGS (extra `--set k=v`) always wins on top; with no config file a
# sensible default framing is used.
CFG="${RENDER_DIFF_CONFIG:-.github/render-diff.json}"
DEFAULT_ARGS="--set width=460 --set height=360 --set azimuth=28 --set elevation=22"
STRUCTURED=0
if [ -f "$CFG" ] && jq -e 'has("overrides") or has("default")' "$CFG" >/dev/null 2>&1; then
  STRUCTURED=1
  ARGS_STR="$DEFAULT_ARGS ${MAQUETTE_ARGS:-}"
elif [ -f "$CFG" ]; then
  ARGS_STR="--config $CFG ${MAQUETTE_ARGS:-}"
else
  ARGS_STR="$DEFAULT_ARGS ${MAQUETTE_ARGS:-}"
fi

mkdir -p "$OUT"
COMMENT="$OUT/comment.md"
{
  echo "<!-- maquette-render-diff -->"
  echo "### 🧊 3D render diff"
  echo
} > "$COMMENT"

mapfile -t files < <(git diff --name-only --diff-filter=d "$BASE_REF"...HEAD \
  | grep -iE '\.(scad|stl|obj|ply|glb|gltf)$' || true)

if [ "${#files[@]}" -eq 0 ]; then
  echo "_No 3D model files changed._" >> "$COMMENT"
  echo "no model changes"
  exit 0
fi

truncated=0
if [ "${#files[@]}" -gt "$MAX_FILES" ]; then
  truncated=$(( ${#files[@]} - MAX_FILES ))
  files=("${files[@]:0:$MAX_FILES}")
fi

safe_name() { echo "$1" | tr '/. ' '___'; }

# Per-model config: merge `default` with the first matching override into a
# per-file JSON that render_one picks up via --config.
if [ "$STRUCTURED" = 1 ]; then
  base_def="$(jq -c '.default // {}' "$CFG")"
  n_ov="$(jq '(.overrides // []) | length' "$CFG")"
  for f in "${files[@]}"; do
    cfg="$base_def"
    for ((i = 0; i < n_ov; i++)); do
      pat="$(jq -r ".overrides[$i].match" "$CFG")"
      if [[ "$f" == $pat ]]; then
        ov="$(jq -c ".overrides[$i].config // {}" "$CFG")"
        cfg="$(jq -cn --argjson a "$cfg" --argjson b "$ov" '$a * $b')"
        break
      fi
    done
    printf '%s\n' "$cfg" > "$OUT/$(safe_name "$f").cfg.json"
  done
fi

render_one() {
  local f="$1" ext safe before after src pfc
  ext="${f##*.}"
  safe="$(echo "$f" | tr '/. ' '___')"
  before="$OUT/$safe.before.png"
  after="$OUT/$safe.after.png"
  pfc="$OUT/$safe.cfg.json"
  if [ -f "$pfc" ]; then
    read -ra A <<< "--config $pfc ${MAQUETTE_ARGS:-}"
  else
    read -ra A <<< "$ARGS_STR"
  fi
  "$BIN" "$f" -o "$after" "${A[@]}" >/dev/null 2>&1 || true
  if git cat-file -e "$BASE_REF:$f" 2>/dev/null; then
    src="$OUT/$safe.beforesrc.$ext"
    git show "$BASE_REF:$f" > "$src"
    "$BIN" "$src" -o "$before" "${A[@]}" >/dev/null 2>&1 || true
  fi
}
export -f render_one
export BIN OUT BASE_REF ARGS_STR MAQUETTE_ARGS

printf '%s\n' "${files[@]}" | xargs -r -P "$JOBS" -I{} bash -c 'render_one "$@"' _ {}

for f in "${files[@]}"; do
  safe="$(safe_name "$f")"
  before="$OUT/$safe.before.png"
  after="$OUT/$safe.after.png"
  echo "**\`$f\`**" >> "$COMMENT"
  if [ -s "$before" ] && [ -s "$after" ]; then
    {
      echo "| before | after |"
      echo "|---|---|"
      echo "| ![before](%%IMGBASE%%/$safe.before.png) | ![after](%%IMGBASE%%/$safe.after.png) |"
    } >> "$COMMENT"
  elif [ -s "$after" ]; then
    echo "_new file_" >> "$COMMENT"
    echo "![render](%%IMGBASE%%/$safe.after.png)" >> "$COMMENT"
  else
    echo "⚠️ render failed" >> "$COMMENT"
  fi
  echo >> "$COMMENT"
done

if [ "$truncated" -gt 0 ]; then
  echo "_…and $truncated more changed model file(s) not shown._" >> "$COMMENT"
fi
echo "rendered ${#files[@]} file(s)"
