//! `slint-mapping-prefetch` — pre-fetch OSM raster tiles covering a
//! geographic bounding box across a zoom range into a local
//! [`FileTileCache`]. Synchronous, polite, progress-reporting.
//!
//! This is the maintainer tool that populates `sample-tiles/` for the
//! demos. App authors should NOT call this binary; they should use
//! the library API directly to build an in-app "download maps for
//! offline use" feature:
//!
//! ```ignore
//! use slint_mapping::cache::FileTileCache;
//! use slint_mapping::prefetch::{self, RegionConfig};
//!
//! let cache = FileTileCache::new(app_cache_dir.join("tiles"));
//! prefetch::region(
//!     &cache,
//!     west, south, east, north, zoom_min, zoom_max,
//!     &RegionConfig::default(),
//!     |i, total, _key, outcome| {
//!         // update your app's progress UI here
//!     },
//! )?;
//! ```
//!
//! Invoke via cargo:
//!
//! ```sh
//! cargo run --bin slint-mapping-prefetch --features http -- \
//!     --dest sample-tiles --zoom 4-12 \
//!     -0.5 51.25 0.3 51.75      # Greater London z=4-12
//! ```

use slint_mapping::cache::FileTileCache;
use slint_mapping::prefetch::{self, FetchOutcome, RegionConfig};
use std::path::PathBuf;

fn main() {
    let args = Args::parse_from(std::env::args().skip(1));

    let cache = FileTileCache::new(&args.dest);
    let config = RegionConfig {
        url_template: args.url.clone(),
        user_agent: args.ua.clone(),
        sleep_ms: args.sleep_ms,
        max_zoom: args.zoom_max.max(prefetch::MAX_BULK_FETCH_ZOOM),
    };

    eprintln!(
        "bbox: lon [{}, {}]  lat [{}, {}]",
        args.lon_min, args.lon_max, args.lat_min, args.lat_max
    );
    eprintln!("zoom: {}-{}", args.zoom_min, args.zoom_max);
    eprintln!("dest: {}", args.dest.display());
    eprintln!("url:  {}", args.url);
    eprintln!();

    let result = prefetch::region(
        &cache,
        args.lon_min,
        args.lat_min,
        args.lon_max,
        args.lat_max,
        args.zoom_min,
        args.zoom_max,
        &config,
        |i, total, key, outcome| match outcome {
            FetchOutcome::Cached => {
                eprint!("\r[{i:4}/{total:4}] cached {}/{}/{}        ", key.z, key.x, key.y)
            }
            FetchOutcome::Fetched { bytes } => eprint!(
                "\r[{i:4}/{total:4}] fetched {}/{}/{} ({} B)      ",
                key.z, key.x, key.y, bytes
            ),
            FetchOutcome::Failed(e) => eprintln!(
                "\n[{i:4}/{total:4}] FAILED {}/{}/{}: {e}",
                key.z, key.x, key.y
            ),
        },
    );

    match result {
        Ok(n) => eprintln!("\n\nDone. {n} tile(s) processed under {}/", args.dest.display()),
        Err(e) => {
            eprintln!("\nerror: {e}");
            std::process::exit(1);
        }
    }
}

/// Minimal manual arg parser — no `clap` dep just for one example.
struct Args {
    lon_min: f64,
    lat_min: f64,
    lon_max: f64,
    lat_max: f64,
    zoom_min: u8,
    zoom_max: u8,
    dest: PathBuf,
    url: String,
    ua: String,
    sleep_ms: u64,
}

impl Args {
    fn parse_from(args: impl Iterator<Item = String>) -> Self {
        let mut dest = PathBuf::from("sample-tiles");
        let mut zoom = "0-12".to_string();
        let mut url = slint_mapping::sources::OSM_TILE_URL.to_string();
        let mut ua = "slint-mapping/0.1 (https://github.com/slint-rs/slint-mapping)".to_string();
        let mut sleep_ms = 300u64;
        let mut positional: Vec<String> = Vec::new();

        let mut it = args.peekable();
        while let Some(a) = it.next() {
            match a.as_str() {
                "--dest" => dest = it.next().expect("--dest takes a path").into(),
                "--zoom" => zoom = it.next().expect("--zoom takes Z_MIN-Z_MAX"),
                "--url" => url = it.next().expect("--url takes a template"),
                "--ua" => ua = it.next().expect("--ua takes a string"),
                "--sleep-ms" => {
                    sleep_ms = it
                        .next()
                        .expect("--sleep-ms takes milliseconds")
                        .parse()
                        .expect("--sleep-ms must be u64")
                }
                "--help" | "-h" => {
                    print_usage();
                    std::process::exit(0);
                }
                _ => positional.push(a),
            }
        }
        if positional.len() != 4 {
            print_usage();
            eprintln!("\nerror: expected 4 positional args (lon_min lat_min lon_max lat_max)");
            std::process::exit(2);
        }
        let parse = |s: &str| s.parse::<f64>().expect("bbox values must be numbers");
        let (zmin, zmax) = match zoom.split_once('-') {
            Some((a, b)) => (a.parse().unwrap(), b.parse().unwrap()),
            None => {
                let z: u8 = zoom.parse().expect("--zoom must be Z or Z_MIN-Z_MAX");
                (z, z)
            }
        };
        Self {
            lon_min: parse(&positional[0]),
            lat_min: parse(&positional[1]),
            lon_max: parse(&positional[2]),
            lat_max: parse(&positional[3]),
            zoom_min: zmin,
            zoom_max: zmax,
            dest,
            url,
            ua,
            sleep_ms,
        }
    }
}

fn print_usage() {
    eprintln!(
        "usage: prefetch [--dest DIR] [--zoom Z_MIN-Z_MAX] [--url TEMPLATE]\n\
         \x20               [--ua STRING] [--sleep-ms MS]\n\
         \x20               LON_MIN LAT_MIN LON_MAX LAT_MAX\n\
         \n\
         Pre-fetch raster tiles covering a bbox into a FileTileCache.\n\
         Defaults: dest=sample-tiles, zoom=0-12, url=OSM standard,\n\
         ua=slint-mapping/0.1, sleep-ms=300."
    );
}
