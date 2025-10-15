use dptree::di::DependencyMap;
use grammers_client::{
    Client, InvocationError,
    types::{User, update::Message},
};
use std::{collections::HashMap, sync::Arc};
use tokio::{
    sync::{Mutex, broadcast, oneshot},
    task::JoinHandle,
};
use tracing::{error, info};

use crate::router::Router;

pub struct Swarm {
    shutdown_tx: broadcast::Sender<()>,    // global shutdown
    tasks: Mutex<HashMap<i64, TaskEntry>>, // keyed by me.id()
    waiters: Mutex<HashMap<(i64, i64), oneshot::Sender<Message>>>,
}

struct TaskEntry {
    handle: JoinHandle<()>,
    shutdown_tx: oneshot::Sender<()>, // per-task shutdown
}

pub struct SwarmObject {
    pub client: Arc<Client>,
    pub router: Arc<Router>,
    pub deps: DependencyMap,
    me: User,
}

impl SwarmObject {
    pub async fn new(
        client: Arc<Client>,
        me: Option<User>,
        router: Arc<Router>,
        deps: DependencyMap,
    ) -> Result<Self, InvocationError> {
        let me = match me {
            Some(user) => user,
            None => client.get_me().await?,
        };
        Ok(Self {
            client,
            router,
            deps,
            me,
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum WaitError {
    #[error("A waiter already exists for client {0} chat {1}")]
    AlreadyWaiting(i64, i64),
    #[error("Waiter was cancelled before a message arrived")]
    Cancelled,
}

impl Swarm {
    pub fn new() -> Arc<Self> {
        let (shutdown_tx, _) = broadcast::channel::<()>(1);
        Arc::new(Self {
            shutdown_tx,
            tasks: Mutex::new(HashMap::new()),
            waiters: Mutex::new(HashMap::new()),
        })
    }

    /// Wait for the next incoming message in `chat_id` for `client_id`.
    /// Fails if there is already a waiter registered for that (client, chat).
    pub async fn wait_for_reply(&self, client_id: i64, chat_id: i64) -> Result<Message, WaitError> {
        let (tx, rx) = oneshot::channel::<Message>();

        // Ensure only one waiter per (client, chat)
        let mut waiters = self.waiters.lock().await;
        if waiters.contains_key(&(client_id, chat_id)) {
            return Err(WaitError::AlreadyWaiting(client_id, chat_id));
        }
        waiters.insert((client_id, chat_id), tx);
        drop(waiters);

        rx.await.map_err(|_| WaitError::Cancelled)
    }

    pub async fn add(self: &Arc<Self>, object: SwarmObject) {
        let id = object.me.id();

        if self.tasks.lock().await.contains_key(&id) {
            self.shutdown_one(id).await;
        }

        let mut shutdown_rx = self.shutdown_tx.subscribe();
        let client = object.client.clone();

        let router = if let Some(username) = object.me.username() {
            let mut r = (*object.router).clone();
            r.reinit_command_regexes(username);
            Arc::new(r)
        } else {
            object.router.clone()
        };

        let mut deps = object.deps.clone();
        let _ = deps.insert(client.clone());

        // per-task oneshot
        let (task_shutdown_tx, mut task_shutdown_rx) = oneshot::channel::<()>();

        let handle = tokio::spawn(async move {
            loop {
                tokio::select! {
                    // global shutdown (broadcast)
                    _ = shutdown_rx.recv() => {
                        info!("Shutting down client task (global)…");
                        break;
                    }
                    // targeted shutdown (oneshot)
                    _ = &mut task_shutdown_rx => {
                        info!("Shutting down client task (targeted)…");
                        break;
                    }
                    result = client.next_update() => {
                        match result {
                            Ok(update) => {
                                let _ = deps.insert(update);
                                let deps = deps.clone();
                                let router = router.clone();
                                tokio::spawn(async move {
                                    router.dispatch(deps).await;
                                });
                            }
                            Err(err) => error!("Client error: {err}"),
                        }
                    }
                }
            }
        });

        self.tasks.lock().await.insert(
            id,
            TaskEntry {
                handle,
                shutdown_tx: task_shutdown_tx,
            },
        );
    }

    /// Shutdown a specific task by its me.id()
    pub async fn shutdown_one(&self, id: i64) {
        if let Some(entry) = self.tasks.lock().await.remove(&id) {
            // sending may fail if task already finished; that's okay
            let _ = entry.shutdown_tx.send(());
            if let Err(e) = entry.handle.await {
                error!("Task {id} join error: {e}");
            }
        } else {
            info!("No task with id {id} to shutdown");
        }
    }

    pub async fn active_ids(&self) -> Vec<i64> {
        self.tasks.lock().await.keys().copied().collect()
    }

    /// Shutdown all tasks
    pub async fn shutdown(&self) {
        let _ = self.shutdown_tx.send(());
        let mut tasks = self.tasks.lock().await;
        for (id, entry) in tasks.drain() {
            // best-effort: also close per-task channel
            let _ = entry.shutdown_tx.send(());
            if let Err(e) = entry.handle.await {
                error!("Task {id} join error: {e}");
            }
        }
    }
}
