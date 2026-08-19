//! Production ANCS bridge core.
//!
//! Notification payloads deliberately use types that do not implement
//! `Debug`, `Display`, `Serialize`, or `Clone`. Diagnostic types never contain
//! payload fields.

pub mod ancs;
pub mod bluetooth;
pub mod clock;
pub mod notification;
pub mod status;
