use crate::{Driver, Failure, IntoResponse, Operation, ProgressEvent, Reply, Response, Stream};
use futures::StreamExt;
use std::{pin::Pin, sync::Arc, time::Duration};
use tokio::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgressStrategy {
    Editing,
    Temporary,
}

pub enum ProgressItem {
    Update(String),
    Finish(Reply),
}

impl ProgressItem {
    pub fn update(text: impl Into<String>) -> Self {
        Self::Update(text.into())
    }
    pub fn finish(body: impl Into<Reply>) -> Self {
        Self::Finish(body.into())
    }
}

pub trait IntoProgressItem {
    fn into_progress_item(self) -> Result<ProgressItem, Failure>;
}

impl IntoProgressItem for ProgressItem {
    fn into_progress_item(self) -> Result<ProgressItem, Failure> {
        Ok(self)
    }
}

impl IntoProgressItem for Result<ProgressItem, Failure> {
    fn into_progress_item(self) -> Result<ProgressItem, Failure> {
        self
    }
}

pub struct Progress {
    strategy: ProgressStrategy,
    stream: Pin<Box<dyn Stream<Item = Result<ProgressItem, Failure>> + Send>>,
    interval: Duration,
}

impl Progress {
    pub fn editing<S>(stream: S) -> Self
    where
        S: Stream + Send + 'static,
        S::Item: IntoProgressItem,
    {
        Self::new(ProgressStrategy::Editing, stream)
    }

    pub fn temporary<S>(stream: S) -> Self
    where
        S: Stream + Send + 'static,
        S::Item: IntoProgressItem,
    {
        Self::new(ProgressStrategy::Temporary, stream)
    }

    /// Send the first update immediately, coalesce later updates to the latest pending
    /// value, and deliver Finish immediately. Zero disables throttling (the default).
    /// Semantic progress observations still include every yielded item.
    pub fn throttle(mut self, interval: Duration) -> Self {
        self.interval = interval;
        self
    }

    fn new<S>(strategy: ProgressStrategy, stream: S) -> Self
    where
        S: Stream + Send + 'static,
        S::Item: IntoProgressItem,
    {
        Self {
            strategy,
            stream: Box::pin(stream.map(IntoProgressItem::into_progress_item)),
            interval: Duration::ZERO,
        }
    }

    pub(crate) async fn execute(mut self, driver: Arc<dyn Driver>) -> Result<(), Failure> {
        let mut message = None;
        let result = self.deliver(&driver, &mut message).await;
        if result.is_err()
            && self.strategy == ProgressStrategy::Temporary
            && let Some(id) = message
        {
            match driver
                .execute(Operation::Delete { message: Some(id) })
                .await
            {
                Ok(_) => driver.forget_temporary(id),
                Err(error) => tracing::warn!(%error, "temporary progress cleanup failed"),
            }
        }
        result
    }

    async fn deliver(
        &mut self,
        driver: &Arc<dyn Driver>,
        message: &mut Option<i32>,
    ) -> Result<(), Failure> {
        let mut pending = None;
        let mut next_update = Instant::now();
        loop {
            let item = if pending.is_some() {
                tokio::select! {
                    // Prefer a ready Finish or newer update over a stale pending edit.
                    biased;
                    item = self.stream.next() => item,
                    _ = tokio::time::sleep_until(next_update) => {
                        self.update(driver, message, pending.take().unwrap()).await?;
                        next_update = Instant::now() + self.interval;
                        continue;
                    }
                }
            } else {
                self.stream.next().await
            };
            match item
                .ok_or_else(|| Failure("progress stream ended without a final result".into()))??
            {
                ProgressItem::Update(text) => {
                    driver.progress(ProgressEvent::Update(text.clone()));
                    if message.is_none() || Instant::now() >= next_update {
                        pending = None;
                        self.update(driver, message, text).await?;
                        next_update = Instant::now() + self.interval;
                    } else {
                        pending = Some(text);
                    }
                }
                ProgressItem::Finish(body) => {
                    driver.progress(ProgressEvent::Finished(body.clone()));
                    match (self.strategy, *message) {
                        (ProgressStrategy::Editing, Some(id)) => {
                            driver
                                .execute(Operation::Edit {
                                    message: Some(id),
                                    body,
                                })
                                .await?;
                        }
                        (ProgressStrategy::Temporary, Some(id)) => {
                            driver
                                .execute(Operation::Delete { message: Some(id) })
                                .await?;
                            driver.forget_temporary(id);
                            *message = None;
                            driver
                                .execute(Operation::Send { body, reply: true })
                                .await?;
                        }
                        (_, None) => {
                            driver
                                .execute(Operation::Send { body, reply: true })
                                .await?;
                        }
                    }
                    return Ok(());
                }
            }
        }
    }

    async fn update(
        &mut self,
        driver: &Arc<dyn Driver>,
        message: &mut Option<i32>,
        text: String,
    ) -> Result<(), Failure> {
        let body = Reply::text(text);
        if let Some(id) = *message {
            driver
                .execute(Operation::Edit {
                    message: Some(id),
                    body,
                })
                .await?;
        } else {
            let id = driver
                .execute(Operation::Send { body, reply: true })
                .await?
                .ok_or_else(|| Failure("driver did not return a progress message ID".into()))?;
            *message = Some(id);
            if self.strategy == ProgressStrategy::Temporary {
                driver.track_temporary(id);
            }
        }
        Ok(())
    }
}

impl IntoResponse for Progress {
    fn into_response(self) -> Response {
        Response::Progress(self)
    }
}
