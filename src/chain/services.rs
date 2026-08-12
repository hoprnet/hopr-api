use futures::stream::BoxStream;
pub use hopr_types::internal::prelude::{ServiceEntry, ServiceMetadata, ServiceType};
use hopr_types::primitive::prelude::{Address, HoprBalance};

/// Registry configuration of a single service type.
///
/// Mirrors the per-type record of `HoprServiceRegistry`, whose changes are reported by the
/// `ChainEvent::ServiceType*` variants.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ServiceTypeConfig {
    /// Owner of the service type, or `None` if the type was abandoned.
    ///
    /// Abandoning is one-way: an abandoned type keeps this configuration forever.
    pub owner: Option<Address>,
    /// Requirement contract gating registrations under the type, or `None` if the type is open.
    ///
    /// An open type applies no policy beyond the Safe binding and the burn.
    pub requirement: Option<Address>,
    /// Amount burned when registering an entry under the type.
    pub registration_burn: HoprBalance,
    /// Amount burned when updating an entry under the type.
    pub update_burn: HoprBalance,
}

/// Selector for entries in the on-chain service registry.
///
/// See [`ChainReadServiceOperations::stream_services`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct ServiceSelector {
    /// Selects entries registered under the given service type.
    pub service_type: Option<ServiceType>,
    /// Selects entries offered by the given node.
    pub node: Option<Address>,
    /// Selects only entries whose node still has a Safe binding.
    ///
    /// An entry whose node has no binding (`nodeToSafe(node) == 0`) is orphaned: the registry
    /// accepts writes for an entry only from the node's Safe, so an orphaned entry can never be
    /// updated or removed again and must be treated as dead.
    pub live_only: bool,
}

impl ServiceSelector {
    /// Selects entries registered under the given service type.
    #[must_use]
    pub fn with_service_type(mut self, service_type: ServiceType) -> Self {
        self.service_type = Some(service_type);
        self
    }

    /// Selects entries offered by the given node.
    #[must_use]
    pub fn with_node(mut self, node: Address) -> Self {
        self.node = Some(node);
        self
    }

    /// Selects only entries whose node still has a Safe binding.
    #[must_use]
    pub fn with_live_only(mut self, live_only: bool) -> Self {
        self.live_only = live_only;
        self
    }

    /// Checks if the given [`entry`](ServiceEntry) satisfies the selector.
    ///
    /// Liveness cannot be read off the entry: [`ServiceEntry::safe`] records the Safe that
    /// performed the last write, not the binding the node has now. The caller therefore supplies
    /// it as `node_is_live`, which is `nodeToSafe(entry.node) != 0` on the node-Safe registry the
    /// service registry points at. It is ignored unless [`ServiceSelector::live_only`] is set.
    pub fn satisfies(&self, entry: &ServiceEntry, node_is_live: bool) -> bool {
        if self.live_only && !node_is_live {
            return false;
        }

        if let Some(service_type) = &self.service_type
            && &entry.service_type != service_type
        {
            return false;
        }

        if let Some(node) = &self.node
            && &entry.node != node
        {
            return false;
        }

        true
    }
}

/// Chain operations that read the on-chain service registry.
///
/// Implementors resolve the node-Safe binding themselves and pass it to
/// [`ServiceSelector::satisfies`], which is the single definition of the filter.
#[async_trait::async_trait]
#[auto_impl::auto_impl(&, Box, Arc)]
pub trait ChainReadServiceOperations {
    type Error: std::error::Error + Send + Sync + 'static;

    /// Returns the registry entries matching the given [`ServiceSelector`].
    fn stream_services<'a>(&'a self, selector: ServiceSelector) -> Result<BoxStream<'a, ServiceEntry>, Self::Error>;

    /// Counts the registry entries matching the given [`ServiceSelector`].
    ///
    /// This is potentially done more effectively than counting the elements of
    /// the stream returned by [`ChainReadServiceOperations::stream_services`].
    async fn count_services(&self, selector: ServiceSelector) -> Result<usize, Self::Error>;

    /// Returns the registry configuration of the given service type,
    /// or `None` if no such type is registered.
    async fn get_service_type_config(
        &self,
        service_type: ServiceType,
    ) -> Result<Option<ServiceTypeConfig>, Self::Error>;
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, UNIX_EPOCH};

    use super::*;

    fn node() -> Address {
        Address::new(&[1u8; 20])
    }

    fn other_node() -> Address {
        Address::new(&[2u8; 20])
    }

    fn service_entry(service_type: ServiceType, node: Address) -> anyhow::Result<ServiceEntry> {
        let registered_at = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        Ok(ServiceEntry::new(
            service_type,
            node,
            Address::new(&[9u8; 20]),
            ServiceMetadata::default(),
            registered_at,
            registered_at,
        )?)
    }

    #[test]
    fn default_service_selector_should_accept_live_and_dead_entries_alike() -> anyhow::Result<()> {
        let entry = service_entry(ServiceType::GVPN_EXIT, node())?;

        assert!(ServiceSelector::default().satisfies(&entry, true));
        assert!(ServiceSelector::default().satisfies(&entry, false));

        Ok(())
    }

    #[test]
    fn service_selector_should_filter_by_service_type() -> anyhow::Result<()> {
        let entry = service_entry(ServiceType::GVPN_EXIT, node())?;
        let other_type: ServiceType = "other".parse()?;

        assert!(
            ServiceSelector::default()
                .with_service_type(ServiceType::GVPN_EXIT)
                .satisfies(&entry, true)
        );
        assert!(
            !ServiceSelector::default()
                .with_service_type(other_type)
                .satisfies(&entry, true)
        );

        Ok(())
    }

    #[test]
    fn service_selector_should_filter_by_node() -> anyhow::Result<()> {
        let entry = service_entry(ServiceType::GVPN_EXIT, node())?;

        assert!(ServiceSelector::default().with_node(node()).satisfies(&entry, true));
        assert!(
            !ServiceSelector::default()
                .with_node(other_node())
                .satisfies(&entry, true)
        );

        Ok(())
    }

    #[test]
    fn service_selector_live_only_should_drop_entries_whose_node_has_no_safe_binding() -> anyhow::Result<()> {
        let entry = service_entry(ServiceType::GVPN_EXIT, node())?;

        assert!(ServiceSelector::default().with_live_only(true).satisfies(&entry, true));
        assert!(!ServiceSelector::default().with_live_only(true).satisfies(&entry, false));

        Ok(())
    }

    #[test]
    fn service_selector_builder_methods_should_compose() -> anyhow::Result<()> {
        let selector = ServiceSelector::default()
            .with_service_type(ServiceType::GVPN_EXIT)
            .with_node(node())
            .with_live_only(true);

        assert_eq!(
            selector,
            ServiceSelector {
                service_type: Some(ServiceType::GVPN_EXIT),
                node: Some(node()),
                live_only: true,
            }
        );

        assert!(selector.satisfies(&service_entry(ServiceType::GVPN_EXIT, node())?, true));

        // Every field of the composed selector must still reject on its own.
        assert!(!selector.satisfies(&service_entry(ServiceType::GVPN_EXIT, node())?, false));
        assert!(!selector.satisfies(&service_entry("other".parse()?, node())?, true));
        assert!(!selector.satisfies(&service_entry(ServiceType::GVPN_EXIT, other_node())?, true));

        Ok(())
    }
}
