use futures::future::BoxFuture;
pub use hopr_types::{
    internal::prelude::{RedeemableTicket, VerifiedTicket},
    primitive::prelude::HoprBalance,
};

use crate::chain::{ChainReceipt, WinningProbability};

/// On-chain operations to read values related to tickets.
///
/// These operations are used in critical packet processing pipelines, and therefore,
/// should not query the chain information directly, and they MUST NOT block.
#[auto_impl::auto_impl(&, Box, Arc)]
pub trait ChainReadTicketOperations {
    type Error: std::error::Error + Send + Sync + 'static;

    /// Retrieves the expected minimum winning probability and ticket price for **incoming** tickets.
    ///
    /// This operation MUST not block, as it is typically used within the critical packet processing pipeline.
    fn incoming_ticket_values(&self) -> Result<(WinningProbability, HoprBalance), Self::Error>;

    /// Retrieves the winning probability and ticket price for **outgoing** tickets,
    /// with respect to the optionally pre-configured values.
    ///
    /// The caller, not the implementation, should make sure that the configured values (if provided)
    /// are not less than the expected values for incoming tickets.
    ///
    /// This operation MUST not block, as it is typically used within the critical packet processing pipeline.
    ///
    /// The default implementation calls the `incoming_ticket_values` method and uses the incoming values
    /// unless the configured values are provided.
    fn outgoing_ticket_values(
        &self,
        configured_wp: Option<WinningProbability>,
        configured_price: Option<HoprBalance>,
    ) -> Result<(WinningProbability, HoprBalance), Self::Error> {
        match (configured_wp, configured_price) {
            (Some(wp), Some(price)) => Ok((wp, price)),
            (None, Some(price)) => Ok((self.incoming_ticket_values()?.0, price)),
            (Some(wp), None) => Ok((wp, self.incoming_ticket_values()?.1)),
            _ => self.incoming_ticket_values(),
        }
    }
}

/// Errors that can occur during ticket redemption.
#[derive(Debug, thiserror::Error)]
pub enum TicketRedeemError<E> {
    /// Non-retryable error, the ticket should be discarded
    #[error("redemption of ticket {0} was rejected due to: {1}")]
    Rejected(VerifiedTicket, String),
    /// Retryable error, the ticket redemption should be retried.
    #[error("processing error during redemption of ticket {0}: {1}")]
    ProcessingError(VerifiedTicket, E),
}

/// On-chain write operations with tickets.
#[async_trait::async_trait]
#[auto_impl::auto_impl(&, Box, Arc)]
pub trait ChainWriteTicketOperations {
    type Error: std::error::Error + Send + Sync + 'static;
    /// Redeems a single ticket on-chain.
    ///
    /// The input `ticket` is always returned as [`VerifiedTicket`], either on success or failure.
    async fn redeem_ticket<'a>(
        &'a self,
        ticket: RedeemableTicket,
    ) -> Result<
        BoxFuture<'a, Result<(VerifiedTicket, ChainReceipt), TicketRedeemError<Self::Error>>>,
        TicketRedeemError<Self::Error>,
    >;
}
