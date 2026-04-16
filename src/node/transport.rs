//! Minimal transport operations trait for upper-layer network functionality.
//!
//! This trait exposes only what the node's network and peer observation layer needs
//! from the transport stack. Session management is a separate concern handled
//! at the hopr-lib level under the `session-client` feature.

use std::time::Duration;

use crate::{graph::traits::EdgeObservable, Multiaddr, OffchainPublicKey};

/// Minimal transport operations required by the node's upper API layer.
///
/// This deliberately does NOT expose session management, graph mutations,
/// or lifecycle methods — those are separate concerns internal to the transport.
#[async_trait::async_trait]
pub trait TransportOperations: Send + Sync {
    /// Observable type for peer quality measurements.
    type Observable: EdgeObservable + Send;
    /// Error type for transport operations.
    type Error: std::error::Error + Send + Sync + 'static;

    /// Pings a peer, returns the round-trip time and quality observations.
    async fn ping(&self, key: &OffchainPublicKey) -> Result<(Duration, Self::Observable), Self::Error>;

    /// Returns all peers with quality above the given threshold.
    async fn all_network_peers(
        &self,
        min_quality: f64,
    ) -> Result<Vec<(OffchainPublicKey, Self::Observable)>, Self::Error>;

    /// Returns quality observations for a specific peer, if any.
    fn network_peer_info(&self, key: &OffchainPublicKey) -> Option<Self::Observable>;

    /// Returns the observed multiaddresses for a peer.
    async fn observed_multiaddresses(&self, key: &OffchainPublicKey) -> Vec<Multiaddr>;
}
