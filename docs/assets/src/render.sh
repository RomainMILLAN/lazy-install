#!/usr/bin/env bash
# Rasterises the HTML sources in this directory to the PNGs in docs/assets/,
# and the SVG marks to their PNG sizes.
#
# The wordmark is rasterised rather than shipped as SVG text on purpose: an SVG
# with a `font-family` renders in whatever font the viewer has. The marks carry
# no text and stay vector.
#
# Needs Chrome/Chromium, ImageMagick, and network access for Google Fonts.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
out="$(dirname "$here")"

chrome=""
for candidate in google-chrome chromium chromium-browser; do
  if command -v "$candidate" >/dev/null 2>&1; then chrome="$candidate"; break; fi
done
[ -n "$chrome" ] || { echo "no chrome/chromium on PATH" >&2; exit 1; }

render() {
  local name="$1" width="$2" height="$3"
  "$chrome" --headless --disable-gpu --hide-scrollbars --force-device-scale-factor=2 \
    --virtual-time-budget=6000 --window-size="${width},${height}" \
    --screenshot="${out}/${name}.png" "file://${here}/${name}.html" >/dev/null 2>&1
  echo "rendered ${name}.png"
}

render banner-dark 1280 320
render banner-light 1280 320
render social-preview 1280 640

# Terminal captures. Regenerate their HTML first: cargo run --example screenshots
render screenshot-list-dark 1080 442
render screenshot-list-light 1080 442
render screenshot-update-dark 1080 442
render screenshot-update-light 1080 442

# Marks: transparent mark at 256, filled app icon at 512 and 128.
magick -background none -density 600 "${out}/logo-mark.svg" -resize 256x256 "${out}/logo-mark.png"
magick -background none -density 600 "${out}/logo-mark-light.svg" -resize 256x256 "${out}/logo-mark-light.png"
magick -background none -density 600 "${out}/icon.svg" -resize 512x512 "${out}/icon-512.png"
magick -background none -density 600 "${out}/icon.svg" -resize 128x128 "${out}/icon-128.png"
echo "rendered marks"
