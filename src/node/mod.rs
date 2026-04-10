//! High-level HOPR node API trait definitions.
//!
//! This module defines the external API interface for interacting with a running HOPR node.
//! The `HoprNodeNetworkOperations` and `HoprNodeOperations` traits provide the
//! operations available to external consumers, abstracting over implementation details.

pub mod state;
pub mod incentives;
pub mod network;

use hopr_types::{crypto::prelude::Hash, primitive::prelude::Address};
pub use multiaddr::PeerId;

pub use crate::chain::ChainInfo;
use crate::{chain::ChannelId, graph::traits::EdgeObservable, network::Health};

/// Result of opening a channel on-chain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenChannelResult {
    /// Transaction hash of the channel open operation.
    pub tx_hash: Hash,
    /// The ID of the opened channel.
    pub channel_id: ChannelId,
}

/// Result of closing a channel on-chain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloseChannelResult {
    /// Transaction hash of the channel close operation.
    pub tx_hash: Hash,
}

/// Configuration for the Safe module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SafeModuleConfig {
    /// Address of the Safe contract.
    pub safe_address: Address,
    /// Address of the module contract.
    pub module_address: Address,
}

pub trait HoprNodeOperations {
    fn status(&self) -> state::HoprState;
}
