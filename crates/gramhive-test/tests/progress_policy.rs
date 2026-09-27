use gramhive_core::*;
use gramhive_test::{FakeDriver, TestApp};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

const SECOND: Duration = Duration::from_secs(1);

#[tokio::test(start_paused = true)]
async fn throttle_flushes_the_latest_pending_update_without_waiting_for_another_item() {
    let driver = Arc::new(FakeDriver::default());
    let progress = Progress::editing(async_stream::stream! {
        yield ProgressItem::update("first");
        yield ProgressItem::update("superseded");
        yield ProgressItem::update("latest");
        tokio::time::sleep(SECOND * 2).await;
        yield ProgressItem::finish(Reply::text("done"));
    })
    .throttle(SECOND);
    let task = tokio::spawn(execute(progress.into_response(), driver.clone()));
    tokio::task::yield_now().await;
    assert_eq!(driver.operations().len(), 1);
    tokio::time::advance(SECOND).await;
    tokio::task::yield_now().await;
    assert_eq!(driver.operations().len(), 2);
    assert!(
        matches!(&driver.operations()[1], Operation::Edit { body, .. } if body.text == "latest")
    );
    task.await.unwrap().unwrap();
    assert_eq!(driver.operations().len(), 3);
    assert_eq!(driver.progress().len(), 4); // Semantics are not coalesced.
}

#[tokio::test(start_paused = true)]
async fn finish_bypasses_throttling_and_discards_a_pending_update() {
    let driver = Arc::new(FakeDriver::default());
    let start = tokio::time::Instant::now();
    execute(
        Progress::editing(futures::stream::iter([
            ProgressItem::update("first"),
            ProgressItem::update("pending"),
            ProgressItem::finish(Reply::text("done")),
        ]))
        .throttle(SECOND * 60)
        .into_response(),
        driver.clone(),
    )
    .await
    .unwrap();
    assert_eq!(start.elapsed(), Duration::ZERO);
    assert_eq!(driver.operations().len(), 2);
    assert!(matches!(&driver.operations()[1], Operation::Edit { body, .. } if body.text == "done"));
}

fn stalled(temporary: bool) -> Progress {
    let stream = async_stream::stream! {
        yield ProgressItem::update("working");
        futures::future::pending::<()>().await;
        yield ProgressItem::finish(Reply::text("never"));
    };
    if temporary {
        Progress::temporary(stream)
    } else {
        Progress::editing(stream)
    }
}

#[tokio::test(start_paused = true)]
async fn cancellation_cleans_temporary_progress_but_retains_editing_progress() {
    for temporary in [true, false] {
        let app =
            TestApp::new(Router::new().route(text(), move || async move { stalled(temporary) }));
        let result = app
            .message("go")
            .send_until(tokio::time::sleep(SECOND), SECOND)
            .await;
        assert_eq!(result.outcome, Ok(Dispatch::Cancelled));
        assert_eq!(result.operations.len(), if temporary { 2 } else { 1 });
        if temporary {
            assert!(matches!(
                result.operations[1],
                Operation::Delete { message: Some(1) }
            ));
        }
    }
}

#[tokio::test(start_paused = true)]
async fn cancellation_cleans_parallel_statuses_and_does_not_delete_completed_results() {
    let app = TestApp::new(Router::new().route(text(), || async {
        Parallel::new([stalled(true).into_response(), stalled(true).into_response()])
    }));
    let result = app
        .message("go")
        .send_until(tokio::time::sleep(SECOND), SECOND)
        .await;
    assert_eq!(result.outcome, Ok(Dispatch::Cancelled));
    assert_eq!(
        result
            .operations
            .iter()
            .filter(|op| matches!(op, Operation::Delete { .. }))
            .count(),
        2
    );

    let app = TestApp::new(Router::new().route(text(), || async {
        Sequence::new([
            Progress::temporary(futures::stream::iter([
                ProgressItem::update("status"),
                ProgressItem::finish(Reply::text("done")),
            ]))
            .into_response(),
            stalled(false).into_response(),
        ])
    }));
    let result = app
        .message("go")
        .send_until(tokio::time::sleep(SECOND), SECOND)
        .await;
    assert_eq!(result.outcome, Ok(Dispatch::Cancelled));
    assert_eq!(
        result
            .operations
            .iter()
            .filter(|op| matches!(op, Operation::Delete { .. }))
            .count(),
        1
    );
}

struct Dropped(Arc<AtomicBool>);
impl Drop for Dropped {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

#[tokio::test(start_paused = true)]
async fn cancellation_drops_a_handler_before_it_returns_a_response() {
    let dropped = Arc::new(AtomicBool::new(false));
    let state = dropped.clone();
    let app = TestApp::new(Router::with_state(state).route(
        text(),
        async |State(state): State<Arc<AtomicBool>>| {
            let _guard = Dropped(state);
            futures::future::pending::<()>().await;
        },
    ));
    let result = app
        .message("go")
        .send_until(tokio::time::sleep(SECOND), SECOND)
        .await;
    assert_eq!(result.outcome, Ok(Dispatch::Cancelled));
    assert!(dropped.load(Ordering::SeqCst));
    assert!(result.operations.is_empty());
}

struct HungDelete(FakeDriver);
impl Driver for HungDelete {
    fn execute(&self, operation: Operation) -> BoxFuture<'_, Result<Option<i32>, Failure>> {
        Box::pin(async move {
            let deleting = matches!(operation, Operation::Delete { .. });
            let result = self.0.execute(operation).await;
            if deleting {
                futures::future::pending::<()>().await;
            }
            result
        })
    }
}

#[tokio::test(start_paused = true)]
async fn cancellation_cleanup_has_a_bounded_budget_even_if_the_driver_hangs() {
    let driver = Arc::new(HungDelete(FakeDriver::default()));
    let event = Event {
        account: AccountInfo {
            name: "test".into(),
            username: None,
        },
        chat: Some(1),
        sender: None,
        kind: EventKind::Message(MessageInfo {
            id: 1,
            text: Some("go".into()),
        }),
        driver: driver.clone(),
        raw: None,
    };
    let router = Router::new().route(text(), || async { stalled(true) });
    let start = tokio::time::Instant::now();
    let outcome = router
        .handle_until(event, tokio::time::sleep(SECOND), SECOND * 2)
        .await;
    assert_eq!(outcome, Ok(Dispatch::Cancelled));
    assert_eq!(start.elapsed(), SECOND * 3);
    assert!(matches!(
        driver.0.operations().last(),
        Some(Operation::Delete { .. })
    ));
}
