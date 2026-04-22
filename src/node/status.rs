use std::borrow::Cow;

/// Health status of an individual component within the HOPR node.
///
/// Each component (chain, network, transport, tickets) reports its own status
/// independently through its corresponding `Has*` accessor trait.
///
/// Detail messages use `Cow<'static, str>` so that components returning
/// fixed diagnostic strings avoid heap allocation on every status query.
#[derive(Debug, Clone, PartialEq, Eq, Hash, strum::Display, strum::EnumIs)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ComponentStatus {
    /// Component is fully operational.
    #[strum(to_string = "Ready")]
    Ready,
    /// Component is starting up or waiting on a dependency.
    #[strum(to_string = "Initializing: {0}")]
    Initializing(Cow<'static, str>),
    /// Component is running but in a degraded state.
    #[strum(to_string = "Degraded: {0}")]
    Degraded(Cow<'static, str>),
    /// Component is not operational.
    #[strum(to_string = "Unavailable: {0}")]
    Unavailable(Cow<'static, str>),
}

/// Trait for components that can report their own health status.
///
/// Implementors track their health internally and return the current
/// [`ComponentStatus`] on demand. This enables the `Has*` accessor
/// traits to delegate status queries directly to the underlying component
/// rather than computing status from global node state.
#[auto_impl::auto_impl(&, Arc)]
pub trait ComponentStatusReporter {
    /// Returns the current health status of this component.
    fn component_status(&self) -> ComponentStatus;
}
