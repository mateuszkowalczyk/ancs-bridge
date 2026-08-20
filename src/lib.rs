//! Production ANCS bridge core.
//!
//! Notification payloads deliberately use types that do not implement
//! `Debug`, `Display`, `Serialize`, or `Clone`. Diagnostic types never contain
//! payload fields.

pub mod ancs;
mod atomic_file;
pub mod audio;
pub mod bluetooth;
pub mod clock;
pub mod config;
pub mod diagnostics;
pub mod machine;
pub mod notification;
pub mod service;
pub mod setup;
pub mod status;
pub mod teardown;
