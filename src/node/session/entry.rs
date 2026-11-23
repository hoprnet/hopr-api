//! Session entry operations for establishing HOPR sessions.
//!
//! Gated behind the `node-session-entry` feature.

use futures::io::{AsyncRead, AsyncWrite};

use crate::Address;

/// Trait for establishing HOPR sessions to remote destinations.
///
/// The concrete session, configurator, target, and config types are defined
/// by the implementor (typically hopr-lib), keeping transport-level types
/// out of the API crate.
#[async_trait::async_trait]
#[auto_impl::auto_impl(&, Arc)]
pub trait HoprSessionEntryOperations: Send + Sync {
    /// An established session implementing async read/write.
    type Session: AsyncRead + AsyncWrite + Send + Unpin;
    /// Handle for controlling a session after creation (e.g. keep-alive, SURB config).
    type SessionConfigurator: Send;
    /// Describes the remote service to connect to.
    type Target: Send;
    /// Configuration for the session (routing, capabilities, etc.).
    type Config: Send;
    /// Error type for session operations.
    type Error: std::error::Error + Send + Sync + 'static;

    /// Establishes a new session to the given `destination` via the HOPR network.
    ///
    /// Returns the session (implementing [`AsyncRead`] + [`AsyncWrite`]) and a configurator
    /// for controlling the session after creation.
    ///
    /// Implementations may retry automatically on failure.
    async fn connect_to(
        &self,
        destination: Address,
        target: Self::Target,
        config: Self::Config,
    ) -> Result<(Self::Session, Self::SessionConfigurator), Self::Error>;
}

/// Deprecated name of [`HoprSessionEntryOperations`].
///
/// Bounds written against this name keep working, because every [`HoprSessionEntryOperations`]
/// implementor satisfies it. Implementors must implement the new trait instead.
#[deprecated(note = "renamed to `HoprSessionEntryOperations`")]
pub trait HoprSessionClientOperations: HoprSessionEntryOperations {}

#[allow(deprecated)]
impl<T: HoprSessionEntryOperations + ?Sized> HoprSessionClientOperations for T {}
