use futures::future::{BoxFuture, join_all};
pub use hopr_types::crypto::primitives::{PixDepositAddress, PixDepositSecret};
use hopr_types::{
    crypto::prelude::Keypair,
    network::PixAddressId,
    primitive::prelude::{Address, HoprBalance},
};

/// Opaque pool-specific data associated with a PIX deposit.
///
/// The PIX protocol and strategy only transport these bytes. Their meaning is defined by the
/// selected [`DepositPool`] implementation. Callers must not log the contents because an
/// implementation may use them to carry private scanning material on the Exit side.
pub type AdditionalDepositData = Box<[u8]>;

/// A future that resolves once `min_amount` has been deposited to the `dst` [`PixDepositAddress`]
/// or an error occurs.
pub type DepositNotification<'a, P, E> = BoxFuture<'a, Result<(PixAddressId, P, HoprBalance), E>>;

/// Contains abstraction over the deposit pool from PIX.
///
/// The funds within this pool are represented by the given keypair `K`.
///
/// The secret and public key of the keypair should be convertible to [`PixDepositSecret`] and [`PixDepositAddress`],
/// respectively. The former is enforced by matching the [length of the keypair secret](Keypair::SecretLen) to the
/// length of `PixDepositSecret` and the latter is enforced by the `Into<PixDepositAddress>` bound.
///
/// The implementations can be completely non-anonymous (e.g., plain Ethereum transactions from
/// node's Safe), or anonymous using a privacy pool in the background.
///
/// In general, any anonymous privacy pool must be able to implement this trait
/// to be used with PIX in production setup.
///
/// The implementations should take care of all the retry/reliabililty of the operations, so the
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
    /// `id` is the stable local PIX allocation identifier. `additional_data` is interpreted only
    /// by the pool implementation and should be handled idempotently together with `id`.
    async fn deposit_funds_to(
        &self,
        id: PixAddressId,
        dst: K::Public,
        additional_data: Option<AdditionalDepositData>,
        amount: HoprBalance,
    ) -> Result<Self::Receipt, Self::Error>;

    /// Performs batch deposit of funds from node's Safe to multiple deposit addresses.
    ///
    /// This default implementation simply concurrently calls [`self.deposit_funds_to`].
    /// Implementors may choose a more efficient pool-native batching.
    ///
    /// The method is allowed to return fewer receipts than deposits.
    async fn deposit_funds_to_multiple(
        &self,
        deposits: Vec<(PixAddressId, K::Public, Option<AdditionalDepositData>, HoprBalance)>,
    ) -> Result<Vec<Self::Receipt>, Self::Error> {
        let futures = deposits.into_iter().map(|(id, dst, additional_data, amount)| async move {
            self.deposit_funds_to(id, dst, additional_data, amount).await
        });
        join_all(futures).await.into_iter().collect()
    }

    /// Returns a future that resolves once `min_amount` has been deposited to the `dst` [`PixDepositAddress`].
    ///
    /// The returned future is `'static` so it can be spawned independently of the borrow on `&self`.
    fn notify_deposit(
        &self,
        id: PixAddressId,
        dst: K::Public,
        additional_data: Option<AdditionalDepositData>,
        min_amount: HoprBalance,
    ) -> Result<DepositNotification<'static, K::Public, Self::Error>, Self::Error>;

    /// Performs withdrawal of a previously made deposit using its [`PixDepositSecret`] to the
    /// `dst` Ethereum address.
    ///
    /// Should allow for partial withdrawals if `amount` is specified,
    /// otherwise withdraws the entire deposit.
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
    async fn withdraw_multiple_deposits(
        &self,
        keys: &[(PixAddressId, K)],
        dst: Address,
    ) -> Result<Vec<Result<(Address, Self::Receipt), Self::Error>>, Self::Error> {
        let futures = keys.iter().map(|(id, key)| async move {
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
        destination_data: Option<AdditionalDepositData>,
        amount: Option<HoprBalance>,
    ) -> Result<Self::Receipt, Self::Error>;

    /// Performs batch [full transfer](Self::pool_transfer) of multiple deposits into a single deposit address.
    ///
    /// This default implementation simply concurrently calls [`self.pool_transfer`].
    /// Implementors may choose a more efficient pool-native batching.
    async fn pool_transfer_multiple(
        &self,
        keys: &[(PixAddressId, K)],
        destination_id: PixAddressId,
        dst: K::Public,
        destination_data: Option<AdditionalDepositData>,
    ) -> Result<Vec<Result<(K::Public, Self::Receipt), Self::Error>>, Self::Error> {
        let futures = keys.iter().map(|(source_id, key)| {
            let dst = dst.clone();
            let destination_data = destination_data.clone();
            async move {
                self.pool_transfer(
                    *source_id,
                    key,
                    destination_id,
                    dst.clone(),
                    destination_data,
                    None,
                )
                    .await
                    .map(|receipt| (dst, receipt))
            }
        });
        Ok(join_all(futures).await)
    }
}
