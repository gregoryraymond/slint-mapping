//! Bulk cache warming — pre-download every tile covering a
//! geographic bounding box across a range of zoom levels, store
//! them in a [`TileCache`] so the runtime tile source serves them
//! offline.
//!
//! This is the synchronous, blocking, progress-reporting counterpart
//! to [`crate::sources::OsmTileSource`]: use it from a CLI / setup
//! script when you want a guaranteed-complete bundle; use the source
//! at runtime when you want lazy fetching.

use crate::cache::TileCache;
use crate::projection::lonlat_to_tile;
use crate::source::TileKey;
use crate::sources::osm::{fetch_bytes, OSM_TILE_URL};
use crate::sources::util::format_url;

/// Maximum zoom this function will fetch — guards against
/// accidentally downloading hundreds of thousands of tiles. Override
/// (carefully) at the call site if you've negotiated with the tile
/// provider; the default mirrors the OSM Foundation policy.
pub const MAX_BULK_FETCH_ZOOM: u8 = 16;

/// Inclusive (x_min, x_max, y_min, y_max) tile range covering the
/// geographic bbox at zoom `z`.
pub fn tile_range_for_bbox(
    lon_min: f64,
    lat_min: f64,
    lon_max: f64,
    lat_max: f64,
    zoom: u8,
) -> (u32, u32, u32, u32) {
    let z = zoom as f64;
    let (x0, y_at_min_lat) = lonlat_to_tile(lon_min, lat_min, z);
    let (x1, y_at_max_lat) = lonlat_to_tile(lon_max, lat_max, z);
    let max_tile = (1u64 << zoom).saturating_sub(1) as u32;
    let xs = (x0.floor() as i64, x1.floor() as i64);
    let ys = (y_at_min_lat.floor() as i64, y_at_max_lat.floor() as i64);
    let (x_min, x_max) = (xs.0.min(xs.1).max(0) as u32, xs.0.max(xs.1).max(0) as u32);
    let (y_min, y_max) = (ys.0.min(ys.1).max(0) as u32, ys.0.max(ys.1).max(0) as u32);
    (x_min.min(max_tile), x_max.min(max_tile), y_min.min(max_tile), y_max.min(max_tile))
}

/// Enumerate every tile key required to cover the bbox across the
/// inclusive zoom range. Returned in (z asc, x asc, y asc) order.
pub fn keys_for_region(
    lon_min: f64,
    lat_min: f64,
    lon_max: f64,
    lat_max: f64,
    zoom_min: u8,
    zoom_max: u8,
) -> Vec<TileKey> {
    let mut out = Vec::new();
    for z in zoom_min..=zoom_max {
        let (x_min, x_max, y_min, y_max) = tile_range_for_bbox(lon_min, lat_min, lon_max, lat_max, z);
        for x in x_min..=x_max {
            for y in y_min..=y_max {
                out.push(TileKey { x, y, z });
            }
        }
    }
    out
}

/// Outcome of a single tile fetch — handed to the progress callback.
pub enum FetchOutcome {
    /// Tile was already in the cache; no network request issued.
    Cached,
    /// Tile was downloaded + written to the cache.
    Fetched { bytes: usize },
    /// Fetch failed; tile not in cache.
    Failed(String),
}

/// Configuration for [`region`].
pub struct RegionConfig {
    /// URL template with `{z}`, `{x}`, `{y}` placeholders.
    pub url_template: String,
    /// HTTP User-Agent — pick something descriptive for the provider's
    /// abuse-detection logs.
    pub user_agent: String,
    /// Sleep between requests, milliseconds. 300ms keeps you within
    /// OSM's "polite" limits.
    pub sleep_ms: u64,
    /// Refuse to fetch past this zoom; bumping it is up to you.
    pub max_zoom: u8,
}

impl Default for RegionConfig {
    fn default() -> Self {
        Self {
            url_template: OSM_TILE_URL.to_string(),
            user_agent: "slint-mapping/0.1 (https://github.com/slint-rs/slint-mapping)".to_string(),
            sleep_ms: 300,
            max_zoom: MAX_BULK_FETCH_ZOOM,
        }
    }
}

/// Pre-fetch every tile that covers the bbox + zoom range, writing
/// them through `cache`. Blocks the caller; for non-blocking
/// prefetch, spawn this on a worker thread.
///
/// `progress` is called once per tile with the (1-based) index, the
/// total, the key, and the outcome — log it however you like.
///
/// Returns the count of tiles processed (cached + fetched + failed).
#[allow(clippy::too_many_arguments)]
pub fn region(
    cache: &dyn TileCache,
    lon_min: f64,
    lat_min: f64,
    lon_max: f64,
    lat_max: f64,
    zoom_min: u8,
    zoom_max: u8,
    config: &RegionConfig,
    mut progress: impl FnMut(usize, usize, TileKey, &FetchOutcome),
) -> Result<usize, String> {
    if zoom_max > config.max_zoom {
        return Err(format!(
            "zoom_max {zoom_max} exceeds the configured cap ({}); raise RegionConfig::max_zoom \
             if you've cleared bulk download with the provider",
            config.max_zoom
        ));
    }
    let keys = keys_for_region(lon_min, lat_min, lon_max, lat_max, zoom_min, zoom_max);
    let total = keys.len();
    let sleep = std::time::Duration::from_millis(config.sleep_ms);

    for (i, key) in keys.iter().enumerate() {
        let outcome = if cache.contains(*key) {
            FetchOutcome::Cached
        } else {
            let url = format_url(&config.url_template, *key);
            match fetch_bytes(&url, &config.user_agent) {
                Ok(bytes) => {
                    let len = bytes.len();
                    match cache.put(*key, &bytes) {
                        Ok(()) => FetchOutcome::Fetched { bytes: len },
                        Err(e) => FetchOutcome::Failed(format!("cache put: {e}")),
                    }
                }
                Err(e) => FetchOutcome::Failed(e),
            }
        };
        progress(i + 1, total, *key, &outcome);
        if matches!(outcome, FetchOutcome::Fetched { .. }) {
            std::thread::sleep(sleep);
        }
    }
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_for_region_world_z2_is_16_tiles() {
        let keys = keys_for_region(-180.0, -85.0, 180.0, 85.0, 2, 2);
        assert_eq!(keys.len(), 16);
    }

    #[test]
    fn keys_for_region_single_zoom_london() {
        // Greater London at z=12 — should be ~9×9 ≈ 81 tiles, give
        // or take a tile depending on bbox alignment.
        let keys = keys_for_region(-0.5, 51.25, 0.3, 51.75, 12, 12);
        assert!(keys.len() > 50 && keys.len() < 150, "got {} tiles", keys.len());
        assert!(keys.iter().all(|k| k.z == 12));
    }

    #[test]
    fn keys_for_region_spans_zoom_range() {
        let keys = keys_for_region(-1.0, 51.0, 1.0, 52.0, 4, 6);
        let zooms: std::collections::HashSet<u8> = keys.iter().map(|k| k.z).collect();
        assert_eq!(zooms, [4u8, 5, 6].into_iter().collect());
    }
}
