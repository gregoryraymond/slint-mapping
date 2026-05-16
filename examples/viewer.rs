//! Map viewer demo.
//!
//! ```sh
//! cargo run --example viewer --features viewer            # synthetic tiles
//! cargo run --example viewer --features viewer -- /tiles  # file source
//! ```
//!
//! Drag to pan, scroll-wheel to zoom (anchored at the cursor).

use slint::ComponentHandle;
use slint_mapping::sources::{FileTileSource, SyntheticTileSource};
use slint_mapping::{MapController, MapView, TileSource};

fn main() -> Result<(), slint::PlatformError> {
    let arg = std::env::args().nth(1);
    let source: Box<dyn TileSource> = match arg {
        Some(path) => {
            eprintln!("Reading tiles from {path}");
            Box::new(FileTileSource::new(path))
        }
        None => {
            eprintln!("No tile directory given — using synthetic source.");
            eprintln!("(Pass a slippy-map dir to use real tiles: `cargo run --example viewer --features viewer -- /path/to/tiles`)");
            Box::new(SyntheticTileSource::new())
        }
    };

    let map = MapView::new()?;
    let controller = MapController::new(&map, source);
    controller.set_centre(-0.1276, 51.5074); // London
    controller.set_zoom(4.0);
    // Keep the controller alive for the run.
    let _hold = controller;

    map.run()
}
