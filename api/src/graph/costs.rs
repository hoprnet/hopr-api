use super::traits::{
    CostFn, EdgeLinkObservable, EdgeNetworkObservableRead, EdgeObservableRead, EdgeProtocolObservable,
};

/// A boxed cost function accepting `(current_cost, edge_weight, path_index) -> new_cost`.
pub type BasicCostFn<C, W> = Box<dyn Fn(C, &W, usize) -> C>;

/// Build a forward HOPR cost function for full graph traversals.
pub struct HoprForwardCostFn<C, W> {
    initial: C,
    min: Option<C>,
    cost_fn: BasicCostFn<C, W>,
}

impl<C, W> CostFn for HoprForwardCostFn<C, W>
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

    fn into_cost_fn(self) -> Box<dyn Fn(Self::Cost, &Self::Weight, usize) -> Self::Cost> {
        self.cost_fn
    }
}

impl<W> HoprForwardCostFn<f64, W>
where
    W: EdgeObservableRead + Send + 'static,
{
    pub fn new(length: std::num::NonZeroUsize) -> Self {
        let length = length.get();
        Self {
            initial: 1.0,
            min: Some(0.0),
            cost_fn: Box::new(move |initial_cost: f64, observation: &W, path_index: usize| {
                match path_index {
                    0 => {
                        // the first edge should always go to an already connected and measured peer,
                        // otherwise use a negative cost that should remove the edge from consideration
                        if let Some(immediate_observation) = observation.immediate_qos()
                            && immediate_observation.is_connected()
                            && let Some(intermediate_observation) = observation.intermediate_qos()
                            && intermediate_observation.capacity().is_some()
                        {
                            // loopbacks through a single peer are forbidden, therefore the first edge
                            // may consider the preexisting measurements over an immediate observation
                            return initial_cost * immediate_observation.score().max(intermediate_observation.score());
                        }

                        -initial_cost
                    }
                    v if v == (length - 1) => {
                        // The last edge (relay -> dest) may lack immediate QoS in me's graph
                        // because me doesn't directly observe relay-to-dest connectivity.
                        // Accept capacity (on-chain channel) OR connectivity + score.
                        if let Some(intermediate_observation) = observation.intermediate_qos()
                            && intermediate_observation.capacity().is_some()
                        {
                            let score = intermediate_observation.score();
                            return if score > 0.0 {
                                initial_cost * score
                            } else {
                                initial_cost
                            };
                        }

                        initial_cost
                    }
                    _ => {
                        // Intermediary edges need capacity. When probes exist, scale
                        // by score; otherwise pass through initial_cost as baseline
                        // trust (capacity-only from on-chain, probes not yet run).
                        if let Some(intermediate_observation) = observation.intermediate_qos()
                            && intermediate_observation.capacity().is_some()
                        {
                            let score = intermediate_observation.score();
                            return if score > 0.0 {
                                initial_cost * score
                            } else {
                                initial_cost
                            };
                        }

                        -initial_cost
                    }
                }
            }),
        }
    }
}

/// Build a HOPR cost function for full graph traversals in the return direction.
///
/// Used when the planner (`me`) constructs the return path `dest -> relay -> me`.
/// The first edge (`dest -> relay`) has relaxed requirements compared to [`HoprForwardCostFn`]
/// because the planner lacks intermediate QoS (probe) data for that edge.
///
/// Only payment channel capacity is required for the first edge. If probe-based QoS with a
/// positive score is available, that score is used to scale the edge cost; otherwise the
/// initial cost is effectively passed through without score-based scaling.
pub struct HoprReturnCostFn<C, W> {
    initial: C,
    min: Option<C>,
    cost_fn: BasicCostFn<C, W>,
}

impl<C, W> CostFn for HoprReturnCostFn<C, W>
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

    fn into_cost_fn(self) -> Box<dyn Fn(Self::Cost, &Self::Weight, usize) -> Self::Cost> {
        self.cost_fn
    }
}

impl<W> HoprReturnCostFn<f64, W>
where
    W: EdgeObservableRead + Send + 'static,
{
    pub fn new(length: std::num::NonZeroUsize) -> Self {
        let length = length.get();
        Self {
            initial: 1.0,
            min: Some(0.0),
            cost_fn: Box::new(move |initial_cost: f64, observation: &W, path_index: usize| {
                match path_index {
                    0 => {
                        // The first edge of the return path (dest -> relay) requires
                        // payment channel capacity.
                        // When probes exist, scale by score; otherwise pass through
                        // the cost as baseline trust (capacity-only from on-chain).
                        if let Some(intermediate_observation) = observation.intermediate_qos()
                            && intermediate_observation.capacity().is_some()
                        {
                            let score = intermediate_observation.score();
                            return if score > 0.0 {
                                initial_cost * score
                            } else {
                                initial_cost
                            };
                        }

                        -initial_cost
                    }
                    v if v == (length - 1) => {
                        // The last edge of the return path (relay -> me) requires connectivity.
                        // Use the immediate observation score since me has direct measurement data
                        // for this edge; Observations::score() may return 0 if an empty
                        // intermediate_probe record exists and shadows the immediate data.
                        if let Some(immediate_observation) = observation.immediate_qos()
                            && immediate_observation.is_connected()
                        {
                            let score = immediate_observation.score();
                            return if score > 0.0 {
                                initial_cost * score
                            } else {
                                initial_cost
                            };
                        }

                        -initial_cost
                    }
                    _ => {
                        // Intermediary edges need capacity. When probes exist, scale
                        // by score; otherwise pass through initial_cost as baseline
                        // trust (capacity-only from on-chain, probes not yet run).
                        if let Some(intermediate_observation) = observation.intermediate_qos()
                            && intermediate_observation.capacity().is_some()
                        {
                            let score = intermediate_observation.score();
                            return if score > 0.0 {
                                initial_cost * score
                            } else {
                                initial_cost
                            };
                        }

                        -initial_cost
                    }
                }
            }),
        }
    }
}

/// Used for finding simple paths without the final loopback in a loopback call.
pub struct ForwardPathCostFn<C, W> {
    initial: C,
    min: Option<C>,
    cost_fn: BasicCostFn<C, W>,
}

impl<W> Default for ForwardPathCostFn<f64, W>
where
    W: EdgeObservableRead + Send + 'static,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<C, W> CostFn for ForwardPathCostFn<C, W>
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

    fn into_cost_fn(self) -> Box<dyn Fn(Self::Cost, &Self::Weight, usize) -> Self::Cost> {
        self.cost_fn
    }
}

impl<W> ForwardPathCostFn<f64, W>
where
    W: EdgeObservableRead + Send + 'static,
{
    pub fn new() -> Self {
        Self {
            initial: 1.0,
            min: Some(0.0),
            cost_fn: Box::new(move |initial_cost: f64, observation: &W, path_index: usize| {
                match path_index {
                    0 => {
                        // the first edge should always go to an already connected and measured peer,
                        // otherwise use a negative cost that should remove the edge from consideration
                        if let Some(immediate_observation) = observation.immediate_qos()
                            && immediate_observation.is_connected()
                            && let Some(intermediate_observation) = observation.intermediate_qos()
                            && intermediate_observation.capacity().is_some()
                        {
                            // loopbacks through a single peer are forbidden, therefore the first edge
                            // may consider the preexisting measurements over an immediate observation
                            return initial_cost * immediate_observation.score().max(intermediate_observation.score());
                        }

                        -initial_cost
                    }
                    _ => {
                        // intermediary edges only need to have capacity and score.
                        // When capacity exists but no probes have run yet (score 0), pass through
                        // initial_cost to allow the first probe to discover this path.
                        if let Some(intermediate_observation) = observation.intermediate_qos()
                            && intermediate_observation.capacity().is_some()
                        {
                            let score = intermediate_observation.score();
                            return if score > 0.0 {
                                initial_cost * score
                            } else {
                                initial_cost
                            };
                        }

                        -initial_cost
                    }
                }
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Context;

    use super::*;
    use crate::graph::traits::{
        EdgeLinkObservable, EdgeNetworkObservableRead, EdgeObservableRead, EdgeProtocolObservable,
        EdgeTransportMeasurement,
    };

    // ── Serializable stub types (pure value holders) ─────────────────────

    /// Stub for immediate (1-hop) probe measurement.
    #[derive(Debug, Default, Clone, serde::Serialize)]
    struct StubImmediate {
        connected: bool,
        score: f64,
    }

    impl EdgeNetworkObservableRead for StubImmediate {
        fn is_connected(&self) -> bool {
            self.connected
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

    // ── HoprForwardCostFn trait method tests ─────────────────────────────

    #[test]
    fn forward_cost_fn_invariants() -> anyhow::Result<()> {
        let cost_fn =
            HoprForwardCostFn::<_, Observations>::new(std::num::NonZeroUsize::new(3).context("should be non-zero")?);
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
        let cost_fn =
            HoprForwardCostFn::<_, Observations>::new(std::num::NonZeroUsize::new(3).context("should be non-zero")?);
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
        let cost_fn =
            HoprForwardCostFn::<_, Observations>::new(std::num::NonZeroUsize::new(3).context("should be non-zero")?);
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
        let cost_fn =
            HoprForwardCostFn::<_, Observations>::new(std::num::NonZeroUsize::new(3).context("should be non-zero")?);
        let f = cost_fn.into_cost_fn();
        let obs = Observations {
            immediate: Some(StubImmediate {
                connected: true,
                score: 0.95,
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
        let cost_fn =
            HoprForwardCostFn::<_, Observations>::new(std::num::NonZeroUsize::new(3).context("should be non-zero")?);
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
        let cost_fn =
            HoprForwardCostFn::<_, Observations>::new(std::num::NonZeroUsize::new(3).context("should be non-zero")?);
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
        let cost_fn =
            HoprForwardCostFn::<_, Observations>::new(std::num::NonZeroUsize::new(3).context("should be non-zero")?);
        let f = cost_fn.into_cost_fn();
        let obs = Observations {
            immediate: Some(StubImmediate {
                connected: true,
                score: 0.95,
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
        let cost_fn =
            HoprForwardCostFn::<_, Observations>::new(std::num::NonZeroUsize::new(3).context("should be non-zero")?);
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
        let cost_fn =
            HoprForwardCostFn::<_, Observations>::new(std::num::NonZeroUsize::new(3).context("should be non-zero")?);
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
        let cost_fn =
            HoprForwardCostFn::<_, Observations>::new(std::num::NonZeroUsize::new(3).context("should be non-zero")?);
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
        let cost_fn =
            HoprForwardCostFn::<_, Observations>::new(std::num::NonZeroUsize::new(3).context("should be non-zero")?);
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
        let cost_fn =
            HoprForwardCostFn::<_, Observations>::new(std::num::NonZeroUsize::new(3).context("should be non-zero")?);
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
        let cost_fn =
            HoprForwardCostFn::<_, Observations>::new(std::num::NonZeroUsize::new(3).context("should be non-zero")?);
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
        let cost_fn =
            HoprForwardCostFn::<_, Observations>::new(std::num::NonZeroUsize::new(3).context("should be non-zero")?);
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
        let cost_fn =
            HoprForwardCostFn::<_, Observations>::new(std::num::NonZeroUsize::new(3).context("should be non-zero")?);
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
        let cost_fn =
            HoprForwardCostFn::<_, Observations>::new(std::num::NonZeroUsize::new(3).context("should be non-zero")?);
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
        let cost_fn =
            HoprForwardCostFn::<_, Observations>::new(std::num::NonZeroUsize::new(3).context("should be non-zero")?);
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
        let cost_fn =
            HoprForwardCostFn::<_, Observations>::new(std::num::NonZeroUsize::new(3).context("should be non-zero")?);
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
        let cost_fn =
            HoprForwardCostFn::<_, Observations>::new(std::num::NonZeroUsize::new(3).context("should be non-zero")?);
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
        let cost_fn =
            HoprForwardCostFn::<_, Observations>::new(std::num::NonZeroUsize::new(3).context("should be non-zero")?);
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
        let cost_fn =
            HoprForwardCostFn::<_, Observations>::new(std::num::NonZeroUsize::new(3).context("should be non-zero")?);
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
        let cost_fn =
            HoprForwardCostFn::<_, Observations>::new(std::num::NonZeroUsize::new(3).context("should be non-zero")?);
        let f = cost_fn.into_cost_fn();

        let cost_empty = f(1.0, &with_empty(), 1);
        let cost_full = f(1.0, &with_connected_and_capacity(), 1);
        assert_ne!(cost_empty, cost_full, "intermediate edges should use observations");
        Ok(())
    }

    // ── Forward length boundary tests ───────────────────────────────────

    #[test]
    fn forward_length_one_has_only_first_and_last_edge() -> anyhow::Result<()> {
        let cost_fn =
            HoprForwardCostFn::<_, Observations>::new(std::num::NonZeroUsize::new(1).context("should be non-zero")?);
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
    fn forward_length_two_intermediate_at_index_one() -> anyhow::Result<()> {
        let cost_fn =
            HoprForwardCostFn::<_, Observations>::new(std::num::NonZeroUsize::new(2).context("should be non-zero")?);
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
        let cost_fn =
            HoprForwardCostFn::<_, Observations>::new(std::num::NonZeroUsize::new(3).context("should be non-zero")?);
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

    // ── HoprReturnCostFn trait method tests ──────────────────────────────

    #[test]
    fn return_cost_fn_invariants() -> anyhow::Result<()> {
        let cost_fn =
            HoprReturnCostFn::<_, Observations>::new(std::num::NonZeroUsize::new(3).context("should be non-zero")?);
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
        let cost_fn =
            HoprReturnCostFn::<_, Observations>::new(std::num::NonZeroUsize::new(2).context("should be non-zero")?);
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
        let cost_fn =
            HoprReturnCostFn::<_, Observations>::new(std::num::NonZeroUsize::new(2).context("should be non-zero")?);
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
        let cost_fn =
            HoprReturnCostFn::<_, Observations>::new(std::num::NonZeroUsize::new(2).context("should be non-zero")?);
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
        let cost_fn =
            HoprReturnCostFn::<_, Observations>::new(std::num::NonZeroUsize::new(2).context("should be non-zero")?);
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
        let cost_fn =
            HoprReturnCostFn::<_, Observations>::new(std::num::NonZeroUsize::new(2).context("should be non-zero")?);
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
        let cost_fn =
            HoprReturnCostFn::<_, Observations>::new(std::num::NonZeroUsize::new(2).context("should be non-zero")?);
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
        let cost_fn =
            HoprReturnCostFn::<_, Observations>::new(std::num::NonZeroUsize::new(2).context("should be non-zero")?);
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
        let ret = HoprReturnCostFn::<_, Observations>::new(length);
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
        let ret = HoprReturnCostFn::<_, Observations>::new(length);
        let ret_fn = ret.into_cost_fn();

        let obs = Observations {
            immediate: Some(StubImmediate {
                connected: true,
                score: 0.95,
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
    fn forward_last_edge_differs_from_return_last_edge() -> anyhow::Result<()> {
        let length = std::num::NonZeroUsize::new(2).context("should be non-zero")?;

        let fwd = HoprForwardCostFn::<_, Observations>::new(length);
        let ret = HoprReturnCostFn::<_, Observations>::new(length);
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
        let cost_fn =
            HoprReturnCostFn::<_, Observations>::new(std::num::NonZeroUsize::new(3).context("should be non-zero")?);
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

        let fwd = HoprForwardCostFn::<_, Observations>::new(length);
        let ret = HoprReturnCostFn::<_, Observations>::new(length);
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
    fn symmetrical_forward_path_works_with_forward_cost_fn() -> anyhow::Result<()> {
        let length = std::num::NonZeroUsize::new(2).context("should be non-zero")?;
        let cost_fn = HoprForwardCostFn::<_, Observations>::new(length);
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
        let cost_fn = HoprForwardCostFn::<_, Observations>::new(length);
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
        let cost_fn = HoprReturnCostFn::<_, Observations>::new(length);
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

        let fwd = HoprForwardCostFn::<_, Observations>::new(length);
        let fwd_fn = fwd.into_cost_fn();

        let me_to_relay = with_connected_and_capacity();
        let relay_to_dest = with_capacity_only();

        let fwd_cost = fwd_fn(1.0, &me_to_relay, 0);
        let fwd_cost = fwd_fn(fwd_cost, &relay_to_dest, 1);

        let ret = HoprReturnCostFn::<_, Observations>::new(length);
        let ret_fn = ret.into_cost_fn();

        let dest_to_relay = with_capacity_only();
        let relay_to_me = with_connected_and_capacity();

        let ret_cost = ret_fn(1.0, &dest_to_relay, 0);
        let ret_cost = ret_fn(ret_cost, &relay_to_me, 1);

        #[derive(serde::Serialize)]
        struct BidirectionalCost {
            forward_path_cost: f64,
            return_path_cost: f64,
        }

        insta::assert_yaml_snapshot!(BidirectionalCost {
            forward_path_cost: fwd_cost,
            return_path_cost: ret_cost,
        });

        Ok(())
    }
}
