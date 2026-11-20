//! marbles-ui — the Marbles work graph as a server-rendered web page.
//!
//! The Spore owns the HTML and every projection on it, and reads the book
//! through `soil:records`. The host owns routing, durability and the clock we
//! refuse to invent: "now" is the newest moment the book records.
//!
//! The arithmetic lives in [`model`] and the markup in [`render`]; both compile
//! for the host, so `cargo test` exercises the kanban, the burndown, readiness
//! and escaping without a wasm runtime. The HTTP lane in [`guest`] is the only
//! part that needs the component model.

pub mod model;
pub mod render;

#[cfg(target_family = "wasm")]
mod guest;
