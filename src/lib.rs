//! slint-mapping — a map framework for Slint.
//!
//! Minimal usage (offline raster tiles from a slippy-map directory):
//!
//! ```ignore
//! use slint::ComponentHandle;
//! use slint_mapping::{MapView, MapController, sources::FileTileSource};
//!
//! let map = MapView::new()?;
//! let _controller = MapController::new(
//!     &map,
//!     FileTileSource::new("/data/osm-tiles").with_extension("png"),
//! );
//! map.show()?;
//! slint::run_event_loop()?;
//! ```
//!
//! ## Adapters
//!
//! Tile sources are pluggable: implement [`TileSource`] for any
//! backing store (filesystem, MBTiles, in-memory generator, HTTP cache,
//! …). [`sources::FileTileSource`] is the only adapter that ships
//! today.
//!
//! ## Status
//!
//! v0.x. Pan + scroll-zoom work; pinch-zoom waits on Slint exposing
//! multi-pointer touch events. No marker / polyline overlays yet.

slint::include_modules!();

pub mod controller;
pub mod projection;
pub mod source;
pub mod sources;
pub mod viewport;

pub use controller::MapController;
pub use source::{TileKey, TileSource};
