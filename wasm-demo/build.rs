//! Compile the demo's `.slint` file. The `@mapping` library path
//! points at the parent crate's `ui/` directory so `import { … }
//! from "@mapping/map.slint"` resolves to the shared MapEmbed +
//! Tile / Marker / Polyline / Layer structs.

fn main() {
    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mapping_ui = manifest_dir
        .parent()
        .expect("wasm-demo lives inside slint-mapping/")
        .join("ui");

    let mut library_paths = std::collections::HashMap::new();
    library_paths.insert("mapping".to_string(), mapping_ui);

    slint_build::compile_with_config(
        "ui/demo.slint",
        slint_build::CompilerConfiguration::new().with_library_paths(library_paths),
    )
    .expect("slint-build compile");
}
