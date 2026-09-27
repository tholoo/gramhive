use crate::{Event, EventKind, IntoResponse, Rejection, Reply, Response};

/// User-facing responses are configurable independently of dispatch outcomes.
/// `Rejection::Invalid` is public validation text; internal Failure details are never passed here.
pub trait ResponsePolicy: Send + Sync + 'static {
    fn rejected(&self, event: &Event, rejection: &Rejection) -> Response;
    fn failed(&self, event: &Event) -> Response;
    fn expired_callback(&self, event: &Event) -> Response;
}

/// Helpful validation messages, generic internal errors, and an expired-button fallback.
pub struct DefaultPolicy;

impl ResponsePolicy for DefaultPolicy {
    fn rejected(&self, event: &Event, rejection: &Rejection) -> Response {
        let text = match (&event.kind, rejection) {
            (EventKind::Callback(_), _) => {
                "This button is invalid. Please request a new one.".into()
            }
            (_, Rejection::Invalid(reason)) => format!("Invalid input: {reason}"),
            (_, Rejection::Missing(_)) => "This request is missing required information.".into(),
        };
        notice(event, text)
    }

    fn failed(&self, event: &Event) -> Response {
        notice(event, "Something went wrong. Please try again.".into())
    }

    fn expired_callback(&self, event: &Event) -> Response {
        notice(
            event,
            "This button has expired. Please request a new one.".into(),
        )
    }
}

/// For applications that implement their own presentation in middleware.
pub struct SilentPolicy;

impl ResponsePolicy for SilentPolicy {
    fn rejected(&self, _: &Event, _: &Rejection) -> Response {
        Response::None
    }
    fn failed(&self, _: &Event) -> Response {
        Response::None
    }
    fn expired_callback(&self, _: &Event) -> Response {
        Response::None
    }
}

fn notice(event: &Event, text: String) -> Response {
    match &event.kind {
        EventKind::Callback(callback) => callback
            .answer()
            .text(text.chars().take(200).collect::<String>())
            .into_response(),
        EventKind::Message(_) => {
            Reply::text(text.chars().take(1000).collect::<String>()).into_response()
        }
        EventKind::Other => Response::None,
    }
}
