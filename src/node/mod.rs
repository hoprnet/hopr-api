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

/// General operations performed by a HOPR node.
pub trait HoprNodeOperations {
    /// Returns the [runtime status](state::HoprState) of the node.
    fn status(&self) -> state::HoprState;
}

/// Allows chaining two errors `E1` and `E2` into a single error type
/// that acts transparently.
#[derive(Debug, thiserror::Error)]
pub enum CompoundError<E1, E2>
where
    E1: std::error::Error + Send + Sync + 'static,
    E2: std::error::Error + Send + Sync + 'static,
{
    /// The first error.
    #[error(transparent)]
    Left(E1),
    /// The second error.
    #[error(transparent)]
    Right(E2),
}

impl<E1, E2> CompoundError<E1, E2>
where
    E1: std::error::Error + Send + Sync + 'static,
    E2: std::error::Error + Send + Sync + 'static,
{
    pub fn left<E: Into<E1>>(err: E) -> Self {
        Self::Left(err.into())
    }

    pub fn right<E: Into<E2>>(err: E) -> Self {
        Self::Right(err.into())
    }
}

/// Simple alias `Result<T, CompoundError<E1, E2>>`.
pub type CompoundResult<T, E1, E2> = Result<T, CompoundError<E1, E2>>;
