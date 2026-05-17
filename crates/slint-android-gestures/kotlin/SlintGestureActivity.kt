// Canonical Activity subclass that wires GestureBridge into a Slint
// app. Copy alongside GestureBridge.kt at
// `app/src/main/java/dev/slint/gestures/SlintGestureActivity.kt`,
// then point AndroidManifest.xml at this class instead of the
// default `android.app.NativeActivity`:
//
//   <activity
//       android:name="dev.slint.gestures.SlintGestureActivity"
//       android:exported="true"
//       android:configChanges="orientation|keyboardHidden|screenSize|smallestScreenSize|screenLayout|density|uiMode">
//       <meta-data
//           android:name="android.app.lib_name"
//           android:value="your_crate_name" /> <!-- the name of your Rust cdylib -->
//       <intent-filter>
//           <action android:name="android.intent.action.MAIN" />
//           <category android:name="android.intent.category.LAUNCHER" />
//       </intent-filter>
//   </activity>
//
// Slint uses NativeActivity by default — we extend it so the rest of
// Slint's plumbing (lifecycle, surface, input forwarding to the
// underlying winit/Skia stack) keeps working unmodified. We just
// peek at touch events in dispatchTouchEvent before letting them
// continue.

package dev.slint.gestures

import android.app.NativeActivity
import android.os.Bundle
import android.view.MotionEvent

class SlintGestureActivity : NativeActivity() {
    private lateinit var pinchDetector: android.view.ScaleGestureDetector

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        pinchDetector = GestureBridge.makeDetector(
            this,
            resources.displayMetrics.density,
        )
    }

    override fun dispatchTouchEvent(ev: MotionEvent): Boolean {
        // Feed the detector first — it inspects without consuming.
        pinchDetector.onTouchEvent(ev)
        // Then let NativeActivity (and through it, Slint) handle the
        // event normally so single-touch pan / tap keep working.
        return super.dispatchTouchEvent(ev)
    }
}
