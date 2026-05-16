//! slint-mapping — a map framework for Slint.
//!
//! Status: scaffold. The `ui/map.slint` `MapView` component is a
//! placeholder Rectangle with a single `Image` slot for tile output;
//! the Rust side that actually fetches/renders tiles, handles
//! pan/zoom gestures, and overlays markers is yet to be written.
//!
//! See `README.md` for the open design questions that will shape
//! the next steps.

slint::include_modules!();
