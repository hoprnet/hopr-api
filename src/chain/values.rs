use std::{error::Error, time::Duration};

pub use hopr_types::{
    chain::ContractAddresses, internal::prelude::WinningProbability, primitive::balance::HoprBalance,
};
use hopr_types::{
    crypto::prelude::Hash,
    primitive::{
        balance::{Balance, Currency},
        prelude::Address,
    },
};

/// Contains domain separator information.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct DomainSeparators {
    /// HOPR Ledger smart contract domain separator.
    pub ledger: Hash,
    /// HOPR Node Safe Registry smart contract domain separator.
    pub safe_registry: Hash,
    /// HOPR Channels smart contract domain separator.
    pub channel: Hash,
}

/// Contains information about the HOPR on-chain network deployment.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ChainInfo {
    /// ID of the blockchain network (e.g.: `0x64` for Gnosis Chain)
    pub chain_id: u64,
    /// Name of the HOPR network (e.g.: `dufour`)
    pub hopr_network_name: String,
    /// Addresses of the deployed HOPR smart contracts.
    pub contract_addresses: ContractAddresses,
}

/// Ticket redemption statistics for a Safe.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct RedemptionStats {
    /// Total number of tickets that have been redeemed.
    pub redeemed_count: u64,
    /// Total value of tickets that have been redeemed.
    pub redeemed_value: HoprBalance,
}

/// Retrieves various on-chain information.
#[async_trait::async_trait]
#[auto_impl::auto_impl(&, Box, Arc)]
pub trait ChainValues {
    type Error: Error + Send + Sync + 'static;

    /// Returns the native or token currency balance of the given on-chain account.
    async fn balance<C: Currency, A: Into<Address> + Send>(&self, address: A) -> Result<Balance<C>, Self::Error>;
    /// Retrieves the domain separators of HOPR smart contracts.
    async fn domain_separators(&self) -> Result<DomainSeparators, Self::Error>;
    /// Retrieves the network-set minimum **incoming** ticket winning probability.
    async fn minimum_incoming_ticket_win_prob(&self) -> Result<WinningProbability, Self::Error>;
    /// Retrieves the network-set minimum ticket price.
    async fn minimum_ticket_price(&self) -> Result<HoprBalance, Self::Error>;
    /// Retrieves the current key binding fee
    /// used for new key-binding [announcements](crate::chain::ChainWriteAccountOperations::announce).
    async fn key_binding_fee(&self) -> Result<HoprBalance, Self::Error>;
    /// Gets the grace period for channel closure finalization.
    async fn channel_closure_notice_period(&self) -> Result<Duration, Self::Error>;
    /// Gets the information about the HOPR network on-chain deployment.
    async fn chain_info(&self) -> Result<ChainInfo, Self::Error>;
    /// Gets the ticket redemption stats for the given safe address.
    async fn redemption_stats<A: Into<Address> + Send>(&self, safe_addr: A) -> Result<RedemptionStats, Self::Error>;
    /// Returns the expected time for on-chain events to be resolved.
    async fn typical_resolution_time(&self) -> Result<Duration, Self::Error>;
}
