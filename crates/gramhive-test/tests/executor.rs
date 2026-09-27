use gramhive_core::*;
use gramhive_test::FakeDriver;
use std::sync::{Arc, Mutex};

fn stream() -> impl Stream<Item = ProgressItem> {
    futures::stream::iter([
        ProgressItem::update("one"),
        ProgressItem::update("two"),
        ProgressItem::finish(Reply::text("done")),
    ])
}
#[tokio::test]
async fn editing_reuses_the_sent_message() {
    let driver = Arc::new(FakeDriver::default());
    execute(Progress::editing(stream()).into_response(), driver.clone())
        .await
        .unwrap();
    assert_eq!(
        driver.operations(),
        vec![
            Operation::Send {
                body: Reply::text("one"),
                reply: true
            },
            Operation::Edit {
                message: Some(1),
                body: Reply::text("two")
            },
            Operation::Edit {
                message: Some(1),
                body: Reply::text("done")
            },
        ]
    );
}
#[tokio::test]
async fn temporary_deletes_before_sending_the_final_result() {
    let driver = Arc::new(FakeDriver::default());
    execute(
        Progress::temporary(stream()).into_response(),
        driver.clone(),
    )
    .await
    .unwrap();
    assert_eq!(
        driver.operations(),
        vec![
            Operation::Send {
                body: Reply::text("one"),
                reply: true
            },
            Operation::Edit {
                message: Some(1),
                body: Reply::text("two")
            },
            Operation::Delete { message: Some(1) },
            Operation::Send {
                body: Reply::text("done"),
                reply: true
            },
        ]
    );
}
#[tokio::test]
async fn final_without_updates_sends_once_and_stops_polling() {
    let driver = Arc::new(FakeDriver::default());
    let stream = futures::stream::iter([
        ProgressItem::finish(Reply::text("done")),
        ProgressItem::update("never"),
    ]);
    execute(Progress::editing(stream).into_response(), driver.clone())
        .await
        .unwrap();
    assert_eq!(driver.operations().len(), 1);
    assert_eq!(driver.progress().len(), 1);
}
#[tokio::test]
async fn fallible_streams_and_missing_final_are_errors_with_temporary_cleanup() {
    for stream in [
        vec![Ok(ProgressItem::update("one"))],
        vec![
            Ok(ProgressItem::update("one")),
            Err(Failure("task failed".into())),
        ],
    ] {
        let driver = Arc::new(FakeDriver::default());
        assert!(
            execute(
                Progress::temporary(futures::stream::iter(stream)).into_response(),
                driver.clone()
            )
            .await
            .is_err()
        );
        assert!(matches!(
            driver.operations().last(),
            Some(Operation::Delete { message: Some(1) })
        ));
    }
    let driver = Arc::new(FakeDriver::failing_at(1));
    assert!(
        execute(
            Progress::temporary(stream()).into_response(),
            driver.clone()
        )
        .await
        .is_err()
    );
    assert!(matches!(
        driver.operations().last(),
        Some(Operation::Delete { .. })
    ));
}
#[tokio::test]
async fn sequence_stops_on_failure_parallel_finishes_all_children() {
    let driver = Arc::new(FakeDriver::failing_at(0));
    let result = execute(
        Sequence::new([
            Reply::text("one").into_response(),
            Reply::text("two").into_response(),
        ])
        .into_response(),
        driver.clone(),
    )
    .await;
    assert!(result.is_err());
    assert_eq!(driver.operations().len(), 1);
    let driver = Arc::new(FakeDriver::failing_at(0));
    let result = execute(
        Parallel::new([
            Reply::text("one").into_response(),
            Reply::text("two").into_response(),
        ])
        .into_response(),
        driver.clone(),
    )
    .await;
    assert!(result.is_err());
    assert_eq!(driver.operations().len(), 2);
}
struct RendezvousDriver {
    barrier: tokio::sync::Barrier,
    operations: Mutex<Vec<Operation>>,
}
impl Driver for RendezvousDriver {
    fn execute(&self, operation: Operation) -> BoxFuture<'_, Result<Option<i32>, Failure>> {
        Box::pin(async move {
            self.operations.lock().unwrap().push(operation);
            self.barrier.wait().await;
            Ok(None)
        })
    }
}
#[tokio::test]
async fn callback_answer_and_edit_are_polled_concurrently() {
    let driver = Arc::new(RendezvousDriver {
        barrier: tokio::sync::Barrier::new(2),
        operations: Mutex::new(Vec::new()),
    });
    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        execute(
            CallbackInfo {
                id: 9,
                data: vec![],
            }
            .answer()
            .edit("done")
            .into_response(),
            driver.clone(),
        ),
    )
    .await
    .expect("executor serialized independent RPCs")
    .unwrap();
    assert_eq!(driver.operations.lock().unwrap().len(), 2);
}
#[tokio::test]
async fn file_final_is_semantic_until_the_transport_executes_it() {
    let driver = Arc::new(FakeDriver::default());
    execute(
        Progress::temporary(futures::stream::iter([
            ProgressItem::update("convert"),
            ProgressItem::finish(File::new("result.txt")),
        ]))
        .into_response(),
        driver.clone(),
    )
    .await
    .unwrap();
    assert!(
        matches!(driver.operations().last(), Some(Operation::Send { body, .. }) if body.file.as_deref() == Some(std::path::Path::new("result.txt")))
    );
}
