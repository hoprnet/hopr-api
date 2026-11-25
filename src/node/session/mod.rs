//! Session traits for HOPR node interactions.
//!
//! - `entry` — session entry operations (establishing outgoing sessions)
//! - `exit` — session exit operations (processing incoming sessions)

#[cfg(feature = "node-session-entry")]
pub mod entry;

#[cfg(feature = "node-session-exit")]
/// Session exit traits for handling incoming sessions.
pub mod exit;

/// Deprecated path of [`entry`].
#[cfg(feature = "node-session-entry")]
#[deprecated(note = "renamed to `entry`")]
pub mod client {
    pub use super::entry::*;
}

/// Deprecated path of [`exit`].
#[cfg(feature = "node-session-exit")]
#[deprecated(note = "renamed to `exit`")]
pub mod server {
    pub use super::exit::*;
}

/// The old client/server names must keep working for code that names only them: it implements the
/// traits, finds their methods and associated types, and is accepted where the new names are expected.
#[cfg(all(test, feature = "node-session-entry", feature = "node-session-exit"))]
mod old_names {
    use futures::io::Cursor;

    use crate::node::{HoprSessionClientOperations, HoprSessionServer};

    struct Old;

    #[async_trait::async_trait]
    impl HoprSessionServer for Old {
        type Error = std::io::Error;
        type Session = u8;

        async fn process(&self, _session: u8) -> Result<(), std::io::Error> {
            Ok(())
        }
    }

    #[async_trait::async_trait]
    impl HoprSessionClientOperations for Old {
        type Config = ();
        type Error = std::io::Error;
        type Session = Cursor<Vec<u8>>;
        type SessionConfigurator = ();
        type Target = ();

        async fn connect_to(
            &self,
            _destination: crate::Address,
            _target: (),
            _config: (),
        ) -> Result<(Self::Session, ()), std::io::Error> {
            Ok((Cursor::new(Vec::new()), ()))
        }
    }

    fn takes_new_exit<T: crate::node::HoprSessionExit>(_: &T) {}
    fn takes_new_entry<T: crate::node::HoprSessionEntryOperations>(_: &T) {}

    #[tokio::test]
    async fn an_implementor_of_the_old_names_is_accepted_as_the_new_ones() {
        // Only the old names are imported, so `process` and `connect_to` resolve through them.
        Old.process(1).await.expect("process");
        Old.connect_to(Default::default(), (), ()).await.expect("connect");
        takes_new_exit(&Old);
        takes_new_entry(&Old);
    }
}
