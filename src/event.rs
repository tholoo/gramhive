use std::ops::ControlFlow;

use async_trait::async_trait;
use dptree::di::DependencyMap;

pub enum Event<'a> {
    BeginDispatch { deps: &'a mut DependencyMap },
    EndDispatch { deps: DependencyMap },
}

#[async_trait]
pub trait EventListener: Send + Sync + 'static {
    async fn handle(&self, event: &Event) -> ControlFlow<()>;
}
