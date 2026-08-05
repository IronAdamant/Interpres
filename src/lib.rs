//! Interpres — pure-`std` core for an OS Live Captions companion.
//!
//! Strict zero third-party crates. Platform scrape lives behind helpers / `cfg` FFI.

pub mod buffer;
pub mod config;
pub mod lifecycle;
pub mod platform;
pub mod plugin_host;
pub mod probe;
pub mod protocol;
pub mod session;
pub mod transcript;
pub mod watcher;
