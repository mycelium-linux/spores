//! board — a Marbles dashboard as a GPUI Spore.
//!
//! The Spore owns the view; the host owns the WebGPU device, the compositor
//! surface and the records book. Nothing here opens a socket or reads a file:
//! the issues come from `soil:records`, and the frame goes back out through
//! `soil:present`.
//!
//! The arithmetic lives in [`model`], which has no GPU or component dependency
//! and is covered by ordinary host tests. This module is the part that cannot
//! be checked without a device, so it stays as thin as an honest renderer can.

pub mod model;

#[cfg(target_family = "wasm")]
mod spore;
