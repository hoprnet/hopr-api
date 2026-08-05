//! Session server trait for processing incoming HOPR sessions.
//!
//! Gated behind the `node-session-server` feature.

use hopr_types::network::{SessionId, SessionTarget};

/// An incoming HOPR session to be processed by a [`HoprSessionServer`].
///
/// Generic over the concrete session byte-stream `S` (supplied by the implementor,
/// typically hopr-lib), keeping transport-level types out of this crate. Only the
/// plain [`SessionId`] and [`SessionTarget`] descriptors are named here.
#[derive(Debug)]
pub struct IncomingSession<S> {
    /// Identifier of the incoming session.
    pub id: SessionId,
    /// The session byte-stream carrying the forwarded data.
    pub session: S,
    /// Describes where data received over the session should be forwarded.
    pub target: SessionTarget,
}

/// Trait for processing incoming HOPR sessions on exit nodes.
///
/// The concrete session type is defined by the implementor (typically hopr-lib),
/// keeping transport-level types out of the API crate.
///
/// Nodes that do not run a session server simply omit calling `with_session_server`.
#[async_trait::async_trait]
#[auto_impl::auto_impl(Arc)]
pub trait HoprSessionServer {
    /// An incoming session to be processed.
    type Session: Send;
    /// Error type for session processing.
    type Error: std::error::Error + Send + Sync + 'static;

    /// Fully process a single incoming HOPR session.
    async fn process(&self, session: Self::Session) -> Result<(), Self::Error>;
}
