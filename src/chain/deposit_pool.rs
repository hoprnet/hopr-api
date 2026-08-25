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

/// Per-item outcomes of a batch [`DepositPool`] operation.
pub type BatchOutcomes<R, E> = Vec<Result<(PixAddressId, R), E>>;

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
    async fn deposit_funds_to_multiple(
        &self,
        deposits: &[(PixAddressId, K::Public, HoprBalance, Self::PoolDepositData)],
    ) -> Result<BatchOutcomes<Self::Receipt, Self::Error>, Self::Error> {
        let futures = deposits.iter().map(|(id, dst, amount, data)| {
            let data = data.clone();
            async move {
                self.deposit_funds_to(id, dst, *amount, data)
                    .await
                    .map(|receipt| (*id, receipt))
            }
        });
        Ok(join_all(futures).await)
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
    ) -> Result<BatchOutcomes<Self::Receipt, Self::Error>, Self::Error> {
        let futures = keys.iter().map(|(id, key)| async move {
            self.withdraw_deposit(id, key, dst, None)
                .await
                .map(|receipt| (*id, receipt))
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
    ///
    /// One result per key, identified by its *source* allocation.
    async fn pool_transfer_multiple(
        &self,
        keys: &[(PixAddressId, K)],
        dst_id: &PixAddressId,
        dst: K::Public,
        additional_dst_data: Self::PoolDepositData,
    ) -> Result<BatchOutcomes<Self::Receipt, Self::Error>, Self::Error> {
        let futures = keys.iter().map(|(id, key)| {
            let dst = dst.clone();
            let dst_data = additional_dst_data.clone();
            let dst_id = *dst_id;
            async move {
                self.pool_transfer(id, key, &dst_id, dst, dst_data, None)
                    .await
                    .map(|receipt| (*id, receipt))
            }
        });
        Ok(join_all(futures).await)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use hopr_types::{
        crypto::prelude::{BjjKeypair, BjjPublicKey},
        primitive::traits::BytesRepresentable,
    };

    use super::*;

    #[derive(Debug, thiserror::Error)]
    #[error("mock pool failure")]
    struct MockError;

    /// The mock's pool-native deposit data, fallibly convertible both ways as the trait requires.
    /// Wrapping [`PixDepositData`] keeps the conversions total, so a test never fails for a reason
    /// unrelated to what it asserts.
    #[derive(Clone, Debug, PartialEq, Eq)]
    struct MockDepositData(PixDepositData);

    impl TryFrom<PixDepositData> for MockDepositData {
        type Error = MockError;

        fn try_from(value: PixDepositData) -> Result<Self, Self::Error> {
            Ok(Self(value))
        }
    }

    impl TryFrom<MockDepositData> for PixDepositData {
        type Error = MockError;

        fn try_from(value: MockDepositData) -> Result<Self, Self::Error> {
            Ok(value.0)
        }
    }

    /// A pool that fails for a chosen set of destinations and succeeds for the rest, so a batch
    /// can be made to partially fail. The receipt is the amount, which makes it visible whether a
    /// receipt was paired with the right element.
    #[derive(Default)]
    struct MockPool {
        fail_for: Vec<BjjPublicKey>,
        seen: Mutex<Vec<PixAddressId>>,
    }

    impl MockPool {
        fn fails_for(dst: &BjjPublicKey) -> Self {
            Self {
                fail_for: vec![*dst],
                ..Default::default()
            }
        }

        fn would_fail(&self, dst: &BjjPublicKey) -> bool {
            self.fail_for.contains(dst)
        }
    }

    #[async_trait::async_trait]
    impl DepositPool<BjjKeypair> for MockPool {
        type Error = MockError;
        type PoolDepositData = MockDepositData;
        type Receipt = HoprBalance;

        async fn generate_deposit_data(&self, id: &PixAddressId) -> Result<Self::PoolDepositData, Self::Error> {
            Ok(deposit_data_for(*id))
        }

        async fn deposit_funds_to(
            &self,
            id: &PixAddressId,
            dst: &BjjPublicKey,
            amount: HoprBalance,
            _additional_data: Self::PoolDepositData,
        ) -> Result<Self::Receipt, Self::Error> {
            self.seen.lock().unwrap().push(*id);
            if self.would_fail(dst) {
                Err(MockError)
            } else {
                Ok(amount)
            }
        }

        fn notify_deposit(
            &self,
            _id: PixAddressId,
            _dst: BjjPublicKey,
            _min_amount: HoprBalance,
        ) -> Result<DepositNotification<'static, BjjPublicKey, Self::Error>, Self::Error> {
            Err(MockError)
        }

        async fn withdraw_deposit(
            &self,
            id: &PixAddressId,
            key: &BjjKeypair,
            _dst: Address,
            _amount: Option<HoprBalance>,
        ) -> Result<Self::Receipt, Self::Error> {
            self.seen.lock().unwrap().push(*id);
            if self.would_fail(key.public()) {
                Err(MockError)
            } else {
                Ok(HoprBalance::new_base(1))
            }
        }

        async fn pool_transfer(
            &self,
            src_id: &PixAddressId,
            key: &BjjKeypair,
            _dst_id: &PixAddressId,
            _dst: BjjPublicKey,
            _additional_dst_data: Self::PoolDepositData,
            _amount: Option<HoprBalance>,
        ) -> Result<Self::Receipt, Self::Error> {
            self.seen.lock().unwrap().push(*src_id);
            if self.would_fail(key.public()) {
                Err(MockError)
            } else {
                Ok(HoprBalance::new_base(1))
            }
        }
    }

    /// A distinct allocation id per `n`, built from bytes so the test needs no RNG feature.
    fn id(n: u8) -> PixAddressId {
        let mut bytes = [0u8; PixAddressId::SIZE];
        bytes[0] = n;
        // The SSA index is the trailing big-endian `NonZeroU32` and must not be zero.
        bytes[PixAddressId::SIZE - 1] = 1;
        PixAddressId::try_from(&bytes[..]).expect("must be a valid allocation id")
    }

    /// Deposit data for the allocation `id`, carrying no pool-specific payload.
    fn deposit_data_for(id: PixAddressId) -> MockDepositData {
        MockDepositData(PixDepositData {
            id,
            data: Box::default(),
        })
    }

    /// Shorthand for the deposit data of allocation `n`.
    fn data(n: u8) -> MockDepositData {
        deposit_data_for(id(n))
    }

    /// The point of the per-item result: one failure must not take the successful receipts with it.
    ///
    /// The previous signature returned `Result<Vec<Receipt>, Error>` and the default implementation
    /// `collect()`ed into it, so the first `Err` short-circuited and every receipt already earned
    /// was dropped — while the doc claimed the method "is allowed to return fewer receipts than
    /// deposits".
    #[tokio::test]
    async fn deposit_funds_to_multiple_keeps_receipts_of_the_deposits_that_succeeded() {
        let doomed = BjjKeypair::random();
        let pool = MockPool::fails_for(doomed.public());

        let (first, third) = (BjjKeypair::random(), BjjKeypair::random());
        let deposits = [
            (id(1), *first.public(), HoprBalance::new_base(10), data(1)),
            (id(2), *doomed.public(), HoprBalance::new_base(20), data(2)),
            (id(3), *third.public(), HoprBalance::new_base(30), data(3)),
        ];

        let results = pool
            .deposit_funds_to_multiple(&deposits)
            .await
            .expect("the batch itself must be attempted");

        assert_eq!(results.len(), deposits.len(), "one result per deposit");
        assert!(results[1].is_err(), "the doomed deposit must report its own failure");

        let (reported_id, receipt) = results[0].as_ref().expect("the first deposit succeeded");
        assert_eq!(*reported_id, id(1), "a receipt must name the allocation it belongs to");
        assert_eq!(*receipt, HoprBalance::new_base(10));

        let (reported_id, receipt) = results[2].as_ref().expect("the third deposit succeeded");
        assert_eq!(*reported_id, id(3), "a receipt after a failure must still be correct");
        assert_eq!(*receipt, HoprBalance::new_base(30));

        // Redundant given the above, but states the property the id exists for: nothing here
        // needed the position of a result to know which deposit it describes.
        let _ = (first, third);
    }

    /// Every deposit is attempted, including the ones after a failure.
    #[tokio::test]
    async fn deposit_funds_to_multiple_attempts_every_deposit() {
        let doomed = BjjKeypair::random();
        let pool = MockPool::fails_for(doomed.public());
        let deposits = [
            (id(1), *doomed.public(), HoprBalance::new_base(10), data(1)),
            (
                id(2),
                *BjjKeypair::random().public(),
                HoprBalance::new_base(20),
                data(2),
            ),
        ];

        let results = pool.deposit_funds_to_multiple(&deposits).await.expect("attempted");

        assert_eq!(results.len(), 2);
        let mut seen = pool.seen.lock().unwrap().clone();
        seen.sort();
        assert_eq!(
            seen,
            vec![id(1), id(2)],
            "a failure must not skip the rest of the batch"
        );
    }

    /// A withdrawal outcome names its own allocation, not the batch's shared destination.
    #[tokio::test]
    async fn withdraw_multiple_deposits_identifies_each_outcome_by_allocation() {
        let doomed = BjjKeypair::random();
        let pool = MockPool::fails_for(doomed.public());
        let keys = [(id(1), BjjKeypair::random()), (id(2), doomed)];

        let results = pool
            .withdraw_multiple_deposits(&keys, Address::from([7u8; 20]))
            .await
            .expect("attempted");

        assert_eq!(results.len(), 2);
        let (reported_id, _) = results[0].as_ref().expect("the first key was swept");
        assert_eq!(*reported_id, id(1));
        assert!(results[1].is_err());
    }

    /// Likewise for the in-pool transfer batch: the *source* allocation identifies the outcome.
    /// `dst_id` cannot, being one value for the whole batch and the caller's own argument.
    #[tokio::test]
    async fn pool_transfer_multiple_identifies_each_outcome_by_source_allocation() {
        let doomed = BjjKeypair::random();
        let pool = MockPool::fails_for(doomed.public());
        let keys = [(id(1), doomed), (id(2), BjjKeypair::random())];
        let dst_id = id(9);

        let results = pool
            .pool_transfer_multiple(&keys, &dst_id, *BjjKeypair::random().public(), deposit_data_for(dst_id))
            .await
            .expect("attempted");

        assert_eq!(results.len(), 2);
        assert!(results[0].is_err());
        let (reported_id, _) = results[1].as_ref().expect("the second transfer succeeded");
        assert_eq!(*reported_id, id(2), "the source allocation, not the shared destination");
        assert_ne!(*reported_id, dst_id);
    }

    /// The property the ids buy: an implementor may return outcomes in any order, and a caller
    /// still attributes them correctly.
    ///
    /// A pool overriding a batch method with pool-native batching has no obligation to preserve
    /// input order — and the trait already allows returning fewer outcomes than inputs. Before
    /// 4.0 both facts were unusable, because matching an outcome to its input meant trusting its
    /// position. `ReorderingPool` returns its outcomes reversed and drops one entirely; nothing
    /// below counts positions.
    #[tokio::test]
    async fn outcomes_are_attributable_when_an_implementor_reorders_them() {
        struct ReorderingPool;

        #[async_trait::async_trait]
        impl DepositPool<BjjKeypair> for ReorderingPool {
            type Error = MockError;
            type PoolDepositData = MockDepositData;
            type Receipt = HoprBalance;

            async fn generate_deposit_data(&self, id: &PixAddressId) -> Result<Self::PoolDepositData, Self::Error> {
                Ok(deposit_data_for(*id))
            }

            async fn deposit_funds_to(
                &self,
                _id: &PixAddressId,
                _dst: &BjjPublicKey,
                amount: HoprBalance,
                _additional_data: Self::PoolDepositData,
            ) -> Result<Self::Receipt, Self::Error> {
                Ok(amount)
            }

            async fn deposit_funds_to_multiple(
                &self,
                deposits: &[(PixAddressId, BjjPublicKey, HoprBalance, Self::PoolDepositData)],
            ) -> Result<BatchOutcomes<Self::Receipt, Self::Error>, Self::Error> {
                // Reversed, and the first input is omitted as if it had never been attempted.
                Ok(deposits
                    .iter()
                    .skip(1)
                    .rev()
                    .map(|(id, _, amount, _)| Ok((*id, *amount)))
                    .collect())
            }

            fn notify_deposit(
                &self,
                _id: PixAddressId,
                _dst: BjjPublicKey,
                _min_amount: HoprBalance,
            ) -> Result<DepositNotification<'static, BjjPublicKey, Self::Error>, Self::Error> {
                Err(MockError)
            }

            async fn withdraw_deposit(
                &self,
                _id: &PixAddressId,
                _key: &BjjKeypair,
                _dst: Address,
                _amount: Option<HoprBalance>,
            ) -> Result<Self::Receipt, Self::Error> {
                Err(MockError)
            }

            async fn pool_transfer(
                &self,
                _src_id: &PixAddressId,
                _key: &BjjKeypair,
                _dst_id: &PixAddressId,
                _dst: BjjPublicKey,
                _additional_dst_data: Self::PoolDepositData,
                _amount: Option<HoprBalance>,
            ) -> Result<Self::Receipt, Self::Error> {
                Err(MockError)
            }
        }

        let deposits = [
            (
                id(1),
                *BjjKeypair::random().public(),
                HoprBalance::new_base(10),
                data(1),
            ),
            (
                id(2),
                *BjjKeypair::random().public(),
                HoprBalance::new_base(20),
                data(2),
            ),
            (
                id(3),
                *BjjKeypair::random().public(),
                HoprBalance::new_base(30),
                data(3),
            ),
        ];

        let by_allocation: std::collections::BTreeMap<_, _> = ReorderingPool
            .deposit_funds_to_multiple(&deposits)
            .await
            .expect("attempted")
            .into_iter()
            .flatten()
            .collect();

        assert_eq!(by_allocation.get(&id(2)), Some(&HoprBalance::new_base(20)));
        assert_eq!(by_allocation.get(&id(3)), Some(&HoprBalance::new_base(30)));
        assert_eq!(by_allocation.get(&id(1)), None, "an unattempted input is simply absent");
    }
}
