#!/usr/bin/env bash
# Render the 3D model files changed on this branch, before (base) and after
# (head), into side-by-side images, and emit a Markdown comment body. Image
# URLs are left as the token (%%IMGBASE%%) for the caller to substitute once the
# PNGs are hosted. Local use: build the CLI, then
#   .github/scripts/render-diff.sh <base-ref> [out-dir]
set -euo pipefail

BASE_REF="${1:?usage: render-diff.sh <base-ref> [out-dir]}"
OUT="${2:-render-diff-out}"
BIN="${MAQUETTE_BIN:-target/release/maquette}"
MAX_FILES="${MAX_FILES:-12}"

ARGS=(--set width=460 --set height=360 --set azimuth=28 --set elevation=22)
if [ -n "${MAQUETTE_ARGS:-}" ]; then
  read -ra EXTRA <<< "$MAQUETTE_ARGS"
  ARGS+=("${EXTRA[@]}")
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

render() { "$BIN" "$1" -o "$2" "${ARGS[@]}" >/dev/null 2>&1; }

for f in "${files[@]}"; do
  ext="${f##*.}"
  safe="$(echo "$f" | tr '/. ' '___')"
  before="$OUT/$safe.before.png"
  after="$OUT/$safe.after.png"
  ok_before=0; ok_after=0

  render "$f" "$after" && ok_after=1 || true

  if git cat-file -e "$BASE_REF:$f" 2>/dev/null; then
    src="$OUT/$safe.beforesrc.$ext"
    git show "$BASE_REF:$f" > "$src"
    render "$src" "$before" && ok_before=1 || true
  fi

  echo "**\`$f\`**" >> "$COMMENT"
  if [ "$ok_before" = 1 ] && [ "$ok_after" = 1 ]; then
    {
      echo "| before | after |"
      echo "|---|---|"
      echo "| ![before](%%IMGBASE%%/$safe.before.png) | ![after](%%IMGBASE%%/$safe.after.png) |"
    } >> "$COMMENT"
  elif [ "$ok_after" = 1 ]; then
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
