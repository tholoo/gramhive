use gramhive::{Dispatch, Event, Rejection, Response, prelude::*};
use gramhive_test::TestApp;
use std::time::{Duration, SystemTime};

#[derive(Command)]
#[command(name = "add")]
struct Add {
    a: i64,
    b: i64,
}

#[derive(CallbackData)]
#[callback(prefix = "old")]
enum Old {
    Go,
}

#[derive(CallbackData)]
#[callback(prefix = "new")]
enum New {
    Go,
}

#[tokio::test]
async fn default_validation_message_preserves_rejected_outcome() {
    let app = TestApp::new(
        Router::new().route(command::<Add>(), async |Command(add): Command<Add>| {
            Reply::text((add.a + add.b).to_string())
        }),
    );
    let result = app.message("/add 1").send().await;
    result.assert_rejected();
    result.assert_notice("Invalid input: missing argument `b`");
    assert_eq!(result.operations.len(), 1);
}

#[tokio::test]
async fn internal_details_are_not_sent_and_notification_failure_does_not_recurse() {
    let app = TestApp::new(Router::new().route(text(), || async {
        Failure("secret database credentials".into())
    }));
    for fail_at in [None, Some(0)] {
        let event = app.message("go");
        let result = match fail_at {
            None => event,
            Some(index) => event.failing_at(index),
        }
        .send()
        .await;
        assert_eq!(
            result.outcome,
            Err(Failure("secret database credentials".into()))
        );
        result.assert_notice("Something went wrong. Please try again.");
        assert_eq!(result.operations.len(), 1);
    }
}

struct Localized;

impl ResponsePolicy for Localized {
    fn rejected(&self, _: &Event, _: &Rejection) -> Response {
        Reply::text("Check your input").into_response()
    }
    fn failed(&self, _: &Event) -> Response {
        Reply::text("Please try later").into_response()
    }
    fn expired_callback(&self, _: &Event) -> Response {
        Response::None
    }
}

#[tokio::test]
async fn applications_can_replace_or_disable_the_policy() {
    let router = Router::new()
        .route(command::<Add>(), || async {})
        .response_policy(Localized);
    TestApp::new(router.clone())
        .message("/add")
        .send()
        .await
        .assert_notice("Check your input");
    let result = TestApp::new(router.response_policy(SilentPolicy))
        .message("/add")
        .send()
        .await;
    result.assert_rejected();
    assert!(result.operations.is_empty());
    TestApp::new(Router::new().response_policy(SilentPolicy))
        .callback(Old::Go)
        .send()
        .await
        .assert_not_matched();
}

#[tokio::test]
async fn callback_errors_answer_the_spinner_without_leaking_codec_details() {
    let app = TestApp::new(
        Router::new().route(callback::<Old>(), async |Callback(cb): Callback| {
            cb.answer()
        }),
    );
    let mut data = Old::Go.encode().unwrap();
    data.push(0);
    let result = app.callback_bytes(data).send().await;
    result.assert_rejected();
    result.assert_notice("This button is invalid. Please request a new one.");
    assert_eq!(result.operations.len(), 1);
}

#[tokio::test]
async fn old_callbacks_work_during_transition_and_expire_after_the_deadline() {
    for deadline in [
        SystemTime::now() + Duration::from_secs(86400),
        SystemTime::UNIX_EPOCH,
    ] {
        let app = TestApp::new(
            Router::new()
                .route(
                    callback::<Old>().until(deadline),
                    async |Callback(cb): Callback| cb.answer().text("Old handler"),
                )
                .route(callback::<New>(), async |Callback(cb): Callback| {
                    cb.answer().text("New handler")
                }),
        );
        let result = app.callback(Old::Go).send().await;
        if deadline == SystemTime::UNIX_EPOCH {
            result.assert_notice("This button has expired. Please request a new one.");
        } else {
            result.assert_notice("Old handler");
        }
        assert_eq!(result.outcome, Ok(Dispatch::Handled));
        app.callback(New::Go)
            .send()
            .await
            .assert_notice("New handler");
    }
}
