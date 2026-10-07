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
V=(-c:v libx264 -preset veryslow -b:v 1680k -maxrate 6000k -bufsize 12000k -x264-params aq-mode=3
   -pix_fmt yuv420p -color_primaries bt709 -color_trc bt709 -colorspace bt709)
ffmpeg -nostdin -y -loglevel error -framerate 30 -i frames/%04d.png "${V[@]}" -pass 1 -passlogfile x264 -an -f mp4 /dev/null
ffmpeg -nostdin -y -loglevel error -framerate 30 -i frames/%04d.png -i music.wav "${V[@]}" -pass 2 -passlogfile x264 \
  -c:a aac -b:a 192k -shortest -movflags +faststart maquette-40s.mp4
rm -f x264*.log x264*.mbtree
echo "→ maquette-40s.mp4"
