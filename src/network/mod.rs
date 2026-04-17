//! Network layer abstractions: peer connectivity, health, and stream control.
//!
//! - [`NetworkView`] — read-only view of network state (connected peers, health, addresses)
//! - [`NetworkStreamControl`] — opening and accepting network streams
//! - [`NetworkBuilder`] — factory for constructing network implementations
//! - [`Health`] — network health indicator (Red → Green spectrum)

pub mod traits;
pub mod types;

pub use traits::*;
pub use types::*;
