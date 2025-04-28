use async_trait::async_trait;
use grammers_client::{
    InvocationError, SignInError,
    client::{
        chats::{ParticipantIter, ProfilePhotoIter},
        dialogs::DialogIter,
    },
    grammers_tl_types as tl,
    session::Session,
    types::{Chat, ChatMap, InputMessage, LoginToken, PackedChat, Update, User},
};
use grammers_mtsender::{AuthorizationError, ReadError};
use mockall::automock;
use std::sync::Arc;

#[automock]
#[async_trait]
pub trait ClientTrait: Send + Sync {
    async fn invoke<R>(&self, request: &R) -> Result<R::Return, InvocationError>
    where
        R: tl::RemoteCall + Send + Sync + 'static;

    async fn invoke_in_dc<R>(&self, request: &R, dc_id: i32) -> Result<R::Return, InvocationError>
    where
        R: tl::RemoteCall + Send + Sync + 'static;

    async fn next_update(&self) -> Result<Update, InvocationError>;

    async fn next_raw_update(&self) -> Result<(tl::enums::Update, Arc<ChatMap>), InvocationError>;

    async fn step(&self) -> Result<(), ReadError>;

    async fn run_until_disconnected(&self) -> Result<(), ReadError>;

    async fn bot_sign_in(&self, token: &str) -> Result<User, AuthorizationError>;

    async fn request_login_code(&self, phone: &str) -> Result<LoginToken, AuthorizationError>;

    async fn sign_in(&self, token: &LoginToken, code: &str) -> Result<User, SignInError>;

    async fn sign_out(&self) -> Result<tl::enums::auth::LoggedOut, InvocationError>;

    fn session(&self) -> &Session;

    async fn sign_out_disconnect(&self) -> Result<(), InvocationError>;

    fn sync_update_state(&self);

    fn iter_dialogs(&self) -> DialogIter;

    async fn delete_dialog(&self, chat: &Chat) -> Result<(), InvocationError>;

    async fn mark_as_read(&self, chat: &Chat) -> Result<(), InvocationError>;

    async fn clear_mentions(&self, chat: &Chat) -> Result<(), InvocationError>;

    fn iter_participants(&self, chat: PackedChat) -> ParticipantIter;

    async fn kick_participant(
        &self,
        chat: PackedChat,
        user: PackedChat,
    ) -> Result<(), InvocationError>;

    fn iter_profile_photos(&self, chat: PackedChat) -> ProfilePhotoIter;

    async fn resolve_username(&self, username: &str) -> Result<Option<Chat>, InvocationError>;

    async fn get_me(&self) -> Result<User, InvocationError>;

    async fn unpack_chat(&self, packed_chat: PackedChat) -> Result<Chat, InvocationError>;
}

#[automock]
#[async_trait]
pub trait MessageTrait: Send + Sync {
    fn outgoing(&self) -> bool;
    fn mentioned(&self) -> bool;
    fn media_unread(&self) -> bool;
    fn silent(&self) -> bool;
    fn post(&self) -> bool;
    fn from_scheduled(&self) -> bool;
    fn edit_hide(&self) -> bool;
    fn pinned(&self) -> bool;
    fn id(&self) -> i32;

    fn sender(&self) -> Option<Chat>;
    fn chat(&self) -> Chat;
    fn forward_header(&self) -> Option<tl::enums::MessageFwdHeader>;
    fn via_bot_id(&self) -> Option<i64>;
    fn reply_header(&self) -> Option<tl::enums::MessageReplyHeader>;
    fn reply_to_message_id(&self) -> Option<i32>;

    fn text(&self) -> &str;

    fn date(&self) -> chrono::DateTime<chrono::Utc>;

    fn media(&self) -> Option<grammers_client::types::Media>;
    fn reply_markup(&self) -> Option<tl::enums::ReplyMarkup>;

    fn fmt_entities(&self) -> Option<&'static Vec<tl::enums::MessageEntity>>;

    fn view_count(&self) -> Option<i32>;
    fn forward_count(&self) -> Option<i32>;
    fn reply_count(&self) -> Option<i32>;
    fn reaction_count(&self) -> Option<i32>;
    fn edit_date(&self) -> Option<chrono::DateTime<chrono::Utc>>;

    fn post_author(&self) -> Option<&'static str>;

    fn grouped_id(&self) -> Option<i64>;

    fn restriction_reason(&self) -> Option<&'static Vec<tl::enums::RestrictionReason>>;

    fn action(&self) -> Option<&'static tl::enums::MessageAction>;

    fn photo(&self) -> Option<grammers_client::types::Photo>;

    async fn react<R>(&self, reactions: R) -> Result<(), InvocationError>
    where
        R: Into<grammers_client::types::InputReactions> + Send + Sync + 'static;

    async fn get_reply(&self) -> Result<Option<Self>, InvocationError>
    where
        Self: Sized + Send + Sync + 'static;

    async fn respond<M>(&self, message: M) -> Result<Self, InvocationError>
    where
        Self: Sized + Send + Sync + 'static,
        M: Into<InputMessage> + Send + Sync + 'static;

    async fn forward_to<C>(&self, chat: C) -> Result<Self, InvocationError>
    where
        Self: Sized + Send + Sync + 'static,
        C: Into<PackedChat> + Send + Sync + 'static;

    async fn edit<M>(&self, new_message: M) -> Result<(), InvocationError>
    where
        M: Into<InputMessage> + Send + Sync + 'static;

    async fn delete(&self) -> Result<(), InvocationError>;

    async fn mark_as_read(&self) -> Result<(), InvocationError>;

    async fn pin(&self) -> Result<(), InvocationError>;

    async fn unpin(&self) -> Result<(), InvocationError>;

    async fn refetch(&self) -> Result<(), InvocationError>;

    async fn reply<M: Into<InputMessage> + Send + 'static>(
        &self,
        message: M,
    ) -> Result<Self, InvocationError>
    where
        Self: Sized + Send + Sync + 'static;
}
