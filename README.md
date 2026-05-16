<a name="top"></a>

# slint-mapping

[![Live demo](https://img.shields.io/badge/demo-live-1d76db?style=flat-square)](https://gregoryraymond.github.io/slint-mapping/)
[![License](https://img.shields.io/badge/license-MIT_OR_Apache--2.0-blue?style=flat-square)](#-license)
[![Slint](https://img.shields.io/badge/Slint-1.x-2379f4?style=flat-square)](https://slint.dev)
[![Rust](https://img.shields.io/badge/rust-1.74%2B-orange?style=flat-square)](https://www.rust-lang.org)
[![Platforms](https://img.shields.io/badge/platforms-Linux%20%7C%20macOS%20%7C%20Windows%20%7C%20Android%20%7C%20Wasm-555?style=flat-square)](#)
[![Pages](https://github.com/gregoryraymond/slint-mapping/actions/workflows/pages.yml/badge.svg)](https://github.com/gregoryraymond/slint-mapping/actions/workflows/pages.yml)

A slippy-map widget for [Slint](https://slint.dev). Pan, scroll-zoom,
OSM tiles, markers, route polylines — written so a Slint app can drop
it in the way you'd drop in a `ListView`.

> 🌍 **[Try the live demo →](https://gregoryraymond.github.io/slint-mapping/)**
> It renders real OSM tiles in the browser, supports pan and
> cursor-anchored scroll-zoom, and shows the marker and polyline
> overlays working. The whole thing compiles to wasm.

## Table of contents

- [🗺️ About](#%EF%B8%8F-about)
- [✨ What works](#-what-works)
- [🚧 What's missing](#-whats-missing)
- [🚀 Quick start](#-quick-start)
- [🧱 How it's built](#-how-its-built)
- [📂 Layout](#-layout)
- [🧪 Cargo features](#-cargo-features)
- [🤝 Contributing](#-contributing)
- [📃 License](#-license)

## 🗺️ About

The library covers the map side of an app: tiles on the screen, the
right tiles for where the camera is pointed, gestures that move it
around, and overlays drawn on top. Slint handles everything else —
your search bar, your bottom sheet, your turn-by-turn list. The two
fit together because both use Slint's normal `for` loops and property
bindings; the map isn't a black box that breaks the rest of your
layout.

The architecture is straightforward:

- A **`TileSource`** trait says "give me the PNG for `(x, y, z)`".
  Implementations exist for an offline directory, the live OSM HTTP
  CDN, and a browser version using `gloo-net`.
- A **`TileCache`** trait sits in front, layered memory-then-disk so
  warm tiles paint instantly.
- A **`MapController`** owns the camera and wires user gestures to
  pan/zoom math.
- A **`Router`** trait with one implementation against OSRM-compatible
  servers, for turn-by-turn directions.

Each trait is small enough to implement in an afternoon if you want
to plug in a different backend (Mapbox, Stadia, a self-hosted
Valhalla).

## ✨ What works

Pan and scroll-zoom, with the zoom anchored on the cursor: the point
under the mouse stays put when you zoom in. Fractional zoom levels
(10.5, 11.25, etc.) render at the right scale instead of snapping to
the nearest integer.

OSM tiles over HTTP with a layered cache — in memory in front of a
plain-files disk cache. Two fetch workers pull in parallel and a
third thread handles PNG decode off the network path. Tiles that 404
or fail to decode get remembered for the session so a single bad key
doesn't loop forever.

Marker overlays (icons or filled circles) and polyline overlays
(routes, GPS traces, drawn paths). Both live inside `Layer`s that you
can show or hide as a unit, the way you would in a GIS app.

A `Router` trait with one implementation, `OsrmRouter`, that talks to
any OSRM-compatible server. The OSRM project's public demo server
works for prototyping; self-host for real traffic.

A small CLI, `slint-mapping-prefetch`, that warms the on-disk cache
for a bounding box across a zoom range so the map works offline once
you've taken the app somewhere with no signal.

## 🚧 What's missing

Pinch-zoom waits on Slint exposing multi-pointer touch events.
Polygon fills need upstream improvements to Slint's `Path`; only
stroked polylines work today. Vector tiles (MVT) would be another
`TileSource` impl and nobody has asked for them yet. On-device
routing graphs are out — `OsrmRouter` talks HTTP, so bring your own
server.

> [!NOTE]
> If any of these are blockers for what you're building, open an
> issue. The order I tackle them is heavily influenced by whether
> someone actually wants them.

## 🚀 Quick start

The sample tile bundle shipped with the crate covers the whole world
at zoom 0–3, which is enough to confirm the pan/zoom math works
without touching the network:

```rust
use slint::ComponentHandle;
use slint_mapping::{sources::FileTileSource, MapController, MapView, SAMPLE_TILES_DIR};

fn main() -> Result<(), slint::PlatformError> {
    let map = MapView::new()?;
    let _ctl = MapController::new(&map, FileTileSource::new(SAMPLE_TILES_DIR));
    map.show()?;
    slint::run_event_loop()
}
```

Swap `FileTileSource` for `OsmTileSource` (behind the `http` feature)
when you want live tiles, and set a real `User-Agent` — OSM's tile
CDN quietly rejects anything that doesn't identify itself:

```rust
use slint_mapping::{
    cache::{FileTileCache, LayeredTileCache},
    sources::OsmTileSource,
};
use std::sync::Arc;

let cache: Arc<dyn slint_mapping::TileCache> = Arc::new(LayeredTileCache::new(
    Box::new(FileTileCache::new("~/.cache/my-app/tiles")),
    vec![],
));
let source = OsmTileSource::new(cache)
    .with_user_agent("my-app/0.1 (+https://example.com)");
```

If you want the map inside a richer Slint scene rather than as the
whole window — a search bar pinned to the top, say, or a bottom
sheet sliding up over the map — use `MapEmbed` instead of `MapView`.
Same property surface, no Window of its own.

## 🧱 How it's built

Web Mercator is the only projection. Every common slippy-map provider
uses it; the few outliers (national grids, polar projections) can
ship as their own crate the day someone needs one.

There is no async runtime in the dependency tree. The library does
its own threading on `std::thread` and posts results back to the UI
through `slint::invoke_from_event_loop`. The two traits an app sees,
`TileSource` and `Router`, are both synchronous and adapters own
their own workers. The point is to drop into any Slint app without
forcing tokio or async-std into someone else's build.

Geometry uses bespoke types: `(f64, f64)` for points, `TileKey` for
tile addresses, Slint structs for overlay shapes. `geo-types` was on
the table but it mostly adds a dep for what amounts to a tuple. If
spatial joins or buffer ops start showing up, that calculation
changes.

The disk cache writes plain PNGs in the standard `{z}/{x}/{y}.png`
layout. You can seed it by rsyncing an existing OSM mirror, and
other slippy-map tools (JOSM, QGIS, mb-util) read what we write.
`sled` and `redb` were considered and neither earned the dep.

The wasm side is a separate story. `ureq` doesn't compile to
`wasm32-unknown-unknown` (no sockets in the browser sandbox), and
`std::thread::spawn` panics without cross-origin-isolation headers
that GitHub Pages won't send. So there's a parallel
`WasmOsmTileSource` that uses `gloo-net` and `spawn_local`. Same
trait, different shape underneath.

## 📂 Layout

```
slint-mapping/
├── src/
│   ├── projection.rs    Web Mercator
│   ├── viewport.rs      visible tiles, lon/lat ↔ viewport px, anchor math
│   ├── camera.rs        pan + zoom_anchored
│   ├── controller.rs    wires MapView to a TileSource
│   ├── source.rs        TileSource trait + TileKey
│   ├── sources/         FileTileSource, OsmTileSource, WasmOsmTileSource
│   ├── cache.rs         TileCache trait, FileTileCache, LayeredTileCache
│   ├── routing.rs       Router trait + Route / Maneuver
│   ├── routers/         OsrmRouter
│   ├── prefetch.rs      bulk-fetch a bbox across a zoom range
│   └── bin/prefetch.rs  the slint-mapping-prefetch CLI
├── ui/
│   └── map.slint        MapEmbed, MapView, MapPanel,
│                        Tile / Marker / Polyline / Layer structs
├── wasm-demo/           browser demo (workspace member)
└── .github/workflows/
    └── pages.yml        builds and publishes the demo on push to main
```

## 🧪 Cargo features

| Feature   | What it turns on                         | Extra deps                         |
| :-------- | :--------------------------------------- | :--------------------------------- |
| (default) | Offline tile sources, rendering          | none                               |
| `http`    | `OsmTileSource`, `prefetch::region`      | `ureq`, `image`                    |
| `routing` | `OsrmRouter`, routing types              | `ureq`, `serde`, `serde_json`      |
| `wasm`    | `WasmOsmTileSource` (wasm32 target only) | `gloo-net`, `wasm-bindgen`, others |
| `viewer`  | the `viewer` and `map_page` examples     | Slint's desktop backend + Skia     |

## 🤝 Contributing

Issues and PRs welcome. If you've got an opinion on what should land
next — pinch-zoom, vector tiles, polygon fills, an offline router,
something I haven't thought of — say so on the issue tracker. There's
no roadmap document; the next thing built is usually the next thing
someone actually needs.

If you're poking at the internals, the `tests/` directory is the
honest documentation: every non-obvious bit of math has a test that
explains why it's the way it is.

## 📃 License

Dual-licensed under either [MIT](LICENSE-MIT) or
[Apache-2.0](LICENSE-APACHE) at your option.

---

<sub>[↑ Back to top](#top)</sub>
