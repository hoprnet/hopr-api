//! High-level HOPR node API trait definitions.
//!
//! This module defines the external public API interface for interacting with a running HOPR node.

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
    /// Returns the [runtime status](HoprState) of the node.
    fn status(&self) -> HoprState;
}

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
