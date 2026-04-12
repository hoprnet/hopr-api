use futures::{Stream, TryStreamExt};
use hopr_types::{
    internal::prelude::{RedeemableTicket, Ticket},
    primitive::{balance::HoprBalance, prelude::Address},
};

use crate::{
    chain::{ChannelSelector, HoprChainApi},
    node::{CompoundError, CompoundResult, HoprNodeChainOperations},
    tickets::{ChannelStats, RedemptionResult, TicketManagement, TicketManagementExt},
};

/// Ticket events emitted from the packet processing pipeline.
#[derive(Debug, Clone, strum::EnumIs, strum::EnumTryAs)]
pub enum TicketEvent {
    /// A winning ticket was received.
    WinningTicket(Box<RedeemableTicket>),
    /// A ticket has been rejected.
    ///
    /// The optional address represents the ticket issuer and is present only
    /// if the ticket could be at least successfully verified.
    RejectedTicket(Box<Ticket>, Option<Address>),
}

/// Trait implemented by nodes that support processing of tickets.
///
/// These are typically Relay nodes.
#[async_trait::async_trait]
pub trait HoprNodeTicketOperations: HoprNodeChainOperations {
    /// Implementation of [`TicketManagement`]
    type TicketManager: TicketManagement + Clone + Send + Sync + 'static;

    /// How long before the channel closure grace period elapses should we still try to redeem tickets?
    const PENDING_TO_CLOSE_REDEMPTION_TOLERANCE: std::time::Duration = std::time::Duration::from_secs(30);

    /// Returns a stream of [`TicketEvents`](TicketEvent) from the underlying transport.
    fn subscribe_ticket_events(&self) -> impl Stream<Item = TicketEvent> + Send + 'static;

    /// Returns a reference to the underlying ticket management implementation.
    fn ticket_management(&self) -> &Self::TicketManager;

    /// Returns [`ChannelStats`] for all incoming channels which have tickets in them,
    /// or had neglected tickets.
    fn ticket_statistics(
        &self,
    ) -> CompoundResult<
        ChannelStats,
        <<Self as HoprNodeChainOperations>::ChainApi as HoprChainApi>::ChainError,
        <Self::TicketManager as TicketManagement>::Error,
    > {
        self.ticket_management()
            .ticket_stats(None)
            .map_err(CompoundError::right)
    }

    /// Redeems all redeemable tickets in all incoming channels.
    ///
    /// Tickets with a value lower than `min_value` are neglected and lost forever.
    async fn redeem_all_tickets<B: Into<HoprBalance> + Send>(
        &self,
        min_value: B,
    ) -> CompoundResult<
        Vec<RedemptionResult>,
        <<Self as HoprNodeChainOperations>::ChainApi as HoprChainApi>::ChainError,
        <Self::TicketManager as TicketManagement>::Error,
    > {
        let min_value = min_value.into();

        self.ticket_management()
            .redeem_in_channels(
                self.chain_api().clone(),
                None,
                min_value.into(),
                Some(Self::PENDING_TO_CLOSE_REDEMPTION_TOLERANCE),
            )
            .await
            .map_err(CompoundError::left)?
            .try_collect::<Vec<_>>()
            .await
            .map_err(CompoundError::right)
    }

    /// Redeems all incoming tickets from the given issuer.
    ///
    /// Tickets with a value lower than `min_value` are neglected.
    ///
    /// Returns an error if the given `issuer` has not opened an incoming channel to this node.
    ///
    /// To redeem tickets for a specific channel ID, the user must first
    /// [retrieve](HoprNodeChainOperations::channel_by_id) the channel to find out its source (= ticket issuer).
    async fn redeem_tickets_with_counterparty<A: Into<Address> + Send, B: Into<HoprBalance> + Send>(
        &self,
        issuer: A,
        min_value: B,
    ) -> CompoundResult<
        Vec<RedemptionResult>,
        <<Self as HoprNodeChainOperations>::ChainApi as HoprChainApi>::ChainError,
        <Self::TicketManager as TicketManagement>::Error,
    > {
        let min_value = min_value.into();

        self.ticket_management()
            .redeem_in_channels(
                self.chain_api().clone(),
                ChannelSelector::default()
                    .with_source(issuer)
                    .with_destination(self.identity().node_address)
                    .into(),
                min_value.into(),
                Some(Self::PENDING_TO_CLOSE_REDEMPTION_TOLERANCE),
            )
            .await
            .map_err(CompoundError::left)?
            .try_collect::<Vec<_>>()
            .await
            .map_err(CompoundError::right)
    }
}
