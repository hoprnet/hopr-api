//! Has* accessor traits providing minimal per-component access to the HOPR node.
//!
//! Each accessor trait exposes:
//! - A typed reference to the component's API
//! - A sync `status()` method reporting component health
//!
//! Composed traits (`IncentiveChannelOperations`, `IncentiveRedeemOperations`, etc.)
//! are blanket-implemented over combinations of these accessors.

use std::time::Duration;

use futures::Stream;
use hopr_types::chain::chain_events::ChainEvent;

use super::{ComponentStatus, EventWaitResult, NodeOnchainIdentity, TicketEvent, transport::TransportOperations};
use crate::{
    OffchainPublicKey,
    chain::HoprChainApi,
    graph::{NetworkGraphConnectivity, NetworkGraphTraverse, NetworkGraphView},
    network::NetworkView,
    tickets::TicketManagement,
};

// ---------------------------------------------------------------------------
// HasChainApi
// ---------------------------------------------------------------------------

/// Provides access to the chain component of a HOPR node.
#[auto_impl::auto_impl(&, Arc)]
pub trait HasChainApi {
    /// The concrete chain API implementation.
    type ChainApi: HoprChainApi + Clone + Send + Sync + 'static;

    /// Error type for node-level chain operations (distinct from on-chain errors).
    type ChainError: std::error::Error + Send + Sync + 'static;

    /// Returns the node's on-chain identity (node address, Safe address, module address).
    fn identity(&self) -> &NodeOnchainIdentity;

    /// Returns a reference to the underlying chain API.
    fn chain_api(&self) -> &Self::ChainApi;

    /// Reports the current health of the chain component.
    fn status(&self) -> ComponentStatus;

    /// Spawns an asynchronous waiter that subscribes to [`ChainEvent`]s
    /// and resolves when `predicate` matches or `timeout` elapses.
    fn wait_for_on_chain_event<F>(
        &self,
        predicate: F,
        context: String,
        timeout: Duration,
    ) -> EventWaitResult<<Self::ChainApi as HoprChainApi>::ChainError, Self::ChainError>
    where
        F: Fn(&ChainEvent) -> bool + Send + Sync + 'static;
}

// ---------------------------------------------------------------------------
// HasNetworkView
// ---------------------------------------------------------------------------

/// Provides read-only access to the network layer (peer connectivity, addresses).
#[auto_impl::auto_impl(&, Arc)]
pub trait HasNetworkView {
    /// The concrete [`NetworkView`] implementation.
    type NetworkView: NetworkView + Send + Sync;

    /// Returns a reference to the network view.
    fn network_view(&self) -> &Self::NetworkView;

    /// Reports the current health of the network component.
    fn status(&self) -> ComponentStatus;
}

// ---------------------------------------------------------------------------
// HasGraphView
// ---------------------------------------------------------------------------

/// Provides read-only access to the network graph and its health status.
#[auto_impl::auto_impl(&, Arc)]
pub trait HasGraphView {
    /// The concrete graph type, constrained to read-only operations.
    type Graph: NetworkGraphView<NodeId = OffchainPublicKey>
        + NetworkGraphConnectivity<NodeId = OffchainPublicKey>
        + NetworkGraphTraverse<NodeId = OffchainPublicKey>
        + Send
        + Sync;

    /// Returns a reference to the network graph.
    fn graph(&self) -> &Self::Graph;

    /// Reports the current health of the graph component.
    fn status(&self) -> ComponentStatus;
}

// ---------------------------------------------------------------------------
// HasTransportApi
// ---------------------------------------------------------------------------

/// Provides access to transport-level operations (ping, peer observations).
#[auto_impl::auto_impl(&, Arc)]
pub trait HasTransportApi {
    /// The concrete [`TransportOperations`] implementation.
    type Transport: TransportOperations;

    /// Returns a reference to the transport API.
    fn transport(&self) -> &Self::Transport;

    /// Reports the current health of the transport component.
    fn status(&self) -> ComponentStatus;
}

// ---------------------------------------------------------------------------
// HasTicketManagement
// ---------------------------------------------------------------------------

/// Provides access to the ticket management component.
///
/// Only available on relay nodes that process tickets.
#[auto_impl::auto_impl(&, Arc)]
pub trait HasTicketManagement {
    /// The concrete [`TicketManagement`] implementation.
    type TicketManager: TicketManagement + Clone + Send + Sync + 'static;

    /// Returns a reference to the ticket manager.
    fn ticket_management(&self) -> &Self::TicketManager;

    /// Returns a stream of [`TicketEvent`]s from the transport layer.
    fn subscribe_ticket_events(&self) -> impl Stream<Item = TicketEvent> + Send + 'static;

    /// Reports the current health of the ticket management component.
    fn status(&self) -> ComponentStatus;
}
