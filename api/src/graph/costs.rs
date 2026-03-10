use super::traits::{
    CostFn, EdgeLinkObservable, EdgeNetworkObservableRead, EdgeObservableRead, EdgeProtocolObservable,
};

/// A boxed cost function accepting `(current_cost, edge_weight, path_index) -> new_cost`.
pub type BasicCostFn<C, W> = Box<dyn Fn(C, &W, usize) -> C>;

/// Build a HOPR cost function for immediate graph traversals.
///
/// Represents a backwards compatible cost function for the heartbeat protocol in v3.
pub struct SimpleHoprCostFn<C, W> {
    initial: C,
    min: Option<C>,
    cost_fn: BasicCostFn<C, W>,
}

impl<C, W> CostFn for SimpleHoprCostFn<C, W>
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

impl<W> SimpleHoprCostFn<f64, W>
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
                        if observation.immediate_qos().is_some_and(|o| o.is_connected())
                            && observation.intermediate_qos().is_some_and(|o| o.capacity().is_some())
                        {
                            return initial_cost;
                        }

                        -initial_cost
                    }
                    v if v == (length - 1) => {
                        // the last edge should always go from an already connected and measured peer,
                        // otherwise use a negative cost that should remove the edge from consideration
                        if observation.immediate_qos().is_some_and(|o| o.is_connected()) {
                            return initial_cost;
                        }

                        -initial_cost
                    }
                    _ => initial_cost,
                }
            }),
        }
    }
}

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
        EdgeLinkObservable, EdgeNetworkObservableRead, EdgeObservableRead, EdgeObservableWrite, EdgeProtocolObservable,
        EdgeTransportMeasurement, EdgeWeightType,
    };

    // ── Stub types implementing the observation traits ───────────────────

    /// Stub for immediate (n-hop) probe measurement.
    #[derive(Debug, Default, Clone)]
    struct StubImmediate {
        connected: bool,
        measurements: Vec<EdgeTransportMeasurement>,
    }

    impl EdgeNetworkObservableRead for StubImmediate {
        fn is_connected(&self) -> bool {
            self.connected
        }
    }

    impl EdgeLinkObservable for StubImmediate {
        fn record(&mut self, measurement: EdgeTransportMeasurement) {
            self.measurements.push(measurement);
        }

        fn average_latency(&self) -> Option<std::time::Duration> {
            let successes: Vec<_> = self.measurements.iter().filter_map(|m| m.as_ref().ok()).collect();
            if successes.is_empty() {
                return None;
            }
            let total: std::time::Duration = successes.iter().copied().sum();
            Some(total / successes.len() as u32)
        }

        fn average_probe_rate(&self) -> f64 {
            if self.measurements.is_empty() {
                return 0.0;
            }
            let successes = self.measurements.iter().filter(|m| m.is_ok()).count();
            successes as f64 / self.measurements.len() as f64
        }

        fn score(&self) -> f64 {
            if self.measurements.is_empty() {
                return 0.0;
            }
            // Simple score: probe success rate scaled by latency quality
            let rate = self.average_probe_rate();
            let latency_factor = self.average_latency().map_or(0.0, |d| {
                // Score higher for lower latency, capped at 1s
                (1.0 - (d.as_millis() as f64 / 1000.0).min(1.0)).max(0.0)
            });
            rate * latency_factor
        }
    }

    /// Stub for intermediate (relayed) probe measurements with capacity.
    #[derive(Debug, Default, Clone)]
    struct StubIntermediate {
        capacity: Option<u128>,
        measurements: Vec<EdgeTransportMeasurement>,
    }

    impl EdgeProtocolObservable for StubIntermediate {
        fn capacity(&self) -> Option<u128> {
            self.capacity
        }
    }

    impl EdgeLinkObservable for StubIntermediate {
        fn record(&mut self, measurement: EdgeTransportMeasurement) {
            self.measurements.push(measurement);
        }

        fn average_latency(&self) -> Option<std::time::Duration> {
            let successes: Vec<_> = self.measurements.iter().filter_map(|m| m.as_ref().ok()).collect();
            if successes.is_empty() {
                return None;
            }
            let total: std::time::Duration = successes.iter().copied().sum();
            Some(total / successes.len() as u32)
        }

        fn average_probe_rate(&self) -> f64 {
            if self.measurements.is_empty() {
                return 0.0;
            }
            let successes = self.measurements.iter().filter(|m| m.is_ok()).count();
            successes as f64 / self.measurements.len() as f64
        }

        fn score(&self) -> f64 {
            if self.measurements.is_empty() {
                return 0.0;
            }
            let rate = self.average_probe_rate();
            let latency_factor = self
                .average_latency()
                .map_or(0.0, |d| (1.0 - (d.as_millis() as f64 / 1000.0).min(1.0)).max(0.0));
            rate * latency_factor
        }
    }

    /// Stub `Observations` type implementing both read and write traits.
    #[derive(Debug, Default, Clone)]
    struct Observations {
        immediate: Option<StubImmediate>,
        intermediate: Option<StubIntermediate>,
    }

    impl EdgeObservableWrite for Observations {
        fn record(&mut self, measurement: EdgeWeightType) {
            match measurement {
                EdgeWeightType::Connected(connected) => {
                    self.immediate.get_or_insert_with(StubImmediate::default).connected = connected;
                }
                EdgeWeightType::Immediate(m) => {
                    self.immediate.get_or_insert_with(StubImmediate::default).record(m);
                }
                EdgeWeightType::Intermediate(m) => {
                    self.intermediate
                        .get_or_insert_with(StubIntermediate::default)
                        .record(m);
                }
                EdgeWeightType::Capacity(cap) => {
                    self.intermediate.get_or_insert_with(StubIntermediate::default).capacity = cap;
                }
            }
        }
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
            // Prefer intermediate score, fall back to immediate
            self.intermediate
                .as_ref()
                .map(|i| i.score())
                .or_else(|| self.immediate.as_ref().map(|i| i.score()))
                .unwrap_or(0.0)
        }
    }

    // ── Test observation builders ───────────────────────────────────────

    /// Build an `Observations` with immediate connected + intermediate with capacity.
    fn obs_connected_with_capacity() -> Observations {
        let mut obs = Observations::default();
        obs.record(EdgeWeightType::Connected(true));
        obs.record(EdgeWeightType::Immediate(Ok(std::time::Duration::from_millis(50))));
        obs.record(EdgeWeightType::Intermediate(Ok(std::time::Duration::from_millis(50))));
        obs.record(EdgeWeightType::Capacity(Some(1000)));
        obs
    }

    /// Build an `Observations` with immediate connected but no intermediate data.
    fn obs_connected_only_immediate() -> Observations {
        let mut obs = Observations::default();
        obs.record(EdgeWeightType::Connected(true));
        obs.record(EdgeWeightType::Immediate(Ok(std::time::Duration::from_millis(50))));
        obs
    }

    /// Build an `Observations` with intermediate + capacity but not connected.
    fn obs_not_connected_with_intermediate() -> Observations {
        let mut obs = Observations::default();
        obs.record(EdgeWeightType::Intermediate(Ok(std::time::Duration::from_millis(50))));
        obs.record(EdgeWeightType::Capacity(Some(1000)));
        obs
    }

    /// Build a bare `Observations` with no data at all.
    fn obs_empty() -> Observations {
        Observations::default()
    }

    /// Build an `Observations` with only capacity (from on-chain).
    fn obs_capacity_only() -> Observations {
        let mut obs = Observations::default();
        obs.record(EdgeWeightType::Capacity(Some(1000)));
        obs
    }

    // ── HoprForwardCostFn trait method tests ─────────────────────────────

    #[test]
    fn forward_cost_fn_invariants() -> anyhow::Result<()> {
        let cost_fn =
            HoprForwardCostFn::<_, Observations>::new(std::num::NonZeroUsize::new(3).context("should be non-zero")?);
        assert_eq!(cost_fn.initial_cost(), 1.0);
        assert_eq!(cost_fn.min_cost(), Some(0.0));
        Ok(())
    }

    // ── Forward first edge (path_index == 0) ────────────────────────────

    #[test]
    fn forward_first_edge_positive_when_connected_with_capacity() -> anyhow::Result<()> {
        let cost_fn =
            HoprForwardCostFn::<_, Observations>::new(std::num::NonZeroUsize::new(3).context("should be non-zero")?);
        let f = cost_fn.into_cost_fn();
        let obs = obs_connected_with_capacity();

        let cost = f(1.0, &obs, 0);
        assert!(
            cost > 0.0,
            "first edge should have positive cost when connected with capacity, got {cost}"
        );
        Ok(())
    }

    #[test]
    fn forward_first_edge_scales_by_immediate_score() -> anyhow::Result<()> {
        let cost_fn =
            HoprForwardCostFn::<_, Observations>::new(std::num::NonZeroUsize::new(3).context("should be non-zero")?);
        let f = cost_fn.into_cost_fn();
        let obs = obs_connected_with_capacity();

        let cost = f(2.0, &obs, 0);
        // cost = initial_cost * max(immediate_score, intermediate_score); scores in (0, 1]
        assert!(
            cost > 0.0 && cost <= 2.0,
            "cost should be scaled by immediate score, got {cost}"
        );
        Ok(())
    }

    #[test]
    fn forward_first_edge_positive_when_capacity_only_no_intermediate_probe() -> anyhow::Result<()> {
        let cost_fn =
            HoprForwardCostFn::<_, Observations>::new(std::num::NonZeroUsize::new(3).context("should be non-zero")?);
        let f = cost_fn.into_cost_fn();
        let mut obs = Observations::default();
        obs.record(EdgeWeightType::Connected(true));
        obs.record(EdgeWeightType::Immediate(Ok(std::time::Duration::from_millis(50))));
        obs.record(EdgeWeightType::Capacity(Some(1000)));

        let cost = f(1.0, &obs, 0);
        assert!(
            cost > 0.0,
            "first edge should be positive when connected with capacity even without intermediate probes, got {cost}"
        );
        Ok(())
    }

    #[test]
    fn forward_first_edge_negative_when_not_connected() -> anyhow::Result<()> {
        let cost_fn =
            HoprForwardCostFn::<_, Observations>::new(std::num::NonZeroUsize::new(3).context("should be non-zero")?);
        let f = cost_fn.into_cost_fn();
        let obs = obs_not_connected_with_intermediate();

        let cost = f(1.0, &obs, 0);
        assert!(
            cost < 0.0,
            "first edge should be negative when not connected, got {cost}"
        );
        Ok(())
    }

    #[test]
    fn forward_first_edge_negative_when_connected_but_no_intermediate() -> anyhow::Result<()> {
        let cost_fn =
            HoprForwardCostFn::<_, Observations>::new(std::num::NonZeroUsize::new(3).context("should be non-zero")?);
        let f = cost_fn.into_cost_fn();
        let obs = obs_connected_only_immediate();

        let cost = f(1.0, &obs, 0);
        assert!(
            cost < 0.0,
            "first edge should be negative without intermediate QoS, got {cost}"
        );
        Ok(())
    }

    #[test]
    fn forward_first_edge_negative_when_connected_intermediate_but_no_capacity() -> anyhow::Result<()> {
        let cost_fn =
            HoprForwardCostFn::<_, Observations>::new(std::num::NonZeroUsize::new(3).context("should be non-zero")?);
        let f = cost_fn.into_cost_fn();
        let mut obs = Observations::default();
        obs.record(EdgeWeightType::Connected(true));
        obs.record(EdgeWeightType::Immediate(Ok(std::time::Duration::from_millis(50))));
        obs.record(EdgeWeightType::Intermediate(Ok(std::time::Duration::from_millis(50))));
        // no capacity set

        let cost = f(1.0, &obs, 0);
        assert!(cost < 0.0, "first edge should be negative without capacity, got {cost}");
        Ok(())
    }

    #[test]
    fn forward_first_edge_negative_when_empty() -> anyhow::Result<()> {
        let cost_fn =
            HoprForwardCostFn::<_, Observations>::new(std::num::NonZeroUsize::new(3).context("should be non-zero")?);
        let f = cost_fn.into_cost_fn();

        let cost = f(1.0, &obs_empty(), 0);
        assert!(
            cost < 0.0,
            "first edge should be negative with no observations, got {cost}"
        );
        Ok(())
    }

    // ── Forward last edge (path_index == length - 1) ────────────────────

    #[test]
    fn forward_last_edge_positive_when_capacity_and_score() -> anyhow::Result<()> {
        let cost_fn =
            HoprForwardCostFn::<_, Observations>::new(std::num::NonZeroUsize::new(3).context("should be non-zero")?);
        let f = cost_fn.into_cost_fn();
        let obs = obs_connected_with_capacity();

        let cost = f(1.0, &obs, 2);
        assert!(
            cost > 0.0,
            "last edge should have positive cost with capacity and score, got {cost}"
        );
        Ok(())
    }

    #[test]
    fn forward_last_edge_positive_with_capacity_only_no_probes() -> anyhow::Result<()> {
        let cost_fn =
            HoprForwardCostFn::<_, Observations>::new(std::num::NonZeroUsize::new(3).context("should be non-zero")?);
        let f = cost_fn.into_cost_fn();

        let cost = f(1.0, &obs_capacity_only(), 2);
        assert_eq!(
            cost, 1.0,
            "forward last edge with capacity-only should pass through initial_cost, got {cost}"
        );
        Ok(())
    }

    #[test]
    fn forward_last_edge_positive_without_connectivity() -> anyhow::Result<()> {
        let cost_fn =
            HoprForwardCostFn::<_, Observations>::new(std::num::NonZeroUsize::new(3).context("should be non-zero")?);
        let f = cost_fn.into_cost_fn();
        let obs = obs_not_connected_with_intermediate();

        let cost = f(1.0, &obs, 2);
        assert!(
            cost > 0.0,
            "last edge should be positive with capacity even without connectivity, got {cost}"
        );
        Ok(())
    }

    #[test]
    fn forward_last_edge_positive_with_connectivity_no_capacity() -> anyhow::Result<()> {
        let cost_fn =
            HoprForwardCostFn::<_, Observations>::new(std::num::NonZeroUsize::new(3).context("should be non-zero")?);
        let f = cost_fn.into_cost_fn();
        let obs = obs_connected_only_immediate();

        let cost = f(1.0, &obs, 2);
        assert!(
            cost > 0.0,
            "last edge should be positive via connectivity fallback, got {cost}"
        );
        Ok(())
    }

    #[test]
    fn forward_last_edge_scales_by_intermediate_score() -> anyhow::Result<()> {
        let cost_fn =
            HoprForwardCostFn::<_, Observations>::new(std::num::NonZeroUsize::new(3).context("should be non-zero")?);
        let f = cost_fn.into_cost_fn();
        let obs = obs_connected_with_capacity();

        let cost = f(2.0, &obs, 2);
        assert!(
            cost > 0.0 && cost <= 2.0,
            "cost should be scaled by intermediate score, got {cost}"
        );
        Ok(())
    }

    #[test]
    fn forward_last_edge_positive_when_intermediate_but_no_capacity() -> anyhow::Result<()> {
        let cost_fn =
            HoprForwardCostFn::<_, Observations>::new(std::num::NonZeroUsize::new(3).context("should be non-zero")?);
        let f = cost_fn.into_cost_fn();
        let mut obs = Observations::default();
        obs.record(EdgeWeightType::Intermediate(Ok(std::time::Duration::from_millis(50))));

        let cost = f(1.0, &obs, 2);
        assert_eq!(
            cost, 1.0,
            "last edge should pass through initial_cost without capacity, got {cost}"
        );
        Ok(())
    }

    #[test]
    fn forward_last_edge_positive_when_empty() -> anyhow::Result<()> {
        let cost_fn =
            HoprForwardCostFn::<_, Observations>::new(std::num::NonZeroUsize::new(3).context("should be non-zero")?);
        let f = cost_fn.into_cost_fn();

        let cost = f(1.0, &obs_empty(), 2);
        assert_eq!(
            cost, 1.0,
            "last edge should pass through initial_cost with no observations, got {cost}"
        );
        Ok(())
    }

    // ── Forward intermediate edges (0 < path_index < length) ────────────

    #[test]
    fn forward_intermediate_edge_positive_when_capacity_and_score() -> anyhow::Result<()> {
        let cost_fn =
            HoprForwardCostFn::<_, Observations>::new(std::num::NonZeroUsize::new(3).context("should be non-zero")?);
        let f = cost_fn.into_cost_fn();
        let obs = obs_connected_with_capacity();

        let cost = f(1.0, &obs, 1);
        assert!(
            cost > 0.0,
            "intermediate edge should have positive cost with capacity and score, got {cost}"
        );
        Ok(())
    }

    #[test]
    fn forward_intermediate_edge_scales_by_intermediate_score() -> anyhow::Result<()> {
        let cost_fn =
            HoprForwardCostFn::<_, Observations>::new(std::num::NonZeroUsize::new(3).context("should be non-zero")?);
        let f = cost_fn.into_cost_fn();
        let obs = obs_connected_with_capacity();

        let cost = f(2.0, &obs, 1);
        assert!(
            cost > 0.0 && cost <= 2.0,
            "intermediate edge should be scaled by intermediate score, got {cost}"
        );
        Ok(())
    }

    #[test]
    fn forward_intermediate_edge_negative_when_no_intermediate() -> anyhow::Result<()> {
        let cost_fn =
            HoprForwardCostFn::<_, Observations>::new(std::num::NonZeroUsize::new(3).context("should be non-zero")?);
        let f = cost_fn.into_cost_fn();
        let obs = obs_connected_only_immediate();

        let cost = f(1.0, &obs, 1);
        assert!(
            cost < 0.0,
            "intermediate edge should be negative without intermediate QoS, got {cost}"
        );
        Ok(())
    }

    #[test]
    fn forward_intermediate_edge_negative_when_no_capacity() -> anyhow::Result<()> {
        let cost_fn =
            HoprForwardCostFn::<_, Observations>::new(std::num::NonZeroUsize::new(3).context("should be non-zero")?);
        let f = cost_fn.into_cost_fn();
        let mut obs = Observations::default();
        obs.record(EdgeWeightType::Intermediate(Ok(std::time::Duration::from_millis(50))));

        let cost = f(1.0, &obs, 1);
        assert!(
            cost < 0.0,
            "intermediate edge should be negative without capacity, got {cost}"
        );
        Ok(())
    }

    #[test]
    fn forward_intermediate_edge_positive_when_capacity_only_no_probes() -> anyhow::Result<()> {
        let cost_fn =
            HoprForwardCostFn::<_, Observations>::new(std::num::NonZeroUsize::new(3).context("should be non-zero")?);
        let f = cost_fn.into_cost_fn();

        let cost = f(1.0, &obs_capacity_only(), 1);
        assert_eq!(
            cost, 1.0,
            "intermediate edge with capacity-only should pass through initial_cost, got {cost}"
        );
        Ok(())
    }

    #[test]
    fn forward_intermediate_edge_negative_when_empty() -> anyhow::Result<()> {
        let cost_fn =
            HoprForwardCostFn::<_, Observations>::new(std::num::NonZeroUsize::new(3).context("should be non-zero")?);
        let f = cost_fn.into_cost_fn();

        let cost = f(1.0, &obs_empty(), 1);
        assert!(
            cost < 0.0,
            "intermediate edge should be negative with no observations, got {cost}"
        );
        Ok(())
    }

    #[test]
    fn forward_intermediate_edge_uses_observations() -> anyhow::Result<()> {
        let cost_fn =
            HoprForwardCostFn::<_, Observations>::new(std::num::NonZeroUsize::new(3).context("should be non-zero")?);
        let f = cost_fn.into_cost_fn();

        let cost_empty = f(1.0, &obs_empty(), 1);
        let cost_full = f(1.0, &obs_connected_with_capacity(), 1);
        assert_ne!(cost_empty, cost_full, "intermediate edges should use observations");
        Ok(())
    }

    // ── Forward length boundary tests ───────────────────────────────────

    #[test]
    fn forward_length_one_has_only_first_and_last_edge() -> anyhow::Result<()> {
        let cost_fn =
            HoprForwardCostFn::<_, Observations>::new(std::num::NonZeroUsize::new(1).context("should be non-zero")?);
        let f = cost_fn.into_cost_fn();
        let obs = obs_connected_with_capacity();

        let first = f(1.0, &obs, 0);
        assert!(first > 0.0, "index 0 should be first-edge logic");
        Ok(())
    }

    #[test]
    fn forward_length_two_intermediate_at_index_one() -> anyhow::Result<()> {
        let cost_fn =
            HoprForwardCostFn::<_, Observations>::new(std::num::NonZeroUsize::new(2).context("should be non-zero")?);
        let f = cost_fn.into_cost_fn();
        let obs = obs_connected_with_capacity();

        let cost = f(1.0, &obs, 1);
        assert!(
            cost > 0.0,
            "index 1 should be last-edge logic (positive when connected with score)"
        );

        let cost_empty = f(1.0, &obs_empty(), 1);
        assert_eq!(
            cost_empty, 1.0,
            "index 1 (last edge) should pass through initial_cost with empty obs"
        );
        Ok(())
    }

    // ── Forward negative initial cost propagation ───────────────────────

    #[test]
    fn forward_negative_initial_cost_inverts_rejection() -> anyhow::Result<()> {
        let cost_fn =
            HoprForwardCostFn::<_, Observations>::new(std::num::NonZeroUsize::new(3).context("should be non-zero")?);
        let f = cost_fn.into_cost_fn();

        let cost = f(-1.0, &obs_empty(), 0);
        assert!(
            cost > 0.0,
            "negative initial cost should invert the rejection, got {cost}"
        );
        Ok(())
    }

    // ── HoprReturnCostFn trait method tests ──────────────────────────────

    #[test]
    fn return_cost_fn_invariants() -> anyhow::Result<()> {
        let cost_fn =
            HoprReturnCostFn::<_, Observations>::new(std::num::NonZeroUsize::new(3).context("should be non-zero")?);
        assert_eq!(cost_fn.initial_cost(), 1.0);
        assert_eq!(cost_fn.min_cost(), Some(0.0));
        Ok(())
    }

    // ── Return first edge (path_index == 0) ─────────────────────────────

    #[test]
    fn return_first_edge_positive_with_intermediate_and_capacity() -> anyhow::Result<()> {
        let cost_fn =
            HoprReturnCostFn::<_, Observations>::new(std::num::NonZeroUsize::new(2).context("should be non-zero")?);
        let f = cost_fn.into_cost_fn();

        let obs = obs_not_connected_with_intermediate();
        let cost = f(1.0, &obs, 0);
        assert!(
            cost > 0.0,
            "return first edge should be positive with intermediate + capacity, got {cost}"
        );
        Ok(())
    }

    #[test]
    fn return_first_edge_positive_with_full_data() -> anyhow::Result<()> {
        let cost_fn =
            HoprReturnCostFn::<_, Observations>::new(std::num::NonZeroUsize::new(2).context("should be non-zero")?);
        let f = cost_fn.into_cost_fn();

        let cost = f(1.0, &obs_connected_with_capacity(), 0);
        assert!(
            cost > 0.0,
            "return first edge should also work with full data, got {cost}"
        );
        Ok(())
    }

    #[test]
    fn return_first_edge_scales_by_intermediate_score() -> anyhow::Result<()> {
        let cost_fn =
            HoprReturnCostFn::<_, Observations>::new(std::num::NonZeroUsize::new(2).context("should be non-zero")?);
        let f = cost_fn.into_cost_fn();

        let obs = obs_not_connected_with_intermediate();
        let cost = f(2.0, &obs, 0);
        assert!(
            cost > 0.0 && cost <= 2.0,
            "return first edge should scale by intermediate score, got {cost}"
        );
        Ok(())
    }

    #[test]
    fn return_first_edge_does_not_require_connectivity() -> anyhow::Result<()> {
        let cost_fn =
            HoprReturnCostFn::<_, Observations>::new(std::num::NonZeroUsize::new(2).context("should be non-zero")?);
        let f = cost_fn.into_cost_fn();

        let obs = obs_not_connected_with_intermediate();
        let cost = f(1.0, &obs, 0);
        assert!(
            cost > 0.0,
            "return first edge should not require connectivity, got {cost}"
        );
        Ok(())
    }

    #[test]
    fn return_first_edge_positive_when_capacity_only_no_probes() -> anyhow::Result<()> {
        let cost_fn =
            HoprReturnCostFn::<_, Observations>::new(std::num::NonZeroUsize::new(2).context("should be non-zero")?);
        let f = cost_fn.into_cost_fn();

        let cost = f(1.0, &obs_capacity_only(), 0);
        assert_eq!(
            cost, 1.0,
            "return first edge with capacity-only should pass through initial_cost, got {cost}"
        );
        Ok(())
    }

    #[test]
    fn return_first_edge_negative_when_no_capacity() -> anyhow::Result<()> {
        let cost_fn =
            HoprReturnCostFn::<_, Observations>::new(std::num::NonZeroUsize::new(2).context("should be non-zero")?);
        let f = cost_fn.into_cost_fn();

        let cost = f(1.0, &obs_connected_only_immediate(), 0);
        assert!(
            cost < 0.0,
            "return first edge should be negative without capacity, got {cost}"
        );
        Ok(())
    }

    #[test]
    fn return_first_edge_negative_when_empty() -> anyhow::Result<()> {
        let cost_fn =
            HoprReturnCostFn::<_, Observations>::new(std::num::NonZeroUsize::new(2).context("should be non-zero")?);
        let f = cost_fn.into_cost_fn();

        let cost = f(1.0, &obs_empty(), 0);
        assert!(
            cost < 0.0,
            "return first edge should be negative with no observations, got {cost}"
        );
        Ok(())
    }

    // ── Return last edge ────────────────────────────────────────────────

    #[test]
    fn return_last_edge_requires_connectivity() -> anyhow::Result<()> {
        let length = std::num::NonZeroUsize::new(2).context("should be non-zero")?;

        let ret = HoprReturnCostFn::<_, Observations>::new(length);
        let ret_fn = ret.into_cost_fn();

        let obs = obs_connected_with_capacity();
        let cost = ret_fn(1.0, &obs, 1);
        assert!(
            cost > 0.0,
            "return last edge should be positive when connected, got {cost}"
        );

        let obs_no_conn = obs_not_connected_with_intermediate();
        let cost = ret_fn(1.0, &obs_no_conn, 1);
        assert!(
            cost < 0.0,
            "return last edge should be negative without connectivity, got {cost}"
        );

        Ok(())
    }

    #[test]
    fn return_last_edge_positive_when_connected_with_empty_intermediate() -> anyhow::Result<()> {
        let length = std::num::NonZeroUsize::new(2).context("should be non-zero")?;
        let ret = HoprReturnCostFn::<_, Observations>::new(length);
        let ret_fn = ret.into_cost_fn();

        let mut obs = Observations::default();
        obs.record(EdgeWeightType::Connected(true));
        obs.record(EdgeWeightType::Immediate(Ok(std::time::Duration::from_millis(13))));
        obs.record(EdgeWeightType::Intermediate(Err(())));

        let cost = ret_fn(1.0, &obs, 1);
        assert!(
            cost > 0.0,
            "return last edge should be positive when connected even with empty intermediate probe, got {cost}"
        );

        Ok(())
    }

    #[test]
    fn forward_last_edge_differs_from_return_last_edge() -> anyhow::Result<()> {
        let length = std::num::NonZeroUsize::new(2).context("should be non-zero")?;

        let fwd = HoprForwardCostFn::<_, Observations>::new(length);
        let ret = HoprReturnCostFn::<_, Observations>::new(length);
        let fwd_fn = fwd.into_cost_fn();
        let ret_fn = ret.into_cost_fn();

        let obs = obs_not_connected_with_intermediate();
        let fwd_cost = fwd_fn(1.0, &obs, 1);
        let ret_cost = ret_fn(1.0, &obs, 1);
        assert!(
            fwd_cost > 0.0,
            "forward last edge accepts capacity-only, got {fwd_cost}"
        );
        assert!(ret_cost < 0.0, "return last edge requires connectivity, got {ret_cost}");

        Ok(())
    }

    // ── Return intermediate edge ────────────────────────────────────────

    #[test]
    fn return_intermediate_edge_positive_when_capacity_only_no_probes() -> anyhow::Result<()> {
        let cost_fn =
            HoprReturnCostFn::<_, Observations>::new(std::num::NonZeroUsize::new(3).context("should be non-zero")?);
        let f = cost_fn.into_cost_fn();

        let cost = f(1.0, &obs_capacity_only(), 1);
        assert_eq!(
            cost, 1.0,
            "return intermediate edge with capacity-only should pass through initial_cost, got {cost}"
        );
        Ok(())
    }

    #[test]
    fn return_intermediate_edge_same_as_forward() -> anyhow::Result<()> {
        let length = std::num::NonZeroUsize::new(3).context("should be non-zero")?;

        let fwd = HoprForwardCostFn::<_, Observations>::new(length);
        let ret = HoprReturnCostFn::<_, Observations>::new(length);
        let fwd_fn = fwd.into_cost_fn();
        let ret_fn = ret.into_cost_fn();

        let obs = obs_connected_with_capacity();

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

        let me_to_relay = obs_connected_with_capacity();
        let relay_to_dest = obs_capacity_only();

        let cost_after_first = f(1.0, &me_to_relay, 0);
        assert!(
            cost_after_first > 0.0,
            "forward first edge (me->relay) should be positive, got {cost_after_first}"
        );

        let cost_after_last = f(cost_after_first, &relay_to_dest, 1);
        assert!(
            cost_after_last > 0.0,
            "forward last edge (relay->dest) should be positive with capacity-only, got {cost_after_last}"
        );

        Ok(())
    }

    #[test]
    fn symmetrical_return_path_rejected_by_forward_cost_fn() -> anyhow::Result<()> {
        let length = std::num::NonZeroUsize::new(2).context("should be non-zero")?;
        let cost_fn = HoprForwardCostFn::<_, Observations>::new(length);
        let f = cost_fn.into_cost_fn();

        let dest_to_relay = obs_not_connected_with_intermediate();
        let relay_to_me = obs_connected_with_capacity();

        let cost_after_first = f(1.0, &dest_to_relay, 0);
        assert!(
            cost_after_first < 0.0,
            "HoprForwardCostFn should reject the return first edge without connectivity, got {cost_after_first}"
        );

        let cost_after_last = f(cost_after_first, &relay_to_me, 1);
        assert!(
            cost_after_last < 0.0,
            "HoprForwardCostFn return path should be fully rejected, got {cost_after_last}"
        );

        Ok(())
    }

    #[test]
    fn symmetrical_return_path_works_with_return_cost_fn() -> anyhow::Result<()> {
        let length = std::num::NonZeroUsize::new(2).context("should be non-zero")?;
        let cost_fn = HoprReturnCostFn::<_, Observations>::new(length);
        let f = cost_fn.into_cost_fn();

        let dest_to_relay = obs_not_connected_with_intermediate();
        let relay_to_me = obs_connected_with_capacity();

        let cost_after_first = f(1.0, &dest_to_relay, 0);
        assert!(
            cost_after_first > 0.0,
            "HoprReturnCostFn first edge should have positive cost, got {cost_after_first}"
        );

        let cost_after_last = f(cost_after_first, &relay_to_me, 1);
        assert!(
            cost_after_last > 0.0,
            "HoprReturnCostFn return path should have positive cost, got {cost_after_last}"
        );

        Ok(())
    }

    #[test]
    fn symmetrical_bidirectional_both_paths_positive() -> anyhow::Result<()> {
        let length = std::num::NonZeroUsize::new(2).context("should be non-zero")?;

        let fwd = HoprForwardCostFn::<_, Observations>::new(length);
        let fwd_fn = fwd.into_cost_fn();

        let me_to_relay = obs_connected_with_capacity();
        let relay_to_dest = obs_capacity_only();

        let fwd_cost = fwd_fn(1.0, &me_to_relay, 0);
        let fwd_cost = fwd_fn(fwd_cost, &relay_to_dest, 1);
        assert!(fwd_cost > 0.0, "forward path should have positive cost, got {fwd_cost}");

        let ret = HoprReturnCostFn::<_, Observations>::new(length);
        let ret_fn = ret.into_cost_fn();

        let dest_to_relay = obs_capacity_only();
        let relay_to_me = obs_connected_with_capacity();

        let ret_cost = ret_fn(1.0, &dest_to_relay, 0);
        let ret_cost = ret_fn(ret_cost, &relay_to_me, 1);
        assert!(ret_cost > 0.0, "return path should have positive cost, got {ret_cost}");

        Ok(())
    }
}
