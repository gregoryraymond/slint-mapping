fn main() {
    // Compile the validate stub (a Window-rooted file that imports
    // `MapView`). This gives slint-build a Window entry to anchor the
    // codegen on, so consumers see the deprecation-clean `MapView` and
    // can embed it in their own Window.
    slint_build::compile("ui/_validate.slint").expect("Slint build failed");
}
