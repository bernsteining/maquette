#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"
[ -d node_modules ] || npm install
node ../packages/maquette-js/scripts/copy-wasm.mjs
if [ ! -d seq ]; then
  node render_seq.mjs all
  for p in 0 1 2; do node render_seq.mjs tokyo "$p" 3 & done
  wait
fi
python3 music.py
rm -rf frames
python3 capture.py frames
ffmpeg -y -loglevel error -framerate 30 -i frames/%04d.png -i music.wav \
  -c:v libx264 -preset slow -crf 17 -pix_fmt yuv420p \
  -color_primaries bt709 -color_trc bt709 -colorspace bt709 \
  -c:a aac -b:a 192k -shortest -movflags +faststart maquette-27s.mp4
echo "→ maquette-27s.mp4"
