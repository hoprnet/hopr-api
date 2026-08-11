use std::sync::Arc;

use super::traits::{
    ChannelBalance, EdgeImmediateProtocolObservable, EdgeLinkObservable, EdgeNetworkObservableRead, EdgeObservableRead,
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
pub fn default_ticket_face_value() -> ChannelBalance {
    ChannelBalance::one()
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
/// [`ChannelBalance`](super::traits::ChannelBalance).
///
/// `None` face value defaults to [`default_ticket_face_value`], which treats the balance as already
/// counted in single-hop tickets. A zero face value cannot be reasoned about and is treated as
/// unfundable rather than free.
///
/// `remaining_hops == 0` is the final hop: zero-value ticket, no channel needed. A `None` balance is
/// unknown, which is not evidence of sufficiency.
fn balance_suffices(
    balance: Option<ChannelBalance>,
    remaining_hops: usize,
    ticket_face_value: Option<ChannelBalance>,
) -> bool {
    if remaining_hops == 0 {
        return true;
    }

    let ticket_face_value = ticket_face_value.unwrap_or_else(default_ticket_face_value);
    if ticket_face_value.is_zero() {
        return false;
    }

    balance.is_some_and(|balance| {
        ChannelBalance::from(remaining_hops as u64)
            .checked_mul(ticket_face_value)
            .and_then(|required| required.checked_mul(ChannelBalance::from(MIN_BALANCE_HEADROOM)))
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
    ticket_face_value: Option<ChannelBalance>,
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
    /// - **First edge**: requires connectivity and a balance that funds the rest of the path; scores by the better of
    ///   immediate/intermediate observations, then applies the ack rate modifier.
    /// - **Last edge**: accepts an intermediate balance or immediate connectivity; penalizes when neither is available
    ///   (last hop is not monetized). When `length == 1` the single edge is both first and last; the ack rate modifier
    ///   is applied when immediate QoS data is available.
    /// - **Intermediate edges**: require a funding balance; penalize when unprobed.
    pub fn forward(
        length: std::num::NonZeroUsize,
        penalty: f64,
        min_ack_rate: f64,
        ticket_face_value: Option<ChannelBalance>,
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
        ticket_face_value: Option<ChannelBalance>,
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
        ticket_face_value: Option<ChannelBalance>,
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
                        && balance_suffices(intermediate.balance(), length.saturating_sub(1), ticket_face_value)
                    {
                        let base = score_or_penalize(cost, observation.score(), penalty);
                        return apply_ack_rate(immediate.ack_rate(), base, min_ack_rate, penalty);
                    }
                    -cost
                }
                index => require_funding(
                    observation,
                    cost,
                    penalty,
                    length.saturating_sub(index + 1),
                    ticket_face_value,
                ),
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

    use super::*;
    use crate::graph::traits::{
        EdgeImmediateProtocolObservable, EdgeLinkObservable, EdgeNetworkObservableRead, EdgeObservableRead,
        EdgeProtocolObservable, EdgeTransportMeasurement,
    };

    const TEST_PENALTY: f64 = 0.5;
    const TEST_MIN_ACK_RATE: f64 = 0.1;

    // ── Serializable stub types (pure value holders) ─────────────────────

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
    fn serialize_balance<S: serde::Serializer>(balance: &Option<ChannelBalance>, s: S) -> Result<S::Ok, S::Error> {
        match balance {
            Some(balance) => s.serialize_str(&balance.to_string()),
            None => s.serialize_none(),
        }
    }

    /// Stub for intermediate (relayed) probe measurement with channel balance.
    #[derive(Debug, Default, Clone, serde::Serialize)]
    struct StubIntermediate {
        #[serde(serialize_with = "serialize_balance")]
        balance: Option<ChannelBalance>,
        /// `None` models a stream with no observations; `Some(0.0)` one measured and found dead.
        score: Option<f64>,
    }

    impl EdgeProtocolObservable for StubIntermediate {
        fn balance(&self) -> Option<ChannelBalance> {
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

    /// Stub `Observations` type: a serializable value holder for test fixtures.
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

    // ── Test observation builders ───────────────────────────────────────

    /// Connected peer with good QoS scores and a funded channel.
    fn with_connected_and_capacity() -> Observations {
        Observations {
            immediate: Some(StubImmediate {
                connected: true,
                score: Some(0.95),
                ack_rate: Some(0.9),
            }),
            intermediate: Some(StubIntermediate {
                balance: Some(ChannelBalance::from(1000u64)),
                score: Some(0.95),
            }),
        }
    }

    /// Connected peer with only immediate (1-hop) data, no intermediate.
    fn with_connected_only_immediate() -> Observations {
        Observations {
            immediate: Some(StubImmediate {
                connected: true,
                score: Some(0.95),
                ack_rate: Some(0.9),
            }),
            intermediate: None,
        }
    }

    /// Not connected, but has intermediate QoS + a funded channel.
    fn with_not_connected_and_intermediate() -> Observations {
        Observations {
            immediate: None,
            intermediate: Some(StubIntermediate {
                balance: Some(ChannelBalance::from(1000u64)),
                score: Some(0.95),
            }),
        }
    }

    /// No data at all.
    fn with_empty() -> Observations {
        Observations::default()
    }

    /// Only an on-chain balance, no probes run yet.
    fn with_balance_only() -> Observations {
        Observations {
            immediate: None,
            intermediate: Some(StubIntermediate {
                balance: Some(ChannelBalance::from(1000u64)),
                score: None,
            }),
        }
    }

    // ── Snapshot helper ─────────────────────────────────────────────────

    /// Captures the full value function evaluation context for snapshot testing.
    #[derive(serde::Serialize)]
    struct ValueResult {
        observations: Observations,
        initial_value: f64,
        path_index: usize,
        result_value: f64,
    }

    // ── Forward value function trait method tests ─────────────────────────

    #[test]
    fn forward_value_fn_invariants() -> anyhow::Result<()> {
        let value_fn = EdgeValueFn::<_, Observations>::forward(
            std::num::NonZeroUsize::new(3).context("should be non-zero")?,
            TEST_PENALTY,
            TEST_MIN_ACK_RATE,
            None,
        );
        #[derive(serde::Serialize)]
        struct Invariants {
            initial_value: f64,
            min_value: Option<f64>,
        }
        insta::assert_yaml_snapshot!(Invariants {
            initial_value: value_fn.initial_value(),
            min_value: value_fn.min_value(),
        });
        Ok(())
    }

    // ── Forward first edge (path_index == 0) ────────────────────────────

    #[test]
    fn forward_first_edge_positive_when_connected_with_balance() -> anyhow::Result<()> {
        let value_fn = EdgeValueFn::<_, Observations>::forward(
            std::num::NonZeroUsize::new(3).context("should be non-zero")?,
            TEST_PENALTY,
            TEST_MIN_ACK_RATE,
            None,
        );
        let f = value_fn.into_value_fn();
        let obs = with_connected_and_capacity();

        let cost = f(1.0, &obs, 0);
        insta::assert_yaml_snapshot!(ValueResult {
            observations: obs,
            initial_value: 1.0,
            path_index: 0,
            result_value: cost
        });
        Ok(())
    }

    #[test]
    fn forward_first_edge_scales_by_immediate_score() -> anyhow::Result<()> {
        let value_fn = EdgeValueFn::<_, Observations>::forward(
            std::num::NonZeroUsize::new(3).context("should be non-zero")?,
            TEST_PENALTY,
            TEST_MIN_ACK_RATE,
            None,
        );
        let f = value_fn.into_value_fn();
        let obs = with_connected_and_capacity();

        let cost = f(2.0, &obs, 0);
        insta::assert_yaml_snapshot!(ValueResult {
            observations: obs,
            initial_value: 2.0,
            path_index: 0,
            result_value: cost
        });
        Ok(())
    }

    #[test]
    fn forward_first_edge_should_not_let_immediate_mask_a_measured_dead_intermediate() -> anyhow::Result<()> {
        let value_fn = EdgeValueFn::<_, Observations>::forward(
            std::num::NonZeroUsize::new(3).context("should be non-zero")?,
            TEST_PENALTY,
            TEST_MIN_ACK_RATE,
            None,
        );
        let f = value_fn.into_value_fn();
        let obs = Observations {
            immediate: Some(StubImmediate {
                connected: true,
                score: Some(0.95),
                ack_rate: Some(0.9),
            }),
            intermediate: Some(StubIntermediate {
                balance: Some(ChannelBalance::from(1000u64)),
                score: Some(0.0),
            }),
        };

        let cost = f(1.0, &obs, 0);
        insta::assert_yaml_snapshot!(ValueResult {
            observations: obs,
            initial_value: 1.0,
            path_index: 0,
            result_value: cost
        });
        Ok(())
    }

    #[test]
    fn forward_first_edge_negative_when_not_connected() -> anyhow::Result<()> {
        let value_fn = EdgeValueFn::<_, Observations>::forward(
            std::num::NonZeroUsize::new(3).context("should be non-zero")?,
            TEST_PENALTY,
            TEST_MIN_ACK_RATE,
            None,
        );
        let f = value_fn.into_value_fn();
        let obs = with_not_connected_and_intermediate();

        let cost = f(1.0, &obs, 0);
        insta::assert_yaml_snapshot!(ValueResult {
            observations: obs,
            initial_value: 1.0,
            path_index: 0,
            result_value: cost
        });
        Ok(())
    }

    #[test]
    fn forward_first_edge_negative_when_connected_but_no_intermediate() -> anyhow::Result<()> {
        let value_fn = EdgeValueFn::<_, Observations>::forward(
            std::num::NonZeroUsize::new(3).context("should be non-zero")?,
            TEST_PENALTY,
            TEST_MIN_ACK_RATE,
            None,
        );
        let f = value_fn.into_value_fn();
        let obs = with_connected_only_immediate();

        let cost = f(1.0, &obs, 0);
        insta::assert_yaml_snapshot!(ValueResult {
            observations: obs,
            initial_value: 1.0,
            path_index: 0,
            result_value: cost
        });
        Ok(())
    }

    #[test]
    fn forward_first_edge_negative_when_connected_intermediate_but_no_capacity() -> anyhow::Result<()> {
        let value_fn = EdgeValueFn::<_, Observations>::forward(
            std::num::NonZeroUsize::new(3).context("should be non-zero")?,
            TEST_PENALTY,
            TEST_MIN_ACK_RATE,
            None,
        );
        let f = value_fn.into_value_fn();
        let obs = Observations {
            immediate: Some(StubImmediate {
                connected: true,
                score: Some(0.95),
                ack_rate: Some(0.9),
            }),
            intermediate: Some(StubIntermediate {
                balance: None,
                score: Some(0.95),
            }),
        };

        let cost = f(1.0, &obs, 0);
        insta::assert_yaml_snapshot!(ValueResult {
            observations: obs,
            initial_value: 1.0,
            path_index: 0,
            result_value: cost
        });
        Ok(())
    }

    #[test]
    fn forward_first_edge_negative_when_empty() -> anyhow::Result<()> {
        let value_fn = EdgeValueFn::<_, Observations>::forward(
            std::num::NonZeroUsize::new(3).context("should be non-zero")?,
            TEST_PENALTY,
            TEST_MIN_ACK_RATE,
            None,
        );
        let f = value_fn.into_value_fn();
        let obs = with_empty();

        let cost = f(1.0, &obs, 0);
        insta::assert_yaml_snapshot!(ValueResult {
            observations: obs,
            initial_value: 1.0,
            path_index: 0,
            result_value: cost
        });
        Ok(())
    }

    // ── Forward last edge (path_index == length - 1) ────────────────────

    #[test]
    fn forward_last_edge_positive_when_capacity_and_score() -> anyhow::Result<()> {
        let value_fn = EdgeValueFn::<_, Observations>::forward(
            std::num::NonZeroUsize::new(3).context("should be non-zero")?,
            TEST_PENALTY,
            TEST_MIN_ACK_RATE,
            None,
        );
        let f = value_fn.into_value_fn();
        let obs = with_connected_and_capacity();

        let cost = f(1.0, &obs, 2);
        insta::assert_yaml_snapshot!(ValueResult {
            observations: obs,
            initial_value: 1.0,
            path_index: 2,
            result_value: cost
        });
        Ok(())
    }

    #[test]
    fn forward_last_edge_positive_with_capacity_only_no_probes() -> anyhow::Result<()> {
        let value_fn = EdgeValueFn::<_, Observations>::forward(
            std::num::NonZeroUsize::new(3).context("should be non-zero")?,
            TEST_PENALTY,
            TEST_MIN_ACK_RATE,
            None,
        );
        let f = value_fn.into_value_fn();
        let obs = with_balance_only();

        let cost = f(1.0, &obs, 2);
        insta::assert_yaml_snapshot!(ValueResult {
            observations: obs,
            initial_value: 1.0,
            path_index: 2,
            result_value: cost
        });
        Ok(())
    }

    #[test]
    fn forward_last_edge_positive_without_connectivity() -> anyhow::Result<()> {
        let value_fn = EdgeValueFn::<_, Observations>::forward(
            std::num::NonZeroUsize::new(3).context("should be non-zero")?,
            TEST_PENALTY,
            TEST_MIN_ACK_RATE,
            None,
        );
        let f = value_fn.into_value_fn();
        let obs = with_not_connected_and_intermediate();

        let cost = f(1.0, &obs, 2);
        insta::assert_yaml_snapshot!(ValueResult {
            observations: obs,
            initial_value: 1.0,
            path_index: 2,
            result_value: cost
        });
        Ok(())
    }

    #[test]
    fn forward_last_edge_positive_with_connectivity_no_capacity() -> anyhow::Result<()> {
        let value_fn = EdgeValueFn::<_, Observations>::forward(
            std::num::NonZeroUsize::new(3).context("should be non-zero")?,
            TEST_PENALTY,
            TEST_MIN_ACK_RATE,
            None,
        );
        let f = value_fn.into_value_fn();
        let obs = with_connected_only_immediate();

        let cost = f(1.0, &obs, 2);
        insta::assert_yaml_snapshot!(ValueResult {
            observations: obs,
            initial_value: 1.0,
            path_index: 2,
            result_value: cost
        });
        Ok(())
    }

    #[test]
    fn forward_last_edge_scales_by_intermediate_score() -> anyhow::Result<()> {
        let value_fn = EdgeValueFn::<_, Observations>::forward(
            std::num::NonZeroUsize::new(3).context("should be non-zero")?,
            TEST_PENALTY,
            TEST_MIN_ACK_RATE,
            None,
        );
        let f = value_fn.into_value_fn();
        let obs = with_connected_and_capacity();

        let cost = f(2.0, &obs, 2);
        insta::assert_yaml_snapshot!(ValueResult {
            observations: obs,
            initial_value: 2.0,
            path_index: 2,
            result_value: cost
        });
        Ok(())
    }

    #[test]
    fn forward_last_edge_positive_when_intermediate_but_no_capacity() -> anyhow::Result<()> {
        let value_fn = EdgeValueFn::<_, Observations>::forward(
            std::num::NonZeroUsize::new(3).context("should be non-zero")?,
            TEST_PENALTY,
            TEST_MIN_ACK_RATE,
            None,
        );
        let f = value_fn.into_value_fn();
        let obs = Observations {
            immediate: None,
            intermediate: Some(StubIntermediate {
                balance: None,
                score: Some(0.95),
            }),
        };

        let cost = f(1.0, &obs, 2);
        insta::assert_yaml_snapshot!(ValueResult {
            observations: obs,
            initial_value: 1.0,
            path_index: 2,
            result_value: cost
        });
        Ok(())
    }

    #[test]
    fn forward_last_edge_positive_when_empty() -> anyhow::Result<()> {
        let value_fn = EdgeValueFn::<_, Observations>::forward(
            std::num::NonZeroUsize::new(3).context("should be non-zero")?,
            TEST_PENALTY,
            TEST_MIN_ACK_RATE,
            None,
        );
        let f = value_fn.into_value_fn();
        let obs = with_empty();

        let cost = f(1.0, &obs, 2);
        insta::assert_yaml_snapshot!(ValueResult {
            observations: obs,
            initial_value: 1.0,
            path_index: 2,
            result_value: cost
        });
        Ok(())
    }

    // ── Forward intermediate edges (0 < path_index < length - 1) ────────

    #[test]
    fn forward_intermediate_edge_positive_when_capacity_and_score() -> anyhow::Result<()> {
        let value_fn = EdgeValueFn::<_, Observations>::forward(
            std::num::NonZeroUsize::new(3).context("should be non-zero")?,
            TEST_PENALTY,
            TEST_MIN_ACK_RATE,
            None,
        );
        let f = value_fn.into_value_fn();
        let obs = with_connected_and_capacity();

        let cost = f(1.0, &obs, 1);
        insta::assert_yaml_snapshot!(ValueResult {
            observations: obs,
            initial_value: 1.0,
            path_index: 1,
            result_value: cost
        });
        Ok(())
    }

    #[test]
    fn forward_intermediate_edge_scales_by_intermediate_score() -> anyhow::Result<()> {
        let value_fn = EdgeValueFn::<_, Observations>::forward(
            std::num::NonZeroUsize::new(3).context("should be non-zero")?,
            TEST_PENALTY,
            TEST_MIN_ACK_RATE,
            None,
        );
        let f = value_fn.into_value_fn();
        let obs = with_connected_and_capacity();

        let cost = f(2.0, &obs, 1);
        insta::assert_yaml_snapshot!(ValueResult {
            observations: obs,
            initial_value: 2.0,
            path_index: 1,
            result_value: cost
        });
        Ok(())
    }

    #[test]
    fn forward_intermediate_edge_negative_when_no_intermediate() -> anyhow::Result<()> {
        let value_fn = EdgeValueFn::<_, Observations>::forward(
            std::num::NonZeroUsize::new(3).context("should be non-zero")?,
            TEST_PENALTY,
            TEST_MIN_ACK_RATE,
            None,
        );
        let f = value_fn.into_value_fn();
        let obs = with_connected_only_immediate();

        let cost = f(1.0, &obs, 1);
        insta::assert_yaml_snapshot!(ValueResult {
            observations: obs,
            initial_value: 1.0,
            path_index: 1,
            result_value: cost
        });
        Ok(())
    }

    #[test]
    fn forward_intermediate_edge_negative_when_no_capacity() -> anyhow::Result<()> {
        let value_fn = EdgeValueFn::<_, Observations>::forward(
            std::num::NonZeroUsize::new(3).context("should be non-zero")?,
            TEST_PENALTY,
            TEST_MIN_ACK_RATE,
            None,
        );
        let f = value_fn.into_value_fn();
        let obs = Observations {
            immediate: None,
            intermediate: Some(StubIntermediate {
                balance: None,
                score: Some(0.95),
            }),
        };

        let cost = f(1.0, &obs, 1);
        insta::assert_yaml_snapshot!(ValueResult {
            observations: obs,
            initial_value: 1.0,
            path_index: 1,
            result_value: cost
        });
        Ok(())
    }

    #[test]
    fn forward_intermediate_edge_positive_when_capacity_only_no_probes() -> anyhow::Result<()> {
        let value_fn = EdgeValueFn::<_, Observations>::forward(
            std::num::NonZeroUsize::new(3).context("should be non-zero")?,
            TEST_PENALTY,
            TEST_MIN_ACK_RATE,
            None,
        );
        let f = value_fn.into_value_fn();
        let obs = with_balance_only();

        let cost = f(1.0, &obs, 1);
        insta::assert_yaml_snapshot!(ValueResult {
            observations: obs,
            initial_value: 1.0,
            path_index: 1,
            result_value: cost
        });
        Ok(())
    }

    #[test]
    fn forward_intermediate_edge_negative_when_empty() -> anyhow::Result<()> {
        let value_fn = EdgeValueFn::<_, Observations>::forward(
            std::num::NonZeroUsize::new(3).context("should be non-zero")?,
            TEST_PENALTY,
            TEST_MIN_ACK_RATE,
            None,
        );
        let f = value_fn.into_value_fn();
        let obs = with_empty();

        let cost = f(1.0, &obs, 1);
        insta::assert_yaml_snapshot!(ValueResult {
            observations: obs,
            initial_value: 1.0,
            path_index: 1,
            result_value: cost
        });
        Ok(())
    }

    #[test]
    fn forward_intermediate_edge_uses_observations() -> anyhow::Result<()> {
        let value_fn = EdgeValueFn::<_, Observations>::forward(
            std::num::NonZeroUsize::new(3).context("should be non-zero")?,
            TEST_PENALTY,
            TEST_MIN_ACK_RATE,
            None,
        );
        let f = value_fn.into_value_fn();

        let cost_empty = f(1.0, &with_empty(), 1);
        let cost_full = f(1.0, &with_connected_and_capacity(), 1);
        assert_ne!(cost_empty, cost_full, "intermediate edges should use observations");
        Ok(())
    }

    // ── Forward length boundary tests ───────────────────────────────────

    #[test]
    fn forward_length_one_has_only_first_and_last_edge() -> anyhow::Result<()> {
        let value_fn = EdgeValueFn::<_, Observations>::forward(
            std::num::NonZeroUsize::new(1).context("should be non-zero")?,
            TEST_PENALTY,
            TEST_MIN_ACK_RATE,
            None,
        );
        let f = value_fn.into_value_fn();
        let obs = with_connected_and_capacity();

        let cost = f(1.0, &obs, 0);
        insta::assert_yaml_snapshot!(ValueResult {
            observations: obs,
            initial_value: 1.0,
            path_index: 0,
            result_value: cost
        });
        Ok(())
    }

    #[test]
    fn forward_length_one_rejected_when_ack_rate_below_threshold() -> anyhow::Result<()> {
        let value_fn = EdgeValueFn::<_, Observations>::forward(
            std::num::NonZeroUsize::new(1).context("should be non-zero")?,
            TEST_PENALTY,
            TEST_MIN_ACK_RATE,
            None,
        );
        let f = value_fn.into_value_fn();
        let obs = Observations {
            immediate: Some(StubImmediate {
                connected: true,
                score: Some(0.95),
                ack_rate: Some(0.05),
            }),
            intermediate: Some(StubIntermediate {
                balance: Some(ChannelBalance::from(1000u64)),
                score: Some(0.95),
            }),
        };

        let cost = f(1.0, &obs, 0);
        insta::assert_yaml_snapshot!(ValueResult {
            observations: obs,
            initial_value: 1.0,
            path_index: 0,
            result_value: cost
        });
        Ok(())
    }

    #[test]
    fn forward_length_two_intermediate_at_index_one() -> anyhow::Result<()> {
        let value_fn = EdgeValueFn::<_, Observations>::forward(
            std::num::NonZeroUsize::new(2).context("should be non-zero")?,
            TEST_PENALTY,
            TEST_MIN_ACK_RATE,
            None,
        );
        let f = value_fn.into_value_fn();
        let obs = with_connected_and_capacity();

        let cost = f(1.0, &obs, 1);
        insta::assert_yaml_snapshot!(ValueResult {
            observations: obs,
            initial_value: 1.0,
            path_index: 1,
            result_value: cost
        });

        let obs_e = with_empty();
        let cost_empty = f(1.0, &obs_e, 1);
        insta::assert_yaml_snapshot!(ValueResult {
            observations: obs_e,
            initial_value: 1.0,
            path_index: 1,
            result_value: cost_empty
        });
        Ok(())
    }

    // ── Forward negative initial value propagation ───────────────────────

    #[test]
    fn forward_negative_initial_value_inverts_rejection() -> anyhow::Result<()> {
        let value_fn = EdgeValueFn::<_, Observations>::forward(
            std::num::NonZeroUsize::new(3).context("should be non-zero")?,
            TEST_PENALTY,
            TEST_MIN_ACK_RATE,
            None,
        );
        let f = value_fn.into_value_fn();
        let obs = with_empty();

        let cost = f(-1.0, &obs, 0);
        insta::assert_yaml_snapshot!(ValueResult {
            observations: obs,
            initial_value: -1.0,
            path_index: 0,
            result_value: cost
        });
        Ok(())
    }

    // ── Return value function trait method tests ──────────────────────────

    #[test]
    fn return_value_fn_invariants() -> anyhow::Result<()> {
        let value_fn = EdgeValueFn::<_, Observations>::returning(
            std::num::NonZeroUsize::new(3).context("should be non-zero")?,
            TEST_PENALTY,
            TEST_MIN_ACK_RATE,
            None,
        );
        #[derive(serde::Serialize)]
        struct Invariants {
            initial_value: f64,
            min_value: Option<f64>,
        }
        insta::assert_yaml_snapshot!(Invariants {
            initial_value: value_fn.initial_value(),
            min_value: value_fn.min_value(),
        });
        Ok(())
    }

    // ── Return first edge (path_index == 0) ─────────────────────────────

    #[test]
    fn return_first_edge_positive_with_intermediate_and_capacity() -> anyhow::Result<()> {
        let value_fn = EdgeValueFn::<_, Observations>::returning(
            std::num::NonZeroUsize::new(2).context("should be non-zero")?,
            TEST_PENALTY,
            TEST_MIN_ACK_RATE,
            None,
        );
        let f = value_fn.into_value_fn();
        let obs = with_not_connected_and_intermediate();

        let cost = f(1.0, &obs, 0);
        insta::assert_yaml_snapshot!(ValueResult {
            observations: obs,
            initial_value: 1.0,
            path_index: 0,
            result_value: cost
        });
        Ok(())
    }

    #[test]
    fn return_first_edge_positive_with_full_data() -> anyhow::Result<()> {
        let value_fn = EdgeValueFn::<_, Observations>::returning(
            std::num::NonZeroUsize::new(2).context("should be non-zero")?,
            TEST_PENALTY,
            TEST_MIN_ACK_RATE,
            None,
        );
        let f = value_fn.into_value_fn();
        let obs = with_connected_and_capacity();

        let cost = f(1.0, &obs, 0);
        insta::assert_yaml_snapshot!(ValueResult {
            observations: obs,
            initial_value: 1.0,
            path_index: 0,
            result_value: cost
        });
        Ok(())
    }

    #[test]
    fn return_first_edge_scales_by_intermediate_score() -> anyhow::Result<()> {
        let value_fn = EdgeValueFn::<_, Observations>::returning(
            std::num::NonZeroUsize::new(2).context("should be non-zero")?,
            TEST_PENALTY,
            TEST_MIN_ACK_RATE,
            None,
        );
        let f = value_fn.into_value_fn();
        let obs = with_not_connected_and_intermediate();

        let cost = f(2.0, &obs, 0);
        insta::assert_yaml_snapshot!(ValueResult {
            observations: obs,
            initial_value: 2.0,
            path_index: 0,
            result_value: cost
        });
        Ok(())
    }

    #[test]
    fn return_first_edge_does_not_require_connectivity() -> anyhow::Result<()> {
        let value_fn = EdgeValueFn::<_, Observations>::returning(
            std::num::NonZeroUsize::new(2).context("should be non-zero")?,
            TEST_PENALTY,
            TEST_MIN_ACK_RATE,
            None,
        );
        let f = value_fn.into_value_fn();
        let obs = with_not_connected_and_intermediate();

        let cost = f(1.0, &obs, 0);
        insta::assert_yaml_snapshot!(ValueResult {
            observations: obs,
            initial_value: 1.0,
            path_index: 0,
            result_value: cost
        });
        Ok(())
    }

    #[test]
    fn return_first_edge_positive_when_capacity_only_no_probes() -> anyhow::Result<()> {
        let value_fn = EdgeValueFn::<_, Observations>::returning(
            std::num::NonZeroUsize::new(2).context("should be non-zero")?,
            TEST_PENALTY,
            TEST_MIN_ACK_RATE,
            None,
        );
        let f = value_fn.into_value_fn();
        let obs = with_balance_only();

        let cost = f(1.0, &obs, 0);
        insta::assert_yaml_snapshot!(ValueResult {
            observations: obs,
            initial_value: 1.0,
            path_index: 0,
            result_value: cost
        });
        Ok(())
    }

    #[test]
    fn return_first_edge_negative_when_no_capacity() -> anyhow::Result<()> {
        let value_fn = EdgeValueFn::<_, Observations>::returning(
            std::num::NonZeroUsize::new(2).context("should be non-zero")?,
            TEST_PENALTY,
            TEST_MIN_ACK_RATE,
            None,
        );
        let f = value_fn.into_value_fn();
        let obs = with_connected_only_immediate();

        let cost = f(1.0, &obs, 0);
        insta::assert_yaml_snapshot!(ValueResult {
            observations: obs,
            initial_value: 1.0,
            path_index: 0,
            result_value: cost
        });
        Ok(())
    }

    #[test]
    fn return_first_edge_negative_when_empty() -> anyhow::Result<()> {
        let value_fn = EdgeValueFn::<_, Observations>::returning(
            std::num::NonZeroUsize::new(2).context("should be non-zero")?,
            TEST_PENALTY,
            TEST_MIN_ACK_RATE,
            None,
        );
        let f = value_fn.into_value_fn();
        let obs = with_empty();

        let cost = f(1.0, &obs, 0);
        insta::assert_yaml_snapshot!(ValueResult {
            observations: obs,
            initial_value: 1.0,
            path_index: 0,
            result_value: cost
        });
        Ok(())
    }

    // ── Return last edge ────────────────────────────────────────────────

    #[test]
    fn return_last_edge_requires_connectivity() -> anyhow::Result<()> {
        let length = std::num::NonZeroUsize::new(2).context("should be non-zero")?;
        let ret = EdgeValueFn::<_, Observations>::returning(length, TEST_PENALTY, TEST_MIN_ACK_RATE, None);
        let ret_fn = ret.into_value_fn();

        let obs_conn = with_connected_and_capacity();
        let cost_connected = ret_fn(1.0, &obs_conn, 1);
        insta::assert_yaml_snapshot!(ValueResult {
            observations: obs_conn,
            initial_value: 1.0,
            path_index: 1,
            result_value: cost_connected
        });

        let obs_no_conn = with_not_connected_and_intermediate();
        let cost_not_connected = ret_fn(1.0, &obs_no_conn, 1);
        insta::assert_yaml_snapshot!(ValueResult {
            observations: obs_no_conn,
            initial_value: 1.0,
            path_index: 1,
            result_value: cost_not_connected
        });

        Ok(())
    }

    #[test]
    fn return_last_edge_positive_when_connected_with_empty_intermediate() -> anyhow::Result<()> {
        let length = std::num::NonZeroUsize::new(2).context("should be non-zero")?;
        let ret = EdgeValueFn::<_, Observations>::returning(length, TEST_PENALTY, TEST_MIN_ACK_RATE, None);
        let ret_fn = ret.into_value_fn();

        let obs = Observations {
            immediate: Some(StubImmediate {
                connected: true,
                score: Some(0.95),
                ack_rate: Some(0.9),
            }),
            intermediate: Some(StubIntermediate {
                balance: None,
                score: Some(0.0),
            }),
        };

        let cost = ret_fn(1.0, &obs, 1);
        insta::assert_yaml_snapshot!(ValueResult {
            observations: obs,
            initial_value: 1.0,
            path_index: 1,
            result_value: cost
        });

        Ok(())
    }

    #[test]
    fn return_last_edge_starved_when_connected_but_measured_dead() -> anyhow::Result<()> {
        let length = std::num::NonZeroUsize::new(2).context("should be non-zero")?;
        let ret = EdgeValueFn::<_, Observations>::returning(length, TEST_PENALTY, TEST_MIN_ACK_RATE, None);
        let ret_fn = ret.into_value_fn();

        let obs = Observations {
            immediate: Some(StubImmediate {
                connected: true,
                score: Some(0.0),
                ack_rate: Some(0.9),
            }),
            intermediate: None,
        };

        let cost = ret_fn(1.0, &obs, 1);
        insta::assert_yaml_snapshot!(ValueResult {
            observations: obs,
            initial_value: 1.0,
            path_index: 1,
            result_value: cost
        });

        Ok(())
    }

    #[test]
    fn forward_last_edge_differs_from_return_last_edge() -> anyhow::Result<()> {
        let length = std::num::NonZeroUsize::new(2).context("should be non-zero")?;

        let fwd = EdgeValueFn::<_, Observations>::forward(length, TEST_PENALTY, TEST_MIN_ACK_RATE, None);
        let ret = EdgeValueFn::<_, Observations>::returning(length, TEST_PENALTY, TEST_MIN_ACK_RATE, None);
        let fwd_fn = fwd.into_value_fn();
        let ret_fn = ret.into_value_fn();

        let obs = with_not_connected_and_intermediate();
        let fwd_cost = fwd_fn(1.0, &obs, 1);
        let ret_cost = ret_fn(1.0, &obs, 1);

        #[derive(serde::Serialize)]
        struct Comparison {
            observations: Observations,
            forward_last_edge_value: f64,
            return_last_edge_value: f64,
        }

        insta::assert_yaml_snapshot!(Comparison {
            observations: obs,
            forward_last_edge_value: fwd_cost,
            return_last_edge_value: ret_cost,
        });

        Ok(())
    }

    // ── Return intermediate edge ────────────────────────────────────────

    #[test]
    fn return_intermediate_edge_positive_when_capacity_only_no_probes() -> anyhow::Result<()> {
        let value_fn = EdgeValueFn::<_, Observations>::returning(
            std::num::NonZeroUsize::new(3).context("should be non-zero")?,
            TEST_PENALTY,
            TEST_MIN_ACK_RATE,
            None,
        );
        let f = value_fn.into_value_fn();
        let obs = with_balance_only();

        let cost = f(1.0, &obs, 1);
        insta::assert_yaml_snapshot!(ValueResult {
            observations: obs,
            initial_value: 1.0,
            path_index: 1,
            result_value: cost
        });
        Ok(())
    }

    #[test]
    fn return_intermediate_edge_same_as_forward() -> anyhow::Result<()> {
        let length = std::num::NonZeroUsize::new(3).context("should be non-zero")?;

        let fwd = EdgeValueFn::<_, Observations>::forward(length, TEST_PENALTY, TEST_MIN_ACK_RATE, None);
        let ret = EdgeValueFn::<_, Observations>::returning(length, TEST_PENALTY, TEST_MIN_ACK_RATE, None);
        let fwd_fn = fwd.into_value_fn();
        let ret_fn = ret.into_value_fn();

        let obs = with_connected_and_capacity();
        let fwd_cost = fwd_fn(1.0, &obs, 1);
        let ret_cost = ret_fn(1.0, &obs, 1);

        assert_eq!(
            fwd_cost, ret_cost,
            "return intermediate edge should behave identically to forward intermediate edge"
        );

        Ok(())
    }

    // ── Symmetrical communication tests ─────────────────────────────────

    #[test]
    fn symmetrical_forward_without_self_loopback_works_with_forward_value_fn() -> anyhow::Result<()> {
        let length = std::num::NonZeroUsize::new(2).context("should be non-zero")?;
        let value_fn = EdgeValueFn::<_, Observations>::forward(length, TEST_PENALTY, TEST_MIN_ACK_RATE, None);
        let f = value_fn.into_value_fn();

        let me_to_relay = with_connected_and_capacity();
        let relay_to_dest = with_balance_only();

        let cost_after_first = f(1.0, &me_to_relay, 0);
        let cost_after_last = f(cost_after_first, &relay_to_dest, 1);

        #[derive(serde::Serialize)]
        struct PathCost {
            after_first_edge: f64,
            after_last_edge: f64,
        }

        insta::assert_yaml_snapshot!(PathCost {
            after_first_edge: cost_after_first,
            after_last_edge: cost_after_last,
        });

        Ok(())
    }

    #[test]
    fn symmetrical_return_path_rejected_by_forward_value_fn() -> anyhow::Result<()> {
        let length = std::num::NonZeroUsize::new(2).context("should be non-zero")?;
        let value_fn = EdgeValueFn::<_, Observations>::forward(length, TEST_PENALTY, TEST_MIN_ACK_RATE, None);
        let f = value_fn.into_value_fn();

        let dest_to_relay = with_not_connected_and_intermediate();
        let relay_to_me = with_connected_and_capacity();

        let cost_after_first = f(1.0, &dest_to_relay, 0);
        let cost_after_last = f(cost_after_first, &relay_to_me, 1);

        #[derive(serde::Serialize)]
        struct PathCost {
            after_first_edge: f64,
            after_last_edge: f64,
        }

        insta::assert_yaml_snapshot!(PathCost {
            after_first_edge: cost_after_first,
            after_last_edge: cost_after_last,
        });

        Ok(())
    }

    #[test]
    fn symmetrical_return_path_works_with_return_value_fn() -> anyhow::Result<()> {
        let length = std::num::NonZeroUsize::new(2).context("should be non-zero")?;
        let value_fn = EdgeValueFn::<_, Observations>::returning(length, TEST_PENALTY, TEST_MIN_ACK_RATE, None);
        let f = value_fn.into_value_fn();

        let dest_to_relay = with_not_connected_and_intermediate();
        let relay_to_me = with_connected_and_capacity();

        let cost_after_first = f(1.0, &dest_to_relay, 0);
        let cost_after_last = f(cost_after_first, &relay_to_me, 1);

        #[derive(serde::Serialize)]
        struct PathCost {
            after_first_edge: f64,
            after_last_edge: f64,
        }

        insta::assert_yaml_snapshot!(PathCost {
            after_first_edge: cost_after_first,
            after_last_edge: cost_after_last,
        });

        Ok(())
    }

    #[test]
    fn symmetrical_bidirectional_both_paths_positive() -> anyhow::Result<()> {
        let length = std::num::NonZeroUsize::new(2).context("should be non-zero")?;

        let fwd = EdgeValueFn::<_, Observations>::forward(length, TEST_PENALTY, TEST_MIN_ACK_RATE, None);
        let fwd_fn = fwd.into_value_fn();

        let me_to_relay = with_connected_and_capacity();
        let relay_to_dest = with_balance_only();

        let fwd_cost = fwd_fn(1.0, &me_to_relay, 0);
        let fwd_cost = fwd_fn(fwd_cost, &relay_to_dest, 1);

        let ret = EdgeValueFn::<_, Observations>::returning(length, TEST_PENALTY, TEST_MIN_ACK_RATE, None);
        let ret_fn = ret.into_value_fn();

        let dest_to_relay = with_balance_only();
        let relay_to_me = with_connected_and_capacity();

        let ret_cost = ret_fn(1.0, &dest_to_relay, 0);
        let ret_cost = ret_fn(ret_cost, &relay_to_me, 1);

        #[derive(serde::Serialize)]
        struct BidirectionalValue {
            forward_without_self_loopback_value: f64,
            return_path_value: f64,
        }

        insta::assert_yaml_snapshot!(BidirectionalValue {
            forward_without_self_loopback_value: fwd_cost,
            return_path_value: ret_cost,
        });

        Ok(())
    }

    // ── Ack rate value function tests ─────────────────────────────────

    #[test]
    fn forward_first_edge_rejected_when_ack_rate_below_threshold() -> anyhow::Result<()> {
        let value_fn = EdgeValueFn::<_, Observations>::forward(
            std::num::NonZeroUsize::new(3).context("should be non-zero")?,
            TEST_PENALTY,
            TEST_MIN_ACK_RATE,
            None,
        );
        let f = value_fn.into_value_fn();
        let obs = Observations {
            immediate: Some(StubImmediate {
                connected: true,
                score: Some(0.95),
                ack_rate: Some(0.05),
            }),
            intermediate: Some(StubIntermediate {
                balance: Some(ChannelBalance::from(1000u64)),
                score: Some(0.95),
            }),
        };

        let cost = f(1.0, &obs, 0);
        insta::assert_yaml_snapshot!(ValueResult {
            observations: obs,
            initial_value: 1.0,
            path_index: 0,
            result_value: cost
        });
        Ok(())
    }

    #[test]
    fn forward_first_edge_penalized_when_no_ack_data() -> anyhow::Result<()> {
        let value_fn = EdgeValueFn::<_, Observations>::forward(
            std::num::NonZeroUsize::new(3).context("should be non-zero")?,
            TEST_PENALTY,
            TEST_MIN_ACK_RATE,
            None,
        );
        let f = value_fn.into_value_fn();
        let obs = Observations {
            immediate: Some(StubImmediate {
                connected: true,
                score: Some(0.95),
                ack_rate: None,
            }),
            intermediate: Some(StubIntermediate {
                balance: Some(ChannelBalance::from(1000u64)),
                score: Some(0.95),
            }),
        };

        let cost = f(1.0, &obs, 0);
        insta::assert_yaml_snapshot!(ValueResult {
            observations: obs,
            initial_value: 1.0,
            path_index: 0,
            result_value: cost
        });
        Ok(())
    }

    #[test]
    fn forward_first_edge_scales_by_ack_rate() -> anyhow::Result<()> {
        let value_fn = EdgeValueFn::<_, Observations>::forward(
            std::num::NonZeroUsize::new(3).context("should be non-zero")?,
            TEST_PENALTY,
            TEST_MIN_ACK_RATE,
            None,
        );
        let f = value_fn.into_value_fn();
        let obs_high = Observations {
            immediate: Some(StubImmediate {
                connected: true,
                score: Some(0.95),
                ack_rate: Some(0.9),
            }),
            intermediate: Some(StubIntermediate {
                balance: Some(ChannelBalance::from(1000u64)),
                score: Some(0.95),
            }),
        };
        let obs_low = Observations {
            immediate: Some(StubImmediate {
                connected: true,
                score: Some(0.95),
                ack_rate: Some(0.3),
            }),
            intermediate: Some(StubIntermediate {
                balance: Some(ChannelBalance::from(1000u64)),
                score: Some(0.95),
            }),
        };

        let cost_high = f(1.0, &obs_high, 0);
        let cost_low = f(1.0, &obs_low, 0);

        assert!(
            cost_high > cost_low,
            "higher ack rate should produce higher value: {cost_high} vs {cost_low}"
        );
        Ok(())
    }

    #[test]
    fn return_last_edge_rejected_when_ack_rate_below_threshold() -> anyhow::Result<()> {
        let value_fn = EdgeValueFn::<_, Observations>::returning(
            std::num::NonZeroUsize::new(2).context("should be non-zero")?,
            TEST_PENALTY,
            TEST_MIN_ACK_RATE,
            None,
        );
        let f = value_fn.into_value_fn();
        let obs = Observations {
            immediate: Some(StubImmediate {
                connected: true,
                score: Some(0.95),
                ack_rate: Some(0.05),
            }),
            intermediate: Some(StubIntermediate {
                balance: Some(ChannelBalance::from(1000u64)),
                score: Some(0.95),
            }),
        };

        let cost = f(1.0, &obs, 1);
        insta::assert_yaml_snapshot!(ValueResult {
            observations: obs,
            initial_value: 1.0,
            path_index: 1,
            result_value: cost
        });
        Ok(())
    }

    #[test]
    fn adversarial_peer_good_probes_but_zero_acks() -> anyhow::Result<()> {
        let value_fn = EdgeValueFn::<_, Observations>::forward(
            std::num::NonZeroUsize::new(3).context("should be non-zero")?,
            TEST_PENALTY,
            TEST_MIN_ACK_RATE,
            None,
        );
        let f = value_fn.into_value_fn();
        let obs = Observations {
            immediate: Some(StubImmediate {
                connected: true,
                score: Some(0.95),
                ack_rate: Some(0.0),
            }),
            intermediate: Some(StubIntermediate {
                balance: Some(ChannelBalance::from(1000u64)),
                score: Some(0.95),
            }),
        };

        let cost = f(1.0, &obs, 0);
        insta::assert_yaml_snapshot!(ValueResult {
            observations: obs,
            initial_value: 1.0,
            path_index: 0,
            result_value: cost
        });
        Ok(())
    }

    #[test]
    fn forward_without_loopback_first_edge_rejected_when_ack_rate_below_threshold() -> anyhow::Result<()> {
        let value_fn = EdgeValueFn::<_, Observations>::forward_without_self_loopback(
            std::num::NonZeroUsize::new(3).context("should be non-zero")?,
            TEST_PENALTY,
            TEST_MIN_ACK_RATE,
            None,
        );
        let f = value_fn.into_value_fn();
        let obs = Observations {
            immediate: Some(StubImmediate {
                connected: true,
                score: Some(0.95),
                ack_rate: Some(0.05),
            }),
            intermediate: Some(StubIntermediate {
                balance: Some(ChannelBalance::from(1000u64)),
                score: Some(0.95),
            }),
        };

        let cost = f(1.0, &obs, 0);
        insta::assert_yaml_snapshot!(ValueResult {
            observations: obs,
            initial_value: 1.0,
            path_index: 0,
            result_value: cost
        });
        Ok(())
    }

    // ── Capacity sufficiency (C3) ───────────────────────────────────────

    /// Builds a fully connected, probed edge with the given channel balance.
    fn with_balance(balance: u64) -> Observations {
        Observations {
            immediate: Some(StubImmediate {
                connected: true,
                score: Some(0.95),
                ack_rate: Some(0.9),
            }),
            intermediate: Some(StubIntermediate {
                balance: Some(ChannelBalance::from(balance)),
                score: Some(0.95),
            }),
        }
    }

    #[test]
    fn first_edge_must_not_score_above_the_documented_aggregate() -> anyhow::Result<()> {
        // The masking case: immediate looks healthy, intermediate has been measured and found dead.
        // Taking the better of the two streams would hide the dead one entirely, which is the
        // defect the presence rules exist to close. RFC-0014 §4.2 averages when both are present.
        let length = std::num::NonZeroUsize::new(3).context("should be non-zero")?;
        let f = EdgeValueFn::<_, Observations>::forward(length, TEST_PENALTY, TEST_MIN_ACK_RATE, None).into_value_fn();

        let masking = Observations {
            immediate: Some(StubImmediate {
                connected: true,
                score: Some(0.95),
                ack_rate: Some(1.0),
            }),
            intermediate: Some(StubIntermediate {
                balance: Some(ChannelBalance::from(1_000u64)),
                score: Some(0.0),
            }),
        };
        let healthy = Observations {
            intermediate: Some(StubIntermediate {
                balance: Some(ChannelBalance::from(1_000u64)),
                score: Some(0.95),
            }),
            ..masking.clone()
        };

        let masked = f(1.0, &masking, 0);
        let sound = f(1.0, &healthy, 0);

        assert!(
            masked < sound,
            "a dead intermediate must cost the edge, not be hidden by a healthy immediate: {masked} vs {sound}"
        );
        assert!(
            (masked - (0.95 + 0.0) / 2.0).abs() < 0.001,
            "expected the documented average, got {masked}"
        );
        Ok(())
    }

    #[test]
    fn balance_suffices_should_scale_with_a_supplied_face_value() {
        // Every other assertion in this file passes `None`, which resolves to a face value of one
        // and makes the balance read as a ticket count. This exercises the real path.
        let face = ChannelBalance::from(1_000u64);
        let required =
            |hops: usize| ChannelBalance::from(hops as u64) * face * ChannelBalance::from(MIN_BALANCE_HEADROOM);

        for hops in 1..4usize {
            assert!(
                !balance_suffices(Some(required(hops) - ChannelBalance::one()), hops, Some(face)),
                "{hops} hops must reject a balance one unit below the requirement"
            );
            assert!(
                balance_suffices(Some(required(hops)), hops, Some(face)),
                "{hops} hops must accept a balance exactly at the requirement"
            );
        }

        // A balance that suffices at face value one must not suffice at a thousand times that.
        let one_ticket_at_unit_face = ChannelBalance::from(MIN_BALANCE_HEADROOM);
        assert!(balance_suffices(Some(one_ticket_at_unit_face), 1, None));
        assert!(
            !balance_suffices(Some(one_ticket_at_unit_face), 1, Some(face)),
            "a larger face value must make the same balance insufficient"
        );
    }

    #[test]
    fn balance_suffices_should_treat_a_zero_face_value_as_unfundable() {
        assert!(
            !balance_suffices(Some(ChannelBalance::MAX), 1, Some(ChannelBalance::zero())),
            "a zero face value cannot be reasoned about, so it must not read as free"
        );
        assert!(
            balance_suffices(Some(ChannelBalance::zero()), 0, Some(ChannelBalance::zero())),
            "the final hop is exempt regardless of face value"
        );
    }

    #[test]
    fn forward_first_edge_rejected_when_face_value_outgrows_the_balance() -> anyhow::Result<()> {
        let length = std::num::NonZeroUsize::new(3).context("should be non-zero")?;
        let observation = with_balance(1_000);

        let cheap = EdgeValueFn::<_, Observations>::forward(length, TEST_PENALTY, TEST_MIN_ACK_RATE, None)
            .into_value_fn()(1.0, &observation, 0);
        let expensive = EdgeValueFn::<_, Observations>::forward(
            length,
            TEST_PENALTY,
            TEST_MIN_ACK_RATE,
            Some(ChannelBalance::from(10_000u64)),
        )
        .into_value_fn()(1.0, &observation, 0);

        assert!(cheap > 0.0, "the same balance funds the path at a unit face value");
        assert!(
            expensive < 0.0,
            "and fails to fund it once a ticket costs more than the balance holds, got {expensive}"
        );
        Ok(())
    }

    #[test]
    fn balance_suffices_should_exempt_the_final_hop() {
        // The final hop's ticket is zero-value, so it needs no channel at all.
        assert!(balance_suffices(None, 0, None), "final hop needs no balance");
        assert!(
            balance_suffices(Some(ChannelBalance::zero()), 0, None),
            "final hop needs no balance even at zero"
        );
    }

    #[test]
    fn balance_suffices_should_reject_a_drained_but_open_channel() {
        // `Some(0)` is a real value: an OPEN channel whose balance no longer covers one ticket.
        // A bare presence check accepts it, which is the bug this guards.
        assert!(
            !balance_suffices(Some(ChannelBalance::zero()), 1, None),
            "a channel that cannot cover a single ticket must not fund a relay hop"
        );
    }

    #[test]
    fn balance_suffices_should_reject_unknown_capacity_for_a_funded_hop() {
        assert!(
            !balance_suffices(None, 1, None),
            "unknown capacity is not evidence of sufficiency"
        );
    }

    #[test]
    fn balance_suffices_should_scale_with_remaining_hops() {
        // A relayer with `r` hops after it issues a ticket worth `r` single-hop tickets, and
        // MIN_BALANCE_HEADROOM is applied on top.
        let required = |hops: usize| {
            ChannelBalance::from(hops as u64) * default_ticket_face_value() * ChannelBalance::from(MIN_BALANCE_HEADROOM)
        };

        for hops in 1..4usize {
            assert!(
                !balance_suffices(Some(required(hops) - ChannelBalance::one()), hops, None),
                "{hops} remaining hops must reject capacity just below the requirement"
            );
            assert!(
                balance_suffices(Some(required(hops)), hops, None),
                "{hops} remaining hops must accept capacity exactly at the requirement"
            );
        }

        assert!(
            balance_suffices(Some(required(1)), 1, None) && !balance_suffices(Some(required(1)), 3, None),
            "the same capacity must fund a short remainder but not a longer one"
        );
    }

    #[test]
    fn forward_first_edge_rejected_when_capacity_cannot_fund_remaining_hops() -> anyhow::Result<()> {
        let length = std::num::NonZeroUsize::new(3).context("should be non-zero")?;
        let f = EdgeValueFn::<_, Observations>::forward(length, TEST_PENALTY, TEST_MIN_ACK_RATE, None).into_value_fn();

        // Two hops remain after the first edge, so it must fund a two-hop ticket.
        let insufficient = f(1.0, &with_balance(1), 0);
        let sufficient = f(1.0, &with_balance(1_000), 0);

        assert!(
            insufficient < 0.0,
            "an underfunded first hop must be rejected, got {insufficient}"
        );
        assert!(
            sufficient > 0.0,
            "a funded first hop must be accepted, got {sufficient}"
        );
        Ok(())
    }

    #[test]
    fn forward_intermediate_edge_rejected_when_capacity_cannot_fund_remaining_hops() -> anyhow::Result<()> {
        let length = std::num::NonZeroUsize::new(3).context("should be non-zero")?;
        let f = EdgeValueFn::<_, Observations>::forward(length, TEST_PENALTY, TEST_MIN_ACK_RATE, None).into_value_fn();

        assert!(f(1.0, &with_balance(0), 1) < 0.0, "drained intermediate edge rejected");
        assert!(
            f(1.0, &with_balance(1_000), 1) > 0.0,
            "funded intermediate edge accepted"
        );
        Ok(())
    }

    #[test]
    fn forward_last_edge_accepted_regardless_of_capacity() -> anyhow::Result<()> {
        let length = std::num::NonZeroUsize::new(3).context("should be non-zero")?;
        let f = EdgeValueFn::<_, Observations>::forward(length, TEST_PENALTY, TEST_MIN_ACK_RATE, None).into_value_fn();

        assert!(
            f(1.0, &with_balance(0), 2) > 0.0,
            "the final hop carries a zero-value ticket, so a drained channel must not reject it"
        );
        Ok(())
    }

    // ── Measured-dead vs unobserved (C2) ────────────────────────────────

    #[test]
    fn measured_dead_edge_should_rank_below_a_partially_working_one() -> anyhow::Result<()> {
        let length = std::num::NonZeroUsize::new(3).context("should be non-zero")?;
        let f = EdgeValueFn::<_, Observations>::forward(length, TEST_PENALTY, TEST_MIN_ACK_RATE, None).into_value_fn();

        let scored = |score: Option<f64>| Observations {
            immediate: None,
            intermediate: Some(StubIntermediate {
                balance: Some(ChannelBalance::from(1_000u64)),
                score,
            }),
        };

        // Worst achievable positive score: one success in the window, slowest latency bucket.
        let barely_working = f(1.0, &scored(Some(0.2 * 0.15)), 1);
        let measured_dead = f(1.0, &scored(Some(0.0)), 1);
        let never_probed = f(1.0, &scored(None), 1);

        assert!(
            measured_dead > 0.0,
            "measured-dead must stay strictly positive so the edge is starved, not pruned out of the probe candidate \
             set that would let it recover"
        );
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
    fn forward_first_edge_should_not_be_dragged_down_by_an_empty_intermediate_stream() -> anyhow::Result<()> {
        // The permanent state of every edge incident to this node: immediate probes observe it,
        // loopback attribution never targets it, and a capacity update allocated the empty
        // intermediate stream.
        let length = std::num::NonZeroUsize::new(2).context("should be non-zero")?;
        let f = EdgeValueFn::<_, Observations>::forward(length, TEST_PENALTY, TEST_MIN_ACK_RATE, None).into_value_fn();

        let both_observed = f(1.0, &with_balance(1_000), 0);
        let intermediate_empty = f(
            1.0,
            &Observations {
                immediate: Some(StubImmediate {
                    connected: true,
                    score: Some(0.95),
                    ack_rate: Some(0.9),
                }),
                intermediate: Some(StubIntermediate {
                    balance: Some(ChannelBalance::from(1_000u64)),
                    score: None,
                }),
            },
            0,
        );

        assert!(
            (both_observed - intermediate_empty).abs() < 1e-12,
            "an allocated-but-empty intermediate stream must not change the score: {both_observed} vs \
             {intermediate_empty}"
        );
        Ok(())
    }
}
