use std::sync::Arc;

use super::traits::{
    Balance, EdgeImmediateProtocolObservable, EdgeLinkObservable, EdgeNetworkObservableRead, EdgeObservableRead,
    EdgeProtocolObservable, ValueFn,
};

/// A shared value function accepting `(current_value, edge_weight, path_index) -> new_value`.
pub type BasicValueFn<C, W> = Arc<dyn Fn(C, &W, usize) -> C + Send + Sync>;

/// Floor applied to a measured-dead stream, i.e. one scoring `Some(0.0)`.
///
/// Kept strictly positive, not to outrank degraded edges — no fixed constant can, since scores decay
/// arbitrarily close to zero — but because zero costs harm twice: an all-zero candidate set yields no
/// route at all, and a non-positive cost is pruned. Probe generation shares these value functions, so
/// a pruned edge stops being probed and can never recover, which RFC-0010 §4.2.3 forbids.
const MEASURED_DEAD_FLOOR: f64 = 1e-9;

/// Multiple of the bare-minimum balance an edge must hold to be selected.
///
/// Requiring exactly the ticket value flaps: the balance is indexed from chain, so it lags
/// redemptions, and probabilistic wins make it fall in jumps rather than smoothly.
pub const MIN_BALANCE_HEADROOM: u64 = 2;

/// Face value assumed when a caller supplies none.
///
/// One base unit per single-hop ticket, i.e. the balance is taken to be counted in tickets already.
/// Callers that know the live value — recomputed from the ticket price and winning probability
/// whenever either changes — pass it explicitly; those that do not, such as probe path generation,
/// get a neutral comparison against the remaining hop count.
pub fn default_ticket_face_value() -> Balance {
    Balance::one()
}

/// Scales value by score: `None` (unobserved) applies `penalty`; `Some(s)` scales by `s`, floored at
/// [`MEASURED_DEAD_FLOOR`].
fn score_or_penalize(cost: f64, score: Option<f64>, penalty: f64) -> f64 {
    // Both branches are floored. `penalty` is caller-supplied and clamped to `[0.0, 1.0]`, so a
    // configured `0.0` would drive the cost to zero and prune the edge — the same outcome the floor
    // exists to prevent, reachable by configuration alone.
    let multiplier = match score {
        None => penalty,
        Some(score) => score,
    };
    cost * multiplier.max(MEASURED_DEAD_FLOOR)
}

/// Applies the ack rate as a value modifier for immediate edges.
///
/// When the ack rate is available and below `min_ack_rate`, the edge is rejected.
/// When available and above the threshold, the value is scaled by the ack rate.
/// When unavailable (insufficient data), the penalty multiplier is applied.
fn apply_ack_rate(ack_rate: Option<f64>, cost: f64, min_ack_rate: f64, penalty: f64) -> f64 {
    match ack_rate {
        Some(rate) if rate < min_ack_rate => -cost,
        Some(rate) => cost * rate,
        None => cost * penalty,
    }
}

/// Whether a channel's balance can fund the ticket an edge at this position must issue.
///
/// A relayer with `remaining_hops` hops after it issues a ticket worth that many times the face
/// value, plus [`MIN_BALANCE_HEADROOM`]. Face value is network-wide — identical for every channel —
/// but time-varying, so it is supplied per evaluation rather than stored on the edge; see
/// [`Balance`](super::traits::Balance).
///
/// `None` face value defaults to [`default_ticket_face_value`], which treats the balance as already
/// counted in single-hop tickets. A zero face value prices relaying at nothing, so any channel funds
/// any hop — but one must still exist, since the relayer issues a (zero-value) ticket on it. That is
/// a supported network-wide mode, not a producer error; it waives [`MIN_BALANCE_HEADROOM`] for every
/// edge at once, leaving the probe scores as the only thing separating relays.
///
/// `remaining_hops == 0` is the final hop: zero-value ticket, no channel needed. A `None` balance is
/// unknown, which is not evidence of sufficiency.
fn balance_suffices(balance: Option<Balance>, remaining_hops: usize, ticket_face_value: Option<Balance>) -> bool {
    if remaining_hops == 0 {
        return true;
    }

    let ticket_face_value = ticket_face_value.unwrap_or_else(default_ticket_face_value);
    if ticket_face_value.is_zero() {
        return balance.is_some();
    }

    balance.is_some_and(|balance| {
        Balance::from(remaining_hops as u64)
            .checked_mul(ticket_face_value)
            .and_then(|required| required.checked_mul(Balance::from(MIN_BALANCE_HEADROOM)))
            .is_some_and(|required| balance >= required)
    })
}

/// Rejects the edge unless its channel can fund this hop, else applies score-or-penalize.
///
/// A presence check is not enough: `Some(0)` is a real value for an OPEN but drained channel, whose
/// relayer cannot re-issue a ticket.
fn require_funding<W: EdgeObservableRead>(
    observation: &W,
    cost: f64,
    penalty: f64,
    remaining_hops: usize,
    ticket_face_value: Option<Balance>,
) -> f64 {
    if let Some(intermediate) = observation.intermediate_qos()
        && balance_suffices(intermediate.balance(), remaining_hops, ticket_face_value)
    {
        return score_or_penalize(cost, intermediate.score(), penalty);
    }

    -cost
}

/// A graph edge value function implementing a fold over path edges.
///
/// The `penalty` is a penalizing multiplier applied to edges that lack
/// probe-based quality observations (e.g. only on-chain balance or only
/// immediate connectivity data). It scales the accumulated value downward,
/// making unprobed edges less attractive than measured ones while still
/// allowing path discovery. A value of `1.0` means no penalty; lower
/// values (e.g. `0.5`) increasingly penalize unprobed edges.
///
/// Use one of the named constructors to create the appropriate variant:
/// - [`EdgeValueFn::forward`] — full graph traversal in the forward direction
/// - [`EdgeValueFn::returning`] — full graph traversal in the return direction
/// - [`EdgeValueFn::forward_without_self_loopback`] — simple forward paths without final loopback
pub struct EdgeValueFn<C, W> {
    initial: C,
    min: Option<C>,
    value_fn: BasicValueFn<C, W>,
}

impl<C: Clone, W> Clone for EdgeValueFn<C, W> {
    fn clone(&self) -> Self {
        Self {
            initial: self.initial.clone(),
            min: self.min.clone(),
            value_fn: Arc::clone(&self.value_fn),
        }
    }
}

impl<C, W> ValueFn for EdgeValueFn<C, W>
where
    C: Clone + PartialOrd + Send + Sync + 'static,
    W: EdgeObservableRead + Send + 'static,
{
    type Value = C;
    type Weight = W;

    fn initial_value(&self) -> Self::Value {
        self.initial.clone()
    }

    fn min_value(&self) -> Option<Self::Value> {
        self.min.clone()
    }

    fn into_value_fn(self) -> BasicValueFn<Self::Value, Self::Weight> {
        self.value_fn
    }
}

impl<W> EdgeValueFn<f64, W>
where
    W: EdgeObservableRead + Send + 'static,
{
    /// Build a forward HOPR value function for full graph traversals.
    ///
    /// `penalty` is clamped to `[0.0, 1.0]` — the penalizing multiplier applied
    /// to edges lacking probe-based quality observations.
    ///
    /// `min_ack_rate` is clamped to `[0.0, 1.0]` — the minimum acceptable message
    /// acknowledgment rate for the immediate peer. Edges with an ack rate below this
    /// threshold are rejected.
    ///
    /// - **First edge**: requires connectivity and a balance that funds the rest of the path; scores by the aggregate
    ///   of the immediate and intermediate observations, then applies the ack rate modifier.
    /// - **Last edge**: accepts an intermediate balance or immediate connectivity; penalizes when neither is available
    ///   (last hop is not monetized). When `length == 1` the single edge is both first and last; the ack rate modifier
    ///   is applied when immediate QoS data is available.
    /// - **Intermediate edges**: require a funding balance; penalize when unprobed.
    pub fn forward(
        length: std::num::NonZeroUsize,
        penalty: f64,
        min_ack_rate: f64,
        ticket_face_value: Option<Balance>,
    ) -> Self {
        let length = length.get();
        let penalty = penalty.clamp(0.0, 1.0);
        let min_ack_rate = min_ack_rate.clamp(0.0, 1.0);
        Self {
            initial: 1.0,
            min: Some(0.0),
            value_fn: Arc::new(move |cost: f64, observation: &W, path_index: usize| match path_index {
                v if v == (length - 1) => {
                    // Last edge (relay -> dest): accept an intermediate balance or immediate connectivity.
                    // No funding requirement here — the final hop's ticket is zero-value, so it
                    // needs no channel; the balance check only selects which stream to score by.
                    if let Some(intermediate) = observation.intermediate_qos()
                        && intermediate.balance().is_some()
                    {
                        let base = score_or_penalize(cost, intermediate.score(), penalty);
                        // For direct routes (length == 1) the single edge is also the immediate peer,
                        // so apply ack rate when immediate data is available.
                        if length == 1
                            && let Some(immediate) = observation.immediate_qos()
                        {
                            return apply_ack_rate(immediate.ack_rate(), base, min_ack_rate, penalty);
                        }
                        return base;
                    }

                    // Fallback: use immediate connectivity score if available
                    if let Some(immediate) = observation.immediate_qos()
                        && immediate.is_connected()
                    {
                        let base = score_or_penalize(cost, immediate.score(), penalty);
                        // Same as above: enforce ack rate for 1-hop routes
                        if length == 1 {
                            return apply_ack_rate(immediate.ack_rate(), base, min_ack_rate, penalty);
                        }
                        return base;
                    }

                    // Last hop is not monetized — penalize but do not reject
                    cost * penalty
                }
                0 => {
                    // First edge: require a connected peer whose channel can fund the rest of the path
                    if let Some(immediate) = observation.immediate_qos()
                        && immediate.is_connected()
                        && let Some(intermediate) = observation.intermediate_qos()
                        && balance_suffices(intermediate.balance(), length - 1, ticket_face_value)
                    {
                        let base = score_or_penalize(cost, observation.score(), penalty);
                        return apply_ack_rate(immediate.ack_rate(), base, min_ack_rate, penalty);
                    }
                    -cost
                }
                index => require_funding(observation, cost, penalty, length - index - 1, ticket_face_value),
            }),
        }
    }

    /// Build a HOPR value function for full graph traversals in the return direction.
    ///
    /// `penalty` is clamped to `[0.0, 1.0]` — the penalizing multiplier applied
    /// to edges lacking probe-based quality observations.
    ///
    /// `min_ack_rate` is clamped to `[0.0, 1.0]` — the minimum acceptable message
    /// acknowledgment rate for the immediate peer. Edges with an ack rate below this
    /// threshold are rejected.
    ///
    /// Used when the planner (`me`) constructs the return path `dest -> relay -> me`.
    /// The first edge (`dest -> relay`) has relaxed requirements compared to
    /// [`EdgeValueFn::forward`] because the planner lacks intermediate QoS data.
    ///
    /// - **Last edge** (relay -> me): requires immediate connectivity; applies the ack rate modifier.
    /// - **All other edges**: require a funding balance; the `penalty` penalizing multiplier is applied when probe
    ///   scores are absent.
    pub fn returning(
        length: std::num::NonZeroUsize,
        penalty: f64,
        min_ack_rate: f64,
        ticket_face_value: Option<Balance>,
    ) -> Self {
        let length = length.get();
        let penalty = penalty.clamp(0.0, 1.0);
        let min_ack_rate = min_ack_rate.clamp(0.0, 1.0);
        Self {
            initial: 1.0,
            min: Some(0.0),
            value_fn: Arc::new(move |cost: f64, observation: &W, path_index: usize| match path_index {
                v if v == (length - 1) => {
                    // Last edge (relay -> me): require connectivity with immediate score
                    if let Some(immediate) = observation.immediate_qos()
                        && immediate.is_connected()
                    {
                        let base = score_or_penalize(cost, immediate.score(), penalty);
                        return apply_ack_rate(immediate.ack_rate(), base, min_ack_rate, penalty);
                    }
                    -cost
                }
                // First edge and intermediaries share the same funding requirement
                index => require_funding(observation, cost, penalty, length - index - 1, ticket_face_value),
            }),
        }
    }

    /// Build a value function for simple forward paths without the final loopback.
    ///
    /// `penalty` is clamped to `[0.0, 1.0]` — the penalizing multiplier applied
    /// to edges lacking probe-based quality observations.
    ///
    /// `min_ack_rate` is clamped to `[0.0, 1.0]` — the minimum acceptable message
    /// acknowledgment rate for the immediate peer. Edges with an ack rate below this
    /// threshold are rejected.
    ///
    /// - **First edge**: same as [`EdgeValueFn::forward`].
    /// - **All other edges**: require a funding balance; the `penalty` penalizing multiplier is applied when probe
    ///   scores are absent.
    ///
    /// `length` counts edges in the **finished** path, including any the caller appends after
    /// traversal — `h + 1` for a loopback of `h` hops. Needed to size each edge's ticket.
    pub fn forward_without_self_loopback(
        length: std::num::NonZeroUsize,
        penalty: f64,
        min_ack_rate: f64,
        ticket_face_value: Option<Balance>,
    ) -> Self {
        let length = length.get();
        let penalty = penalty.clamp(0.0, 1.0);
        let min_ack_rate = min_ack_rate.clamp(0.0, 1.0);
        Self {
            initial: 1.0,
            min: Some(0.0),
            value_fn: Arc::new(move |cost: f64, observation: &W, path_index: usize| match path_index {
                0 => {
                    // First edge: require a connected peer whose channel can fund the rest of the path
                    if let Some(immediate) = observation.immediate_qos()
                        && immediate.is_connected()
                        && let Some(intermediate) = observation.intermediate_qos()
                        && balance_suffices(intermediate.balance(), length - 1, ticket_face_value)
                    {
                        let base = score_or_penalize(cost, observation.score(), penalty);
                        return apply_ack_rate(immediate.ack_rate(), base, min_ack_rate, penalty);
                    }
                    -cost
                }
                // An index at or past `length` means the caller understated it. Reject, rather than
                // let the subtraction collapse to zero remaining hops and waive funding entirely.
                index if index >= length => -cost,
                index => require_funding(observation, cost, penalty, length - index - 1, ticket_face_value),
            }),
        }
    }
}

/// Type alias preserving the original forward value function name.
pub type HoprForwardValueFn<C, W> = EdgeValueFn<C, W>;

/// Type alias preserving the original return value function name.
pub type HoprReturnValueFn<C, W> = EdgeValueFn<C, W>;

/// Type alias preserving the original forward path value function name.
pub type ForwardWithoutSelfLoopbackValueFn<C, W> = EdgeValueFn<C, W>;

#[cfg(test)]
mod tests {
    use anyhow::Context;
    use rstest::rstest;

    use super::*;
    use crate::graph::traits::{
        EdgeImmediateProtocolObservable, EdgeLinkObservable, EdgeNetworkObservableRead, EdgeObservableRead,
        EdgeProtocolObservable, EdgeTransportMeasurement,
    };

    const TEST_PENALTY: f64 = 0.5;
    const TEST_MIN_ACK_RATE: f64 = 0.1;
    /// Probe score well clear of every threshold under test.
    const GOOD_SCORE: f64 = 0.95;
    /// Ack rate comfortably above [`TEST_MIN_ACK_RATE`].
    const GOOD_ACK: f64 = 0.9;
    /// Balance that funds any path length used here.
    const FUNDED: u64 = 1_000;

    // ── Stubs ───────────────────────────────────────────────────────────

    /// Stub for immediate (1-hop) probe measurement.
    #[derive(Debug, Default, Clone, serde::Serialize)]
    struct StubImmediate {
        connected: bool,
        /// `None` models a stream with no observations; `Some(0.0)` one measured and found dead.
        score: Option<f64>,
        ack_rate: Option<f64>,
    }

    impl EdgeNetworkObservableRead for StubImmediate {
        fn is_connected(&self) -> bool {
            self.connected
        }
    }

    impl EdgeImmediateProtocolObservable for StubImmediate {
        fn ack_rate(&self) -> Option<f64> {
            self.ack_rate
        }
    }

    impl EdgeLinkObservable for StubImmediate {
        fn record(&mut self, _: EdgeTransportMeasurement) {
            unreachable!("not used in value function tests")
        }

        fn average_latency(&self) -> Option<std::time::Duration> {
            unreachable!("not used in value function tests")
        }

        fn average_probe_rate(&self) -> f64 {
            unreachable!("not used in value function tests")
        }

        fn has_observations(&self) -> bool {
            self.score.is_some()
        }

        fn score(&self) -> Option<f64> {
            self.score
        }
    }

    /// Renders a `U256` balance as a plain decimal string so snapshots stay readable.
    fn serialize_balance<S: serde::Serializer>(balance: &Option<Balance>, s: S) -> Result<S::Ok, S::Error> {
        match balance {
            Some(balance) => s.serialize_str(&balance.to_string()),
            None => s.serialize_none(),
        }
    }

    /// Stub for intermediate (relayed) probe measurement with channel balance.
    #[derive(Debug, Default, Clone, serde::Serialize)]
    struct StubIntermediate {
        #[serde(serialize_with = "serialize_balance")]
        balance: Option<Balance>,
        /// `None` models a stream with no observations; `Some(0.0)` one measured and found dead.
        score: Option<f64>,
    }

    impl EdgeProtocolObservable for StubIntermediate {
        fn balance(&self) -> Option<Balance> {
            self.balance
        }
    }

    impl EdgeLinkObservable for StubIntermediate {
        fn record(&mut self, _: EdgeTransportMeasurement) {
            unreachable!("not used in value function tests")
        }

        fn average_latency(&self) -> Option<std::time::Duration> {
            unreachable!("not used in value function tests")
        }

        fn average_probe_rate(&self) -> f64 {
            unreachable!("not used in value function tests")
        }

        fn has_observations(&self) -> bool {
            self.score.is_some()
        }

        fn score(&self) -> Option<f64> {
            self.score
        }
    }

    /// Stub `Observations`: a serializable holder for the two measurement streams.
    #[derive(Debug, Default, Clone, serde::Serialize)]
    struct Observations {
        immediate: Option<StubImmediate>,
        intermediate: Option<StubIntermediate>,
    }

    impl EdgeObservableRead for Observations {
        type ImmediateMeasurement = StubImmediate;
        type IntermediateMeasurement = StubIntermediate;

        fn last_update(&self) -> std::time::Duration {
            std::time::Duration::ZERO
        }

        fn immediate_qos(&self) -> Option<&Self::ImmediateMeasurement> {
            self.immediate.as_ref()
        }

        fn intermediate_qos(&self) -> Option<&Self::IntermediateMeasurement> {
            self.intermediate.as_ref()
        }

        fn score(&self) -> Option<f64> {
            let immediate = self.immediate.as_ref().and_then(|i| i.score);
            let intermediate = self.intermediate.as_ref().and_then(|i| i.score);
            match (immediate, intermediate) {
                (Some(immediate), Some(intermediate)) => Some((immediate + intermediate) / 2.0),
                (Some(only), None) | (None, Some(only)) => Some(only),
                (None, None) => None,
            }
        }
    }

    impl Observations {
        /// Attaches a *connected* immediate stream.
        fn with_immediate(mut self, score: Option<f64>, ack_rate: Option<f64>) -> Self {
            self.immediate = Some(StubImmediate {
                connected: true,
                score,
                ack_rate,
            });
            self
        }

        /// Attaches an intermediate stream over a channel holding `balance`.
        fn with_intermediate(mut self, balance: Option<u64>, score: Option<f64>) -> Self {
            self.intermediate = Some(StubIntermediate {
                balance: balance.map(Balance::from),
                score,
            });
            self
        }
    }

    // ── Fixtures ────────────────────────────────────────────────────────

    /// Connected, healthy on both streams, channel funded.
    fn healthy() -> Observations {
        Observations::default()
            .with_immediate(Some(GOOD_SCORE), Some(GOOD_ACK))
            .with_intermediate(Some(FUNDED), Some(GOOD_SCORE))
    }

    /// Connected and healthy, but never reached by a loopback probe.
    fn immediate_only() -> Observations {
        Observations::default().with_immediate(Some(GOOD_SCORE), Some(GOOD_ACK))
    }

    /// Relayed observations over a funded channel, with no direct connection.
    fn relayed_only() -> Observations {
        Observations::default().with_intermediate(Some(FUNDED), Some(GOOD_SCORE))
    }

    /// A funded channel and nothing probed yet.
    fn balance_only() -> Observations {
        Observations::default().with_intermediate(Some(FUNDED), None)
    }

    /// Healthy and probed, but no channel is known.
    fn unfunded() -> Observations {
        Observations::default().with_intermediate(None, Some(GOOD_SCORE))
    }

    /// No streams at all.
    fn unobserved() -> Observations {
        Observations::default()
    }

    /// Healthy over a funded channel, with the given ack rate.
    fn acking(ack_rate: Option<f64>) -> Observations {
        healthy().with_immediate(Some(GOOD_SCORE), ack_rate)
    }

    /// Healthy on the wire, but measured and found dead when relaying.
    fn dead_intermediate() -> Observations {
        healthy().with_intermediate(Some(FUNDED), Some(0.0))
    }

    /// Fully connected and probed, over a channel holding exactly `balance`.
    fn funded_with(balance: u64) -> Observations {
        healthy().with_intermediate(Some(balance), Some(GOOD_SCORE))
    }

    // ── Harness ─────────────────────────────────────────────────────────

    /// Which constructor is under test, recorded in snapshots so a case is self-describing.
    #[derive(Debug, Clone, Copy, serde::Serialize)]
    #[serde(rename_all = "snake_case")]
    enum Direction {
        Forward,
        Returning,
        ForwardWithoutSelfLoopback,
    }

    impl Direction {
        fn build(
            self,
            length: usize,
            ticket_face_value: Option<Balance>,
        ) -> anyhow::Result<EdgeValueFn<f64, Observations>> {
            let length = std::num::NonZeroUsize::new(length).context("path length must be non-zero")?;
            Ok(match self {
                Self::Forward => EdgeValueFn::forward(length, TEST_PENALTY, TEST_MIN_ACK_RATE, ticket_face_value),
                Self::Returning => EdgeValueFn::returning(length, TEST_PENALTY, TEST_MIN_ACK_RATE, ticket_face_value),
                Self::ForwardWithoutSelfLoopback => EdgeValueFn::forward_without_self_loopback(
                    length,
                    TEST_PENALTY,
                    TEST_MIN_ACK_RATE,
                    ticket_face_value,
                ),
            })
        }
    }

    /// Full evaluation context, so a snapshot is readable without its source.
    #[derive(serde::Serialize)]
    struct ValueResult {
        direction: Direction,
        path_length: usize,
        path_index: usize,
        observations: Observations,
        initial_value: f64,
        result_value: f64,
    }

    /// Evaluates one edge and snapshots the whole context under an explicit `case` name.
    ///
    /// The name is explicit because `rstest` would otherwise leave `insta` deriving it from the
    /// generated `case_<n>_<label>`, which renumbers whenever a case is inserted.
    fn assert_edge_value(
        case: &str,
        direction: Direction,
        path_length: usize,
        path_index: usize,
        initial_value: f64,
        observations: Observations,
    ) -> anyhow::Result<()> {
        let result_value =
            direction.build(path_length, None)?.into_value_fn()(initial_value, &observations, path_index);
        insta::assert_yaml_snapshot!(
            case,
            ValueResult {
                direction,
                path_length,
                path_index,
                observations,
                initial_value,
                result_value,
            }
        );
        Ok(())
    }

    /// Evaluates one edge from a unit incoming value, for ordering assertions.
    fn edge_value(
        direction: Direction,
        path_length: usize,
        path_index: usize,
        observations: &Observations,
    ) -> anyhow::Result<f64> {
        Ok(direction.build(path_length, None)?.into_value_fn()(
            1.0,
            observations,
            path_index,
        ))
    }

    // ── Fold invariants ─────────────────────────────────────────────────

    #[rstest]
    #[case::forward(Direction::Forward)]
    #[case::returning(Direction::Returning)]
    #[case::forward_without_self_loopback(Direction::ForwardWithoutSelfLoopback)]
    fn value_fn_should_start_at_the_multiplicative_identity_and_floor_at_zero(
        #[case] direction: Direction,
    ) -> anyhow::Result<()> {
        let value_fn = direction.build(3, None)?;
        assert_eq!(
            value_fn.initial_value(),
            1.0,
            "a product fold must start at the multiplicative identity"
        );
        assert_eq!(
            value_fn.min_value(),
            Some(0.0),
            "a non-positive fold result must prune the path"
        );
        Ok(())
    }

    // ── Forward edge valuation ──────────────────────────────────────────

    #[rstest]
    // First edge (index 0): needs connectivity plus a balance funding the remaining hops.
    #[case::forward_first_edge_healthy("forward_first_edge_healthy", 3, 0, 1.0, healthy())]
    #[case::forward_first_edge_scales_with_the_incoming_value(
        "forward_first_edge_scales_with_the_incoming_value",
        3,
        0,
        2.0,
        healthy()
    )]
    #[case::forward_first_edge_dead_intermediate_is_not_masked(
        "forward_first_edge_dead_intermediate_is_not_masked",
        3,
        0,
        1.0,
        dead_intermediate()
    )]
    #[case::forward_first_edge_not_connected("forward_first_edge_not_connected", 3, 0, 1.0, relayed_only())]
    #[case::forward_first_edge_no_intermediate_stream(
        "forward_first_edge_no_intermediate_stream",
        3,
        0,
        1.0,
        immediate_only()
    )]
    #[case::forward_first_edge_unknown_balance("forward_first_edge_unknown_balance", 3, 0, 1.0, healthy().with_intermediate(None, Some(GOOD_SCORE)))]
    #[case::forward_first_edge_unobserved("forward_first_edge_unobserved", 3, 0, 1.0, unobserved())]
    #[case::forward_first_edge_ack_rate_below_threshold(
        "forward_first_edge_ack_rate_below_threshold",
        3,
        0,
        1.0,
        acking(Some(0.05))
    )]
    #[case::forward_first_edge_no_ack_data("forward_first_edge_no_ack_data", 3, 0, 1.0, acking(None))]
    #[case::forward_first_edge_zero_ack_rate("forward_first_edge_zero_ack_rate", 3, 0, 1.0, acking(Some(0.0)))]
    #[case::forward_first_edge_negative_incoming_value("forward_first_edge_negative_incoming_value", 3, 0, -1.0, unobserved())]
    // Intermediate edges (0 < index < length - 1): need a funding balance.
    #[case::forward_intermediate_edge_healthy("forward_intermediate_edge_healthy", 3, 1, 1.0, healthy())]
    #[case::forward_intermediate_edge_scales_with_the_incoming_value(
        "forward_intermediate_edge_scales_with_the_incoming_value",
        3,
        1,
        2.0,
        healthy()
    )]
    #[case::forward_intermediate_edge_no_intermediate_stream(
        "forward_intermediate_edge_no_intermediate_stream",
        3,
        1,
        1.0,
        immediate_only()
    )]
    #[case::forward_intermediate_edge_unknown_balance(
        "forward_intermediate_edge_unknown_balance",
        3,
        1,
        1.0,
        unfunded()
    )]
    #[case::forward_intermediate_edge_balance_only("forward_intermediate_edge_balance_only", 3, 1, 1.0, balance_only())]
    #[case::forward_intermediate_edge_unobserved("forward_intermediate_edge_unobserved", 3, 1, 1.0, unobserved())]
    // Last edge (index == length - 1): not monetized, so it is penalized rather than rejected.
    #[case::forward_last_edge_healthy("forward_last_edge_healthy", 3, 2, 1.0, healthy())]
    #[case::forward_last_edge_scales_with_the_incoming_value(
        "forward_last_edge_scales_with_the_incoming_value",
        3,
        2,
        2.0,
        healthy()
    )]
    #[case::forward_last_edge_balance_only("forward_last_edge_balance_only", 3, 2, 1.0, balance_only())]
    #[case::forward_last_edge_not_connected("forward_last_edge_not_connected", 3, 2, 1.0, relayed_only())]
    #[case::forward_last_edge_no_intermediate_stream(
        "forward_last_edge_no_intermediate_stream",
        3,
        2,
        1.0,
        immediate_only()
    )]
    #[case::forward_last_edge_unknown_balance("forward_last_edge_unknown_balance", 3, 2, 1.0, unfunded())]
    #[case::forward_last_edge_unobserved("forward_last_edge_unobserved", 3, 2, 1.0, unobserved())]
    // Length boundaries. At length 1 the single edge is both first and last; at length 2 index 1 is
    // already the last edge, so no intermediate arm is reachable.
    #[case::forward_single_edge_is_both_first_and_last(
        "forward_single_edge_is_both_first_and_last",
        1,
        0,
        1.0,
        healthy()
    )]
    #[case::forward_single_edge_ack_rate_below_threshold(
        "forward_single_edge_ack_rate_below_threshold",
        1,
        0,
        1.0,
        acking(Some(0.05))
    )]
    #[case::forward_two_edge_last_edge_healthy("forward_two_edge_last_edge_healthy", 2, 1, 1.0, healthy())]
    #[case::forward_two_edge_last_edge_unobserved("forward_two_edge_last_edge_unobserved", 2, 1, 1.0, unobserved())]
    fn forward_should_value_an_edge_by_position_and_observations(
        #[case] case: &str,
        #[case] path_length: usize,
        #[case] path_index: usize,
        #[case] initial_value: f64,
        #[case] observations: Observations,
    ) -> anyhow::Result<()> {
        assert_edge_value(
            case,
            Direction::Forward,
            path_length,
            path_index,
            initial_value,
            observations,
        )
    }

    // ── Return-direction edge valuation ─────────────────────────────────

    #[rstest]
    // First edge (dest -> relay): the planner has no immediate data for it, so connectivity is not
    // required — only a funding balance.
    #[case::returning_first_edge_relayed_only("returning_first_edge_relayed_only", 2, 0, 1.0, relayed_only())]
    #[case::returning_first_edge_healthy("returning_first_edge_healthy", 2, 0, 1.0, healthy())]
    #[case::returning_first_edge_scales_with_the_incoming_value(
        "returning_first_edge_scales_with_the_incoming_value",
        2,
        0,
        2.0,
        relayed_only()
    )]
    #[case::returning_first_edge_balance_only("returning_first_edge_balance_only", 2, 0, 1.0, balance_only())]
    #[case::returning_first_edge_no_intermediate_stream(
        "returning_first_edge_no_intermediate_stream",
        2,
        0,
        1.0,
        immediate_only()
    )]
    #[case::returning_first_edge_unobserved("returning_first_edge_unobserved", 2, 0, 1.0, unobserved())]
    // Last edge (relay -> me): requires immediate connectivity and applies the ack rate.
    #[case::returning_last_edge_connected("returning_last_edge_connected", 2, 1, 1.0, healthy())]
    #[case::returning_last_edge_not_connected("returning_last_edge_not_connected", 2, 1, 1.0, relayed_only())]
    #[case::returning_last_edge_connected_over_a_dead_unfunded_intermediate("returning_last_edge_connected_over_a_dead_unfunded_intermediate", 2, 1, 1.0, healthy().with_intermediate(None, Some(0.0)))]
    #[case::returning_last_edge_measured_dead_immediate("returning_last_edge_measured_dead_immediate", 2, 1, 1.0, Observations::default().with_immediate(Some(0.0), Some(GOOD_ACK)))]
    #[case::returning_last_edge_ack_rate_below_threshold(
        "returning_last_edge_ack_rate_below_threshold",
        2,
        1,
        1.0,
        acking(Some(0.05))
    )]
    // Intermediate edges share the first edge's funding requirement.
    #[case::returning_intermediate_edge_balance_only(
        "returning_intermediate_edge_balance_only",
        3,
        1,
        1.0,
        balance_only()
    )]
    fn returning_should_value_an_edge_by_position_and_observations(
        #[case] case: &str,
        #[case] path_length: usize,
        #[case] path_index: usize,
        #[case] initial_value: f64,
        #[case] observations: Observations,
    ) -> anyhow::Result<()> {
        assert_edge_value(
            case,
            Direction::Returning,
            path_length,
            path_index,
            initial_value,
            observations,
        )
    }

    #[rstest]
    #[case::loopback_first_edge_ack_rate_below_threshold(
        "loopback_first_edge_ack_rate_below_threshold",
        3,
        0,
        1.0,
        acking(Some(0.05))
    )]
    #[case::loopback_first_edge_healthy("loopback_first_edge_healthy", 3, 0, 1.0, healthy())]
    fn forward_without_self_loopback_should_value_an_edge_by_position_and_observations(
        #[case] case: &str,
        #[case] path_length: usize,
        #[case] path_index: usize,
        #[case] initial_value: f64,
        #[case] observations: Observations,
    ) -> anyhow::Result<()> {
        assert_edge_value(
            case,
            Direction::ForwardWithoutSelfLoopback,
            path_length,
            path_index,
            initial_value,
            observations,
        )
    }

    // ── Whole-path folds ────────────────────────────────────────────────

    #[rstest]
    #[case::forward_over_a_forward_shaped_path("forward_over_a_forward_shaped_path", Direction::Forward, [healthy(), balance_only()])]
    #[case::forward_over_a_return_shaped_path("forward_over_a_return_shaped_path", Direction::Forward, [relayed_only(), healthy()])]
    #[case::returning_over_a_return_shaped_path("returning_over_a_return_shaped_path", Direction::Returning, [relayed_only(), healthy()])]
    #[case::returning_over_a_funded_path("returning_over_a_funded_path", Direction::Returning, [balance_only(), healthy()])]
    fn value_fn_should_fold_a_two_edge_path(
        #[case] case: &str,
        #[case] direction: Direction,
        #[case] edges: [Observations; 2],
    ) -> anyhow::Result<()> {
        let value_fn = direction.build(2, None)?.into_value_fn();
        let after_first_edge = value_fn(1.0, &edges[0], 0);
        let after_last_edge = value_fn(after_first_edge, &edges[1], 1);

        #[derive(serde::Serialize)]
        struct PathCost {
            direction: Direction,
            after_first_edge: f64,
            after_last_edge: f64,
        }

        insta::assert_yaml_snapshot!(
            case,
            PathCost {
                direction,
                after_first_edge,
                after_last_edge,
            }
        );
        Ok(())
    }

    #[test]
    fn forward_and_returning_should_disagree_on_a_return_shaped_last_edge() -> anyhow::Result<()> {
        // `forward` scores the last edge off the intermediate balance, so a relay it has never
        // connected to is still usable. `returning` ends at `me` and therefore demands connectivity.
        let observations = relayed_only();
        let forward = edge_value(Direction::Forward, 2, 1, &observations)?;
        let returning = edge_value(Direction::Returning, 2, 1, &observations)?;

        assert!(
            forward > 0.0,
            "forward accepts an unconnected final relay, got {forward}"
        );
        assert!(
            returning < 0.0,
            "returning must reject a final edge it cannot reach directly, got {returning}"
        );
        Ok(())
    }

    #[test]
    fn returning_should_match_forward_on_intermediate_edges() -> anyhow::Result<()> {
        let observations = healthy();
        assert_eq!(
            edge_value(Direction::Forward, 3, 1, &observations)?,
            edge_value(Direction::Returning, 3, 1, &observations)?,
            "both directions apply the same funding requirement to intermediate edges"
        );
        Ok(())
    }

    // ── Scoring properties ──────────────────────────────────────────────

    #[test]
    fn forward_first_edge_should_score_the_aggregate_rather_than_the_better_stream() -> anyhow::Result<()> {
        // Taking the better of the two streams would hide a dead intermediate entirely, which is
        // the defect the presence rules exist to close. RFC-0014 §4.2 averages when both are
        // present. Acks are fully accounted for here so the assertion isolates the aggregate.
        let masking = dead_intermediate().with_immediate(Some(GOOD_SCORE), Some(1.0));
        let sound = healthy().with_immediate(Some(GOOD_SCORE), Some(1.0));

        let masked = edge_value(Direction::Forward, 3, 0, &masking)?;
        let intact = edge_value(Direction::Forward, 3, 0, &sound)?;

        assert!(
            masked < intact,
            "a dead intermediate must cost the edge, not be hidden by a healthy immediate: {masked} vs {intact}"
        );
        assert!(
            (masked - (GOOD_SCORE + 0.0) / 2.0).abs() < 1e-9,
            "expected the documented average, got {masked}"
        );
        Ok(())
    }

    #[test]
    fn forward_first_edge_should_ignore_an_allocated_but_empty_intermediate_stream() -> anyhow::Result<()> {
        // The permanent state of every edge incident to this node: immediate probes observe it,
        // loopback attribution never targets it, and a balance update allocated the empty
        // intermediate stream.
        let both_observed = edge_value(Direction::Forward, 2, 0, &funded_with(FUNDED))?;
        let intermediate_empty = edge_value(
            Direction::Forward,
            2,
            0,
            &healthy().with_intermediate(Some(FUNDED), None),
        )?;

        assert!(
            (both_observed - intermediate_empty).abs() < 1e-12,
            "an allocated-but-empty intermediate stream must not change the score: {both_observed} vs \
             {intermediate_empty}"
        );
        Ok(())
    }

    #[test]
    fn forward_first_edge_should_rank_a_higher_ack_rate_higher() -> anyhow::Result<()> {
        let generous = edge_value(Direction::Forward, 3, 0, &acking(Some(GOOD_ACK)))?;
        let stingy = edge_value(Direction::Forward, 3, 0, &acking(Some(0.3)))?;
        assert!(
            generous > stingy,
            "a higher ack rate must produce a higher value: {generous} vs {stingy}"
        );
        Ok(())
    }

    #[test]
    fn forward_intermediate_edge_should_depend_on_its_observations() -> anyhow::Result<()> {
        assert_ne!(
            edge_value(Direction::Forward, 3, 1, &unobserved())?,
            edge_value(Direction::Forward, 3, 1, &healthy())?,
            "intermediate edges must be scored from their observations"
        );
        Ok(())
    }

    // ── Measured-dead versus unobserved ─────────────────────────────────

    #[test]
    fn forward_should_rank_a_measured_dead_edge_below_a_barely_working_one() -> anyhow::Result<()> {
        let relayed = |score: Option<f64>| Observations::default().with_intermediate(Some(FUNDED), score);

        // Worst achievable positive score: one success in the window, slowest latency bucket.
        let barely_working = edge_value(Direction::Forward, 3, 1, &relayed(Some(0.2 * 0.15)))?;
        let measured_dead = edge_value(Direction::Forward, 3, 1, &relayed(Some(0.0)))?;
        let never_probed = edge_value(Direction::Forward, 3, 1, &relayed(None))?;

        assert!(
            measured_dead < barely_working,
            "an edge that never relayed anything must rank below one that sometimes does: {measured_dead} vs \
             {barely_working}"
        );
        assert!(
            never_probed > barely_working,
            "an unprobed edge keeps the benefit of the doubt so it stays discoverable"
        );
        Ok(())
    }

    #[test]
    fn forward_should_starve_a_measured_dead_edge_without_pruning_it() -> anyhow::Result<()> {
        // Two requirements pull in opposite directions. The value must stay strictly positive, or
        // the edge is pruned from the probe candidate set and can never recover (RFC-0010 §4.2.3).
        // It must also be minuscule, since weighted-random sampling picks proportionally to value
        // — merely ranking last would still see the edge relaying traffic.
        let measured_dead = edge_value(
            Direction::Forward,
            3,
            1,
            &Observations::default().with_intermediate(Some(FUNDED), Some(0.0)),
        )?;
        let sound = edge_value(Direction::Forward, 3, 1, &relayed_only())?;

        // Both bounds are literal on purpose. Comparing against `MEASURED_DEAD_FLOOR` would move
        // with the constant and assert nothing about it.
        assert!(
            measured_dead > 0.0,
            "a non-positive value prunes the edge from the probe candidate set, so it can never recover"
        );
        assert!(
            measured_dead < 1e-6,
            "and it must be minuscule rather than merely last, got {measured_dead}"
        );

        let share = measured_dead / (measured_dead + sound);
        assert!(
            share < 1e-8,
            "a measured-dead edge must draw a negligible share of the sampling weight, got {share}"
        );
        Ok(())
    }

    // ── Channel funding ────────────────────────────────────────────────

    #[rstest]
    #[case::final_hop_needs_no_channel(None, 0, None, true)]
    #[case::final_hop_exempt_when_drained(Some(0), 0, None, true)]
    #[case::final_hop_exempt_at_a_zero_face_value(Some(0), 0, Some(0), true)]
    #[case::unknown_balance_is_not_evidence_of_sufficiency(None, 1, None, false)]
    #[case::drained_but_open_channel(Some(0), 1, None, false)]
    #[case::exactly_the_requirement(Some(MIN_BALANCE_HEADROOM), 1, None, true)]
    #[case::one_unit_below_the_requirement(Some(MIN_BALANCE_HEADROOM - 1), 1, None, false)]
    // A zero face value is a network pricing relaying at nothing, so any channel funds any hop —
    // but one must still exist, since the relayer issues a (zero-value) ticket on it.
    #[case::free_relaying_over_a_drained_channel(Some(0), 3, Some(0), true)]
    #[case::free_relaying_still_needs_a_channel(None, 3, Some(0), false)]
    fn balance_suffices_should_gate_on_a_fundable_channel(
        #[case] balance: Option<u64>,
        #[case] remaining_hops: usize,
        #[case] ticket_face_value: Option<u64>,
        #[case] expected: bool,
    ) {
        assert_eq!(
            balance_suffices(
                balance.map(Balance::from),
                remaining_hops,
                ticket_face_value.map(Balance::from)
            ),
            expected
        );
    }

    #[rstest]
    fn balance_suffices_should_scale_with_remaining_hops_and_face_value(
        #[values(1, 2, 3)] remaining_hops: usize,
        #[values(None, Some(1_000))] ticket_face_value: Option<u64>,
    ) {
        let face_value = ticket_face_value.map(Balance::from);
        let required = Balance::from(remaining_hops as u64)
            * face_value.unwrap_or_else(default_ticket_face_value)
            * Balance::from(MIN_BALANCE_HEADROOM);

        assert!(
            balance_suffices(Some(required), remaining_hops, face_value),
            "{remaining_hops} hops must accept a balance exactly at the requirement"
        );
        assert!(
            !balance_suffices(Some(required - Balance::one()), remaining_hops, face_value),
            "{remaining_hops} hops must reject a balance one unit below it"
        );
    }

    #[test]
    fn balance_suffices_should_require_more_as_the_face_value_grows() {
        // Every case passing `None` reads the balance as a ticket count. A real face value must
        // make the same balance insufficient.
        let one_ticket_at_unit_face = Balance::from(MIN_BALANCE_HEADROOM);
        assert!(balance_suffices(Some(one_ticket_at_unit_face), 1, None));
        assert!(
            !balance_suffices(Some(one_ticket_at_unit_face), 1, Some(Balance::from(1_000u64))),
            "a larger face value must make the same balance insufficient"
        );
    }

    #[rstest]
    #[case::first_edge(0)]
    #[case::intermediate_edge(1)]
    fn forward_should_reject_an_edge_whose_balance_cannot_fund_the_remaining_hops(
        #[case] path_index: usize,
    ) -> anyhow::Result<()> {
        let insufficient = edge_value(Direction::Forward, 3, path_index, &funded_with(1))?;
        let sufficient = edge_value(Direction::Forward, 3, path_index, &funded_with(FUNDED))?;

        assert!(
            insufficient < 0.0,
            "an underfunded hop at index {path_index} must be rejected, got {insufficient}"
        );
        assert!(
            sufficient > 0.0,
            "a funded hop at index {path_index} must be accepted, got {sufficient}"
        );
        Ok(())
    }

    #[test]
    fn forward_last_edge_should_accept_a_drained_channel() -> anyhow::Result<()> {
        let value = edge_value(Direction::Forward, 3, 2, &funded_with(0))?;
        assert!(
            value > 0.0,
            "the final hop carries a zero-value ticket, so a drained channel must not reject it"
        );
        Ok(())
    }

    #[test]
    fn forward_first_edge_should_reject_when_the_face_value_outgrows_the_balance() -> anyhow::Result<()> {
        let observations = funded_with(1_000);
        let length = std::num::NonZeroUsize::new(3).context("path length must be non-zero")?;

        let cheap = EdgeValueFn::<_, Observations>::forward(length, TEST_PENALTY, TEST_MIN_ACK_RATE, None)
            .into_value_fn()(1.0, &observations, 0);
        let expensive = EdgeValueFn::<_, Observations>::forward(
            length,
            TEST_PENALTY,
            TEST_MIN_ACK_RATE,
            Some(Balance::from(10_000u64)),
        )
        .into_value_fn()(1.0, &observations, 0);

        assert!(cheap > 0.0, "the same balance funds the path at a unit face value");
        assert!(
            expensive < 0.0,
            "and fails once a single ticket costs more than the balance holds, got {expensive}"
        );
        Ok(())
    }

    #[rstest]
    #[case::at_the_declared_length(2)]
    #[case::beyond_the_declared_length(5)]
    fn forward_without_self_loopback_should_reject_an_index_past_the_declared_length(
        #[case] path_index: usize,
    ) -> anyhow::Result<()> {
        // `length` counts the finished path, including the edge the caller appends, so it is the
        // caller that keeps it consistent with the traversed indices. Understating it must not
        // collapse to zero remaining hops, which would waive the funding check and accept a
        // drained relay.
        let value =
            Direction::ForwardWithoutSelfLoopback.build(2, None)?.into_value_fn()(1.0, &funded_with(0), path_index);
        assert!(
            value < 0.0,
            "index {path_index} exceeds a declared length of 2 and must be rejected, got {value}"
        );
        Ok(())
    }
}
