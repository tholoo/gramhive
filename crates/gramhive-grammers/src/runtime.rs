use crate::translate;
use gramhive_core::{AccountInfo, Dispatch, Failure, Router};
use grammers_client::{Client, SenderPool, client::UpdatesConfiguration};
use grammers_session::storages::SqliteSession;
use std::{collections::HashSet, future::Future, path::PathBuf, sync::Arc};
use tokio::{sync::watch, task::JoinSet};

pub struct BotAccount {
    token: String,
    session: Option<PathBuf>,
}
impl BotAccount {
    pub fn token(token: impl Into<String>) -> Self {
        Self {
            token: token.into(),
            session: None,
        }
    }
    pub fn session(mut self, path: impl Into<PathBuf>) -> Self {
        self.session = Some(path.into());
        self
    }
}
/// An already-authorized grammers SQLite session. Interactive login stays in grammers.
pub struct UserAccount {
    session: PathBuf,
}
impl UserAccount {
    pub fn session(path: impl Into<PathBuf>) -> Self {
        Self {
            session: path.into(),
        }
    }
}
enum Credentials {
    Bot(BotAccount),
    User(UserAccount),
}
struct Registration {
    name: String,
    credentials: Credentials,
}
pub struct Hive {
    api_id: i32,
    api_hash: String,
    accounts: Vec<Registration>,
    max_in_flight: usize,
}
impl Hive {
    pub fn new(api_id: i32, api_hash: impl Into<String>) -> Self {
        Self {
            api_id,
            api_hash: api_hash.into(),
            accounts: Vec::new(),
            max_in_flight: 64,
        }
    }
    pub fn bot(mut self, name: impl Into<String>, account: BotAccount) -> Self {
        self.accounts.push(Registration {
            name: name.into(),
            credentials: Credentials::Bot(account),
        });
        self
    }
    pub fn user(mut self, name: impl Into<String>, account: UserAccount) -> Self {
        self.accounts.push(Registration {
            name: name.into(),
            credentials: Credentials::User(account),
        });
        self
    }
    /// Bounds spawned handlers per account, including time spent in response streams.
    pub fn max_in_flight(mut self, max: usize) -> Self {
        self.max_in_flight = max;
        self
    }
    pub async fn serve<S: Send + Sync + 'static>(self, app: Router<S>) -> Result<(), Failure> {
        self.serve_until(app, async {
            if let Err(error) = tokio::signal::ctrl_c().await {
                tracing::error!(%error, "signal listener failed");
            }
        })
        .await
    }
    /// Stops intake, drains handlers, synchronizes update state, then closes each sender pool.
    pub async fn serve_until<S, F>(self, app: Router<S>, shutdown: F) -> Result<(), Failure>
    where
        S: Send + Sync + 'static,
        F: Future<Output = ()>,
    {
        self.validate()?;
        let (stop, rx) = watch::channel(false);
        let mut tasks = JoinSet::new();
        for registration in self.accounts {
            tasks.spawn(run_account(
                registration,
                self.api_id,
                self.api_hash.clone(),
                self.max_in_flight,
                app.clone(),
                rx.clone(),
            ));
        }
        tokio::pin!(shutdown);
        let first = tokio::select! {
            _ = &mut shutdown => Ok(()),
            result = tasks.join_next() => flatten(result),
        };
        let _ = stop.send(true);
        let mut result = first;
        while let Some(account) = tasks.join_next().await {
            if let Err(error) = flatten(Some(account))
                && result.is_ok()
            {
                result = Err(error);
            }
        }
        result
    }
    fn validate(&self) -> Result<(), Failure> {
        if self.accounts.is_empty() {
            return Err(Failure("register at least one account".into()));
        }
        if self.max_in_flight == 0 {
            return Err(Failure("max_in_flight must be positive".into()));
        }
        if self.api_id <= 0 || self.api_hash.is_empty() {
            return Err(Failure("Telegram API ID and hash are required".into()));
        }
        let mut names = HashSet::new();
        let mut paths = HashSet::new();
        for account in &self.accounts {
            if account.name.is_empty()
                || !account
                    .name
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
            {
                return Err(Failure(
                    "account names must contain ASCII letters, digits, '-' or '_'".into(),
                ));
            }
            if !names.insert(&account.name) {
                return Err(Failure(format!("duplicate account name: {}", account.name)));
            }
            if !paths.insert(account.path()) {
                return Err(Failure("accounts must use separate session files".into()));
            }
        }
        Ok(())
    }
}
impl Registration {
    fn path(&self) -> PathBuf {
        match &self.credentials {
            Credentials::Bot(bot) => bot
                .session
                .clone()
                .unwrap_or_else(|| format!("{}.session", self.name).into()),
            Credentials::User(user) => user.session.clone(),
        }
    }
}
fn flatten(
    result: Option<Result<Result<(), Failure>, tokio::task::JoinError>>,
) -> Result<(), Failure> {
    result
        .ok_or_else(|| Failure("account task disappeared".into()))?
        .map_err(|e| Failure(e.to_string()))?
}
struct PoolGuard {
    handle: grammers_client::sender::SenderPoolFatHandle,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for PoolGuard {
    fn drop(&mut self) {
        self.handle.quit();
        self.task.abort();
    }
}
async fn stopped(rx: &mut watch::Receiver<bool>) {
    if *rx.borrow() {
        return;
    }
    let _ = rx.changed().await;
}
async fn run_account<S: Send + Sync + 'static>(
    registration: Registration,
    api_id: i32,
    api_hash: String,
    max: usize,
    app: Router<S>,
    mut stop: watch::Receiver<bool>,
) -> Result<(), Failure> {
    let session = tokio::select! {
        _ = stopped(&mut stop) => return Ok(()),
        session = SqliteSession::open(registration.path()) => Arc::new(session.map_err(|e| Failure(e.to_string()))?),
    };
    let SenderPool {
        runner,
        updates,
        handle,
    } = SenderPool::new(session, api_id);
    let client = Client::new(handle.clone());
    let mut pool = PoolGuard {
        handle,
        task: tokio::spawn(runner.run()),
    };
    let initialize = async {
        if !client
            .is_authorized()
            .await
            .map_err(|e| Failure(e.to_string()))?
        {
            match &registration.credentials {
                Credentials::Bot(bot) => {
                    client
                        .bot_sign_in(&bot.token, &api_hash)
                        .await
                        .map_err(|e| Failure(e.to_string()))?;
                }
                Credentials::User(_) => {
                    return Err(Failure(
                        "user session is not authorized; sign in with grammers first".into(),
                    ));
                }
            }
        }
        let me = client.get_me().await.map_err(|e| Failure(e.to_string()))?;
        match &registration.credentials {
            Credentials::Bot(bot) => {
                let token_id = bot
                    .token
                    .split_once(':')
                    .and_then(|(id, _)| id.parse::<i64>().ok());
                if !me.is_bot() || token_id != me.id().bare_id() {
                    return Err(Failure(
                        "session belongs to a different account than the bot token".into(),
                    ));
                }
            }
            Credentials::User(_) if me.is_bot() => {
                return Err(Failure("user session belongs to a bot".into()));
            }
            _ => {}
        }
        let account = AccountInfo {
            name: registration.name,
            username: me.username().map(str::to_owned),
        };
        let updates = client
            .stream_updates(
                updates,
                UpdatesConfiguration {
                    catch_up: true,
                    ..Default::default()
                },
            )
            .await
            .map_err(|e| Failure(e.to_string()))?;
        Ok((account, updates))
    };
    let (account, mut updates) = tokio::select! {
        _ = stopped(&mut stop) => return Ok(()),
        result = initialize => result?,
    };
    let mut handlers = JoinSet::new();
    let outcome = loop {
        tokio::select! {
            _ = stopped(&mut stop) => break Ok(()),
            result = handlers.join_next(), if !handlers.is_empty() => report_handler(result),
            update = updates.next(), if handlers.len() < max => {
                match update {
                    Err(error) => break Err(Failure(error.to_string())),
                    Ok(update) => if let Some(event) = translate(account.clone(), client.clone(), update) {
                        let app = app.clone();
                        handlers.spawn(async move { app.handle(event).await });
                    }
                }
            }
        }
    };
    while let Some(result) = handlers.join_next().await {
        report_handler(Some(result));
    }
    let sync = updates
        .sync_update_state()
        .await
        .map_err(|e| Failure(e.to_string()));
    pool.handle.quit();
    let _ = (&mut pool.task).await;
    outcome.and(sync)
}
fn report_handler(result: Option<Result<Result<Dispatch, Failure>, tokio::task::JoinError>>) {
    match result {
        Some(Ok(Err(error))) => tracing::error!(%error, "handler or response failed"),
        Some(Err(error)) => tracing::error!(%error, "handler task panicked"),
        Some(Ok(Ok(Dispatch::Rejected(error)))) => tracing::warn!(%error, "event rejected"),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_invalid_registration_before_connecting() {
        assert!(Hive::new(1, "hash").validate().is_err());
        assert!(
            Hive::new(1, "hash")
                .bot("../bad", BotAccount::token("1:x"))
                .validate()
                .is_err()
        );
        assert!(
            Hive::new(1, "hash")
                .bot("a", BotAccount::token("1:x"))
                .bot("a", BotAccount::token("2:x"))
                .validate()
                .is_err()
        );
        assert!(
            Hive::new(1, "hash")
                .bot("a", BotAccount::token("1:x").session("same"))
                .user("b", UserAccount::session("same"))
                .validate()
                .is_err()
        );
        assert!(
            Hive::new(1, "hash")
                .bot("a", BotAccount::token("1:x"))
                .user("b", UserAccount::session("b.session"))
                .validate()
                .is_ok()
        );
    }
}
