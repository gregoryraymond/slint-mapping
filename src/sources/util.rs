//! Shared helpers for HTTP-style tile sources — URL templating and
//! PNG decoding. Both `OsmTileSource` (native, gated by `http`) and
//! `WasmOsmTileSource` (browser, gated by `wasm`) call into this so
//! the templating + decoding logic doesn't fork between the two
//! transport stacks.

use crate::source::TileKey;
use slint::{Rgba8Pixel, SharedPixelBuffer};

/// Substitute `{z}` / `{x}` / `{y}` placeholders in a slippy-map URL
/// template against a tile key. Format matches every common provider
/// (`https://tile.openstreetmap.org/{z}/{x}/{y}.png`,
/// `https://tile.stadia.com/.../{z}/{x}/{y}@2x.png`, …).
pub(crate) fn format_url(template: &str, key: TileKey) -> String {
    template
        .replace("{z}", &key.z.to_string())
        .replace("{x}", &key.x.to_string())
        .replace("{y}", &key.y.to_string())
}

/// Decode a PNG byte buffer into an owned `SharedPixelBuffer` ready
/// to wrap with `slint::Image::from_rgba8`. Returns `None` on any
/// decode error (malformed PNG, truncated data, wrong colour space) —
/// callers should mark the key as failed and serve a placeholder.
pub(crate) fn decode_png_to_buffer(bytes: &[u8]) -> Option<SharedPixelBuffer<Rgba8Pixel>> {
    let img = image::load_from_memory(bytes).ok()?;
    let rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();
    let mut buf = SharedPixelBuffer::<Rgba8Pixel>::new(w, h);
    let dst = buf.make_mut_slice();
    for (out, chunk) in dst.iter_mut().zip(rgba.chunks_exact(4)) {
        *out = Rgba8Pixel {
            r: chunk[0],
            g: chunk[1],
            b: chunk[2],
            a: chunk[3],
        };
    }
    Some(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_url_substitutes() {
        assert_eq!(
            format_url("https://x/{z}/{x}/{y}.png", TileKey { x: 5, y: 10, z: 2 }),
            "https://x/2/5/10.png"
        );
    }
}
