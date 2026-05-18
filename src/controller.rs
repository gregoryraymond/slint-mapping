//! [`MapController`] — wires a Slint `MapView` instance to a
//! [`TileSource`](crate::TileSource).
//!
//! The controller:
//! 1. Owns the active tile source (boxed, so it's swappable at
//!    runtime).
//! 2. Holds the canonical camera state (`centre_lon`, `centre_lat`,
//!    `zoom`) — the Slint properties of the same name are mirrors,
//!    updated by [`MapController::refresh`].
//! 3. Translates the `pan` and `zoom-by` callbacks the Slint
//!    `MapView` fires (during user gestures) back into camera moves.
//! 4. Pushes the visible-tile model into the `MapView` after every
//!    refresh.

use crate::camera::{pan as camera_pan, zoom_anchored as camera_zoom_anchored};
use crate::source::TileSource;
use crate::viewport::visible_tiles;
use crate::MapView;
use crate::Tile as SlintTile;
use slint::{ComponentHandle, ModelRc, VecModel};
use std::cell::RefCell;
use std::rc::Rc;

/// Camera state. Authoritative on the Rust side; the Slint mirror
/// properties are written by [`MapController::refresh`].
#[derive(Debug, Clone, Copy)]
struct Camera {
    longitude: f64,
    latitude: f64,
    zoom: f64,
}

/// Owns the source + camera; refreshes the `MapView`'s tile model.
pub struct MapController {
    inner: Rc<RefCell<Inner>>,
}

struct Inner {
    map: slint::Weak<MapView>,
    source: Box<dyn TileSource>,
    camera: Camera,
    tiles_model: Rc<VecModel<SlintTile>>,
}

impl MapController {
    /// Wire a `MapView` instance to a tile source. The controller
    /// immediately computes the visible tiles for the current camera
    /// and pushes them into the view; subsequent
    /// [`refresh`](Self::refresh) calls do the same.
    ///
    /// Callbacks (`pan`, `zoom-by`) on the `MapView` are bound to
    /// camera-mutating handlers — user gestures Just Work.
    pub fn new<S: TileSource + 'static>(map: &MapView, source: S) -> Self {
        let tiles_model = Rc::new(VecModel::<SlintTile>::from(Vec::new()));
        map.set_tiles(ModelRc::from(tiles_model.clone()));

        let inner = Rc::new(RefCell::new(Inner {
            map: map.as_weak(),
            source: Box::new(source),
            camera: Camera {
                longitude: map.get_longitude() as f64,
                latitude: map.get_latitude() as f64,
                zoom: map.get_zoom() as f64,
            },
            tiles_model,
        }));

        // Wire the pan callback: pixel delta → camera shift in lat/lon.
        // All math lives in `crate::camera::pan` so it can be unit-tested.
        {
            let inner_cb = Rc::clone(&inner);
            map.on_pan(move |dx, dy| {
                {
                    let mut i = inner_cb.borrow_mut();
                    let tile_size = i.source.tile_size();
                    let (lon, lat) = camera_pan(
                        i.camera.longitude,
                        i.camera.latitude,
                        i.camera.zoom,
                        dx as f64,
                        dy as f64,
                        tile_size,
                    );
                    i.camera.longitude = lon;
                    i.camera.latitude = lat;
                }
                refresh_inner(&inner_cb);
            });
        }

        // Wire the zoom-by callback: scroll delta + anchor → camera
        // zoom + re-centre so the anchor stays under the cursor.
        {
            let inner_cb = Rc::clone(&inner);
            map.on_zoom_by(move |delta, anchor_x, anchor_y| {
                {
                    let mut i = inner_cb.borrow_mut();
                    let Some(map) = i.map.upgrade() else { return };
                    let (vw, vh) = logical_size(&map);
                    let (lon, lat, z) = camera_zoom_anchored(
                        i.camera.longitude,
                        i.camera.latitude,
                        i.camera.zoom,
                        delta as f64,
                        anchor_x as f64,
                        anchor_y as f64,
                        vw,
                        vh,
                        i.source.tile_size(),
                        i.source.min_zoom(),
                        i.source.max_zoom(),
                    );
                    i.camera.longitude = lon;
                    i.camera.latitude = lat;
                    i.camera.zoom = z;
                }
                refresh_inner(&inner_cb);
            });
        }

        let c = MapController { inner };
        c.refresh();
        c
    }

    /// Recompute the visible tiles for the current camera and push
    /// them into the view. Called automatically after every gesture;
    /// call manually after [`set_centre`](Self::set_centre) /
    /// [`set_zoom`](Self::set_zoom) / a viewport resize.
    pub fn refresh(&self) {
        refresh_inner(&self.inner);
    }

    pub fn set_centre(&self, longitude: f64, latitude: f64) {
        {
            let mut i = self.inner.borrow_mut();
            i.camera.longitude = longitude;
            i.camera.latitude = latitude;
        }
        self.refresh();
    }

    pub fn set_zoom(&self, zoom: f64) {
        {
            let mut i = self.inner.borrow_mut();
            let min_z = i.source.min_zoom() as f64;
            let max_z = i.source.max_zoom() as f64;
            i.camera.zoom = zoom.clamp(min_z, max_z);
        }
        self.refresh();
    }

    /// Swap the underlying tile source at runtime — useful for layer
    /// toggling. Triggers an immediate refresh.
    pub fn set_source<S: TileSource + 'static>(&self, source: S) {
        {
            let mut i = self.inner.borrow_mut();
            i.source = Box::new(source);
        }
        self.refresh();
    }
}

// ---- internal helpers ----

/// Recompute visible tiles + push to model. Free function so closures
/// can call it without borrowing through `self`.
fn refresh_inner(inner: &Rc<RefCell<Inner>>) {
    let i = inner.borrow();
    let Some(map) = i.map.upgrade() else { return };

    // Mirror camera state out to the Slint properties.
    map.set_longitude(i.camera.longitude as f32);
    map.set_latitude(i.camera.latitude as f32);
    map.set_zoom(i.camera.zoom as f32);

    let (vw, vh) = logical_size(&map);
    if vw <= 0.0 || vh <= 0.0 {
        return; // Window not laid out yet.
    }

    let placed = visible_tiles(
        i.camera.longitude,
        i.camera.latitude,
        i.camera.zoom,
        vw,
        vh,
        i.source.tile_size(),
    );

    // Replace the entire model. For 30-50 tiles this is cheap; if it
    // becomes a hot path we can diff against the existing model.
    let mut new_rows: Vec<SlintTile> = Vec::with_capacity(placed.len());
    for p in placed {
        if let Some(image) = i.source.tile(p.key) {
            new_rows.push(SlintTile {
                x: p.x,
                y: p.y,
                size: p.size,
                image,
            });
        }
        // If `tile()` returned None, just skip — the source will have
        // started any background fetch internally; next refresh picks
        // it up.
    }
    // `set_vec` swaps the whole backing storage and emits one reset
    // event — cheaper than per-row diffing for a full repaint.
    i.tiles_model.set_vec(new_rows);
}

/// Read the MapView Window's current logical (DPI-independent) size.
/// We can't use `get_width`/`get_height` accessors because slint
/// doesn't generate them for inherited `Window` properties; route
/// through the underlying `slint::Window` instead.
fn logical_size(map: &MapView) -> (f64, f64) {
    let w = map.window();
    let phys = w.size();
    let scale = w.scale_factor() as f64;
    let scale = if scale == 0.0 { 1.0 } else { scale };
    (phys.width as f64 / scale, phys.height as f64 / scale)
}
