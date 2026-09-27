use gramhive_core::*;
use grammers_client::{
    Client,
    message::{InputMessage, ReplyMarkup},
    tl,
    update::Update,
};
use grammers_session::types::PeerRef;
use std::{
    any::{Any, TypeId},
    sync::Arc,
};

fn failure(error: impl std::fmt::Display) -> Failure {
    Failure(error.to_string())
}
/// A lightweight handle holding the original upstream update and client.
pub struct TelegramDriver {
    client: Client,
    update: Update,
}
impl RawSource for TelegramDriver {
    fn get(&self, id: TypeId) -> Option<&(dyn Any + Send + Sync)> {
        if id == TypeId::of::<Client>() {
            return Some(&self.client);
        }
        if id == TypeId::of::<Update>() {
            return Some(&self.update);
        }
        match &self.update {
            Update::NewMessage(m) | Update::MessageEdited(m)
                if id == TypeId::of::<grammers_client::message::Message>() =>
            {
                Some(&**m)
            }
            Update::CallbackQuery(c)
                if id == TypeId::of::<grammers_client::update::CallbackQuery>() =>
            {
                Some(c)
            }
            _ => None,
        }
    }
}
/// Ignores outgoing new messages to avoid echo loops; preserves other updates for Raw<Update>.
pub fn translate(account: AccountInfo, client: Client, update: Update) -> Option<Event> {
    let (kind, chat, sender) = match &update {
        Update::NewMessage(m) => {
            if m.outgoing() {
                return None;
            }
            (
                EventKind::Message(MessageInfo {
                    id: m.id(),
                    text: (!m.text().is_empty()).then(|| m.text().to_owned()),
                }),
                m.peer_id().bot_api_dialog_id(),
                m.sender_id().and_then(|id| id.bot_api_dialog_id()),
            )
        }
        Update::CallbackQuery(c) => {
            let id = match &c.raw {
                tl::enums::Update::BotCallbackQuery(c) => c.query_id,
                tl::enums::Update::InlineBotCallbackQuery(c) => c.query_id,
                _ => return None,
            };
            (
                EventKind::Callback(CallbackInfo {
                    id,
                    data: c.data().to_vec(),
                }),
                if c.is_from_inline() {
                    None
                } else {
                    c.peer_id().bot_api_dialog_id()
                },
                c.sender_id().bot_api_dialog_id(),
            )
        }
        _ => (EventKind::Other, None, None),
    };
    let driver = Arc::new(TelegramDriver { client, update });
    Some(Event {
        account,
        kind,
        chat,
        sender,
        driver: driver.clone(),
        raw: Some(driver),
    })
}
impl TelegramDriver {
    async fn peer(&self) -> Result<PeerRef, Failure> {
        let peer = match &self.update {
            Update::NewMessage(m) | Update::MessageEdited(m) => m.peer_ref().await,
            Update::CallbackQuery(c) if !c.is_from_inline() => c.peer_ref().await,
            _ => {
                return Err(Failure(
                    "event has no chat target; use raw grammers for this operation".into(),
                ));
            }
        };
        peer.map_err(failure)?
            .ok_or_else(|| Failure("Telegram peer is unavailable in the session".into()))
    }
    fn message_id(&self) -> Result<i32, Failure> {
        match &self.update {
            Update::NewMessage(m) | Update::MessageEdited(m) => Ok(m.id()),
            Update::CallbackQuery(c) => match &c.raw {
                tl::enums::Update::BotCallbackQuery(c) => Ok(c.msg_id),
                _ => Err(Failure("inline callbacks have no chat message ID".into())),
            },
            _ => Err(Failure("event has no message target".into())),
        }
    }
    async fn input(&self, body: Reply) -> Result<InputMessage, Failure> {
        let markup = markup(&body)?;
        let mut input = InputMessage::new().text(body.text).reply_markup(markup);
        if let Some(file) = body.file {
            input = input.document(self.client.upload_file(file).await.map_err(failure)?);
        }
        Ok(input)
    }
}
fn markup(body: &Reply) -> Result<ReplyMarkup, Failure> {
    let rows = body
        .buttons
        .iter()
        .map(|row| {
            row.iter()
                .map(|button| {
                    codec::check(button.data()).map_err(failure)?;
                    Ok(grammers_client::message::Button::data(
                        &button.label,
                        button.data(),
                    ))
                })
                .collect::<Result<Vec<_>, Failure>>()
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(ReplyMarkup::from_buttons(&rows))
}
impl Driver for TelegramDriver {
    fn execute(&self, operation: Operation) -> BoxFuture<'_, Result<Option<i32>, Failure>> {
        Box::pin(async move {
            match operation {
                Operation::Send { body, reply } => {
                    let peer = self.peer().await?;
                    let mut input = self.input(body).await?;
                    if reply {
                        input = input.reply_to(Some(self.message_id()?));
                    }
                    Ok(Some(
                        self.client
                            .send_message(peer, input)
                            .await
                            .map_err(failure)?
                            .id(),
                    ))
                }
                Operation::Edit { message, body } => {
                    if message.is_none()
                        && let Update::CallbackQuery(c) = &self.update
                        && let tl::enums::Update::InlineBotCallbackQuery(c) = &c.raw
                    {
                        if body.file.is_some() {
                            return Err(Failure("inline file edits require raw grammers".into()));
                        }
                        let reply_markup = Some(markup(&body)?.raw);
                        self.client
                            .invoke(&tl::functions::messages::EditInlineBotMessage {
                                no_webpage: true,
                                invert_media: false,
                                id: c.msg_id.clone(),
                                message: Some(body.text),
                                media: None,
                                reply_markup,
                                entities: None,
                                rich_message: None,
                            })
                            .await
                            .map_err(failure)?;
                        return Ok(None);
                    }
                    let id = match message {
                        Some(id) => id,
                        None => self.message_id()?,
                    };
                    self.client
                        .edit_message(self.peer().await?, id, self.input(body).await?)
                        .await
                        .map_err(failure)?;
                    Ok(None)
                }
                Operation::Delete { message } => {
                    let id = match message {
                        Some(id) => id,
                        None => self.message_id()?,
                    };
                    self.client
                        .delete_messages(self.peer().await?, &[id])
                        .await
                        .map_err(failure)?;
                    Ok(None)
                }
                Operation::Answer { callback, text } => {
                    self.client
                        .invoke(&tl::functions::messages::SetBotCallbackAnswer {
                            alert: false,
                            query_id: callback,
                            message: text,
                            url: None,
                            cache_time: 0,
                        })
                        .await
                        .map_err(failure)?;
                    Ok(None)
                }
            }
        })
    }
}
