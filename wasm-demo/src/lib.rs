//! Browser-side entry for the slint-mapping live demo.
//!
//! Build with `wasm-pack build --release --target web wasm-demo`,
//! then serve `wasm-demo/web/index.html` against the `pkg/` it emits.
//! The GitHub Actions workflow at `.github/workflows/pages.yml` does
//! exactly that and publishes the result to GitHub Pages.
//!
//! The demo shows:
//!   - live OSM tiles via `WasmOsmTileSource` (in-memory cache,
//!     gloo-net fetch, `wasm_bindgen_futures::spawn_local` per tile)
//!   - pan + cursor-anchored scroll-zoom
//!   - three hardcoded markers (London POIs)
//!   - one hardcoded polyline traced along the Thames
//!
//! Routing is intentionally out of scope here — every additional
//! crate dep widens the .wasm bundle, and the polyline overlay
//! already exercises the route-rendering code path. A future demo
//! that actually fetches via `OsrmRouter` would just swap the
//! hardcoded coord list for the parsed `Route::geometry`.

use slint::{Color, ComponentHandle, Image, ModelRc, SharedString, VecModel};
use slint_mapping::camera::{pan as camera_pan, zoom_anchored as camera_zoom_anchored};
use slint_mapping::source::TileSource;
use slint_mapping::sources::WasmOsmTileSource;
use slint_mapping::viewport::{lonlat_to_viewport_px, visible_tiles};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

slint::include_modules!();

// Weak handle to the live Demo window, stored at run() time. Lets the
// JS-side pinch detector reach into the Slint scene and invoke the
// existing zoom-by callback directly, instead of trying to round-trip
// the gesture through synthetic WheelEvents (which Slint's TouchArea
// doesn't always pick up on wasm). Thread-local because we're
// single-threaded on wasm32 and slint::Weak isn't Send anyway.
#[cfg(target_arch = "wasm32")]
thread_local! {
    static DEMO_HANDLE: RefCell<Option<slint::Weak<Demo>>> = const { RefCell::new(None) };
}

/// Authoritative camera state mirrored into the Slint `Demo` window's
/// `latitude` / `longitude` / `zoom` properties on every refresh.
#[derive(Clone, Copy)]
struct Camera {
    longitude: f64,
    latitude: f64,
    zoom: f64,
}

/// One hardcoded marker, projected from `(lon, lat)` to viewport px
/// at refresh time.
struct DemoMarker {
    lon: f64,
    lat: f64,
    colour: Color,
}

/// `#[wasm_bindgen(start)]` makes this run automatically when the
/// `init()` JS shim resolves — no separate `main()` invocation
/// needed from the page's script tag.
#[cfg_attr(target_arch = "wasm32", wasm_bindgen(start))]
pub fn run() {
    // Surface Rust panics in the browser console with a real
    // stack trace rather than the default opaque "unreachable
    // executed" wasm trap. Cheap; always wanted in dev + prod.
    #[cfg(target_arch = "wasm32")]
    console_error_panic_hook::set_once();

    let demo = Demo::new().expect("create Demo window");

    // Stash a weak handle so the wasm-exported `pinch_zoom` below can
    // reach back into the scene from a JS-driven gesture without
    // having to round-trip through the slint event loop or our
    // shared-state refresh closure.
    #[cfg(target_arch = "wasm32")]
    DEMO_HANDLE.with(|h| *h.borrow_mut() = Some(demo.as_weak()));

    // Centre on London at z=12 so tiles render straight away over a
    // recognisable city. Mirrored to Slint properties at the bottom
    // of this function via the initial refresh.
    let camera = Rc::new(RefCell::new(Camera {
        longitude: -0.1276,
        latitude: 51.5074,
        zoom: 12.0,
    }));

    let source: Arc<WasmOsmTileSource> = Arc::new(WasmOsmTileSource::new());

    // Three London POIs. Colours are chosen to show that per-marker
    // tinting works (red / blue / green) — they're rendered as
    // filled-circle pins because we don't bundle icon SVGs in the
    // wasm binary (every byte counts; circles read fine at this
    // scale).
    let markers: Rc<Vec<DemoMarker>> = Rc::new(vec![
        DemoMarker {
            lon: -0.1346,
            lat: 51.5099,
            colour: Color::from_rgb_u8(0xEF, 0x44, 0x44),
        },
        DemoMarker {
            lon: -0.1262,
            lat: 51.5194,
            colour: Color::from_rgb_u8(0x3B, 0x82, 0xF6),
        },
        DemoMarker {
            lon: -0.0014,
            lat: 51.4769,
            colour: Color::from_rgb_u8(0x22, 0xC5, 0x5E),
        },
    ]);

    // Sketched polyline approximating the Thames through central
    // London. Six points is enough to show that fractional zoom
    // keeps the line registered against the tile features beneath
    // it; a real routing response would have hundreds of points
    // along the road network.
    let polyline_coords: Rc<Vec<(f64, f64)>> = Rc::new(vec![
        (-0.1900, 51.4870),
        (-0.1500, 51.4910),
        (-0.1100, 51.5030),
        (-0.0700, 51.5050),
        (-0.0300, 51.4980),
        (0.0000, 51.4880),
    ]);

    // The refresh closure recomputes (visible tiles + projected
    // markers + projected polyline) for the current camera and
    // pushes them into the Slint models. Wrapped in `Rc` so the
    // pan / zoom-by / on_tile_ready handlers can all share it.
    let refresh: Rc<dyn Fn()> = {
        let demo_weak = demo.as_weak();
        let camera = Rc::clone(&camera);
        let source = Arc::clone(&source);
        let markers = Rc::clone(&markers);
        let polyline_coords = Rc::clone(&polyline_coords);
        Rc::new(move || {
            let Some(demo) = demo_weak.upgrade() else {
                return;
            };
            let cam = *camera.borrow();
            // Mirror camera to Slint properties so any in-scene UI
            // (e.g. a coords readout) sees the live values.
            demo.set_longitude(cam.longitude as f32);
            demo.set_latitude(cam.latitude as f32);
            demo.set_zoom(cam.zoom as f32);

            let vw = demo.get_map_viewport_width() as f64;
            let vh = demo.get_map_viewport_height() as f64;
            // Window not laid out yet — Slint reports 0 / 0 before
            // the first frame. Skip; the next layout pass will
            // trigger another refresh via the resize callback /
            // on_tile_ready fire.
            if vw <= 0.0 || vh <= 0.0 {
                return;
            }

            // ---- Tile model ----
            let placed = visible_tiles(
                cam.longitude,
                cam.latitude,
                cam.zoom,
                vw,
                vh,
                source.tile_size(),
            );
            let tiles: Vec<Tile> = placed
                .into_iter()
                .map(|p| Tile {
                    x: p.x,
                    y: p.y,
                    size: p.size,
                    // `tile()` may return None on a fresh fetch —
                    // emit the placeholder image (default Image has
                    // width 0, which map.slint checks for the
                    // loading visual) so the slot still paints.
                    image: source.tile(p.key).unwrap_or_default(),
                })
                .collect();
            demo.set_tiles(ModelRc::new(VecModel::from(tiles)));

            // ---- Layer model (markers + polylines, one layer) ----
            let marker_rows: Vec<Marker> = markers
                .iter()
                .map(|m| {
                    let (x, y) = lonlat_to_viewport_px(
                        m.lon,
                        m.lat,
                        cam.longitude,
                        cam.latitude,
                        cam.zoom,
                        vw,
                        vh,
                        source.tile_size(),
                    );
                    Marker {
                        x: x as f32,
                        y: y as f32,
                        size: 18.0,
                        colour: m.colour,
                        // No icon — falls back to the circle visual.
                        icon: Image::default(),
                    }
                })
                .collect();

            let mut commands = String::with_capacity(polyline_coords.len() * 16);
            for (i, (lon, lat)) in polyline_coords.iter().enumerate() {
                let (x, y) = lonlat_to_viewport_px(
                    *lon,
                    *lat,
                    cam.longitude,
                    cam.latitude,
                    cam.zoom,
                    vw,
                    vh,
                    source.tile_size(),
                );
                if i == 0 {
                    commands.push_str(&format!("M {x:.1} {y:.1}"));
                } else {
                    commands.push_str(&format!(" L {x:.1} {y:.1}"));
                }
            }
            let polyline = Polyline {
                commands: SharedString::from(commands),
                colour: Color::from_argb_u8(0xCC, 0x06, 0xB6, 0xD4),
                width: 4.0,
            };

            let layer = Layer {
                markers: ModelRc::new(VecModel::from(marker_rows)),
                polylines: ModelRc::new(VecModel::from(vec![polyline])),
            };
            demo.set_layers(ModelRc::new(VecModel::from(vec![layer])));
        })
    };

    // ---- pan ----
    {
        let camera = Rc::clone(&camera);
        let source = Arc::clone(&source);
        let refresh = Rc::clone(&refresh);
        demo.on_pan(move |dx, dy| {
            {
                let mut c = camera.borrow_mut();
                let (lon, lat) = camera_pan(
                    c.longitude,
                    c.latitude,
                    c.zoom,
                    dx as f64,
                    dy as f64,
                    source.tile_size(),
                );
                c.longitude = lon;
                c.latitude = lat;
            }
            refresh();
        });
    }

    // ---- zoom-by (cursor-anchored) ----
    {
        let camera = Rc::clone(&camera);
        let source = Arc::clone(&source);
        let refresh = Rc::clone(&refresh);
        let demo_weak = demo.as_weak();
        demo.on_zoom_by(move |delta, ax, ay| {
            let Some(demo) = demo_weak.upgrade() else {
                return;
            };
            let vw = demo.get_map_viewport_width() as f64;
            let vh = demo.get_map_viewport_height() as f64;
            {
                let mut c = camera.borrow_mut();
                let (lon, lat, z) = camera_zoom_anchored(
                    c.longitude,
                    c.latitude,
                    c.zoom,
                    delta as f64,
                    ax as f64,
                    ay as f64,
                    vw,
                    vh,
                    source.tile_size(),
                    source.min_zoom(),
                    source.max_zoom(),
                );
                c.longitude = lon;
                c.latitude = lat;
                c.zoom = z;
            }
            refresh();
        });
    }

    // ---- viewport-changed → refresh ----
    //
    // First call to refresh() below runs before the browser has
    // laid out the canvas (Slint reports 0×0 until the first frame
    // is committed). The `changed` hooks in demo.slint re-fire
    // viewport-changed once layout produces a real size, which is
    // what actually paints the map. Also covers browser-window
    // resize after that.
    {
        let refresh = Rc::clone(&refresh);
        demo.on_viewport_changed(move || refresh());
    }

    // ---- tile-ready → refresh ----
    //
    // Browser is single-threaded so the spawn_local task that
    // completes a fetch is on the same JS thread as Slint —
    // calling `refresh()` here is safe without any cross-thread
    // dispatch. The closure captures `Rc<refresh>` which is !Send,
    // so we rely on the SendWrapper inside WasmOsmTileSource to
    // store it.
    {
        let refresh = Rc::clone(&refresh);
        source.on_tile_ready(move || {
            refresh();
        });
    }

    // First paint: enqueues the initial viewport's tile fetches and
    // pushes the markers / polyline immediately (those don't need
    // network).
    refresh();

    // On wasm, `.run()` hands off to winit's web backend which
    // schedules the Slint event loop via requestAnimationFrame.
    // The function returns immediately on wasm32 (browser owns the
    // event loop from here); on a native dev build it blocks like
    // any other Slint app.
    demo.run().expect("run Slint event loop");
}

/// Drive the same `zoom-by` callback the wheel handler would, but
/// from JavaScript. Used by `wasm-demo/web/index.html`'s pinch
/// detector — synthesised wheel events don't reliably reach Slint's
/// TouchArea on wasm, so we cut out the middleman.
///
/// Arguments are: zoom-level delta (positive == zoom in, negative ==
/// out, one unit == one tile-zoom level), and anchor pixel in
/// canvas-local CSS coordinates.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn pinch_zoom(delta: f32, anchor_x: f32, anchor_y: f32) -> f32 {
    DEMO_HANDLE.with(|h| {
        let Some(weak) = h.borrow().clone() else {
            return -1.0;
        };
        let Some(demo) = weak.upgrade() else {
            return -2.0;
        };
        demo.invoke_zoom_by(delta, anchor_x, anchor_y);
        demo.window().request_redraw();
        demo.get_zoom()
    })
}

/// Debug accessor: current camera zoom level. Exposed to JS so the
/// Playwright verification can confirm pinch_zoom actually moved the
/// camera (independent of whether the canvas has visually repainted).
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn current_zoom() -> f32 {
    DEMO_HANDLE.with(|h| {
        h.borrow()
            .as_ref()
            .and_then(|w| w.upgrade())
            .map(|d| d.get_zoom())
            .unwrap_or(-1.0)
    })
}
