#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
RENDER_DIR="$SCRIPT_DIR/rendered"
RENDER_EFFECTS_DIR="$RENDER_DIR/effects"

mkdir -p "$RENDER_DIR" "$RENDER_EFFECTS_DIR"

for diagram in "$SCRIPT_DIR"/*.mmd; do
  [ -e "$diagram" ] || continue
  base="$(basename "$diagram" .mmd)"
  mmdr -i "$diagram" -o "$RENDER_DIR/$base.png" -e png
done

for diagram in "$SCRIPT_DIR"/effects/*.mmd; do
  [ -e "$diagram" ] || continue
  base="$(basename "$diagram" .mmd)"
  mmdr -i "$diagram" -o "$RENDER_EFFECTS_DIR/$base.png" -e png
done
