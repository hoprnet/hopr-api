//! Pollable diagnostics with implementation-owned snapshot models.
use std::{error::Error, future::Future, sync::Arc};

use serde::{Serialize, de::DeserializeOwned};

/// Optional diagnostic state collection shared by components and their callers.
///
/// Available when both the `component` and `serde` features are enabled.
///
/// Implementations keep their concrete snapshot models private. Callers receive an owned,
/// opaque Serde value whose schema is defined by the implementation and may change.
/// Collection errors describe diagnostics availability, independently of [`super::ComponentState`].
///
/// Collection must bound its work and avoid blocking the async executor. Implementations document
/// whether polling resets interval counters or otherwise consumes observations. Timers, logging,
/// deadlines and cancellation remain the responsibility of the implementation or its host.
/// For components needing caller-supplied context, the implementing crate can expose a separate
/// constructor or method that accepts that context and returns a view implementing this trait.
/// Callers invoke `state_dump()` on that view; this trait does not construct or return views.
///
/// The interface supports generic callers and delegates through references, `Box` and `Arc`.
/// Collection is asynchronous so implementations can await access to component state (for example,
/// an async lock or a request to a component task). Collectors that can snapshot immediately can
/// return a ready future. Both the collection future and its owned result can be sent between tasks.
///
/// ```
/// use hopr_api::StateDump;
///
/// async fn serialize_dump<T: StateDump>(source: &T) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
///     let snapshot = source.state_dump().await?;
///     Ok(serde_json::to_string(&snapshot)?)
/// }
/// ```
pub trait StateDump: Send + Sync {
    /// Diagnostic collection error; this does not assign an operational failure state.
    type Error: Error + Send + Sync + 'static;

    /// Collect an owned snapshot without exposing its concrete type.
    fn state_dump(
        &self,
    ) -> impl Future<Output = Result<impl Serialize + DeserializeOwned + Send + 'static, Self::Error>> + Send;
}

impl<T: StateDump + ?Sized> StateDump for &T {
    type Error = T::Error;

    fn state_dump(
        &self,
    ) -> impl Future<Output = Result<impl Serialize + DeserializeOwned + Send + 'static, Self::Error>> + Send {
        (**self).state_dump()
    }
}
impl<T: StateDump + ?Sized> StateDump for Box<T> {
    type Error = T::Error;

    fn state_dump(
        &self,
    ) -> impl Future<Output = Result<impl Serialize + DeserializeOwned + Send + 'static, Self::Error>> + Send {
        (**self).state_dump()
    }
}
impl<T: StateDump + ?Sized> StateDump for Arc<T> {
    type Error = T::Error;

    fn state_dump(
        &self,
    ) -> impl Future<Output = Result<impl Serialize + DeserializeOwned + Send + 'static, Self::Error>> + Send {
        (**self).state_dump()
    }
}

#[cfg(test)]
mod tests {
    use std::{
        convert::Infallible,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
    };

    use serde::{Deserialize, Serialize};

    use super::StateDump;

    // This model is deliberately private: a public associated snapshot type would expose it.
    #[derive(Serialize, Deserialize)]
    struct PrivateSnapshot {
        queue: usize,
    }
    struct Queue(AtomicUsize);
    impl StateDump for Queue {
        type Error = Infallible;

        async fn state_dump(
            &self,
        ) -> Result<impl Serialize + serde::de::DeserializeOwned + Send + 'static, Self::Error> {
            Ok(PrivateSnapshot {
                queue: self.0.load(Ordering::Relaxed),
            })
        }
    }

    async fn read<T: StateDump + ?Sized>(source: &T) -> anyhow::Result<serde_json::Value> {
        // The generic interface must guarantee the future is Send.
        fn send<T: Send>(value: T) -> T {
            value
        }
        Ok(serde_json::to_value(send(source.state_dump()).await?)?)
    }

    #[tokio::test]
    async fn generic_callers_can_poll_private_snapshots_through_shared_handles() -> anyhow::Result<()> {
        let queue = Arc::new(Queue(AtomicUsize::new(3)));
        assert_eq!(read(&queue).await?["queue"], 3);
        queue.0.store(7, Ordering::Relaxed);
        assert_eq!(read(&queue).await?["queue"], 7);
        let borrowed = &*queue;
        assert_eq!(read(&borrowed).await?["queue"], 7);
        let boxed = Box::new(Queue(AtomicUsize::new(11)));
        assert_eq!(read(&boxed).await?["queue"], 11);
        let snapshot = {
            let temporary = Queue(AtomicUsize::new(13));
            temporary.state_dump().await?
        };
        let json = serde_json::to_string(&snapshot)?;
        assert_eq!(serde_json::from_str::<PrivateSnapshot>(&json)?.queue, 13);
        Ok(())
    }

    struct Unavailable;
    impl StateDump for Unavailable {
        type Error = std::io::Error;

        async fn state_dump(
            &self,
        ) -> Result<impl Serialize + serde::de::DeserializeOwned + Send + 'static, Self::Error> {
            Err::<(), _>(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "collection unavailable",
            ))
        }
    }

    #[tokio::test]
    async fn collection_errors_remain_distinct_from_operational_state() {
        let result = Unavailable.state_dump().await;
        let error = match result {
            Err(error) => error,
            Ok(_) => panic!("expected collection failure"),
        };
        assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
    }
}
