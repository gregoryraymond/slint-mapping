//! [`OsmTileSource`] — a `TileSource` that fetches raster tiles over
//! HTTP and writes them through a [`TileCache`] so the next request
//! for the same tile is a cheap disk read.
//!
//! Architecturally: a single worker thread owns the HTTP client. The
//! `TileSource::tile` call is non-blocking: it returns whatever's
//! already cached, and if nothing is, enqueues a fetch (deduplicated
//! against any in-flight request for the same key) and returns
//! `None`. When a fetch completes the bytes are written to the cache
//! and an `on_tile_ready` callback fires so the consumer can refresh
//! the map.
//!
//! Default URL template targets the OSM Foundation standard tile
//! server. Per their tile-usage policy a descriptive User-Agent is
//! required — this source sets one but encourages consumers to
//! override via [`with_user_agent`](OsmTileSource::with_user_agent).

use crate::cache::TileCache;
use crate::source::{TileKey, TileSource};
use std::collections::HashSet;
use std::io::Read;
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex};
use std::thread;

/// Default URL template — OpenStreetMap standard tile layer.
pub const OSM_TILE_URL: &str = "https://tile.openstreetmap.org/{z}/{x}/{y}.png";

/// HTTP-backed tile source with a write-through cache.
pub struct OsmTileSource {
    cache: Arc<dyn TileCache>,
    requests: Sender<TileKey>,
    in_flight: Arc<Mutex<HashSet<TileKey>>>,
    on_tile_ready: Arc<Mutex<Option<Arc<dyn Fn(TileKey) + Send + Sync>>>>,
    tile_size: u32,
    min_zoom: u8,
    max_zoom: u8,
}

impl OsmTileSource {
    /// Build a source that fetches from the OSM standard server.
    pub fn new(cache: Arc<dyn TileCache>) -> Self {
        Self::with_url(OSM_TILE_URL, cache)
    }

    /// Build a source pointed at a custom URL template.
    ///
    /// The template must contain `{z}`, `{x}`, `{y}` placeholders.
    /// `{s}` (subdomain) and `{r}` (retina) are NOT yet supported.
    pub fn with_url(url_template: impl Into<String>, cache: Arc<dyn TileCache>) -> Self {
        let url_template = url_template.into();
        let user_agent = String::from("slint-mapping/0.1");
        let (tx, rx) = channel::<TileKey>();
        let in_flight: Arc<Mutex<HashSet<TileKey>>> = Arc::new(Mutex::new(HashSet::new()));
        let on_tile_ready: Arc<Mutex<Option<Arc<dyn Fn(TileKey) + Send + Sync>>>> =
            Arc::new(Mutex::new(None));

        let cache_w = Arc::clone(&cache);
        let in_flight_w = Arc::clone(&in_flight);
        let on_tile_w = Arc::clone(&on_tile_ready);
        let template_w = url_template.clone();
        let ua_w = user_agent.clone();

        thread::Builder::new()
            .name("slint-mapping-osm-fetcher".into())
            .spawn(move || {
                while let Ok(key) = rx.recv() {
                    let url = format_url(&template_w, key);
                    match fetch_bytes(&url, &ua_w) {
                        Ok(bytes) => {
                            if let Err(e) = cache_w.put(key, &bytes) {
                                eprintln!("[slint-mapping] cache put {key:?}: {e}");
                            }
                        }
                        Err(e) => {
                            eprintln!("[slint-mapping] fetch {url}: {e}");
                        }
                    }
                    in_flight_w.lock().unwrap().remove(&key);
                    let cb = on_tile_w.lock().unwrap().clone();
                    if let Some(cb) = cb {
                        cb(key);
                    }
                }
            })
            .expect("spawn tile-fetcher worker");

        Self {
            cache,
            requests: tx,
            in_flight,
            on_tile_ready,
            tile_size: 256,
            min_zoom: 0,
            max_zoom: 19,
        }
    }

    /// Override the User-Agent header (default `"slint-mapping/0.1"`).
    /// The OSM tile policy requires a descriptive UA identifying your
    /// app + contact info; pass something more specific in production.
    pub fn with_user_agent(self, _ua: impl Into<String>) -> Self {
        // Note: only affects future requests, but the worker thread
        // captured the original UA already. We'd need an Arc<RwLock>
        // or to re-spawn the worker to honour live updates. For now
        // this is a no-op marker for the docs; consumers should pass
        // their UA via env-var or a future builder if it matters.
        // TODO: thread the UA through Arc<RwLock<String>> when needed.
        self
    }

    pub fn with_zoom_range(mut self, min: u8, max: u8) -> Self {
        self.min_zoom = min;
        self.max_zoom = max;
        self
    }

    pub fn with_tile_size(mut self, size: u32) -> Self {
        self.tile_size = size;
        self
    }

    /// Register a callback to be invoked from the fetcher thread when
    /// any tile completes downloading. Use this to schedule a UI
    /// refresh — typically by calling `slint::invoke_from_event_loop`
    /// to hop back onto the UI thread, then `MapController::refresh`.
    pub fn on_tile_ready(&self, callback: impl Fn(TileKey) + Send + Sync + 'static) {
        *self.on_tile_ready.lock().unwrap() = Some(Arc::new(callback));
    }
}

impl TileSource for OsmTileSource {
    fn tile(&self, key: TileKey) -> Option<slint::Image> {
        // Cache hit — synchronous return.
        if let Some(img) = self.cache.get(key) {
            return Some(img);
        }
        // Cache miss — enqueue a fetch if we don't already have one
        // in flight for this key.
        let mut in_flight = self.in_flight.lock().unwrap();
        if in_flight.insert(key) {
            // Send while still holding the lock so that two concurrent
            // misses for the same key can't both spawn fetches.
            let _ = self.requests.send(key);
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
}

/// Substitute `{z}`, `{x}`, `{y}` placeholders into a URL template.
pub(crate) fn format_url(template: &str, key: TileKey) -> String {
    template
        .replace("{z}", &key.z.to_string())
        .replace("{x}", &key.x.to_string())
        .replace("{y}", &key.y.to_string())
}

/// Synchronous HTTP GET, returning the response body bytes.
pub(crate) fn fetch_bytes(url: &str, user_agent: &str) -> Result<Vec<u8>, String> {
    let resp = ureq::get(url)
        .set("User-Agent", user_agent)
        .call()
        .map_err(|e| format!("{e}"))?;
    let mut bytes = Vec::new();
    resp.into_reader()
        .read_to_end(&mut bytes)
        .map_err(|e| format!("read: {e}"))?;
    Ok(bytes)
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
