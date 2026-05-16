//! Visible-tile computation.
//!
//! Given the camera (`centre_lon`, `centre_lat`, `zoom`) and viewport
//! size in pixels, decide which `(x, y, z)` tiles overlap the viewport
//! and at what pixel offset each one should be drawn.

use crate::projection::{lonlat_to_tile, tile_to_lonlat};
use crate::source::TileKey;

/// One tile selected for display, with its pixel offset inside the
/// viewport. Mirrors the `Tile` slint struct exposed by `MapView`.
#[derive(Debug, Clone, Copy)]
pub struct PlacedTile {
    pub key: TileKey,
    /// Pixel offset of the tile's top-left corner from the viewport
    /// top-left. May be negative when the tile extends off-screen left
    /// or above the viewport.
    pub x: f32,
    pub y: f32,
    pub size: f32,
}

/// Compute the set of tiles that overlap the viewport, in row-major
/// order (top-to-bottom, left-to-right within each row). The result
/// is small (typically <50 tiles even for a large window) so we
/// return an owned `Vec`.
pub fn visible_tiles(
    centre_longitude: f64,
    centre_latitude: f64,
    zoom: f64,
    viewport_width: f64,
    viewport_height: f64,
    tile_size: u32,
) -> Vec<PlacedTile> {
    // Use `floor` to pick which integer zoom layer of tiles to fetch,
    // and scale each tile by `2^(zoom - z_floor)` when rendered. This
    // keeps the tile layer at the same geographic scale as the
    // fractional zoom used by marker projection + the cursor-anchor
    // math in `lonlat_to_viewport_px` / `center_for_anchor_at_viewport_px`
    // — without scaling, a half-step wheel notch (zoom 10 → 10.5)
    // rendered tiles as if at zoom 11 while markers and the anchor
    // computation lived at 10.5, so the cursor and markers visibly
    // drifted off the tile features they should sit on.
    let z_floor = zoom.floor().clamp(0.0, 22.0) as u8;
    let z_for_proj = z_floor as f64;
    let frac = zoom - z_for_proj;
    let scale = 2.0_f64.powf(frac);
    let tile_size_f = tile_size as f64 * scale;

    // Where is the camera centre, in fractional tile-space at z_floor?
    let (centre_tx, centre_ty) = lonlat_to_tile(centre_longitude, centre_latitude, z_for_proj);

    // Which pixel inside the centre tile is the viewport centre?
    let centre_px_in_tile_x = (centre_tx.fract()) * tile_size_f;
    let centre_px_in_tile_y = (centre_ty.fract()) * tile_size_f;
    let centre_tile_x = centre_tx.floor() as i64;
    let centre_tile_y = centre_ty.floor() as i64;

    // Viewport centre in screen pixels.
    let vp_cx = viewport_width / 2.0;
    let vp_cy = viewport_height / 2.0;

    // How many tiles do we need on each side of the centre tile to
    // cover the viewport? Add one extra ring for slight over-draw so
    // panning doesn't expose blank edges before the next refresh.
    let tiles_left = ((vp_cx + centre_px_in_tile_x) / tile_size_f).ceil() as i64 + 1;
    let tiles_right =
        ((viewport_width - vp_cx + (tile_size_f - centre_px_in_tile_x)) / tile_size_f).ceil() as i64 + 1;
    let tiles_above = ((vp_cy + centre_px_in_tile_y) / tile_size_f).ceil() as i64 + 1;
    let tiles_below = ((viewport_height - vp_cy + (tile_size_f - centre_px_in_tile_y))
        / tile_size_f).ceil() as i64 + 1;

    let max_tile_idx = 1i64 << z_floor as i64; // 2^z_floor

    let mut out = Vec::with_capacity(
        ((tiles_left + tiles_right) * (tiles_above + tiles_below)) as usize,
    );

    for ty in (centre_tile_y - tiles_above)..=(centre_tile_y + tiles_below) {
        // Clamp Y to valid tile range; outside is "no tile here" (poles).
        if ty < 0 || ty >= max_tile_idx {
            continue;
        }
        for tx in (centre_tile_x - tiles_left)..=(centre_tile_x + tiles_right) {
            // Wrap X across the antimeridian so the world tiles
            // seamlessly when panned past ±180°.
            let wrapped_tx = ((tx % max_tile_idx) + max_tile_idx) % max_tile_idx;
            // Pixel offset: where this tile's top-left sits relative to
            // the viewport's top-left. Negative for tiles whose left
            // edge is off-screen.
            let pixel_x = vp_cx
                - centre_px_in_tile_x
                + ((tx - centre_tile_x) as f64) * tile_size_f;
            let pixel_y = vp_cy
                - centre_px_in_tile_y
                + ((ty - centre_tile_y) as f64) * tile_size_f;
            out.push(PlacedTile {
                key: TileKey {
                    x: wrapped_tx as u32,
                    y: ty as u32,
                    z: z_floor,
                },
                x: pixel_x as f32,
                y: pixel_y as f32,
                // Rendered tile edge length on screen, including the
                // fractional-zoom scale-up factor. Tiles between
                // integer zoom levels render larger than their native
                // 256 px so the visual scale matches the marker /
                // anchor maths.
                size: tile_size_f as f32,
            });
        }
    }
    out
}

/// Project a geographic point (`lon`, `lat`) into viewport pixel
/// coordinates for the same camera (`centre_lon`, `centre_lat`, `zoom`)
/// and viewport size used by [`visible_tiles`]. Used to place markers /
/// overlays on top of the tile layer at the correct screen position.
///
/// Returns the viewport-space (x, y) in pixels with origin at the
/// viewport's top-left. Values can fall outside `[0, viewport_width]` /
/// `[0, viewport_height]` for points that are off-screen — callers
/// that want to cull should filter on those bounds.
pub fn lonlat_to_viewport_px(
    lon: f64,
    lat: f64,
    centre_longitude: f64,
    centre_latitude: f64,
    zoom: f64,
    viewport_width: f64,
    viewport_height: f64,
    tile_size: u32,
) -> (f64, f64) {
    let ts = tile_size as f64;
    let (tx_c, ty_c) = lonlat_to_tile(centre_longitude, centre_latitude, zoom);
    let (tx_p, ty_p) = lonlat_to_tile(lon, lat, zoom);
    (
        viewport_width / 2.0 + (tx_p - tx_c) * ts,
        viewport_height / 2.0 + (ty_p - ty_c) * ts,
    )
}

/// Inverse of [`lonlat_to_viewport_px`]: given a viewport pixel and the
/// current camera, return the geographic point that pixel represents.
/// Used at the start of a zoom burst to lock the anchor's lon/lat —
/// see [`center_for_anchor_at_viewport_px`] for the corresponding
/// "place this lon/lat at this pixel" step.
pub fn viewport_px_to_lonlat(
    px: f64,
    py: f64,
    centre_longitude: f64,
    centre_latitude: f64,
    zoom: f64,
    viewport_width: f64,
    viewport_height: f64,
    tile_size: u32,
) -> (f64, f64) {
    let ts = tile_size as f64;
    let (tx_c, ty_c) = lonlat_to_tile(centre_longitude, centre_latitude, zoom);
    let dx = px - viewport_width / 2.0;
    let dy = py - viewport_height / 2.0;
    tile_to_lonlat(tx_c + dx / ts, ty_c + dy / ts, zoom)
}

/// Given a fixed geographic anchor (`anchor_lon`, `anchor_lat`) and the
/// viewport pixel where it must land, compute the camera centre
/// (lon, lat) at the given zoom. The pairing of this with
/// [`viewport_px_to_lonlat`] lets callers run a multi-event zoom
/// gesture against a single anchor captured at burst-start, so the
/// camera doesn't drift between scroll-events as it would if the
/// anchor's geographic position were re-derived each event.
pub fn center_for_anchor_at_viewport_px(
    anchor_lon: f64,
    anchor_lat: f64,
    anchor_x_px: f64,
    anchor_y_px: f64,
    zoom: f64,
    viewport_width: f64,
    viewport_height: f64,
    tile_size: u32,
) -> (f64, f64) {
    let ts = tile_size as f64;
    let (tx_a, ty_a) = lonlat_to_tile(anchor_lon, anchor_lat, zoom);
    let adx = anchor_x_px - viewport_width / 2.0;
    let ady = anchor_y_px - viewport_height / 2.0;
    tile_to_lonlat(tx_a - adx / ts, ty_a - ady / ts, zoom)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zoom_zero_returns_just_the_world_tile() {
        let tiles = visible_tiles(0.0, 0.0, 0.0, 256.0, 256.0, 256);
        assert!(
            tiles.iter().any(|t| t.key == TileKey { x: 0, y: 0, z: 0 }),
            "should include (0,0,0)"
        );
        // Other "tiles" requested past the single 1×1 world get clamped
        // out by the Y check; X wraps to 0.
    }

    #[test]
    fn antimeridian_wraps() {
        // At zoom 1 there are 2 tiles wide. Camera near +180° should
        // still find tiles at x=0 (the wrap) and x=1.
        let tiles = visible_tiles(179.0, 0.0, 1.0, 1024.0, 256.0, 256);
        let xs: Vec<u32> = tiles.iter().map(|t| t.key.x).collect();
        assert!(xs.contains(&0));
        assert!(xs.contains(&1));
    }

    #[test]
    fn polar_extents_are_clipped() {
        // North of Mercator's valid range (~85.05°) there are no
        // tiles. With the camera centred at extreme high latitude and
        // a tall viewport, we should NOT see tiles with y < 0 or
        // y >= 2^z.
        let z = 4u8;
        let max_y = 1u32 << z;
        let tiles = visible_tiles(0.0, 85.0, z as f64, 1024.0, 2048.0, 256);
        for t in &tiles {
            assert!(
                t.key.y < max_y,
                "y={} exceeds valid range at zoom {z}",
                t.key.y
            );
        }
    }

    #[test]
    fn every_tile_has_unique_pixel_position_at_centre_camera() {
        // No two visible tiles should collide on (x, y) — would mean
        // the viewport math placed them on top of each other.
        let tiles = visible_tiles(0.0, 0.0, 3.0, 1024.0, 768.0, 256);
        for (i, a) in tiles.iter().enumerate() {
            for b in &tiles[i + 1..] {
                let same_x = (a.x - b.x).abs() < 0.5;
                let same_y = (a.y - b.y).abs() < 0.5;
                assert!(
                    !(same_x && same_y),
                    "tiles {a:?} and {b:?} share a pixel slot"
                );
            }
        }
    }

    #[test]
    fn project_centre_lands_at_viewport_centre() {
        // The camera's own centre point should always land at the
        // viewport's centre pixel, regardless of zoom / size.
        let (x, y) = lonlat_to_viewport_px(
            -0.1276, 51.5074, -0.1276, 51.5074, 13.0, 800.0, 600.0, 256,
        );
        assert!((x - 400.0).abs() < 1e-6, "x={x}");
        assert!((y - 300.0).abs() < 1e-6, "y={y}");
    }

    #[test]
    fn project_offset_point_falls_off_centre_in_expected_direction() {
        // Point east of camera centre → x > viewport centre x.
        let cx = 400.0;
        let cy = 300.0;
        let (x_east, y_east) = lonlat_to_viewport_px(
            1.0, 51.5074, 0.0, 51.5074, 8.0, 800.0, 600.0, 256,
        );
        assert!(x_east > cx, "east of camera should be right-of-centre, got x={x_east}");
        assert!((y_east - cy).abs() < 1.0, "same latitude → near vertical centre");

        // Point south → y > viewport centre y (Mercator y increases going south).
        let (x_south, y_south) = lonlat_to_viewport_px(
            0.0, 50.0, 0.0, 51.5074, 8.0, 800.0, 600.0, 256,
        );
        assert!(y_south > cy, "south of camera should be below centre, got y={y_south}");
        assert!((x_south - cx).abs() < 1.0, "same longitude → near horizontal centre");
    }

    #[test]
    fn viewport_round_trip_is_identity() {
        // lon/lat → px → lon/lat should round-trip to high precision
        // when nothing else changes — the projection itself is
        // numerically stable at typical zooms.
        let (centre_lon, centre_lat, zoom) = (-0.1276, 51.5074, 13.0);
        let pts = [(-0.05, 51.51), (-0.18, 51.49), (0.0, 51.5074)];
        for (lon, lat) in pts {
            let (px, py) = lonlat_to_viewport_px(
                lon, lat, centre_lon, centre_lat, zoom, 800.0, 600.0, 256,
            );
            let (rlon, rlat) = viewport_px_to_lonlat(
                px, py, centre_lon, centre_lat, zoom, 800.0, 600.0, 256,
            );
            assert!((rlon - lon).abs() < 1e-9, "lon: {lon} → {rlon}");
            assert!((rlat - lat).abs() < 1e-9, "lat: {lat} → {rlat}");
        }
    }

    #[test]
    fn burst_zoom_keeps_anchor_pinned_to_pixel() {
        // Simulate a multi-event zoom burst: capture anchor at burst
        // start, then zoom several steps. Anchor must land at the
        // same viewport pixel after each step. This is the property
        // the burst-locked zoom in the viewer relies on to stop the
        // camera drifting between scroll events.
        let (centre_lon, centre_lat) = (-0.1276, 51.5074);
        let (vp_w, vp_h) = (800.0, 600.0);
        let (anchor_px, anchor_py) = (620.0, 180.0); // offset from centre
        let start_zoom = 10.0;

        let (alon, alat) = viewport_px_to_lonlat(
            anchor_px, anchor_py, centre_lon, centre_lat, start_zoom, vp_w, vp_h, 256,
        );

        for &z in &[10.5_f64, 11.0, 12.0, 13.7, 15.0] {
            let (new_lon, new_lat) = center_for_anchor_at_viewport_px(
                alon, alat, anchor_px, anchor_py, z, vp_w, vp_h, 256,
            );
            let (rpx, rpy) = lonlat_to_viewport_px(
                alon, alat, new_lon, new_lat, z, vp_w, vp_h, 256,
            );
            assert!((rpx - anchor_px).abs() < 1e-6, "zoom={z} px: {anchor_px} → {rpx}");
            assert!((rpy - anchor_py).abs() < 1e-6, "zoom={z} py: {anchor_py} → {rpy}");
        }
    }

    // ============================================================
    // Cursor-anchored zoom mechanics
    // ============================================================
    //
    // End-to-end simulation of the burst-locked zoom-on-cursor flow
    // implemented in `slint-mobile-components/crates/viewer/src/main.rs`:
    //
    //   1. Lock the anchor at burst start:
    //        anchor_geo = viewport_px_to_lonlat(cursor_px, camera_before)
    //   2. For each subsequent event in the burst, recompute the camera
    //      so that the (unchanged) anchor_geo lands back on the
    //      (unchanged) cursor pixel:
    //        new_centre = center_for_anchor_at_viewport_px(
    //            anchor_geo, cursor_px, new_zoom)
    //   3. Verify: lonlat_to_viewport_px(anchor_geo, new_centre, new_zoom)
    //      == cursor_px
    //
    // The point on the map under the cursor must not move — that's the
    // whole UX promise. Tested at three deliberately chosen cursor
    // positions (top-left corner, off-centre, "random") and a sweep of
    // zoom deltas covering both directions plus fractional steps.

    /// Helper — for a given cursor position, run a multi-step zoom
    /// burst through every (zoom_before + delta) listed and assert the
    /// anchor's geographic position lands back at the original cursor
    /// pixel after each step.
    fn assert_anchor_stays_pinned(label: &str, cursor_px: (f64, f64)) {
        // London at z=10. Arbitrary but realistic — exercises non-zero
        // tile-coord fractions on both axes.
        let centre_lon = -0.1276_f64;
        let centre_lat = 51.5074_f64;
        let zoom_before = 10.0_f64;
        let (vp_w, vp_h) = (800.0_f64, 600.0_f64);
        let tile_size = 256_u32;

        let (anchor_lon, anchor_lat) = viewport_px_to_lonlat(
            cursor_px.0, cursor_px.1,
            centre_lon, centre_lat, zoom_before,
            vp_w, vp_h, tile_size,
        );

        // Sweep deltas: small + large, positive + negative, integer +
        // fractional. Any of these breaking would surface as the cursor
        // visually drifting across a continuous scroll.
        let deltas = [0.25_f64, 0.5, 1.0, 1.7, 3.0, -0.5, -1.0, -2.3];
        for delta in deltas {
            let new_zoom = zoom_before + delta;
            let (new_centre_lon, new_centre_lat) = center_for_anchor_at_viewport_px(
                anchor_lon, anchor_lat,
                cursor_px.0, cursor_px.1,
                new_zoom,
                vp_w, vp_h, tile_size,
            );
            let (rpx, rpy) = lonlat_to_viewport_px(
                anchor_lon, anchor_lat,
                new_centre_lon, new_centre_lat,
                new_zoom,
                vp_w, vp_h, tile_size,
            );
            // Tolerance is 1e-6 logical pixels — well below any
            // possible visible drift. Float precision lets us go even
            // tighter; this leaves headroom.
            assert!(
                (rpx - cursor_px.0).abs() < 1e-6,
                "{label}: x drifted at delta={delta} — wanted {}, got {rpx}",
                cursor_px.0,
            );
            assert!(
                (rpy - cursor_px.1).abs() < 1e-6,
                "{label}: y drifted at delta={delta} — wanted {}, got {rpy}",
                cursor_px.1,
            );
        }
    }

    #[test]
    fn zoom_anchor_pinned_at_top_left_corner() {
        // Worst case for anchor math: maximal (adx, ady) magnitudes
        // relative to the viewport centre, so any centre-offset bug
        // would show up here largest.
        assert_anchor_stays_pinned("top-left", (10.0, 10.0));
    }

    #[test]
    fn zoom_anchor_pinned_off_centre() {
        // Realistic "user puts cursor on a feature near the top-right"
        // — non-symmetric offset on both axes.
        assert_anchor_stays_pinned("off-centre", (650.0, 120.0));
    }

    #[test]
    fn zoom_anchor_pinned_at_random_point() {
        // A deterministically-chosen "random" cursor — picked once via
        // a hash of the test name to land at a non-axis-aligned spot
        // that wouldn't be caught by a symmetric configuration. Kept
        // hard-coded so failures are reproducible.
        assert_anchor_stays_pinned("random", (317.42, 463.81));
    }

    #[test]
    fn zoom_anchor_pinned_through_a_simulated_burst() {
        // Multi-event burst from the same anchor — what actually
        // happens during a continuous wheel-spin. Each successive
        // event reuses the burst's locked anchor against an updated
        // camera; the cursor pixel must still pin.
        let (vp_w, vp_h) = (800.0_f64, 600.0_f64);
        let tile_size = 256_u32;
        let cursor = (520.0_f64, 95.0_f64);

        let mut centre_lon = -0.1276_f64;
        let mut centre_lat = 51.5074_f64;
        let mut zoom = 10.0_f64;

        // Lock the anchor against the *initial* camera (burst start).
        let (anchor_lon, anchor_lat) = viewport_px_to_lonlat(
            cursor.0, cursor.1,
            centre_lon, centre_lat, zoom,
            vp_w, vp_h, tile_size,
        );

        // Apply a burst of zoom-in events.
        for step in 0..8 {
            zoom += 0.5;
            let (new_centre_lon, new_centre_lat) = center_for_anchor_at_viewport_px(
                anchor_lon, anchor_lat,
                cursor.0, cursor.1,
                zoom,
                vp_w, vp_h, tile_size,
            );
            centre_lon = new_centre_lon;
            centre_lat = new_centre_lat;

            let (rpx, rpy) = lonlat_to_viewport_px(
                anchor_lon, anchor_lat,
                centre_lon, centre_lat, zoom,
                vp_w, vp_h, tile_size,
            );
            assert!(
                (rpx - cursor.0).abs() < 1e-6 && (rpy - cursor.1).abs() < 1e-6,
                "step {step}: cursor drifted from {cursor:?} to ({rpx}, {rpy}) at zoom {zoom}",
            );
        }
    }

    #[test]
    fn zoom_anchor_pinned_across_full_bleed_phone_viewport() {
        // Regression for the bug that motivated these tests: hardcoded
        // 412×892 viewport constants in the viewer didn't match the
        // page's actual MapEmbed size at runtime, so the anchor maths
        // computed against the wrong viewport centre and the cursor
        // appeared to drift. Run with the real cell size to make sure
        // a future regression of "hardcoded constants creep back in"
        // would be caught with realistic geometry.
        let (vp_w, vp_h) = (412.0_f64, 892.0_f64);
        let tile_size = 256_u32;
        let cursor = (305.0_f64, 740.0_f64);
        let centre_lon = -0.1276_f64;
        let centre_lat = 51.5074_f64;
        let zoom_before = 13.0_f64;

        let (anchor_lon, anchor_lat) = viewport_px_to_lonlat(
            cursor.0, cursor.1,
            centre_lon, centre_lat, zoom_before,
            vp_w, vp_h, tile_size,
        );

        for &dz in &[0.5_f64, 1.0, 2.0, -1.0] {
            let new_zoom = zoom_before + dz;
            let (new_centre_lon, new_centre_lat) = center_for_anchor_at_viewport_px(
                anchor_lon, anchor_lat,
                cursor.0, cursor.1,
                new_zoom,
                vp_w, vp_h, tile_size,
            );
            let (rpx, rpy) = lonlat_to_viewport_px(
                anchor_lon, anchor_lat,
                new_centre_lon, new_centre_lat,
                new_zoom,
                vp_w, vp_h, tile_size,
            );
            assert!((rpx - cursor.0).abs() < 1e-6, "x at dz={dz}: {} → {rpx}", cursor.0);
            assert!((rpy - cursor.1).abs() < 1e-6, "y at dz={dz}: {} → {rpy}", cursor.1);
        }
    }

    #[test]
    fn fractional_zoom_scales_rendered_tile_size() {
        // Regression: visible_tiles used to snap zoom to the nearest
        // integer and render tiles at native size, while marker /
        // anchor maths ran at the fractional zoom. Cursor + markers
        // drifted off the tile features they should have sat on at
        // every non-integer zoom.
        //
        // Now: at z = z_floor + frac, each tile renders at
        // 256 * 2^frac px on screen, so the tile layer's geographic
        // scale matches the maths that uses fractional zoom directly.
        let at_int = visible_tiles(0.0, 0.0, 10.0, 800.0, 600.0, 256);
        let at_half = visible_tiles(0.0, 0.0, 10.5, 800.0, 600.0, 256);
        let at_one_below_next = visible_tiles(0.0, 0.0, 10.999, 800.0, 600.0, 256);

        let int_size = at_int[0].size as f64;
        let half_size = at_half[0].size as f64;
        let nearly_next_size = at_one_below_next[0].size as f64;

        assert!((int_size - 256.0).abs() < 1e-3, "int zoom: {int_size}");
        // 256 * 2^0.5 ≈ 362.04
        assert!(
            (half_size - 362.039).abs() < 0.1,
            "half-step zoom should scale tile size to ~362, got {half_size}",
        );
        // 256 * 2^0.999 ≈ 511.65 — almost back to native-double
        assert!(
            (nearly_next_size - 511.65).abs() < 0.2,
            "almost-next zoom should scale to ~512, got {nearly_next_size}",
        );

        // All three should pick z_floor for their tile keys (10 in
        // every case, since 10.999 still floors to 10).
        for t in &at_half {
            assert_eq!(t.key.z, 10, "fractional zoom must keep z_floor in tile key");
        }
        for t in &at_one_below_next {
            assert_eq!(t.key.z, 10);
        }
    }

    #[test]
    fn marker_lines_up_with_tile_under_it_at_fractional_zoom() {
        // The actual UX guarantee: a marker placed at the camera centre
        // should land at the viewport centre regardless of fractional
        // zoom, AND the centre tile (the one containing the camera
        // centre) should render across the viewport centre too. Tile
        // and marker maths must agree on what "1 fractional tile"
        // means on screen.
        let centre_lon = -0.1276_f64;
        let centre_lat = 51.5074_f64;
        let (vp_w, vp_h) = (800.0_f64, 600.0_f64);

        for zoom in [10.0_f64, 10.25, 10.5, 10.75, 11.0] {
            let (mpx, mpy) = lonlat_to_viewport_px(
                centre_lon, centre_lat,
                centre_lon, centre_lat, zoom,
                vp_w, vp_h, 256,
            );
            assert!((mpx - vp_w / 2.0).abs() < 1e-6, "marker x at z={zoom}: {mpx}");
            assert!((mpy - vp_h / 2.0).abs() < 1e-6, "marker y at z={zoom}: {mpy}");

            // The tile whose footprint contains the camera centre.
            let tiles = visible_tiles(centre_lon, centre_lat, zoom, vp_w, vp_h, 256);
            let scale = 2.0_f64.powf(zoom - zoom.floor());
            let expected_tile_size = 256.0 * scale;
            for t in &tiles {
                assert!(
                    (t.size as f64 - expected_tile_size).abs() < 1e-3,
                    "tile size at z={zoom}: expected {expected_tile_size}, got {}",
                    t.size,
                );
            }
        }
    }

    #[test]
    fn larger_viewport_returns_at_least_as_many_tiles() {
        // Monotonicity: doubling the viewport size should never reduce
        // the visible-tile count.
        let small = visible_tiles(0.0, 0.0, 4.0, 512.0, 512.0, 256).len();
        let big = visible_tiles(0.0, 0.0, 4.0, 1024.0, 1024.0, 256).len();
        assert!(
            big >= small,
            "bigger viewport had fewer tiles ({big} vs {small})"
        );
    }
}
