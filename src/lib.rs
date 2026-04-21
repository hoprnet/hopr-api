#![doc = include_str!("../README.md")]

/// On-chain operations-related API traits.
#[cfg(feature = "chain")]
pub mod chain;
/// Cover traffic-related API traits.
#[cfg(feature = "ct")]
pub mod ct;
/// Network graph related API traits.
#[cfg(feature = "graph")]
pub mod graph;
/// Network state and peer observation API traits.
#[cfg(feature = "network")]
pub mod network;
/// High-level HOPR node API traits.
#[cfg(feature = "node")]
pub mod node;
/// Winning ticket management API traits.
#[cfg(feature = "tickets")]
pub mod tickets;

pub use hopr_types as types;
pub use hopr_types::{
    crypto::prelude::{ChainKeypair, OffchainKeypair, OffchainPublicKey},
    primitive::prelude::{Address, HoprBalance, WxHOPR, XDai, XDaiBalance},
};
pub use libp2p_identity::PeerId;
pub use multiaddr::Multiaddr;
