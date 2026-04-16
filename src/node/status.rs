/// Health status of an individual component within the HOPR node.
///
/// Each component (chain, network, transport, tickets) reports its own status
/// independently through its corresponding `Has*` accessor trait.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ComponentStatus {
    /// Component is fully operational.
    Ready,
    /// Component is starting up or waiting on a dependency.
    Initializing(String),
    /// Component is running but in a degraded state.
    Degraded(String),
    /// Component is not operational.
    Unavailable(String),
}

impl ComponentStatus {
    /// Returns `true` if the component is fully operational.
    pub fn is_ready(&self) -> bool {
        matches!(self, Self::Ready)
    }
}

impl std::fmt::Display for ComponentStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Ready => write!(f, "Ready"),
            Self::Initializing(msg) => write!(f, "Initializing: {msg}"),
            Self::Degraded(msg) => write!(f, "Degraded: {msg}"),
            Self::Unavailable(msg) => write!(f, "Unavailable: {msg}"),
        }
    }
}
