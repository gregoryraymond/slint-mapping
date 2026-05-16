//! MapPage demo — `MapEmbed` inside a richer layout.
//!
//! ```sh
//! cargo run --example map_page --features viewer            # synthetic tiles
//! cargo run --example map_page --features viewer -- /tiles  # file source
//! ```
//!
//! This is the same composition pattern as the MapPage in
//! `slint-mobile-components/crates/pages-travel/`: a full-bleed map
//! with a floating search card pinned at the top and a "camera info"
//! card pinned at the bottom. It demonstrates how to embed `MapEmbed`
//! inside a parent Window without using `MapController` — useful when
//! you want full control over how camera state flows.

use slint::{ComponentHandle, ModelRc, SharedString, VecModel};
use slint_mapping::sources::{FileTileSource, SyntheticTileSource};
use slint_mapping::{MapPageDemo, Tile, TileSource};
use std::cell::RefCell;
use std::rc::Rc;

fn main() -> Result<(), slint::PlatformError> {
    let arg = std::env::args().nth(1);
    let source: Box<dyn TileSource> = match arg.as_deref() {
        Some("--synthetic") => Box::new(SyntheticTileSource::new()),
        Some(path) => Box::new(FileTileSource::new(path)),
        None => Box::new(FileTileSource::new(slint_mapping::SAMPLE_TILES_DIR)),
    };

    let win = MapPageDemo::new()?;
    win.set_query(SharedString::from(""));
    // Start in central London at zoom 10 — sample bundle has London
    // detail at zoom 4-12.
    win.set_longitude(-0.1276);
    win.set_latitude(51.5074);
    win.set_zoom(10.0);

    let tiles_model: Rc<VecModel<Tile>> = Rc::new(VecModel::from(Vec::new()));
    win.set_tiles(ModelRc::from(tiles_model.clone()));

    // Shared state across the on_pan / on_zoom_by callbacks.
    let state = Rc::new(RefCell::new(State {
        source,
        tiles_model: tiles_model.clone(),
    }));

    // Initial paint once the window has a size — wait one tick.
    {
        let win_weak = win.as_weak();
        let state = Rc::clone(&state);
        slint::Timer::single_shot(std::time::Duration::from_millis(0), move || {
            if let Some(win) = win_weak.upgrade() {
                refresh(&win, &state.borrow());
            }
        });
    }

    // Pan: pixel delta → lat/lon shift via the projection.
    {
        let win_weak = win.as_weak();
        let state = Rc::clone(&state);
        win.on_pan(move |dx, dy| {
            let Some(win) = win_weak.upgrade() else { return };
            let s = state.borrow();
            let tile_size = s.source.tile_size() as f64;
            let z = win.get_zoom() as f64;
            let (tx, ty) = slint_mapping::projection::lonlat_to_tile(
                win.get_longitude() as f64,
                win.get_latitude() as f64,
                z,
            );
            let (lon, lat) = slint_mapping::projection::tile_to_lonlat(
                tx - dx as f64 / tile_size,
                ty - dy as f64 / tile_size,
                z,
            );
            win.set_longitude(lon as f32);
            win.set_latitude(lat as f32);
            refresh(&win, &s);
        });
    }

    // Zoom: cursor-anchored, clamped to source's range.
    {
        let win_weak = win.as_weak();
        let state = Rc::clone(&state);
        win.on_zoom_by(move |delta, anchor_x, anchor_y| {
            let Some(win) = win_weak.upgrade() else { return };
            let s = state.borrow();
            let tile_size = s.source.tile_size() as f64;
            let min_z = s.source.min_zoom() as f64;
            let max_z = s.source.max_zoom() as f64;

            let (vw, vh) = logical_size(&win);
            let z_before = win.get_zoom() as f64;
            let lon = win.get_longitude() as f64;
            let lat = win.get_latitude() as f64;

            // Lat/lon of the anchor point at the current zoom.
            let (tx_c, ty_c) = slint_mapping::projection::lonlat_to_tile(lon, lat, z_before);
            let adx = anchor_x as f64 - vw / 2.0;
            let ady = anchor_y as f64 - vh / 2.0;
            let (anchor_lon, anchor_lat) = slint_mapping::projection::tile_to_lonlat(
                tx_c + adx / tile_size,
                ty_c + ady / tile_size,
                z_before,
            );

            let z_after = (z_before + delta as f64).clamp(min_z, max_z);

            // Re-centre so the anchor coord lands back under the anchor pixel.
            let (tx_a_new, ty_a_new) =
                slint_mapping::projection::lonlat_to_tile(anchor_lon, anchor_lat, z_after);
            let (new_lon, new_lat) = slint_mapping::projection::tile_to_lonlat(
                tx_a_new - adx / tile_size,
                ty_a_new - ady / tile_size,
                z_after,
            );

            win.set_longitude(new_lon as f32);
            win.set_latitude(new_lat as f32);
            win.set_zoom(z_after as f32);
            refresh(&win, &s);
        });
    }

    win.run()
}

struct State {
    source: Box<dyn TileSource>,
    tiles_model: Rc<VecModel<Tile>>,
}

/// Recompute visible tiles for the current camera + viewport, push to the model.
fn refresh(win: &MapPageDemo, s: &State) {
    let (vw, vh) = logical_size(win);
    if vw <= 0.0 || vh <= 0.0 {
        return;
    }
    let placed = slint_mapping::viewport::visible_tiles(
        win.get_longitude() as f64,
        win.get_latitude() as f64,
        win.get_zoom() as f64,
        vw,
        vh,
        s.source.tile_size(),
    );
    let mut rows: Vec<Tile> = Vec::with_capacity(placed.len());
    for p in placed {
        if let Some(image) = s.source.tile(p.key) {
            rows.push(Tile {
                x: p.x,
                y: p.y,
                size: p.size,
                image,
            });
        }
    }
    s.tiles_model.set_vec(rows);
}

fn logical_size(win: &MapPageDemo) -> (f64, f64) {
    let w = win.window();
    let phys = w.size();
    let scale = w.scale_factor() as f64;
    let scale = if scale == 0.0 { 1.0 } else { scale };
    (phys.width as f64 / scale, phys.height as f64 / scale)
}
