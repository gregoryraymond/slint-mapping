//! Symbols Kotlin calls into.
//!
//! One exported function for now — pinch updates. Add more here as
//! the crate grows (swipe, long-press, two-finger pan, etc.) and
//! match them with new methods on the Kotlin side.

use crate::PinchEvent;
use jni::objects::JClass;
use jni::sys::jfloat;
use jni::JNIEnv;

/// JNI entry point for the Kotlin `GestureBridge.nativePinch(...)`
/// method. Name follows the JNI mangling rule:
/// `Java_<package_underscored>_<class>_<method>`, so it matches
/// `package dev.slint.gestures` + `class GestureBridge` +
/// `method nativePinch`.
///
/// Don't rename without updating `kotlin/GestureBridge.kt` and any
/// AndroidManifest.xml entries that reference the package.
///
/// # Safety
/// Standard JNI extern. Arguments are primitive (jfloat) so no
/// pointer validity to check. The function is `Sync` because the
/// dispatched callback storage is behind a Mutex.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_slint_gestures_GestureBridge_nativePinch(
    _env: JNIEnv,
    _class: JClass,
    delta: jfloat,
    center_x: jfloat,
    center_y: jfloat,
) {
    super::dispatch_pinch(PinchEvent {
        delta: delta as f32,
        center_x: center_x as f32,
        center_y: center_y as f32,
    });
}
