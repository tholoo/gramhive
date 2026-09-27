//! Offline application testing, with the same response executor as production.
#![forbid(unsafe_code)]
use gramhive_core::*;
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct Recording {
    operations: Vec<Operation>,
    progress: Vec<ProgressEvent>,
    next_id: i32,
    fail_at: Option<usize>,
}
#[derive(Default)]
pub struct FakeDriver {
    recording: Mutex<Recording>,
}
impl FakeDriver {
    /// Fail on the zero-based operation index, after recording the attempted operation.
    pub fn failing_at(index: usize) -> Self {
        Self {
            recording: Mutex::new(Recording {
                fail_at: Some(index),
                ..Recording::default()
            }),
        }
    }
    pub fn operations(&self) -> Vec<Operation> {
        self.recording.lock().unwrap().operations.clone()
    }
    pub fn progress(&self) -> Vec<ProgressEvent> {
        self.recording.lock().unwrap().progress.clone()
    }
}
impl Driver for FakeDriver {
    fn execute(&self, operation: Operation) -> BoxFuture<'_, Result<Option<i32>, Failure>> {
        Box::pin(async move {
            let mut r = self.recording.lock().unwrap();
            let sends = matches!(operation, Operation::Send { .. });
            let index = r.operations.len();
            r.operations.push(operation);
            if r.fail_at == Some(index) {
                return Err(Failure("injected driver failure".into()));
            }
            if sends {
                r.next_id += 1;
                Ok(Some(r.next_id))
            } else {
                Ok(None)
            }
        })
    }
    fn progress(&self, event: ProgressEvent) {
        self.recording.lock().unwrap().progress.push(event);
    }
}

pub struct TestApp<S = ()> {
    router: Router<S>,
}
impl<S: Send + Sync + 'static> TestApp<S> {
    pub fn new(router: Router<S>) -> Self {
        Self { router }
    }
    pub fn message(&self, text: impl Into<String>) -> EventBuilder<S> {
        self.event(EventKind::Message(MessageInfo {
            id: 100,
            text: Some(text.into()),
        }))
    }
    pub fn media(&self) -> EventBuilder<S> {
        self.event(EventKind::Message(MessageInfo {
            id: 100,
            text: None,
        }))
    }
    /// Panics if the fixture cannot be encoded. Use try_callback to assert encoding errors.
    pub fn callback(&self, data: impl CallbackData) -> EventBuilder<S> {
        self.try_callback(data).expect("invalid callback fixture")
    }
    pub fn try_callback(
        &self,
        data: impl CallbackData,
    ) -> Result<EventBuilder<S>, CallbackDataError> {
        Ok(self.callback_bytes(data.encode()?))
    }
    pub fn callback_bytes(&self, data: impl Into<Vec<u8>>) -> EventBuilder<S> {
        self.event(EventKind::Callback(CallbackInfo {
            id: 200,
            data: data.into(),
        }))
    }
    pub fn event(&self, kind: EventKind) -> EventBuilder<S> {
        let driver = Arc::new(FakeDriver::default());
        EventBuilder {
            router: self.router.clone(),
            event: Event {
                kind,
                account: AccountInfo {
                    name: "test".into(),
                    username: Some("test_bot".into()),
                },
                chat: Some(1),
                sender: Some(42),
                driver: driver.clone(),
                raw: None,
            },
            driver,
        }
    }
}
pub struct EventBuilder<S> {
    router: Router<S>,
    event: Event,
    driver: Arc<FakeDriver>,
}
impl<S: Send + Sync + 'static> EventBuilder<S> {
    pub fn from_user(mut self, user: i64) -> Self {
        self.event.sender = Some(user);
        self
    }
    pub fn in_chat(mut self, chat: i64) -> Self {
        self.event.chat = Some(chat);
        self
    }
    pub fn on_account(mut self, name: impl Into<String>, username: Option<&str>) -> Self {
        self.event.account = AccountInfo {
            name: name.into(),
            username: username.map(str::to_owned),
        };
        self
    }
    pub fn raw(mut self, raw: Arc<dyn RawSource>) -> Self {
        self.event.raw = Some(raw);
        self
    }
    pub fn failing_at(mut self, index: usize) -> Self {
        self.driver = Arc::new(FakeDriver::failing_at(index));
        self.event.driver = self.driver.clone();
        self
    }
    pub async fn send(self) -> TestResult {
        let outcome = self.router.handle(self.event).await;
        TestResult {
            outcome,
            operations: self.driver.operations(),
            progress: self.driver.progress(),
        }
    }

    /// Exercise cancellation and temporary progress cleanup without a live runtime.
    pub async fn send_until<F>(self, cancel: F, cleanup_timeout: std::time::Duration) -> TestResult
    where
        F: std::future::Future<Output = ()> + Send,
    {
        let outcome = self
            .router
            .handle_until(self.event, cancel, cleanup_timeout)
            .await;
        TestResult {
            outcome,
            operations: self.driver.operations(),
            progress: self.driver.progress(),
        }
    }
}
#[derive(Debug)]
pub struct TestResult {
    pub outcome: Result<Dispatch, Failure>,
    pub operations: Vec<Operation>,
    pub progress: Vec<ProgressEvent>,
}
impl TestResult {
    pub fn assert_handled(&self) {
        assert_eq!(self.outcome, Ok(Dispatch::Handled));
    }
    pub fn assert_not_matched(&self) {
        assert_eq!(self.outcome, Ok(Dispatch::NotMatched));
        assert!(self.operations.is_empty());
    }
    pub fn assert_rejected(&self) {
        assert!(
            matches!(self.outcome, Ok(Dispatch::Rejected(_))),
            "{self:?}"
        );
    }

    /// Assert a user-facing reply or callback notice while retaining the failure/rejection outcome.
    pub fn assert_notice(&self, text: &str) {
        assert!(
            self.operations.iter().any(|op| match op {
                Operation::Send { body, reply: true } => body.text == text,
                Operation::Answer {
                    text: Some(message),
                    ..
                } => message == text,
                _ => false,
            }),
            "missing notice {text:?}: {self:?}"
        );
    }

    pub fn assert_reply(&self, text: &str) {
        self.assert_handled();
        assert!(
            self.operations
                .iter()
                .any(|op| matches!(op, Operation::Send { body, reply: true } if body.text == text)),
            "missing reply {text:?}: {self:?}"
        );
    }
    pub fn assert_edit(&self, text: &str) {
        self.assert_handled();
        assert!(
            self.operations
                .iter()
                .any(|op| matches!(op, Operation::Edit { body, .. } if body.text == text)),
            "missing edit {text:?}: {self:?}"
        );
    }
    pub fn assert_callback_answered(&self) {
        self.assert_handled();
        assert!(
            self.operations
                .iter()
                .any(|op| matches!(op, Operation::Answer { .. })),
            "callback was not answered: {self:?}"
        );
    }
    pub fn assert_progress(&self, expected: impl IntoIterator<Item = ProgressEvent>) {
        self.assert_handled();
        assert_eq!(self.progress, expected.into_iter().collect::<Vec<_>>());
    }
}
