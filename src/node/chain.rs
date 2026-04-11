use std::convert::identity;

pub use futures::future::AbortHandle;
use futures::{StreamExt, TryFutureExt, future::BoxFuture};
use hopr_types::{chain::chain_events::ChainEvent, crypto::prelude::Hash, internal::prelude::*, primitive::prelude::*};
use libp2p_identity::PeerId;

use crate::{
    chain::{
        AccountSelector, ChainInfo, ChainKeyOperations, ChainReadAccountOperations, ChainReadChannelOperations,
        ChainReadSafeOperations, ChainValues, ChainWriteAccountOperations, ChainWriteChannelOperations,
        ChannelSelector, HoprChainApi,
    },
    node::network::HoprNodeNetworkOperations,
};

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

/// Result of opening a channel on-chain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenChannelResult {
    /// Transaction hash of the channel open operation.
    pub tx_hash: Hash,
    /// The ID of the opened channel.
    pub channel_id: ChannelId,
}

/// Result of closing a channel on-chain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloseChannelResult {
    /// Transaction hash of the channel close operation.
    pub tx_hash: Hash,
}

/// Future that resolves when a [`ChainEvent`] is resolved, times out, or is aborted
/// via the associated abort handle.
pub type ChainEventResolver<E> = (BoxFuture<'static, Result<ChainEvent, E>>, AbortHandle);

/// Implemented by nodes that support interaction with an [underlying chain](HoprChainApi).
#[async_trait::async_trait]
#[auto_impl::auto_impl(&, Box, Arc)]
pub trait HoprNodeChainOperations {
    /// Error returned by the node's on-chain operations.
    ///
    /// This error must be convertible from [`HoprChainApi::ChainError`].
    type NodeChainError: std::error::Error + From<<Self::ChainApi as HoprChainApi>::ChainError> + Send + Sync + 'static;

    /// Implementation of the [`HoprChainApi`] trait for the underlying chain.
    type ChainApi: HoprChainApi + Clone + Send + Sync + 'static;

    /// Timeout multiplier to [`ChainValues::typical_resolution_time`] used to wait for on-chain operations
    /// to be observed on the event bus.
    ///
    /// Every operation that uses [`wait_for_on_chain_event`](Self::wait_for_on_chain_event) will use this multiplier.
    const CHAIN_OPERATION_TIMEOUT_MULTIPLIER: u32 = 2;

    /// Returns the address of the node's on-chain account.
    fn identity(&self) -> &NodeOnchainIdentity;

    /// Returns reference to the underlying chain API.
    fn chain_api(&self) -> &Self::ChainApi;

    /// Spawns an asynchronous waiter that hooks up to the [`ChainEvent`] [bus](crate::chain::ChainEvents::subscribe) and either
    /// matching the given `predicate` or timing out after `timeout`.
    ///
    /// The implementor decides on the async runtime used to spawn the operation that resolves
    /// with the returned future.
    ///
    /// The `context` contains a human-readable description of the operation being awaited.
    /// The implementors are free to decide whether to use the `context` or not.
    ///
    /// The given `timeout` is usually already pre-multiplied with [`Self::CHAIN_OPERATION_TIMEOUT_MULTIPLIER`].
    fn wait_for_on_chain_event<F>(
        &self,
        predicate: F,
        context: String,
        timeout: std::time::Duration,
    ) -> Result<ChainEventResolver<Self::NodeChainError>, Self::NodeChainError>
    where
        F: Fn(&ChainEvent) -> bool + Send + Sync + 'static;

    /// Opens a channel from the node to the given `destination` with the given `amount` as the initial stake.
    ///
    /// Returns an error if the channel exists and is not closed, or the operation times out.
    async fn open_channel<A: Into<Address> + Send>(
        &self,
        destination: A,
        amount: HoprBalance,
    ) -> Result<OpenChannelResult, Self::NodeChainError> {
        let destination = destination.into();
        let channel_id = generate_channel_id(&self.identity().node_address, &destination);

        // Subscribe to chain events BEFORE sending the transaction to avoid
        // a race where the event is broadcast before the subscriber activates.
        let (event_awaiter, event_abort) = self.wait_for_on_chain_event(
            move |event| matches!(event, ChainEvent::ChannelOpened(c) if c.get_id() == &channel_id),
            format!("open channel to {destination} ({channel_id})"),
            Self::CHAIN_OPERATION_TIMEOUT_MULTIPLIER * self.chain_api().typical_resolution_time().await?,
        )?;

        let confirm_awaiter = self.chain_api().open_channel(&destination, amount).await?;

        let tx_hash = confirm_awaiter.await.inspect_err(|_| {
            event_abort.abort();
        })?;

        let event = event_awaiter.await?;
        tracing::debug!(%event, "open channel event received");

        Ok(OpenChannelResult { tx_hash, channel_id })
    }

    /// Funds an existing channel with the given `amount`.
    ///
    /// Returns an error if the channel does not exist or is not [opened](ChannelStatus).
    async fn fund_channel(&self, channel_id: &ChannelId, amount: HoprBalance) -> Result<Hash, Self::NodeChainError> {
        let channel_id = *channel_id;

        // Subscribe to chain events BEFORE sending the transaction to avoid
        // a race where the event is broadcast before the subscriber activates.
        let (event_awaiter, event_abort) = self.wait_for_on_chain_event(
            move |event| matches!(event, ChainEvent::ChannelBalanceIncreased(c, a) if c.get_id() == &channel_id && a == &amount),
            format!("fund channel {channel_id}"),
            Self::CHAIN_OPERATION_TIMEOUT_MULTIPLIER *
                self.chain_api().typical_resolution_time().await?
        )?;

        let confirm_awaiter = self.chain_api().fund_channel(&channel_id, amount).await?;

        let res = confirm_awaiter.await.inspect_err(|_| {
            event_abort.abort();
        })?;

        let event = event_awaiter.await?;
        tracing::debug!(%event, "fund channel event received");

        Ok(res)
    }

    /// Initiates or finalizes the closure of a channel with the given `channel_id`.
    ///
    /// Returns an error if the channel does not exist, or its closure has been already finalized.
    async fn close_channel_by_id(&self, channel_id: &ChannelId) -> Result<CloseChannelResult, Self::NodeChainError> {
        let channel_id = *channel_id;

        // Subscribe to chain events BEFORE sending the transaction to avoid
        // a race where the event is broadcast before the subscriber activates.
        let (event_awaiter, event_abort) = self.wait_for_on_chain_event(
            move |event| {
                matches!(event, ChainEvent::ChannelClosed(c) if c.get_id() == &channel_id)
                    || matches!(event, ChainEvent::ChannelClosureInitiated(c) if c.get_id() == &channel_id)
            },
            format!("close channel {channel_id}"),
            Self::CHAIN_OPERATION_TIMEOUT_MULTIPLIER * self.chain_api().typical_resolution_time().await?,
        )?;

        let confirm_awaiter = self.chain_api().close_channel(&channel_id).await?;

        let tx_hash = confirm_awaiter.await.inspect_err(|_| {
            event_abort.abort();
        })?;

        let event = event_awaiter.await?;
        tracing::debug!(%event, "close channel event received");

        Ok(CloseChannelResult { tx_hash })
    }

    /// Withdraws the given `amount` of [`Currency`] from the node's Safe or node account to the `recipient`.
    ///
    /// Whether withdrawal is done from the node's Safe or node account depends on the underlying [`HoprChainApi`]
    /// implementation.
    async fn withdraw<C: Currency + Send>(
        &self,
        recipient: &Address,
        amount: Balance<C>,
    ) -> Result<Hash, Self::NodeChainError> {
        Ok(self.chain_api().withdraw(amount, recipient).and_then(identity).await?)
    }

    /// Returns the balance of [`Currency`] in the node's account.
    async fn get_balance<C: Currency + Send>(&self) -> Result<Balance<C>, Self::NodeChainError> {
        Ok(self.chain_api().balance(self.identity().node_address).await?)
    }

    /// Returns the balance of [`Currency`] the node's Safe.
    async fn get_safe_balance<C: Currency + Send>(&self) -> Result<Balance<C>, Self::NodeChainError> {
        Ok(self.chain_api().balance(self.identity().safe_address).await?)
    }

    /// Returns the allowance of the node's Safe to spend funds in channels.
    async fn safe_allowance(&self) -> Result<HoprBalance, Self::NodeChainError> {
        Ok(self.chain_api().safe_allowance(self.identity().safe_address).await?)
    }

    /// Shorthand to retrieve information about the connected blockchain.
    async fn chain_info(&self) -> Result<ChainInfo, Self::NodeChainError> {
        Ok(self.chain_api().chain_info().await?)
    }

    /// Shorthand to retrieve the minimum price of an incoming ticket given by the connected blockchain.
    async fn get_ticket_price(&self) -> Result<HoprBalance, Self::NodeChainError> {
        Ok(self.chain_api().minimum_ticket_price().await?)
    }

    /// Shorthand to retrieve the minimum win probability of an incoming ticket given by the connected blockchain.
    async fn get_minimum_incoming_ticket_win_probability(&self) -> Result<WinningProbability, Self::NodeChainError> {
        Ok(self.chain_api().minimum_incoming_ticket_win_prob().await?)
    }

    /// Shorthand to return the channel closure grace period.
    ///
    /// This is how much time is required for a channel to transition from
    /// `ChannelStatus::PendingToClose` to `ChannelStatus::Closed`.
    async fn get_channel_closure_notice_period(&self) -> Result<std::time::Duration, Self::NodeChainError> {
        Ok(self.chain_api().channel_closure_notice_period().await?)
    }

    /// Returns all peers that have been publicly announced on-chain.
    async fn announced_peers(&self) -> Result<Vec<AccountEntry>, Self::NodeChainError> {
        Ok(self
            .chain_api()
            .stream_accounts(AccountSelector {
                public_only: true,
                ..Default::default()
            })?
            .collect()
            .await)
    }

    /// Returns a channel with the given `channel_id`.
    ///
    /// Depending on the underlying chain implementation [`closed`](ChannelStatus) channels may or may not be returned.
    fn channel_by_id(&self, channel_id: &ChannelId) -> Result<Option<ChannelEntry>, Self::NodeChainError> {
        Ok(self.chain_api().channel_by_id(channel_id)?)
    }

    /// Returns a channel with the given `source` and `destination`.
    ///
    /// Depending on the underlying chain implementation [`closed`](ChannelStatus) channels may or may not be returned.
    fn channel<A: Into<Address> + Send, B: Into<Address> + Send>(
        &self,
        source: A,
        destination: B,
    ) -> Result<Option<ChannelEntry>, Self::NodeChainError> {
        Ok(self
            .chain_api()
            .channel_by_parties(&source.into(), &destination.into())?)
    }

    /// Returns all channels to the given `destination`.
    ///
    /// Depending on the underlying chain implementation [`closed`](ChannelStatus) channels may or may not be returned.
    async fn channels_to<A: Into<Address> + Send>(
        &self,
        destination: A,
    ) -> Result<Vec<ChannelEntry>, Self::NodeChainError> {
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
    ///
    /// Depending on the underlying chain implementation [`closed`](ChannelStatus) channels may or may not be returned.
    async fn channels_from<A: Into<Address> + Send>(
        &self,
        source: A,
    ) -> Result<Vec<ChannelEntry>, Self::NodeChainError> {
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
}

/// Trait implemented by nodes that support interaction with an [underlying chain](HoprChainApi) and the
/// [network](HoprNodeNetworkOperations).
///
/// This trait is automatically implemented for the nodes matching the criteria.
pub trait HoprNodeChainNetworkOperationsExt: HoprNodeChainOperations + HoprNodeNetworkOperations
where
    <Self as HoprNodeChainOperations>::NodeChainError: From<<Self as HoprNodeNetworkOperations>::NodeNetworkError>
        + From<<<Self as HoprNodeChainOperations>::ChainApi as HoprChainApi>::ChainError>,
{
    /// Allows translation of a peer's transport identity to the corresponding on-chain address.
    fn peerid_to_chain_key(&self, peer_id: &PeerId) -> Result<Option<Address>, Self::NodeChainError> {
        Ok(self
            .chain_api()
            .packet_key_to_chain_key(&self.peer_id_to_offchain_key(peer_id)?)?)
    }
    /// Allows translation of an on-chain address to the corresponding peer's transport identity.
    fn chain_key_to_peerid<A: Into<Address> + Send>(&self, address: A) -> Result<Option<PeerId>, Self::NodeChainError> {
        Ok(self
            .chain_api()
            .chain_key_to_packet_key(&address.into())
            .map(|pk| pk.map(|v| v.into()))?)
    }
}

// Automatically implement the trait for all nodes that implement both traits.
impl<T> HoprNodeChainNetworkOperationsExt for T
where
    T: ?Sized + HoprNodeChainOperations + HoprNodeNetworkOperations,
    <Self as HoprNodeChainOperations>::NodeChainError: From<<Self as HoprNodeNetworkOperations>::NodeNetworkError>
        + From<<<Self as HoprNodeChainOperations>::ChainApi as HoprChainApi>::ChainError>,
{
}
