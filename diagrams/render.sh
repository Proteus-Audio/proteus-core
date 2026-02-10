#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
RENDER_DIR="$SCRIPT_DIR/rendered"

mkdir -p "$RENDER_DIR"

for diagram in "$SCRIPT_DIR"/*.mmd; do
  [ -e "$diagram" ] || continue
  base="$(basename "$diagram" .mmd)"
  mmdr -i "$diagram" -o "$RENDER_DIR/$base.png" -e png
  mmdr -i "$diagram" -o "$RENDER_DIR/$base.svg" -e svg
done
