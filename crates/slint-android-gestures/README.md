# slint-android-gestures

Multi-touch gesture detection for Slint apps on Android.

Slint 1.16 ships a `ScaleRotateGestureHandler` element, but on
Android it sits silent — winit-android doesn't yet synthesise
`PinchGesture` events from raw `MotionEvent`s. This crate fills
the gap with a tiny Kotlin shim that uses Android's built-in
`ScaleGestureDetector` and forwards detected gestures into Rust
via JNI.

First gesture supported: **pinch**. The name is deliberately
generic — swipe, long-press, rotation, two-finger pan all land in
this same crate later without forcing a rename.

The setup assumes your app uses
[`cargo-apk2`](https://github.com/mzdk100/cargo-apk2), the active
fork of cargo-apk that supports compiling Kotlin sources into the
APK. The slint-mobile template ships with cargo-apk2 from version
0.2.0 onwards.

## Setup (3 lines of config + 4 lines of Rust)

Add the crate to your app — both as a runtime dep and as a
build-dep (cargo dedups the actual compile):

```toml
# app/Cargo.toml
[dependencies]
slint-android-gestures = { path = "../../slint-mapping/crates/slint-android-gestures" }

[build-dependencies]
slint-android-gestures = { path = "../../slint-mapping/crates/slint-android-gestures" }
```

Have your `build.rs` write the bundled Kotlin into `app/kotlin/`:

```rust
// app/build.rs
fn main() {
    slint_build::compile("ui/main.slint").expect("Slint build failed");
    slint_android_gestures::build::copy_kotlin_to("kotlin")
        .expect("write kotlin sources");
}
```

Tell cargo-apk2 where to find the Kotlin + which Activity to
launch — three new lines in `[package.metadata.android]`:

```toml
[package.metadata.android]
# … your existing config …
kotlin_sources = "kotlin"

[[package.metadata.android.activity]]
name = "dev.slint.gestures.SlintGestureActivity"
exported = true
intent_filter = [{ action = "android.intent.action.MAIN", category = "android.intent.category.LAUNCHER" }]
```

Wire the callback in `android_main`:

```rust
use slint_android_gestures::on_pinch;

#[unsafe(no_mangle)]
fn android_main(app: slint::android::AndroidApp) {
    slint::android::init(app).expect("slint::android::init");
    let ui = MainWindow::new().expect("MainWindow::new");

    let weak = ui.as_weak();
    on_pinch(move |e| {
        let weak = weak.clone();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(ui) = weak.upgrade() {
                ui.invoke_zoom_by(e.delta, e.center_x, e.center_y);
            }
        });
    });

    ui.run().expect("event loop");
}
```

Build as usual:

```sh
cargo apk2 build
```

`e.delta` is in zoom levels (1 unit = 2× scale), `e.center_x/y`
is logical pixels (already DPR-corrected on the Kotlin side),
matching what Slint's window coordinates use.

## What the build step actually does

`copy_kotlin_to("kotlin")` writes the two bundled `.kt` files
(included in the crate via `include_str!`) into:

```
kotlin/dev/slint/gestures/GestureBridge.kt
kotlin/dev/slint/gestures/SlintGestureActivity.kt
```

cargo-apk2 then picks them up via `kotlin_sources = "kotlin"`,
runs `kotlinc` to produce `.class` files, packages them as
`classes.jar`, and bundles into the APK's `classes.dex` —
alongside `libapp.so`.

At runtime, `SlintGestureActivity` extends `NativeActivity`
(Slint's default), wraps `dispatchTouchEvent` to forward events
into a `ScaleGestureDetector`, and the detector's `onScale`
callback fires `GestureBridge.nativePinch(...)` — the JNI symbol
that `src/jni_impl.rs` exposes.

## Prerequisites the host machine needs

The build needs the same toolchain any Kotlin Android build needs:

- **JDK** on `PATH` (`JAVA_HOME` set is also fine)
- **Kotlin compiler** — set `KOTLIN_HOME` to the unpacked Kotlin
  distribution, available from
  <https://kotlinlang.org/docs/command-line.html>
- **Android SDK + NDK** — `ANDROID_HOME` and `ANDROID_NDK_ROOT`
  as for any cargo-apk-style build

cargo-apk2 prints a clear error if any of these are missing.

## Lifetime

This crate exists because winit-android currently doesn't emit
`PinchGesture` events from `MotionEvent`s. When that ships,
Slint's `ScaleRotateGestureHandler` will start firing natively on
Android, and the consumer-facing footprint of this crate is small
enough to remove in one commit:

- Drop the dep from `Cargo.toml` (both sections)
- Delete the `build.rs` line
- Delete the three `[package.metadata.android]` lines
- Delete the `on_pinch(...)` registration

`ui.invoke_zoom_by(...)` then gets called directly from
`ScaleRotateGestureHandler.updated`, which the rest of the
mapping stack is already wired to.

## License

MIT OR Apache-2.0.
