use futures::future::BoxFuture;
pub use hopr_types::internal::prelude::{RedeemableTicket, VerifiedTicket};

use crate::chain::ChainReceipt;

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
