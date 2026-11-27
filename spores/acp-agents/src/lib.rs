//! ACP over HTTP -> `soil:agents`.
//!
//! ACP is a protocol. This adapter uses the same transport a Spore already has:
//! `soil:http`, bounded by the host's allowlist. It does not spawn a process and
//! it does not own a session daemon.

#[cfg(target_arch = "wasm32")]
mod guest;
