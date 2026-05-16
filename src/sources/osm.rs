//! [`OsmTileSource`] — a `TileSource` that fetches raster tiles over
//! HTTP and decodes them in the background. The UI thread never blocks
//! on either network I/O or PNG decoding.
//!
//! Architecture (everything off the UI thread except the final
//! `Image::from_rgba8` wrap, which is O(1)):
//!
//! ```text
//!   UI thread                   worker thread
//!   ─────────                   ─────────────
//!   tile(key) ─────────────────► request fetch / decode
//!     │                            │
//!     │ if in-memory cache hit:    │ fetch_bytes(url)        (ureq)
//!     │   return Some(Image)       │  ↓
//!     │                            │ cache.put(key, bytes)   (disk)
//!     │ else: returns None         │  ↓
//!     │                            │ image::decode_png       (image crate)
//!     │                            │  ↓
//!     │                            │ SharedPixelBuffer::new + copy
//!     │                            │  ↓
//!     │  ◄──── on_tile_ready ──────│ memory.insert(key, buf)
//!     │                            │
//!     │ tile(key) again            │
//!     │   returns Some(Image)      │
//! ```
//!
//! `SharedPixelBuffer<Rgba8Pixel>` is `Send + Sync` (it's backed by an
//! `Arc<[Rgba8Pixel]>` internally), so the worker can decode and
//! insert into the shared cache. `slint::Image::from_rgba8` on the UI
//! thread is just a cheap wrapper around the buffer.

use crate::cache::TileCache;
use crate::source::{TileKey, TileSource};
use slint::{Image, Rgba8Pixel, SharedPixelBuffer};
use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex};
use std::thread;

/// Default URL template — OpenStreetMap standard tile layer.
pub const OSM_TILE_URL: &str = "https://tile.openstreetmap.org/{z}/{x}/{y}.png";

/// One unit of work for the worker thread.
enum Job {
    /// Fetch over HTTP + write to disk cache + decode + insert in memory.
    Fetch(TileKey),
    /// Tile is already in the disk cache — just decode + insert in memory.
    Decode(TileKey),
}

/// HTTP-backed tile source with a write-through disk cache + in-memory
/// decoded-image cache. UI calls into `tile()` are non-blocking.
pub struct OsmTileSource {
    cache: Arc<dyn TileCache>,
    memory: Arc<Mutex<HashMap<TileKey, SharedPixelBuffer<Rgba8Pixel>>>>,
    requests: Sender<Job>,
    in_flight: Arc<Mutex<HashSet<TileKey>>>,
    on_tile_ready: Arc<Mutex<Option<Arc<dyn Fn(TileKey) + Send + Sync>>>>,
    tile_size: u32,
    min_zoom: u8,
    max_zoom: u8,
    request_interval: Arc<std::sync::atomic::AtomicU64>,
}

impl OsmTileSource {
    /// Build a source that fetches from the OSM standard server.
    pub fn new(cache: Arc<dyn TileCache>) -> Self {
        Self::with_url(OSM_TILE_URL, cache)
    }

    /// Build a source pointed at a custom URL template.
    /// Template must contain `{z}`, `{x}`, `{y}` placeholders.
    pub fn with_url(url_template: impl Into<String>, cache: Arc<dyn TileCache>) -> Self {
        let url_template = url_template.into();
        let user_agent = String::from("slint-mapping/0.1");
        let (tx, rx) = channel::<Job>();
        let in_flight: Arc<Mutex<HashSet<TileKey>>> = Arc::new(Mutex::new(HashSet::new()));
        let on_tile_ready: Arc<Mutex<Option<Arc<dyn Fn(TileKey) + Send + Sync>>>> =
            Arc::new(Mutex::new(None));
        let memory: Arc<Mutex<HashMap<TileKey, SharedPixelBuffer<Rgba8Pixel>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let request_interval = Arc::new(std::sync::atomic::AtomicU64::new(300));

        let cache_w = Arc::clone(&cache);
        let in_flight_w = Arc::clone(&in_flight);
        let on_tile_w = Arc::clone(&on_tile_ready);
        let memory_w = Arc::clone(&memory);
        let template_w = url_template.clone();
        let ua_w = user_agent.clone();
        let interval_w = Arc::clone(&request_interval);

        thread::Builder::new()
            .name("slint-mapping-osm-worker".into())
            .spawn(move || {
                let mut last_request_at = std::time::Instant::now()
                    - std::time::Duration::from_secs(60);
                while let Ok(job) = rx.recv() {
                    let key = match job {
                        Job::Fetch(k) | Job::Decode(k) => k,
                    };
                    // Resolve bytes: either the disk cache already has
                    // them (Decode), or we need to GET them from the
                    // network (Fetch). For Fetch we throttle to stay
                    // polite under the OSM tile policy.
                    let bytes: Option<Vec<u8>> = match job {
                        Job::Decode(k) => cache_w.get_bytes(k),
                        Job::Fetch(k) => {
                            let interval = std::time::Duration::from_millis(
                                interval_w.load(std::sync::atomic::Ordering::Relaxed),
                            );
                            let since = last_request_at.elapsed();
                            if since < interval {
                                std::thread::sleep(interval - since);
                            }
                            last_request_at = std::time::Instant::now();
                            let url = format_url(&template_w, k);
                            match fetch_bytes(&url, &ua_w) {
                                Ok(b) => {
                                    if let Err(e) = cache_w.put(k, &b) {
                                        eprintln!("[slint-mapping] cache put {k:?}: {e}");
                                    }
                                    Some(b)
                                }
                                Err(e) => {
                                    eprintln!("[slint-mapping] fetch {url}: {e}");
                                    None
                                }
                            }
                        }
                    };
                    // Decode + insert in memory cache. Decoding is the
                    // expensive step we're keeping off the UI thread.
                    if let Some(b) = bytes {
                        if let Some(buf) = decode_png_to_buffer(&b) {
                            memory_w.lock().unwrap().insert(key, buf);
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
            memory,
            requests: tx,
            in_flight,
            on_tile_ready,
            tile_size: 256,
            min_zoom: 0,
            max_zoom: 19,
            request_interval,
        }
    }

    pub fn with_user_agent(self, _ua: impl Into<String>) -> Self {
        // TODO: thread the UA through Arc<RwLock<String>> for live
        // updates. For now this is a no-op — the worker captured the
        // initial UA when it was spawned.
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

    /// Minimum milliseconds between HTTP requests (the OSM tile policy
    /// forbids "heavy use" — 300ms is a reasonable polite default).
    pub fn with_request_interval(self, ms: u64) -> Self {
        self.request_interval
            .store(ms, std::sync::atomic::Ordering::Relaxed);
        self
    }

    /// Register a callback invoked (from the worker thread) when any
    /// tile finishes its fetch+decode pipeline. Typically wired to
    /// `slint::invoke_from_event_loop` to repaint the map cell.
    pub fn on_tile_ready(&self, callback: impl Fn(TileKey) + Send + Sync + 'static) {
        *self.on_tile_ready.lock().unwrap() = Some(Arc::new(callback));
    }
}

impl TileSource for OsmTileSource {
    fn tile(&self, key: TileKey) -> Option<slint::Image> {
        // 1. In-memory hit → wrap and return immediately. The decode
        //    already happened off-thread; this is just a `Rc` clone.
        if let Some(buf) = self.memory.lock().unwrap().get(&key).cloned() {
            return Some(Image::from_rgba8(buf));
        }
        // 2. Disk hit → enqueue a decode job (if not already queued)
        //    and return None for this frame. on_tile_ready will fire
        //    when the decoded buffer lands in memory.
        // 3. Total miss → enqueue a fetch+decode job.
        let job = if self.cache.contains(key) {
            Job::Decode(key)
        } else {
            Job::Fetch(key)
        };
        let mut in_flight = self.in_flight.lock().unwrap();
        if in_flight.insert(key) {
            let _ = self.requests.send(job);
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

// ---- internal helpers (worker-thread only) ----

/// Decode PNG bytes into a `SharedPixelBuffer<Rgba8Pixel>` that the UI
/// thread can wrap into a `slint::Image` for free via `from_rgba8`.
/// Returns `None` on a malformed PNG; the worker logs and moves on.
fn decode_png_to_buffer(bytes: &[u8]) -> Option<SharedPixelBuffer<Rgba8Pixel>> {
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

/// Substitute `{z}`, `{x}`, `{y}` placeholders into a URL template.
pub(crate) fn format_url(template: &str, key: TileKey) -> String {
    template
        .replace("{z}", &key.z.to_string())
        .replace("{x}", &key.x.to_string())
        .replace("{y}", &key.y.to_string())
}

/// Synchronous HTTP GET — returns the response body bytes.
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

    #[test]
    fn decode_png_roundtrips() {
        let bytes = include_bytes!("../../sample-tiles/0/0/0.png");
        let buf = decode_png_to_buffer(bytes).expect("decode the world tile");
        assert_eq!(buf.width(), 256);
        assert_eq!(buf.height(), 256);
    }
}
