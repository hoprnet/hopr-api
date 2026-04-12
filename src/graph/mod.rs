pub mod costs;
pub mod traits;
pub mod types;

pub use traits::{
    CostFn, EdgeImmediateProtocolObservable, EdgeLinkObservable, NetworkGraphTraverse, NetworkGraphUpdate,
    NetworkGraphView, NetworkGraphWrite,
};
pub use types::*;

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
