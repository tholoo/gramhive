use crate::*;
use std::{
    future::Future,
    sync::Arc,
    task::{Context, Poll},
    time::SystemTime,
};
use tracing::Instrument;

pub trait Handler<S, Args>: Send + Sync + 'static {
    fn call(&self, event: Event, state: Arc<S>) -> BoxFuture<'static, Result<Response, Rejection>>;
}
impl<S, F, Fut, R> Handler<S, ()> for F
where
    S: Send + Sync + 'static,
    F: Fn() -> Fut + Send + Sync + 'static,
    Fut: Future<Output = R> + Send + 'static,
    R: IntoResponse,
{
    fn call(&self, _: Event, _: Arc<S>) -> BoxFuture<'static, Result<Response, Rejection>> {
        let future = self();
        Box::pin(async move { Ok(future.await.into_response()) })
    }
}
macro_rules! handler {
    ($($arg:ident),+) => {
        impl<S, F, Fut, R, $($arg,)+> Handler<S, ($($arg,)+)> for F
        where S: Send + Sync + 'static, F: Fn($($arg),+) -> Fut + Clone + Send + Sync + 'static,
              Fut: Future<Output = R> + Send + 'static, R: IntoResponse,
              $($arg: FromEvent<S> + 'static,)+ {
            fn call(&self, event: Event, state: Arc<S>) -> BoxFuture<'static, Result<Response, Rejection>> {
                let f = self.clone();
                Box::pin(async move { Ok(f($($arg::from_event(&event, &state).await?),+).await.into_response()) })
            }
        }
    };
}
handler!(A);
handler!(A, B);
handler!(A, B, C);
handler!(A, B, C, D);
handler!(A, B, C, D, E);
handler!(A, B, C, D, E, F0);

type MatchFn = dyn Fn(&Event) -> Result<bool, Rejection> + Send + Sync;
pub struct Matcher(Arc<MatchFn>);
impl Matcher {
    pub fn new(f: impl Fn(&Event) -> Result<bool, Rejection> + Send + Sync + 'static) -> Self {
        Self(Arc::new(f))
    }

    /// Stop matching at this deployment-stable deadline. Unmatched callbacks use the
    /// router's expired-button policy. Register old and new schema routes side by side.
    pub fn until(self, deadline: SystemTime) -> Self {
        Self::new(move |event| {
            if SystemTime::now() >= deadline {
                return Ok(false);
            }
            (self.0)(event)
        })
    }
}
pub fn text() -> Matcher {
    Matcher::new(|e| Ok(e.text().is_some()))
}
pub fn any() -> Matcher {
    Matcher::new(|_| Ok(true))
}
pub fn command<T: CommandSpec>() -> Matcher {
    Matcher::new(|e| {
        let Some((name, args)) = e.command() else {
            return Ok(false);
        };
        if name != T::NAME {
            return Ok(false);
        }
        T::parse(args)
            .map(|_| true)
            .map_err(|e| Rejection::Invalid(e.to_string()))
    })
}
pub fn callback<T: CallbackData>() -> Matcher {
    Matcher::new(|e| {
        let EventKind::Callback(c) = &e.kind else {
            return Ok(false);
        };
        if !codec::matches(&c.data, T::PREFIX) {
            return Ok(false);
        }
        T::decode(&c.data)
            .map(|_| true)
            .map_err(|e| Rejection::Invalid(e.to_string()))
    })
}

type RouteFn<S> =
    dyn Fn(Event, Arc<S>) -> BoxFuture<'static, Result<Response, Rejection>> + Send + Sync;
struct Route<S> {
    matcher: Matcher,
    call: Arc<RouteFn<S>>,
}
/// First successful route wins. No implicit multicast or command-to-text fallthrough.
pub struct Router<S = ()> {
    routes: Arc<Vec<Route<S>>>,
    state: Arc<S>,
    layers: Vec<Arc<dyn Middleware>>,
    policy: Arc<dyn ResponsePolicy>,
}
impl<S> Clone for Router<S> {
    fn clone(&self) -> Self {
        Self {
            routes: self.routes.clone(),
            state: self.state.clone(),
            layers: self.layers.clone(),
            policy: self.policy.clone(),
        }
    }
}
impl Router<()> {
    pub fn new() -> Self {
        Self::with_state(())
    }
}
impl<S: Default + Send + Sync + 'static> Default for Router<S> {
    fn default() -> Self {
        Self::with_state(S::default())
    }
}
impl<S: Send + Sync + 'static> Router<S> {
    /// Construct with explicit state; no Default bound is required.
    pub fn with_state(state: S) -> Self {
        Self {
            routes: Arc::new(Vec::new()),
            state: Arc::new(state),
            layers: Vec::new(),
            policy: Arc::new(DefaultPolicy),
        }
    }
    pub fn route<H, A>(mut self, matcher: Matcher, handler: H) -> Self
    where
        H: Handler<S, A>,
    {
        // The builder can be cloned; keep its route list copy-on-write without cloning handlers.
        let routes = Arc::make_mut(&mut self.routes);
        routes.push(Route {
            matcher,
            call: Arc::new(move |event, state| handler.call(event, state)),
        });
        self
    }
    /// Last added layer is outermost. Middleware also surrounds response execution.
    pub fn layer(mut self, middleware: impl Middleware) -> Self {
        self.layers.push(Arc::new(middleware));
        self
    }
    /// Replace user-facing rejection, failure, and expired callback responses.
    pub fn response_policy(mut self, policy: impl ResponsePolicy) -> Self {
        self.policy = Arc::new(policy);
        self
    }

    pub fn handle(&self, event: Event) -> BoxFuture<'static, Result<Dispatch, Failure>> {
        let router = self.clone();
        let mut next = Next(Arc::new(move |event: Event| {
            let router = router.clone();
            Box::pin(async move {
                for route in router.routes.iter() {
                    match (route.matcher.0)(&event) {
                        Ok(false) => continue,
                        Err(e) => return Ok(Dispatch::Rejected(e)),
                        Ok(true) => {}
                    }
                    match (route.call)(event.clone(), router.state.clone()).await {
                        Ok(response) => {
                            execute(response, event.driver.clone()).await?;
                            return Ok(Dispatch::Handled);
                        }
                        Err(Rejection::Missing(_)) => continue,
                        Err(e) => return Ok(Dispatch::Rejected(e)),
                    }
                }
                if matches!(event.kind, EventKind::Callback(_)) {
                    let response = router.policy.expired_callback(&event);
                    if !matches!(response, Response::None) {
                        execute(response, event.driver.clone()).await?;
                        return Ok(Dispatch::Handled);
                    }
                }
                Ok(Dispatch::NotMatched)
            })
        }));
        for layer in &self.layers {
            let layer = layer.clone();
            next = Next(Arc::new(move |event| layer.handle(event, next.clone())));
        }
        let policy = self.policy.clone();
        Box::pin(async move {
            let outcome = next.run(event.clone()).await;
            let response = match &outcome {
                Ok(Dispatch::Rejected(error)) => {
                    tracing::warn!(%error, account = %event.account.name, "event rejected");
                    policy.rejected(&event, error)
                }
                Err(error) => {
                    tracing::error!(%error, account = %event.account.name, "handler or response failed");
                    policy.failed(&event)
                }
                _ => Response::None,
            };
            // One best-effort notification. A failed notification must not recursively
            // trigger another response or replace the original application outcome.
            if let Err(error) = execute(response, event.driver).await {
                tracing::warn!(%error, "could not deliver error notice");
            }
            outcome
        })
    }
}
impl<S> Clone for Route<S> {
    fn clone(&self) -> Self {
        Self {
            matcher: Matcher(self.matcher.0.clone()),
            call: self.call.clone(),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Dispatch {
    Cancelled,
    NotMatched,
    Rejected(Rejection),
    Handled,
}
#[derive(Clone)]
pub struct Next(Arc<dyn Fn(Event) -> BoxFuture<'static, Result<Dispatch, Failure>> + Send + Sync>);
impl Next {
    pub fn run(&self, event: Event) -> BoxFuture<'static, Result<Dispatch, Failure>> {
        (self.0)(event)
    }
}
pub trait Middleware: Send + Sync + 'static {
    fn handle(&self, event: Event, next: Next) -> BoxFuture<'static, Result<Dispatch, Failure>>;
}
pub struct Trace;
impl Middleware for Trace {
    fn handle(&self, event: Event, next: Next) -> BoxFuture<'static, Result<Dispatch, Failure>> {
        let span = tracing::info_span!("telegram_update", account = %event.account.name);
        Box::pin(
            async move {
                let result = next.run(event).await;
                tracing::debug!(?result, "dispatch completed");
                result
            }
            .instrument(span),
        )
    }
}
pub struct ConcurrencyLimit(Arc<tokio::sync::Semaphore>);
impl ConcurrencyLimit {
    /// Panics for zero; zero would block every event forever.
    pub fn new(max: usize) -> Self {
        assert!(max > 0, "concurrency must be positive");
        Self(Arc::new(tokio::sync::Semaphore::new(max)))
    }
}
impl Middleware for ConcurrencyLimit {
    fn handle(&self, event: Event, next: Next) -> BoxFuture<'static, Result<Dispatch, Failure>> {
        let semaphore = self.0.clone();
        Box::pin(async move {
            let _permit = semaphore
                .acquire_owned()
                .await
                .map_err(|e| Failure(e.to_string()))?;
            next.run(event).await
        })
    }
}
impl<S: Send + Sync + 'static> tower_service::Service<Event> for Router<S> {
    type Response = Dispatch;
    type Error = Failure;
    type Future = BoxFuture<'static, Result<Dispatch, Failure>>;
    fn poll_ready(&mut self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }
    fn call(&mut self, event: Event) -> Self::Future {
        self.handle(event)
    }
}
