use std::collections::HashSet;

use futures::{AsyncRead, AsyncWrite, Stream, future::BoxFuture};

use super::{Health, NetworkEvent};
use crate::{Multiaddr, PeerId};

/// Type alias for a boxed function returning a boxed future.
pub type BoxedProcessFn = Box<dyn FnOnce() -> BoxFuture<'static, ()> + Send>;

/// Trait representing a read-only view of the network state.
#[auto_impl::auto_impl(&, Arc)]
pub trait NetworkView {
    /// Multiaddresses used for listening by the local node.
    fn listening_as(&self) -> HashSet<Multiaddr>;

    /// Translation of the peer into its known multiaddresses.
    fn multiaddress_of(&self, peer: &PeerId) -> Option<HashSet<Multiaddr>>;

    /// Peers collected by the network discovery mechanism.
    fn discovered_peers(&self) -> HashSet<PeerId>;

    /// Peers currently connected and tracked by the network.
    fn connected_peers(&self) -> HashSet<PeerId>;

    /// Peers currently connected and tracked by the network.
    fn is_connected(&self, peer: &PeerId) -> bool;

    /// Represents perceived health of the network.
    fn health(&self) -> Health;

    /// Subscribes to network events (peer connected/disconnected).
    ///
    /// Each call creates a new independent subscription. Must be called
    /// before the network starts processing to avoid missing initial events.
    fn subscribe_network_events(&self) -> impl Stream<Item = NetworkEvent> + Send + 'static;
}

/// Control object for the opening and receiving of network connections in the
/// form of network streams.
#[async_trait::async_trait]
pub trait NetworkStreamControl: std::fmt::Debug {
    fn accept(
        self,
    ) -> Result<impl Stream<Item = (PeerId, impl AsyncRead + AsyncWrite + Send)> + Send, impl std::error::Error>;

    async fn open(self, peer: PeerId) -> Result<impl AsyncRead + AsyncWrite + Send, impl std::error::Error>;
}
