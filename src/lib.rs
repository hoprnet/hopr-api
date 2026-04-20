//! Common high-level external and internal API traits for the HOPR protocol.
//!
//! This crate defines **trait-based interfaces** that separate API contract from implementation.
//! Concrete implementations live in their respective crates (`hopr-lib`, `hopr-transport`, etc.)
//! and depend on the traits defined here.
//!
//! ## Module Organization
//!
//! - [`chain`] — On-chain operations: channel management, account queries, safe operations, event subscriptions
//! - [`ct`] — Cover traffic and probing traffic generation
//! - [`graph`] — Network graph: topology view, pathfinding, edge quality observations
//! - [`network`] — Network layer: peer connectivity, health, stream control
//! - [`node`] — High-level node API: accessor traits (`Has*`), composed operations, session client
//! - [`tickets`] — Winning ticket management and redemption
//!
//! ## Design Principle
//!
//! The interface mandates trait behavior defined in this crate and does not rely on
//! specific types outside of this crate. External types (from `hopr-types`) are
//! re-exported at the crate root for convenience.

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
