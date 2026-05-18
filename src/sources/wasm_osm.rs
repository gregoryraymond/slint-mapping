//! [`WasmOsmTileSource`] — browser-side OSM tile source.
//!
//! The native [`crate::sources::OsmTileSource`] uses `ureq` +
//! `std::thread`, neither of which work on `wasm32-unknown-unknown`:
//! `ureq` needs sockets the browser sandbox doesn't expose, and
//! `std::thread::spawn` panics unless you set up `wasm-bindgen-rayon`
//! with cross-origin-isolation headers (which GitHub Pages doesn't
//! send).
//!
//! This source uses `gloo_net::http::Request` for the network call
//! and `wasm_bindgen_futures::spawn_local` to drive each in-flight
//! fetch on the browser's event loop. The whole pipeline is
//! single-threaded — `Arc<Mutex<...>>` is used for the shared state
//! to satisfy the [`TileSource`] `Send + Sync` bounds (the locks are
//! effectively no-ops on wasm but compile-time required).
//!
//! Cache is in-memory only. A persistent IndexedDB backend would be a
//! useful follow-up but isn't worth the dep weight for the
//! current-purpose demo — tiles re-download on page reload, which is
//! the same UX as any browser without local storage.

use crate::source::{TileKey, TileSource};
use crate::sources::util::{decode_png_to_buffer, format_url};
use send_wrapper::SendWrapper;
use slint::{Image, Rgba8Pixel, SharedPixelBuffer};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use wasm_bindgen_futures::spawn_local;

pub const OSM_TILE_URL: &str = "https://tile.openstreetmap.org/{z}/{x}/{y}.png";

pub struct WasmOsmTileSource {
    memory: Arc<Mutex<HashMap<TileKey, SharedPixelBuffer<Rgba8Pixel>>>>,
    in_flight: Arc<Mutex<HashSet<TileKey>>>,
    failed: Arc<Mutex<HashSet<TileKey>>>,
    url_template: String,
    // SendWrapper lets a `!Send` closure (e.g. one that captures
    // `Rc<…>` — common when wiring up Slint state from wasm-demo
    // code) live behind the `Send + Sync` bound the TileSource trait
    // requires. Safe on single-threaded wasm; would panic on the
    // first cross-thread deref, which can't happen on
    // wasm32-unknown-unknown.
    on_tile_ready: Arc<Mutex<Option<SendWrapper<Arc<dyn Fn()>>>>>,
    tile_size: u32,
    min_zoom: u8,
    max_zoom: u8,
}

impl WasmOsmTileSource {
    pub fn new() -> Self {
        Self::with_url(OSM_TILE_URL)
    }

    pub fn with_url(url_template: impl Into<String>) -> Self {
        Self {
            memory: Arc::new(Mutex::new(HashMap::new())),
            in_flight: Arc::new(Mutex::new(HashSet::new())),
            failed: Arc::new(Mutex::new(HashSet::new())),
            url_template: url_template.into(),
            on_tile_ready: Arc::new(Mutex::new(None)),
            tile_size: 256,
            min_zoom: 0,
            max_zoom: 19,
        }
    }

    pub fn with_zoom_range(mut self, min: u8, max: u8) -> Self {
        self.min_zoom = min;
        self.max_zoom = max;
        self
    }

    /// Register a callback fired after each tile finishes fetching +
    /// decoding. The closure runs on the same JS task that completed
    /// the fetch (browser microtask), so callers can repaint
    /// synchronously without going through any equivalent of
    /// `slint::invoke_from_event_loop` (Slint shares the same event
    /// loop on wasm). The closure isn't required to be `Send` /
    /// `Sync` — it's stored behind a `SendWrapper` so consumer code
    /// can capture `Rc<...>` freely.
    pub fn on_tile_ready(&self, cb: impl Fn() + 'static) {
        *self.on_tile_ready.lock().unwrap() = Some(SendWrapper::new(Arc::new(cb)));
    }
}

impl Default for WasmOsmTileSource {
    fn default() -> Self {
        Self::new()
    }
}

impl TileSource for WasmOsmTileSource {
    fn tile(&self, key: TileKey) -> Option<Image> {
        if let Some(buf) = self.memory.lock().unwrap().get(&key).cloned() {
            return Some(Image::from_rgba8(buf));
        }
        if self.failed.lock().unwrap().contains(&key) {
            return None;
        }
        // Dedupe — if a fetch is already in flight for this key, the
        // existing one will populate memory and notify; we don't need
        // a second concurrent request.
        if !self.in_flight.lock().unwrap().insert(key) {
            return None;
        }

        let url = format_url(&self.url_template, key);
        let memory = Arc::clone(&self.memory);
        let in_flight = Arc::clone(&self.in_flight);
        let failed = Arc::clone(&self.failed);
        let on_ready = Arc::clone(&self.on_tile_ready);

        spawn_local(async move {
            match fetch_png_bytes(&url).await {
                Some(bytes) => match decode_png_to_buffer(&bytes) {
                    Some(buf) => {
                        memory.lock().unwrap().insert(key, buf);
                    }
                    None => {
                        failed.lock().unwrap().insert(key);
                    }
                },
                None => {
                    failed.lock().unwrap().insert(key);
                }
            }
            in_flight.lock().unwrap().remove(&key);
            // Fire the repaint hook regardless of success — even a
            // failed fetch should let the UI swap its loading
            // placeholder for the (still-blank) failed state. We
            // clone out the inner Arc so the lock isn't held across
            // the user callback.
            let cb = on_ready.lock().unwrap().as_ref().map(|w| Arc::clone(&**w));
            if let Some(cb) = cb {
                cb();
            }
        });
        None
    }

    fn tile_size(&self) -> u32 {
        self.tile_size
    }
    fn min_zoom(&self) -> u8 {
        self.min_zoom
    }
    fn max_zoom(&self) -> u8 {
        self.max_zoom
    }

    fn cancel_all_except(&self, _keep: &HashSet<TileKey>) {
        // No-op for now — `spawn_local` doesn't expose cancellation,
        // so we'd need to add an Atomic flag per in-flight task to
        // make this meaningful. The browser's fetch queue parallelises
        // requests well enough that the cost of letting off-screen
        // tiles complete is small. Revisit if this shows up in
        // profiling.
    }
}

/// One-shot fetch via `gloo_net::http::Request`. Returns `None` on
/// any HTTP-level error (non-2xx, network failure, body read error);
/// callers mark the key as failed and serve a placeholder.
async fn fetch_png_bytes(url: &str) -> Option<Vec<u8>> {
    let resp = gloo_net::http::Request::get(url).send().await.ok()?;
    if !resp.ok() {
        return None;
    }
    resp.binary().await.ok()
}
