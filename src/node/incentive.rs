//! Unified incentive operations: channels, balances, withdrawals, and ticket redemption.
//!
//! This trait is automatically implemented for any type providing both
//! [`HasChainApi`] and [`HasTicketManagement`] access.

use std::convert::identity;

use futures::{StreamExt, TryFutureExt, TryStreamExt};
use hopr_types::{internal::prelude::*, primitive::prelude::*};

use crate::{
    chain::{
        AccountSelector, ChainInfo, ChainReadAccountOperations, ChainReadChannelOperations,
        ChainReadSafeOperations, ChainValues, ChainWriteAccountOperations, ChainWriteChannelOperations,
        ChannelSelector, HoprChainApi,
    },
    node::{ChainOutput, CompoundResult, EitherErr, accessors::{HasChainApi, HasTicketManagement}},
    tickets::{ChannelStats, RedemptionResult, TicketManagement, TicketManagementExt},
};

use super::ChannelId;

/// Unified incentive operations combining channel management, balance queries,
/// withdrawals, and ticket redemption.
///
/// All methods have default implementations that delegate to [`HasChainApi`]
/// and [`HasTicketManagement`] accessors.
#[async_trait::async_trait]
pub trait HoprIncentiveOperations: HasChainApi + HasTicketManagement {
    /// Timeout multiplier applied to [`ChainValues::typical_resolution_time`]
    /// when waiting for on-chain operations to be confirmed via the event bus.
    const CHAIN_OPERATION_TIMEOUT_MULTIPLIER: u32 = 2;

    /// How long before the channel closure grace period elapses should we still try to redeem tickets.
    const PENDING_TO_CLOSE_REDEMPTION_TOLERANCE: std::time::Duration = std::time::Duration::from_secs(30);

    // -----------------------------------------------------------------------
    // Channel operations
    // -----------------------------------------------------------------------

    /// Opens a channel from the node to the given `destination` with the given `amount` as the initial stake.
    ///
    /// On success, returns the ID of the new channel as [output](ChainOutput).
    /// Returns an error if the channel exists and is not closed, or the operation times out.
    async fn open_channel<A: Into<Address> + Send>(
        &self,
        destination: A,
        amount: HoprBalance,
    ) -> CompoundResult<ChainOutput<ChannelId>, <Self::ChainApi as HoprChainApi>::ChainError, Self::ChainError> {
        let destination = destination.into();
        let channel_id = generate_channel_id(&self.identity().node_address, &destination);

        let (event_awaiter, event_abort) = self
            .wait_for_on_chain_event(
                move |event| matches!(event, hopr_types::chain::chain_events::ChainEvent::ChannelOpened(c) if c.get_id() == &channel_id),
                format!("open channel to {destination} ({channel_id})"),
                Self::CHAIN_OPERATION_TIMEOUT_MULTIPLIER
                    * self
                        .chain_api()
                        .typical_resolution_time()
                        .await
                        .map_err(EitherErr::left)?,
            )
            .map_err(EitherErr::left)?;

        let confirm_awaiter = self
            .chain_api()
            .open_channel(&destination, amount)
            .await
            .map_err(EitherErr::left)?;

        let tx_hash = confirm_awaiter
            .await
            .inspect_err(|_| {
                event_abort.abort();
            })
            .map_err(EitherErr::left)?;

        let event = event_awaiter.await?;
        tracing::debug!(%event, "open channel event received");

        Ok(ChainOutput::new(tx_hash, channel_id))
    }

    /// Funds an existing channel with the given `amount`.
    ///
    /// Returns an error if the channel does not exist or is not [opened](ChannelStatus).
    async fn fund_channel(
        &self,
        channel_id: &ChannelId,
        amount: HoprBalance,
    ) -> CompoundResult<ChainOutput<()>, <Self::ChainApi as HoprChainApi>::ChainError, Self::ChainError> {
        let channel_id = *channel_id;

        let (event_awaiter, event_abort) = self.wait_for_on_chain_event(
            move |event| matches!(event, hopr_types::chain::chain_events::ChainEvent::ChannelBalanceIncreased(c, a) if c.get_id() == &channel_id && a == &amount),
            format!("fund channel {channel_id}"),
            Self::CHAIN_OPERATION_TIMEOUT_MULTIPLIER *
                self.chain_api().typical_resolution_time().await.map_err(EitherErr::left)?
        ).map_err(EitherErr::left)?;

        let confirm_awaiter = self
            .chain_api()
            .fund_channel(&channel_id, amount)
            .await
            .map_err(EitherErr::left)?;

        let res = confirm_awaiter
            .await
            .inspect_err(|_| {
                event_abort.abort();
            })
            .map_err(EitherErr::left)?;

        let event = event_awaiter.await?;
        tracing::debug!(%event, "fund channel event received");

        Ok(res.into())
    }

    /// Initiates or finalizes the closure of a channel with the given `channel_id`.
    ///
    /// Returns an error if the channel does not exist, or its closure has been already finalized.
    /// On success, returns the new [status](ChannelStatus) of the channel as [output](ChainOutput).
    async fn close_channel_by_id(
        &self,
        channel_id: &ChannelId,
    ) -> CompoundResult<ChainOutput<ChannelStatus>, <Self::ChainApi as HoprChainApi>::ChainError, Self::ChainError>
    {
        let channel_id = *channel_id;

        let (event_awaiter, event_abort) = self
            .wait_for_on_chain_event(
                move |event| {
                    use hopr_types::chain::chain_events::ChainEvent;
                    matches!(event, ChainEvent::ChannelClosed(c) if c.get_id() == &channel_id)
                        || matches!(event, ChainEvent::ChannelClosureInitiated(c) if c.get_id() == &channel_id)
                },
                format!("close channel {channel_id}"),
                Self::CHAIN_OPERATION_TIMEOUT_MULTIPLIER
                    * self
                        .chain_api()
                        .typical_resolution_time()
                        .await
                        .map_err(EitherErr::left)?,
            )
            .map_err(EitherErr::left)?;

        let confirm_awaiter = self
            .chain_api()
            .close_channel(&channel_id)
            .await
            .map_err(EitherErr::left)?;

        let tx_hash = confirm_awaiter
            .await
            .inspect_err(|_| {
                event_abort.abort();
            })
            .map_err(EitherErr::left)?;

        let event = event_awaiter.await?;
        tracing::debug!(%event, "close channel event received");

        let status = match event {
            hopr_types::chain::chain_events::ChainEvent::ChannelClosureInitiated(c)
            | hopr_types::chain::chain_events::ChainEvent::ChannelClosed(c) => c.status,
            _ => unreachable!(),
        };

        Ok(ChainOutput::new(tx_hash, status))
    }

    /// Returns a channel with the given `channel_id`.
    fn channel_by_id(
        &self,
        channel_id: &ChannelId,
    ) -> Result<Option<ChannelEntry>, <Self::ChainApi as HoprChainApi>::ChainError> {
        self.chain_api().channel_by_id(channel_id)
    }

    /// Returns a channel between `source` and `destination`.
    fn channel<A: Into<Address> + Send, B: Into<Address> + Send>(
        &self,
        source: A,
        destination: B,
    ) -> Result<Option<ChannelEntry>, <Self::ChainApi as HoprChainApi>::ChainError> {
        self.chain_api().channel_by_parties(&source.into(), &destination.into())
    }

    /// Returns all channels to the given `destination`.
    async fn channels_to<A: Into<Address> + Send>(
        &self,
        destination: A,
    ) -> Result<Vec<ChannelEntry>, <Self::ChainApi as HoprChainApi>::ChainError> {
        let dest = destination.into();
        Ok(self
            .chain_api()
            .stream_channels(ChannelSelector::default().with_destination(dest).with_allowed_states(&[
                ChannelStatusDiscriminants::Closed,
                ChannelStatusDiscriminants::Open,
                ChannelStatusDiscriminants::PendingToClose,
            ]))?
            .collect()
            .await)
    }

    /// Returns all channels from the given `source`.
    async fn channels_from<A: Into<Address> + Send>(
        &self,
        source: A,
    ) -> Result<Vec<ChannelEntry>, <Self::ChainApi as HoprChainApi>::ChainError> {
        let src = source.into();
        Ok(self
            .chain_api()
            .stream_channels(ChannelSelector::default().with_source(src).with_allowed_states(&[
                ChannelStatusDiscriminants::Closed,
                ChannelStatusDiscriminants::Open,
                ChannelStatusDiscriminants::PendingToClose,
            ]))?
            .collect()
            .await)
    }

    // -----------------------------------------------------------------------
    // Balance & withdrawal
    // -----------------------------------------------------------------------

    /// Returns the balance of [`Currency`] in the node's account.
    async fn get_balance<C: Currency + Send>(
        &self,
    ) -> Result<Balance<C>, <Self::ChainApi as HoprChainApi>::ChainError> {
        self.chain_api().balance(self.identity().node_address).await
    }

    /// Returns the balance of [`Currency`] in the node's Safe.
    async fn get_safe_balance<C: Currency + Send>(
        &self,
    ) -> Result<Balance<C>, <Self::ChainApi as HoprChainApi>::ChainError> {
        self.chain_api().balance(self.identity().safe_address).await
    }

    /// Returns the allowance of the node's Safe to spend funds in channels.
    async fn safe_allowance(&self) -> Result<HoprBalance, <Self::ChainApi as HoprChainApi>::ChainError> {
        self.chain_api().safe_allowance(self.identity().safe_address).await
    }

    /// Withdraws the given `amount` of [`Currency`] from the node to the `recipient`.
    async fn withdraw<C: Currency + Send>(
        &self,
        recipient: &Address,
        amount: Balance<C>,
    ) -> Result<ChainOutput<()>, <Self::ChainApi as HoprChainApi>::ChainError> {
        Ok(self
            .chain_api()
            .withdraw(amount, recipient)
            .and_then(identity)
            .await?
            .into())
    }

    // -----------------------------------------------------------------------
    // Chain info shortcuts
    // -----------------------------------------------------------------------

    /// Returns information about the connected blockchain.
    async fn chain_info(&self) -> Result<ChainInfo, <Self::ChainApi as HoprChainApi>::ChainError> {
        self.chain_api().chain_info().await
    }

    /// Returns the minimum ticket price from the blockchain.
    async fn get_ticket_price(&self) -> Result<HoprBalance, <Self::ChainApi as HoprChainApi>::ChainError> {
        self.chain_api().minimum_ticket_price().await
    }

    /// Returns the minimum win probability of an incoming ticket.
    async fn get_minimum_incoming_ticket_win_probability(
        &self,
    ) -> Result<WinningProbability, <Self::ChainApi as HoprChainApi>::ChainError> {
        self.chain_api().minimum_incoming_ticket_win_prob().await
    }

    /// Returns the channel closure grace period.
    async fn get_channel_closure_notice_period(
        &self,
    ) -> Result<std::time::Duration, <Self::ChainApi as HoprChainApi>::ChainError> {
        self.chain_api().channel_closure_notice_period().await
    }

    // -----------------------------------------------------------------------
    // Ticket redemption
    // -----------------------------------------------------------------------

    /// Redeems all redeemable tickets in all incoming channels.
    ///
    /// Tickets with a value lower than `min_value` are neglected and lost forever.
    async fn redeem_all_tickets<B: Into<HoprBalance> + Send>(
        &self,
        min_value: B,
    ) -> CompoundResult<
        Vec<RedemptionResult>,
        <Self::ChainApi as HoprChainApi>::ChainError,
        <<Self as HasTicketManagement>::TicketManager as TicketManagement>::Error,
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
            .map_err(EitherErr::left)?
            .try_collect::<Vec<_>>()
            .await
            .map_err(EitherErr::right)
    }

    /// Redeems all incoming tickets from the given issuer.
    ///
    /// Tickets with a value lower than `min_value` are neglected.
    async fn redeem_tickets_with_counterparty<A: Into<Address> + Send, B: Into<HoprBalance> + Send>(
        &self,
        issuer: A,
        min_value: B,
    ) -> CompoundResult<
        Vec<RedemptionResult>,
        <Self::ChainApi as HoprChainApi>::ChainError,
        <<Self as HasTicketManagement>::TicketManager as TicketManagement>::Error,
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
            .map_err(EitherErr::left)?
            .try_collect::<Vec<_>>()
            .await
            .map_err(EitherErr::right)
    }

    /// Returns [`ChannelStats`] for all incoming channels which have tickets.
    fn ticket_statistics(
        &self,
    ) -> CompoundResult<
        ChannelStats,
        <Self::ChainApi as HoprChainApi>::ChainError,
        <<Self as HasTicketManagement>::TicketManager as TicketManagement>::Error,
    > {
        self.ticket_management().ticket_stats(None).map_err(EitherErr::right)
    }

    // -----------------------------------------------------------------------
    // Announced peers
    // -----------------------------------------------------------------------

    /// Returns all peers that have been publicly announced on-chain.
    async fn announced_peers(&self) -> Result<Vec<AccountEntry>, <Self::ChainApi as HoprChainApi>::ChainError> {
        Ok(self
            .chain_api()
            .stream_accounts(AccountSelector {
                public_only: true,
                ..Default::default()
            })?
            .collect()
            .await)
    }
}

/// Blanket implementation: any type with chain and ticket access gets full incentive operations.
impl<T> HoprIncentiveOperations for T where T: HasChainApi + HasTicketManagement + Send + Sync {}
