#!/usr/bin/env bash
# Usage: scripts/pwa-icons.sh
# Cuts the web app's install icons in ui/icons from icons/icon.png. Needs macOS `sips`.
set -euo pipefail
cd "$(dirname "$0")/.."

src=icons/icon.png
out=ui/icons
mkdir -p "$out"

sips -z 192 192 "$src" --out "$out/icon-192.png" >/dev/null
cp "$src" "$out/icon-512.png"

# A maskable icon must be opaque with its artwork inside the central 80% circle, so keep the
# glyphs on the plate's own #0f1115 interior, drop the rounded plate and fill the rest with it.
sips -c 300 300 --cropOffset 106 106 "$src" --out "$out/maskable-512.png" >/dev/null
sips -p 512 512 --padColor 0F1115 "$out/maskable-512.png" >/dev/null
sips -z 180 180 "$out/maskable-512.png" --out "$out/apple-touch-180.png" >/dev/null
