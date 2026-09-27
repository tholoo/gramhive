use gramhive_core::*;
use gramhive_test::TestApp;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

struct Record {
    label: &'static str,
    log: Arc<Mutex<Vec<String>>>,
}
impl Middleware for Record {
    fn handle(&self, event: Event, next: Next) -> BoxFuture<'static, Result<Dispatch, Failure>> {
        let label = self.label;
        let log = self.log.clone();
        Box::pin(async move {
            log.lock().unwrap().push(format!("{label} before"));
            let driver = event.driver.clone();
            let result = next.run(event).await;
            // A middleware after-hook runs after the response is executed.
            driver
                .execute(Operation::Answer {
                    callback: 0,
                    text: Some(label.into()),
                })
                .await?;
            log.lock().unwrap().push(format!("{label} after"));
            result
        })
    }
}
#[tokio::test]
async fn last_layer_is_outermost_and_wraps_response_execution() {
    let log = Arc::new(Mutex::new(vec![]));
    let app = TestApp::new(
        Router::new()
            .route(text(), || async { Reply::text("hi") })
            .layer(Record {
                label: "inner",
                log: log.clone(),
            })
            .layer(Record {
                label: "outer",
                log: log.clone(),
            }),
    );
    let result = app.message("hi").send().await;
    assert_eq!(
        *log.lock().unwrap(),
        ["outer before", "inner before", "inner after", "outer after"]
    );
    assert!(matches!(result.operations[0], Operation::Send { .. }));
}
#[tokio::test]
async fn concurrency_limit_covers_long_running_response_streams() {
    let active = Arc::new(AtomicUsize::new(0));
    let maximum = Arc::new(AtomicUsize::new(0));
    let app = TestApp::new(
        Router::with_state((active.clone(), maximum.clone()))
            .route(
                text(),
                async |State((active, maximum)): State<(Arc<AtomicUsize>, Arc<AtomicUsize>)>| {
                    Progress::editing(async_stream::stream! {
                        let count = active.fetch_add(1, Ordering::SeqCst) + 1;
                        maximum.fetch_max(count, Ordering::SeqCst);
                        yield ProgressItem::update("working");
                        tokio::task::yield_now().await;
                        active.fetch_sub(1, Ordering::SeqCst);
                        yield ProgressItem::finish(Reply::text("done"));
                    })
                },
            )
            .layer(ConcurrencyLimit::new(2)),
    );
    let results = futures::future::join_all((0..12).map(|_| app.message("go").send())).await;
    for result in results {
        result.assert_handled();
    }
    assert_eq!(maximum.load(Ordering::SeqCst), 2);
    assert_eq!(active.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn router_clone_preserves_routes_without_mutating_original() {
    let original = Router::new().route(command::<Ping>(), || async { Reply::text("pong") });
    let extended = original
        .clone()
        .route(text(), || async { Reply::text("text") });
    TestApp::new(original)
        .message("hi")
        .send()
        .await
        .assert_not_matched();
    TestApp::new(extended)
        .message("hi")
        .send()
        .await
        .assert_reply("text");
}
struct Ping;
impl CommandSpec for Ping {
    const NAME: &'static str = "ping";
    fn parse(input: &str) -> Result<Self, CommandError> {
        Arguments::new(input).finish()?;
        Ok(Self)
    }
}
