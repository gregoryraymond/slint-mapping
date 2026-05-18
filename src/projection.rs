//! Web Mercator projection — the one every slippy-map tile source uses.
//!
//! At zoom level `z` the world is `2^z × 2^z` tiles, each `256 × 256`
//! pixels by convention. `lon ∈ (-180, 180]` maps linearly to
//! `x ∈ [0, 2^z)`; `lat ∈ (-85.0511, 85.0511)` maps non-linearly to
//! `y ∈ [0, 2^z)` via the Mercator formula. We work in tile-space
//! floating-point coordinates (e.g. `tile_x = 4.7`) and convert to /
//! from integer tile indices + pixel offsets at the boundary.
//!
//! References: <https://wiki.openstreetmap.org/wiki/Slippy_map_tilenames>

/// Tile edge length in pixels. Every common slippy-map source uses 256.
/// Vector / retina sources may emit 512; we keep that as a per-source
/// option (`Tile::size` in the Slint model) rather than a global const.
pub const TILE_SIZE_DEFAULT: f64 = 256.0;

/// Maximum latitude representable in Web Mercator. Beyond this the
/// projection blows up to infinity (the math is `atanh(sin(lat))`).
pub const MAX_LATITUDE: f64 = 85.051_128_779_806_59;

/// Convert a geographic coordinate + zoom to fractional tile-space.
///
/// At zoom 0 the whole world is one tile so the result is in `[0, 1)`;
/// at zoom 12 it's in `[0, 4096)`; etc.
#[inline]
pub fn lonlat_to_tile(longitude: f64, latitude: f64, zoom: f64) -> (f64, f64) {
    let n = 2f64.powf(zoom);
    let x = (longitude + 180.0) / 360.0 * n;
    let lat_rad = latitude.clamp(-MAX_LATITUDE, MAX_LATITUDE).to_radians();
    let y = (1.0 - (lat_rad.tan() + 1.0 / lat_rad.cos()).ln() / std::f64::consts::PI) / 2.0 * n;
    (x, y)
}

/// Inverse of [`lonlat_to_tile`].
#[inline]
pub fn tile_to_lonlat(tile_x: f64, tile_y: f64, zoom: f64) -> (f64, f64) {
    let n = 2f64.powf(zoom);
    let longitude = tile_x / n * 360.0 - 180.0;
    let lat_rad = (std::f64::consts::PI * (1.0 - 2.0 * tile_y / n))
        .sinh()
        .atan();
    (longitude, lat_rad.to_degrees())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_zero_meridian() {
        for z in 0..=18 {
            let (x, y) = lonlat_to_tile(0.0, 0.0, z as f64);
            let (lon, lat) = tile_to_lonlat(x, y, z as f64);
            assert!((lon - 0.0).abs() < 1e-9, "z={z} lon drift");
            assert!((lat - 0.0).abs() < 1e-9, "z={z} lat drift");
        }
    }

    #[test]
    fn roundtrip_known_cities() {
        let cases = [
            ("NYC", -74.0060, 40.7128),
            ("Sydney", 151.2093, -33.8688),
            ("Reykjavik", -21.9426, 64.1466),
        ];
        for (name, lon, lat) in cases {
            for z in [0.0, 5.0, 10.0, 17.0] {
                let (x, y) = lonlat_to_tile(lon, lat, z);
                let (lon2, lat2) = tile_to_lonlat(x, y, z);
                assert!((lon - lon2).abs() < 1e-9, "{name}@z{z} lon drift");
                assert!((lat - lat2).abs() < 1e-9, "{name}@z{z} lat drift");
            }
        }
    }

    #[test]
    fn z0_covers_world_in_one_tile() {
        // At zoom 0, every point on earth maps into the single tile
        // (0, 0) at the [0, 1) coordinate range.
        for lon in [-179.9, -90.0, 0.0, 90.0, 179.9] {
            for lat in [-80.0, -45.0, 0.0, 45.0, 80.0] {
                let (x, y) = lonlat_to_tile(lon, lat, 0.0);
                assert!((0.0..1.0).contains(&x), "x out of range: {x}");
                assert!((0.0..1.0).contains(&y), "y out of range: {y}");
            }
        }
    }
}
