use std::error::Error;

pub use hopr_types::primitive::prelude::{KeyIdMapping, KeyIdent as HoprKeyIdent};
use hopr_types::{crypto::prelude::OffchainPublicKey, primitive::prelude::Address};

/// Operations for offchain keys.
///
/// Provides translation between on-chain [`Address`] values and offchain [`OffchainPublicKey`] values.
#[auto_impl::auto_impl(&, Box, Arc)]
pub trait ChainKeyOperations {
    type Error: Error + Send + Sync + 'static;
    /// [Mapping](KeyIdMapping) between [`HoprKeyIdent`] and [`OffchainPublicKey`].
    type Mapper: KeyIdMapping<HoprKeyIdent, OffchainPublicKey> + Clone + Send + Sync + 'static;

    /// Translates [`Address`] into [`OffchainPublicKey`].
    fn chain_key_to_packet_key(&self, chain: &Address) -> Result<Option<OffchainPublicKey>, Self::Error>;

    /// Translates [`OffchainPublicKey`] into [`Address`].
    fn packet_key_to_chain_key(&self, packet: &OffchainPublicKey) -> Result<Option<Address>, Self::Error>;

    /// Returns a reference to [`KeyIdMapping`] for offchain key IDs.
    fn key_id_mapper_ref(&self) -> &Self::Mapper;
}
