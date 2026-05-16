//! Concrete [`TileSource`](crate::TileSource) implementations.
//!
//! Each adapter lives in its own submodule and is re-exported at this
//! module's root. Adding a new source = drop a file in this directory,
//! register it here.

mod file;
mod synthetic;
pub use file::FileTileSource;
pub use synthetic::SyntheticTileSource;

// Shared helpers for both the native and the wasm HTTP tile sources
// (URL templating + PNG decoding). Compiled in if either feature
// requests them, since both share the same logic. `pub(crate)` so
// sibling modules (osm, wasm_osm, prefetch) can import from it.
#[cfg(any(feature = "http", feature = "wasm"))]
pub(crate) mod util;

#[cfg(feature = "http")]
pub(crate) mod osm;
#[cfg(feature = "http")]
pub use osm::{OsmTileSource, OSM_TILE_URL};

// wasm_osm uses gloo-net + wasm_bindgen_futures which are themselves
// gated to `cfg(target_arch = "wasm32")` in Cargo.toml. Mirror that
// here so a native `cargo check` with the `wasm` feature on (e.g.
// from a workspace `cargo check --all-features`) doesn't try to
// compile the module against missing crates.
#[cfg(all(feature = "wasm", target_arch = "wasm32"))]
pub(crate) mod wasm_osm;
#[cfg(all(feature = "wasm", target_arch = "wasm32"))]
pub use wasm_osm::WasmOsmTileSource;
