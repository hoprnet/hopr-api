use hopr_types::internal::routing::PathId;

use super::{MeasurablePath, MeasurablePeer};
use crate::graph::{MeasurableEdge, MeasurableNode};

/// The result of a transport-level probe over a transport path segment.
///
/// Contains the measured latency on success, or a unit error on failure.
pub type EdgeTransportMeasurement = std::result::Result<std::time::Duration, ()>;

/// Payment channel balance, in the chain's base currency units.
///
/// Not a ticket count: that divides by the network-wide ticket face value, which changes over time
/// and would stale every edge at once. Producers push the face value separately; consumers supply
/// it at decision time. See [`EdgeValueFn`](super::function::EdgeValueFn).
///
/// `Some(0)` is an OPEN channel with nothing left to spend; `None` is unknown.
pub type Balance = hopr_types::primitive::primitives::U256;

/// Represents the different kinds of observations that can be recorded for a graph edge.
#[derive(Debug)]
pub enum EdgeWeightType {
    /// A direct transport measurement between this and another adjacent peer.
    Immediate(EdgeTransportMeasurement),
    /// A transport measurement relayed through an intermediate peer.
    Intermediate(EdgeTransportMeasurement),
    /// An update to the payment channel balance along this edge.
    Balance(Option<Balance>),
    /// An update to the physical connectivity status of this edge.
    Connected(bool),
    /// An update to the immediate hop protocol conformance metrics (messages sent / acks received).
    ImmediateProtocolConformance {
        /// Total number of packets sent to the immediate peer.
        num_packets: u64,
        /// Total number of acknowledgments received from the immediate peer.
        num_acks: u64,
    },
    /// SURB round-trips expected across this edge over a reporting interval, and how many of them
    /// were observed to complete.
    ///
    /// Deliberately not an [`EdgeTransportMeasurement`]: a round-trip proves the whole loop was
    /// traversed but carries no per-edge latency, so recording it as a transport measurement would
    /// mean inventing a duration and feeding it to the latency average — corrupting a figure that
    /// is otherwise measured directly.
    ///
    /// Counts rather than single events, for the same reason
    /// [`Self::ImmediateProtocolConformance`] carries counts: a SURB is minted far too often to
    /// take a graph write lock per occurrence.
    SurbRoundTrips {
        /// SURBs minted across this edge during the interval.
        expected: u64,
        /// How many of them had their reply arrive.
        observed: u64,
    },
}

/// Trait for recording new observations onto a graph edge.
pub trait EdgeObservableWrite {
    /// Records a new measurement or status update for this edge.
    fn record(&mut self, measurement: EdgeWeightType);
}

/// Trait for reading network-level properties of an edge.
pub trait EdgeNetworkObservableRead {
    /// Whether this edge represents also an existing physical connection between the peers.
    ///
    /// This is obviously settable only between the emitter of the measurement (this node) and
    /// arbitrary other node in the graph, but could be used for optimizations and path planning.
    ///
    /// `None` when never observed — not the same as observing there is none. Treating "unchecked"
    /// as "down" excludes the edge, which stops it being probed, which keeps it unchecked.
    fn is_connected(&self) -> Option<bool>;
}

/// Trait for reading HOPR protocol-level properties of an edge.
pub trait EdgeProtocolObservable {
    /// Remaining balance of the channel backing this path segment, in base currency units.
    ///
    /// See [`Balance`] for why this is not pre-divided into a ticket count.
    fn balance(&self) -> Option<Balance>;
}

/// Trait for reading immediate hop protocol conformance metrics.
///
/// Tracks point-to-point message acknowledgment behavior between directly
/// connected peers. The ack rate can be used by cost functions to detect
/// adversarial nodes that drop or fail to acknowledge messages.
pub trait EdgeImmediateProtocolObservable {
    /// The ratio of acknowledged messages to sent messages for this immediate edge.
    ///
    /// Returns `None` when insufficient messages have been sent to compute
    /// a meaningful ratio. Returns `Some(rate)` in the range \[0.0, 1.0\]
    /// where 1.0 means all messages were acknowledged.
    fn ack_rate(&self) -> Option<f64>;
}

/// Trait for reading aggregated quality-of-service observations from a graph edge.
pub trait EdgeObservableRead {
    /// Measurement type for direct (1-hop) probes, including network connectivity and protocol conformance info.
    type ImmediateMeasurement: EdgeLinkObservable + EdgeNetworkObservableRead + EdgeImmediateProtocolObservable + Send;
    /// Measurement type for relayed probes through an intermediate, including channel capacity.
    type IntermediateMeasurement: EdgeLinkObservable + EdgeProtocolObservable + Send;

    /// The timestamp of the last update.
    fn last_update(&self) -> std::time::Duration;

    /// Transport level measurements between this node and any other node in the network.
    fn immediate_qos(&self) -> Option<&Self::ImmediateMeasurement>;

    /// Transport level measurements performed in a transparent mode using looping measurements.
    fn intermediate_qos(&self) -> Option<&Self::IntermediateMeasurement>;

    /// Score in `[0.0, 1.0]`; higher is better. `None` when neither stream has observations.
    ///
    /// Per RFC-0014 §4.2 the streams combine as `(imm + inter) / 2` only when both are present,
    /// else the single present one. "Present" means *has observations*, not *allocated* — a
    /// capacity or connectivity update creates a stream without recording a probe, and averaging
    /// against that empty stream halves every edge only one stream can observe.
    fn score(&self) -> Option<f64>;
}

/// Combined trait for full read/write access to edge observations.
///
/// Automatically implemented for any type that implements both [`EdgeObservableRead`]
/// and [`EdgeObservableWrite`].
pub trait EdgeObservable: EdgeObservableRead + EdgeObservableWrite {}

impl<T: EdgeObservableWrite + EdgeObservableRead> EdgeObservable for T {}

/// Trait for recording and querying transport-level link quality metrics for a transport link.
///
/// Accessors report `None` for "not measured" rather than a neutral number, because the neutral
/// number is not neutral: a rate of `0.0` is also what a wholly failing stream reports. Selection
/// needs the difference — unmeasured earns an exploration penalty, measured-dead is starved.
///
/// How much evidence counts as measured is each signal's own business: a probe stream may report
/// from its first outcome, an acknowledgement rate may wait for a minimum volume, a windowed signal
/// may require a populated window. An aggregate is present when *any* constituent is.
pub trait EdgeLinkObservable {
    /// Records a new result of the probe over this path segment.
    fn record(&mut self, measurement: EdgeTransportMeasurement);

    /// Returns average latency observed for the measured peer.
    fn average_latency(&self) -> Option<std::time::Duration>;

    /// A value representing the average success rate of probes.
    ///
    /// It is from the range [0.0, 1.0]. The higher the value, the better the score.
    ///
    /// `None` when nothing has been recorded: an all-failed stream holds `0.0`, and so does an
    /// unprobed one.
    fn average_probe_rate(&self) -> Option<f64>;

    /// Whether any outcome has been recorded, successful or failed.
    ///
    /// Derived from [`average_probe_rate`](Self::average_probe_rate) so the two cannot disagree.
    fn has_observations(&self) -> bool {
        self.average_probe_rate().is_some()
    }

    /// Score in `[0.0, 1.0]`; higher is better.
    ///
    /// `None` = no observations, `Some(0.0)` = measured and unusable. Cost functions need the
    /// distinction: unobserved edges get `edge_penalty` to stay discoverable, measured-dead ones
    /// are starved. Collapsing both to `0.0` ranks a never-working relay above a partly-working
    /// one.
    fn score(&self) -> Option<f64>;
}

/// Lifecycle events observed for a node in the network.
#[derive(Debug, Clone)]
pub enum NodeObservation<T> {
    /// The node was discovered in the network.
    Discovered(T),
    /// A direct connection to the node was established.
    Connected(T),
    /// The direct connection to the node was lost.
    Disconnected(T),
}

/// Trait for recording node lifecycle observations into the graph.
pub trait NodeObservable {
    /// The node identifier type that can be measured as a peer.
    type Node: MeasurablePeer + Send;

    /// Record a new observation for the given node.
    fn record_node(&mut self, observation: NodeObservation<Self::Node>);
}

/// A trait specifying read-only graph view functionality.
///
/// Provides methods to inspect the graph topology: node membership, node count,
/// edge existence, and edge observation retrieval.
#[auto_impl::auto_impl(&, Box, Arc)]
pub trait NetworkGraphView {
    /// The concrete type of observations for peers.
    type Observed: EdgeObservable + Send;
    /// The identifier type used to reference nodes in the graph.
    type NodeId: Send;

    /// The current face value of a single-hop ticket, in base currency units.
    ///
    /// Network-wide and identical for every channel, but time-varying: producers recompute it from
    /// the ticket price and winning probability whenever either changes and push it in via
    /// [`NetworkGraphUpdate::set_ticket_face_value`]. It lives here rather than on each edge so a
    /// price change costs one write instead of staling the whole graph.
    ///
    /// `None` when no value has been pushed yet, in which case path selection falls back to
    /// [`default_ticket_face_value`](super::function::default_ticket_face_value).
    fn ticket_face_value(&self) -> Option<Balance>;

    /// Returns the number of nodes in the graph.
    fn node_count(&self) -> usize;

    /// Checks whether the graph contains the given node.
    fn contains_node(&self, key: &Self::NodeId) -> bool;

    /// Returns a stream of all known nodes in the network graph.
    fn nodes(&self) -> futures::stream::BoxStream<'static, Self::NodeId>;

    /// Checks whether a directed edge exists between two nodes.
    ///
    /// The default implementation delegates to [`edge`](Self::edge).
    fn has_edge(&self, src: &Self::NodeId, dest: &Self::NodeId) -> bool {
        self.edge(src, dest).is_some()
    }

    /// Returns the weight represented by the observations for the edge between the
    /// given source and destination, if available.
    fn edge(&self, src: &Self::NodeId, dest: &Self::NodeId) -> Option<Self::Observed>;

    /// Returns the self-identity node of this graph.
    fn identity(&self) -> &Self::NodeId;

    /// Resolves a node to the slot it occupies in a [`PathId`], if the graph knows it.
    ///
    /// A [`PathId`] identifies a path by position rather than by key, so anything reporting a path
    /// it did not obtain from [`simple_paths`](GraphPathSelection::simple_paths) -- notably SURB
    /// round-trips, which know their route as public keys -- needs this to speak the same language.
    ///
    /// The mapping is only valid for as long as the node stays in the graph; a caller that holds an
    /// id across a removal may find it refers to a different node, so ids are best resolved and
    /// consumed promptly.
    fn path_slot(&self, key: &Self::NodeId) -> Option<u64>;
}

/// A trait for mutating the graph topology.
///
/// Provides methods to add/remove nodes and add edges.
#[auto_impl::auto_impl(&, Box, Arc)]
pub trait NetworkGraphWrite {
    /// The error type returned by fallible write operations.
    type Error;
    /// The concrete type of observations for peers.
    type Observed: EdgeObservable + Send;
    /// The identifier type used to reference nodes in the graph.
    type NodeId: Send;

    /// Adds a node to the graph if it does not already exist.
    fn add_node(&self, key: Self::NodeId);

    /// Removes a node and all its associated edges from the graph.
    fn remove_node(&self, key: &Self::NodeId);

    /// Adds a directed edge between two existing nodes with default observations.
    ///
    /// Returns an error if either node is not present in the graph.
    fn add_edge(&self, src: &Self::NodeId, dest: &Self::NodeId) -> Result<(), Self::Error>;

    /// Removes a directed edge between two nodes.
    ///
    /// If the edge does not exist, this operation has no effect.
    fn remove_edge(&self, src: &Self::NodeId, dest: &Self::NodeId);

    /// Updates an existing edge or inserts a new edge between two nodes.
    ///
    /// If the nodes do not exist, they are inserted into the graph.
    ///
    /// The provided closure `f` is applied to modify the edge's observations.
    /// If the edge already exists, its observations are updated.
    /// If the edge does not exist, it is created and the closure is applied.
    fn upsert_edge<F>(&self, src: &Self::NodeId, dest: &Self::NodeId, f: F)
    where
        F: FnOnce(&mut Self::Observed);
}

/// A trait for recording observed measurement updates to graph edges and nodes.
#[auto_impl::auto_impl(&, Box, Arc)]
pub trait NetworkGraphUpdate {
    /// Records a newly computed single-hop ticket face value.
    ///
    /// Call whenever the ticket price or winning probability changes. Cheap by construction: no
    /// edge stores the face value, so none has to be revisited — which is why edges carry a raw
    /// balance instead of a pre-divided ticket count.
    fn set_ticket_face_value(&self, ticket_face_value: Balance);

    /// Records an edge measurement derived from network telemetry.
    fn record_edge<N, P>(&self, update: MeasurableEdge<N, P>)
    where
        N: MeasurablePeer + Clone + Send + Sync + 'static,
        P: MeasurablePath + Clone + Send + Sync + 'static;

    /// Records a node observation derived from network telemetry.
    fn record_node<N>(&self, update: N)
    where
        N: MeasurableNode + Clone + Send + Sync + 'static;
}

/// A fold-like value function for graph traversal path scoring.
///
/// A **value function** produces scores where **higher is better** (to be maximized),
/// as opposed to a **cost function** where lower is better (to be minimized).
/// The accumulated value is folded over each edge in the path: edges that improve
/// the path increase the value, while poor-quality edges decrease it.
/// Paths whose value drops below [`min_value`](Self::min_value) are discarded.
#[allow(clippy::type_complexity)]
pub trait ValueFn {
    type Weight: EdgeObservableRead + Send;
    type Value: Clone + PartialOrd + Send + Sync;

    /// The initial value that will be modified by the value function.
    fn initial_value(&self) -> Self::Value;

    /// The minimum value, below which the value function will force discard upon traversal.
    fn min_value(&self) -> Option<Self::Value>;

    /// The value function accepting graph properties to establish the final value.
    fn into_value_fn(self) -> std::sync::Arc<dyn Fn(Self::Value, &Self::Weight, usize) -> Self::Value + Send + Sync>;
}

/// A trait specifying the graph traversal functionality.
///
/// Provides methods for finding simple paths between nodes in the network graph.
#[auto_impl::auto_impl(&, Box, Arc)]
pub trait NetworkGraphTraverse {
    /// The identifier type used to reference nodes in the graph.
    type NodeId: Send + Sync;
    /// The concrete edge observation type used by value functions during traversal.
    type Observed: EdgeObservableRead + Send;

    /// Returns a list of routes from the source to the destination with the specified length
    /// at the time of calling.
    ///
    /// The length argument specifies the number of edges in the graph, over which the path should
    /// be formed, i.e. source -> intermediate -> destination is 2 edges.
    ///
    /// The take count argument should be set in case the graph is expected to be large enough
    /// to be traversed slowly.
    fn simple_paths<V: ValueFn<Weight = Self::Observed>>(
        &self,
        source: &Self::NodeId,
        destination: &Self::NodeId,
        length: usize,
        take_count: Option<usize>,
        value_fn: V,
    ) -> Vec<(Vec<Self::NodeId>, PathId, V::Value)>;

    /// Returns a list of routes from the source to **any** reachable node with the
    /// specified edge length.
    ///
    /// Unlike [`simple_paths`](Self::simple_paths), this method does not target a
    /// specific destination. All graph nodes (except the source) are eligible
    /// destinations. The caller can further filter the results.
    fn simple_paths_from<V: ValueFn<Weight = Self::Observed>>(
        &self,
        source: &Self::NodeId,
        length: usize,
        take_count: Option<usize>,
        value_fn: V,
    ) -> Vec<(Vec<Self::NodeId>, PathId, V::Value)>;

    /// Return a list of nodes with a full loopback from myself to myself.
    ///
    /// The length argument specifies the number of edges in the graph, over which the path should
    /// be formed, i.e. source -> intermediate -> destination is 2 edges.
    ///
    /// At least length 2 is required to provide a path through a single relay.
    fn simple_loopback_to_self(&self, length: usize, take_count: Option<usize>) -> Vec<(Vec<Self::NodeId>, PathId)>;
}

/// Topology enumeration — which edges exist and which are reachable.
///
/// Unlike [`NetworkGraphTraverse`] (path planning), this trait answers
/// "what is connected to what" without computing routes.
#[auto_impl::auto_impl(&, Box, Arc)]
pub trait NetworkGraphConnectivity {
    /// The identifier type used to reference nodes in the graph.
    type NodeId: Send + Sync;
    /// The concrete edge observation type.
    type Observed: EdgeObservableRead + Send;

    /// Returns all edges in the graph as `(source, destination, observations)` triples.
    ///
    /// Only nodes that participate in at least one edge appear in the result.
    /// Isolated nodes (no incoming or outgoing edges) are omitted.
    fn connected_edges(&self) -> Vec<(Self::NodeId, Self::NodeId, Self::Observed)>;

    /// Returns edges reachable from the graph's
    /// [`identity`](NetworkGraphView::identity) node via directed traversal.
    ///
    /// Only edges where both the source and destination are reachable are
    /// included. Disconnected subgraphs that cannot be routed through are
    /// excluded.
    fn reachable_edges(&self) -> Vec<(Self::NodeId, Self::NodeId, Self::Observed)>;
}
