//! Subwindow integration tests — MapEmbed inside a Window-rooted
//! parent at a non-(0, 0) offset, with chrome around it.
//!
//! Covers the regression where the controller assumed the map's
//! viewport == the window's size. With an embedded MapEmbed the
//! correct viewport is the embed's measured size (smaller than the
//! window), surfaced via `map-viewport-width` / `map-viewport-height`.
//!
//! Two layers of assertion:
//!   1. Structural — the panel reports the right inner-embed size for
//!      a given window size, proving the property forwarding works.
//!   2. Mechanical — wire pan / zoom-by callbacks to the same camera
//!      math the real controller uses, drive them with anchor pixels
//!      in *embed-local* coordinates, and verify the cursor-pinned
//!      zoom invariant still holds when the embed isn't at the
//!      window's origin.

use slint::{ComponentHandle, PhysicalSize};
use slint_mapping::camera::{pan as camera_pan, zoom_anchored as camera_zoom_anchored};
use slint_mapping::viewport::{
    center_for_anchor_at_viewport_px, lonlat_to_viewport_px, viewport_px_to_lonlat,
};
use slint_mapping::MapPanel;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// Per-thread one-shot init for the testing backend. `cargo test` runs
/// tests on individual threads and slint's backend state is
/// thread-local, so this must run in each thread but only once.
fn init_backend() {
    thread_local! {
        static INITED: Cell<bool> = const { Cell::new(false) };
    }
    INITED.with(|done| {
        if !done.get() {
            i_slint_backend_testing::init_no_event_loop();
            done.set(true);
        }
    });
}

/// Construct a MapPanel at a fixed window size. The panel's chrome
/// inset (12 px border on three sides + 32 px titlebar at top) is
/// internal to the component, so the inner embed's size is
/// `(window - 24, window - 56)`.
fn make_panel(window_w: u32, window_h: u32) -> MapPanel {
    init_backend();
    let panel = MapPanel::new().expect("MapPanel::new");
    panel
        .window()
        .set_size(PhysicalSize::new(window_w, window_h));
    panel
}

#[test]
fn panel_reports_inner_embed_size_not_window_size() {
    // Window 600x400 → embed is at (12, 44) sized (576, 344). The
    // controller must use that 576x344, not the window's 600x400.
    let panel = make_panel(600, 400);
    let vw = panel.get_map_viewport_width() as f64;
    let vh = panel.get_map_viewport_height() as f64;
    assert!(
        (vw - 576.0).abs() < 0.5,
        "viewport width should report embed.width=576, got {vw}"
    );
    assert!(
        (vh - 344.0).abs() < 0.5,
        "viewport height should report embed.height=344, got {vh}"
    );
}

#[test]
fn panel_inner_size_scales_with_window_resize() {
    let panel = make_panel(480, 360);
    let vw0 = panel.get_map_viewport_width() as f64;
    let vh0 = panel.get_map_viewport_height() as f64;

    // Resize the window — the inner embed must follow.
    panel.window().set_size(PhysicalSize::new(800, 500));
    let vw1 = panel.get_map_viewport_width() as f64;
    let vh1 = panel.get_map_viewport_height() as f64;

    assert!(
        vw1 > vw0,
        "viewport width should grow on resize: {vw0} → {vw1}"
    );
    assert!(
        vh1 > vh0,
        "viewport height should grow on resize: {vh0} → {vh1}"
    );
    assert!((vw1 - 776.0).abs() < 0.5, "expected 800-24=776, got {vw1}");
    assert!((vh1 - 444.0).abs() < 0.5, "expected 500-56=444, got {vh1}");
}

/// Wire the panel's `pan` + `zoom-by` callbacks to the same camera
/// math the standalone MapController uses, reading the panel's
/// surfaced viewport size for the projection step. Returns a closure
/// the test can call to manually drive a refresh-like sync; everything
/// it touches lives in the closures' captured state.
fn wire_controller(panel: &MapPanel) {
    // Authoritative camera. Slint mirrors are updated each event.
    let camera = Rc::new(RefCell::new((
        panel.get_longitude() as f64,
        panel.get_latitude() as f64,
        panel.get_zoom() as f64,
    )));
    // Reasonable defaults — exercise zoom at a non-trivial level.
    *camera.borrow_mut() = (-0.1276, 51.5074, 12.0);
    panel.set_longitude(-0.1276);
    panel.set_latitude(51.5074);
    panel.set_zoom(12.0);

    let tile_size = 256u32;

    // pan(dx, dy) — pixel delta → camera shift in lat/lon. No viewport
    // needed; the per-pixel→per-tile ratio is in `camera::pan`.
    {
        let camera = Rc::clone(&camera);
        let weak = panel.as_weak();
        panel.on_pan(move |dx, dy| {
            let Some(panel) = weak.upgrade() else { return };
            let mut c = camera.borrow_mut();
            let (lon, lat) = camera_pan(c.0, c.1, c.2, dx as f64, dy as f64, tile_size);
            c.0 = lon;
            c.1 = lat;
            panel.set_longitude(lon as f32);
            panel.set_latitude(lat as f32);
        });
    }

    // zoom-by(delta, anchor-x, anchor-y) — anchor coords are
    // embed-local (TouchArea-local), NOT window-local. Crucial: the
    // viewport size handed to `zoom_anchored` is the panel's
    // map-viewport-{width,height}, NOT the window size.
    {
        let camera = Rc::clone(&camera);
        let weak = panel.as_weak();
        panel.on_zoom_by(move |delta, anchor_x, anchor_y| {
            let Some(panel) = weak.upgrade() else { return };
            let vw = panel.get_map_viewport_width() as f64;
            let vh = panel.get_map_viewport_height() as f64;
            let mut c = camera.borrow_mut();
            let (lon, lat, z) = camera_zoom_anchored(
                c.0,
                c.1,
                c.2,
                delta as f64,
                anchor_x as f64,
                anchor_y as f64,
                vw,
                vh,
                tile_size,
                0,
                22,
            );
            c.0 = lon;
            c.1 = lat;
            c.2 = z;
            panel.set_longitude(lon as f32);
            panel.set_latitude(lat as f32);
            panel.set_zoom(z as f32);
        });
    }
}

#[test]
fn zoom_anchored_at_embed_centre_does_not_move_camera() {
    let panel = make_panel(600, 400);
    wire_controller(&panel);

    let lon0 = panel.get_longitude() as f64;
    let lat0 = panel.get_latitude() as f64;

    // Anchor at the embed's centre — should be a pure zoom with no
    // camera shift. Embed is 576x344, so its centre is (288, 172) in
    // embed-local pixels. This is what `self.mouse-x` would report
    // for a cursor sitting in the geometric middle of the map area.
    panel.invoke_zoom_by(1.0, 288.0, 172.0);

    let lon1 = panel.get_longitude() as f64;
    let lat1 = panel.get_latitude() as f64;
    assert!(
        (lon1 - lon0).abs() < 1e-4,
        "centre-anchored zoom: lon should not move, {lon0} → {lon1}",
    );
    assert!(
        (lat1 - lat0).abs() < 1e-4,
        "centre-anchored zoom: lat should not move, {lat0} → {lat1}",
    );
    assert!((panel.get_zoom() as f64 - 13.0).abs() < 1e-6);
}

#[test]
fn zoom_anchored_at_embed_corner_pins_cursor_pixel() {
    // The full end-to-end invariant for the subwindow case: capture
    // the geographic point under a corner-anchored cursor before
    // zoom, run the zoom callback the panel exposes, then re-project
    // that same geographic point through the panel's *measured*
    // viewport and confirm it lands at the same embed-local pixel.
    let panel = make_panel(600, 400);
    wire_controller(&panel);

    let vw = panel.get_map_viewport_width() as f64; // 576
    let vh = panel.get_map_viewport_height() as f64; // 344
    let cursor = (24.0_f64, 18.0_f64); // top-left of embed area
    let tile_size = 256_u32;

    let lon_before = panel.get_longitude() as f64;
    let lat_before = panel.get_latitude() as f64;
    let zoom_before = panel.get_zoom() as f64;

    let (anchor_lon, anchor_lat) = viewport_px_to_lonlat(
        cursor.0,
        cursor.1,
        lon_before,
        lat_before,
        zoom_before,
        vw,
        vh,
        tile_size,
    );

    panel.invoke_zoom_by(1.0, cursor.0 as f32, cursor.1 as f32);

    let lon_after = panel.get_longitude() as f64;
    let lat_after = panel.get_latitude() as f64;
    let zoom_after = panel.get_zoom() as f64;
    assert!((zoom_after - (zoom_before + 1.0)).abs() < 1e-6);

    // Reproject the anchor geo through the *new* camera + *measured*
    // viewport. If anything along the chain used a wrong viewport
    // (window 600x400 instead of embed 576x344) this would drift.
    let (rpx, rpy) = lonlat_to_viewport_px(
        anchor_lon, anchor_lat, lon_after, lat_after, zoom_after, vw, vh, tile_size,
    );
    assert!(
        (rpx - cursor.0).abs() < 1.0,
        "cursor x should pin after zoom, want {} got {rpx}",
        cursor.0,
    );
    assert!(
        (rpy - cursor.1).abs() < 1.0,
        "cursor y should pin after zoom, want {} got {rpy}",
        cursor.1,
    );

    // And confirm the new centre matches what
    // `center_for_anchor_at_viewport_px` would directly compute. Slint
    // panel properties are `float` (f32) so we lose ~1e-5 precision
    // on the round-trip — 1e-3 degrees (~100 m at this zoom) is well
    // within both f32 representable resolution and any visible drift.
    let (expected_lon, expected_lat) = center_for_anchor_at_viewport_px(
        anchor_lon, anchor_lat, cursor.0, cursor.1, zoom_after, vw, vh, tile_size,
    );
    assert!(
        (lon_after - expected_lon).abs() < 1e-3,
        "centre lon off: expected {expected_lon}, got {lon_after}",
    );
    assert!(
        (lat_after - expected_lat).abs() < 1e-3,
        "centre lat off: expected {expected_lat}, got {lat_after}",
    );
}

#[test]
fn pan_through_panel_callback_moves_camera() {
    // Sanity: the panel's pan callback forwards through to the embed
    // and the camera updates. Independent of the zoom math but a
    // structural check that the offset embed receives gestures.
    let panel = make_panel(600, 400);
    wire_controller(&panel);

    let lon_before = panel.get_longitude() as f64;
    // Drag right by 100 embed-local pixels → camera should move west
    // (longitude decrease).
    panel.invoke_pan(100.0, 0.0);
    let lon_after = panel.get_longitude() as f64;
    assert!(
        lon_after < lon_before,
        "drag right should decrease longitude, {lon_before} → {lon_after}",
    );
}
