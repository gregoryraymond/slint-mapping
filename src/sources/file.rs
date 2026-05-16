//! [`FileTileSource`] — read raster tiles from a slippy-map directory tree
//! laid out on disk as `{root}/{z}/{x}/{y}.{ext}`.
//!
//! This is the standard "OSM tile cache" layout — any tile bundle you
//! pull from a slippy-map provider (Stamen, Stadia, an MBTiles export
//! flattened to PNGs, your own pre-rendered set) will already match
//! this shape, or be trivially `mv`-able into it.

use crate::source::{TileKey, TileSource};
use std::path::PathBuf;

/// Reads raster tiles from a slippy-map directory tree on disk.
///
/// ```ignore
/// use slint_mapping::sources::FileTileSource;
/// let src = FileTileSource::new("/data/tiles")
///     .with_extension("png")
///     .with_tile_size(256)
///     .with_zoom_range(0, 14);
/// ```
pub struct FileTileSource {
    root: PathBuf,
    extension: String,
    tile_size: u32,
    min_zoom: u8,
    max_zoom: u8,
}

impl FileTileSource {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            extension: "png".to_string(),
            tile_size: 256,
            min_zoom: 0,
            max_zoom: 22,
        }
    }

    /// Override the tile file extension (default `"png"`).
    pub fn with_extension(mut self, ext: impl Into<String>) -> Self {
        self.extension = ext.into();
        self
    }

    /// Override the tile edge length (default 256).
    pub fn with_tile_size(mut self, size: u32) -> Self {
        self.tile_size = size;
        self
    }

    /// Override the available zoom range. Tiles outside this range
    /// won't be requested by the controller, even if the user zooms
    /// past it.
    pub fn with_zoom_range(mut self, min: u8, max: u8) -> Self {
        self.min_zoom = min;
        self.max_zoom = max;
        self
    }
}

impl TileSource for FileTileSource {
    fn tile(&self, key: TileKey) -> Option<slint::Image> {
        // `{root}/{z}/{x}/{y}.{ext}`
        let path = self
            .root
            .join(key.z.to_string())
            .join(key.x.to_string())
            .join(format!("{}.{}", key.y, self.extension));
        slint::Image::load_from_path(&path).ok()
    }

    fn tile_size(&self) -> u32 {
        self.tile_size
    }

    fn min_zoom(&self) -> u8 {
        self.min_zoom
    }

    fn max_zoom(&self) -> u8 {
        self.max_zoom
    }
}
