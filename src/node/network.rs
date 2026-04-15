use std::time::Duration;

use hopr_types::{crypto::prelude::OffchainPublicKey, primitive::prelude::Address};
use libp2p_identity::PeerId;
use multiaddr::Multiaddr;

use crate::{graph::traits::EdgeObservable, network::Health};

/// High-level network operations.
#[async_trait::async_trait]
pub trait HoprNodeNetworkOperations {
    /// Error type for node operations.
    type NodeNetworkError: std::error::Error + Send + Sync + 'static;

    /// Observable type returned by peer information queries.
    type TransportObservable: EdgeObservable + Send;

    // === Identity ===

    /// Returns the PeerId of this node used in the transport layer.
    fn me_peer_id(&self) -> PeerId;

    /// Allows translation of a peer's transport identity to the corresponding off-chain key.
    ///
    /// The implementor may wish to cache this operation for performance reasons.
    fn peer_id_to_offchain_key(&self, peer_id: &PeerId) -> Result<OffchainPublicKey, Self::NodeNetworkError>;

    /// Returns all public nodes announced on the network.
    async fn get_public_nodes(&self) -> Result<Vec<(PeerId, Address, Vec<Multiaddr>)>, Self::NodeNetworkError>;

    /// Returns the current network health status.
    async fn network_health(&self) -> Health;

    /// Returns all currently connected peers.
    async fn network_connected_peers(&self) -> Result<Vec<PeerId>, Self::NodeNetworkError>;

    /// Returns observations for a specific peer.
    fn network_peer_info(&self, peer: &PeerId) -> Option<Self::TransportObservable>;

    /// Returns all network peers with quality above the minimum score.
    async fn all_network_peers(
        &self,
        minimum_score: f64,
    ) -> Result<Vec<(Option<Address>, PeerId, Self::TransportObservable)>, Self::NodeNetworkError>;

    // === Transport ===

    /// Returns the multiaddresses this node is announcing.
    fn local_multiaddresses(&self) -> Vec<Multiaddr>;

    /// Returns the multiaddresses this node is listening to.
    async fn listening_multiaddresses(&self) -> Vec<Multiaddr>;

    /// Returns the observed multiaddresses for a peer.
    async fn network_observed_multiaddresses(&self, peer: &PeerId) -> Vec<Multiaddr>;

    /// Returns the multiaddresses announced on-chain for a peer.
    async fn multiaddresses_announced_on_chain(&self, peer: &PeerId) -> Result<Vec<Multiaddr>, Self::NodeNetworkError>;

    // === Peers ===

    /// Pings a peer and returns the round-trip time along with observable data.
    async fn ping(&self, peer: &PeerId) -> Result<(Duration, Self::TransportObservable), Self::NodeNetworkError>;
}
