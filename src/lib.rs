use std::sync::Arc;

pub use gramhive_macros::*;
pub mod commands;
pub mod errors;
pub mod event;
pub mod extractors;
pub mod filters;
pub mod helpers;
pub mod router;
pub mod swarm;
pub mod tests;
pub mod tg_html;

pub use errors::ArgumentError;
pub use event::Event;
pub use event::EventListener;
pub use helpers::ClientBuilder;
pub use helpers::get_reply;
pub use swarm::Swarm;
pub use tg_html::TgHtml;
pub use tg_html::tg_html;

pub type GenericResult = Result<(), Box<dyn std::error::Error + Send + Sync + 'static>>;
pub type ArcBoxedError = Arc<Box<dyn std::error::Error + Send + Sync + 'static>>;
