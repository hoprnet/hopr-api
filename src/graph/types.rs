use hopr_types::{crypto::types::OffchainPublicKey, internal::routing::PathId};

/// Error observed during the measurements updating the graph edges.
#[derive(thiserror::Error, Debug)]
pub enum NetworkGraphError<P>
where
    P: MeasurablePath,
{
    /// The immediate-neighbor probe did not complete before timeout.
    #[error("timed out for near neighbor probe '{0:?}'")]
    ProbeNeighborTimeout(Box<OffchainPublicKey>),

    /// The loopback probe did not complete before timeout.
    #[error("timed out for loopback probe")]
    ProbeLoopbackTimeout(P),
}

/// Marker trait for node identifiers that can be converted into an off-chain public key.
pub trait MeasurableNode: Into<OffchainPublicKey> {}

impl<T: Into<OffchainPublicKey>> MeasurableNode for T {}

/// Measurable neighbor peer attributes.
pub trait MeasurablePeer {
    /// Returns the measured peer public key.
    fn peer(&self) -> &OffchainPublicKey;
    /// Returns the measured round-trip time.
    fn rtt(&self) -> std::time::Duration;
}

/// Measurable path segment telemetry.
pub trait MeasurablePath {
    /// Returns the unique measurement identifier.
    fn id(&self) -> &[u8];
    /// Returns the serialized measured path.
    fn path(&self) -> &[u8];
    /// Returns the measurement timestamp in milliseconds since epoch.
    fn timestamp(&self) -> u128;
}

/// Update for the edge between src and dest.
///
/// * `None` - the balance is unknown, whether because the channel closed or because the indexer has not reported one
///   yet. The two are deliberately collapsed: nothing downstream may spend against either, and distinguishing them
///   would invite a consumer to drop an edge that is merely unseen.
/// * `Some(balance)` - the balance was updated
#[derive(Debug, Copy, Clone)]
pub struct EdgeBalanceUpdate {
    /// Updated channel balance in base currency units; `None` when no longer known.
    pub balance: Option<crate::graph::traits::Balance>,
    /// Source node of the edge.
    pub src: OffchainPublicKey,
    /// Destination node of the edge.
    pub dest: OffchainPublicKey,
}

/// The two legs a SURB round-trip traverses.
///
/// A SURB rides a forward path to reach the replier and carries a return path for the reply to come
/// back on, so one completed round-trip is evidence about **both** — which is why the pair is
/// reported together rather than as two unrelated observations.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub struct ForwardAndReturnPath {
    /// Path the SURB travelled to reach the replier.
    pub forward: PathId,
    /// Path encoded in the SURB for the reply to travel back on.
    pub reply: PathId,
}

/// SURB round-trip outcomes over one reporting interval, as seen by the node that minted them.
///
/// Unlike a probe this costs no extra traffic — it is a by-product of data a session was sending
/// anyway, so it accrues at data rates rather than at the probing interval. That is what lets a
/// failed relay be noticed in seconds, instead of after an average has moved behind a path cache.
///
/// **Counts, not single events.** A round-trip is minted per SURB and a packet carries several, so
/// reporting each one individually would take a graph write lock thousands of times a second on the
/// packet hot path. Senders accumulate cheaply and report periodically, exactly as
/// [`super::EdgeWeightType::ImmediateProtocolConformance`] does for packet/ack counts.
#[derive(Debug, Copy, Clone)]
pub struct SurbTelemetry {
    /// The legs these round-trips traversed.
    pub paths: ForwardAndReturnPath,
    /// When the interval was reported, in milliseconds since epoch.
    pub timestamp: u128,
    /// SURBs minted over these paths during the interval.
    pub expected: u64,
    /// How many of them had their reply arrive.
    pub observed: u64,
}

/// Edge measurements accepted for an edge in the graph.
#[derive(Debug)]
pub enum MeasurableEdge<N, P>
where
    N: MeasurablePeer + Clone,
    P: MeasurablePath + Clone,
{
    /// Probe outcome produced by cover-traffic or transport telemetry.
    Probe(std::result::Result<EdgeTransportTelemetry<N, P>, NetworkGraphError<P>>),
    /// Outcome of a SURB round-trip, reported by the node that minted it.
    Surb(SurbTelemetry),
    /// Balance update for a specific directed edge.
    Balance(Box<EdgeBalanceUpdate>),
    /// Connection-state change observed for a peer.
    ConnectionStatus {
        /// Peer whose connection state changed.
        peer: OffchainPublicKey,
        /// `true` when connected, `false` when disconnected.
        connected: bool,
    },
}

/// Enum representing different types of telemetry data used by the CT mechanism.
#[derive(Debug, Clone)]
pub enum EdgeTransportTelemetry<N, P>
where
    N: MeasurablePeer + Clone,
    P: MeasurablePath + Clone,
{
    /// Telemetry data looping the traffic through multiple peers back to self.
    ///
    /// Does not require a cooperating peer.
    Loopback(P),
    /// Immediate neighbor telemetry data.
    ///
    /// Assumes a cooperating immediate peer to receive responses for telemetry construction
    Neighbor(N),
}
