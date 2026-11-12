use futures::{future::BoxFuture, stream::BoxStream};
use hopr_types::{
    chain::prelude::Erc777Payload,
    crypto::prelude::{ChainKeypair, OffchainKeypair, OffchainPublicKey},
    primitive::prelude::{Address, HoprBalance},
};
pub use hopr_types::{
    internal::prelude::AccountEntry,
    primitive::{
        multiaddr::Multiaddr,
        prelude::{Balance, Currency},
    },
};

use crate::chain::ChainReceipt;

/// Error that can occur when making a node announcement.
///
/// See [`ChainWriteAccountOperations::announce`]
#[derive(Debug, strum::EnumIs, strum::EnumTryAs, thiserror::Error)]
pub enum AnnouncementError<E> {
    /// Special error when an account is already announced.
    #[error("already announced")]
    AlreadyAnnounced,
    /// Error that can occur when processing an announcement.
    #[error("account announcement error: {0}")]
    ProcessingError(E),
}

impl<E> AnnouncementError<E> {
    /// Constructs a [`AnnouncementError::ProcessingError`].
    pub fn processing<F: Into<E>>(error: F) -> Self {
        Self::ProcessingError(error.into())
    }
}

/// Error that can occur when registering a node with a Safe.
#[derive(Debug, strum::EnumIs, strum::EnumTryAs, thiserror::Error)]
pub enum SafeRegistrationError<E> {
    /// Special error when a Safe is already registered.
    #[error("node is already registered with safe {0}")]
    AlreadyRegistered(Address),
    /// Error that can occur when processing a Safe registration.
    #[error("safe registration error: {0}")]
    ProcessingError(E),
}

impl<E> SafeRegistrationError<E> {
    /// Constructs a [`SafeRegistrationError::ProcessingError`].
    pub fn processing<F: Into<E>>(error: F) -> Self {
        Self::ProcessingError(error.into())
    }
}

/// What a caller may assume about value it tried to move, given a failed write.
///
/// See [`ChainWriteAccountOperations::submission_failure`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, strum::EnumIs)]
pub enum SubmissionFailure {
    /// Nothing moved: the transaction was refused before it was broadcast, so no copy of it can
    /// reach the chain and the value is still where it was.
    NotSubmitted,
    /// The chain rejected the transaction, or the Safe's inner call failed. The transaction was
    /// included and moved nothing.
    Reverted,
    /// The transaction may have been broadcast (a timeout, a transport failure, an error the chain
    /// API cannot classify), so the value may or may not have moved. A caller must not treat it
    /// as unsent.
    Unknown,
}

/// On-chain write operations regarding on-chain node accounts.
#[async_trait::async_trait]
#[auto_impl::auto_impl(&, Box, Arc)]
pub trait ChainWriteAccountOperations {
    type Error: std::error::Error + Send + Sync + 'static;

    /// What the caller may assume about value it tried to move when one of the writes of this trait
    /// (or the pending future it returned) failed with `error`.
    ///
    /// A strategy that must know whether a failed transfer left its funds untouched cannot learn
    /// that from [`Self::Error`], which is opaque here; without this it has to downcast to the
    /// chain API's concrete error type. An implementation that can tell classifies the error; the
    /// default is the only answer that is safe without that knowledge, [`SubmissionFailure::Unknown`].
    fn submission_failure(&self, _error: &Self::Error) -> SubmissionFailure {
        SubmissionFailure::Unknown
    }

    /// Announces transport key and list of multi addresses.
    async fn announce(
        &self,
        multiaddrs: &[Multiaddr],
        key: &OffchainKeypair,
    ) -> Result<BoxFuture<'life0, Result<ChainReceipt, Self::Error>>, AnnouncementError<Self::Error>>;

    /// Withdraws native or token currency from the Safe or node account (depends on the used `PayloadGenerator`).
    async fn withdraw<C: Currency + Send>(
        &self,
        balance: Balance<C>,
        recipient: &Address,
    ) -> Result<BoxFuture<'life0, Result<ChainReceipt, Self::Error>>, Self::Error>;

    /// Sends token currency from the Safe or node account (depends on the used `PayloadGenerator`)
    /// via ERC777 `send`, with `payload` as the data for the recipient's `tokensReceived` hook.
    async fn send_erc777(
        &self,
        balance: HoprBalance,
        recipient: &Address,
        payload: &Erc777Payload,
    ) -> Result<BoxFuture<'life0, Result<ChainReceipt, Self::Error>>, Self::Error>;

    /// Withdraws native or token currency from the `signer` to `recipient` in a single transaction.
    ///
    /// **Deprecated:** Use `withdraw` with `BasicPayloadGenerator` instead. Will be removed in 5.0.0
    // TODO: remove this in the next major release (5.0.0)
    async fn withdraw_from_signer<C: Currency + Send>(
        &self,
        signer: &ChainKeypair,
        balance: Balance<C>,
        recipient: &Address,
    ) -> Result<BoxFuture<'life0, Result<ChainReceipt, Self::Error>>, Self::Error>;

    /// Registers Safe address with the current node.
    async fn register_safe(
        &self,
        safe_address: &Address,
    ) -> Result<BoxFuture<'life0, Result<ChainReceipt, Self::Error>>, SafeRegistrationError<Self::Error>>;
}

/// Selector for on-chain node accounts.
///
/// See [`ChainReadAccountOperations::stream_accounts`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct AccountSelector {
    /// Selects accounts that announced with publicly routable multi-addresses.
    pub public_only: bool,
    /// Selects accounts bound with the given chain key.
    pub chain_key: Option<Address>,
    /// Selects accounts bound with the given off-chain key.
    pub offchain_key: Option<OffchainPublicKey>,
}

impl AccountSelector {
    /// Selects only accounts that announced with publicly routable multi-addresses.
    #[must_use]
    pub fn with_public_only(mut self, public_only: bool) -> Self {
        self.public_only = public_only;
        self
    }

    /// Selects accounts bound with the given chain key.
    #[must_use]
    pub fn with_chain_key(mut self, chain_key: Address) -> Self {
        self.chain_key = Some(chain_key);
        self
    }

    /// Selects accounts bound with the given off-chain key.
    #[must_use]
    pub fn with_offchain_key(mut self, offchain_key: OffchainPublicKey) -> Self {
        self.offchain_key = Some(offchain_key);
        self
    }

    /// Checks if the given [`account`](AccountEntry) satisfies the selector.
    pub fn satisfies(&self, account: &AccountEntry) -> bool {
        if self.public_only && !account.has_announced_with_routing_info() {
            return false;
        }

        if let Some(chain_key) = &self.chain_key
            && &account.chain_addr != chain_key
        {
            return false;
        }

        if let Some(packet_key) = &self.offchain_key
            && &account.public_key != packet_key
        {
            return false;
        }

        true
    }
}

/// Chain operations that read on-chain node accounts.
#[async_trait::async_trait]
#[auto_impl::auto_impl(&, Box, Arc)]
pub trait ChainReadAccountOperations {
    type Error: std::error::Error + Send + Sync + 'static;

    /// Returns on-chain node accounts with the given [`AccountSelector`].
    fn stream_accounts<'a>(&'a self, selector: AccountSelector) -> Result<BoxStream<'a, AccountEntry>, Self::Error>;

    /// Counts the accounts with the given [`AccountSelector`].
    ///
    /// This is potentially done more effectively than counting more elements of
    /// the stream returned by [`ChainReadAccountOperations::stream_accounts`].
    async fn count_accounts(&self, selector: AccountSelector) -> Result<usize, Self::Error>;

    /// Waits for the account with the key-binding on the `offchain_key` to appear on-chain or the `timeout` expires.
    async fn await_key_binding(
        &self,
        offchain_key: &OffchainPublicKey,
        timeout: std::time::Duration,
    ) -> Result<AccountEntry, Self::Error>;
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    #[derive(Debug, thiserror::Error)]
    #[error("failed")]
    struct Failed;

    /// Implements only the required methods, so everything it answers about a failure is the
    /// trait's default.
    struct Plain;

    #[async_trait::async_trait]
    impl ChainWriteAccountOperations for Plain {
        type Error = Failed;

        async fn announce(
            &self,
            _multiaddrs: &[Multiaddr],
            _key: &OffchainKeypair,
        ) -> Result<BoxFuture<'life0, Result<ChainReceipt, Self::Error>>, AnnouncementError<Self::Error>> {
            Err(AnnouncementError::AlreadyAnnounced)
        }

        async fn withdraw<C: Currency + Send>(
            &self,
            _balance: Balance<C>,
            _recipient: &Address,
        ) -> Result<BoxFuture<'life0, Result<ChainReceipt, Self::Error>>, Self::Error> {
            Err(Failed)
        }

        async fn send_erc777(
            &self,
            _balance: HoprBalance,
            _recipient: &Address,
            _payload: &Erc777Payload,
        ) -> Result<BoxFuture<'life0, Result<ChainReceipt, Self::Error>>, Self::Error> {
            Err(Failed)
        }

        async fn withdraw_from_signer<C: Currency + Send>(
            &self,
            _signer: &ChainKeypair,
            _balance: Balance<C>,
            _recipient: &Address,
        ) -> Result<BoxFuture<'life0, Result<ChainReceipt, Self::Error>>, Self::Error> {
            Err(Failed)
        }

        async fn register_safe(
            &self,
            _safe_address: &Address,
        ) -> Result<BoxFuture<'life0, Result<ChainReceipt, Self::Error>>, SafeRegistrationError<Self::Error>> {
            Err(SafeRegistrationError::ProcessingError(Failed))
        }
    }

    /// An implementation that cannot classify its errors must not claim that nothing moved, and
    /// the provided method is forwarded by the `auto_impl` pointer types.
    #[test]
    fn a_failure_is_unknown_by_default_and_through_a_pointer() {
        assert_eq!(Plain.submission_failure(&Failed), SubmissionFailure::Unknown);
        assert_eq!(Arc::new(Plain).submission_failure(&Failed), SubmissionFailure::Unknown);
        fn classify(ops: impl ChainWriteAccountOperations<Error = Failed>) -> SubmissionFailure {
            ops.submission_failure(&Failed)
        }
        assert_eq!(classify(&Plain), SubmissionFailure::Unknown);
        assert_eq!(Box::new(Plain).submission_failure(&Failed), SubmissionFailure::Unknown);
    }
}
