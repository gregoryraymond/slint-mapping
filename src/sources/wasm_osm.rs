//! [`WasmOsmTileSource`] — browser-side OSM tile source.
//!
//! The native [`crate::sources::OsmTileSource`] uses `ureq` +
//! `std::thread`, neither of which work on `wasm32-unknown-unknown`:
//! `ureq` needs sockets the browser sandbox doesn't expose, and
//! `std::thread::spawn` panics unless you set up `wasm-bindgen-rayon`
//! with cross-origin-isolation headers (which GitHub Pages doesn't
//! send).
//!
//! ## Why XMLHttpRequest, not fetch
//!
//! Two earlier iterations of this source used `gloo_net::http::Request`
//! and then `web_sys::fetch`. Both went through the
//! `wasm_bindgen_futures::JsFuture` + `spawn_local` chain, and both
//! triggered an avalanche of runtime errors under Firefox + the
//! current `js-sys 0.3.98`:
//!
//!   - `panicked at .../js-sys/src/futures/mod.rs: RefCell already borrowed`
//!   - `panicked at .../js-sys/src/futures/mod.rs:160: callbacks should be Some`
//!   - `Error: FnOnce called more than once`
//!   - `RuntimeError: memory access out of bounds`
//!   - cascading heap corruption in `dlmalloc`
//!
//! Root cause: `JsFuture::finish` is re-entered while it still holds
//! `RefCell::borrow_mut`. The reentrancy comes from winit-web's event
//! loop polling pending futures during the same microtask that just
//! resolved one of them, which fires the resolve closure a second
//! time. This is a wasm-bindgen-futures × winit interaction bug, not
//! something application code can paper over from inside a fetch
//! callback.
//!
//! XMLHttpRequest sidesteps the whole class of bugs because it
//! predates Promises: its `onload` handler is invoked exactly once,
//! synchronously, when the request completes. No `JsFuture`, no
//! `spawn_local`, no `Promise.then` chain — just a `Closure::once`
//! registered against the XHR object, which we leak via
//! `Closure::forget` for the request's lifetime.
//!
//! ## Threading
//!
//! Single-threaded — `Arc<Mutex<...>>` is used for the shared state
//! to satisfy the [`TileSource`] `Send + Sync` bounds (the locks are
//! effectively no-ops on wasm but compile-time required).
//!
//! ## Cache
//!
//! In-memory only, bounded LRU at 256 tiles (~64 MB at 256×256 RGBA).
//! Without the bound, the cache grew forever as the user panned and
//! eventually froze the browser tab. A persistent IndexedDB backend
//! would be a useful follow-up but isn't worth the dep weight for the
//! current demo — tiles re-download on page reload, which is the same
//! UX as any browser without local storage.

use crate::source::{TileKey, TileSource};
use crate::sources::util::{decode_png_to_buffer, format_url};
use lru::LruCache;
use send_wrapper::SendWrapper;
use slint::{Image, Rgba8Pixel, SharedPixelBuffer};
use std::cell::RefCell;
use std::collections::HashSet;
use std::num::NonZeroUsize;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use wasm_bindgen::closure::Closure;
use wasm_bindgen::JsCast;

pub const OSM_TILE_URL: &str = "https://tile.openstreetmap.org/{z}/{x}/{y}.png";

/// Cap on the in-memory tile cache. Each 256x256 RGBA tile is 256 KB
/// in `SharedPixelBuffer`, so 256 tiles = 64 MB — enough for the
/// whole viewport plus a few screens of pan-ahead at any zoom, while
/// keeping the browser tab's heap bounded. Without this cap the
/// `HashMap` grew forever as the user panned, eventually freezing
/// the tab.
const TILE_CACHE_CAPACITY: usize = 256;

pub struct WasmOsmTileSource {
    memory: Arc<Mutex<LruCache<TileKey, SharedPixelBuffer<Rgba8Pixel>>>>,
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
        let capacity = NonZeroUsize::new(TILE_CACHE_CAPACITY)
            .expect("TILE_CACHE_CAPACITY is a const, never zero");
        Self {
            memory: Arc::new(Mutex::new(LruCache::new(capacity))),
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
    /// the XHR (browser's UI thread), so callers can repaint
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
        // `.get()` on LruCache promotes the entry to most-recently-
        // used as a side effect, so tiles the viewport is actively
        // showing stay cached even when the LRU is at capacity.
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
        // Clone Arc handles for the onload closure...
        let memory = Arc::clone(&self.memory);
        let in_flight_cb = Arc::clone(&self.in_flight);
        let failed_cb = Arc::clone(&self.failed);
        let on_ready = Arc::clone(&self.on_tile_ready);
        // ...and separately for the synchronous send()-failure path
        // below, which the closure can't reach because it's only
        // invoked if the XHR successfully kicks off.
        let in_flight_err = Arc::clone(&self.in_flight);
        let failed_err = Arc::clone(&self.failed);

        // The XHR + onload chain replaces the earlier `spawn_local +
        // JsFuture` chain. The `closure_slot` holds the
        // `Closure<dyn FnMut()>` we registered with the XHR; when the
        // onload fires (exactly once), we take the closure out so
        // wasm-bindgen drops it cleanly. Without that drop the
        // closure would leak — `forget()` on creation transferred
        // ownership to JS, and only an explicit `take()` from the
        // slot inside the handler reclaims it.
        let closure_slot: Rc<RefCell<Option<Closure<dyn FnMut()>>>> =
            Rc::new(RefCell::new(None));

        let xhr = web_sys::XmlHttpRequest::new()
            .expect("XmlHttpRequest::new should succeed in a browser context");
        if xhr.open_with_async("GET", &url, true).is_err() {
            // Treat URL-rejection as a fetch failure so the loop
            // doesn't get stuck waiting for an in_flight entry to
            // resolve.
            failed_err.lock().unwrap().insert(key);
            in_flight_err.lock().unwrap().remove(&key);
            return None;
        }
        xhr.set_response_type(web_sys::XmlHttpRequestResponseType::Arraybuffer);

        let xhr_for_handler = xhr.clone();
        let closure_slot_for_handler = Rc::clone(&closure_slot);
        let handler = Closure::wrap(Box::new(move || {
            // Drop our own Closure handle as the first thing — XHR's
            // onload fires exactly once, so we don't want to leak.
            let _self_drop = closure_slot_for_handler.borrow_mut().take();

            let status = xhr_for_handler.status().unwrap_or(0);
            let success = (200..300).contains(&status);
            let buf = if success {
                xhr_for_handler
                    .response()
                    .ok()
                    .and_then(|resp| resp.dyn_into::<js_sys::ArrayBuffer>().ok())
                    .and_then(|ab| {
                        let view = js_sys::Uint8Array::new(&ab);
                        let mut bytes = vec![0u8; view.length() as usize];
                        view.copy_to(&mut bytes);
                        decode_png_to_buffer(&bytes)
                    })
            } else {
                None
            };

            match buf {
                Some(b) => {
                    // `put()` on LruCache inserts and silently evicts
                    // the least-recently-used entry if at capacity.
                    memory.lock().unwrap().put(key, b);
                }
                None => {
                    failed_cb.lock().unwrap().insert(key);
                }
            }
            in_flight_cb.lock().unwrap().remove(&key);

            // Fire the repaint hook regardless of success — even a
            // failed fetch should let the UI swap its loading
            // placeholder for the (still-blank) failed state.
            let cb = on_ready
                .lock()
                .unwrap()
                .as_ref()
                .map(|w| Arc::clone(&**w));
            if let Some(cb) = cb {
                cb();
            }
        }) as Box<dyn FnMut()>);

        xhr.set_onload(Some(handler.as_ref().unchecked_ref()));
        // Same callback handles `onerror` and `onabort` — both yield
        // a 0 status, which the handler treats as a fetch failure.
        xhr.set_onerror(Some(handler.as_ref().unchecked_ref()));
        xhr.set_ontimeout(Some(handler.as_ref().unchecked_ref()));

        *closure_slot.borrow_mut() = Some(handler);

        if xhr.send().is_err() {
            // send() may reject for cross-origin or invalid-state
            // reasons; clean up the in-flight slot so a subsequent
            // tile() call can retry.
            failed_err.lock().unwrap().insert(key);
            in_flight_err.lock().unwrap().remove(&key);
            closure_slot.borrow_mut().take();
        }

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
        // No-op for now — XHR exposes `.abort()`, but tracking each
        // in-flight request by key would mean stashing the XHR object
        // in a shared map and abort()ing those whose keys aren't in
        // `_keep`. The browser's concurrent-request limit caps the
        // worst-case wasted work to a few in-flight tiles per pan
        // burst, which has been fine in practice.
    }
}
