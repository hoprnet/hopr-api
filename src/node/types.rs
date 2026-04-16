//! Data types used across the node API.

use std::future::Future;

use hopr_types::{
    chain::chain_events::ChainEvent,
    crypto::prelude::Hash,
    internal::prelude::{RedeemableTicket, Ticket},
    primitive::prelude::Address,
};

use super::CompoundResult;

/// Identity of a node on-chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct NodeOnchainIdentity {
    /// Node's on-chain address.
    pub node_address: Address,
    /// Address of the node's associated Safe.
    pub safe_address: Address,
    /// Address of the Safe module.
    pub module_address: Address,
}

/// Represents an output of a write operation to the chain performed by the node.
///
/// This consists always of a transaction hash and an optional output `T`.
///
/// Operations that produce no useful output use `()` as `T`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ChainOutput<T> {
    tx_hash: hopr_types::crypto::prelude::Hash,
    output: Option<T>,
}

impl<T> ChainOutput<T> {
    /// Creates a new ChainOutput with the given transaction hash and output.
    pub fn new(tx_hash: hopr_types::crypto::prelude::Hash, output: T) -> Self {
        Self {
            tx_hash,
            output: output.into(),
        }
    }

    /// Returns the transaction hash of the chain operation.
    pub fn tx_hash(&self) -> &hopr_types::crypto::prelude::Hash {
        &self.tx_hash
    }

    /// Returns the optional output of the chain operation.
    pub fn output(&self) -> Option<&T> {
        self.output.as_ref()
    }
}

impl ChainOutput<()> {
    /// Creates a new ChainOutput with the given transaction hash and no output.
    pub fn new_empty(tx_hash: hopr_types::crypto::prelude::Hash) -> Self {
        Self { tx_hash, output: None }
    }
}

impl From<hopr_types::crypto::prelude::Hash> for ChainOutput<()> {
    fn from(tx_hash: hopr_types::crypto::prelude::Hash) -> Self {
        Self::new_empty(tx_hash)
    }
}

/// Future that resolves when a [`ChainEvent`] is resolved, times out, or is aborted
/// via the associated abort handle.
pub type ChainEventResolver<ChainErr, WaitErr> = (
    std::pin::Pin<Box<dyn Future<Output = CompoundResult<ChainEvent, ChainErr, WaitErr>> + Send + 'static>>,
    futures::future::AbortHandle,
);

/// Alias for the result of [`HasChainApi::wait_for_on_chain_event`](super::HasChainApi::wait_for_on_chain_event).
pub type EventWaitResult<ChainErr, WaitErr> = Result<ChainEventResolver<ChainErr, WaitErr>, ChainErr>;

/// Ticket events emitted from the packet processing pipeline.
#[derive(Debug, Clone, strum::EnumIs, strum::EnumTryAs)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum TicketEvent {
    /// A winning ticket was received.
    WinningTicket(Box<RedeemableTicket>),
    /// A ticket has been rejected.
    ///
    /// The optional address represents the ticket issuer and is present only
    /// if the ticket could be at least successfully verified.
    RejectedTicket(Box<Ticket>, Option<Address>),
}
