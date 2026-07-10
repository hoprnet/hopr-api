//! Data types used across the node API.

use std::{future::Future, num::NonZeroU32};

use hopr_types::{
    chain::chain_events::ChainEvent,
    crypto::{types::BjjPublicKey, utils::SecretValue},
    internal::prelude::{HoprPseudonym, RedeemableTicket, Ticket},
    primitive::{balance::HoprBalance, prelude::Address, traits::BytesRepresentable},
};

use super::CompoundResult;

/// Identity of a node on-chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct NodeOnchainIdentity {
    /// Node's on-chain address.
    pub node_address: Address,
    /// Address of the node's associated Safe.
    pub safe_address: Address,
    /// Address of the Safe module.
    pub module_address: Address,
}

/// Represents an output of a write operation to the chain performed by the node.
///
/// This consists always of a transaction hash and an optional output `T`.
///
/// Operations that produce no useful output use `()` as `T`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ChainOutput<T> {
    tx_hash: hopr_types::crypto::prelude::Hash,
    output: Option<T>,
}

impl<T> ChainOutput<T> {
    /// Creates a new ChainOutput with the given transaction hash and output.
    pub fn new(tx_hash: hopr_types::crypto::prelude::Hash, output: T) -> Self {
        Self {
            tx_hash,
            output: output.into(),
        }
    }

    /// Returns the transaction hash of the chain operation.
    pub fn tx_hash(&self) -> &hopr_types::crypto::prelude::Hash {
        &self.tx_hash
    }

    /// Returns the optional output of the chain operation.
    pub fn output(&self) -> Option<&T> {
        self.output.as_ref()
    }
}

impl ChainOutput<()> {
    /// Creates a new ChainOutput with the given transaction hash and no output.
    pub fn new_empty(tx_hash: hopr_types::crypto::prelude::Hash) -> Self {
        Self { tx_hash, output: None }
    }
}

impl From<hopr_types::crypto::prelude::Hash> for ChainOutput<()> {
    fn from(tx_hash: hopr_types::crypto::prelude::Hash) -> Self {
        Self::new_empty(tx_hash)
    }
}

/// Future that resolves when a [`ChainEvent`] is resolved, times out, or is aborted
/// via the associated abort handle.
pub type ChainEventResolver<ChainErr, WaitErr> = (
    std::pin::Pin<Box<dyn Future<Output = CompoundResult<ChainEvent, ChainErr, WaitErr>> + Send + 'static>>,
    futures::future::AbortHandle,
);

/// Alias for the result of [`HasChainApi::wait_for_on_chain_event`](super::HasChainApi::wait_for_on_chain_event).
pub type EventWaitResult<ChainErr, WaitErr> = Result<ChainEventResolver<ChainErr, WaitErr>, ChainErr>;

/// Origin of a peer announcement — how the node learned about this peer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum AnnouncementOrigin {
    /// Announced via on-chain registration.
    Chain,
    /// Discovered via DHT (future).
    DHT,
}

/// A peer that has been announced and discovered by the node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnnouncedPeer {
    /// On-chain address of the peer.
    pub address: Address,
    /// Multiaddresses associated with this peer.
    pub multiaddresses: Vec<crate::Multiaddr>,
    /// How the announcement was discovered.
    pub origin: AnnouncementOrigin,
}

/// Identifier for a Pix deposit address.
pub type PixAddressId = (HoprPseudonym, NonZeroU32);

/// Used to notify that the Exit has registered a sufficient deposit on a deposit address.
pub type DepositUpdated = futures::channel::mpsc::Sender<(PixAddressId, HoprBalance)>;

/// An address representing a PIX deposit.
#[derive(Clone, Debug, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct PixDepositAddress(u8, [u8; 32]);

const ADDRESS_TYPE_ETH: u8 = 0x01;
const ADDRESS_TYPE_BJJ: u8 = 0x02;

impl AsRef<[u8]> for PixDepositAddress {
    fn as_ref(&self) -> &[u8] {
        &self.1
    }
}

impl From<Address> for PixDepositAddress {
    fn from(value: Address) -> Self {
        const { assert!(Address::SIZE <= 32) };
        let mut ret = PixDepositAddress::default();
        ret.0 = ADDRESS_TYPE_ETH;
        ret.1[0..Address::SIZE].copy_from_slice(value.as_ref());
        ret
    }
}

impl TryFrom<PixDepositAddress> for Address {
    type Error = hopr_types::primitive::errors::GeneralError;

    fn try_from(value: PixDepositAddress) -> Result<Self, Self::Error> {
        if value.0 != ADDRESS_TYPE_ETH {
            return Err(hopr_types::primitive::errors::GeneralError::InvalidInput);
        }
        if value.1[Address::SIZE..].iter().any(|&b| b != 0) {
            return Err(hopr_types::primitive::errors::GeneralError::InvalidInput);
        }
        let mut ret = [0u8; Address::SIZE];
        ret.copy_from_slice(&value.1[0..Address::SIZE]);
        Ok(ret.into())
    }
}

impl From<BjjPublicKey> for PixDepositAddress {
    fn from(value: BjjPublicKey) -> Self {
        const { assert!(BjjPublicKey::SIZE == 32) };
        let mut ret = PixDepositAddress::default();
        ret.0 = ADDRESS_TYPE_BJJ;
        ret.1[0..BjjPublicKey::SIZE].copy_from_slice(value.as_ref());
        ret
    }
}

impl TryFrom<PixDepositAddress> for BjjPublicKey {
    type Error = hopr_types::primitive::errors::GeneralError;

    fn try_from(value: PixDepositAddress) -> Result<Self, Self::Error> {
        if value.0 != ADDRESS_TYPE_BJJ {
            return Err(hopr_types::primitive::errors::GeneralError::InvalidInput);
        }
        value.1.as_ref().try_into()
    }
}

/// A secret corresponding to a PIX deposit address.
///
/// Usually the [`PixDepositAddress`] can be calculated from the secret.
#[derive(Clone, Debug)]
pub struct PixDepositSecret(pub SecretValue<hopr_types::primitive::typenum::U32>);

/// Data for [`NewDepositAddress`](PixEvent) event.
#[derive(Debug, Clone)]
pub struct PixNewDepositAddress {
    /// Identifier of the deposit address.
    pub id: PixAddressId,
    /// The address that can be used to make a deposit.
    pub address: PixDepositAddress,
    /// The quota in bytes that corresponds to this deposit.
    pub quota: u64,
}

/// Data for [`DepositAddressReceived`](PixEvent) event.
#[derive(Debug, Clone)]
pub struct PixDepositAddressReceived {
    /// Identifier of the deposit address.
    pub id: PixAddressId,
    /// The address that can be used to make a deposit.
    pub address: PixDepositAddress,
    /// The quota in bytes that corresponds to this deposit.
    pub quota: u64,
    /// Sender of the [`DepositUpdated`] events.
    ///
    /// The [`DepositUpdated`] is optionally used to give future feedback that a deposit has
    /// been received to the address. It may be used multiple times, and once the sum
    /// of all deposits reaches the price of the quota, the Exit can continue executing the PIX protocol.
    pub deposit_updated: Option<DepositUpdated>,
}

/// Data for [`PrivateKeyRecovered`](PixEvent) event.
#[derive(Debug, Clone)]
pub struct PixPrivateKeyRecovered {
    /// Identifier of the deposit address.
    pub id: PixAddressId,
    /// The recovered private key corresponding to the deposit address.
    ///
    /// This can be used to withdraw funds from the deposit address.
    pub secret: PixDepositSecret,
}

/// Events generated by the Protocol for Incentivization of eXits (PIX).
#[derive(Debug, Clone, strum::EnumDiscriminants, strum::EnumTryAs)]
#[strum_discriminants(name(PixEventDiscriminant), derive(Hash))]
pub enum PixEvent {
    /// A new deposit address was generated at the Entry node.
    ///
    /// This notifies the Entry node to make a deposit to the address with the given quota per SSA in bytes.
    ///
    /// Generated on the Entry node only.
    NewDepositAddress(PixNewDepositAddress),
    /// The Exit node received a new deposit address with the given quota per SSA in bytes.
    ///
    /// Generated on the Exit node only.
    DepositAddressReceived(PixDepositAddressReceived),
    /// The Exit has recovered the private key for a deposit address.
    ///
    /// Generated on the Exit node only.
    PrivateKeyRecovered(PixPrivateKeyRecovered),
}

/// Ticket events emitted from the packet processing pipeline.
#[derive(Debug, Clone, strum::EnumIs, strum::EnumTryAs)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum TicketEvent {
    /// A winning ticket was received.
    WinningTicket(Box<RedeemableTicket>),
    /// A ticket has been rejected.
    ///
    /// The optional address represents the ticket issuer and is present only
    /// if the ticket could be at least successfully verified.
    RejectedTicket(Box<Ticket>, Option<Address>),
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use hopr_types::crypto::{
        keypairs::Keypair,
        prelude::{BjjKeypair, ChainKeypair},
        types::PublicKey,
    };

    use super::*;

    #[test]
    fn announcement_origin_should_be_usable_as_hash_key() {
        let mut set = HashSet::new();
        set.insert(AnnouncementOrigin::Chain);
        set.insert(AnnouncementOrigin::DHT);
        set.insert(AnnouncementOrigin::Chain);
        assert_eq!(set.len(), 2);
    }

    #[test]
    fn announcement_origin_copy_should_preserve_value() {
        let origin = AnnouncementOrigin::Chain;
        let copied = origin;
        assert_eq!(origin, copied);
    }

    #[test]
    fn announced_peer_should_support_equality() {
        let addr = Address::default();
        let peer_a = AnnouncedPeer {
            address: addr,
            multiaddresses: vec![],
            origin: AnnouncementOrigin::Chain,
        };
        let peer_b = AnnouncedPeer {
            address: addr,
            multiaddresses: vec![],
            origin: AnnouncementOrigin::Chain,
        };
        assert_eq!(peer_a, peer_b);
    }

    #[test]
    fn announced_peers_with_different_origins_should_not_be_equal() {
        let addr = Address::default();
        let chain_peer = AnnouncedPeer {
            address: addr,
            multiaddresses: vec![],
            origin: AnnouncementOrigin::Chain,
        };
        let dht_peer = AnnouncedPeer {
            address: addr,
            multiaddresses: vec![],
            origin: AnnouncementOrigin::DHT,
        };
        assert_ne!(chain_peer, dht_peer);
    }

    #[test]
    fn announced_peer_clone_should_be_independent() {
        let addr = Address::default();
        let peer = AnnouncedPeer {
            address: addr,
            multiaddresses: vec!["/ip4/1.2.3.4/tcp/9091".parse().unwrap()],
            origin: AnnouncementOrigin::Chain,
        };
        let mut cloned = peer.clone();
        cloned.multiaddresses.push("/ip4/5.6.7.8/tcp/9092".parse().unwrap());
        assert_eq!(peer.multiaddresses.len(), 1);
        assert_eq!(cloned.multiaddresses.len(), 2);
    }

    #[test]
    fn deposit_addresses_interop() -> anyhow::Result<()> {
        let (_, pk1) = BjjKeypair::random().unzip();
        let addr1 = PixDepositAddress::from(pk1);
        assert_eq!(pk1, addr1.try_into()?);

        let (_, pk2) = ChainKeypair::random().unzip();
        let addr2 = PixDepositAddress::from(pk2.to_address());
        assert_eq!(pk2.to_address(), addr2.try_into()?);

        assert!(BjjPublicKey::try_from(addr2).is_err());
        assert!(Address::try_from(addr1).is_err());

        let default_addr = PixDepositAddress::default();
        assert!(Address::try_from(default_addr).is_err());
        assert!(BjjPublicKey::try_from(default_addr).is_err());

        Ok(())
    }

    #[test]
    fn address_from_pix_deposit_address_should_reject_non_zero_trailing_bytes() {
        let (_, pk) = ChainKeypair::random().unzip();
        let mut addr = PixDepositAddress::from(pk.to_address());
        addr.1[Address::SIZE..].copy_from_slice(&[0xff; 32 - Address::SIZE]);
        assert!(Address::try_from(addr).is_err());
    }
}
