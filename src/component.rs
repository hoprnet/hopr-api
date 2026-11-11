//! Operational state shared by strategies, components, and their callers.
//!
//! Each implementation defines what work it needs to perform and when it can recover.
//! Diagnostic collection failures do not, by themselves, change operational state.

/// Current ability of a component to perform its responsibilities.
///
/// Defaults to `Running`, including for components without their own state tracking.
/// Values are ordered by severity, so `max` retains the worse state.
/// [`AtomicComponentState`] provides lock-free storage and shared live observations.
///
/// ```
/// use hopr_api::ComponentState;
/// assert_eq!(ComponentState::default(), ComponentState::Running);
/// assert!(ComponentState::Running < ComponentState::Degraded);
/// assert!(ComponentState::Degraded < ComponentState::Failed);
/// assert_eq!(u8::from(ComponentState::Degraded), 1);
/// assert!(ComponentState::try_from(99).is_err());
/// ```
#[atomic_enum::atomic_enum]
#[derive(
    strum::Display, PartialEq, Eq, PartialOrd, Ord, Default, num_enum::IntoPrimitive, num_enum::TryFromPrimitive,
)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[repr(u8)]
pub enum ComponentState {
    /// Operating normally: everything the component currently needs to do, it can do.
    #[default]
    Running = 0,
    /// Can still do some of what it needs to, but not all of it.
    Degraded = 1,
    /// Cannot currently do any of what it needs to.
    Failed = 2,
}

#[cfg(feature = "serde")]
mod dump;
#[cfg(feature = "serde")]
pub use dump::StateDump;

#[cfg(test)]
mod tests {
    use std::sync::{Arc, atomic::Ordering};

    use super::*;

    #[test]
    fn operational_states_preserve_strategy_semantics() -> anyhow::Result<()> {
        assert_eq!(ComponentState::default(), ComponentState::Running);
        assert!(ComponentState::Running < ComponentState::Degraded);
        assert!(ComponentState::Degraded < ComponentState::Failed);
        for (byte, state) in [
            ComponentState::Running,
            ComponentState::Degraded,
            ComponentState::Failed,
        ]
        .into_iter()
        .enumerate()
        {
            assert_eq!(u8::from(state), byte as u8);
            assert_eq!(ComponentState::try_from(byte as u8)?, state);
        }
        assert!(ComponentState::try_from(3).is_err());
        assert_eq!(ComponentState::Degraded.to_string(), "Degraded");
        Ok(())
    }

    #[cfg(feature = "serde")]
    #[test]
    fn serde_preserves_operational_state() -> anyhow::Result<()> {
        for state in [
            ComponentState::Running,
            ComponentState::Degraded,
            ComponentState::Failed,
        ] {
            let json = serde_json::to_string(&state)?;
            assert_eq!(serde_json::from_str::<ComponentState>(&json)?, state);
        }
        assert!(serde_json::from_str::<ComponentState>("\"Unknown\"").is_err());
        Ok(())
    }

    #[test]
    fn shared_handles_observe_live_transitions() {
        let state = Arc::new(AtomicComponentState::new(ComponentState::Running));
        let observer = state.clone();
        state.store(ComponentState::Degraded, Ordering::Relaxed);
        assert_eq!(observer.load(Ordering::Relaxed), ComponentState::Degraded);
        state.store(ComponentState::Failed, Ordering::Relaxed);
        assert_eq!(observer.load(Ordering::Relaxed), ComponentState::Failed);
        state.store(ComponentState::Running, Ordering::Relaxed);
        assert_eq!(observer.load(Ordering::Relaxed), ComponentState::Running);
    }
}
