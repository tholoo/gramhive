use core::slice;
use std::marker::PhantomData;

use dialoguer::{Input, Password};
use grammers_client::{
    Client, Config, InitParams, InvocationError, SignInError, grammers_tl_types as tl,
    session::Session, types::Message,
};

pub async fn get_reply(
    client: Client,
    message: Message,
) -> Result<Option<Message>, InvocationError> {
    if let Some(tl::enums::MessageReplyHeader::Header(reply_header)) = message.reply_header() {
        if let Some(reply_msg_id) = reply_header.reply_to_msg_id {
            return client
                .get_messages_by_id(message.chat(), slice::from_ref(&reply_msg_id))
                .await
                .map(|msgs| msgs.into_iter().flatten().next());
        }
    }
    Ok(None)
}

/// Phantom‐types for mode
pub struct Bot;
pub struct User;

/// Phantom‐types for state
pub struct Disconnected;
pub struct Connected;

pub struct ClientBuilder<Mode, State> {
    // mode‐specific credential
    token: Option<String>,
    phone: Option<String>,

    // shared config
    api_id: Option<i32>,
    api_hash: Option<String>,
    session: Option<String>,
    catch_up: bool,

    // once `.connect()` runs, we stash the live client here
    client: Option<Client>,

    _mode: PhantomData<Mode>,
    _state: PhantomData<State>,
}

impl ClientBuilder<Bot, Disconnected> {
    pub fn bot<T: Into<String>>(token: T) -> Self {
        ClientBuilder {
            token: Some(token.into()),
            phone: None,
            api_id: None,
            api_hash: None,
            session: None,
            catch_up: false,
            client: None,
            _mode: PhantomData,
            _state: PhantomData,
        }
    }
}

impl ClientBuilder<User, Disconnected> {
    pub fn user<T: Into<String>>(phone: T) -> Self {
        ClientBuilder {
            token: None,
            phone: Some(phone.into()),
            api_id: None,
            api_hash: None,
            session: None,
            catch_up: false,
            client: None,
            _mode: PhantomData,
            _state: PhantomData,
        }
    }
}

// —— shared setters (only in Disconnected) —— //

impl<M> ClientBuilder<M, Disconnected> {
    pub fn api_id(mut self, id: i32) -> Self {
        self.api_id = Some(id);
        self
    }

    pub fn api_hash<S: Into<String>>(mut self, hash: S) -> Self {
        self.api_hash = Some(hash.into());
        self
    }

    pub fn session<S: Into<String>>(mut self, session: S) -> Self {
        self.session = Some(session.into());
        self
    }

    pub fn catch_up(mut self, yes: bool) -> Self {
        self.catch_up = yes;
        self
    }

    /// perform MTProto handshake & session load/create
    pub async fn connect(self) -> Result<ClientBuilder<M, Connected>, Box<dyn std::error::Error>> {
        // validate
        let api_id = self.api_id.ok_or("missing api_id")?;
        let api_hash = self.api_hash.clone().ok_or("missing api_hash")?;
        let sess_path = self.session.clone().ok_or("missing session(...)")?;

        // load or create the session file
        let session = Session::load_file_or_create(&sess_path)?;
        let client = Client::connect(Config {
            session,
            api_id,
            api_hash: api_hash.clone(),
            params: InitParams {
                catch_up: self.catch_up,
                ..Default::default()
            },
        })
        .await?;

        Ok(ClientBuilder {
            token: self.token,
            phone: self.phone,
            api_id: Some(api_id),
            api_hash: Some(api_hash),
            session: Some(sess_path),
            catch_up: self.catch_up,
            client: Some(client),
            _mode: PhantomData,
            _state: PhantomData,
        })
    }
}

// —— authorize for Bot, Connected only —— //

impl ClientBuilder<Bot, Connected> {
    pub async fn authorize(mut self) -> Result<Client, Box<dyn std::error::Error>> {
        let client = self.client.as_mut().unwrap();
        let token = self.token.take().unwrap();

        if !client.is_authorized().await? {
            client.bot_sign_in(&token).await?;
            // save updated session
            client
                .session()
                .save_to_file(self.session.as_ref().unwrap())?;
        }

        Ok(client.clone())
    }
}

// —— authorize for User, Connected only —— //

impl ClientBuilder<User, Connected> {
    pub async fn authorize(mut self) -> Result<Client, Box<dyn std::error::Error>> {
        let client = self.client.as_mut().unwrap();
        let phone = self.phone.take().unwrap();

        if !client.is_authorized().await? {
            let login_token = client.request_login_code(&phone).await?;
            let code: String = Input::new().with_prompt("Enter code").interact_text()?;
            match client.sign_in(&login_token, &code).await {
                Err(SignInError::PasswordRequired(pw_tok)) => {
                    let hint = pw_tok.hint().unwrap_or_default();
                    let password = Password::new()
                        .with_prompt(format!("2FA password (hint: {})", hint))
                        .interact()?;
                    client.check_password(pw_tok, password.trim()).await?;
                }
                other => {
                    other?;
                }
            }
            client
                .session()
                .save_to_file(self.session.as_ref().unwrap())?;
        }

        Ok(client.clone())
    }
}
