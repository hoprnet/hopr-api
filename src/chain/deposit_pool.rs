use futures::future::{BoxFuture, join_all};
pub use hopr_types::crypto::primitives::{PixDepositAddress, PixDepositSecret};
use hopr_types::{
    crypto::prelude::Keypair,
    network::PixAddressId,
    primitive::prelude::{Address, HoprBalance},
};

/// A future that resolves once `min_amount` has been deposited to the curve-specific
/// public key `dst`, or an error occurs.
pub type DepositNotification<'a, P, E> = BoxFuture<'a, Result<(PixAddressId, P, HoprBalance), E>>;

/// Contains abstraction over the deposit pool from PIX.
///
/// The funds within this pool are represented by the given keypair `K`.
///
/// Its public key is convertible to [`PixDepositAddress`]. [`PixAddressId`] is
/// curve-agnostic and identifies the protocol allocation independently from the
/// concrete public-key representation selected by `K`.
///
/// The implementations can be completely non-anonymous (e.g., plain Ethereum transactions from
/// node's Safe), or anonymous using a privacy pool in the background.
///
/// In general, any anonymous privacy pool must be able to implement this trait
/// to be used with PIX in production setup.
///
/// The keypair type selects the pool's curve at compile time. Implementations
/// MUST reject a reconstructed [`PixDepositSecret`] whose curve does not match `K`.
///
/// The implementations should take care of all retry and reliability concerns, so the
/// caller can assume that the operations will do best effort to succeed.
#[async_trait::async_trait]
#[auto_impl::auto_impl(&, Box, Arc)]
pub trait DepositPool<K>
where
    K: Keypair<SecretLen = hopr_types::primitive::typenum::U32> + Send + Sync + 'static,
    <K as Keypair>::Public: Into<PixDepositAddress> + Send + Sync + 'static,
{
    /// Errors on failures.
    type Error: std::error::Error + Send + Sync + 'static;
    /// Some receipt returned on successful deposits and withdrawals.
    type Receipt: Send + Sync + 'static;

    /// Deposits `amount` of funds from node's Safe to the given `dst` deposit address.
    ///
    /// `id` is the idempotency key. Repeating a successfully submitted allocation
    /// with the same `id`, `dst`, and `amount` MUST NOT allocate funds twice. Reusing
    /// an `id` with different parameters MUST fail.
    async fn deposit_funds_to(
        &self,
        id: PixAddressId,
        dst: K::Public,
        amount: HoprBalance,
    ) -> Result<Self::Receipt, Self::Error>;

    /// Performs batch deposit of funds from node's Safe to multiple deposit addresses.
    ///
    /// This default implementation simply concurrently calls [`self.deposit_funds_to`].
    /// Implementors may choose a more efficient pool-native batching.
    ///
    /// A successful return means that every deposit was accepted by the pool operation.
    /// Receipts describe pool transactions, not individual inputs: a native atomic batch
    /// may therefore return one receipt, while the default implementation returns one
    /// receipt per input. Callers MUST NOT correlate receipts and deposits by position.
    async fn deposit_funds_to_multiple(
        &self,
        deposits: Vec<(PixAddressId, K::Public, HoprBalance)>,
    ) -> Result<Vec<Self::Receipt>, Self::Error> {
        let futures = deposits
            .into_iter()
            .map(|(id, dst, amount)| async move { self.deposit_funds_to(id, dst, amount).await });
        join_all(futures).await.into_iter().collect()
    }

    /// Returns a future that resolves once at least `min_amount` is committed at
    /// `dst` and available for the PIX session.
    ///
    /// This operation runs on the Exit before the SURB threshold reconstructs the
    /// corresponding [`PixDepositSecret`]. Implementations therefore MUST observe
    /// the allocation using only `dst` and pool-specific viewing material; they
    /// MUST NOT require the reconstructed withdrawal secret.
    ///
    /// The returned amount is the cumulative committed amount observed for `dst`,
    /// including matching deposits committed before this method was called. Replayed
    /// or duplicate chain events MUST NOT be counted twice. Implementations MUST
    /// resume safely across disconnections and process chain reorganizations according
    /// to the finality guarantees of their backend before resolving the future.
    ///
    /// Dropping the returned future cancels only that waiter; it MUST NOT remove
    /// persisted deposit state or affect other waiters. The returned future is
    /// `'static` so it can be spawned independently of the borrow on `&self`.
    fn notify_deposit(
        &self,
        id: PixAddressId,
        dst: K::Public,
        min_amount: HoprBalance,
    ) -> Result<DepositNotification<'static, K::Public, Self::Error>, Self::Error>;

    /// Performs withdrawal of a previously made deposit using its tagged
    /// [`PixDepositSecret`] to the `dst` Ethereum address. On the Exit, this secret
    /// is supplied by the PIX protocol after threshold reconstruction from the
    /// shares carried by used SURBs; the deposit-pool implementation does not
    /// reconstruct it.
    ///
    /// If `amount` is specified, exactly that amount MUST be withdrawn; an
    /// implementation that cannot produce change MUST return an error rather than
    /// over-withdraw. If it is `None`, the entire deposit is withdrawn.
    async fn withdraw_deposit(
        &self,
        id: PixAddressId,
        key: &K,
        dst: Address,
        amount: Option<HoprBalance>,
    ) -> Result<Self::Receipt, Self::Error>;

    /// Performs batch [full withdrawal](Self::withdraw_deposit) of multiple deposits into a single Ethereum address.
    ///
    /// This default implementation simply concurrently calls [`self.withdraw_deposit`].
    /// Implementors may choose a more efficient pool-native batching.
    ///
    /// On outer success, the returned vector MUST contain exactly one result for each
    /// input key in the same order. The outer error represents failure to construct or
    /// submit the batch as a whole; individual key failures belong in the corresponding
    /// inner result. A native atomic batch may use one chain transaction internally,
    /// but must still expose one logical result per input.
    async fn withdraw_multiple_deposits(
        &self,
        deposits: &[(PixAddressId, K)],
        dst: Address,
    ) -> Result<Vec<Result<(Address, Self::Receipt), Self::Error>>, Self::Error> {
        let futures = deposits.iter().map(|(id, key)| async move {
            self.withdraw_deposit(*id, key, dst, None)
                .await
                .map(|receipt| (dst, receipt))
        });
        Ok(join_all(futures).await)
    }

    /// Transfers the funds from the deposit owned by `key` into another address within the pool.
    ///
    /// If `amount` is `None`, the entire deposit balance is transferred.
    async fn pool_transfer(
        &self,
        source_id: PixAddressId,
        key: &K,
        destination_id: PixAddressId,
        dst: K::Public,
        amount: Option<HoprBalance>,
    ) -> Result<Self::Receipt, Self::Error>;

    /// Performs batch [full transfer](Self::pool_transfer) of multiple deposits into a single deposit address.
    ///
    /// This default implementation simply concurrently calls [`self.pool_transfer`].
    /// Implementors may choose a more efficient pool-native batching.
    async fn pool_transfer_multiple(
        &self,
        deposits: &[(PixAddressId, K)],
        destination_id: PixAddressId,
        dst: K::Public,
    ) -> Result<Vec<Result<(K::Public, Self::Receipt), Self::Error>>, Self::Error> {
        let futures = deposits.iter().map(|(source_id, key)| {
            let dst = dst.clone();
            async move {
                self.pool_transfer(*source_id, key, destination_id, dst.clone(), None)
                    .await
                    .map(|receipt| (dst, receipt))
            }
        });
        Ok(join_all(futures).await)
    }
}
