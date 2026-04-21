//! Network layer abstractions: peer connectivity, health, and stream control.
//!
//! - `NetworkView` — read-only view of network state (connected peers, health, addresses, event subscription)
//! - `NetworkStreamControl` — opening and accepting network streams
//! - `Health` — network health indicator (Red → Green spectrum)

/// Network state and stream-control traits.
pub mod traits;
/// Shared network event and health types.
pub mod types;

pub use traits::*;
pub use types::*;
