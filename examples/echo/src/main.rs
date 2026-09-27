use async_stream::stream;
use gramhive::{grammers, prelude::*};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

#[derive(Clone, Default)]
struct AppState {
    echoes: Arc<AtomicUsize>,
}

#[derive(Command)]
#[command(name = "start", description = "Show the welcome message")]
struct Start;

#[derive(Command)]
#[command(name = "echo", description = "Echo text")]
struct Echo {
    #[rest]
    text: String,
}

#[derive(Command)]
#[command(name = "stats")]
struct Stats;

#[derive(Command)]
#[command(name = "progress")]
struct Editing;

#[derive(Command)]
#[command(name = "temporary")]
struct Temporary;

#[derive(Command)]
#[command(name = "manual")]
struct Manual;

#[derive(Command)]
#[command(name = "raw")]
struct Advanced;

#[derive(Debug, Clone, PartialEq, Eq, CallbackData)]
#[callback(prefix = "echo")]
enum EchoAction {
    Uppercase { text: String },
    Lowercase { text: String },
}

async fn echo(Text(text): Text) -> Reply {
    Reply::text(text)
}

async fn echo_command(
    Command(Echo { text }): Command<Echo>,
    State(state): State<AppState>,
) -> Reply {
    state.echoes.fetch_add(1, Ordering::Relaxed);
    Reply::text(text)
}

async fn start(Account(account): Account) -> Result<Reply, Failure> {
    let buttons = vec![vec![
        Button::callback(
            "LOUD",
            EchoAction::Uppercase {
                text: "Hello".into(),
            },
        )
        .map_err(|e| Failure(e.to_string()))?,
        Button::callback(
            "quiet",
            EchoAction::Lowercase {
                text: "Hello".into(),
            },
        )
        .map_err(|e| Failure(e.to_string()))?,
    ]];
    Ok(Reply::text(format!(
        "Hello from {}! Try /echo, /stats, /progress, /temporary, /manual or /raw.",
        account.name
    ))
    .buttons(buttons))
}

async fn action(Callback(cb): Callback, Data(action): Data<EchoAction>) -> impl IntoResponse {
    let text = match action {
        EchoAction::Uppercase { text } => text.to_uppercase(),
        EchoAction::Lowercase { text } => text.to_lowercase(),
    };
    cb.answer().edit(text)
}

async fn stats(State(state): State<AppState>) -> Reply {
    Reply::text(format!(
        "{} /echo requests",
        state.echoes.load(Ordering::Relaxed)
    ))
}

fn stages() -> impl gramhive::Stream<Item = ProgressItem> {
    stream! {
        yield ProgressItem::update("Starting…");
        tokio::time::sleep(std::time::Duration::from_millis(400)).await;
        yield ProgressItem::update("Processing…");
        tokio::time::sleep(std::time::Duration::from_millis(400)).await;
        yield ProgressItem::finish(Reply::text("Done"));
    }
}

async fn editing() -> Progress {
    Progress::editing(stages())
}

async fn temporary() -> Progress {
    Progress::temporary(stages())
}

async fn manual(Tg(tg): Tg) -> Result<(), Failure> {
    let status = tg.send("Starting…").await?;
    status.edit("Finishing…").await?;
    status.delete().await?;
    tg.send("Done").await?;
    Ok(())
}

async fn advanced(Raw(client): Raw<grammers::Client>) -> Result<Reply, Failure> {
    let me = client.get_me().await.map_err(|e| Failure(e.to_string()))?;
    Ok(Reply::text(format!(
        "Telegram account ID: {}",
        me.id().bare_id_unchecked()
    )))
}

fn app() -> Router<AppState> {
    Router::with_state(AppState::default())
        .route(command::<Start>(), start)
        .route(command::<Echo>(), echo_command)
        .route(command::<Stats>(), stats)
        .route(command::<Editing>(), editing)
        .route(command::<Temporary>(), temporary)
        .route(command::<Manual>(), manual)
        .route(command::<Advanced>(), advanced)
        .route(callback::<EchoAction>(), action)
        .route(text(), echo)
        .layer(ConcurrencyLimit::new(16))
        .layer(Trace)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    let api_id = std::env::var("TG_ID")?.parse()?;
    let api_hash = std::env::var("TG_HASH")?;
    let token = std::env::var("BOT_TOKEN")?;
    Hive::new(api_id, api_hash)
        .bot("echo", BotAccount::token(token))
        .serve(app())
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use gramhive_test::TestApp;

    #[tokio::test]
    async fn example_works_without_telegram() {
        let app = TestApp::new(app());

        app.message("hello").send().await.assert_reply("hello");
        app.message("/echo hello from rust")
            .send()
            .await
            .assert_reply("hello from rust");
        app.message("/stats")
            .send()
            .await
            .assert_reply("1 /echo requests");

        let result = app
            .callback(EchoAction::Uppercase {
                text: "hello".into(),
            })
            .send()
            .await;
        result.assert_callback_answered();
        result.assert_edit("HELLO");

        app.message("/progress").send().await.assert_edit("Done");
        app.message("/temporary").send().await.assert_reply("Done");
        app.message("/manual").send().await.assert_handled();
    }
}
