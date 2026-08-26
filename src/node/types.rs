//! Data types used across the node API.

use std::future::Future;

pub use hopr_types::crypto::primitives::PixAddressId;
use hopr_types::{
    chain::chain_events::ChainEvent,
    crypto::primitives::{PixDepositAddress, PixDepositSecret},
    internal::prelude::{RedeemableTicket, Ticket},
    primitive::{balance::HoprBalance, prelude::Address},
};
use crate::chain::PixDepositData;
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

/// Used to notify that the Exit has registered a sufficient deposit on a deposit address.
pub type DepositUpdated = futures::channel::mpsc::Sender<(PixAddressId, HoprBalance)>;

/// Used to notify that the Exit has created additional deposit data for a deposit address.
///
/// If the [`PixDepositDataRequest`] requested data for multiple [`PixAddressId`], they are all
/// delivered using this channel.
///
/// The Exit node is responsible for fitting the received deposit data into the `SsaRequest` message
/// and rejecting the Session if the produced deposit data is not valid for over-the-wire transfer.
pub type DepositDataCreated = futures::channel::mpsc::Sender<PixDepositData>;

/// Data for [`NewDepositAddress`](PixEvent) event.
#[derive(Debug, Clone)]
pub struct PixNewDepositAddress {
    /// Identifier of the deposit address.
    pub id: PixAddressId,
    /// The address that can be used to make a deposit.
    pub address: PixDepositAddress,
    /// The quota in bytes that corresponds to this deposit.
    pub quota: u64,
    /// Additional data associated with the deposit.
    pub deposit_data: PixDepositData,
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
    /// Additional data associated with the deposit.
    pub deposit_data: PixDepositData,
    /// Sender of the [`DepositUpdated`] events.
    ///
    /// The `deposit_updated` is used to give future feedback that a deposit has
    /// been received to the address. It may be used multiple times, and once the sum
    /// of all deposits reaches the price of the quota, the Exit can continue executing the PIX protocol.
    pub deposit_updated: DepositUpdated,
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

/// Data for [`DepositDataRequest`](PixEvent) event.
#[derive(Debug, Clone)]
pub struct PixDepositDataRequest {
    /// PIX address identifiers to generate deposit data for.
    ///
    /// All of them should be delivered using the `deposit_data_created` channel.
    pub deposit_ids: Vec<PixAddressId>,
    /// Channel to deliver the deposit data to.
    pub deposit_data_created: DepositDataCreated,
}

/// Events generated by the Protocol for Incentivization of eXits (PIX).
///
/// The general flow is as follows:
/// 1. Exit node that requires PIX on a Session sends a [`DepositDataRequest`](Self) to the pool.
/// 2. The pool generates [`PixDepositData`] and sends it to the Exit node via the [`DepositDataCreated`] channel.
/// 3. The Exit node sends the PIX request (`SsaRequest`) message to the Entry node (including the deposit data obtained
///    earlier).
/// 4. The Entry node raises the [`NewDepositAddress`](Self) as it receives the PIX request (`SsaRequest`) message.
/// 5. The Entry sends the PIX response (`SsaCommit`) message to the Exit node.
/// 6. Once `SsaCommit` is processed by the Exit, the Exit raises [`DepositAddressReceived`](Self)
/// 7. At some point when Exit successfully recovers a PIX SSA, the Exit raises [`PrivateKeyRecovered`](Self)
#[derive(Debug, Clone, strum::EnumDiscriminants, strum::EnumTryAs)]
#[strum_discriminants(name(PixEventDiscriminant), derive(Hash))]
pub enum PixEvent {
    /// A new deposit address was generated at the Entry node.
    ///
    /// This notifies the Entry node to make a deposit to the address with the given quota per SSA in bytes.
    ///
    /// Generated on the Entry node only.
    NewDepositAddress(PixNewDepositAddress),
    /// The Exit has requested deposit data for the given deposit address IDs.
    ///
    /// This is typically done before the Exit requests commitments from the Entry node.
    ///
    /// Generated on the Exit node only.
    DepositDataRequest(PixDepositDataRequest),
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
}
