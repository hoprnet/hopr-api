use std::sync::Arc;

use super::traits::{
    CostFn, EdgeImmediateProtocolObservable, EdgeLinkObservable, EdgeNetworkObservableRead, EdgeObservableRead,
    EdgeProtocolObservable,
};

/// A shared cost function accepting `(current_cost, edge_weight, path_index) -> new_cost`.
pub type BasicCostFn<C, W> = Arc<dyn Fn(C, &W, usize) -> C + Send + Sync>;

/// Scales cost by score when probes exist, otherwise applies the penalizing multiplier.
fn score_or_penalize(cost: f64, score: f64, penalty: f64) -> f64 {
    if score > 0.0 { cost * score } else { cost * penalty }
}

/// Applies the ack rate as a cost modifier for immediate edges.
///
/// When the ack rate is available and below `min_ack_rate`, the edge is rejected.
/// When available and above the threshold, the cost is scaled by the ack rate.
/// When unavailable (insufficient data), the penalty multiplier is applied.
fn apply_ack_rate(ack_rate: Option<f64>, cost: f64, min_ack_rate: f64, penalty: f64) -> f64 {
    match ack_rate {
        Some(rate) if rate < min_ack_rate => -cost,
        Some(rate) => cost * rate,
        None => cost * penalty,
    }
}

/// Checks for intermediate capacity and applies score-or-penalize, rejecting if absent.
fn require_capacity<W: EdgeObservableRead>(observation: &W, cost: f64, penalty: f64) -> f64 {
    if let Some(intermediate) = observation.intermediate_qos()
        && intermediate.capacity().is_some()
    {
        return score_or_penalize(cost, intermediate.score(), penalty);
    }

    -cost
}

/// A graph edge cost function implementing a fold over path edges.
///
/// The `penalty` is a penalizing multiplier applied to edges that lack
/// probe-based quality observations (e.g. only on-chain capacity or only
/// immediate connectivity data). It scales the accumulated cost downward,
/// making unprobed edges less attractive than measured ones while still
/// allowing path discovery. A value of `1.0` means no penalty; lower
/// values (e.g. `0.5`) increasingly penalize unprobed edges.
///
/// Use one of the named constructors to create the appropriate variant:
/// - [`EdgeCostFn::forward`] — full graph traversal in the forward direction
/// - [`EdgeCostFn::returning`] — full graph traversal in the return direction
/// - [`EdgeCostFn::forward_without_self_loopback`] — simple forward paths without final loopback
pub struct EdgeCostFn<C, W> {
    initial: C,
    min: Option<C>,
    cost_fn: BasicCostFn<C, W>,
}

impl<C: Clone, W> Clone for EdgeCostFn<C, W> {
    fn clone(&self) -> Self {
        Self {
            initial: self.initial.clone(),
            min: self.min.clone(),
            cost_fn: Arc::clone(&self.cost_fn),
        }
    }
}

impl<C, W> CostFn for EdgeCostFn<C, W>
where
    C: Clone + PartialOrd + Send + Sync + 'static,
    W: EdgeObservableRead + Send + 'static,
{
    type Cost = C;
    type Weight = W;

    fn initial_cost(&self) -> Self::Cost {
        self.initial.clone()
    }

    fn min_cost(&self) -> Option<Self::Cost> {
        self.min.clone()
    }

    fn into_cost_fn(self) -> BasicCostFn<Self::Cost, Self::Weight> {
        self.cost_fn
    }
}

impl<W> EdgeCostFn<f64, W>
where
    W: EdgeObservableRead + Send + 'static,
{
    /// Build a forward HOPR cost function for full graph traversals.
    ///
    /// `penalty` is clamped to `[0.0, 1.0]` — the penalizing multiplier applied
    /// to edges lacking probe-based quality observations.
    ///
    /// `min_ack_rate` is clamped to `[0.0, 1.0]` — the minimum acceptable message
    /// acknowledgment rate for the immediate peer. Edges with an ack rate below this
    /// threshold are rejected.
    ///
    /// - **First edge**: requires connectivity and intermediate capacity; scores by the better of
    ///   immediate/intermediate observations, then applies the ack rate modifier.
    /// - **Last edge**: accepts intermediate capacity or immediate connectivity; penalizes when neither is available
    ///   (last hop is not monetized). When `length == 1` the single edge is both first and last; the ack rate modifier
    ///   is applied when immediate QoS data is available.
    /// - **Intermediate edges**: require capacity; penalize when unprobed.
    pub fn forward(length: std::num::NonZeroUsize, penalty: f64, min_ack_rate: f64) -> Self {
        let length = length.get();
        let penalty = penalty.clamp(0.0, 1.0);
        let min_ack_rate = min_ack_rate.clamp(0.0, 1.0);
        Self {
            initial: 1.0,
            min: Some(0.0),
            cost_fn: Arc::new(move |cost: f64, observation: &W, path_index: usize| match path_index {
                v if v == (length - 1) => {
                    // Last edge (relay -> dest): accept intermediate capacity or immediate connectivity
                    if let Some(intermediate) = observation.intermediate_qos()
                        && intermediate.capacity().is_some()
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
                    // First edge: require connected peer with intermediate capacity
                    if let Some(immediate) = observation.immediate_qos()
                        && immediate.is_connected()
                        && let Some(intermediate) = observation.intermediate_qos()
                        && intermediate.capacity().is_some()
                    {
                        let base = cost * immediate.score().max(intermediate.score());
                        return apply_ack_rate(immediate.ack_rate(), base, min_ack_rate, penalty);
                    }
                    -cost
                }
                _ => require_capacity(observation, cost, penalty),
            }),
        }
    }

    /// Build a HOPR cost function for full graph traversals in the return direction.
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
    /// [`EdgeCostFn::forward`] because the planner lacks intermediate QoS data.
    ///
    /// - **Last edge** (relay -> me): requires immediate connectivity; applies the ack rate modifier.
    /// - **All other edges**: require intermediate capacity; the `penalty` penalizing multiplier is applied when probe
    ///   scores are absent.
    pub fn returning(length: std::num::NonZeroUsize, penalty: f64, min_ack_rate: f64) -> Self {
        let length = length.get();
        let penalty = penalty.clamp(0.0, 1.0);
        let min_ack_rate = min_ack_rate.clamp(0.0, 1.0);
        Self {
            initial: 1.0,
            min: Some(0.0),
            cost_fn: Arc::new(move |cost: f64, observation: &W, path_index: usize| match path_index {
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
                // First edge and intermediaries share the same capacity requirement
                _ => require_capacity(observation, cost, penalty),
            }),
        }
    }

    /// Build a cost function for simple forward paths without the final loopback.
    ///
    /// `penalty` is clamped to `[0.0, 1.0]` — the penalizing multiplier applied
    /// to edges lacking probe-based quality observations.
    ///
    /// `min_ack_rate` is clamped to `[0.0, 1.0]` — the minimum acceptable message
    /// acknowledgment rate for the immediate peer. Edges with an ack rate below this
    /// threshold are rejected.
    ///
    /// - **First edge**: same as [`EdgeCostFn::forward`].
    /// - **All other edges**: require capacity; the `penalty` penalizing multiplier is applied when probe scores are
    ///   absent.
    pub fn forward_without_self_loopback(penalty: f64, min_ack_rate: f64) -> Self {
        let penalty = penalty.clamp(0.0, 1.0);
        let min_ack_rate = min_ack_rate.clamp(0.0, 1.0);
        Self {
            initial: 1.0,
            min: Some(0.0),
            cost_fn: Arc::new(move |cost: f64, observation: &W, path_index: usize| match path_index {
                0 => {
                    // First edge: require connected peer with intermediate capacity
                    if let Some(immediate) = observation.immediate_qos()
                        && immediate.is_connected()
                        && let Some(intermediate) = observation.intermediate_qos()
                        && intermediate.capacity().is_some()
                    {
                        let base = cost * immediate.score().max(intermediate.score());
                        return apply_ack_rate(immediate.ack_rate(), base, min_ack_rate, penalty);
                    }
                    -cost
                }
                _ => require_capacity(observation, cost, penalty),
            }),
        }
    }
}

/// Type alias preserving the original forward cost function name.
pub type HoprForwardCostFn<C, W> = EdgeCostFn<C, W>;

/// Type alias preserving the original return cost function name.
pub type HoprReturnCostFn<C, W> = EdgeCostFn<C, W>;

/// Type alias preserving the original forward path cost function name.
pub type ForwardWithoutSelfLoopbackCostFn<C, W> = EdgeCostFn<C, W>;

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
        score: f64,
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
            unreachable!("not used in cost function tests")
        }

        fn average_latency(&self) -> Option<std::time::Duration> {
            unreachable!("not used in cost function tests")
        }

        fn average_probe_rate(&self) -> f64 {
            unreachable!("not used in cost function tests")
        }

        fn score(&self) -> f64 {
            self.score
        }
    }

    /// Stub for intermediate (relayed) probe measurement with capacity.
    #[derive(Debug, Default, Clone, serde::Serialize)]
    struct StubIntermediate {
        capacity: Option<u128>,
        score: f64,
    }

    impl EdgeProtocolObservable for StubIntermediate {
        fn capacity(&self) -> Option<u128> {
            self.capacity
        }
    }

    impl EdgeLinkObservable for StubIntermediate {
        fn record(&mut self, _: EdgeTransportMeasurement) {
            unreachable!("not used in cost function tests")
        }

        fn average_latency(&self) -> Option<std::time::Duration> {
            unreachable!("not used in cost function tests")
        }

        fn average_probe_rate(&self) -> f64 {
            unreachable!("not used in cost function tests")
        }

        fn score(&self) -> f64 {
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

        fn score(&self) -> f64 {
            self.intermediate
                .as_ref()
                .map(|i| i.score)
                .or_else(|| self.immediate.as_ref().map(|i| i.score))
                .unwrap_or(0.0)
        }
    }

    // ── Test observation builders ───────────────────────────────────────

    /// Connected peer with good QoS scores and channel capacity.
    fn with_connected_and_capacity() -> Observations {
        Observations {
            immediate: Some(StubImmediate {
                connected: true,
                score: 0.95,
                ack_rate: Some(0.9),
            }),
            intermediate: Some(StubIntermediate {
                capacity: Some(1000),
                score: 0.95,
            }),
        }
    }

    /// Connected peer with only immediate (1-hop) data, no intermediate.
    fn with_connected_only_immediate() -> Observations {
        Observations {
            immediate: Some(StubImmediate {
                connected: true,
                score: 0.95,
                ack_rate: Some(0.9),
            }),
            intermediate: None,
        }
    }

    /// Not connected, but has intermediate QoS + channel capacity.
    fn with_not_connected_and_intermediate() -> Observations {
        Observations {
            immediate: None,
            intermediate: Some(StubIntermediate {
                capacity: Some(1000),
                score: 0.95,
            }),
        }
    }

    /// No data at all.
    fn with_empty() -> Observations {
        Observations::default()
    }

    /// Only on-chain channel capacity, no probes run yet.
    fn with_capacity_only() -> Observations {
        Observations {
            immediate: None,
            intermediate: Some(StubIntermediate {
                capacity: Some(1000),
                score: 0.0,
            }),
        }
    }

    // ── Snapshot helper ─────────────────────────────────────────────────

    /// Captures the full cost function evaluation context for snapshot testing.
    #[derive(serde::Serialize)]
    struct CostResult {
        observations: Observations,
        initial_cost: f64,
        path_index: usize,
        result_cost: f64,
    }

    // ── Forward cost function trait method tests ─────────────────────────

    #[test]
    fn forward_cost_fn_invariants() -> anyhow::Result<()> {
        let cost_fn = EdgeCostFn::<_, Observations>::forward(
            std::num::NonZeroUsize::new(3).context("should be non-zero")?,
            TEST_PENALTY,
            TEST_MIN_ACK_RATE,
        );
        #[derive(serde::Serialize)]
        struct Invariants {
            initial_cost: f64,
            min_cost: Option<f64>,
        }
        insta::assert_yaml_snapshot!(Invariants {
            initial_cost: cost_fn.initial_cost(),
            min_cost: cost_fn.min_cost(),
        });
        Ok(())
    }

    // ── Forward first edge (path_index == 0) ────────────────────────────

    #[test]
    fn forward_first_edge_positive_when_connected_with_capacity() -> anyhow::Result<()> {
        let cost_fn = EdgeCostFn::<_, Observations>::forward(
            std::num::NonZeroUsize::new(3).context("should be non-zero")?,
            TEST_PENALTY,
            TEST_MIN_ACK_RATE,
        );
        let f = cost_fn.into_cost_fn();
        let obs = with_connected_and_capacity();

        let cost = f(1.0, &obs, 0);
        insta::assert_yaml_snapshot!(CostResult {
            observations: obs,
            initial_cost: 1.0,
            path_index: 0,
            result_cost: cost
        });
        Ok(())
    }

    #[test]
    fn forward_first_edge_scales_by_immediate_score() -> anyhow::Result<()> {
        let cost_fn = EdgeCostFn::<_, Observations>::forward(
            std::num::NonZeroUsize::new(3).context("should be non-zero")?,
            TEST_PENALTY,
            TEST_MIN_ACK_RATE,
        );
        let f = cost_fn.into_cost_fn();
        let obs = with_connected_and_capacity();

        let cost = f(2.0, &obs, 0);
        insta::assert_yaml_snapshot!(CostResult {
            observations: obs,
            initial_cost: 2.0,
            path_index: 0,
            result_cost: cost
        });
        Ok(())
    }

    #[test]
    fn forward_first_edge_positive_when_capacity_only_no_intermediate_probe() -> anyhow::Result<()> {
        let cost_fn = EdgeCostFn::<_, Observations>::forward(
            std::num::NonZeroUsize::new(3).context("should be non-zero")?,
            TEST_PENALTY,
            TEST_MIN_ACK_RATE,
        );
        let f = cost_fn.into_cost_fn();
        let obs = Observations {
            immediate: Some(StubImmediate {
                connected: true,
                score: 0.95,
                ack_rate: Some(0.9),
            }),
            intermediate: Some(StubIntermediate {
                capacity: Some(1000),
                score: 0.0,
            }),
        };

        let cost = f(1.0, &obs, 0);
        insta::assert_yaml_snapshot!(CostResult {
            observations: obs,
            initial_cost: 1.0,
            path_index: 0,
            result_cost: cost
        });
        Ok(())
    }

    #[test]
    fn forward_first_edge_negative_when_not_connected() -> anyhow::Result<()> {
        let cost_fn = EdgeCostFn::<_, Observations>::forward(
            std::num::NonZeroUsize::new(3).context("should be non-zero")?,
            TEST_PENALTY,
            TEST_MIN_ACK_RATE,
        );
        let f = cost_fn.into_cost_fn();
        let obs = with_not_connected_and_intermediate();

        let cost = f(1.0, &obs, 0);
        insta::assert_yaml_snapshot!(CostResult {
            observations: obs,
            initial_cost: 1.0,
            path_index: 0,
            result_cost: cost
        });
        Ok(())
    }

    #[test]
    fn forward_first_edge_negative_when_connected_but_no_intermediate() -> anyhow::Result<()> {
        let cost_fn = EdgeCostFn::<_, Observations>::forward(
            std::num::NonZeroUsize::new(3).context("should be non-zero")?,
            TEST_PENALTY,
            TEST_MIN_ACK_RATE,
        );
        let f = cost_fn.into_cost_fn();
        let obs = with_connected_only_immediate();

        let cost = f(1.0, &obs, 0);
        insta::assert_yaml_snapshot!(CostResult {
            observations: obs,
            initial_cost: 1.0,
            path_index: 0,
            result_cost: cost
        });
        Ok(())
    }

    #[test]
    fn forward_first_edge_negative_when_connected_intermediate_but_no_capacity() -> anyhow::Result<()> {
        let cost_fn = EdgeCostFn::<_, Observations>::forward(
            std::num::NonZeroUsize::new(3).context("should be non-zero")?,
            TEST_PENALTY,
            TEST_MIN_ACK_RATE,
        );
        let f = cost_fn.into_cost_fn();
        let obs = Observations {
            immediate: Some(StubImmediate {
                connected: true,
                score: 0.95,
                ack_rate: Some(0.9),
            }),
            intermediate: Some(StubIntermediate {
                capacity: None,
                score: 0.95,
            }),
        };

        let cost = f(1.0, &obs, 0);
        insta::assert_yaml_snapshot!(CostResult {
            observations: obs,
            initial_cost: 1.0,
            path_index: 0,
            result_cost: cost
        });
        Ok(())
    }

    #[test]
    fn forward_first_edge_negative_when_empty() -> anyhow::Result<()> {
        let cost_fn = EdgeCostFn::<_, Observations>::forward(
            std::num::NonZeroUsize::new(3).context("should be non-zero")?,
            TEST_PENALTY,
            TEST_MIN_ACK_RATE,
        );
        let f = cost_fn.into_cost_fn();
        let obs = with_empty();

        let cost = f(1.0, &obs, 0);
        insta::assert_yaml_snapshot!(CostResult {
            observations: obs,
            initial_cost: 1.0,
            path_index: 0,
            result_cost: cost
        });
        Ok(())
    }

    // ── Forward last edge (path_index == length - 1) ────────────────────

    #[test]
    fn forward_last_edge_positive_when_capacity_and_score() -> anyhow::Result<()> {
        let cost_fn = EdgeCostFn::<_, Observations>::forward(
            std::num::NonZeroUsize::new(3).context("should be non-zero")?,
            TEST_PENALTY,
            TEST_MIN_ACK_RATE,
        );
        let f = cost_fn.into_cost_fn();
        let obs = with_connected_and_capacity();

        let cost = f(1.0, &obs, 2);
        insta::assert_yaml_snapshot!(CostResult {
            observations: obs,
            initial_cost: 1.0,
            path_index: 2,
            result_cost: cost
        });
        Ok(())
    }

    #[test]
    fn forward_last_edge_positive_with_capacity_only_no_probes() -> anyhow::Result<()> {
        let cost_fn = EdgeCostFn::<_, Observations>::forward(
            std::num::NonZeroUsize::new(3).context("should be non-zero")?,
            TEST_PENALTY,
            TEST_MIN_ACK_RATE,
        );
        let f = cost_fn.into_cost_fn();
        let obs = with_capacity_only();

        let cost = f(1.0, &obs, 2);
        insta::assert_yaml_snapshot!(CostResult {
            observations: obs,
            initial_cost: 1.0,
            path_index: 2,
            result_cost: cost
        });
        Ok(())
    }

    #[test]
    fn forward_last_edge_positive_without_connectivity() -> anyhow::Result<()> {
        let cost_fn = EdgeCostFn::<_, Observations>::forward(
            std::num::NonZeroUsize::new(3).context("should be non-zero")?,
            TEST_PENALTY,
            TEST_MIN_ACK_RATE,
        );
        let f = cost_fn.into_cost_fn();
        let obs = with_not_connected_and_intermediate();

        let cost = f(1.0, &obs, 2);
        insta::assert_yaml_snapshot!(CostResult {
            observations: obs,
            initial_cost: 1.0,
            path_index: 2,
            result_cost: cost
        });
        Ok(())
    }

    #[test]
    fn forward_last_edge_positive_with_connectivity_no_capacity() -> anyhow::Result<()> {
        let cost_fn = EdgeCostFn::<_, Observations>::forward(
            std::num::NonZeroUsize::new(3).context("should be non-zero")?,
            TEST_PENALTY,
            TEST_MIN_ACK_RATE,
        );
        let f = cost_fn.into_cost_fn();
        let obs = with_connected_only_immediate();

        let cost = f(1.0, &obs, 2);
        insta::assert_yaml_snapshot!(CostResult {
            observations: obs,
            initial_cost: 1.0,
            path_index: 2,
            result_cost: cost
        });
        Ok(())
    }

    #[test]
    fn forward_last_edge_scales_by_intermediate_score() -> anyhow::Result<()> {
        let cost_fn = EdgeCostFn::<_, Observations>::forward(
            std::num::NonZeroUsize::new(3).context("should be non-zero")?,
            TEST_PENALTY,
            TEST_MIN_ACK_RATE,
        );
        let f = cost_fn.into_cost_fn();
        let obs = with_connected_and_capacity();

        let cost = f(2.0, &obs, 2);
        insta::assert_yaml_snapshot!(CostResult {
            observations: obs,
            initial_cost: 2.0,
            path_index: 2,
            result_cost: cost
        });
        Ok(())
    }

    #[test]
    fn forward_last_edge_positive_when_intermediate_but_no_capacity() -> anyhow::Result<()> {
        let cost_fn = EdgeCostFn::<_, Observations>::forward(
            std::num::NonZeroUsize::new(3).context("should be non-zero")?,
            TEST_PENALTY,
            TEST_MIN_ACK_RATE,
        );
        let f = cost_fn.into_cost_fn();
        let obs = Observations {
            immediate: None,
            intermediate: Some(StubIntermediate {
                capacity: None,
                score: 0.95,
            }),
        };

        let cost = f(1.0, &obs, 2);
        insta::assert_yaml_snapshot!(CostResult {
            observations: obs,
            initial_cost: 1.0,
            path_index: 2,
            result_cost: cost
        });
        Ok(())
    }

    #[test]
    fn forward_last_edge_positive_when_empty() -> anyhow::Result<()> {
        let cost_fn = EdgeCostFn::<_, Observations>::forward(
            std::num::NonZeroUsize::new(3).context("should be non-zero")?,
            TEST_PENALTY,
            TEST_MIN_ACK_RATE,
        );
        let f = cost_fn.into_cost_fn();
        let obs = with_empty();

        let cost = f(1.0, &obs, 2);
        insta::assert_yaml_snapshot!(CostResult {
            observations: obs,
            initial_cost: 1.0,
            path_index: 2,
            result_cost: cost
        });
        Ok(())
    }

    // ── Forward intermediate edges (0 < path_index < length - 1) ────────

    #[test]
    fn forward_intermediate_edge_positive_when_capacity_and_score() -> anyhow::Result<()> {
        let cost_fn = EdgeCostFn::<_, Observations>::forward(
            std::num::NonZeroUsize::new(3).context("should be non-zero")?,
            TEST_PENALTY,
            TEST_MIN_ACK_RATE,
        );
        let f = cost_fn.into_cost_fn();
        let obs = with_connected_and_capacity();

        let cost = f(1.0, &obs, 1);
        insta::assert_yaml_snapshot!(CostResult {
            observations: obs,
            initial_cost: 1.0,
            path_index: 1,
            result_cost: cost
        });
        Ok(())
    }

    #[test]
    fn forward_intermediate_edge_scales_by_intermediate_score() -> anyhow::Result<()> {
        let cost_fn = EdgeCostFn::<_, Observations>::forward(
            std::num::NonZeroUsize::new(3).context("should be non-zero")?,
            TEST_PENALTY,
            TEST_MIN_ACK_RATE,
        );
        let f = cost_fn.into_cost_fn();
        let obs = with_connected_and_capacity();

        let cost = f(2.0, &obs, 1);
        insta::assert_yaml_snapshot!(CostResult {
            observations: obs,
            initial_cost: 2.0,
            path_index: 1,
            result_cost: cost
        });
        Ok(())
    }

    #[test]
    fn forward_intermediate_edge_negative_when_no_intermediate() -> anyhow::Result<()> {
        let cost_fn = EdgeCostFn::<_, Observations>::forward(
            std::num::NonZeroUsize::new(3).context("should be non-zero")?,
            TEST_PENALTY,
            TEST_MIN_ACK_RATE,
        );
        let f = cost_fn.into_cost_fn();
        let obs = with_connected_only_immediate();

        let cost = f(1.0, &obs, 1);
        insta::assert_yaml_snapshot!(CostResult {
            observations: obs,
            initial_cost: 1.0,
            path_index: 1,
            result_cost: cost
        });
        Ok(())
    }

    #[test]
    fn forward_intermediate_edge_negative_when_no_capacity() -> anyhow::Result<()> {
        let cost_fn = EdgeCostFn::<_, Observations>::forward(
            std::num::NonZeroUsize::new(3).context("should be non-zero")?,
            TEST_PENALTY,
            TEST_MIN_ACK_RATE,
        );
        let f = cost_fn.into_cost_fn();
        let obs = Observations {
            immediate: None,
            intermediate: Some(StubIntermediate {
                capacity: None,
                score: 0.95,
            }),
        };

        let cost = f(1.0, &obs, 1);
        insta::assert_yaml_snapshot!(CostResult {
            observations: obs,
            initial_cost: 1.0,
            path_index: 1,
            result_cost: cost
        });
        Ok(())
    }

    #[test]
    fn forward_intermediate_edge_positive_when_capacity_only_no_probes() -> anyhow::Result<()> {
        let cost_fn = EdgeCostFn::<_, Observations>::forward(
            std::num::NonZeroUsize::new(3).context("should be non-zero")?,
            TEST_PENALTY,
            TEST_MIN_ACK_RATE,
        );
        let f = cost_fn.into_cost_fn();
        let obs = with_capacity_only();

        let cost = f(1.0, &obs, 1);
        insta::assert_yaml_snapshot!(CostResult {
            observations: obs,
            initial_cost: 1.0,
            path_index: 1,
            result_cost: cost
        });
        Ok(())
    }

    #[test]
    fn forward_intermediate_edge_negative_when_empty() -> anyhow::Result<()> {
        let cost_fn = EdgeCostFn::<_, Observations>::forward(
            std::num::NonZeroUsize::new(3).context("should be non-zero")?,
            TEST_PENALTY,
            TEST_MIN_ACK_RATE,
        );
        let f = cost_fn.into_cost_fn();
        let obs = with_empty();

        let cost = f(1.0, &obs, 1);
        insta::assert_yaml_snapshot!(CostResult {
            observations: obs,
            initial_cost: 1.0,
            path_index: 1,
            result_cost: cost
        });
        Ok(())
    }

    #[test]
    fn forward_intermediate_edge_uses_observations() -> anyhow::Result<()> {
        let cost_fn = EdgeCostFn::<_, Observations>::forward(
            std::num::NonZeroUsize::new(3).context("should be non-zero")?,
            TEST_PENALTY,
            TEST_MIN_ACK_RATE,
        );
        let f = cost_fn.into_cost_fn();

        let cost_empty = f(1.0, &with_empty(), 1);
        let cost_full = f(1.0, &with_connected_and_capacity(), 1);
        assert_ne!(cost_empty, cost_full, "intermediate edges should use observations");
        Ok(())
    }

    // ── Forward length boundary tests ───────────────────────────────────

    #[test]
    fn forward_length_one_has_only_first_and_last_edge() -> anyhow::Result<()> {
        let cost_fn = EdgeCostFn::<_, Observations>::forward(
            std::num::NonZeroUsize::new(1).context("should be non-zero")?,
            TEST_PENALTY,
            TEST_MIN_ACK_RATE,
        );
        let f = cost_fn.into_cost_fn();
        let obs = with_connected_and_capacity();

        let cost = f(1.0, &obs, 0);
        insta::assert_yaml_snapshot!(CostResult {
            observations: obs,
            initial_cost: 1.0,
            path_index: 0,
            result_cost: cost
        });
        Ok(())
    }

    #[test]
    fn forward_length_one_rejected_when_ack_rate_below_threshold() -> anyhow::Result<()> {
        let cost_fn = EdgeCostFn::<_, Observations>::forward(
            std::num::NonZeroUsize::new(1).context("should be non-zero")?,
            TEST_PENALTY,
            TEST_MIN_ACK_RATE,
        );
        let f = cost_fn.into_cost_fn();
        let obs = Observations {
            immediate: Some(StubImmediate {
                connected: true,
                score: 0.95,
                ack_rate: Some(0.05),
            }),
            intermediate: Some(StubIntermediate {
                capacity: Some(1000),
                score: 0.95,
            }),
        };

        let cost = f(1.0, &obs, 0);
        insta::assert_yaml_snapshot!(CostResult {
            observations: obs,
            initial_cost: 1.0,
            path_index: 0,
            result_cost: cost
        });
        Ok(())
    }

    #[test]
    fn forward_length_two_intermediate_at_index_one() -> anyhow::Result<()> {
        let cost_fn = EdgeCostFn::<_, Observations>::forward(
            std::num::NonZeroUsize::new(2).context("should be non-zero")?,
            TEST_PENALTY,
            TEST_MIN_ACK_RATE,
        );
        let f = cost_fn.into_cost_fn();
        let obs = with_connected_and_capacity();

        let cost = f(1.0, &obs, 1);
        insta::assert_yaml_snapshot!(CostResult {
            observations: obs,
            initial_cost: 1.0,
            path_index: 1,
            result_cost: cost
        });

        let obs_e = with_empty();
        let cost_empty = f(1.0, &obs_e, 1);
        insta::assert_yaml_snapshot!(CostResult {
            observations: obs_e,
            initial_cost: 1.0,
            path_index: 1,
            result_cost: cost_empty
        });
        Ok(())
    }

    // ── Forward negative initial cost propagation ───────────────────────

    #[test]
    fn forward_negative_initial_cost_inverts_rejection() -> anyhow::Result<()> {
        let cost_fn = EdgeCostFn::<_, Observations>::forward(
            std::num::NonZeroUsize::new(3).context("should be non-zero")?,
            TEST_PENALTY,
            TEST_MIN_ACK_RATE,
        );
        let f = cost_fn.into_cost_fn();
        let obs = with_empty();

        let cost = f(-1.0, &obs, 0);
        insta::assert_yaml_snapshot!(CostResult {
            observations: obs,
            initial_cost: -1.0,
            path_index: 0,
            result_cost: cost
        });
        Ok(())
    }

    // ── Return cost function trait method tests ──────────────────────────

    #[test]
    fn return_cost_fn_invariants() -> anyhow::Result<()> {
        let cost_fn = EdgeCostFn::<_, Observations>::returning(
            std::num::NonZeroUsize::new(3).context("should be non-zero")?,
            TEST_PENALTY,
            TEST_MIN_ACK_RATE,
        );
        #[derive(serde::Serialize)]
        struct Invariants {
            initial_cost: f64,
            min_cost: Option<f64>,
        }
        insta::assert_yaml_snapshot!(Invariants {
            initial_cost: cost_fn.initial_cost(),
            min_cost: cost_fn.min_cost(),
        });
        Ok(())
    }

    // ── Return first edge (path_index == 0) ─────────────────────────────

    #[test]
    fn return_first_edge_positive_with_intermediate_and_capacity() -> anyhow::Result<()> {
        let cost_fn = EdgeCostFn::<_, Observations>::returning(
            std::num::NonZeroUsize::new(2).context("should be non-zero")?,
            TEST_PENALTY,
            TEST_MIN_ACK_RATE,
        );
        let f = cost_fn.into_cost_fn();
        let obs = with_not_connected_and_intermediate();

        let cost = f(1.0, &obs, 0);
        insta::assert_yaml_snapshot!(CostResult {
            observations: obs,
            initial_cost: 1.0,
            path_index: 0,
            result_cost: cost
        });
        Ok(())
    }

    #[test]
    fn return_first_edge_positive_with_full_data() -> anyhow::Result<()> {
        let cost_fn = EdgeCostFn::<_, Observations>::returning(
            std::num::NonZeroUsize::new(2).context("should be non-zero")?,
            TEST_PENALTY,
            TEST_MIN_ACK_RATE,
        );
        let f = cost_fn.into_cost_fn();
        let obs = with_connected_and_capacity();

        let cost = f(1.0, &obs, 0);
        insta::assert_yaml_snapshot!(CostResult {
            observations: obs,
            initial_cost: 1.0,
            path_index: 0,
            result_cost: cost
        });
        Ok(())
    }

    #[test]
    fn return_first_edge_scales_by_intermediate_score() -> anyhow::Result<()> {
        let cost_fn = EdgeCostFn::<_, Observations>::returning(
            std::num::NonZeroUsize::new(2).context("should be non-zero")?,
            TEST_PENALTY,
            TEST_MIN_ACK_RATE,
        );
        let f = cost_fn.into_cost_fn();
        let obs = with_not_connected_and_intermediate();

        let cost = f(2.0, &obs, 0);
        insta::assert_yaml_snapshot!(CostResult {
            observations: obs,
            initial_cost: 2.0,
            path_index: 0,
            result_cost: cost
        });
        Ok(())
    }

    #[test]
    fn return_first_edge_does_not_require_connectivity() -> anyhow::Result<()> {
        let cost_fn = EdgeCostFn::<_, Observations>::returning(
            std::num::NonZeroUsize::new(2).context("should be non-zero")?,
            TEST_PENALTY,
            TEST_MIN_ACK_RATE,
        );
        let f = cost_fn.into_cost_fn();
        let obs = with_not_connected_and_intermediate();

        let cost = f(1.0, &obs, 0);
        insta::assert_yaml_snapshot!(CostResult {
            observations: obs,
            initial_cost: 1.0,
            path_index: 0,
            result_cost: cost
        });
        Ok(())
    }

    #[test]
    fn return_first_edge_positive_when_capacity_only_no_probes() -> anyhow::Result<()> {
        let cost_fn = EdgeCostFn::<_, Observations>::returning(
            std::num::NonZeroUsize::new(2).context("should be non-zero")?,
            TEST_PENALTY,
            TEST_MIN_ACK_RATE,
        );
        let f = cost_fn.into_cost_fn();
        let obs = with_capacity_only();

        let cost = f(1.0, &obs, 0);
        insta::assert_yaml_snapshot!(CostResult {
            observations: obs,
            initial_cost: 1.0,
            path_index: 0,
            result_cost: cost
        });
        Ok(())
    }

    #[test]
    fn return_first_edge_negative_when_no_capacity() -> anyhow::Result<()> {
        let cost_fn = EdgeCostFn::<_, Observations>::returning(
            std::num::NonZeroUsize::new(2).context("should be non-zero")?,
            TEST_PENALTY,
            TEST_MIN_ACK_RATE,
        );
        let f = cost_fn.into_cost_fn();
        let obs = with_connected_only_immediate();

        let cost = f(1.0, &obs, 0);
        insta::assert_yaml_snapshot!(CostResult {
            observations: obs,
            initial_cost: 1.0,
            path_index: 0,
            result_cost: cost
        });
        Ok(())
    }

    #[test]
    fn return_first_edge_negative_when_empty() -> anyhow::Result<()> {
        let cost_fn = EdgeCostFn::<_, Observations>::returning(
            std::num::NonZeroUsize::new(2).context("should be non-zero")?,
            TEST_PENALTY,
            TEST_MIN_ACK_RATE,
        );
        let f = cost_fn.into_cost_fn();
        let obs = with_empty();

        let cost = f(1.0, &obs, 0);
        insta::assert_yaml_snapshot!(CostResult {
            observations: obs,
            initial_cost: 1.0,
            path_index: 0,
            result_cost: cost
        });
        Ok(())
    }

    // ── Return last edge ────────────────────────────────────────────────

    #[test]
    fn return_last_edge_requires_connectivity() -> anyhow::Result<()> {
        let length = std::num::NonZeroUsize::new(2).context("should be non-zero")?;
        let ret = EdgeCostFn::<_, Observations>::returning(length, TEST_PENALTY, TEST_MIN_ACK_RATE);
        let ret_fn = ret.into_cost_fn();

        let obs_conn = with_connected_and_capacity();
        let cost_connected = ret_fn(1.0, &obs_conn, 1);
        insta::assert_yaml_snapshot!(CostResult {
            observations: obs_conn,
            initial_cost: 1.0,
            path_index: 1,
            result_cost: cost_connected
        });

        let obs_no_conn = with_not_connected_and_intermediate();
        let cost_not_connected = ret_fn(1.0, &obs_no_conn, 1);
        insta::assert_yaml_snapshot!(CostResult {
            observations: obs_no_conn,
            initial_cost: 1.0,
            path_index: 1,
            result_cost: cost_not_connected
        });

        Ok(())
    }

    #[test]
    fn return_last_edge_positive_when_connected_with_empty_intermediate() -> anyhow::Result<()> {
        let length = std::num::NonZeroUsize::new(2).context("should be non-zero")?;
        let ret = EdgeCostFn::<_, Observations>::returning(length, TEST_PENALTY, TEST_MIN_ACK_RATE);
        let ret_fn = ret.into_cost_fn();

        let obs = Observations {
            immediate: Some(StubImmediate {
                connected: true,
                score: 0.95,
                ack_rate: Some(0.9),
            }),
            intermediate: Some(StubIntermediate {
                capacity: None,
                score: 0.0,
            }),
        };

        let cost = ret_fn(1.0, &obs, 1);
        insta::assert_yaml_snapshot!(CostResult {
            observations: obs,
            initial_cost: 1.0,
            path_index: 1,
            result_cost: cost
        });

        Ok(())
    }

    #[test]
    fn return_last_edge_penalized_when_connected_but_zero_score() -> anyhow::Result<()> {
        let length = std::num::NonZeroUsize::new(2).context("should be non-zero")?;
        let ret = EdgeCostFn::<_, Observations>::returning(length, TEST_PENALTY, TEST_MIN_ACK_RATE);
        let ret_fn = ret.into_cost_fn();

        let obs = Observations {
            immediate: Some(StubImmediate {
                connected: true,
                score: 0.0,
                ack_rate: Some(0.9),
            }),
            intermediate: None,
        };

        let cost = ret_fn(1.0, &obs, 1);
        insta::assert_yaml_snapshot!(CostResult {
            observations: obs,
            initial_cost: 1.0,
            path_index: 1,
            result_cost: cost
        });

        Ok(())
    }

    #[test]
    fn forward_last_edge_differs_from_return_last_edge() -> anyhow::Result<()> {
        let length = std::num::NonZeroUsize::new(2).context("should be non-zero")?;

        let fwd = EdgeCostFn::<_, Observations>::forward(length, TEST_PENALTY, TEST_MIN_ACK_RATE);
        let ret = EdgeCostFn::<_, Observations>::returning(length, TEST_PENALTY, TEST_MIN_ACK_RATE);
        let fwd_fn = fwd.into_cost_fn();
        let ret_fn = ret.into_cost_fn();

        let obs = with_not_connected_and_intermediate();
        let fwd_cost = fwd_fn(1.0, &obs, 1);
        let ret_cost = ret_fn(1.0, &obs, 1);

        #[derive(serde::Serialize)]
        struct Comparison {
            observations: Observations,
            forward_last_edge_cost: f64,
            return_last_edge_cost: f64,
        }

        insta::assert_yaml_snapshot!(Comparison {
            observations: obs,
            forward_last_edge_cost: fwd_cost,
            return_last_edge_cost: ret_cost,
        });

        Ok(())
    }

    // ── Return intermediate edge ────────────────────────────────────────

    #[test]
    fn return_intermediate_edge_positive_when_capacity_only_no_probes() -> anyhow::Result<()> {
        let cost_fn = EdgeCostFn::<_, Observations>::returning(
            std::num::NonZeroUsize::new(3).context("should be non-zero")?,
            TEST_PENALTY,
            TEST_MIN_ACK_RATE,
        );
        let f = cost_fn.into_cost_fn();
        let obs = with_capacity_only();

        let cost = f(1.0, &obs, 1);
        insta::assert_yaml_snapshot!(CostResult {
            observations: obs,
            initial_cost: 1.0,
            path_index: 1,
            result_cost: cost
        });
        Ok(())
    }

    #[test]
    fn return_intermediate_edge_same_as_forward() -> anyhow::Result<()> {
        let length = std::num::NonZeroUsize::new(3).context("should be non-zero")?;

        let fwd = EdgeCostFn::<_, Observations>::forward(length, TEST_PENALTY, TEST_MIN_ACK_RATE);
        let ret = EdgeCostFn::<_, Observations>::returning(length, TEST_PENALTY, TEST_MIN_ACK_RATE);
        let fwd_fn = fwd.into_cost_fn();
        let ret_fn = ret.into_cost_fn();

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
    fn symmetrical_forward_without_self_loopback_works_with_forward_cost_fn() -> anyhow::Result<()> {
        let length = std::num::NonZeroUsize::new(2).context("should be non-zero")?;
        let cost_fn = EdgeCostFn::<_, Observations>::forward(length, TEST_PENALTY, TEST_MIN_ACK_RATE);
        let f = cost_fn.into_cost_fn();

        let me_to_relay = with_connected_and_capacity();
        let relay_to_dest = with_capacity_only();

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
    fn symmetrical_return_path_rejected_by_forward_cost_fn() -> anyhow::Result<()> {
        let length = std::num::NonZeroUsize::new(2).context("should be non-zero")?;
        let cost_fn = EdgeCostFn::<_, Observations>::forward(length, TEST_PENALTY, TEST_MIN_ACK_RATE);
        let f = cost_fn.into_cost_fn();

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
    fn symmetrical_return_path_works_with_return_cost_fn() -> anyhow::Result<()> {
        let length = std::num::NonZeroUsize::new(2).context("should be non-zero")?;
        let cost_fn = EdgeCostFn::<_, Observations>::returning(length, TEST_PENALTY, TEST_MIN_ACK_RATE);
        let f = cost_fn.into_cost_fn();

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

        let fwd = EdgeCostFn::<_, Observations>::forward(length, TEST_PENALTY, TEST_MIN_ACK_RATE);
        let fwd_fn = fwd.into_cost_fn();

        let me_to_relay = with_connected_and_capacity();
        let relay_to_dest = with_capacity_only();

        let fwd_cost = fwd_fn(1.0, &me_to_relay, 0);
        let fwd_cost = fwd_fn(fwd_cost, &relay_to_dest, 1);

        let ret = EdgeCostFn::<_, Observations>::returning(length, TEST_PENALTY, TEST_MIN_ACK_RATE);
        let ret_fn = ret.into_cost_fn();

        let dest_to_relay = with_capacity_only();
        let relay_to_me = with_connected_and_capacity();

        let ret_cost = ret_fn(1.0, &dest_to_relay, 0);
        let ret_cost = ret_fn(ret_cost, &relay_to_me, 1);

        #[derive(serde::Serialize)]
        struct BidirectionalCost {
            forward_without_self_loopback_cost: f64,
            return_path_cost: f64,
        }

        insta::assert_yaml_snapshot!(BidirectionalCost {
            forward_without_self_loopback_cost: fwd_cost,
            return_path_cost: ret_cost,
        });

        Ok(())
    }

    // ── Ack rate cost function tests ─────────────────────────────────

    #[test]
    fn forward_first_edge_rejected_when_ack_rate_below_threshold() -> anyhow::Result<()> {
        let cost_fn = EdgeCostFn::<_, Observations>::forward(
            std::num::NonZeroUsize::new(3).context("should be non-zero")?,
            TEST_PENALTY,
            TEST_MIN_ACK_RATE,
        );
        let f = cost_fn.into_cost_fn();
        let obs = Observations {
            immediate: Some(StubImmediate {
                connected: true,
                score: 0.95,
                ack_rate: Some(0.05),
            }),
            intermediate: Some(StubIntermediate {
                capacity: Some(1000),
                score: 0.95,
            }),
        };

        let cost = f(1.0, &obs, 0);
        insta::assert_yaml_snapshot!(CostResult {
            observations: obs,
            initial_cost: 1.0,
            path_index: 0,
            result_cost: cost
        });
        Ok(())
    }

    #[test]
    fn forward_first_edge_penalized_when_no_ack_data() -> anyhow::Result<()> {
        let cost_fn = EdgeCostFn::<_, Observations>::forward(
            std::num::NonZeroUsize::new(3).context("should be non-zero")?,
            TEST_PENALTY,
            TEST_MIN_ACK_RATE,
        );
        let f = cost_fn.into_cost_fn();
        let obs = Observations {
            immediate: Some(StubImmediate {
                connected: true,
                score: 0.95,
                ack_rate: None,
            }),
            intermediate: Some(StubIntermediate {
                capacity: Some(1000),
                score: 0.95,
            }),
        };

        let cost = f(1.0, &obs, 0);
        insta::assert_yaml_snapshot!(CostResult {
            observations: obs,
            initial_cost: 1.0,
            path_index: 0,
            result_cost: cost
        });
        Ok(())
    }

    #[test]
    fn forward_first_edge_scales_by_ack_rate() -> anyhow::Result<()> {
        let cost_fn = EdgeCostFn::<_, Observations>::forward(
            std::num::NonZeroUsize::new(3).context("should be non-zero")?,
            TEST_PENALTY,
            TEST_MIN_ACK_RATE,
        );
        let f = cost_fn.into_cost_fn();
        let obs_high = Observations {
            immediate: Some(StubImmediate {
                connected: true,
                score: 0.95,
                ack_rate: Some(0.9),
            }),
            intermediate: Some(StubIntermediate {
                capacity: Some(1000),
                score: 0.95,
            }),
        };
        let obs_low = Observations {
            immediate: Some(StubImmediate {
                connected: true,
                score: 0.95,
                ack_rate: Some(0.3),
            }),
            intermediate: Some(StubIntermediate {
                capacity: Some(1000),
                score: 0.95,
            }),
        };

        let cost_high = f(1.0, &obs_high, 0);
        let cost_low = f(1.0, &obs_low, 0);

        assert!(
            cost_high > cost_low,
            "higher ack rate should produce higher cost: {cost_high} vs {cost_low}"
        );
        Ok(())
    }

    #[test]
    fn return_last_edge_rejected_when_ack_rate_below_threshold() -> anyhow::Result<()> {
        let cost_fn = EdgeCostFn::<_, Observations>::returning(
            std::num::NonZeroUsize::new(2).context("should be non-zero")?,
            TEST_PENALTY,
            TEST_MIN_ACK_RATE,
        );
        let f = cost_fn.into_cost_fn();
        let obs = Observations {
            immediate: Some(StubImmediate {
                connected: true,
                score: 0.95,
                ack_rate: Some(0.05),
            }),
            intermediate: Some(StubIntermediate {
                capacity: Some(1000),
                score: 0.95,
            }),
        };

        let cost = f(1.0, &obs, 1);
        insta::assert_yaml_snapshot!(CostResult {
            observations: obs,
            initial_cost: 1.0,
            path_index: 1,
            result_cost: cost
        });
        Ok(())
    }

    #[test]
    fn adversarial_peer_good_probes_but_zero_acks() -> anyhow::Result<()> {
        let cost_fn = EdgeCostFn::<_, Observations>::forward(
            std::num::NonZeroUsize::new(3).context("should be non-zero")?,
            TEST_PENALTY,
            TEST_MIN_ACK_RATE,
        );
        let f = cost_fn.into_cost_fn();
        let obs = Observations {
            immediate: Some(StubImmediate {
                connected: true,
                score: 0.95,
                ack_rate: Some(0.0),
            }),
            intermediate: Some(StubIntermediate {
                capacity: Some(1000),
                score: 0.95,
            }),
        };

        let cost = f(1.0, &obs, 0);
        insta::assert_yaml_snapshot!(CostResult {
            observations: obs,
            initial_cost: 1.0,
            path_index: 0,
            result_cost: cost
        });
        Ok(())
    }

    #[test]
    fn forward_without_loopback_first_edge_rejected_when_ack_rate_below_threshold() -> anyhow::Result<()> {
        let cost_fn = EdgeCostFn::<_, Observations>::forward_without_self_loopback(TEST_PENALTY, TEST_MIN_ACK_RATE);
        let f = cost_fn.into_cost_fn();
        let obs = Observations {
            immediate: Some(StubImmediate {
                connected: true,
                score: 0.95,
                ack_rate: Some(0.05),
            }),
            intermediate: Some(StubIntermediate {
                capacity: Some(1000),
                score: 0.95,
            }),
        };

        let cost = f(1.0, &obs, 0);
        insta::assert_yaml_snapshot!(CostResult {
            observations: obs,
            initial_cost: 1.0,
            path_index: 0,
            result_cost: cost
        });
        Ok(())
    }
}
