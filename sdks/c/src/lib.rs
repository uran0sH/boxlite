//! C SDK for BoxLite
//!
//! This crate provides C FFI bindings for the BoxLite runtime,
//! building the C shared library and static library artifacts.

pub mod ffi;

// Re-export all FFI symbols
pub use ffi::*;
