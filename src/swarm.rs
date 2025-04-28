use std::sync::Arc;

use dptree::di::DependencyMap;
use grammers_client::{Client, InvocationError, types::User};
use tokio::sync;
use tracing::{error, info};

use crate::router::Router;

pub struct Swarm {
    objects: Vec<SwarmObject>,
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
    pub fn new() -> Self {
        Self {
            objects: Vec::new(),
        }
    }

    pub fn add(&mut self, object: SwarmObject) -> &mut Self {
        self.objects.push(object);
        self
    }

    pub async fn run(&self) -> anyhow::Result<()> {
        let (shutdown_tx, _) = sync::broadcast::channel::<()>(1);

        for object in &self.objects {
            let mut shutdown_rx = shutdown_tx.subscribe();
            let client = object.client.clone();

            let router = if let Some(username) = object.me.username() {
                let mut router: Router = match Arc::try_unwrap(object.router.clone()) {
                    Ok(r) => r,
                    Err(shared_arc) => (*shared_arc).clone(),
                };
                router.reinit_command_regexes(username);
                Arc::new(router)
            } else {
                object.router.clone()
            };

            let mut deps = object.deps.clone();
            let _ = deps.insert(client.clone());
            tokio::spawn(async move {
                loop {
                    tokio::select! {
                        biased;

                        _ = shutdown_rx.recv() => {
                            info!("Shutting down client task...");
                            break
                        },

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
                                Err(err) => error!("Client error: {}", err),
                            }
                        }
                    }
                }
            });
        }

        tokio::signal::ctrl_c().await?;
        shutdown_tx.send(())?;

        Ok(())
    }
}

impl Default for Swarm {
    fn default() -> Self {
        Self::new()
    }
}
