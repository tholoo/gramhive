use std::ops::ControlFlow;

use dptree::{Handler, di::DependencyMap};

use crate::{Event, EventListener, GenericResult, commands::CommandMeta};

#[derive(Clone)]
pub struct Router {
    handlers: Vec<Handler<'static, DependencyMap, GenericResult>>,
    pub commands: Vec<CommandMeta>,
    pub error_handler: Handler<'static, DependencyMap, ()>,
    // event_listeners: Vec<Box<dyn EventListener>>,
}

impl Router {
    pub fn new() -> Self {
        Self {
            handlers: Vec::new(),
            commands: Vec::new(),
            error_handler: dptree::entry(),
            // event_listeners: Vec::new(),
        }
    }

    pub fn set_error_handler(&mut self, handler: Handler<'static, DependencyMap, ()>) -> &mut Self {
        self.error_handler = handler;
        self
    }

    pub fn add(&mut self, handler: Handler<'static, DependencyMap, GenericResult>) -> &mut Self {
        self.handlers.push(handler);
        self
    }

    pub fn add_command(&mut self, command: CommandMeta) -> &mut Self {
        self.commands.push(command);
        self
    }

    pub fn add_event_listener<C: EventListener>(&mut self, listener: C) -> &mut Self {
        // self.event_listeners.push(Box::new(listener));
        self
    }

    pub async fn emit(&self, event: Event<'_>) {
        // for listener in &self.event_listeners {
        // listener.handle(&event).await;
        // }
    }

    pub(crate) fn reinit_command_regexes(&mut self, bot_username: &str) -> &mut Self {
        for command in &mut self.commands {
            command.reinit_regex(bot_username);
        }
        self
    }

    pub async fn dispatch(&self, deps: DependencyMap) {
        let mut tasks = Vec::new();

        let mut deps = deps;
        self.emit(Event::BeginDispatch { deps: &mut deps }).await;

        for handler in self.handlers.clone() {
            let mut deps_clone = deps.clone();
            let error_handler = self.error_handler.clone();
            let task = tokio::spawn(async move {
                let dispatched = handler.dispatch(deps_clone.clone()).await;
                if let ControlFlow::Break(Err(err)) = dispatched {
                    deps_clone.insert(err);
                    error_handler.dispatch(deps_clone).await;
                }
            });

            tasks.push(task);
        }

        for task in tasks {
            let _ = task.await;
        }

        self.emit(Event::EndDispatch { deps }).await;
    }
}

impl Default for Router {
    fn default() -> Self {
        Self::new()
    }
}
