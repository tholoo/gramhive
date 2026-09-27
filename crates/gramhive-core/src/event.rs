use crate::{AnswerCallback, BoxFuture, CallbackData, CommandSpec, Driver, Telegram};
use std::{
    any::{Any, TypeId},
    sync::Arc,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountInfo {
    pub name: String,
    pub username: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageInfo {
    pub id: i32,
    pub text: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallbackInfo {
    pub id: i64,
    pub data: Vec<u8>,
}
impl CallbackInfo {
    pub fn answer(&self) -> AnswerCallback {
        AnswerCallback::new(self.id)
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventKind {
    Message(MessageInfo),
    Callback(CallbackInfo),
    Other,
}

/// The transport supplies raw client/update objects, without an application DI map.
pub trait RawSource: Send + Sync {
    fn get(&self, id: TypeId) -> Option<&(dyn Any + Send + Sync)>;
}
#[derive(Clone)]
pub struct Event {
    pub account: AccountInfo,
    pub chat: Option<i64>,
    pub sender: Option<i64>,
    pub kind: EventKind,
    pub driver: Arc<dyn Driver>,
    pub raw: Option<Arc<dyn RawSource>>,
}
impl Event {
    pub fn text(&self) -> Option<&str> {
        match &self.kind {
            EventKind::Message(m) => m.text.as_deref(),
            _ => None,
        }
    }
    pub fn command(&self) -> Option<(&str, &str)> {
        let text = self.text()?.trim().strip_prefix('/')?;
        let end = text.find(char::is_whitespace).unwrap_or(text.len());
        let (head, rest) = text.split_at(end);
        let name = if let Some((name, username)) = head.split_once('@') {
            if !self
                .account
                .username
                .as_deref()
                .is_some_and(|own| own.eq_ignore_ascii_case(username))
            {
                return None;
            }
            name
        } else {
            head
        };
        Some((name, rest.trim()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Rejection {
    #[error("event does not provide {0}")]
    Missing(&'static str),
    #[error("invalid input: {0}")]
    Invalid(String),
}
/// Missing extractors skip the route; invalid input terminates dispatch as rejected.
/// Extraction can be asynchronous for application-defined extractors.
pub trait FromEvent<S>: Sized + Send {
    fn from_event<'a>(event: &'a Event, state: &'a S) -> BoxFuture<'a, Result<Self, Rejection>>;
}
pub struct Text(pub String);
pub struct Message(pub MessageInfo);
pub struct Sender(pub i64);
pub struct Chat(pub i64);
pub struct Account(pub AccountInfo);
pub struct Callback(pub CallbackInfo);
pub struct State<S>(pub S);
pub struct Command<T>(pub T);
pub struct Data<T>(pub T);
pub struct Tg(pub Telegram);
pub struct Raw<T>(pub T);

macro_rules! simple {
    ($ty:ty, $event:ident => $body:expr) => {
        impl<S: Sync> FromEvent<S> for $ty {
            fn from_event<'a>(
                $event: &'a Event,
                _: &'a S,
            ) -> BoxFuture<'a, Result<Self, Rejection>> {
                Box::pin(async move { $body })
            }
        }
    };
}
simple!(Text, e => e.text().map(|t| Text(t.to_owned())).ok_or(Rejection::Missing("text")));
simple!(Message, e => match &e.kind { EventKind::Message(m) => Ok(Message(m.clone())), _ => Err(Rejection::Missing("message")) });
simple!(Sender, e => e.sender.map(Sender).ok_or(Rejection::Missing("sender")));
simple!(Chat, e => e.chat.map(Chat).ok_or(Rejection::Missing("chat")));
simple!(Account, e => Ok(Account(e.account.clone())));
simple!(Callback, e => match &e.kind { EventKind::Callback(c) => Ok(Callback(c.clone())), _ => Err(Rejection::Missing("callback")) });
simple!(Tg, e => Ok(Tg(Telegram::new(e.driver.clone()))));
impl<S: Clone + Send + Sync> FromEvent<S> for State<S> {
    fn from_event<'a>(_: &'a Event, state: &'a S) -> BoxFuture<'a, Result<Self, Rejection>> {
        Box::pin(async move { Ok(State(state.clone())) })
    }
}
impl<S: Sync, T: CommandSpec> FromEvent<S> for Command<T> {
    fn from_event<'a>(event: &'a Event, _: &'a S) -> BoxFuture<'a, Result<Self, Rejection>> {
        Box::pin(async move {
            let (name, input) = event.command().ok_or(Rejection::Missing("command"))?;
            if name != T::NAME {
                return Err(Rejection::Missing("command"));
            }
            T::parse(input)
                .map(Command)
                .map_err(|e| Rejection::Invalid(e.to_string()))
        })
    }
}
impl<S: Sync, T: CallbackData> FromEvent<S> for Data<T> {
    fn from_event<'a>(event: &'a Event, _: &'a S) -> BoxFuture<'a, Result<Self, Rejection>> {
        Box::pin(async move {
            let EventKind::Callback(c) = &event.kind else {
                return Err(Rejection::Missing("callback"));
            };
            T::decode(&c.data)
                .map(Data)
                .map_err(|e| Rejection::Invalid(e.to_string()))
        })
    }
}
impl<S: Sync, T: Clone + Send + Sync + 'static> FromEvent<S> for Raw<T> {
    fn from_event<'a>(event: &'a Event, _: &'a S) -> BoxFuture<'a, Result<Self, Rejection>> {
        Box::pin(async move {
            event
                .raw
                .as_ref()
                .and_then(|r| r.get(TypeId::of::<T>()))
                .and_then(|r| r.downcast_ref::<T>())
                .cloned()
                .map(Raw)
                .ok_or(Rejection::Missing(std::any::type_name::<T>()))
        })
    }
}
