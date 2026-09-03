//! Session server trait for processing incoming HOPR sessions.
//!
//! Gated behind the `node-session-server` feature.

use hopr_types::network::{SessionId, SessionTarget};

/// An incoming HOPR session to be processed by a [`HoprSessionServer`].
///
/// Generic over the concrete session byte-stream `S` (supplied by the implementor,
/// typically hopr-lib), keeping transport-level types out of this crate. Only the
/// plain [`SessionId`] and [`SessionTarget`] descriptors are named here.
#[derive(Debug)]
pub struct IncomingSession<S> {
    /// Identifier of the incoming session.
    pub id: SessionId,
    /// The session byte-stream carrying the forwarded data.
    pub session: S,
    /// Describes where data received over the session should be forwarded.
    pub target: SessionTarget,
}

/// What a [`HoprSessionServer`] is told about a session it is asked to
/// [admit](HoprSessionServer::admit).
///
/// Describes a session that does not exist yet: it names the peer's intent, not a byte-stream. The
/// [`SessionTarget`] arrives exactly as the initiating peer sent it — in particular a
/// [`SealedHost`](hopr_types::network::SealedHost) is still sealed, since the key that opens it is
/// the server's, not the transport's. That asymmetry is the reason this hook exists rather than the
/// transport deciding for itself.
///
/// `#[non_exhaustive]`: build one with [`new`](Self::new), so that later context can be added
/// without breaking implementors.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct SessionAdmissionRequest {
    /// Identifier the session will have if it is admitted.
    pub session_id: SessionId,
    /// Where the peer is asking for data to be forwarded.
    pub target: SessionTarget,
    /// The session capabilities the peer asked for, as the raw wire bitfield.
    ///
    /// Undecoded because the capability set is defined by the transport rather than here, and
    /// mirroring it would be two definitions to keep in step. A server that admits on a capability
    /// must name the bit it means; the transport's own capability type documents the values.
    pub capabilities: u8,
    /// What the peer offered by way of incentivization, or [`None`] if it offered none.
    ///
    /// Already decoded and range-checked by the transport, which is the only side that can read the
    /// wire encoding. A server that only prices by target can ignore it; one that wants to answer
    /// *relative* to the offer — matching it, or holding a dimension to a floor the quota alone
    /// does not express — needs it, and cannot recover it from anything else in this request.
    pub offered: Option<OfferedIncentivization>,
}

impl SessionAdmissionRequest {
    /// Describes a prospective session by the identifier it would take, the target it named, and
    /// the capabilities it asked for.
    ///
    /// The incentivization offer is attached with [`with_offer`](Self::with_offer); a request
    /// without one describes a peer that offered none.
    pub fn new(session_id: SessionId, target: SessionTarget, capabilities: u8) -> Self {
        Self {
            session_id,
            target,
            capabilities,
            offered: None,
        }
    }

    /// Records what the peer offered by way of incentivization.
    pub fn with_offer(mut self, offered: OfferedIncentivization) -> Self {
        self.offered = Some(offered);
        self
    }
}

/// The incentivization a peer offered, as the transport decoded it.
///
/// Plain numbers rather than the transport's own parameter type, which is defined in a crate this
/// one cannot depend on. The dimensions travel beside the quota because they are not recoverable
/// from it: the quota is their product, and the same product admits many splits — so a server that
/// cares how the peer arrived at a quota, rather than only what it totals, can only see that here.
///
/// `#[non_exhaustive]`: build one with [`new`](Self::new).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct OfferedIncentivization {
    /// Independent parts the peer will split each aggregate into.
    pub parts_per_ssa: u16,
    /// Shares of one part needed to reconstruct it.
    pub shares_per_part: u8,
    /// Shares emitted per part beyond that threshold, as insurance against return-path loss.
    pub surplus_shares: u8,
    /// Bytes one aggregate is worth: every emitted share carries one payload, surplus included.
    ///
    /// Computed by the transport, which owns the payload size this is denominated in.
    pub quota_per_ssa: u64,
}

impl OfferedIncentivization {
    /// Records an offer of `parts_per_ssa` parts, each reconstructible from `shares_per_part`
    /// shares and emitting `surplus_shares` more, together worth `quota_per_ssa` bytes.
    pub fn new(parts_per_ssa: u16, shares_per_part: u8, surplus_shares: u8, quota_per_ssa: u64) -> Self {
        Self {
            parts_per_ssa,
            shares_per_part,
            surplus_shares,
            quota_per_ssa,
        }
    }
}

/// The terms a [`HoprSessionServer`] admits a session on.
///
/// Every field is an *override* of the node's configured value, and [`None`] — which is what
/// [`Default`] gives on every field — leaves that value alone. A server with nothing to say about a
/// session therefore returns `SessionAdmissionDecision::default()` and gets the node's own policy,
/// which is what the default [`admit`](HoprSessionServer::admit) does.
///
/// `#[non_exhaustive]`: start from [`Default`] and use the `with_*` methods, so that terms beyond
/// today's incentivization ones can be added without breaking implementors.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct SessionAdmissionDecision {
    /// Whether this session must carry incentivization (PIX), overriding the node's setting.
    ///
    /// Overrides in both directions: `Some(false)` serves a target for free on a node that
    /// otherwise demands payment, `Some(true)` demands payment on a node that otherwise does not.
    pub enforce_pix: Option<bool>,
    /// Acceptable incentivization quota for this session, in bytes.
    ///
    /// **Narrowing only.** The caller intersects this with the node's configured range rather than
    /// replacing it: that range is validated at startup against the deadlines and reconstructor
    /// memory it implies, and sizes preallocated buffers, so a session may ask for less of the
    /// envelope but never for more than the node was configured to honour. An empty intersection
    /// admits nothing, which is the correct reading of two policies that do not overlap.
    pub pix_quota_range: Option<core::ops::RangeInclusive<u64>>,
}

impl SessionAdmissionDecision {
    /// Requires, or waives, incentivization for this session.
    pub fn with_enforce_pix(mut self, enforce_pix: bool) -> Self {
        self.enforce_pix = Some(enforce_pix);
        self
    }

    /// Narrows the acceptable incentivization quota for this session, in bytes.
    pub fn with_pix_quota_range(mut self, quota_range: core::ops::RangeInclusive<u64>) -> Self {
        self.pix_quota_range = Some(quota_range);
        self
    }
}

/// Trait for processing incoming HOPR sessions on exit nodes.
///
/// The concrete session type is defined by the implementor (typically hopr-lib),
/// keeping transport-level types out of the API crate.
///
/// Nodes that do not run a session server simply omit calling `with_session_server`.
#[async_trait::async_trait]
#[auto_impl::auto_impl(Arc)]
pub trait HoprSessionServer {
    /// An incoming session to be processed.
    type Session: Send;
    /// Error type for session processing.
    type Error: std::error::Error + Send + Sync + 'static;

    /// Decides whether, and on what terms, a session is admitted.
    ///
    /// Called on the exit node while the peer's session request is still being negotiated, before
    /// any state is allocated for it, and answered by the transport within a timeout of its own
    /// choosing. Returning [`Err`] refuses the session; the transport tells the peer so and nothing
    /// is established. [`process`](Self::process) is only ever called for a session this admitted.
    ///
    /// Implementations should be prompt and must not block: a server slower than the transport's
    /// timeout refuses every session it is asked about. Anything expensive — name resolution,
    /// dialling the target — belongs in [`process`](Self::process), which runs once the session
    /// exists and can fail without costing the peer a negotiation round trip.
    ///
    /// The default admits every session on the node's own configured terms, which is the behaviour
    /// of a server that does not distinguish between targets.
    async fn admit(&self, request: SessionAdmissionRequest) -> Result<SessionAdmissionDecision, Self::Error> {
        let _ = request;
        Ok(SessionAdmissionDecision::default())
    }

    /// Fully process a single incoming HOPR session.
    async fn process(&self, session: Self::Session) -> Result<(), Self::Error>;
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use anyhow::Context;
    use hopr_types::{
        crypto_random::Randomizable,
        network::{IpOrHost, SealedHost},
    };

    use super::*;

    #[derive(Debug, thiserror::Error)]
    #[error("no")]
    struct NeverError;

    /// A server that only knows how to process, exercising the defaulted `admit`.
    struct ProcessOnlyServer;

    #[async_trait::async_trait]
    impl HoprSessionServer for ProcessOnlyServer {
        type Error = NeverError;
        type Session = ();

        async fn process(&self, _session: ()) -> Result<(), NeverError> {
            Ok(())
        }
    }

    fn any_request() -> anyhow::Result<SessionAdmissionRequest> {
        Ok(SessionAdmissionRequest::new(
            SessionId::random(),
            SessionTarget::TcpStream(SealedHost::Plain(IpOrHost::from_str("127.0.0.1:80")?)),
            0,
        ))
    }

    #[tokio::test]
    async fn a_server_that_does_not_override_admit_imposes_no_terms() -> anyhow::Result<()> {
        let decision = ProcessOnlyServer.admit(any_request()?).await?;

        // Every term unset is what makes the default hook a no-op: the caller keeps its own policy.
        assert_eq!(decision, SessionAdmissionDecision::default());
        assert!(decision.enforce_pix.is_none());
        assert!(decision.pix_quota_range.is_none());

        Ok(())
    }

    /// A request without an offer is what a peer asking for no incentivization looks like, and the
    /// two are the same thing to a server — so the absence has to be representable, not implied.
    #[test]
    fn a_request_carries_an_offer_only_when_one_was_made() -> anyhow::Result<()> {
        let bare = any_request()?;
        assert!(bare.offered.is_none(), "no offer must read as no offer");

        let offered = any_request()?.with_offer(OfferedIncentivization::new(8192, 64, 16, 649_363_456));
        let offer = offered.offered.context("the offer must survive being attached")?;

        assert_eq!(offer.parts_per_ssa, 8192);
        assert_eq!(offer.shares_per_part, 64);
        assert_eq!(offer.surplus_shares, 16);
        assert_eq!(offer.quota_per_ssa, 649_363_456);

        Ok(())
    }

    /// The capability bits are the peer's, passed through undecoded.
    #[test]
    fn a_request_carries_the_capability_bits_verbatim() -> anyhow::Result<()> {
        let request = SessionAdmissionRequest::new(
            SessionId::random(),
            SessionTarget::TcpStream(SealedHost::Plain(IpOrHost::from_str("127.0.0.1:80")?)),
            0b0010_1000,
        );

        assert_eq!(request.capabilities, 0b0010_1000);
        Ok(())
    }

    #[test]
    fn each_term_set_leaves_the_others_untouched() {
        let enforce_only = SessionAdmissionDecision::default().with_enforce_pix(true);
        assert_eq!(enforce_only.enforce_pix, Some(true));
        assert!(enforce_only.pix_quota_range.is_none());

        let quota_only = SessionAdmissionDecision::default().with_pix_quota_range(1..=2);
        assert!(quota_only.enforce_pix.is_none());
        assert_eq!(quota_only.pix_quota_range, Some(1..=2));

        let both = SessionAdmissionDecision::default()
            .with_enforce_pix(false)
            .with_pix_quota_range(3..=4);
        assert_eq!(both.enforce_pix, Some(false));
        assert_eq!(both.pix_quota_range, Some(3..=4));
    }
}
