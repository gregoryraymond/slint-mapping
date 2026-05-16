//! [`TileSource`] — the adapter trait every map data source implements.
//!
//! The framework only ever asks a source for a single tile by
//! `(x, y, z)`; it doesn't care whether the source reads PNG files
//! from disk, decodes vector MVT, or hits a remote HTTP endpoint with
//! a cache. Implementations are responsible for their own internal
//! threading / caching; the call here is synchronous and is expected
//! to return quickly (`None` is a perfectly fine answer for "not
//! ready yet" — the controller will ask again on the next refresh).

/// One tile request. Coordinates are tile-space integers at zoom
/// level `z`; see [`crate::projection`] for the math.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TileKey {
    pub x: u32,
    pub y: u32,
    pub z: u8,
}

/// An on-disk or in-memory tile source. Adapters implement this.
pub trait TileSource: Send + Sync {
    /// Return the tile at `(x, y, z)`, or `None` if it isn't available
    /// (out of bounds, missing from disk, not yet downloaded, …).
    /// Implementations must not block the caller for long — kick off
    /// any expensive fetch on a background task and return `None` until
    /// the result lands in your cache.
    fn tile(&self, key: TileKey) -> Option<slint::Image>;

    /// Tile edge length in pixels. Default 256 matches every common
    /// slippy-map source; vector / retina sources may emit 512.
    fn tile_size(&self) -> u32 {
        256
    }

    /// Inclusive maximum zoom level this source serves. Used by the
    /// controller to clamp user-initiated zoom. Default is "any zoom
    /// up to Web Mercator's practical ceiling".
    fn max_zoom(&self) -> u8 {
        22
    }

    /// Inclusive minimum zoom level. Default 0 (whole world in one tile).
    fn min_zoom(&self) -> u8 {
        0
    }
}

// Blanket impl so a `Box<dyn TileSource>` is itself a `TileSource`,
// letting consumers swap sources at runtime without writing a shim.
impl<T: TileSource + ?Sized> TileSource for Box<T> {
    fn tile(&self, key: TileKey) -> Option<slint::Image> {
        (**self).tile(key)
    }
    fn tile_size(&self) -> u32 {
        (**self).tile_size()
    }
    fn min_zoom(&self) -> u8 {
        (**self).min_zoom()
    }
    fn max_zoom(&self) -> u8 {
        (**self).max_zoom()
    }
}
