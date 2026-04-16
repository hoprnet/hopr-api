//! High-level HOPR node API trait definitions.
//!
//! This module defines the external public API interface for interacting with a running HOPR node.
//!
//! ## Architecture
//!
//! The API is structured around **accessor traits** (`Has*`) that provide typed references
//! to individual components, and **composed traits** that are blanket-implemented over
//! combinations of accessors:
//!
//! - [`HasChainApi`] — chain interaction
//! - [`HasNetworkView`] — network connectivity (read-only)
//! - [`HasGraphView`] — network graph (read-only)
//! - [`HasTransportApi`] — transport operations (ping, peer observations)
//! - [`HasTicketManagement`] — ticket processing
//!
//! Composed traits:
//! - [`HoprIncentiveOperations`] — channels + balances + ticket redemption
//! - [`HoprNodeNetworkOperations`] — network health + connectivity
//! - [`HoprNodeChainNetworkOperationsExt`] — cross-domain peer identity translation

mod accessors;
mod incentive;
mod network;
mod state;
mod status;
mod transport;
mod types;

pub use accessors::*;
pub use incentive::*;
pub use multiaddr::PeerId;
pub use network::*;
pub use state::*;
pub use status::*;
pub use transport::*;
pub use types::*;

pub use crate::chain::{ChainInfo, ChannelId};

/// General operations performed by a HOPR node.
pub trait HoprNodeOperations {
    /// Returns the [runtime status](HoprState) of the node.
    fn status(&self) -> HoprState;
}

// ---------------------------------------------------------------------------
// Error compounding utilities
// ---------------------------------------------------------------------------

/// Allows combining two errors `L` and `R` into a single error type
/// that acts transparently.
#[derive(Debug, Clone, Copy, thiserror::Error, strum::EnumTryAs)]
pub enum EitherErr<L: std::error::Error, R: std::error::Error> {
    /// The left error.
    #[error(transparent)]
    Left(L),
    /// The right error.
    #[error(transparent)]
    Right(R),
}

impl<L: std::error::Error, R: std::error::Error> EitherErr<L, R> {
    /// Creates a new [`EitherErr::Left`] with the given error.
    #[inline]
    pub fn left<E: Into<L>>(err: E) -> Self {
        Self::Left(err.into())
    }

    /// Creates a new [`EitherErr::Right`] with the given error.
    #[inline]
    pub fn right<E: Into<R>>(err: E) -> Self {
        Self::Right(err.into())
    }
}

/// Extension trait for converting an error into an [`EitherErr`].
pub trait EitherErrExt: std::error::Error {
    /// Converts this error into [`EitherErr::Left`].
    #[inline]
    fn into_left<R: std::error::Error>(self) -> EitherErr<Self, R>
    where
        Self: Sized,
    {
        EitherErr::Left(self)
    }
    /// Converts this error into [`EitherErr::Right`].
    #[inline]
    fn into_right<L: std::error::Error>(self) -> EitherErr<L, Self>
    where
        Self: Sized,
    {
        EitherErr::Right(self)
    }
}

impl<T: ?Sized + std::error::Error> EitherErrExt for T {}

/// Simple alias [`Result<T, EitherErr<E1, E2>>`](EitherErr).
pub type CompoundResult<T, E1, E2> = Result<T, EitherErr<E1, E2>>;

// ---------------------------------------------------------------------------
// Cross-domain operations (chain + network)
// ---------------------------------------------------------------------------

/// Chain key resolution operations.
///
/// Automatically implemented for types with [`HasChainApi`].
pub trait HoprChainKeyOperationsExt: HasChainApi {
    /// Translates an off-chain public key to the corresponding on-chain address.
    fn offchain_key_to_chain_key(
        &self,
        offchain_key: &crate::OffchainPublicKey,
    ) -> Result<Option<crate::Address>, <Self::ChainApi as crate::chain::HoprChainApi>::ChainError> {
        use crate::chain::ChainKeyOperations;
        self.chain_api().packet_key_to_chain_key(offchain_key)
    }

    /// Translates an on-chain address to the corresponding off-chain public key and PeerId.
    fn chain_key_to_peerid<A: Into<crate::Address> + Send>(
        &self,
        address: A,
    ) -> Result<Option<PeerId>, <Self::ChainApi as crate::chain::HoprChainApi>::ChainError> {
        use crate::chain::ChainKeyOperations;
        self.chain_api()
            .chain_key_to_packet_key(&address.into())
            .map(|pk| pk.map(|v| v.into()))
    }
}

/// Blanket implementation for all types with chain access.
impl<T> HoprChainKeyOperationsExt for T where T: ?Sized + HasChainApi {}
