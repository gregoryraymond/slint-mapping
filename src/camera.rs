//! Pure-Rust camera math — what happens to `(centre_lon, centre_lat, zoom)`
//! when the user pans by some pixel delta or zooms by some scroll delta?
//!
//! Extracted from `MapController` and the demo wirings so it's testable
//! without spinning up a Slint runtime, and so anyone writing their
//! own controller (e.g. for the `MapEmbed` embedding case) can reuse
//! the same primitives.
//!
//! All inputs/outputs are in geographic degrees + pixel space. The
//! Web Mercator projection lives in [`crate::projection`].

use crate::projection::{lonlat_to_tile, tile_to_lonlat};

/// Apply a pan: the user dragged the visible map by `(dx_px, dy_px)`
/// pixels. Returns the new camera centre.
///
/// Sign convention: a positive `dx_px` means "drag right" — the
/// content under the finger moves right, so the camera moves *left*
/// in geo space and longitude *decreases*. Same for `dy_px` /
/// latitude: drag down → camera moves up → latitude *increases*.
#[inline]
pub fn pan(
    centre_longitude: f64,
    centre_latitude: f64,
    zoom: f64,
    dx_px: f64,
    dy_px: f64,
    tile_size: u32,
) -> (f64, f64) {
    let ts = tile_size as f64;
    let (tx, ty) = lonlat_to_tile(centre_longitude, centre_latitude, zoom);
    tile_to_lonlat(tx - dx_px / ts, ty - dy_px / ts, zoom)
}

/// Apply a zoom change anchored at a viewport pixel.
///
/// The geo-coordinate currently under the anchor pixel is captured at
/// `zoom_before`, then the camera is re-centred at `zoom_after` so the
/// same geo-coordinate still sits under the anchor pixel afterwards.
/// `zoom_after` is clamped to `[min_zoom, max_zoom]`.
///
/// Returns `(new_lon, new_lat, new_zoom)`.
#[inline]
#[allow(clippy::too_many_arguments)]
pub fn zoom_anchored(
    centre_longitude: f64,
    centre_latitude: f64,
    zoom_before: f64,
    delta: f64,
    anchor_x_px: f64,
    anchor_y_px: f64,
    viewport_width_px: f64,
    viewport_height_px: f64,
    tile_size: u32,
    min_zoom: u8,
    max_zoom: u8,
) -> (f64, f64, f64) {
    let ts = tile_size as f64;
    let (tx_c, ty_c) = lonlat_to_tile(centre_longitude, centre_latitude, zoom_before);
    let adx = anchor_x_px - viewport_width_px / 2.0;
    let ady = anchor_y_px - viewport_height_px / 2.0;
    let (anchor_lon, anchor_lat) =
        tile_to_lonlat(tx_c + adx / ts, ty_c + ady / ts, zoom_before);

    let zoom_after = (zoom_before + delta).clamp(min_zoom as f64, max_zoom as f64);

    let (tx_an, ty_an) = lonlat_to_tile(anchor_lon, anchor_lat, zoom_after);
    let (new_lon, new_lat) =
        tile_to_lonlat(tx_an - adx / ts, ty_an - ady / ts, zoom_after);
    (new_lon, new_lat, zoom_after)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::projection::lonlat_to_tile;

    /// Default tile size used in tests (matches OSM raster tiles).
    const TS: u32 = 256;

    fn assert_close(a: f64, b: f64, eps: f64, msg: &str) {
        assert!((a - b).abs() < eps, "{msg}: {a} vs {b} (diff {})", (a - b).abs());
    }

    // ---------- pan ----------

    #[test]
    fn pan_zero_delta_is_noop() {
        let (lon, lat) = pan(-0.1276, 51.5074, 12.0, 0.0, 0.0, TS);
        assert_close(lon, -0.1276, 1e-12, "lon");
        assert_close(lat, 51.5074, 1e-12, "lat");
    }

    #[test]
    fn pan_right_decreases_longitude() {
        // Dragging right (positive dx) should move the camera west.
        let (lon, _) = pan(0.0, 0.0, 4.0, 50.0, 0.0, TS);
        assert!(lon < 0.0, "lon should drop, got {lon}");
    }

    #[test]
    fn pan_left_increases_longitude() {
        let (lon, _) = pan(0.0, 0.0, 4.0, -50.0, 0.0, TS);
        assert!(lon > 0.0, "lon should rise, got {lon}");
    }

    #[test]
    fn pan_down_increases_latitude() {
        // Dragging down should move the camera north.
        let (_, lat) = pan(0.0, 0.0, 4.0, 0.0, 50.0, TS);
        assert!(lat > 0.0, "lat should rise, got {lat}");
    }

    #[test]
    fn pan_up_decreases_latitude() {
        let (_, lat) = pan(0.0, 0.0, 4.0, 0.0, -50.0, TS);
        assert!(lat < 0.0, "lat should drop, got {lat}");
    }

    #[test]
    fn pan_is_reversible_within_tolerance() {
        // Pan by (dx, dy) then by (-dx, -dy) should return to the
        // starting point. Mercator's non-linearity in y means we lose
        // a bit of precision near the poles but at mid-latitudes it
        // should round-trip to many decimal places.
        let start_lon = -0.1276f64;
        let start_lat = 51.5074;
        let zoom = 12.0;
        let dx = 137.0f64;
        let dy = -89.0;
        let (lon1, lat1) = pan(start_lon, start_lat, zoom, dx, dy, TS);
        let (lon2, lat2) = pan(lon1, lat1, zoom, -dx, -dy, TS);
        assert_close(lon2, start_lon, 1e-10, "lon round-trip");
        assert_close(lat2, start_lat, 1e-10, "lat round-trip");
    }

    #[test]
    fn pan_magnitude_scales_with_zoom() {
        // At higher zoom one tile covers less ground, so panning by
        // the same pixel distance should move fewer geographic degrees.
        let (lon_low, _) = pan(0.0, 0.0, 2.0, 256.0, 0.0, TS);
        let (lon_high, _) = pan(0.0, 0.0, 12.0, 256.0, 0.0, TS);
        assert!(
            lon_low.abs() > lon_high.abs() * 100.0,
            "low-zoom pan should cover much more longitude than high-zoom: {lon_low} vs {lon_high}"
        );
    }

    // ---------- zoom_anchored ----------

    #[test]
    fn zoom_at_centre_does_not_move_camera() {
        // Zooming with the anchor at the exact viewport centre should
        // leave the camera centre unchanged.
        let (lon, lat, z) = zoom_anchored(
            -0.1276, 51.5074, 10.0, 1.0,
            /* anchor centred: */ 400.0, 300.0,
            /* viewport       : */ 800.0, 600.0,
            TS, 0, 22,
        );
        assert_close(lon, -0.1276, 1e-9, "centre-anchor lon");
        assert_close(lat, 51.5074, 1e-9, "centre-anchor lat");
        assert_close(z, 11.0, 1e-12, "zoom updated");
    }

    #[test]
    fn zoom_at_corner_keeps_corner_geocoord_fixed() {
        // The invariant that makes scroll-zoom feel natural: whatever
        // geo-coordinate is under the anchor pixel before the zoom
        // must still be under the same anchor pixel after.
        let centre_lon = -0.1276;
        let centre_lat = 51.5074;
        let zoom_before = 10.0;
        let vw = 800.0;
        let vh = 600.0;
        let anchor_x = 0.0;
        let anchor_y = 0.0;

        // Geo-coord under (0,0) before zoom — compute via projection.
        let (tx_c, ty_c) = lonlat_to_tile(centre_lon, centre_lat, zoom_before);
        let adx = anchor_x - vw / 2.0;
        let ady = anchor_y - vh / 2.0;
        let (anchor_lon_before, anchor_lat_before) =
            crate::projection::tile_to_lonlat(tx_c + adx / TS as f64, ty_c + ady / TS as f64, zoom_before);

        // Apply the zoom.
        let (new_lon, new_lat, new_zoom) = zoom_anchored(
            centre_lon, centre_lat, zoom_before, 2.0,
            anchor_x, anchor_y, vw, vh,
            TS, 0, 22,
        );

        // Geo-coord under (0,0) AFTER the zoom — should match.
        let (tx_c2, ty_c2) = lonlat_to_tile(new_lon, new_lat, new_zoom);
        let (anchor_lon_after, anchor_lat_after) = crate::projection::tile_to_lonlat(
            tx_c2 + adx / TS as f64,
            ty_c2 + ady / TS as f64,
            new_zoom,
        );

        assert_close(
            anchor_lon_after, anchor_lon_before, 1e-9,
            "anchor longitude should stay under the cursor",
        );
        assert_close(
            anchor_lat_after, anchor_lat_before, 1e-9,
            "anchor latitude should stay under the cursor",
        );
    }

    #[test]
    fn zoom_clamps_to_max() {
        let (_, _, z) = zoom_anchored(
            0.0, 0.0, 18.0, /* delta */ 10.0,
            0.0, 0.0, 800.0, 600.0, TS, 0, 22,
        );
        assert_eq!(z, 22.0, "should clamp at max");
    }

    #[test]
    fn zoom_clamps_to_min() {
        let (_, _, z) = zoom_anchored(
            0.0, 0.0, 2.0, /* delta */ -10.0,
            0.0, 0.0, 800.0, 600.0, TS, 0, 22,
        );
        assert_eq!(z, 0.0, "should clamp at min");
    }

    #[test]
    fn zoom_is_reversible_at_centre_anchor() {
        let start = (-0.1276, 51.5074, 10.0);
        let (l1, la1, z1) = zoom_anchored(
            start.0, start.1, start.2, 2.0,
            400.0, 300.0, 800.0, 600.0, TS, 0, 22,
        );
        let (l2, la2, z2) = zoom_anchored(
            l1, la1, z1, -2.0,
            400.0, 300.0, 800.0, 600.0, TS, 0, 22,
        );
        assert_close(z2, start.2, 1e-12, "zoom");
        assert_close(l2, start.0, 1e-9, "lon");
        assert_close(la2, start.1, 1e-9, "lat");
    }

    #[test]
    fn pan_then_undo_via_opposite_pan_at_high_zoom() {
        // Higher-zoom round trip — needs to stay accurate when one
        // pixel of pan corresponds to ~30 cm of longitude.
        let start = (-122.4194, 37.7749); // SF
        let zoom = 17.0;
        let (lon1, lat1) = pan(start.0, start.1, zoom, 113.0, 47.0, TS);
        let (lon2, lat2) = pan(lon1, lat1, zoom, -113.0, -47.0, TS);
        assert_close(lon2, start.0, 1e-10, "lon");
        assert_close(lat2, start.1, 1e-10, "lat");
    }
}
