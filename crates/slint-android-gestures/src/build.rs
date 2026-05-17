//! Build-script helpers consumers call from their own `build.rs` to
//! materialise the crate's bundled Kotlin sources into a directory
//! that `cargo-apk2`'s `kotlin_sources = "..."` config can find.
//!
//! The Kotlin source is `include_str!`'d at this crate's compile
//! time, so the consumer's build doesn't need to resolve our
//! source path — they just call [`copy_kotlin_to`] and the files
//! land wherever they want.
//!
//! # Typical use
//!
//! In the consumer's `build.rs`:
//!
//! ```ignore
//! fn main() {
//!     // … whatever else (slint_build::compile, etc.) …
//!
//!     // Emit the gesture-bridge Kotlin into ./kotlin so
//!     // cargo-apk2's `kotlin_sources = "kotlin"` picks it up.
//!     slint_android_gestures::build::copy_kotlin_to("kotlin")
//!         .expect("write kotlin sources");
//! }
//! ```
//!
//! Note: add the crate to `[build-dependencies]` (in addition to
//! `[dependencies]` if you also call the runtime API). Cargo dedups
//! the compilation across the two roles.

use std::fs;
use std::io;
use std::path::Path;

/// One bundled Kotlin source file, with the relative path it should
/// be written to inside the consumer's `kotlin_sources` directory.
/// The path is the standard Java/Kotlin convention — directory
/// segments mirror the `package` declaration.
pub struct KotlinSource {
    pub relative_path: &'static str,
    pub contents: &'static str,
}

/// Every Kotlin file this crate ships. The package path
/// (`dev/slint/gestures/...`) matches the JNI symbol name in
/// `src/jni_impl.rs` — don't rename one without the other.
pub const KOTLIN_SOURCES: &[KotlinSource] = &[
    KotlinSource {
        relative_path: "dev/slint/gestures/GestureBridge.kt",
        contents: include_str!("../kotlin/GestureBridge.kt"),
    },
    KotlinSource {
        relative_path: "dev/slint/gestures/SlintGestureActivity.kt",
        contents: include_str!("../kotlin/SlintGestureActivity.kt"),
    },
];

/// Write every bundled Kotlin source into `dest_dir`, creating
/// intermediate package directories as needed. Existing files are
/// overwritten unconditionally so re-runs after a crate upgrade
/// always pick up the latest shim. `dest_dir` is conventionally
/// `"kotlin"` (matching `kotlin_sources = "kotlin"` in the
/// consumer's `[package.metadata.android]`).
pub fn copy_kotlin_to(dest_dir: impl AsRef<Path>) -> io::Result<()> {
    let dest = dest_dir.as_ref();
    for source in KOTLIN_SOURCES {
        let path = dest.join(source.relative_path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&path, source.contents)?;
        // Tell cargo to rebuild the consumer when our bundled
        // sources change (during local development of this crate
        // via a `path = ".."` dep, mostly).
        println!("cargo:rerun-if-changed={}", path.display());
    }
    Ok(())
}
