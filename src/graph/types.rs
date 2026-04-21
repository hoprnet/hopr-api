use hopr_types::crypto::types::OffchainPublicKey;

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
/// The capacity can be either `None` or a `Some(u128)` value.
/// * `None` - the capacity of the channel disappeared
/// * `Some(u128)` - the capacity was updated
#[derive(Debug, Copy, Clone)]
pub struct EdgeCapacityUpdate {
    /// Updated channel capacity; `None` means capacity is no longer known.
    pub capacity: Option<u128>,
    /// Source node of the edge.
    pub src: OffchainPublicKey,
    /// Destination node of the edge.
    pub dest: OffchainPublicKey,
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
    /// Capacity update for a specific directed edge.
    Capacity(Box<EdgeCapacityUpdate>),
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
