fn main() {
    // We compile the validate stub rather than `ui/map.slint` directly
    // so slint-build sees a Window-rooted entry and doesn't emit the
    // "no code will be generated for MapEmbed" warning on every build.
    // The stub re-exports every public type so consumers (and our own
    // `slint::include_modules!`) get the same surface.
    slint_build::compile("ui/_validate.slint").expect("Slint build failed");
}
