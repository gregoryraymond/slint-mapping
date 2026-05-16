# slint-mapping

[![Live demo](https://img.shields.io/badge/demo-live-blue?style=flat-square)](https://gregoryraymond.github.io/slint-mapping/)

A map framework for [Slint](https://slint.dev). Goal: ship a
production-quality `MapView` Slint component that a mobile or desktop
Slint app can drop in and get an interactive map (pan, zoom, markers,
overlays, routing) with the same effort it takes to add a `ListView`.

**[Try it in the browser →](https://gregoryraymond.github.io/slint-mapping/)**
The live demo loads real OSM tiles, supports pan + cursor-anchored
scroll-zoom, and shows the marker + polyline overlays in action. Built
from `wasm-demo/` and deployed by `.github/workflows/pages.yml` on
every push to `main`.

## Status

v0.x — usable. Working today:

- Pan + scroll-zoom, cursor-anchored, with fractional zoom levels
- Raster slippy-map tile rendering with per-tile loading placeholders
- Pluggable `TileSource`: `FileTileSource` (offline directory),
  `OsmTileSource` (HTTP, 2-worker fetch + 1-worker decode pipeline,
  request debouncing, failed-key memoization)
- Layered in-memory + on-disk tile cache (`LayeredTileCache`)
- `Layer` overlays: markers (icons or pin fallback) and polylines
  (route lines, GPS traces)
- `Router` trait + `OsrmRouter` for turn-by-turn directions against
  any OSRM-compatible HTTP server
- `prefetch::region` + a `slint-mapping-prefetch` CLI for bulk-fetching
  tile bboxes before going offline

Not yet:

- Pinch-zoom (blocked on Slint exposing multi-pointer touch events)
- Polygon fills
- Vector tile (MVT) rendering
- Bundled offline routing graphs (BYO self-hosted OSRM / Valhalla)

## Design decisions

1. **Tile source** — raster slippy-map only. Anything serving 256 px
   PNGs at `/{z}/{x}/{y}.png` (OSM, Stadia, self-hosted, etc.) works
   via `OsmTileSource`. MVT vector tiles aren't ruled out but aren't
   on the roadmap — they're a `TileSource` trait impl away if someone
   needs them.
2. **Renderer** — individual tile images pushed through a Slint
   `for` loop. Each tile is a `Rectangle` (loading placeholder
   background, animated dot) wrapping an `Image`, positioned
   absolutely in the map's local coordinate space. Trade-off: more
   scene-graph items than pre-compositing to one `slint::Image`, but
   lets Slint's renderer batch + cull, supports fractional zoom by
   scaling each tile's `size` to `256 × 2^frac` without a custom
   shader, and means tiles paint as soon as they decode — no
   "composite buffer" to rebuild on every change.
3. **Projection** — Web Mercator only. Matches every common slippy-map
   tile provider. The few outliers (Esri Antarctic, national grids)
   can ship as a separate crate or as a projection trait later;
   adding pluggability now would just be unused indirection.
4. **Coordinate / geometry types** — own types. `(lon, lat): (f64, f64)`
   for points, `TileKey { x, y, z }` for tile addresses, `Marker` /
   `Polyline` / `Layer` Slint structs for overlays. `geo-types` was
   considered but adds a dep for `(f64, f64)` and a heavier API than
   we use; revisit if/when we add spatial joins, buffer ops, or
   bbox arithmetic.
5. **Async runtime** — runtime-agnostic. No `tokio` / `async-std`
   dependency. Long-running work (HTTP fetch, PNG decode, routing)
   runs on `std::thread` workers owned by the concrete source/router;
   results dispatch back to the UI via `slint::invoke_from_event_loop`.
   `TileSource::tile()` and `Router::route()` traits are sync;
   implementations own their own threading. Lets the framework drop
   into any Slint app without dictating an executor.
6. **Caching** — layered. In-memory `HashMap<TileKey,
   SharedPixelBuffer>` in front of `FileTileCache` (plain files in
   slippy-map directory layout). `LayeredTileCache` composes them so
   the OSM source reads memory → disk → network in order, writes
   network results to both. `sled` / `redb` were considered but plain
   files round-trip with every other slippy-map tool and let users
   seed the cache by `rsync`-ing an existing OSM mirror.
7. **Scope of v0.1** — display + pan + scroll-zoom + markers +
   polylines + routing trait + OSRM impl + region prefetch.
   Pinch-zoom and polygon fills are explicitly out until Slint
   exposes the missing primitives (multi-pointer touch, filled
   `Path` with holes).

## Crate layout

```
slint-mapping/
├── Cargo.toml
├── build.rs                 # slint-build → ui/map.slint
├── src/
│   ├── lib.rs               # public surface + re-exports
│   ├── projection.rs        # Web Mercator: (lon,lat) ↔ tile space
│   ├── viewport.rs          # visible_tiles, lonlat ↔ viewport-px,
│   │                        #  cursor-anchor centre maths
│   ├── camera.rs            # pan + zoom_anchored helpers
│   ├── controller.rs        # MapController — wires a MapView to a TileSource
│   ├── source.rs            # TileSource trait + TileKey
│   ├── sources/
│   │   ├── file.rs          # FileTileSource (offline directory)
│   │   ├── synthetic.rs     # SyntheticTileSource (debug / tests)
│   │   └── osm.rs           # OsmTileSource (HTTP, gated by `http`)
│   ├── cache.rs             # TileCache trait, FileTileCache, LayeredTileCache
│   ├── routing.rs           # Router trait, Route, Maneuver, ManeuverKind
│   ├── routers/
│   │   └── osrm.rs          # OsrmRouter (gated by `routing`)
│   ├── prefetch.rs          # bulk-fetch a bbox at multiple zooms
│   └── bin/prefetch.rs      # slint-mapping-prefetch CLI
├── ui/
│   └── map.slint            # MapEmbed (Rectangle), MapView (Window),
│                            #  MapPanel (chrome wrapper),
│                            #  Tile / Marker / Polyline / Layer structs
├── wasm-demo/               # browser demo (member of the workspace)
│   ├── src/lib.rs           # #[wasm_bindgen(start)] entry, wires
│   │                        #  WasmOsmTileSource + camera + overlays
│   ├── ui/demo.slint
│   └── web/index.html       # static page served from /pkg
└── .github/workflows/
    └── pages.yml            # builds wasm-demo with wasm-pack and
                             #  publishes to GitHub Pages on push to main
```

## Cargo features

- **default** — tile rendering only (offline + synthetic sources).
- **`http`** — enables `OsmTileSource` + `prefetch::region`. Pulls
  `ureq` (TLS) and `image` (PNG-only).
- **`routing`** — enables `OsrmRouter`. Pulls `ureq` + `serde` +
  `serde_json`. Independent of `http` so a routing-only consumer
  doesn't drag in the PNG decoder.
- **`viewer`** — enables the `viewer` and `map_page` examples. Pulls
  Slint's default desktop backend + Skia renderer; off by default so
  Android consumers don't get `winit` forced on them.
- **`wasm`** — enables `WasmOsmTileSource` for `wasm32-unknown-unknown`
  targets. Uses `gloo-net` + `wasm_bindgen_futures::spawn_local`
  instead of `ureq` + `std::thread` (neither work in the browser
  sandbox). In-memory cache only — IndexedDB persistence is a
  future addition.

## License

MIT OR Apache-2.0.
