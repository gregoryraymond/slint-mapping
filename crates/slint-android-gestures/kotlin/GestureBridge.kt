// Copy this file into your Android app at
// `app/src/main/java/dev/slint/gestures/GestureBridge.kt` (the
// directory must match the `package` line below — that's what JNI
// uses to find the symbol).
//
// Then wire it up from your Activity subclass: see
// SlintGestureActivity.kt in the same directory for the canonical
// shape, or call `GestureBridge.makeDetector(...)` yourself and
// forward `dispatchTouchEvent` into the returned detector.

package dev.slint.gestures

import android.content.Context
import android.view.ScaleGestureDetector
import kotlin.math.ln

object GestureBridge {
    /**
     * JNI entry point implemented in Rust (see
     * `slint-android-gestures/src/jni_impl.rs`).
     *
     * delta is in zoom levels — positive == pinch-out (zoom in),
     * negative == pinch-in (zoom out). One unit corresponds to a 2x
     * change in linear scale, matching what mapping crates and
     * tile-layer renderers expect from a `zoom_by` callback.
     *
     * (focusX, focusY) is the midpoint of the two fingers in
     * *logical* pixels (already divided by display density), so it
     * lines up with what Slint's window coordinate system uses for
     * its own pointer events.
     */
    @JvmStatic
    external fun nativePinch(delta: Float, focusX: Float, focusY: Float)

    /**
     * Build a ScaleGestureDetector wired to the JNI bridge. The
     * caller (your Activity) is responsible for forwarding
     * MotionEvents into the returned detector via
     * `detector.onTouchEvent(event)` from `dispatchTouchEvent`.
     *
     * @param density the display's `DisplayMetrics.density`, used
     *   to convert physical pixel coordinates from the gesture into
     *   the logical-pixel coordinates Slint expects.
     */
    @JvmStatic
    fun makeDetector(context: Context, density: Float): ScaleGestureDetector {
        return ScaleGestureDetector(
            context,
            object : ScaleGestureDetector.SimpleOnScaleGestureListener() {
                // ScaleGestureDetector reports a cumulative
                // `scaleFactor` (1.0 at gesture start, then grows
                // or shrinks as the user pinches). We want the
                // INCREMENTAL ratio since the previous event so the
                // Rust side can sum deltas cleanly without having
                // to know about gesture lifecycle.
                private var lastScale = 1f

                override fun onScaleBegin(d: ScaleGestureDetector): Boolean {
                    lastScale = 1f
                    return true
                }

                override fun onScale(d: ScaleGestureDetector): Boolean {
                    val current = d.scaleFactor
                    if (current <= 0f || lastScale <= 0f) return true
                    val ratio = current / lastScale
                    lastScale = current
                    // log2(ratio) — ratio of 2 means +1 zoom level.
                    // ln(x) / ln(2) avoids needing kotlin.math.log2
                    // which is API 26+ only (we target API 24+).
                    val delta = (ln(ratio.toDouble()) / ln(2.0)).toFloat()
                    nativePinch(delta, d.focusX / density, d.focusY / density)
                    return true
                }
            }
        )
    }
}
