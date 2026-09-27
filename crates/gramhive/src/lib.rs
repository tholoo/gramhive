//! GramHive: ordinary async functions, explicit routing, and offline tests.
#![forbid(unsafe_code)]
#[doc(hidden)]
pub use gramhive_core as __private;
pub use gramhive_core::*;
#[cfg(feature = "telegram")]
pub use gramhive_grammers::{self as telegram, BotAccount, Hive, UserAccount, grammers};
pub use gramhive_macros::{CallbackData, Command};
pub mod prelude {
    pub use crate::{
        Account, AnswerCallback, Button, Callback, CallbackData, Chat, Command, CommandSpec,
        ConcurrencyLimit, Data, Delete, Edit, Failure, File, IntoResponse, Message, NoResponse,
        Parallel, Progress, ProgressItem, Raw, Reply, Router, SendMessage, Sender, Sequence, State,
        Text, Tg, Trace, any, callback, command, text,
    };
    #[cfg(feature = "telegram")]
    pub use crate::{BotAccount, Hive, UserAccount};
}
