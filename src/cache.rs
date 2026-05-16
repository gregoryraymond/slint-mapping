//! [`TileCache`] — abstract local storage for tiles.
//!
//! Every "fetching" tile source ([`crate::sources::OsmTileSource`] is
//! the only one shipped today) writes through a `TileCache` rather
//! than dealing with the filesystem / SQLite / whatever directly.
//! That keeps the storage backend swappable: an embedded app can
//! ship [`FileTileCache`] writing under the OS cache dir; a desktop
//! app might prefer a single-file format (MBTiles, PMTiles); a test
//! can use a `Vec` in memory.
//!
//! `FileTileCache` is the one concrete implementation right now and
//! is what backs the bundled `sample-tiles/` directory.

use crate::source::TileKey;
use std::path::{Path, PathBuf};

/// Local storage for downloaded tiles. Implementations decide whether
/// the backing is a slippy-map directory tree, an MBTiles SQLite db,
/// a single PMTiles archive, or an in-memory `HashMap` for tests.
pub trait TileCache: Send + Sync {
    /// Look up the cached tile, decoded as a `slint::Image`. Returns
    /// `None` if the tile isn't cached (or if its bytes failed to
    /// decode). Implementations should be cheap on the miss path.
    fn get(&self, key: TileKey) -> Option<slint::Image>;

    /// Store raw bytes (typically a PNG payload exactly as the source
    /// served it — the cache is encoding-agnostic). Errors here are
    /// returned but a fetching source will typically just log and
    /// keep going.
    fn put(&self, key: TileKey, bytes: &[u8]) -> Result<(), CacheError>;

    /// Quick existence check that avoids the decode cost of `get`.
    /// Default implementation falls back to `get(key).is_some()` —
    /// override on backends where existence is cheaper to test.
    fn contains(&self, key: TileKey) -> bool {
        self.get(key).is_some()
    }
}

/// Error returned by [`TileCache::put`].
#[derive(Debug)]
pub enum CacheError {
    Io(std::io::Error),
    Other(String),
}

impl std::fmt::Display for CacheError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CacheError::Io(e) => write!(f, "tile cache I/O error: {e}"),
            CacheError::Other(s) => write!(f, "tile cache error: {s}"),
        }
    }
}

impl std::error::Error for CacheError {}

impl From<std::io::Error> for CacheError {
    fn from(e: std::io::Error) -> Self {
        CacheError::Io(e)
    }
}

/// Stores tiles in a slippy-map directory tree on disk
/// (`{root}/{z}/{x}/{y}.{ext}`). Compatible with every common tile
/// bundle layout (the OSM standard, MapTiler exports, Mapbox tile
/// downloads). The bundled `sample-tiles/` is one of these.
pub struct FileTileCache {
    root: PathBuf,
    extension: String,
}

impl FileTileCache {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            extension: "png".to_string(),
        }
    }

    /// Override the tile file extension (default `"png"`).
    pub fn with_extension(mut self, ext: impl Into<String>) -> Self {
        self.extension = ext.into();
        self
    }

    /// Absolute on-disk path for the given tile.
    pub fn path_for(&self, key: TileKey) -> PathBuf {
        self.root
            .join(key.z.to_string())
            .join(key.x.to_string())
            .join(format!("{}.{}", key.y, self.extension))
    }

    /// Filesystem root this cache writes under.
    pub fn root(&self) -> &Path {
        &self.root
    }
}

impl TileCache for FileTileCache {
    fn get(&self, key: TileKey) -> Option<slint::Image> {
        let path = self.path_for(key);
        if !path.exists() {
            return None;
        }
        slint::Image::load_from_path(&path).ok()
    }

    fn put(&self, key: TileKey, bytes: &[u8]) -> Result<(), CacheError> {
        let path = self.path_for(key);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, bytes)?;
        Ok(())
    }

    fn contains(&self, key: TileKey) -> bool {
        self.path_for(key).exists()
    }
}

/// Read-through composite cache. `get` / `contains` try each layer
/// in order until one hits; `put` only writes to the first layer
/// (which must be writable). Use to overlay a writable user cache
/// over a read-only bundled cache: the bundled tiles serve instantly,
/// new fetches accumulate in the user cache without polluting the
/// bundle.
pub struct LayeredTileCache {
    /// First layer is the writable target for `put`; subsequent
    /// layers are read-only fallbacks for `get` / `contains`.
    layers: Vec<Box<dyn TileCache>>,
}

impl LayeredTileCache {
    /// Build a layered cache. `writable` receives all puts; `fallbacks`
    /// are tried in order on read misses.
    pub fn new(writable: Box<dyn TileCache>, fallbacks: Vec<Box<dyn TileCache>>) -> Self {
        let mut layers = Vec::with_capacity(1 + fallbacks.len());
        layers.push(writable);
        layers.extend(fallbacks);
        Self { layers }
    }
}

impl TileCache for LayeredTileCache {
    fn get(&self, key: TileKey) -> Option<slint::Image> {
        self.layers.iter().find_map(|l| l.get(key))
    }
    fn put(&self, key: TileKey, bytes: &[u8]) -> Result<(), CacheError> {
        self.layers[0].put(key, bytes)
    }
    fn contains(&self, key: TileKey) -> bool {
        self.layers.iter().any(|l| l.contains(key))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_root(name: &str) -> PathBuf {
        // `process::id() + name + thread id` keeps tests in this
        // module from racing on each other's roots (cargo runs them in
        // parallel by default).
        let tid = format!("{:?}", std::thread::current().id());
        let p = std::env::temp_dir().join(format!(
            "slint-mapping-cache-test-{}-{}-{}",
            std::process::id(),
            name,
            tid.replace([' ', '(', ')'], "_"),
        ));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn put_then_get_returns_the_same_bytes_via_image() {
        let root = temp_root("put_then_get");
        let cache = FileTileCache::new(&root);
        let key = TileKey { x: 1, y: 2, z: 3 };
        let png = include_bytes!("../sample-tiles/0/0/0.png"); // any real PNG
        cache.put(key, png).unwrap();
        assert!(cache.contains(key));
        assert!(cache.get(key).is_some(), "round-trip should decode");
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn missing_key_returns_none() {
        let root = temp_root("missing");
        let cache = FileTileCache::new(&root);
        assert!(!cache.contains(TileKey { x: 9, y: 9, z: 9 }));
        assert!(cache.get(TileKey { x: 9, y: 9, z: 9 }).is_none());
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn put_creates_intermediate_directories() {
        let root = temp_root("intermediate_dirs");
        let cache = FileTileCache::new(&root);
        let key = TileKey { x: 1234, y: 5678, z: 12 };
        let png = include_bytes!("../sample-tiles/0/0/0.png");
        cache.put(key, png).unwrap();
        assert!(cache.path_for(key).exists());
        fs::remove_dir_all(&root).ok();
    }
}
