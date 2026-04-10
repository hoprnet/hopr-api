use std::convert::identity;
use futures::{Stream, TryFutureExt, StreamExt};
use hopr_types::crypto::prelude::Hash;
use hopr_types::internal::prelude::*;
use hopr_types::primitive::prelude::*;
use libp2p_identity::PeerId;
use crate::chain::{HoprChainApi, ChainValues, ChainReadAccountOperations, ChainReadChannelOperations, ChainKeyOperations, ChannelSelector, ChainWriteAccountOperations, ChainReadSafeOperations, ChainInfo, AccountSelector};
use crate::tickets::{RedemptionResult, ChannelStats, TicketManagement};
use super::{CloseChannelResult, OpenChannelResult, SafeModuleConfig};


#[async_trait::async_trait]
pub trait HoprNodeChainOperations {
    type Error: std::error::Error + From<<Self::ChainApi as HoprChainApi>::ChainError> + Send + Sync + 'static;

    type ChainApi: HoprChainApi;

    fn me_onchain(&self) -> Address;

    fn chain_api(&self) -> &Self::ChainApi;

    fn safe_config(&self) -> &SafeModuleConfig;

    async fn open_channel(&self, destination: &Address, amount: HoprBalance) -> Result<OpenChannelResult, Self::Error>;
    async fn fund_channel(&self, channel_id: &ChannelId, amount: HoprBalance) -> Result<Hash, Self::Error>;
    async fn close_channel_by_id(&self, channel_id: &ChannelId) -> Result<CloseChannelResult, Self::Error>;

    async fn withdraw<C: Currency + Send>(&self, recipient: Address, amount: Balance<C>) -> Result<Hash, Self::Error> {
        Ok(self.chain_api()
            .withdraw(amount, &recipient)
            .and_then(identity)
            .await?)
    }

    async fn get_balance<C: Currency + Send>(&self) -> Result<Balance<C>, Self::Error> {
        Ok(self.chain_api()
            .balance(self.me_onchain())
            .await?)
    }

    async fn get_safe_balance<C: Currency + Send>(&self) -> Result<Balance<C>, Self::Error> {
        Ok(self.chain_api()
            .balance(self.safe_config().safe_address)
            .await?)
    }

    async fn safe_allowance(&self) -> Result<HoprBalance, Self::Error> {
        Ok(self
            .chain_api()
            .safe_allowance(self.safe_config().safe_address)
            .await?)
    }

    async fn chain_info(&self) -> Result<ChainInfo, Self::Error> {
        Ok(self.chain_api().chain_info().await?)
    }

    async fn get_ticket_price(&self) -> Result<HoprBalance, Self::Error> {
        Ok(self.chain_api().minimum_ticket_price().await?)
    }

    async fn get_minimum_incoming_ticket_win_probability(&self) -> Result<WinningProbability, Self::Error> {
        Ok(self.chain_api()
            .minimum_incoming_ticket_win_prob()
            .await?)
    }

    async fn get_channel_closure_notice_period(&self) -> Result<std::time::Duration, Self::Error> {
        Ok(self.chain_api()
            .channel_closure_notice_period()
            .await?)
    }

    async fn announced_peers(&self) -> Result<Vec<AnnouncedPeer>, Self::Error> {
        Ok(self
            .chain_api()
            .stream_accounts(AccountSelector {
                public_only: true,
                ..Default::default()
            })?
            .map(|entry| AnnouncedPeer {
                address: entry.chain_addr,
                multiaddresses: entry.get_multiaddrs().to_vec(),
                origin: AnnouncementOrigin::Chain,
            })
            .collect()
            .await)
    }

    fn peerid_to_chain_key(&self, peer_id: &PeerId) -> Result<Option<Address>, Self::Error> {
        let pubkey = hopr_transport::peer_id_to_public_key(peer_id)?;

        Ok(self.chain_api()
            .packet_key_to_chain_key(&pubkey)?)
    }

    fn chain_key_to_peerid(&self, address: &Address) -> Result<Option<PeerId>, Self::Error> {
        Ok(self.chain_api()
            .chain_key_to_packet_key(address)
            .map(|pk| pk.map(|v| v.into()))?)
    }

    fn channel_by_id(&self, channel_id: &ChannelId) -> Result<Option<ChannelEntry>, Self::Error> {
        Ok(self.chain_api().channel_by_id(channel_id)?)
    }

    fn channel(&self, src: &Address, dest: &Address) -> Result<Option<ChannelEntry>, Self::Error> {
        Ok(self.chain_api()
            .channel_by_parties(src, dest)?)
    }

    async fn channels_to(&self, dest: &Address) -> Result<Vec<ChannelEntry>, Self::Error> {
        Ok(self
            .chain_api()
            .stream_channels(
                ChannelSelector::default()
                    .with_destination(*dest)
                    .with_allowed_states(&[
                        ChannelStatusDiscriminants::Closed,
                        ChannelStatusDiscriminants::Open,
                        ChannelStatusDiscriminants::PendingToClose,
                    ]),
            )?
            .collect()
            .await)
    }

    async fn channels_from(&self, src: &Address) -> Result<Vec<ChannelEntry>, Self::Error> {
        Ok(self
            .chain_api()
            .stream_channels(ChannelSelector::default().with_source(*src).with_allowed_states(&[
                ChannelStatusDiscriminants::Closed,
                ChannelStatusDiscriminants::Open,
                ChannelStatusDiscriminants::PendingToClose,
            ]))?
            .collect()
            .await)
    }
}

/// Ticket events emitted from the packet processing pipeline.
#[derive(Debug, Clone, strum::EnumIs, strum::EnumTryAs)]
pub enum TicketEvent {
    /// A winning ticket was received.
    WinningTicket(Box<RedeemableTicket>),
    /// A ticket has been rejected.
    RejectedTicket(Box<Ticket>, Option<Address>),
}

#[async_trait::async_trait]
pub trait HoprNodeTicketOperations: HoprNodeChainOperations {
    fn subscribe_ticket_events(&self) -> impl Stream<Item = TicketEvent> + Send + 'static;

    fn ticket_statistics(&self) -> Result<ChannelStats, Self::Error>;

    #[deprecated(since = "1.7.0", note = "Can be removed once strategies depend on Hopr object only")]
    fn ticket_management(&self) -> Result<impl TicketManagement + Clone + Send + 'static, Self::Error>;

    async fn redeem_all_tickets<B: Into<HoprBalance> + Send>(
        &self,
        min_value: B,
    ) -> Result<Vec<RedemptionResult>, Self::Error>;

    async fn redeem_tickets_in_channel<B: Into<HoprBalance> + Send>(
        &self,
        channel_id: &ChannelId,
        min_value: B,
    ) -> Result<Vec<RedemptionResult>, Self::Error>;

    async fn redeem_tickets_with_counterparty<B: Into<HoprBalance> + Send>(
        &self,
        counterparty: &Address,
        min_value: B,
    ) -> Result<Vec<RedemptionResult>, Self::Error> {
        self.redeem_tickets_in_channel(&generate_channel_id(counterparty, &self.me_onchain()), min_value)
            .await
    }
}