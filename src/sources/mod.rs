//! Concrete [`TileSource`](crate::TileSource) implementations.
//!
//! Each adapter lives in its own submodule and is re-exported at this
//! module's root. Adding a new source = drop a file in this directory,
//! register it here.

mod file;
mod synthetic;
pub use file::FileTileSource;
pub use synthetic::SyntheticTileSource;
