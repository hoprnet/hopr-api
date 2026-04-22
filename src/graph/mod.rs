//! Network graph API traits: topology, pathfinding, and edge quality observations.
//!
//! - `NetworkGraphView` — read-only node/edge queries and graph identity
//! - `NetworkGraphConnectivity` — topology enumeration (connected/reachable edges)
//! - `NetworkGraphWrite` — graph mutation (add/remove nodes and edges)
//! - `NetworkGraphUpdate` — record measurements from probes and transport
//! - `NetworkGraphTraverse` — pathfinding (simple paths, loopbacks)
//! - `HoprGraphApi` — composite of all graph traits (full read+write access)
//! - `HoprGraphReadApi` — composite of view+traverse (read-only access)
//! - `ValueFn` — value function trait for path selection
//! - Edge observable traits for quality measurements (QoS, latency, capacity)

/// Edge/path value-function utilities used by graph traversal.
pub mod function;
/// Graph operation traits and observability interfaces.
pub mod traits;
/// Shared graph telemetry and measurement types.
pub mod types;

pub use traits::{
    EdgeImmediateProtocolObservable, EdgeLinkObservable, EdgeObservable, EdgeObservableRead, NetworkGraphConnectivity,
    NetworkGraphTraverse, NetworkGraphUpdate, NetworkGraphView, NetworkGraphWrite, ValueFn,
};
pub use types::*;

/// Read-only graph API for external consumers.
///
/// This trait is automatically implemented for types
/// that implement both [`NetworkGraphView`] and [`NetworkGraphTraverse`]
/// with the same node id.
pub trait HoprGraphReadApi:
    NetworkGraphView<NodeId = Self::HoprNodeId> + NetworkGraphTraverse<NodeId = Self::HoprNodeId>
{
    type HoprNodeId: Send;
}

impl<T, N> HoprGraphReadApi for T
where
    T: NetworkGraphView<NodeId = N> + NetworkGraphTraverse<NodeId = N>,
    N: Send,
{
    type HoprNodeId = N;
}

/// Complete set of HOPR graph operation APIs.
///
/// This trait is automatically implemented for types
/// that implement all the individual graph API traits with the same node id.
pub trait HoprGraphApi:
    NetworkGraphView<NodeId = Self::HoprNodeId>
    + NetworkGraphUpdate
    + NetworkGraphWrite<NodeId = Self::HoprNodeId>
    + NetworkGraphTraverse<NodeId = Self::HoprNodeId>
{
    type HoprNodeId: Send;
}

impl<T, N> HoprGraphApi for T
where
    T: NetworkGraphView<NodeId = N>
        + NetworkGraphUpdate
        + NetworkGraphWrite<NodeId = N>
        + NetworkGraphTraverse<NodeId = N>,
    N: Send,
{
    type HoprNodeId = N;
}
