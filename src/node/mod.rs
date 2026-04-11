//! High-level HOPR node API trait definitions.
//!
//! This module defines the external API interface for interacting with a running HOPR node.

mod chain;
mod network;
mod state;
mod tickets;

pub use chain::*;
pub use multiaddr::PeerId;
pub use network::*;
pub use state::*;
pub use tickets::*;

pub use crate::chain::{ChainInfo, ChannelId};

pub trait HoprNodeOperations {
    /// Returns the [runtime status](state::HoprState) of the node.
    fn status(&self) -> state::HoprState;
}
