use std::convert::identity;

pub use futures::future::AbortHandle;
use futures::{StreamExt, TryFutureExt};
use hopr_types::{chain::chain_events::ChainEvent, crypto::prelude::Hash, internal::prelude::*, primitive::prelude::*};
use libp2p_identity::PeerId;

use crate::{
    chain::{
        AccountSelector, ChainInfo, ChainKeyOperations, ChainReadAccountOperations, ChainReadChannelOperations,
        ChainReadSafeOperations, ChainValues, ChainWriteAccountOperations, ChainWriteChannelOperations,
        ChannelSelector, HoprChainApi,
    },
    node::{CompoundResult, EitherErr, network::HoprNodeNetworkOperations},
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

/// Represents an output of a write operation to the chain performed by the node.
///
/// This consists always of a transaction hash and an optional output `T`.
///
/// Operations that produce no useful output use `()` as `T`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ChainOutput<T> {
    tx_hash: Hash,
    output: Option<T>,
}

impl<T> ChainOutput<T> {
    /// Creates a new ChainOutput with the given transaction hash and output.
    pub fn new(tx_hash: Hash, output: T) -> Self {
        Self {
            tx_hash,
            output: output.into(),
        }
    }

    /// Returns the transaction hash of the chain operation.
    pub fn tx_hash(&self) -> &Hash {
        &self.tx_hash
    }

    /// Returns the optional output of the chain operation.
    pub fn output(&self) -> Option<&T> {
        self.output.as_ref()
    }
}

impl ChainOutput<()> {
    /// Creates a new ChainOutput with the given transaction hash and no output.
    pub fn new_empty(tx_hash: Hash) -> Self {
        Self { tx_hash, output: None }
    }
}

impl From<Hash> for ChainOutput<()> {
    fn from(tx_hash: Hash) -> Self {
        Self::new_empty(tx_hash)
    }
}

/// Future that resolves when a [`ChainEvent`] is resolved, times out, or is aborted
/// via the associated abort handle.
///
/// Error associated with the chain operation is `ChainErr`, all other errors are `WaitErr`.
pub type ChainEventResolver<ChainErr, WaitErr> = (
    std::pin::Pin<Box<dyn Future<Output = CompoundResult<ChainEvent, ChainErr, WaitErr>> + Send + 'static>>,
    AbortHandle,
);

/// Alias for [`HoprNodeChainOperations::wait_for_on_chain_event`] result.
///
/// Error associated with the chain operation is `ChainErr`, all other errors are `WaitErr`.
pub type EventWaitResult<ChainErr, WaitErr> = Result<ChainEventResolver<ChainErr, WaitErr>, ChainErr>;

/// Implemented by nodes that support interaction with an [underlying chain](HoprChainApi).
#[async_trait::async_trait]
#[auto_impl::auto_impl(&, Box, Arc)]
pub trait HoprNodeChainOperations {
    /// General error thrown by the implementors.
    type NodeChainError: std::error::Error + Send + Sync + 'static;

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

    /// Spawns an asynchronous waiter that hooks up to the [`ChainEvent`] [bus](crate::chain::ChainEvents::subscribe)
    /// and either matching the given `predicate` or timing out after `timeout`.
    ///
    /// The implementor decides on the async runtime used to spawn the operation that resolves
    /// with the returned future.
    ///
    /// The `context` contains a human-readable description of the operation being awaited.
    /// The implementors are free to decide whether to use the `context` or not.
    ///
    /// The implementation should take into account that the given `timeout` is
    /// usually **already pre-multiplied** with [`Self::CHAIN_OPERATION_TIMEOUT_MULTIPLIER`].
    fn wait_for_on_chain_event<F>(
        &self,
        predicate: F,
        context: String,
        timeout: std::time::Duration,
    ) -> EventWaitResult<<Self::ChainApi as HoprChainApi>::ChainError, Self::NodeChainError>
    where
        F: Fn(&ChainEvent) -> bool + Send + Sync + 'static;

    /// Opens a channel from the node to the given `destination` with the given `amount` as the initial stake.
    ///
    /// On success, returns the ID of the new channel as [output](ChainOutput).
    /// Returns an error if the channel exists and is not closed, or the operation times out.
    async fn open_channel<A: Into<Address> + Send>(
        &self,
        destination: A,
        amount: HoprBalance,
    ) -> CompoundResult<ChainOutput<ChannelId>, <Self::ChainApi as HoprChainApi>::ChainError, Self::NodeChainError>
    {
        let destination = destination.into();
        let channel_id = generate_channel_id(&self.identity().node_address, &destination);

        // Subscribe to chain events BEFORE sending the transaction to avoid
        // a race where the event is broadcast before the subscriber activates.
        let (event_awaiter, event_abort) = self
            .wait_for_on_chain_event(
                move |event| matches!(event, ChainEvent::ChannelOpened(c) if c.get_id() == &channel_id),
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
    ) -> CompoundResult<ChainOutput<()>, <Self::ChainApi as HoprChainApi>::ChainError, Self::NodeChainError> {
        let channel_id = *channel_id;

        // Subscribe to chain events BEFORE sending the transaction to avoid
        // a race where the event is broadcast before the subscriber activates.
        let (event_awaiter, event_abort) = self.wait_for_on_chain_event(
            move |event| matches!(event, ChainEvent::ChannelBalanceIncreased(c, a) if c.get_id() == &channel_id && a == &amount),
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
    ///
    /// On success, returns the new [status](ChannelStatus) of the channel as [output](ChainOutput).
    async fn close_channel_by_id(
        &self,
        channel_id: &ChannelId,
    ) -> CompoundResult<ChainOutput<ChannelStatus>, <Self::ChainApi as HoprChainApi>::ChainError, Self::NodeChainError>
    {
        let channel_id = *channel_id;

        // Subscribe to chain events BEFORE sending the transaction to avoid
        // a race where the event is broadcast before the subscriber activates.
        let (event_awaiter, event_abort) = self
            .wait_for_on_chain_event(
                move |event| {
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
            ChainEvent::ChannelClosureInitiated(c) | ChainEvent::ChannelClosed(c) => c.status,
            // Guaranteed to be unreachable, due to the above predicate in `wait_for_on_chain_event`
            _ => unreachable!(),
        };

        Ok(ChainOutput::new(tx_hash, status))
    }

    /// Withdraws the given `amount` of [`Currency`] from the node's Safe or node account to the `recipient`.
    ///
    /// Whether withdrawal is done from the node's Safe or node account depends on the underlying [`HoprChainApi`]
    /// implementation.
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

    /// Returns the balance of [`Currency`] in the node's account.
    async fn get_balance<C: Currency + Send>(
        &self,
    ) -> Result<Balance<C>, <Self::ChainApi as HoprChainApi>::ChainError> {
        Ok(self.chain_api().balance(self.identity().node_address).await?)
    }

    /// Returns the balance of [`Currency`] the node's Safe.
    async fn get_safe_balance<C: Currency + Send>(
        &self,
    ) -> Result<Balance<C>, <Self::ChainApi as HoprChainApi>::ChainError> {
        Ok(self.chain_api().balance(self.identity().safe_address).await?)
    }

    /// Returns the allowance of the node's Safe to spend funds in channels.
    async fn safe_allowance(&self) -> Result<HoprBalance, <Self::ChainApi as HoprChainApi>::ChainError> {
        Ok(self.chain_api().safe_allowance(self.identity().safe_address).await?)
    }

    /// Shorthand to retrieve information about the connected blockchain.
    async fn chain_info(&self) -> Result<ChainInfo, <Self::ChainApi as HoprChainApi>::ChainError> {
        Ok(self.chain_api().chain_info().await?)
    }

    /// Shorthand to retrieve the minimum price of an incoming ticket given by the connected blockchain.
    async fn get_ticket_price(&self) -> Result<HoprBalance, <Self::ChainApi as HoprChainApi>::ChainError> {
        Ok(self.chain_api().minimum_ticket_price().await?)
    }

    /// Shorthand to retrieve the minimum win probability of an incoming ticket given by the connected blockchain.
    async fn get_minimum_incoming_ticket_win_probability(
        &self,
    ) -> Result<WinningProbability, <Self::ChainApi as HoprChainApi>::ChainError> {
        Ok(self.chain_api().minimum_incoming_ticket_win_prob().await?)
    }

    /// Shorthand to return the channel closure grace period.
    ///
    /// This is how much time is required for a channel to transition from
    /// `ChannelStatus::PendingToClose` to `ChannelStatus::Closed`.
    async fn get_channel_closure_notice_period(
        &self,
    ) -> Result<std::time::Duration, <Self::ChainApi as HoprChainApi>::ChainError> {
        Ok(self.chain_api().channel_closure_notice_period().await?)
    }

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

    /// Returns a channel with the given `channel_id`.
    ///
    /// Depending on the underlying chain implementation [`closed`](ChannelStatus) channels may or may not be returned.
    fn channel_by_id(
        &self,
        channel_id: &ChannelId,
    ) -> Result<Option<ChannelEntry>, <Self::ChainApi as HoprChainApi>::ChainError> {
        self.chain_api().channel_by_id(channel_id)
    }

    /// Returns a channel with the given `source` and `destination`.
    ///
    /// Depending on the underlying chain implementation [`closed`](ChannelStatus) channels may or may not be returned.
    fn channel<A: Into<Address> + Send, B: Into<Address> + Send>(
        &self,
        source: A,
        destination: B,
    ) -> Result<Option<ChannelEntry>, <Self::ChainApi as HoprChainApi>::ChainError> {
        self.chain_api().channel_by_parties(&source.into(), &destination.into())
    }

    /// Returns all channels to the given `destination`.
    ///
    /// Depending on the underlying chain implementation [`closed`](ChannelStatus) channels may or may not be returned.
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
    ///
    /// Depending on the underlying chain implementation [`closed`](ChannelStatus) channels may or may not be returned.
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
}

/// Trait implemented by nodes that support interaction with an [underlying chain](HoprChainApi) and the
/// [network](HoprNodeNetworkOperations).
///
/// This trait is automatically implemented for the nodes matching the criteria.
pub trait HoprNodeChainNetworkOperationsExt: HoprNodeChainOperations + HoprNodeNetworkOperations {
    /// Allows translation of a peer's transport identity to the corresponding on-chain address.
    fn peerid_to_chain_key(
        &self,
        peer_id: &PeerId,
    ) -> CompoundResult<
        Option<Address>,
        <<Self as HoprNodeChainOperations>::ChainApi as HoprChainApi>::ChainError,
        <Self as HoprNodeNetworkOperations>::NodeNetworkError,
    > {
        self.chain_api()
            .packet_key_to_chain_key(&self.peer_id_to_offchain_key(peer_id).map_err(EitherErr::right)?)
            .map_err(EitherErr::left)
    }
    /// Allows translation of an on-chain address to the corresponding peer's transport identity.
    fn chain_key_to_peerid<A: Into<Address> + Send>(
        &self,
        address: A,
    ) -> CompoundResult<
        Option<PeerId>,
        <<Self as HoprNodeChainOperations>::ChainApi as HoprChainApi>::ChainError,
        <Self as HoprNodeNetworkOperations>::NodeNetworkError,
    > {
        self.chain_api()
            .chain_key_to_packet_key(&address.into())
            .map(|pk| pk.map(|v| v.into()))
            .map_err(EitherErr::left)
    }
}

// Automatically implement the trait for all nodes that implement both traits.
impl<T> HoprNodeChainNetworkOperationsExt for T where T: ?Sized + HoprNodeChainOperations + HoprNodeNetworkOperations {}
