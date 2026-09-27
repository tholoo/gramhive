use crate::{BoxFuture, CallbackData, CallbackDataError, Progress, codec};
use futures::future::join_all;
use std::{path::PathBuf, sync::Arc};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct Failure(pub String);
impl IntoResponse for Failure {
    fn into_response(self) -> Response {
        Response::Failed(self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Button {
    pub label: String,
    data: Vec<u8>,
}
impl Button {
    pub fn callback(
        label: impl Into<String>,
        data: impl CallbackData,
    ) -> Result<Self, CallbackDataError> {
        let data = data.encode()?;
        codec::check(&data)?;
        Ok(Self {
            label: label.into(),
            data,
        })
    }
    pub fn data(&self) -> &[u8] {
        &self.data
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Reply {
    pub text: String,
    pub file: Option<PathBuf>,
    pub buttons: Vec<Vec<Button>>,
}
impl Reply {
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            ..Self::default()
        }
    }
    pub fn buttons(mut self, rows: Vec<Vec<Button>>) -> Self {
        self.buttons = rows;
        self
    }
}
pub struct File(PathBuf);
impl File {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self(path.into())
    }
}
impl From<File> for Reply {
    fn from(file: File) -> Self {
        Self {
            file: Some(file.0),
            ..Self::default()
        }
    }
}
pub struct SendMessage(pub Reply);
pub struct Edit(pub Reply);
pub struct Delete;
pub struct NoResponse;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Operation {
    Send { body: Reply, reply: bool },
    Edit { message: Option<i32>, body: Reply },
    Delete { message: Option<i32> },
    Answer { callback: i64, text: Option<String> },
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProgressEvent {
    Update(String),
    Finished(Reply),
}

/// Bound to one event/account/peer by the transport. Only Send returns a message ID.
pub trait Driver: Send + Sync {
    fn execute(&self, operation: Operation) -> BoxFuture<'_, Result<Option<i32>, Failure>>;
    /// Semantic observation hook used by test drivers; not a Telegram operation.
    fn progress(&self, _event: ProgressEvent) {}
    /// Executor bookkeeping for cancellation; transports normally leave these as no-ops.
    fn track_temporary(&self, _message: i32) {}
    fn forget_temporary(&self, _message: i32) {}
}
#[derive(Clone)]
pub struct Telegram {
    driver: Arc<dyn Driver>,
}
impl Telegram {
    pub fn new(driver: Arc<dyn Driver>) -> Self {
        Self { driver }
    }
    pub async fn send(&self, text: impl Into<String>) -> Result<SentMessage, Failure> {
        let id = self
            .driver
            .execute(Operation::Send {
                body: Reply::text(text),
                reply: false,
            })
            .await?
            .ok_or_else(|| Failure("driver did not return a sent message ID".into()))?;
        Ok(SentMessage {
            id,
            driver: self.driver.clone(),
        })
    }
}
pub struct SentMessage {
    id: i32,
    driver: Arc<dyn Driver>,
}
impl SentMessage {
    pub async fn edit(&self, text: impl Into<String>) -> Result<(), Failure> {
        self.driver
            .execute(Operation::Edit {
                message: Some(self.id),
                body: Reply::text(text),
            })
            .await
            .map(drop)
    }
    pub async fn delete(self) -> Result<(), Failure> {
        self.driver
            .execute(Operation::Delete {
                message: Some(self.id),
            })
            .await
            .map(drop)
    }
}

pub enum Response {
    None,
    Operation(Operation),
    Parallel(Vec<Response>),
    Sequence(Vec<Response>),
    Progress(Progress),
    Failed(Failure),
}
pub trait IntoResponse {
    fn into_response(self) -> Response;
}
impl IntoResponse for Response {
    fn into_response(self) -> Response {
        self
    }
}
impl IntoResponse for () {
    fn into_response(self) -> Response {
        Response::None
    }
}
impl IntoResponse for NoResponse {
    fn into_response(self) -> Response {
        Response::None
    }
}
impl IntoResponse for Reply {
    fn into_response(self) -> Response {
        Response::Operation(Operation::Send {
            body: self,
            reply: true,
        })
    }
}
impl IntoResponse for File {
    fn into_response(self) -> Response {
        Reply::from(self).into_response()
    }
}
impl IntoResponse for SendMessage {
    fn into_response(self) -> Response {
        Response::Operation(Operation::Send {
            body: self.0,
            reply: false,
        })
    }
}
impl IntoResponse for Edit {
    fn into_response(self) -> Response {
        Response::Operation(Operation::Edit {
            message: None,
            body: self.0,
        })
    }
}
impl IntoResponse for Delete {
    fn into_response(self) -> Response {
        Response::Operation(Operation::Delete { message: None })
    }
}
impl<T: IntoResponse, E: IntoResponse> IntoResponse for Result<T, E> {
    fn into_response(self) -> Response {
        match self {
            Ok(v) => v.into_response(),
            Err(e) => e.into_response(),
        }
    }
}
pub struct Parallel(Vec<Response>);
pub struct Sequence(Vec<Response>);
macro_rules! composition {
    ($name:ident, $variant:ident) => {
        impl $name {
            pub fn new(items: impl IntoIterator<Item = Response>) -> Self {
                Self(items.into_iter().collect())
            }
        }
        impl IntoResponse for $name {
            fn into_response(self) -> Response {
                Response::$variant(self.0)
            }
        }
    };
}
composition!(Parallel, Parallel);
composition!(Sequence, Sequence);

pub struct AnswerCallback {
    id: i64,
    text: Option<String>,
    followup: Option<Response>,
}
impl AnswerCallback {
    pub fn new(id: i64) -> Self {
        Self {
            id,
            text: None,
            followup: None,
        }
    }
    pub fn text(mut self, text: impl Into<String>) -> Self {
        self.text = Some(text.into());
        self
    }
    pub fn edit(mut self, text: impl Into<String>) -> Self {
        self.followup = Some(Edit(Reply::text(text)).into_response());
        self
    }
    pub fn reply(mut self, text: impl Into<String>) -> Self {
        self.followup = Some(Reply::text(text).into_response());
        self
    }
}
impl IntoResponse for AnswerCallback {
    fn into_response(self) -> Response {
        let answer = Response::Operation(Operation::Answer {
            callback: self.id,
            text: self.text,
        });
        match self.followup {
            Some(next) => Response::Parallel(vec![answer, next]),
            None => answer,
        }
    }
}

/// Shared by production and test drivers. Parallel waits for all children, even on failure.
/// Sequence stops at the first failure. Neither composition offers rollback.
pub fn execute(
    response: Response,
    driver: Arc<dyn Driver>,
) -> BoxFuture<'static, Result<(), Failure>> {
    Box::pin(async move {
        match response {
            Response::None => Ok(()),
            Response::Failed(e) => Err(e),
            Response::Operation(op) => driver.execute(op).await.map(drop),
            Response::Parallel(items) => {
                let results =
                    join_all(items.into_iter().map(|item| execute(item, driver.clone()))).await;
                results.into_iter().collect::<Result<Vec<_>, _>>().map(drop)
            }
            Response::Sequence(items) => {
                for item in items {
                    execute(item, driver.clone()).await?;
                }
                Ok(())
            }
            Response::Progress(progress) => progress.execute(driver).await,
        }
    })
}
