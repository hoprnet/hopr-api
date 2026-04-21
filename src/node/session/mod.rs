//! Session traits for HOPR node interactions.
//!
//! - `client` — session client operations (establishing outgoing sessions)
//! - `server` — session server operations (processing incoming sessions)

#[cfg(feature = "node-session-client")]
pub mod client;

#[cfg(feature = "node-session-server")]
/// Session server traits for handling incoming sessions.
pub mod server;
