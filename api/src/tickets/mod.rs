use futures::{Stream, StreamExt};
pub use hopr_types::{
    internal::prelude::{ChannelId, VerifiedTicket},
    primitive::balance::HoprBalance,
};

use crate::chain::{ChainReadChannelOperations, ChainWriteTicketOperations, ChannelSelector};

/// Contains ticket statistics for an incoming channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ChannelStats {
    /// Total number of winning tickets received in this channel.
    pub winning_tickets: u128,
    /// Total value of unredeemed tickets in this channel.
    pub unredeemed_value: HoprBalance,
    /// Total value of on-chain rejected tickets in this channel.
    pub rejected_value: HoprBalance,
    /// Total value of on-chain neglected tickets in this channel.
    pub neglected_value: HoprBalance,
}

/// Indicates a non-error result of [ticket redemption](TicketManagement::redeem_stream).
#[derive(Clone, Debug, PartialEq, Eq, strum::Display)]
pub enum RedemptionResult {
    /// Ticket has been redeemed successfully.
    #[strum(to_string = "redeemed {0}")]
    Redeemed(VerifiedTicket),
    /// Ticket has been neglected because its value was lower than the threshold.
    #[strum(to_string = "neglected {0} due to low value")]
    ValueTooLow(VerifiedTicket),
    /// Ticket has been rejected on-chain for the given reason.
    #[strum(to_string = "rejected {0} on-chain: {1}")]
    RejectedOnChain(VerifiedTicket, String),
}

impl AsRef<VerifiedTicket> for RedemptionResult {
    fn as_ref(&self) -> &VerifiedTicket {
        match self {
            RedemptionResult::Redeemed(ticket) => ticket,
            RedemptionResult::ValueTooLow(ticket) => ticket,
            RedemptionResult::RejectedOnChain(ticket, _) => ticket,
        }
    }
}

/// API for managing winning (redeemable) tickets in incoming channels.
///
/// The redeemable tickets are typically organized in a queue ordered by their
/// [`TicketId`](hopr_types::internal::prelude::TicketId). There are 3 possible ways how a redeemable ticket can be
/// extracted (removed) from the queue:
/// 1. Successful on-chain redemption (happens due to a successful on-chain
///    [redemption](crate::chain::ChainWriteTicketOperations::redeem_ticket) operation).
/// 2. Unsuccessful on-chain redemption (rejection happens due to a failed on-chain
///    [redemption](crate::chain::ChainWriteTicketOperations::redeem_ticket) operation).
/// 3. Neglection without trying to redeem it on-chain (can happen for various reasons, e.g.: a channel being closed
///    prior to a ticket being redeemed, low-value ticket ... etc.).
/// Extracting tickets from the queue should not be possible via any other means than the 3 above, and this is what is
/// reflected by this trait.
///
/// The state of the individual channels can be observed via [`ChannelStats`].
#[auto_impl::auto_impl(&, Box, Arc)]
pub trait TicketManagement {
    type Error: std::error::Error + Send + Sync + 'static;
    /// Creates a stream that tries to redeem individual winning tickets from the given channel in the correct order.
    ///
    /// All errors that are due to tickets being invalid (found to be unredeemable) are handled by returning a
    /// [`RedemptionResult::RejectedOnChain`], rejected from the queue, and the stream continues with the next
    /// redeemable ticket in that channel.
    ///
    /// If `min_amount` is specified and tickets are found to be below this value, the ticket is handled by returning
    /// [`RedemptionResult::ValueTooLow`], gets neglected from the queue, and the stream continues with the next
    /// redeemable ticket.
    ///
    /// The stream terminates if there's a processing error (passing the error via the stream), the ticket that
    /// triggered the error remains in the queue and can be attempted to be redeemed once `redeem_stream` is called
    /// again.
    fn redeem_stream<C: ChainWriteTicketOperations + Send + Sync + 'static>(
        &self,
        client: C,
        channel_id: ChannelId,
        min_amount: Option<HoprBalance>,
    ) -> Result<impl Stream<Item = Result<RedemptionResult, Self::Error>> + Send, Self::Error>;
    /// Neglects tickets in the given channel up to the `max_ticket_index` (inclusive, or all tickets if `None`).
    ///
    /// Returns the vector of neglected tickets.
    ///
    /// If this function is called while a redemption stream is active on the same channel and the ticket index range
    /// overlaps with the range of redeemed tickets, the neglection will take precedence over the redemption stream.
    /// This means the stream will terminate earlier because the tickets that were not redeemed will be neglected.
    fn neglect_tickets(
        &self,
        channel_id: &ChannelId,
        max_ticket_index: Option<u64>,
    ) -> Result<Vec<VerifiedTicket>, Self::Error>;

    /// Returns the [`ChannelStats`] for the given channel, or cumulative stats for all channels if `None`.
    ///
    /// Usually the stats could be non-persistent, but it is a choice of the implementation.
    fn ticket_stats(&self, channel_id: Option<&ChannelId>) -> Result<ChannelStats, Self::Error>;
}

/// Asynchronous extension trait for [`TicketManagement`] that adds convenience methods for ticket management.
///
/// Automatically implemented for each type that implements `TicketManagement`.
#[async_trait::async_trait]
pub trait TicketManagementExt: TicketManagement {
    /// Performs redemptions in multiple channels.
    ///
    /// This method queries the chain `client` for all incoming channels that are open or have a ticket redemption
    /// window open (at least `min_grace_period` in the future) and optionally also matching the given `selector`.
    /// It then creates a [redemption stream](TicketManagement::redeem_stream) for each channel that tries to redeem
    /// individual winning tickets in the correct order.
    ///
    /// Tickets that are not worth at least `min_amount` are neglected.
    ///
    /// Incoming channels for which the redeem stream could not be created are skipped.
    ///
    /// The returned stream can be concurrently processed and guarantees that redeemable tickets
    /// are processed in the correct order in their respective channels.
    async fn redeem_in_channels<C>(
        &self,
        client: C,
        selector: Option<ChannelSelector>,
        min_amount: Option<HoprBalance>,
        min_grace_period: Option<std::time::Duration>,
    ) -> Result<
        impl Stream<Item = Result<RedemptionResult, Self::Error>> + Send,
        <C as ChainReadChannelOperations>::Error,
    >
    where
        C: ChainReadChannelOperations + ChainWriteTicketOperations + Clone + Send + Sync + 'static,
    {
        let mut stream_group = futures_concurrency::stream::StreamGroup::new();
        client
            .stream_channels(
                selector
                    .unwrap_or_default()
                    .with_destination(*client.me())
                    .with_redeemable_channels(min_grace_period),
            )
            .await?
            .filter_map(|channel| {
                futures::future::ready(
                    self.redeem_stream(client.clone(), *channel.get_id(), min_amount)
                        .inspect_err(
                            |error| tracing::error!(%error, %channel, "failed to open redeem stream for channel"),
                        )
                        .ok(),
                )
            })
            .for_each(|stream| {
                stream_group.insert(stream);
                futures::future::ready(())
            })
            .await;

        Ok(stream_group)
    }
}

impl<T: TicketManagement + ?Sized> TicketManagementExt for T {}

#[cfg(test)]
mod tests {
    use futures::{StreamExt, stream};
    use hopr_types::{crypto::prelude::Keypair, internal::prelude::*, primitive::prelude::Address};
    use mockall::{mock, predicate::*};

    use super::*;
    use crate::{
        ChainKeypair,
        chain::{ChainReadChannelOperations, ChainWriteTicketOperations},
    };

    mock! {
        pub TicketManager {}
        #[allow(refining_impl_trait)]
        impl TicketManagement for TicketManager {
            type Error = std::io::Error;
            fn redeem_stream<C: ChainWriteTicketOperations + Send + Sync + 'static>(
                &self,
                client: C,
                channel_id: ChannelId,
                min_amount: Option<HoprBalance>,
            ) -> Result<stream::BoxStream<'static, Result<RedemptionResult, std::io::Error>>, std::io::Error>;

            fn neglect_tickets(
                &self,
                channel_id: &ChannelId,
                max_ticket_index: Option<u64>,
            ) -> Result<Vec<VerifiedTicket>, std::io::Error>;

            fn ticket_stats<'a>(&self, channel_id: Option<&'a ChannelId>) -> Result<ChannelStats, std::io::Error>;
        }
    }

    mock! {
        pub ChainClient {}
        #[async_trait::async_trait]
        impl ChainReadChannelOperations for ChainClient {
            type Error = std::io::Error;
            fn me(&self) -> &Address;
            async fn channel_by_id(&self, channel_id: &ChannelId) -> Result<Option<ChannelEntry>, std::io::Error>;
            async fn stream_channels<'a>(
                &'a self,
                selector: ChannelSelector,
            ) -> Result<stream::BoxStream<'a, ChannelEntry>, std::io::Error>;
        }
        #[async_trait::async_trait]
        impl ChainWriteTicketOperations for ChainClient {
            type Error = std::io::Error;
            async fn redeem_ticket<'a>(
                &'a self,
                ticket: hopr_types::internal::prelude::RedeemableTicket,
            ) -> Result<
                futures::future::BoxFuture<'a, Result<(VerifiedTicket, hopr_types::crypto::prelude::Hash), crate::chain::TicketRedeemError<std::io::Error>>>,
                crate::chain::TicketRedeemError<std::io::Error>,
            >;
        }
        impl Clone for ChainClient {
            fn clone(&self) -> Self;
        }
    }

    #[tokio::test]
    async fn test_redeem_in_channels_empty() {
        let mock_tm = MockTicketManager::new();
        let mut mock_client = MockChainClient::new();

        let my_address = Address::default();
        mock_client.expect_me().return_const(my_address);

        mock_client
            .expect_stream_channels()
            .returning(|_| Ok(stream::empty().boxed()));

        let result = mock_tm.redeem_in_channels(mock_client, None, None, None).await.unwrap();

        let results: Vec<_> = result.collect().await;
        assert!(results.is_empty());
    }

    fn generate_tickets_in_channel(issuer: &ChainKeypair, channel: &ChannelEntry, count: usize) -> Vec<VerifiedTicket> {
        assert_eq!(issuer.public().to_address(), channel.source);
        (0..count)
            .map(|index| {
                TicketBuilder::default()
                    .counterparty(channel.destination)
                    .amount(1)
                    .win_prob(WinningProbability::ALWAYS)
                    .index(index as u64)
                    .channel_epoch(channel.channel_epoch)
                    .eth_challenge(Default::default())
                    .build_signed(&issuer, &Default::default())
                    .unwrap()
            })
            .collect()
    }

    #[tokio::test]
    async fn test_redeem_in_channels_multiple_channels() {
        let mut mock_tm = MockTicketManager::new();
        let mut mock_client = MockChainClient::new();

        let my_address = Address::from([0u8; 20]);
        mock_client.expect_me().return_const(my_address);

        let source_1 = ChainKeypair::random();
        let source_2 = ChainKeypair::random();

        let channel_1 = ChannelBuilder::default()
            .source(&source_1)
            .destination(my_address)
            .balance(HoprBalance::default())
            .status(ChannelStatus::Open)
            .build()
            .unwrap();
        let channel_2 = ChannelBuilder::default()
            .source(&source_2)
            .destination(my_address)
            .balance(HoprBalance::default())
            .status(ChannelStatus::Open)
            .build()
            .unwrap();

        let channel_1_clone = channel_1.clone();
        let channel_2_clone = channel_2.clone();
        let channel_1_id = *channel_1.get_id();
        let channel_2_id = *channel_2.get_id();

        mock_client
            .expect_stream_channels()
            .with(function(move |selector: &ChannelSelector| {
                selector.destination == Some(my_address)
            }))
            .returning(move |_| Ok(stream::iter(vec![channel_1_clone.clone(), channel_2_clone.clone()]).boxed()));

        mock_client.expect_clone().returning(MockChainClient::default);

        let min_amount = Some(HoprBalance::from(100));

        let tickets_1 = generate_tickets_in_channel(&source_1, &channel_1, 10);
        let tickets_2 = generate_tickets_in_channel(&source_2, &channel_2, 10);

        let tickets_1_clone = tickets_1.clone();
        mock_tm
            .expect_redeem_stream::<MockChainClient>()
            .once()
            .with(always(), eq(channel_1_id), eq(min_amount))
            .return_once(|_, _, _| {
                Ok(stream::iter(tickets_1_clone)
                    .map(|t| Ok(RedemptionResult::Redeemed(t)))
                    .boxed())
            });

        let tickets_2_clone = tickets_2.clone();
        mock_tm
            .expect_redeem_stream::<MockChainClient>()
            .once()
            .with(always(), eq(channel_2_id), eq(min_amount))
            .return_once(|_, _, _| {
                Ok(stream::iter(tickets_2_clone)
                    .map(|t| Ok(RedemptionResult::Redeemed(t)))
                    .boxed())
            });

        let result = mock_tm
            .redeem_in_channels(mock_client, None, min_amount, None)
            .await
            .unwrap();

        let results: Vec<_> = result.collect().await;
        assert_eq!(results.len(), tickets_1.len() + tickets_2.len());
    }
}
