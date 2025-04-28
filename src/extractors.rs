use core::slice;
use std::{marker::PhantomData, sync::Arc};

use async_trait::async_trait;
use grammers_client::{Client, grammers_tl_types as tl, types::Message};
use regex::Regex;

use crate::{commands::CommandInput, errors::ExtractionError};

#[async_trait]
pub trait Extractor: Send + Sync {
    type Output: Send + Sync;

    async fn extract(
        &self,
        client: Arc<Client>,
        message: &Message,
        command_input: &CommandInput,
    ) -> Result<Self::Output, ExtractionError>;
}

pub struct RegexExtractor<F> {
    pub regex: Regex,
    _marker: PhantomData<F>,
}

#[async_trait]
impl<F> Extractor for RegexExtractor<F>
where
    F: std::str::FromStr + Send + Sync + 'static,
    F::Err: std::error::Error + Send + Sync + 'static,
{
    type Output = F;

    async fn extract(
        &self,
        _client: Arc<Client>,
        message: &Message,
        _cmd: &CommandInput,
    ) -> Result<Self::Output, ExtractionError> {
        if let Some(mat) = self.regex.captures(message.text()).and_then(|c| c.get(0)) {
            let s = mat.as_str();
            s.parse::<F>().map_err(|_| ExtractionError::Mismatched {
                expected: std::any::type_name::<F>().to_string(),
                found: s.to_string(),
            })
        } else {
            Err(ExtractionError::Missing)
        }
    }
}

pub struct ArgumentExtractor<F> {
    pub index: usize,
    _marker: PhantomData<F>,
}

impl<F> ArgumentExtractor<F> {
    pub fn new(index: usize) -> Self {
        Self {
            index,
            _marker: PhantomData,
        }
    }
}

#[async_trait]
impl<F> Extractor for ArgumentExtractor<F>
where
    F: std::str::FromStr + Send + Sync + 'static,
{
    type Output = F;

    async fn extract(
        &self,
        _client: Arc<Client>,
        _msg: &Message,
        command_input: &CommandInput,
    ) -> Result<F, ExtractionError> {
        let raw = command_input
            .args
            .get(self.index)
            .ok_or(ExtractionError::Missing)?;

        raw.parse::<F>().map_err(|_| ExtractionError::Mismatched {
            expected: std::any::type_name::<F>().to_string(),
            found: raw.clone(),
        })
    }
}

pub struct ReplyExtractor {}

#[async_trait]
impl Extractor for ReplyExtractor {
    type Output = Message;

    async fn extract(
        &self,
        client: Arc<Client>,
        message: &Message,
        _cmd: &CommandInput,
    ) -> Result<Self::Output, ExtractionError> {
        if let Some(tl::enums::MessageReplyHeader::Header(reply_header)) = message.reply_header() {
            if let Some(reply_msg_id) = reply_header.reply_to_msg_id {
                let message = client
                    .get_messages_by_id(message.chat(), slice::from_ref(&reply_msg_id))
                    .await
                    .map(|msgs| msgs.into_iter().flatten().next());
                match message {
                    Ok(Some(msg)) => Ok(msg),
                    Ok(None) => Err(ExtractionError::Missing),
                    Err(err) => Err(ExtractionError::Invocation(err)),
                }
            } else {
                return Err(ExtractionError::Missing);
            }
        } else {
            return Err(ExtractionError::Missing);
        }
    }
}

pub struct InputExtractor {}

#[async_trait]
impl Extractor for InputExtractor {
    type Output = String;

    async fn extract(
        &self,
        _client: Arc<Client>,
        _message: &Message,
        command_input: &CommandInput,
    ) -> Result<Self::Output, ExtractionError> {
        match command_input.input.clone() {
            Some(input) => Ok(input),
            None => Err(ExtractionError::Missing),
        }
    }
}
