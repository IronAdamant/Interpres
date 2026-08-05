//! Interpres — pure-`std` core for an OS Live Captions companion.
//!
//! Strict zero third-party crates (crates.io). Platform UI/scrape via system
//! frameworks + hand-written FFI / native clang objects.

pub mod assets_check;
pub mod buffer;
pub mod config;
pub mod debuglog;
pub mod engine;
pub mod gui;
pub mod lifecycle;
pub mod platform;
pub mod plugin_host;
pub mod probe;
pub mod protocol;
pub mod session;
pub mod transcript;
pub mod ui_labels;
pub mod watcher;
