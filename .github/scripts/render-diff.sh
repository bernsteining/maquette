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

# Render settings are user-configurable: commit a render-config dict (the same
# keys the plugins accept — see render-config.schema.json) to this path and it
# is passed to every render via --config. MAQUETTE_ARGS (extra `--set k=v`)
# always wins on top. With neither, a sensible default framing is used.
CFG="${RENDER_DIFF_CONFIG:-.github/render-diff.json}"
if [ -f "$CFG" ]; then
  ARGS_STR="--config $CFG ${MAQUETTE_ARGS:-}"
else
  ARGS_STR="--set width=460 --set height=360 --set azimuth=28 --set elevation=22 ${MAQUETTE_ARGS:-}"
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

render_one() {
  local f="$1" ext safe before after src
  ext="${f##*.}"
  safe="$(echo "$f" | tr '/. ' '___')"
  before="$OUT/$safe.before.png"
  after="$OUT/$safe.after.png"
  read -ra A <<< "$ARGS_STR"
  "$BIN" "$f" -o "$after" "${A[@]}" >/dev/null 2>&1 || true
  if git cat-file -e "$BASE_REF:$f" 2>/dev/null; then
    src="$OUT/$safe.beforesrc.$ext"
    git show "$BASE_REF:$f" > "$src"
    "$BIN" "$src" -o "$before" "${A[@]}" >/dev/null 2>&1 || true
  fi
}
export -f render_one
export BIN OUT BASE_REF ARGS_STR

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
