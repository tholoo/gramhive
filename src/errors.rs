use grammers_client::InvocationError;
use thiserror::Error;

use crate::commands::CommandInput;

#[derive(Error, Debug)]
pub enum ExtractionError {
    #[error("missing argument")]
    Missing,

    #[error("mismatched types (expected {expected:?}, found {found:?})")]
    Mismatched { expected: String, found: String },

    #[error("client invocation error")]
    Invocation(#[from] InvocationError),

    #[error("unknown argument error")]
    Other,
}

#[derive(Error, Debug)]
#[error("error extracting `{var_name}`: {source}")]
pub struct ArgumentError {
    pub var_name: String,
    pub command_input: CommandInput,

    #[source]
    pub source: ExtractionError,
}

impl ExtractionError {
    pub fn with_context(
        self,
        var: impl Into<String>,
        command_input: CommandInput,
    ) -> ArgumentError {
        ArgumentError {
            var_name: var.into(),
            command_input,
            source: self,
        }
    }
}
