//! Actionable event API for strategy and automation consumption.
//!
//! Strategies use [`ActionableEventSource::subscribe_to_actionable_events`] to obtain
//! a unified stream of every event that may trigger an automated node action.
use futures::stream::BoxStream;

use crate::{
    chain::ChainEvent,
    network::NetworkEvent,
    node::TicketEvent,
};

/// Unified event type for strategy consumption.
///
/// Every event a strategy may react to is represented as a variant here.
///
/// The stream is **unfiltered** — strategies receive events for all channels
/// and all peers, not only those involving the local node. A strategy that
/// only cares about its own channels can filter with `channel.direction(&me)`
/// locally.
#[derive(Debug, Clone)]
pub enum ActionableEvent {
    /// An on-chain event from the indexer.
    ///
    /// Covers all [`ChainEvent`] variants.
    Chain(ChainEvent),

    /// A network-layer connectivity event.
    ///
    /// Emitted when a network level observations relevant
    /// for outside operations happens.
    Network(NetworkEvent),

    /// A ticket pipeline event.
    ///
    /// Includes the actual ticket.
    Ticket(TicketEvent),
}

/// Provides a merged stream of all actionable node events.
///
/// This trait is implemented by a HOPR node and gives strategies a single
/// subscription point that unifies on-chain events, network connectivity events,
/// and ticket pipeline events.
///
/// Each call to [`subscribe_to_actionable_events`] returns an **independent**
/// stream backed by its own broadcast receiver, so multiple concurrent
/// strategies each receive every event without interfering with each other.
///
/// [`subscribe_to_actionable_events`]: ActionableEventSource::subscribe_to_actionable_events
#[auto_impl::auto_impl(&, Arc)]
pub trait ActionableEventSource {
    /// Subscribe to the unified stream of actionable events.
    ///
    /// Returns a boxed, `'static` stream that yields [`ActionableEvent`]s until
    /// the node shuts down. It should terminate only when a node terminates.
    ///
    /// # Errors
    ///
    /// Returns an error if the underlying broadcast channel is closed or the
    /// subscription cannot otherwise be established.
    fn subscribe_to_actionable_events(&self) -> Result<BoxStream<'static, ActionableEvent>, String>;
}
