#!/usr/bin/env bash
#
# Download a small sample of raster tiles into a slippy-map directory
# layout (`{root}/{z}/{x}/{y}.png`) the FileTileSource can read.
#
# Default source is the OpenStreetMap standard tile layer
# (https://tile.openstreetmap.org). Their tile-usage policy
# (https://operations.osmfoundation.org/policies/tiles/) requires:
#
#   - A descriptive User-Agent identifying your app + contact URL.
#   - No bulk downloads (be polite — this script sleeps between fetches).
#   - Attribution on display ("© OpenStreetMap contributors").
#
# For larger-scale work, switch to a paid provider (MapTiler, Stadia,
# Mapbox) or self-host via OpenMapTiles / tilemaker — see README.
#
# Usage:
#   scripts/download-osm-tiles.sh [dest-dir] [max-zoom]
#
# Defaults to writing 85 tiles (zoom 0-3, full world) into `sample-tiles/`.
set -euo pipefail

DEST="${1:-sample-tiles}"
MAX_ZOOM="${2:-3}"
UA="${TILE_USER_AGENT:-slint-mapping-demo/0.1 (https://github.com/slint-rs/slint-mapping)}"
BASE_URL="${TILE_BASE_URL:-https://tile.openstreetmap.org}"
SLEEP_SECS="${TILE_SLEEP_SECS:-0.3}"

mkdir -p "$DEST"

total=0
for z in $(seq 0 "$MAX_ZOOM"); do
    n=$((1 << z))
    total=$((total + n * n))
done

echo "Downloading $total tiles from $BASE_URL (zoom 0–$MAX_ZOOM) into $DEST/"
echo "User-Agent: $UA"
echo

i=0
for z in $(seq 0 "$MAX_ZOOM"); do
    n=$((1 << z))
    for x in $(seq 0 $((n - 1))); do
        mkdir -p "$DEST/$z/$x"
        for y in $(seq 0 $((n - 1))); do
            i=$((i + 1))
            f="$DEST/$z/$x/$y.png"
            if [ -f "$f" ]; then
                printf '\r[%4d/%4d] cached: %s        ' "$i" "$total" "$f"
                continue
            fi
            printf '\r[%4d/%4d] fetch:  %s        ' "$i" "$total" "$f"
            curl -fsSL \
                -H "User-Agent: $UA" \
                -o "$f" \
                "$BASE_URL/$z/$x/$y.png"
            sleep "$SLEEP_SECS"
        done
    done
done

echo
echo
echo "Done. $i tile(s) under $DEST/"
echo
echo "Attribution: tile imagery © OpenStreetMap contributors,"
echo "licensed under the Open Database License (https://www.openstreetmap.org/copyright)."
echo
echo "Test with: cargo run --example viewer --features viewer -- $DEST"
