//! Visible-tile computation.
//!
//! Given the camera (`centre_lon`, `centre_lat`, `zoom`) and viewport
//! size in pixels, decide which `(x, y, z)` tiles overlap the viewport
//! and at what pixel offset each one should be drawn.

use crate::projection::lonlat_to_tile;
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
    // Snap zoom to the nearest integer for tile selection — fractional
    // zoom is a future enhancement (it would scale the tile draw size).
    let z_int = zoom.round() as u8;
    let z_for_proj = z_int as f64;
    let tile_size_f = tile_size as f64;

    // Where is the camera centre, in fractional tile-space?
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

    let max_tile_idx = 1i64 << z_int as i64; // 2^z

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
                    z: z_int,
                },
                x: pixel_x as f32,
                y: pixel_y as f32,
                size: tile_size as f32,
            });
        }
    }
    out
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
}
