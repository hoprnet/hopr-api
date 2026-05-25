//! Session client operations for establishing HOPR sessions.
//!
//! Gated behind the `node-session-client` feature.
//! Explicit-path session types are additionally gated behind
//! `node-session-client-explicit-path`.

use futures::io::{AsyncRead, AsyncWrite};

use crate::Address;
use crate::types::internal::protocol::HoprPseudonym;
use crate::types::internal::routing::RoutingOptions;
use crate::types::primitive::bounded::BoundedSize;

/// Trait for establishing HOPR sessions to remote destinations.
///
/// The concrete session, configurator, target, and config types are defined
/// by the implementor (typically hopr-lib), keeping transport-level types
/// out of the API crate.
#[async_trait::async_trait]
#[auto_impl::auto_impl(&, Arc)]
pub trait HoprSessionClientOperations: Send + Sync {
    /// An established session implementing async read/write.
    type Session: AsyncRead + AsyncWrite + Send + Unpin;
    /// Handle for controlling a session after creation (e.g. keep-alive, SURB config).
    type SessionConfigurator: Send;
    /// Describes the remote service to connect to.
    type Target: Send;
    /// Configuration for the session (routing, capabilities, etc.).
    type Config: Send;
    /// Error type for session operations.
    type Error: std::error::Error + Send + Sync + 'static;

    /// Establishes a new session to the given `destination` via the HOPR network.
    ///
    /// Returns the session (implementing [`AsyncRead`] + [`AsyncWrite`]) and a configurator
    /// for controlling the session after creation.
    ///
    /// Implementations may retry automatically on failure.
    async fn connect_to(
        &self,
        destination: Address,
        target: Self::Target,
        config: Self::Config,
    ) -> Result<(Self::Session, Self::SessionConfigurator), Self::Error>;
}

// ---------------------------------------------------------------------------
// Session capability types
// ---------------------------------------------------------------------------

flagset::flags! {
    /// Individual capabilities of a HOPR session.
    ///
    /// Used to negotiate the protocol features enabled for a session between
    /// client and server. Represented as a `u8` bitflag.
    #[repr(u8)]
    #[derive(PartialOrd, Ord, strum::EnumString, strum::Display)]
    #[cfg_attr(feature = "serde", derive(serde_repr::Serialize_repr, serde_repr::Deserialize_repr))]
    pub enum Capability : u8 {
        /// Frame segmentation.
        Segmentation = 0b0000_1000,
        /// Frame retransmission (ACK-based). Implies [`Segmentation`].
        RetransmissionAck = 0b0000_1100,
        /// Frame retransmission (NACK-based). Implies [`Segmentation`].
        RetransmissionNack = 0b0000_1010,
        /// Disable packet buffering. Implies [`Segmentation`].
        NoDelay = 0b0000_1001,
        /// Disable SURB-based egress rate control (applies to the Exit node).
        NoRateControl = 0b0001_0000,
    }
}

/// Set of session [`Capability`] flags.
pub type Capabilities = flagset::FlagSet<Capability>;

// ---------------------------------------------------------------------------
// SURB balancer configuration
// ---------------------------------------------------------------------------

/// Configuration for the SURB balancer.
///
/// Controls how the session manages the SURB (Single Use Reply Block) buffer
/// used for return-path packet forwarding.
#[derive(Clone, Copy, Debug, PartialEq, smart_default::SmartDefault)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct SurbBalancerConfig {
    /// Target number of SURBs to keep buffered at all times. Default: 7000.
    #[default(7_000)]
    pub target_surb_buffer_size: u64,
    /// Maximum SURB outflow (consumption or production) per second. Default: 5000.
    #[default(5_000)]
    pub max_surbs_per_sec: u64,
    /// Optional SURB buffer decay: `(window_duration, fraction_to_discard)`.
    ///
    /// Default: discard 5% of the target buffer every 60 seconds.
    #[default(_code = "Some((std::time::Duration::from_secs(60), 0.05))")]
    pub surb_decay: Option<(std::time::Duration, f64)>,
}

// ---------------------------------------------------------------------------
// Hop-count routing
// ---------------------------------------------------------------------------

/// Public routing configuration using hop-count-based routing.
///
/// The hop count is bounded by [`HopRouting::MAX_HOPS`]. For explicit
/// intermediate-path routing see [`HoprSessionClientExplicitPathConfig`]
/// (requires feature `node-session-client-explicit-path`).
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, smart_default::SmartDefault)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct HopRouting(#[default(BoundedSize::MIN)] BoundedSize<{ RoutingOptions::MAX_INTERMEDIATE_HOPS }>);

impl HopRouting {
    /// Maximum number of intermediate hops supported.
    pub const MAX_HOPS: usize = RoutingOptions::MAX_INTERMEDIATE_HOPS;

    /// Returns the configured hop count.
    pub fn hop_count(self) -> usize {
        self.0.into()
    }
}

impl TryFrom<usize> for HopRouting {
    type Error = crate::types::primitive::errors::GeneralError;

    fn try_from(value: usize) -> Result<Self, Self::Error> {
        Ok(Self(value.try_into()?))
    }
}

impl From<HopRouting> for RoutingOptions {
    fn from(value: HopRouting) -> Self {
        Self::Hops(value.0)
    }
}

impl std::fmt::Display for HopRouting {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}-hop routing", self.hop_count())
    }
}

// ---------------------------------------------------------------------------
// Session client configuration
// ---------------------------------------------------------------------------

/// Configuration for [`HoprSessionClientOperations::connect_to`].
///
/// Specifies routing, capabilities, and SURB management for a new session.
/// Uses hop-count routing; for explicit intermediate-path routing see
/// [`HoprSessionClientExplicitPathConfig`].
#[derive(Debug, Clone, PartialEq, smart_default::SmartDefault)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct HoprSessionClientConfig {
    /// Forward-path routing (client → server).
    pub forward_path: HopRouting,
    /// Return-path routing (server → client).
    pub return_path: HopRouting,
    /// Requested session capabilities.
    #[default(_code = "Capability::Segmentation.into()")]
    pub capabilities: Capabilities,
    /// Optional pseudonym for the session (primarily for testing).
    #[default(None)]
    pub pseudonym: Option<HoprPseudonym>,
    /// SURB balancer configuration. `None` disables automatic SURB management.
    #[default(Some(SurbBalancerConfig::default()))]
    pub surb_management: Option<SurbBalancerConfig>,
    /// If `true`, always send the maximum possible number of SURBs with each
    /// data packet. Increases CPU usage; useful for highly asymmetric traffic.
    #[default(false)]
    pub always_max_out_surbs: bool,
}

// ---------------------------------------------------------------------------
// Explicit-path session types (deprecated, gated on sub-feature)
// ---------------------------------------------------------------------------

/// Configuration for explicit intermediate-path routing.
///
/// Deprecated: prefer hop-count routing via [`HoprSessionClientConfig`].
/// Kept for callers that still require explicit-path routing; enable via
/// the `node-session-client-explicit-path` feature.
#[cfg(feature = "node-session-client-explicit-path")]
#[deprecated(note = "prefer hop-count routing via HoprSessionClientConfig")]
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct HoprSessionClientExplicitPathConfig {
    /// Ordered list of intermediate nodes for the forward path.
    pub forward_path: Vec<crate::types::internal::NodeId>,
    /// Ordered list of intermediate nodes for the return path.
    pub return_path: Vec<crate::types::internal::NodeId>,
    /// Requested session capabilities.
    pub capabilities: Capabilities,
    /// Optional pseudonym for the session (primarily for testing).
    pub pseudonym: Option<HoprPseudonym>,
    /// SURB balancer configuration. `None` disables automatic SURB management.
    pub surb_management: Option<SurbBalancerConfig>,
    /// If `true`, always send the maximum possible number of SURBs with each
    /// data packet.
    pub always_max_out_surbs: bool,
}

#[cfg(feature = "node-session-client-explicit-path")]
#[allow(deprecated)]
impl Default for HoprSessionClientExplicitPathConfig {
    fn default() -> Self {
        Self {
            forward_path: Vec::new(),
            return_path: Vec::new(),
            capabilities: Capability::Segmentation.into(),
            pseudonym: None,
            surb_management: Some(SurbBalancerConfig::default()),
            always_max_out_surbs: false,
        }
    }
}

/// Trait mirroring [`HoprSessionClientOperations`] for explicit-path routing.
///
/// Deprecated: prefer hop-count routing via [`HoprSessionClientOperations`].
/// Kept for callers that still require explicit-path routing; enable via
/// the `node-session-client-explicit-path` feature.
#[cfg(feature = "node-session-client-explicit-path")]
#[async_trait::async_trait]
#[auto_impl::auto_impl(&, Arc)]
pub trait HoprExplicitPathSessionClient: Send + Sync {
    /// An established session implementing async read/write.
    type Session: AsyncRead + AsyncWrite + Send + Unpin;
    /// Handle for controlling a session after creation.
    type SessionConfigurator: Send;
    /// Describes the remote service to connect to.
    type Target: Send;
    /// Configuration for the explicit-path session.
    type Config: Send;
    /// Error type for session operations.
    type Error: std::error::Error + Send + Sync + 'static;

    /// Establishes a new session using an explicit intermediate-node path.
    async fn connect_to_using_explicit_path(
        &self,
        destination: Address,
        target: Self::Target,
        config: Self::Config,
    ) -> Result<(Self::Session, Self::SessionConfigurator), Self::Error>;
}
