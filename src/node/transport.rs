//! Minimal transport operations trait for upper-layer network functionality.
//!
//! This trait exposes only what cannot be obtained from the graph or network view.
//! Session management is a separate concern handled at the hopr-lib level
//! under the `session-client` feature.

use std::time::Duration;

use crate::{Multiaddr, OffchainPublicKey, graph::traits::EdgeObservable};

/// Minimal transport operations that require the full transport stack.
///
/// Peer observations and quality queries should use `HasGraphView` instead.
/// Session management (`connect_to`) is handled at the hopr-lib level.
#[async_trait::async_trait]
pub trait TransportOperations: Send + Sync {
    /// Observable type for peer quality measurements.
    type Observable: EdgeObservable + Send;
    /// Error type for transport operations.
    type Error: std::error::Error + Send + Sync + 'static;

    /// Pings a peer, returns the round-trip time and quality observations.
    async fn ping(&self, key: &OffchainPublicKey) -> Result<(Duration, Self::Observable), Self::Error>;

    /// Returns the observed multiaddresses for a peer.
    async fn observed_multiaddresses(&self, key: &OffchainPublicKey) -> Vec<Multiaddr>;
}
