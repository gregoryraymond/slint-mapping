//! [`OsmTileSource`] — non-blocking, cancellable, debounced HTTP tile
//! source backed by an in-memory + on-disk cache.
//!
//! Four threads cooperate (none of them the UI):
//!
//! ```text
//!   UI thread          fetch workers (×2)      decode worker      notifier
//!   ─────────          ──────────────────      ─────────────      ────────
//!   tile(key) ──► memory hit / push to fetch OR decode queue / return None
//!   pan      ───► cancel_all_except(visible_keys) on fetch queue
//!                            │
//!                            ▼
//!                       drain fetch queue
//!                            │
//!                            ▼
//!                       respect per-worker
//!                       request_interval gap
//!                            │
//!                            ▼
//!                       HTTP fetch
//!                            │
//!                            ▼
//!                       cache.put + push
//!                       Decode(key, bytes) ───────► drain decode queue
//!                                                          │
//!                                                          ▼
//!                                                     PNG → SharedPixelBuffer
//!                                                          │
//!                                                          ▼
//!                                                     memory.insert
//!                                                     completions.fetch_add
//!                                                     notifier.unpark ─────► wake
//!                                                                              │
//!                                                                              ▼
//!                                                                       sleep(settle_ms)
//!                                                                              │
//!                                                                              ▼
//!                                                                       cb()  ◄ ONE
//!                                                                              callback
//!                                                                              per burst
//! ```
//!
//! Two queues so I/O (HTTP) and CPU (PNG decode) overlap: a fetch
//! worker can be on the wire pulling tile N+1 while the decode worker
//! is unpacking tile N. With 2 fetch workers + per-worker politeness
//! gap, OSM sees up to ~6 req/s per source — well under their "no
//! heavy use" cap (their policy allows 2 concurrent connections).
//!
//! The fetcher workers pop keys from a shared deque (not an mpsc
//! channel) so the UI thread can `cancel_all_except(visible_keys)` to
//! drop queued-but-not-started fetches for tiles no longer on-screen.
//! Decode jobs are not cancelled — the bytes are already in hand,
//! decoding is cheap (~1 ms), and the result lands in the memory
//! cache for free next time the consumer pans back.
//!
//! The notifier thread coalesces completions: after the first
//! completion in a burst it sleeps `settle_ms` (default 25ms), then
//! fires `on_tile_ready` once for the whole burst. Eliminates the
//! N-callbacks-per-pan-sweep problem at the library level — consumers
//! get exactly one notification per quiescent period.

use crate::cache::TileCache;
use crate::source::{TileKey, TileSource};
use crate::sources::util::{decode_png_to_buffer, format_url};
use slint::{Image, Rgba8Pixel, SharedPixelBuffer};
use std::collections::{HashMap, HashSet, VecDeque};
use std::io::Read;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// Shared, thread-safe slot for an optional zero-arg callback fired
/// when a tile finishes loading + decoding. Wrapped in `Arc<Mutex<..>>`
/// so the worker threads can swap it without locking the holder; the
/// inner `Arc<dyn Fn>` lets the worker clone the closure out before
/// calling it (no callback executes while the mutex is held).
type TileReadyCallback = Arc<Mutex<Option<Arc<dyn Fn() + Send + Sync>>>>;

pub const OSM_TILE_URL: &str = "https://tile.openstreetmap.org/{z}/{x}/{y}.png";

/// Default number of parallel HTTP workers. OSM's tile usage policy
/// allows up to 2 concurrent connections per IP; this matches.
const DEFAULT_FETCH_WORKERS: usize = 2;

/// One job for the decode worker. `bytes: None` means "read from
/// cache" — used when `tile()` finds a cache hit and pushes directly
/// to the decode queue without going through the network worker.
struct DecodeJob {
    key: TileKey,
    bytes: Option<Vec<u8>>,
}

/// Generic shared queue with cancellation support. Used twice in this
/// module — once for fetch keys, once for decode jobs.
struct WorkQueue<T> {
    items: Mutex<VecDeque<T>>,
    cond: Condvar,
    /// Signals shutdown — set when the source is dropped so worker
    /// threads exit instead of blocking forever on the Condvar.
    shutdown: Mutex<bool>,
}

impl<T> WorkQueue<T> {
    fn new() -> Self {
        Self {
            items: Mutex::new(VecDeque::new()),
            cond: Condvar::new(),
            shutdown: Mutex::new(false),
        }
    }

    fn push(&self, t: T) {
        self.items.lock().unwrap().push_back(t);
        self.cond.notify_one();
    }

    /// Block until an item is available or the queue is shut down.
    fn pop(&self) -> Option<T> {
        let mut items = self.items.lock().unwrap();
        loop {
            if let Some(t) = items.pop_front() {
                return Some(t);
            }
            if *self.shutdown.lock().unwrap() {
                return None;
            }
            items = self.cond.wait(items).unwrap();
        }
    }

    /// Drop every queued item for which `keep` returns false. Returns
    /// the number dropped.
    fn retain<F: FnMut(&T) -> bool>(&self, mut keep: F) -> usize {
        let mut items = self.items.lock().unwrap();
        let before = items.len();
        items.retain(|t| keep(t));
        before - items.len()
    }

    fn shutdown(&self) {
        *self.shutdown.lock().unwrap() = true;
        self.cond.notify_all();
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.items.lock().unwrap().len()
    }
}

pub struct OsmTileSource {
    cache: Arc<dyn TileCache>,
    memory: Arc<Mutex<HashMap<TileKey, SharedPixelBuffer<Rgba8Pixel>>>>,
    /// Keys whose fetch+decode pipeline produced no usable image (404,
    /// network error, malformed PNG). Re-queries skip the network until
    /// `clear_failed()` is called or the source is rebuilt.
    failed: Arc<Mutex<HashSet<TileKey>>>,
    /// HTTP User-Agent header sent on each fetch. Shared mutable so
    /// `with_user_agent` can change it after construction without
    /// re-spawning the fetcher threads.
    user_agent: Arc<Mutex<String>>,
    /// Pending HTTP fetches. Workers respect a per-worker politeness
    /// gap so two workers ≈ 2 × (1 / interval) req/s.
    fetch_queue: Arc<WorkQueue<TileKey>>,
    /// Bytes-in-hand decode jobs (either freshly fetched or cache
    /// hits). Single worker — decode is fast and ordering doesn't
    /// matter, so one thread keeps the model simple.
    decode_queue: Arc<WorkQueue<DecodeJob>>,
    in_flight: Arc<Mutex<HashSet<TileKey>>>,
    on_tile_ready: TileReadyCallback,
    /// Owned for unparking from the worker threads.
    notifier_handle: Mutex<Option<JoinHandle<()>>>,
    /// Owned so we can join on drop (currently we just let the OS reap
    /// when the source is dropped; future improvement).
    fetcher_handles: Mutex<Vec<JoinHandle<()>>>,
    decoder_handle: Mutex<Option<JoinHandle<()>>>,
    tile_size: u32,
    min_zoom: u8,
    max_zoom: u8,
    request_interval: Arc<AtomicU64>,
    settle_ms: Arc<AtomicU64>,
}

impl OsmTileSource {
    pub fn new(cache: Arc<dyn TileCache>) -> Self {
        Self::with_url(OSM_TILE_URL, cache)
    }

    pub fn with_url(url_template: impl Into<String>, cache: Arc<dyn TileCache>) -> Self {
        let url_template = url_template.into();
        // OSM's CDN 404s any UA without a parenthesised contact token —
        // a bare "slint-mapping/0.1" gets blanket-rejected even for
        // valid tile coords. Including a URL in parens satisfies their
        // anti-bot heuristic. Apps embedding this source should override
        // via `with_user_agent` with their own name + contact.
        let user_agent: Arc<Mutex<String>> = Arc::new(Mutex::new(String::from(
            "slint-mapping/0.1 (+https://github.com/slint-ui)",
        )));
        let fetch_queue: Arc<WorkQueue<TileKey>> = Arc::new(WorkQueue::new());
        let decode_queue: Arc<WorkQueue<DecodeJob>> = Arc::new(WorkQueue::new());
        let in_flight = Arc::new(Mutex::new(HashSet::new()));
        // Keys whose fetch returned an error (404 / network / decode).
        // Suppresses re-enqueue from subsequent `tile(key)` calls so a
        // tile that genuinely isn't on the upstream tile server doesn't
        // generate a fetch storm every refresh.
        let failed: Arc<Mutex<HashSet<TileKey>>> = Arc::new(Mutex::new(HashSet::new()));
        let on_tile_ready: TileReadyCallback = Arc::new(Mutex::new(None));
        let memory: Arc<Mutex<HashMap<TileKey, SharedPixelBuffer<Rgba8Pixel>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let request_interval = Arc::new(AtomicU64::new(300));
        let settle_ms = Arc::new(AtomicU64::new(25));
        let completions = Arc::new(AtomicU64::new(0));

        // ---- Notifier thread (debounces on_tile_ready) ----
        let notifier_handle = {
            let on_tile = Arc::clone(&on_tile_ready);
            let completions = Arc::clone(&completions);
            let settle = Arc::clone(&settle_ms);
            let decode_queue = Arc::clone(&decode_queue);
            thread::Builder::new()
                .name("slint-mapping-osm-notifier".into())
                .spawn(move || loop {
                    thread::park();
                    if *decode_queue.shutdown.lock().unwrap() {
                        return;
                    }
                    // Wait a short settle period so we batch up
                    // completions from a burst. New unparks during
                    // sleep are "fused" — park() returns immediately
                    // on the next call.
                    let settle = Duration::from_millis(settle.load(Ordering::Relaxed));
                    thread::sleep(settle);
                    if completions.swap(0, Ordering::SeqCst) == 0 {
                        continue;
                    }
                    let cb = on_tile.lock().unwrap().clone();
                    if let Some(cb) = cb {
                        cb();
                    }
                })
                .expect("spawn notifier thread")
        };

        // ---- Fetcher threads (network only) ----
        let mut fetcher_handles = Vec::with_capacity(DEFAULT_FETCH_WORKERS);
        for worker_id in 0..DEFAULT_FETCH_WORKERS {
            let fetch_queue_w = Arc::clone(&fetch_queue);
            let decode_queue_w = Arc::clone(&decode_queue);
            let cache_w = Arc::clone(&cache);
            let failed_w = Arc::clone(&failed);
            let in_flight_w = Arc::clone(&in_flight);
            let template_w = url_template.clone();
            let ua_w = Arc::clone(&user_agent);
            let interval_w = Arc::clone(&request_interval);
            let completions_w = Arc::clone(&completions);
            let notifier_thread = notifier_handle.thread().clone();
            let handle = thread::Builder::new()
                .name(format!("slint-mapping-osm-fetcher-{worker_id}"))
                .spawn(move || {
                    // Per-worker politeness clock — staggered so the two
                    // workers don't both fire requests at the same
                    // instant on startup. Net effect: ~2 / interval
                    // req/s combined.
                    let mut last_request_at = Instant::now() - Duration::from_secs(60);
                    while let Some(key) = fetch_queue_w.pop() {
                        let interval = Duration::from_millis(interval_w.load(Ordering::Relaxed));
                        let since = last_request_at.elapsed();
                        if since < interval {
                            thread::sleep(interval - since);
                        }
                        last_request_at = Instant::now();
                        let url = format_url(&template_w, key);
                        let ua = ua_w.lock().unwrap().clone();
                        match fetch_bytes(&url, &ua) {
                            Ok(b) => {
                                if let Err(e) = cache_w.put(key, &b) {
                                    eprintln!("[slint-mapping] cache put {key:?}: {e}");
                                }
                                // Hand off bytes to the decode worker —
                                // network thread is now free to start
                                // the next fetch immediately.
                                decode_queue_w.push(DecodeJob {
                                    key,
                                    bytes: Some(b),
                                });
                            }
                            Err(e) => {
                                eprintln!("[slint-mapping] fetch {url}: {e}");
                                // No bytes to decode. Mark failed
                                // directly and notify so the consumer
                                // can re-render (keeps the placeholder
                                // up, but flushes the loading state).
                                failed_w.lock().unwrap().insert(key);
                                in_flight_w.lock().unwrap().remove(&key);
                                completions_w.fetch_add(1, Ordering::SeqCst);
                                notifier_thread.unpark();
                            }
                        }
                    }
                })
                .expect("spawn fetcher thread");
            fetcher_handles.push(handle);
        }

        // ---- Decoder thread (PNG → SharedPixelBuffer, in parallel
        //                      with the network) ----
        let decoder_handle = {
            let decode_queue_w = Arc::clone(&decode_queue);
            let cache_w = Arc::clone(&cache);
            let memory_w = Arc::clone(&memory);
            let failed_w = Arc::clone(&failed);
            let in_flight_w = Arc::clone(&in_flight);
            let completions_w = Arc::clone(&completions);
            let notifier_thread = notifier_handle.thread().clone();
            thread::Builder::new()
                .name("slint-mapping-osm-decoder".into())
                .spawn(move || {
                    while let Some(job) = decode_queue_w.pop() {
                        let DecodeJob { key, bytes } = job;
                        // If the network worker handed us bytes, use
                        // them. Otherwise this is a cache-hit decode
                        // request pushed from `tile()` — pull from
                        // disk now.
                        let bytes = match bytes {
                            Some(b) => Some(b),
                            None => cache_w.get_bytes(key),
                        };
                        match bytes.as_deref().and_then(decode_png_to_buffer) {
                            Some(buf) => {
                                memory_w.lock().unwrap().insert(key, buf);
                                failed_w.lock().unwrap().remove(&key);
                            }
                            None => {
                                failed_w.lock().unwrap().insert(key);
                            }
                        }
                        in_flight_w.lock().unwrap().remove(&key);
                        completions_w.fetch_add(1, Ordering::SeqCst);
                        notifier_thread.unpark();
                    }
                })
                .expect("spawn decoder thread")
        };

        // `completions` is held by every worker + the notifier; the
        // original Arc is dropped here, leaving the thread clones to
        // keep it alive.
        drop(completions);

        Self {
            cache,
            memory,
            failed,
            user_agent,
            fetch_queue,
            decode_queue,
            in_flight,
            on_tile_ready,
            notifier_handle: Mutex::new(Some(notifier_handle)),
            fetcher_handles: Mutex::new(fetcher_handles),
            decoder_handle: Mutex::new(Some(decoder_handle)),
            tile_size: 256,
            min_zoom: 0,
            max_zoom: 19,
            request_interval,
            settle_ms,
        }
    }

    /// Override the HTTP User-Agent sent on tile fetches. OSM's tile
    /// usage policy expects every requesting app to identify itself
    /// with a name + contact (URL or email). Defaults to a generic
    /// `slint-mapping/0.1 (+https://github.com/slint-ui)` — embedding
    /// apps should override with their own.
    pub fn with_user_agent(self, ua: impl Into<String>) -> Self {
        *self.user_agent.lock().unwrap() = ua.into();
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

    /// Minimum milliseconds between HTTP fetches *per worker*. With
    /// the default of 2 workers and `ms = 300`, that's ~6 req/s
    /// combined. Default 300ms.
    pub fn with_request_interval(self, ms: u64) -> Self {
        self.request_interval.store(ms, Ordering::Relaxed);
        self
    }

    /// Notifier debounce window — `on_tile_ready` fires no more than
    /// once per `settle_ms` (default 25ms). Set to 0 for per-tile.
    pub fn with_settle_ms(self, ms: u64) -> Self {
        self.settle_ms.store(ms, Ordering::Relaxed);
        self
    }

    /// Register a callback fired (from the notifier thread, debounced)
    /// when one or more tile fetches/decodes have completed. Wire it
    /// to `slint::invoke_from_event_loop` to repaint your map cells —
    /// one repaint per burst, not per tile.
    pub fn on_tile_ready(&self, callback: impl Fn() + Send + Sync + 'static) {
        *self.on_tile_ready.lock().unwrap() = Some(Arc::new(callback));
    }
}

impl Drop for OsmTileSource {
    fn drop(&mut self) {
        // Both queues need to be told to drain. Worker threads block
        // on the queue's Condvar; shutdown signals + a final notify
        // wakes them so they return cleanly.
        self.fetch_queue.shutdown();
        self.decode_queue.shutdown();
        if let Some(notifier) = self.notifier_handle.lock().unwrap().take() {
            notifier.thread().unpark();
            std::mem::drop(notifier);
        }
        for h in self.fetcher_handles.lock().unwrap().drain(..) {
            std::mem::drop(h);
        }
        if let Some(decoder) = self.decoder_handle.lock().unwrap().take() {
            std::mem::drop(decoder);
        }
    }
}

impl TileSource for OsmTileSource {
    fn tile(&self, key: TileKey) -> Option<slint::Image> {
        if let Some(buf) = self.memory.lock().unwrap().get(&key).cloned() {
            return Some(Image::from_rgba8(buf));
        }
        // A key that's already failed this session is silently
        // swallowed — no re-enqueue, no log spam. The viewer keeps
        // painting its "loading" placeholder for that slot, which is
        // the right visual for "we tried and the upstream doesn't have
        // it".
        if self.failed.lock().unwrap().contains(&key) {
            return None;
        }
        let mut in_flight = self.in_flight.lock().unwrap();
        if !in_flight.insert(key) {
            // Already somewhere in the pipeline; don't double-queue.
            return None;
        }
        drop(in_flight);
        // Disk-cache hit → skip the network worker entirely and push
        // straight to the decoder. Net effect: cold-cache pages take
        // network round-trips, but warm cache (panning back to a tile
        // you've seen before) is just decode-and-paint.
        if self.cache.contains(key) {
            self.decode_queue.push(DecodeJob { key, bytes: None });
        } else {
            self.fetch_queue.push(key);
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

    fn cancel_all_except(&self, keep: &HashSet<TileKey>) {
        // Only the fetch queue is worth cancelling — those haven't
        // touched the network yet. Decode jobs already have bytes in
        // hand and complete in ~1ms, so let them finish and populate
        // the memory cache for free.
        let removed = self.fetch_queue.retain(|key| keep.contains(key));
        if removed > 0 {
            // Drop cancelled keys from in_flight so a future tile()
            // call re-queues the work if the user pans back into view.
            let memory = self.memory.lock().unwrap();
            let mut inflight = self.in_flight.lock().unwrap();
            inflight.retain(|k| keep.contains(k) || memory.contains_key(k));
        }
    }
}

impl OsmTileSource {
    /// Clear the "tried and failed" memo. Use after a network outage
    /// recovers, or to manually force a retry of upstream 404s. Called
    /// rarely — the typical user path is to just leave failed keys
    /// suppressed for the session.
    pub fn clear_failed(&self) {
        self.failed.lock().unwrap().clear();
    }
}

// ---- internal helpers ----

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
        let buf = decode_png_to_buffer(bytes).expect("decode");
        assert_eq!(buf.width(), 256);
        assert_eq!(buf.height(), 256);
    }

    #[test]
    fn failed_key_is_not_re_enqueued() {
        // Build a source pointed at a URL no server will answer, mark
        // a key failed manually, and verify `tile()` returns None
        // without pushing into either queue.
        let cache: Arc<dyn TileCache> = Arc::new(crate::cache::FileTileCache::new(
            std::env::temp_dir().join("slint-mapping-failed-key-test"),
        ));
        let src = OsmTileSource::with_url("http://127.0.0.1:1/{z}/{x}/{y}.png", cache);
        let key = TileKey { x: 1, y: 2, z: 3 };
        src.failed.lock().unwrap().insert(key);

        let before_fetch = src.fetch_queue.len();
        let before_decode = src.decode_queue.len();
        let result = src.tile(key);
        let after_fetch = src.fetch_queue.len();
        let after_decode = src.decode_queue.len();

        assert!(result.is_none());
        assert_eq!(after_fetch, before_fetch, "failed key enqueued for fetch");
        assert_eq!(
            after_decode, before_decode,
            "failed key enqueued for decode"
        );

        src.clear_failed();
        assert!(src.failed.lock().unwrap().is_empty());
    }

    #[test]
    fn cancel_drops_queued_work() {
        let q: WorkQueue<TileKey> = WorkQueue::new();
        q.push(TileKey { x: 1, y: 1, z: 5 });
        q.push(TileKey { x: 2, y: 2, z: 5 });
        q.push(TileKey { x: 3, y: 3, z: 5 });
        let keep: HashSet<TileKey> = [TileKey { x: 2, y: 2, z: 5 }].into_iter().collect();
        let dropped = q.retain(|k| keep.contains(k));
        assert_eq!(dropped, 2);
        assert_eq!(q.len(), 1);
    }
}
