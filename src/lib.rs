// Crate-level documentation is sourced from the README.
#![doc = include_str!("../README.md")]

/// Maximum usable payload size in bytes for a single HOPR packet.
///
/// HOPR packets use a fixed-size SPHINX envelope. After subtracting the
/// SPHINX padding byte, the per-packet header (`PacketMessage::HEADER_LEN = 1`
/// byte for the SURB-count / flag field), the remaining bytes are available
/// to the application layer.
///
/// Derivation (see `hopr-crypto-packet`):
///
/// ```text
/// DefaultSphinxPacketSize = 1024 + 14 = 1038  (typenum arithmetic)
/// PAYLOAD_SIZE_INT        = 1038 - 1  = 1037  (minus SPHINX padding byte)
/// PACKET_PAYLOAD_SIZE     = 1037 - 1  = 1036  (minus PacketMessage header byte)
/// ```
///
/// This corresponds to `HoprPacket::PAYLOAD_SIZE` in `hopr-crypto-packet`.
pub const PACKET_PAYLOAD_SIZE: usize = 1036;

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
