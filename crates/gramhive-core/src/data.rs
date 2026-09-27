use std::str::FromStr;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct CommandError(pub String);

/// Parses arguments after the command name. Implementable without a derive.
pub trait CommandSpec: Sized + Send + 'static {
    const NAME: &'static str;
    const DESCRIPTION: Option<&'static str> = None;
    fn parse(input: &str) -> Result<Self, CommandError>;
}

/// Deliberately small whitespace parser. No quoting or shell expansion.
pub struct Arguments<'a>(&'a str);
impl<'a> Arguments<'a> {
    pub fn new(input: &'a str) -> Self {
        Self(input.trim())
    }
    pub fn take<T: FromStr>(&mut self, name: &str) -> Result<T, CommandError> {
        let end = self.0.find(char::is_whitespace).unwrap_or(self.0.len());
        let token = &self.0[..end];
        if token.is_empty() {
            return Err(CommandError(format!("missing argument `{name}`")));
        }
        self.0 = self.0[end..].trim_start();
        token
            .parse()
            .map_err(|_| CommandError(format!("invalid argument `{name}`")))
    }
    pub fn rest(&mut self) -> Option<String> {
        let value = self.0.trim();
        self.0 = "";
        (!value.is_empty()).then(|| value.to_owned())
    }
    pub fn finish(self) -> Result<(), CommandError> {
        if self.0.is_empty() {
            Ok(())
        } else {
            Err(CommandError("unexpected extra arguments".into()))
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CallbackDataError {
    #[error("callback data is {0} bytes; Telegram allows 1..=64 bytes")]
    Size(usize),
    #[error("malformed callback data: {0}")]
    Malformed(&'static str),
}

pub trait CallbackData: Sized + Send + 'static {
    const PREFIX: &'static str;
    fn encode(&self) -> Result<Vec<u8>, CallbackDataError>;
    fn decode(data: &[u8]) -> Result<Self, CallbackDataError>;
}

/// Version 1: length-prefixed UTF-8 prefix, variant name, then field strings.
/// Names keep existing variants stable when an enum is reordered or extended.
#[doc(hidden)]
pub mod codec {
    use super::CallbackDataError as Error;
    pub fn check(data: &[u8]) -> Result<(), Error> {
        if data.is_empty() || data.len() > 64 {
            Err(Error::Size(data.len()))
        } else {
            Ok(())
        }
    }
    pub fn push(out: &mut Vec<u8>, value: &str) -> Result<(), Error> {
        let len =
            u8::try_from(value.len()).map_err(|_| Error::Size(out.len() + value.len() + 1))?;
        out.push(len);
        out.extend_from_slice(value.as_bytes());
        check(out)
    }
    pub fn header(prefix: &str, variant: &str) -> Result<Vec<u8>, Error> {
        let mut out = vec![1];
        push(&mut out, prefix)?;
        push(&mut out, variant)?;
        Ok(out)
    }
    pub fn take<'a>(input: &mut &'a [u8]) -> Result<&'a str, Error> {
        let (&len, rest) = input
            .split_first()
            .ok_or(Error::Malformed("missing field"))?;
        let (value, rest) = rest
            .split_at_checked(len as usize)
            .ok_or(Error::Malformed("truncated field"))?;
        *input = rest;
        std::str::from_utf8(value).map_err(|_| Error::Malformed("invalid UTF-8"))
    }
    pub fn open<'a>(mut input: &'a [u8], prefix: &str) -> Result<(&'a str, &'a [u8]), Error> {
        check(input)?;
        if input[0] != 1 {
            return Err(Error::Malformed("unsupported version"));
        }
        input = &input[1..];
        if take(&mut input)? != prefix {
            return Err(Error::Malformed("wrong prefix"));
        }
        let variant = take(&mut input)?;
        Ok((variant, input))
    }
    pub fn matches(input: &[u8], prefix: &str) -> bool {
        // Match the namespace even when the version/variant/body is invalid:
        // extraction should reject corrupt data rather than silently falling through.
        input
            .get(1..)
            .is_some_and(|mut data| take(&mut data).ok() == Some(prefix))
    }
}
