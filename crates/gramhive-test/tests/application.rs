use gramhive::prelude::*;
use gramhive::{CallbackDataError, Dispatch, EventKind, Operation, ProgressEvent};
use gramhive_test::TestApp;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

#[derive(Command)]
#[command(name = "start", description = "Welcome")]
struct Start;
#[derive(Debug, PartialEq, Command)]
#[command(name = "echo")]
struct Echo {
    #[rest]
    text: String,
}
#[derive(Debug, PartialEq, Command)]
#[command(name = "add")]
struct Add {
    a: i64,
    b: i64,
}
#[derive(Debug, PartialEq, Command)]
#[command(name = "ban")]
struct Ban {
    user: String,
    #[rest]
    reason: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, CallbackData)]
#[callback(prefix = "echo")]
enum Action {
    Uppercase { text: String },
    Lowercase { text: String },
    Count { value: i64 },
    Reset,
}
async fn echo(Text(text): Text) -> Reply {
    Reply::text(text)
}
async fn echo_command(Command(Echo { text }): Command<Echo>) -> Reply {
    Reply::text(text)
}
async fn action(Callback(cb): Callback, Data(action): Data<Action>) -> impl IntoResponse {
    let text = match action {
        Action::Uppercase { text } => text.to_uppercase(),
        Action::Lowercase { text } => text.to_lowercase(),
        Action::Count { value } => value.to_string(),
        Action::Reset => "reset".into(),
    };
    cb.answer().edit(text)
}
fn app() -> Router {
    Router::new()
        .route(command::<Start>(), || async { Reply::text("welcome") })
        .route(command::<Echo>(), echo_command)
        .route(callback::<Action>(), action)
        .route(text(), echo)
}

#[tokio::test]
async fn ordinary_functions_route_and_do_not_fall_through() {
    let app = TestApp::new(app());
    app.message("hello")
        .from_user(42)
        .send()
        .await
        .assert_reply("hello");
    let start = app.message("/start").send().await;
    start.assert_reply("welcome");
    assert_eq!(start.operations.len(), 1);
    app.message("/echo hello from rust")
        .send()
        .await
        .assert_reply("hello from rust");
    app.message("/echo").send().await.assert_rejected();
    app.message("/start extra").send().await.assert_rejected();
    app.media().send().await.assert_not_matched();
}
#[tokio::test]
async fn command_mentions_are_account_specific() {
    let app = TestApp::new(Router::new().route(command::<Echo>(), echo_command));
    app.message("/echo@Test_Bot hi")
        .send()
        .await
        .assert_reply("hi");
    app.message("/echo@other hi")
        .send()
        .await
        .assert_not_matched();
    app.message("/echo@second hi")
        .on_account("second", Some("second"))
        .send()
        .await
        .assert_reply("hi");
}
#[test]
fn command_parser_preserves_rest_and_validates_positions() {
    assert_eq!(Add::parse("10 20").unwrap(), Add { a: 10, b: 20 });
    assert!(Add::parse("10").is_err());
    assert!(Add::parse("ten 20").is_err());
    assert!(Add::parse("10 20 30").is_err());
    assert_eq!(
        Ban::parse(" @bob  posting  spoilers\n repeatedly  ").unwrap(),
        Ban {
            user: "@bob".into(),
            reason: Some("posting  spoilers\n repeatedly".into())
        }
    );
    assert_eq!(Ban::parse("@bob").unwrap().reason, None);
    assert!(Echo::parse("  ").is_err());
    assert_eq!(Echo::parse(" hi   there  ").unwrap().text, "hi   there");
    assert_eq!(Start::DESCRIPTION, Some("Welcome"));
}
#[test]
fn callback_codec_is_deterministic_and_strict() {
    for action in [
        Action::Uppercase {
            text: "héllo | : \0".into(),
        },
        Action::Lowercase { text: "HI".into() },
        Action::Count { value: -123 },
        Action::Reset,
    ] {
        let data = action.encode().unwrap();
        assert_eq!(data, action.encode().unwrap());
        assert_eq!(Action::decode(&data).unwrap(), action);
        for end in 0..data.len() {
            assert!(Action::decode(&data[..end]).is_err());
        }
        let mut trailing = data;
        trailing.push(0);
        assert!(Action::decode(&trailing).is_err());
    }
    let mut bad = Action::Reset.encode().unwrap();
    bad[0] = 2;
    assert!(Action::decode(&bad).is_err());
    let mut bad = Action::Reset.encode().unwrap();
    bad[2] = 255;
    assert!(Action::decode(&bad).is_err());
    let oversized = Action::Uppercase {
        text: "a".repeat(64),
    }
    .encode();
    assert!(matches!(oversized, Err(CallbackDataError::Size(_))));
    // 1 version + 1 prefix length + 4 prefix + 1 variant length + 9 variant + 1 field length = 17.
    assert_eq!(
        Action::Uppercase {
            text: "a".repeat(47)
        }
        .encode()
        .unwrap()
        .len(),
        64
    );
    assert!(
        Action::Uppercase {
            text: "a".repeat(48)
        }
        .encode()
        .is_err()
    );
}
#[tokio::test]
async fn typed_callback_is_answered_and_edited() {
    let app = TestApp::new(app());
    let result = app
        .callback(Action::Uppercase {
            text: "hello".into(),
        })
        .send()
        .await;
    result.assert_callback_answered();
    result.assert_edit("HELLO");
    assert_eq!(result.operations.len(), 2);
    let mut bad = Action::Reset.encode().unwrap();
    bad[0] = 9;
    app.callback_bytes(bad).send().await.assert_rejected();
    app.callback_bytes(b"different".to_vec())
        .send()
        .await
        .assert_notice("This button has expired. Please request a new one.");
}
#[tokio::test]
async fn missing_extraction_skips_but_invalid_extraction_rejects() {
    let app = TestApp::new(
        Router::new()
            .route(any(), echo)
            .route(any(), || async { Reply::text("fallback") }),
    );
    app.media().send().await.assert_reply("fallback");
    let app = TestApp::new(
        Router::new()
            .route(text(), echo_command)
            .route(text(), echo),
    );
    app.message("/echo").send().await.assert_rejected();
}
#[tokio::test]
async fn state_and_account_are_explicit() {
    let count = Arc::new(AtomicUsize::new(0));
    let app = TestApp::new(Router::with_state(count.clone()).route(
        text(),
        async |State(count): State<Arc<AtomicUsize>>,
               Account(account): Account,
               Sender(sender): Sender,
               Chat(chat): Chat| {
            count.fetch_add(1, Ordering::Relaxed);
            Reply::text(format!("{} {sender} {chat}", account.name))
        },
    ));
    app.message("hi")
        .from_user(7)
        .in_chat(9)
        .on_account("second", None)
        .send()
        .await
        .assert_reply("second 7 9");
    assert_eq!(count.load(Ordering::Relaxed), 1);
}
#[tokio::test]
async fn imperative_handle_uses_the_same_driver() {
    let app = TestApp::new(Router::new().route(
        text(),
        async |Tg(tg): Tg| -> Result<(), Failure> {
            let status = tg.send("Starting").await?;
            status.edit("Stage 2").await?;
            status.delete().await?;
            tg.send("Done").await?;
            Ok(())
        },
    ));
    let result = app.message("go").send().await;
    result.assert_handled();
    assert!(matches!(
        &result.operations[..],
        [
            Operation::Send { reply: false, .. },
            Operation::Edit { .. },
            Operation::Delete { .. },
            Operation::Send { reply: false, .. }
        ]
    ));
}
#[tokio::test]
async fn application_errors_become_responses_and_failures_stay_failures() {
    struct AppError;
    impl IntoResponse for AppError {
        fn into_response(self) -> gramhive::Response {
            Reply::text("invalid input").into_response()
        }
    }
    let app = TestApp::new(Router::new().route(text(), || async { Err::<Reply, _>(AppError) }));
    app.message("hi").send().await.assert_reply("invalid input");
    let result = app.message("hi").failing_at(0).send().await;
    assert!(result.outcome.is_err());
}
fn progress_items() -> impl futures::Stream<Item = ProgressItem> {
    futures::stream::iter([
        ProgressItem::update("Starting"),
        ProgressItem::update("Processing"),
        ProgressItem::finish(Reply::text("Done")),
    ])
}
#[tokio::test]
async fn progress_is_asserted_semantically() {
    for temporary in [false, true] {
        let app = TestApp::new(Router::new().route(text(), move || async move {
            if temporary {
                Progress::temporary(progress_items())
            } else {
                Progress::editing(progress_items())
            }
        }));
        let result = app.message("go").send().await;
        result.assert_progress([
            ProgressEvent::Update("Starting".into()),
            ProgressEvent::Update("Processing".into()),
            ProgressEvent::Finished(Reply::text("Done")),
        ]);
    }
}
#[tokio::test]
async fn empty_router_and_other_event_are_unmatched() {
    TestApp::new(Router::<()>::new())
        .event(EventKind::Other)
        .send()
        .await
        .assert_not_matched();
    assert_eq!(Dispatch::NotMatched, Dispatch::NotMatched);
}

#[test]
fn derive_internal_names_do_not_collide_with_fields() {
    #[derive(Command)]
    #[command(name = "hygiene")]
    struct Hygiene {
        args: String,
        input: String,
        #[rest]
        result: String,
    }
    let parsed = Hygiene::parse("one two three").unwrap();
    assert_eq!(
        (parsed.args, parsed.input, parsed.result),
        ("one".into(), "two".into(), "three".into())
    );
    #[derive(Debug, PartialEq, CallbackData)]
    #[callback(prefix = "h")]
    enum Collision {
        Data {
            input: String,
            out: String,
            result: i64,
        },
    }
    let original = Collision::Data {
        input: "a".into(),
        out: "b".into(),
        result: 7,
    };
    assert_eq!(
        Collision::decode(&original.encode().unwrap()).unwrap(),
        original
    );
}

#[tokio::test]
async fn raw_extraction_is_an_explicit_transport_escape_hatch() {
    #[derive(Clone)]
    struct RawClient(&'static str);
    impl gramhive::RawSource for RawClient {
        fn get(&self, id: std::any::TypeId) -> Option<&(dyn std::any::Any + Send + Sync)> {
            (id == std::any::TypeId::of::<Self>()).then_some(self)
        }
    }
    let app = TestApp::new(
        Router::new().route(text(), async |Raw(client): Raw<RawClient>| {
            Reply::text(client.0)
        }),
    );
    app.message("go").send().await.assert_not_matched();
    app.message("go")
        .raw(Arc::new(RawClient("raw")))
        .send()
        .await
        .assert_reply("raw");
}

#[test]
fn callback_button_checks_hand_written_codecs_too() {
    struct Oversized;
    impl CallbackData for Oversized {
        const PREFIX: &'static str = "bad";
        fn encode(&self) -> Result<Vec<u8>, CallbackDataError> {
            Ok(vec![0; 65])
        }
        fn decode(_: &[u8]) -> Result<Self, CallbackDataError> {
            Ok(Self)
        }
    }
    assert!(matches!(
        Button::callback("bad", Oversized),
        Err(CallbackDataError::Size(65))
    ));
}

#[test]
fn derives_work_with_an_application_result_alias() {
    type Result<T> = std::result::Result<T, ()>;
    #[derive(Command)]
    #[command(name = "alias")]
    struct Alias;
    #[derive(CallbackData)]
    #[callback(prefix = "alias")]
    enum CallbackAlias {
        Unit,
    }
    let _: Result<()> = Ok(());
    assert!(Alias::parse("").is_ok());
    assert!(CallbackAlias::Unit.encode().is_ok());
}
