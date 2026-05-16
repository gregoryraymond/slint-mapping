#!/usr/bin/env python3
"""Download OSM raster tiles covering a geographic bounding box across a
range of zoom levels into a slippy-map directory layout that
`FileTileSource` can read.

Usage:
    download-osm-region.py [--dest DIR] [--zoom Z_MIN-Z_MAX]
                           LON_MIN LAT_MIN LON_MAX LAT_MAX

Example (Greater London, zoom 4-12):
    python scripts/download-osm-region.py \\
        --dest sample-tiles --zoom 4-12 \\
        -0.5 51.25 0.3 51.75

Honours the OSM Foundation tile policy: sets a descriptive User-Agent,
sleeps between fetches, refuses to fetch beyond zoom 16, displays the
total before starting so you can cancel.
"""
import argparse, math, os, sys, time, urllib.request

UA = "slint-mapping-demo/0.1 (https://github.com/slint-rs/slint-mapping)"
BASE = "https://tile.openstreetmap.org"
SLEEP = 0.3
MAX_ALLOWED_ZOOM = 16  # OSMF: never bulk-download past this


def deg2tile(lon, lat, z):
    """(longitude, latitude, zoom) -> integer (x, y) tile indices."""
    n = 2 ** z
    x = int((lon + 180.0) / 360.0 * n)
    lat_rad = math.radians(max(-85.0511, min(85.0511, lat)))
    y = int((1.0 - math.asinh(math.tan(lat_rad)) / math.pi) / 2.0 * n)
    return x, y


def tile_ranges(lon_min, lat_min, lon_max, lat_max, z):
    """Inclusive (x_min, x_max, y_min, y_max) tile range covering the box at z."""
    x0, y1 = deg2tile(lon_min, lat_min, z)  # SW: min x, max y
    x1, y0 = deg2tile(lon_max, lat_max, z)  # NE: max x, min y
    return min(x0, x1), max(x0, x1), min(y0, y1), max(y0, y1)


def fetch(url, dest):
    req = urllib.request.Request(url, headers={"User-Agent": UA})
    with urllib.request.urlopen(req, timeout=30) as r, open(dest, "wb") as f:
        f.write(r.read())


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("lon_min", type=float)
    ap.add_argument("lat_min", type=float)
    ap.add_argument("lon_max", type=float)
    ap.add_argument("lat_max", type=float)
    ap.add_argument("--dest", default="sample-tiles", help="output dir")
    ap.add_argument("--zoom", default="0-12", help="zoom range, e.g. 4-12")
    args = ap.parse_args()

    z_min, z_max = (int(z) for z in args.zoom.split("-"))
    if z_max > MAX_ALLOWED_ZOOM:
        sys.exit(f"refusing zoom > {MAX_ALLOWED_ZOOM} (OSMF policy)")

    # Plan the work.
    plan = []
    total_new = total_cached = 0
    for z in range(z_min, z_max + 1):
        x0, x1, y0, y1 = tile_ranges(args.lon_min, args.lat_min, args.lon_max, args.lat_max, z)
        for x in range(x0, x1 + 1):
            for y in range(y0, y1 + 1):
                path = os.path.join(args.dest, str(z), str(x), f"{y}.png")
                if os.path.exists(path):
                    total_cached += 1
                else:
                    plan.append((z, x, y, path))
                    total_new += 1

    print(f"bbox: lon [{args.lon_min}, {args.lon_max}]  lat [{args.lat_min}, {args.lat_max}]")
    print(f"zoom: {z_min}-{z_max}")
    print(f"tiles: {total_new} to fetch, {total_cached} already cached")
    print(f"estimated time: {total_new * SLEEP:.1f}s ({total_new} * {SLEEP}s sleep)")
    if not plan:
        print("nothing to do.")
        return
    print()

    for i, (z, x, y, path) in enumerate(plan, 1):
        print(f"\r[{i:4d}/{total_new:4d}] {z}/{x}/{y}      ", end="", flush=True)
        os.makedirs(os.path.dirname(path), exist_ok=True)
        try:
            fetch(f"{BASE}/{z}/{x}/{y}.png", path)
        except Exception as e:
            print(f"\n  FAILED {z}/{x}/{y}: {e}")
            continue
        time.sleep(SLEEP)

    print()
    print(f"\nDone. {total_new} tile(s) written under {args.dest}/")
    print("Attribution: tile imagery (c) OpenStreetMap contributors, ODbL.")


if __name__ == "__main__":
    main()
