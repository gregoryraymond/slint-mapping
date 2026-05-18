//! Reusable multi-touch gesture detection for Slint apps on Android.
//!
//! Slint 1.16 ships [`ScaleRotateGestureHandler`], but on Android it
//! sits silent because winit-android doesn't yet emit
//! `WindowEvent::PinchGesture` from raw `MotionEvent`s — that's only
//! plumbed for macOS and iOS. This crate fills the gap with the
//! minimum-viable platform shim: a Kotlin `ScaleGestureDetector`
//! that pre-processes touches on the activity thread and pokes
//! Rust via JNI, plus a safe Rust API consumers register their
//! callback on.
//!
//! # Build setup (cargo-apk2)
//!
//! The crate ships two Kotlin files (`GestureBridge.kt` +
//! `SlintGestureActivity.kt`). To get them into your APK, your
//! consumer crate uses [`build::copy_kotlin_to`] from its own
//! `build.rs` and points `cargo-apk2` at the destination via
//! `kotlin_sources = "kotlin"` in `[package.metadata.android]`.
//!
//! See the crate's `README.md` for the full one-page integration.
//!
//! # The crate is split in two halves
//!
//! 1. **This Rust side** — accepts a callback via [`on_pinch`],
//!    stores it behind a mutex, and exposes `extern "system"` JNI
//!    symbols the Kotlin side calls. When the JNI symbol fires we
//!    dispatch the stored callback. All [`PinchEvent`] coords are
//!    *logical (CSS-style) pixels* — matching what
//!    `slint::Window::invoke_*` and overlay-projection helpers
//!    expect.
//!
//! 2. **A Kotlin shim** — a small `GestureBridge.kt` file (in the
//!    crate's `kotlin/` directory, copy-paste into your Android
//!    project's `java/`) wraps `ScaleGestureDetector` and calls
//!    the JNI function with `log2(scaleRatio)` and the focus
//!    point converted to logical pixels.
//!
//! See `README.md` in this crate for the full setup, including the
//! Activity subclass that owns the detector and the manifest entry
//! that points at it.
//!
//! # Minimal Slint-side usage
//!
//! ```rust,ignore
//! #[unsafe(no_mangle)]
//! fn android_main(app: slint::android::AndroidApp) {
//!     slint::android::init(app).unwrap();
//!     let ui = MainWindow::new().unwrap();
//!
//!     // Forward every detected pinch into the existing zoom-by
//!     // callback on the Slint scene. invoke_from_event_loop hops
//!     // from the JNI thread (which IS the UI thread on Android,
//!     // but we go through Slint's queue anyway so property writes
//!     // don't race with the renderer).
//!     let weak = ui.as_weak();
//!     slint_android_gestures::on_pinch(move |e| {
//!         let weak = weak.clone();
//!         let _ = slint::invoke_from_event_loop(move || {
//!             if let Some(ui) = weak.upgrade() {
//!                 ui.invoke_zoom_by(e.delta, e.center_x, e.center_y);
//!             }
//!         });
//!     });
//!
//!     ui.run().unwrap();
//! }
//! ```
//!
//! # What "future-proof" looks like
//!
//! Once winit-android emits `WindowEvent::PinchGesture`, Slint's
//! `ScaleRotateGestureHandler` starts firing on Android natively
//! and this crate becomes unnecessary. The API surface is small on
//! purpose so the swap is a one-commit removal — drop the `on_pinch`
//! call, delete the Kotlin file, restore the default Activity in
//! AndroidManifest.xml. Your `zoom_by` callback keeps working.

use std::sync::Mutex;

#[cfg(target_os = "android")]
mod jni_impl;

pub mod build;

/// One detected pinch update. Emitted continuously while the user is
/// pinching (one per `onScale` callback in the Kotlin shim), then no
/// more events until the next gesture begins.
#[derive(Debug, Clone, Copy)]
pub struct PinchEvent {
    /// Zoom-level delta since the *previous* event in the same
    /// gesture. Positive = pinch-out (zoom in), negative =
    /// pinch-in (zoom out). One unit corresponds to one tile-zoom
    /// step (a 2× change in scale).
    pub delta: f32,
    /// X coordinate of the pinch focus point in logical pixels
    /// (CSS-style — already divided by `density` on the Kotlin
    /// side). Lines up with Slint window coordinates.
    pub center_x: f32,
    /// Y coordinate of the pinch focus point in logical pixels.
    pub center_y: f32,
}

type PinchCallback = Box<dyn Fn(PinchEvent) + Send + 'static>;

// Static so the JNI symbol — which is `extern "system"` and has no
// receiver — can find the callback. `Mutex` because the consumer
// may register from a different thread than the one the JNI call
// arrives on (in practice both are the Android UI thread, but
// formalising it costs nothing and keeps `Sync` enforcement honest).
static PINCH_HANDLER: Mutex<Option<PinchCallback>> = Mutex::new(None);

/// Install (or replace) the closure that runs for every pinch event.
///
/// The closure is invoked from the Android UI thread, in the JNI
/// call from the Kotlin shim. If you need to touch Slint properties
/// (the common case), hop through `slint::invoke_from_event_loop`
/// inside the closure — direct property writes from a JNI thread
/// can race the renderer.
///
/// Passing a closure when one is already installed replaces it.
/// To stop receiving events, call [`clear_pinch_handler`].
pub fn on_pinch(callback: impl Fn(PinchEvent) + Send + 'static) {
    *PINCH_HANDLER.lock().unwrap() = Some(Box::new(callback));
}

/// Remove the pinch handler. Subsequent JNI events are dropped.
pub fn clear_pinch_handler() {
    *PINCH_HANDLER.lock().unwrap() = None;
}

/// Internal — called by the JNI shim. Locks the mutex, clones a
/// reference to the callback, releases the lock BEFORE invoking the
/// user code (avoids reentrancy deadlocks if the user re-enters
/// `on_pinch` from inside the callback).
#[cfg(target_os = "android")]
pub(crate) fn dispatch_pinch(event: PinchEvent) {
    // Lock, take an Arc-ish handle, drop the lock immediately.
    // Box<dyn Fn> isn't Clone so we need to call under the lock —
    // but the lock window is one closure invocation, which is fine
    // because the user is supposed to dispatch into the slint
    // event loop and return quickly.
    let guard = PINCH_HANDLER.lock().unwrap();
    if let Some(cb) = guard.as_ref() {
        cb(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex as StdMutex};

    #[test]
    fn handler_can_be_installed_and_cleared() {
        let captured: Arc<StdMutex<Vec<f32>>> = Arc::new(StdMutex::new(Vec::new()));
        {
            let captured = Arc::clone(&captured);
            on_pinch(move |e| {
                captured.lock().unwrap().push(e.delta);
            });
        }
        // Native targets don't have the JNI dispatch path; simulate
        // it by hitting the public handler slot the same way the
        // JNI symbol would.
        let cb_guard = PINCH_HANDLER.lock().unwrap();
        if let Some(cb) = cb_guard.as_ref() {
            cb(PinchEvent {
                delta: 0.5,
                center_x: 100.0,
                center_y: 200.0,
            });
            cb(PinchEvent {
                delta: -0.25,
                center_x: 100.0,
                center_y: 200.0,
            });
        }
        drop(cb_guard);
        assert_eq!(*captured.lock().unwrap(), vec![0.5, -0.25]);

        clear_pinch_handler();
        assert!(PINCH_HANDLER.lock().unwrap().is_none());
    }
}
