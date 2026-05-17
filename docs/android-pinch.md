# Pinch-to-zoom on Android

`slint-mapping`'s [`ScaleRotateGestureHandler`](../ui/map.slint) only
fires when the platform sends a `WindowEvent::PinchGesture` up the
input tree. Today winit emits those on macOS and iOS only —
winit-android still hands Slint raw `MotionEvent`s without doing
gesture recognition. So on Android the handler sits silent until
winit grows the support upstream.

The shortest path to working pinch on a Slint Android app today is:

1. Use [cargo-apk2](https://github.com/mzdk100/cargo-apk2) instead of
   cargo-apk. It's the active fork; supports `kotlin_sources` and
   `[[package.metadata.android.activity]]` declaratively.
2. Add the [`slint-android-gestures`](../crates/slint-android-gestures)
   crate. It ships the ~80 lines of Kotlin that wrap
   `ScaleGestureDetector` and forward events into Rust via JNI.

If you'd rather wire it by hand (no extra crate), the bottom of this
doc covers that path. Both approaches end up calling the same
`zoom_by(delta, anchor_x, anchor_y)` callback the desktop wheel
zoom and the wasm pinch path already use, so the rest of the
camera stack is unchanged.

## The drop-in path: cargo-apk2 + slint-android-gestures

Switch the app to cargo-apk2:

```sh
cargo install cargo-apk2 --locked
```

Then the crate's README has the full walkthrough:
[`crates/slint-android-gestures/README.md`](../crates/slint-android-gestures/README.md).
Summary:

```toml
# app/Cargo.toml
[dependencies]
slint-android-gestures = { path = "…/slint-android-gestures" }

[build-dependencies]
slint-android-gestures = { path = "…/slint-android-gestures" }

[package.metadata.android]
# … your existing config …
kotlin_sources = "kotlin"

[[package.metadata.android.activity]]
name = "dev.slint.gestures.SlintGestureActivity"
exported = true
intent_filter = [{ action = "android.intent.action.MAIN", category = "android.intent.category.LAUNCHER" }]
```

```rust
// app/build.rs
fn main() {
    slint_build::compile("ui/main.slint").unwrap();
    slint_android_gestures::build::copy_kotlin_to("kotlin").unwrap();
}
```

```rust
// app/src/lib.rs (android_main)
use slint_android_gestures::on_pinch;

let weak = ui.as_weak();
on_pinch(move |e| {
    let weak = weak.clone();
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_zoom_by(e.delta, e.center_x, e.center_y);
        }
    });
});
```

Build:

```sh
cargo apk2 build
```

That's the whole integration on the consumer side. No manual Kotlin
edits, no manifest XML, no JNI handlers in your own code.

## The hand-rolled path

If you don't want a crate dependency for the gesture detection — for
example, you also need fling / long-press / system-gesture
coordination beyond pinch — write the Kotlin yourself and bind it
via JNI from your `android_main`. Same shape as the crate just
written by you:

### Kotlin Activity subclass

Put this at `app/kotlin/your/package/MapActivity.kt`:

```kotlin
package your.package

import android.app.NativeActivity
import android.view.MotionEvent
import android.view.ScaleGestureDetector
import kotlin.math.ln

class MapActivity : NativeActivity() {
    private external fun nativePinch(delta: Float, focusX: Float, focusY: Float)

    private val scaleDetector by lazy {
        ScaleGestureDetector(this, object : ScaleGestureDetector.SimpleOnScaleGestureListener() {
            private var lastScale = 1f

            override fun onScaleBegin(d: ScaleGestureDetector): Boolean {
                lastScale = 1f; return true
            }

            override fun onScale(d: ScaleGestureDetector): Boolean {
                val ratio = d.scaleFactor / lastScale
                lastScale = d.scaleFactor
                val delta = (ln(ratio.toDouble()) / ln(2.0)).toFloat()
                val dpr = resources.displayMetrics.density
                nativePinch(delta, d.focusX / dpr, d.focusY / dpr)
                return true
            }
        })
    }

    override fun dispatchTouchEvent(ev: MotionEvent): Boolean {
        scaleDetector.onTouchEvent(ev)
        return super.dispatchTouchEvent(ev)
    }
}
```

### Rust JNI symbol + manifest entry

```rust
#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_your_package_MapActivity_nativePinch(
    _env: jni::JNIEnv,
    _class: jni::objects::JClass,
    delta: jni::sys::jfloat,
    focus_x: jni::sys::jfloat,
    focus_y: jni::sys::jfloat,
) {
    APP_HANDLE.with(|h| {
        if let Some(ui) = h.borrow().as_ref().and_then(|w| w.upgrade()) {
            ui.invoke_zoom_by(delta, focus_x, focus_y);
        }
    });
}
```

```toml
[package.metadata.android]
kotlin_sources = "kotlin"

[[package.metadata.android.activity]]
name = "your.package.MapActivity"
exported = true
intent_filter = [{ action = "android.intent.action.MAIN", category = "android.intent.category.LAUNCHER" }]
```

Add `jni = "0.21"` under `[target.'cfg(target_os = "android")'.dependencies]`.

## Why both paths use cargo-apk2

The blocker, before cargo-apk2, was that **the original cargo-apk
doesn't compile a single line of Kotlin or Java**. There's no
`kotlin_sources` field, no `[[package.metadata.android.activity]]`
array — the resulting APK has zero JVM code in `classes.dex` because
it never bundles a `classes.dex` at all. Without that, both options
above are dead in the water.

cargo-apk2 ([crates.io](https://crates.io/crates/cargo-apk2),
[GitHub](https://github.com/mzdk100/cargo-apk2)) is an active fork
that added exactly the missing pieces. The slint-mobile template
moved to it for that reason. See the [`rust-android-build`](../docs/android-build-tools.md)
notes for the broader lineage.

## Future-proof checklist

When `winit-android` adds `WindowEvent::PinchGesture` emission,
both options above become deletable:

- **cargo-apk2 + slint-android-gestures**: drop the crate dep,
  delete the `build.rs` line, delete the three
  `[package.metadata.android]` lines.
- **Hand-rolled**: delete the `.kt` file, the JNI symbol, the
  activity entry.

`ScaleRotateGestureHandler` already in `ui/map.slint` then fires
natively and the `zoom_by` callback you're wired to keeps working
unchanged.
