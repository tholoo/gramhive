use crate::{BoxFuture, Dispatch, Driver, Event, Failure, Operation, ProgressEvent, Router};
use futures::future::join_all;
use std::{
    collections::BTreeSet,
    future::Future,
    sync::{Arc, Mutex},
    time::Duration,
};

struct CleanupDriver {
    inner: Arc<dyn Driver>,
    messages: Mutex<BTreeSet<i32>>,
}

impl Driver for CleanupDriver {
    fn execute(&self, operation: Operation) -> BoxFuture<'_, Result<Option<i32>, Failure>> {
        self.inner.execute(operation)
    }

    fn progress(&self, event: ProgressEvent) {
        self.inner.progress(event);
    }

    fn track_temporary(&self, message: i32) {
        self.messages.lock().unwrap().insert(message);
        self.inner.track_temporary(message);
    }

    fn forget_temporary(&self, message: i32) {
        self.messages.lock().unwrap().remove(&message);
        self.inner.forget_temporary(message);
    }
}

impl<S: Send + Sync + 'static> Router<S> {
    /// Cancel handlers, middleware, and responses when `cancel` resolves, then attempt
    /// deletion of known temporary progress messages within one shared cleanup budget.
    /// Dropping this future directly skips cleanup; await it to completion.
    pub async fn handle_until<F>(
        &self,
        mut event: Event,
        cancel: F,
        cleanup_timeout: Duration,
    ) -> Result<Dispatch, Failure>
    where
        F: Future<Output = ()> + Send,
    {
        let driver = Arc::new(CleanupDriver {
            inner: event.driver.clone(),
            messages: Mutex::new(BTreeSet::new()),
        });
        event.driver = driver.clone();
        // The losing handle future is dropped before cleanup starts, so it cannot
        // continue editing/deleting concurrently with cleanup.
        tokio::select! {
            biased;
            _ = cancel => {}
            result = self.handle(event) => return result,
        }
        let messages: Vec<_> = driver.messages.lock().unwrap().iter().copied().collect();
        let cleanup = join_all(messages.into_iter().map(|message| {
            let driver = driver.clone();
            async move {
                match driver
                    .inner
                    .execute(Operation::Delete {
                        message: Some(message),
                    })
                    .await
                {
                    Ok(_) => driver.forget_temporary(message),
                    Err(error) => tracing::warn!(%error, "cancelled progress cleanup failed"),
                }
            }
        }));
        if tokio::time::timeout(cleanup_timeout, cleanup)
            .await
            .is_err()
        {
            tracing::warn!("cancelled progress cleanup timed out");
        }
        Ok(Dispatch::Cancelled)
    }
}
