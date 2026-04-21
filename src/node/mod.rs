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
//! - `HasChainApi` — chain interaction
//! - `HasNetworkView` — network connectivity (read-only [`NetworkView`](crate::network::NetworkView))
//! - `HasGraphView` — network graph (read-only)
//! - `HasTransportApi` — transport operations (ping, observed multiaddresses)
//! - `HasTicketManagement` — ticket processing
//!
//! Composed traits:
//! - `IncentiveChannelOperations` — channels, balances, withdrawals, chain info
//! - `IncentiveRedeemOperations` — ticket redemption and statistics (relay nodes only)

mod accessors;
mod incentive;
#[cfg(any(feature = "node-session-client", feature = "node-session-server"))]
/// Session APIs for outgoing/incoming HOPR sessions.
pub mod session;
mod state;
mod status;
mod transport;
mod types;

pub use accessors::*;
pub use incentive::*;
#[cfg(feature = "node-session-client")]
pub use session::client::*;
#[cfg(feature = "node-session-server")]
pub use session::server::*;
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
