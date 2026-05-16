//! [`SyntheticTileSource`] — generates coloured test tiles in-process.
//!
//! Useful for end-to-end testing the framework without a real tile
//! bundle. Each tile gets:
//!   - A solid background colour derived from `(z, x, y)` so adjacent
//!     tiles are visually distinguishable.
//!   - A 1-pixel border so tile seams are visible.
//!   - A pair of crossing lines through the centre at higher zooms so
//!     you can sanity-check the projection (the antimeridian should
//!     wrap; the prime meridian and equator should sit on tile edges
//!     at appropriate zoom levels).

use crate::source::{TileKey, TileSource};
use slint::{Image, Rgba8Pixel, SharedPixelBuffer};

pub struct SyntheticTileSource {
    tile_size: u32,
}

impl Default for SyntheticTileSource {
    fn default() -> Self {
        Self { tile_size: 256 }
    }
}

impl SyntheticTileSource {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_tile_size(mut self, size: u32) -> Self {
        self.tile_size = size;
        self
    }
}

impl TileSource for SyntheticTileSource {
    fn tile(&self, key: TileKey) -> Option<Image> {
        let size = self.tile_size;
        let mut buf = SharedPixelBuffer::<Rgba8Pixel>::new(size, size);
        let stride = size as usize;
        let pixels = buf.make_mut_slice();

        // Derive a colour from (z, x, y) so neighbours differ.
        // Hash-ish: take low bits of each, spread across RGB.
        let r = (40 + (key.x.wrapping_mul(73) & 0x7f)) as u8;
        let g = (40 + (key.y.wrapping_mul(151) & 0x7f)) as u8;
        let b = (40 + ((key.x ^ key.y).wrapping_mul(199) & 0x7f)) as u8;
        let fill = Rgba8Pixel { r, g, b, a: 255 };
        let border = Rgba8Pixel { r: 255, g: 255, b: 255, a: 100 };
        let cross = Rgba8Pixel { r: 255, g: 255, b: 255, a: 60 };

        for y in 0..size as usize {
            for x in 0..size as usize {
                let on_border = x == 0 || y == 0 || x == stride - 1 || y == stride - 1;
                let on_cross = (x == stride / 2) || (y == stride / 2);
                pixels[y * stride + x] = if on_border {
                    border
                } else if on_cross && key.z >= 4 {
                    cross
                } else {
                    fill
                };
            }
        }

        Some(Image::from_rgba8(buf))
    }

    fn tile_size(&self) -> u32 {
        self.tile_size
    }

    fn max_zoom(&self) -> u8 {
        18
    }
}
