pub mod costs;
pub mod traits;
pub mod types;

pub use traits::{
    CostFn, EdgeImmediateProtocolObservable, EdgeLinkObservable, EdgeObservable, EdgeObservableRead,
    NetworkGraphTraverse, NetworkGraphUpdate, NetworkGraphView, NetworkGraphWrite,
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
