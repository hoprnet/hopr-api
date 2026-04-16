//! Network operations derived purely from [`NetworkView`].
//!
//! These operations are automatically available on any type that implements [`HasNetworkView`].

use std::collections::HashSet;

use crate::{Multiaddr, PeerId, network::{Health, NetworkView}};

use super::accessors::HasNetworkView;

/// High-level network operations backed by [`NetworkView`](crate::network::NetworkView).
///
/// Automatically implemented for any type with a [`HasNetworkView`] accessor.
pub trait HoprNodeNetworkOperations: HasNetworkView {
    /// Returns the current network health indicator.
    fn network_health(&self) -> Health {
        self.network_view().health()
    }

    /// Returns the set of currently connected peers.
    fn network_connected_peers(&self) -> HashSet<PeerId> {
        self.network_view().connected_peers()
    }

    /// Returns the multiaddresses this node is listening on.
    fn local_multiaddresses(&self) -> HashSet<Multiaddr> {
        self.network_view().listening_as()
    }

    /// Returns whether the given peer is currently connected.
    fn is_connected(&self, peer: &PeerId) -> bool {
        self.network_view().is_connected(peer)
    }

    /// Returns the set of discovered peers (may include disconnected peers).
    fn discovered_peers(&self) -> HashSet<PeerId> {
        self.network_view().discovered_peers()
    }
}

/// Blanket implementation for all types with network view access.
impl<T: HasNetworkView> HoprNodeNetworkOperations for T {}
