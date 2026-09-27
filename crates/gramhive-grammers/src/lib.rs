//! Telegram adapter using the published Codeberg-era grammers 0.10 APIs.
#![forbid(unsafe_code)]
mod driver;
mod runtime;
pub use driver::{TelegramDriver, translate};
pub use grammers_client as grammers;
pub use runtime::{BotAccount, Hive, ShutdownPolicy, UserAccount};
