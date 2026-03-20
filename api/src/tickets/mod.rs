use futures::Stream;
pub use hopr_types::{
    internal::prelude::{ChannelId, VerifiedTicket},
    primitive::balance::HoprBalance,
};

use crate::chain::ChainWriteTicketOperations;

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
