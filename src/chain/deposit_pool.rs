use futures::future::{BoxFuture, join_all};
pub use hopr_types::crypto::primitives::{PixAddressId, PixDepositAddress, PixDepositSecret};
use hopr_types::{
    crypto::prelude::Keypair,
    primitive::prelude::{Address, HoprBalance},
};

use crate::node::PixDepositData;

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

    /// Pool-specific data associated with a PIX deposit.
    ///
    /// The type must be fallibly-convertible to/from [`PixDepositData`].
    type PoolDepositData: Clone
        + TryFrom<PixDepositData, Error = Self::Error>
        + TryInto<PixDepositData, Error = Self::Error>
        + Send
        + Sync
        + 'static;

    /// Generates additional deposit data identified by `id`.
    async fn generate_deposit_data(&self, _id: &PixAddressId) -> Result<Self::PoolDepositData, Self::Error>;

    /// Deposits `amount` of funds from node's Safe to the given `dst` deposit address.
    async fn deposit_funds_to(
        &self,
        id: &PixAddressId,
        dst: &K::Public,
        amount: HoprBalance,
        additional_data: Self::PoolDepositData,
    ) -> Result<Self::Receipt, Self::Error>;

    /// Performs batch deposit of funds from node's Safe to multiple deposit addresses.
    ///
    /// This default implementation simply concurrently calls `deposit_funds_to`.
    /// Implementors may choose a more efficient pool-native batching.
    ///
    /// The method is allowed to return fewer receipts than deposits.
    async fn deposit_funds_to_multiple(
        &self,
        deposits: &[(PixAddressId, K::Public, HoprBalance, Self::PoolDepositData)],
    ) -> Result<Vec<Self::Receipt>, Self::Error> {
        let futures = deposits.iter().map(|(id, dst, amount, data)| {
            let data = data.clone();
            async move { self.deposit_funds_to(id, dst, *amount, data).await }
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
        min_amount: HoprBalance,
    ) -> Result<DepositNotification<'static, K::Public, Self::Error>, Self::Error>;

    /// Performs withdrawal of a previously made deposit using its [`PixDepositSecret`] to the
    /// `dst` Ethereum address.
    ///
    /// Should allow for partial withdrawals if `amount` is specified,
    /// otherwise withdraws the entire deposit.
    async fn withdraw_deposit(
        &self,
        id: &PixAddressId,
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
            self.withdraw_deposit(id, key, dst, None)
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
        src_id: &PixAddressId,
        key: &K,
        dst_id: &PixAddressId,
        dst: K::Public,
        additional_dst_data: Self::PoolDepositData,
        amount: Option<HoprBalance>,
    ) -> Result<Self::Receipt, Self::Error>;

    /// Performs batch [full transfer](Self::pool_transfer) of multiple deposits into a single deposit address.
    ///
    /// This default implementation simply concurrently calls [`self.pool_transfer`].
    /// Implementors may choose a more efficient pool-native batching.
    async fn pool_transfer_multiple(
        &self,
        keys: &[(PixAddressId, K)],
        dst_id: &PixAddressId,
        dst: K::Public,
        additional_dst_data: Self::PoolDepositData,
    ) -> Result<Vec<Result<(K::Public, Self::Receipt), Self::Error>>, Self::Error> {
        let futures = keys.iter().map(|(id, key)| {
            let dst = dst.clone();
            let dst_data = additional_dst_data.clone();
            let dst_id = *dst_id;
            async move {
                self.pool_transfer(id, key, &dst_id, dst.clone(), dst_data, None)
                    .await
                    .map(|receipt| (dst, receipt))
            }
        });
        Ok(join_all(futures).await)
    }
}
