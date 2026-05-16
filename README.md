# slint-mapping

A map framework for [Slint](https://slint.dev). Goal: ship a
production-quality `MapView` Slint component that a mobile or desktop
Slint app can drop in and get an interactive map (pan, zoom, markers,
overlays) with the same effort it takes to add a `ListView`.

## Status

Scaffold only. The skeleton compiles a placeholder `MapView` component
from `ui/map.slint`; the Rust side that actually fetches tiles,
renders them, and wires gesture input is yet to be written.

## Open design choices

Before serious code lands, we need to agree on:

1. **Tile source** — raster slippy-map tiles (OSM, Mapbox, Stadia) over
   HTTP, or offline vector tiles (MVT) parsed locally? Hybrid?
2. **Renderer** — pre-composite tiles to a single `slint::Image`
   (simplest, works with any Slint backend including software/Android),
   or push individual tile images through a `for` loop in Slint
   (more flexible but more `Image` items in the scene graph)?
3. **Projection** — Web Mercator only (matches every common tile
   provider), or pluggable?
4. **Coordinate / geometry types** — roll our own `LatLng` / `BBox` /
   `Marker` types, or depend on `geo-types`?
5. **Async runtime** — `tokio`, `async-std`, or runtime-agnostic via
   `slint::spawn_local` for tile fetches?
6. **Caching** — in-memory only, or on-disk persistent? If on-disk,
   which crate (`sled`, `redb`, plain files)?
7. **Scope of v0.1** — display + pan + zoom only? Or markers from the
   start? Or routing too?

## Layout

```
slint-mapping/
├── Cargo.toml
├── build.rs           # slint-build → ui/map.slint
├── src/lib.rs         # slint::include_modules!() + (eventually) tile fetcher,
│                      #  projection, gesture handlers, marker model
└── ui/
    └── map.slint      # MapView component
```

## License

MIT OR Apache-2.0.
