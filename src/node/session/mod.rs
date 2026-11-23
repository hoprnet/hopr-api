//! Session traits for HOPR node interactions.
//!
//! - `entry` — session entry operations (establishing outgoing sessions)
//! - `exit` — session exit operations (processing incoming sessions)

#[cfg(feature = "node-session-entry")]
pub mod entry;

#[cfg(feature = "node-session-exit")]
/// Session exit traits for handling incoming sessions.
pub mod exit;

/// Deprecated path of [`entry`].
#[cfg(feature = "node-session-entry")]
#[deprecated(note = "renamed to `entry`")]
pub mod client {
    pub use super::entry::*;
}

/// Deprecated path of [`exit`].
#[cfg(feature = "node-session-exit")]
#[deprecated(note = "renamed to `exit`")]
pub mod server {
    pub use super::exit::*;
}
