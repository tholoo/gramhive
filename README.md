# GramHive 2

```rust
use gramhive::prelude::*;

async fn echo(Text(text): Text) -> Reply {
    Reply::text(text)
}
```

**Axum for Telegram.** Ordinary async Rust functions, typed extractors, explicit
routing, deferred responses, and application tests that work without Telegram.
This is a breaking, working prototype built on published **grammers 0.10.0**.
The old implementation remains in Git at `v1-legacy` and `legacy`.

## Run the example

```sh
direnv allow                  # or: nix develop
export TG_ID=12345
export TG_HASH=your_api_hash
export BOT_TOKEN=your_bot_token
RUST_LOG=info just echo
```

Obtain the API ID/hash through [Telegram's API development tools](https://my.telegram.org)
and a bot token from BotFather. Credentials are read at runtime, never compiled in.
The example creates `echo.session` using grammers' SQLite session storage.
Session files contain authorization credentials; keep them private.
Ctrl+C stops intake, allows 30 seconds for handlers to finish, then cancels unfinished
work with bounded cleanup before saving update state and closing connections.

[The complete example](examples/echo/src/main.rs) demonstrates `/start`, `/echo`,
`/stats`, `/progress`, `/temporary`, `/manual`, `/raw`, and typed inline buttons.
It includes an offline integration test.

## Routes and extractors

```rust
fn app() -> Router {
    Router::new()
        .route(command::<Echo>(), echo_command)
        .route(callback::<EchoAction>(), action)
        .route(text(), echo)
        .layer(ConcurrencyLimit::new(16))
        .layer(Trace)
}
```

Routes are tried in declaration order. The first handler that extracts successfully
runs, and its response is executed. A matched command never also reaches the text
handler. Place specialized routes before broad routes. Unknown commands are text,
so the final text route can deliberately echo them.

| Outcome | Meaning |
| --- | --- |
| `Dispatch::NotMatched` | No route accepted the event |
| `Rejection::Missing` | Required data absent; try the next route |
| `Dispatch::Rejected(Rejection::Invalid(..))` | Invalid command/callback input; stop routing |
| `Dispatch::Handled` | Handler/response or expired-button fallback completed |
| `Dispatch::Cancelled` | Work was cancelled; temporary progress cleanup was attempted |
| `Err(Failure)` | Handler explicitly failed or response/transport execution failed |

The default `ResponsePolicy` sends helpful validation messages, a generic
“Something went wrong. Please try again.” for internal failures, and callback
answers for invalid or expired buttons. Internal error details are logged, never
included in the default failure message. `Rejection::Invalid` is explicitly public
validation text; do not put internal error details in it.

Use `.response_policy(MyPolicy)` to implement application-specific wording or
localization, or `.response_policy(SilentPolicy)` to disable automatic notices.
Presentation runs after middleware, preserving the original rejection/failure
outcome for callers and tests. Failed notices are logged once without retry loops.
Applications that return an error as a normal `Reply` retain full control.
A `command::<T>()` route validates arguments even if its handler does not extract
`Command<T>`. A typed extractor parses them again, keeping routing and extraction
independent and avoiding hidden dependency storage. Custom parsers should be pure.

Handlers accept zero to six extractors. Names of arguments have no meaning.

| Extractor | Value |
| --- | --- |
| `Text` | Message text or caption; absent on nontext updates |
| `Message` | Minimal message metadata (`id`, optional text) |
| `Sender`, `Chat` | IDs in Telegram Bot API dialog-ID format |
| `Account` | GramHive account name and optional Telegram username |
| `State<T>` | Clone of explicit application state |
| `Command<T>` | Parsed command arguments |
| `Callback`, `Data<T>` | Callback metadata and decoded typed payload |
| `Tg` | Testable procedural Telegram handle |
| `Raw<T>` | Transport-provided upstream value |

Implement `FromEvent<S>` for custom asynchronous extractors, and `Matcher::new`
for custom matching. No DI container, parameter-name inspection, or handler macros.

## Commands and `#[rest]`

```rust
#[derive(Command)]
#[command(name = "start", description = "Show the welcome message")]
struct Start;

#[derive(Command)]
#[command(name = "echo", description = "Echo text")]
struct Echo {
    #[rest]
    text: String,
}

async fn echo_command(Command(Echo { text }): Command<Echo>) -> Reply {
    Reply::text(text)
}

#[derive(Command)]
#[command(name = "add")]
struct Add { a: i64, b: i64 }

#[derive(Command)]
#[command(name = "ban")]
struct Ban {
    user: String,
    #[rest]
    reason: Option<String>,
}
```

`/add 10 20` uses `FromStr`. There is no shell quoting or escape syntax; missing,
invalid, and surplus arguments reject the event. `/echo@your_bot hello` is accepted
only for the current account's username (case-insensitive username comparison).
Command names are case-sensitive.

`#[rest]` must occur at most once, on the final field. `String` requires nonempty
remaining input; `Option<String>` permits none. Only surrounding whitespace is
trimmed: internal spaces and newlines are preserved. Generic declarations and tuple
forms are intentionally unsupported. The public `CommandSpec` trait can be
implemented by hand; the name leaves `Command<T>` available for extraction.
Metadata does not automatically publish bot command menus.

## Typed callback data

```rust
#[derive(Debug, Clone, PartialEq, Eq, CallbackData)]
#[callback(prefix = "echo")]
enum EchoAction {
    Uppercase { text: String },
    Lowercase { text: String },
}

let button = Button::callback(
    "LOUD",
    EchoAction::Uppercase { text: "hello".into() },
)?;
let reply = Reply::text("Choose").buttons(vec![vec![button]]);
```

One codec owns encoding and decoding. The public `CallbackData` trait is manually
implementable. Derives support unit and named variants with concrete
`Display + FromStr` fields. Prefixes distinguish routes; use unique prefixes.

The wire format is version byte `1`, then one-byte-length-prefixed UTF-8 strings:
prefix, variant name, and each field in declaration order. Reordering variants and
adding variants preserve existing payloads. Renaming variants or changing field
order/types breaks them; change the prefix when changing a schema. Invalid UTF-8,
truncation, unknown variants/versions, extra bytes, and payloads outside 1–64 bytes
produce errors. Unmatched callbacks receive “This button has expired. Please
request a new one.” after all routes have been tried. During a schema transition,
register the old handler alongside the new handler with an absolute deadline:

```rust
Router::new()
    .route(callback::<OldAction>().until(migration_deadline), old_action)
    .route(callback::<NewAction>(), new_action)
```

`migration_deadline` is a configured `std::time::SystemTime`; keep it fixed across
restarts. After the deadline the old matcher stops accepting input, and the expired
fallback answers old buttons. Remove the old route in a later deployment. This does
not migrate payloads automatically, and existing valid routes never expire implicitly.

Unicode is measured in **bytes**. Keep large data server-side and
encode a short identifier. `Button::callback` is fallible and checks even manually
implemented codecs before a button can be sent.

```rust
async fn action(
    Callback(cb): Callback,
    Data(action): Data<EchoAction>,
) -> impl IntoResponse {
    let text = match action {
        EchoAction::Uppercase { text } => text.to_uppercase(),
        EchoAction::Lowercase { text } => text.to_lowercase(),
    };
    cb.answer().edit(text)
}
```

`cb.answer()`, `.text("Saved!")`, `.reply("Result")`, and `.edit("Done")`
describe work without awaiting Telegram in the handler. Answer and follow-up are
independent and executed concurrently. They are separate RPCs, not a transaction.
Inline callbacks support answering and text editing; replying or deleting needs an
ordinary chat target. The raw API remains available for advanced inline operations.

## Responses and errors

`IntoResponse` supports `Reply`, `SendMessage`, `Edit`, `Delete`, `AnswerCallback`,
`File`, `NoResponse`, `()`, `Progress`, and `Result<T, E>` when both sides implement
`IntoResponse`. `Reply` replies to the triggering message; `SendMessage` sends
without a reply link. `File::new(path)` uploads a local document.

Application errors can implement `IntoResponse` to send useful messages.
Returning `Failure` preserves the failure for the caller and triggers the generic
notice from the response policy. No blanket conversion turns arbitrary errors into
user-visible technical details.

Arbitrary composition is explicit:

```rust
Sequence::new([
    Reply::text("First").into_response(),
    Reply::text("Second").into_response(),
])
```

`Sequence` stops at the first failure. `Parallel::new(...)` polls all responses
concurrently and waits for all, reporting the first error in declaration order.
Neither promises rollback, and tuples have no response semantics.

## Progress

```rust
use async_stream::stream;

async fn slow() -> Progress {
    Progress::editing(stream! {
        yield ProgressItem::update("Starting…");
        // await application work here
        yield ProgressItem::update("Processing…");
        yield ProgressItem::finish(Reply::text("Done"));
    })
}
```

The same task stream can use either strategy:

| Strategy | First update | Later updates | Finish |
| --- | --- | --- | --- |
| `Progress::editing` | Send status | Edit status | Edit status into result |
| `Progress::temporary` | Send status | Edit status | Delete status, then send result |

Finish without any updates sends the result directly. Finish is terminal and drops
the producer; a stream that ends without finishing is an error. Streams may yield
`Result<ProgressItem, Failure>` (for example with `async_stream::try_stream!`) to use
`?`. On task or delivery failure, temporary progress attempts to delete its status;
editing progress leaves the last delivered status visible. A temporary final result
may be a `File`.

Use `.throttle(Duration::from_secs(1))` on either strategy to send the first update
immediately and retain only the latest pending update between edits. A pending
update flushes when its interval ends, even if the producer is still working.
Finish bypasses the timer and replaces any pending update; it is never coalesced
away. Semantic progress assertions still see every yielded item. Throttling is
opt-in; zero disables it. Delivery retains backpressure while a Telegram call is
in flight. The echo example uses a one-second interval.

The runtime cancels through `Router::handle_until(event, cancel_future,
cleanup_timeout)`. This drops unfinished handlers, middleware, and response streams,
then attempts to delete known temporary progress messages within one shared cleanup
budget. Parallel temporary statuses are all tracked. Editing progress remains
visible. Directly dropping the dispatch future skips cleanup. A send cancelled
before its message ID is known cannot be cleaned up reliably; raw calls, imperative
`Tg` messages, and detached application tasks remain the application's responsibility.

## State, middleware, and escape hatches

```rust
#[derive(Clone, Default)]
struct AppState { /* services, counters, configuration */ }

let app = Router::with_state(AppState::default())
    .route(text(), async |State(_state): State<AppState>, Text(text): Text| {
        Reply::text(text)
    });
```

State is one explicit type; compose services with ordinary structs. The router
implements `tower_service::Service<Event>`. Its custom `Middleware`/`Next` boundary
keeps layer types erased. Last added layer is outermost. `Trace` creates a span per
event; `ConcurrencyLimit` covers both handlers and response execution, including
progress streams. Automatic error notices run after middleware and are outside this
layer's permit; the runtime's task bound still covers them. Limits are shared across
router clones. `poll_ready` is always ready; middleware admission happens in the returned future. The Telegram runtime
also bounds spawned handlers per account (64 by default).

```rust
async fn unusual(Tg(tg): Tg) -> Result<(), Failure> {
    let status = tg.send("Starting…").await?;
    status.edit("Stage 2…").await?;
    status.delete().await?;
    tg.send("Done").await?;
    Ok(())
}
```

`Raw<gramhive::grammers::Client>` exposes `invoke` and all other upstream APIs.
`Raw<gramhive::grammers::update::Update>`, raw messages and raw callbacks also work.
The adapter retains the original grammers objects rather than reproducing Telegram's
entity model. Other incoming update kinds remain available through `any()` and raw
extraction. Outgoing new messages are ignored to prevent echo loops.

## Offline tests

```rust
use gramhive_test::TestApp;

#[tokio::test]
async fn echoes_text() {
    let app = TestApp::new(Router::new().route(text(), echo));
    app.message("hello").from_user(42).send().await.assert_reply("hello");
}
```

Use `.callback(typed_data)`, `.callback_bytes(bytes)`, `.media()`, `.in_chat(id)`,
`.on_account(name, username)`, and `.failing_at(operation_index)` for fixtures.
`assert_callback_answered()`, `assert_edit(...)`, `assert_rejected()`, and
`assert_progress([ProgressEvent::Update(...), ProgressEvent::Finished(...)])` assert
application semantics. The fixture callback builder panics on invalid encoding;
`try_callback` exposes the error instead. `TestResult` also exposes dispatch results,
recorded operations, and semantic progress. `assert_rejected()` checks the outcome;
`assert_notice(text)` checks a reply or callback answer even when dispatch rejected
or failed. `.send_until(cancel_future, cleanup_timeout)` tests cancellation and
cleanup offline. `FakeDriver` can test the executor directly.

For application crates that need only offline behavior:

```toml
[dependencies]
gramhive = { path = "../gramhive/crates/gramhive", default-features = false }

[dev-dependencies]
gramhive-test = { path = "../gramhive/crates/gramhive-test" }
tokio = { version = "1", features = ["macros", "rt"] }
```

`just offline` compiles and tests the framework without grammers, MTProto, network,
credentials, or sessions. Derive diagnostics are checked with trybuild. Workspace
checks also compile the real adapter and runnable example.

## Multi-account runtime

```rust
Hive::new(api_id, api_hash)
    .bot("bot-a", BotAccount::token(token_a))
    .bot("bot-b", BotAccount::token(token_b).session("bot-b.session"))
    .user("personal", UserAccount::session("personal.session"))
    .max_in_flight(64)
    .serve(app())
    .await?;
```

Each registration has its own sender pool, update stream, and session. Account names
must be unique and contain ASCII letters, digits, `_`, or `-`. Explicit session paths
must be distinct. Existing bot sessions are checked against the token's account ID;
user sessions must already be authorized. An account connection/startup failure stops
the hive; individual handler errors are logged and intake continues.
`serve_until(app, shutdown_future)` supports embedding and custom shutdown signals.
The default shutdown policy gives handlers 30 seconds to finish, then signals
cancellation and allows up to 5 seconds for cleanup before aborting remaining tasks.
Saving update state and closing the sender pool each have the same 5-second timeout.
Configure these budgets explicitly when needed:

```rust
.shutdown_policy(ShutdownPolicy {
    grace_period: std::time::Duration::from_secs(30),
    cleanup_timeout: std::time::Duration::from_secs(5),
})
```

Cleanup is best effort; timeouts cannot preempt synchronous blocking code. Account
registration remains a startup operation by design. Runtime start/stop controls
will be added only when an application needs them. Interactive user login remains
outside this prototype.

## Workspace and development

| Crate | Responsibility |
| --- | --- |
| `gramhive-core` | Events, routing, extractors, state, responses, shared executor, middleware |
| `gramhive-grammers` | Upstream update translation, RPC driver, sessions and account lifecycle |
| `gramhive-macros` | Only `Command` and `CallbackData` derives |
| `gramhive-test` | Fake driver, fixtures and assertions; no grammers dependency |
| `gramhive` | Facade and prelude; optional `telegram` feature, enabled by default |

Routes, middleware and streams are erased at their boundaries. Core uses Tokio's
synchronization, timer, and select-macro features and has no grammers dependency. No unsafe code, dptree,
network mocks, or generalized dependency map are used.

```sh
just fmt
just lint                     # fmt check + clippy, warnings denied
just test
just offline
nix develop -c cargo check --workspace
```

The simple flake uses flake-utils and pins nixpkgs through `flake.lock`. It supplies
Rust, Cargo, rustfmt, clippy, rust-analyzer, pkg-config, and just. SQLite is built by
grammers' upstream session dependency; no external database service is needed.
`Cargo.lock` pins the published dependency graph. A direct `glass_pumpkin =
"=2.0.0-rc0"` constraint works around grammers-crypto 0.10's incompatibility with rc1's
changed BigUint and safe-prime return types.

See [architecture notes](docs/architecture.md) for tradeoffs and remaining design
questions. This prototype does not yet provide dialogues, persistent jobs, automatic
rate policies, or every authentication flow.
