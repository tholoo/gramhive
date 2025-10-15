use std::sync::Arc;

use dptree::di::DependencyMap;
use grammers_client::{Client, InvocationError, types::User};
use tokio::{
    sync::{Mutex, broadcast},
    task::JoinHandle,
};
use tracing::{error, info};

use crate::router::Router;

pub struct Swarm {
    shutdown_tx: broadcast::Sender<()>,
    tasks: Mutex<Vec<JoinHandle<()>>>,
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
        router: Arc<Router>,
        deps: DependencyMap,
    ) -> Result<Self, InvocationError> {
        let me = client.get_me().await?;
        Ok(Self {
            client,
            router,
            deps,
            me,
        })
    }
}

impl Swarm {
    pub fn new() -> Arc<Self> {
        let (shutdown_tx, _) = broadcast::channel::<()>(1);
        Arc::new(Self {
            shutdown_tx,
            tasks: Mutex::new(Vec::new()),
        })
    }

    pub async fn add(self: &Arc<Self>, object: SwarmObject) {
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

        let handle = tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = shutdown_rx.recv() => {
                        info!("Shutting down client task…");
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

        self.tasks.lock().await.push(handle);
    }

    pub async fn shutdown(&self) {
        let _ = self.shutdown_tx.send(());
        let mut tasks = self.tasks.lock().await;
        for h in tasks.drain(..) {
            if let Err(e) = h.await {
                error!("Task join error: {e}");
            }
        }
    }
}
