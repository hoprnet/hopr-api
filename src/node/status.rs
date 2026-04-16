/// Health status of an individual component within the HOPR node.
///
/// Each component (chain, network, transport, tickets) reports its own status
/// independently through its corresponding `Has*` accessor trait.
#[derive(Debug, Clone, PartialEq, Eq, strum::Display)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ComponentStatus {
    /// Component is fully operational.
    #[strum(to_string = "Ready")]
    Ready,
    /// Component is starting up or waiting on a dependency.
    #[strum(to_string = "Initializing: {0}")]
    Initializing(String),
    /// Component is running but in a degraded state.
    #[strum(to_string = "Degraded: {0}")]
    Degraded(String),
    /// Component is not operational.
    #[strum(to_string = "Unavailable: {0}")]
    Unavailable(String),
}

impl ComponentStatus {
    /// Returns `true` if the component is fully operational.
    pub fn is_ready(&self) -> bool {
        matches!(self, Self::Ready)
    }
}
