use async_trait::async_trait;
use thiserror::Error;

use crate::{ChatRequest, ChatResponse, ChatStream, ModelInfo};

pub type Result<T> = std::result::Result<T, LlmError>;

#[derive(Debug, Error)]
pub enum LlmError {
    #[error("missing provider field: {0}")]
    MissingField(&'static str),
    #[error(
        "API key/token environment variable is not set: {env}. Set {env} before starting chat."
    )]
    MissingApiKeyEnv { env: String },
    #[error("API key/token environment variable is empty: {env}. Set {env} to a non-empty value.")]
    EmptyApiKeyEnv { env: String },
    #[error("provider request construction failed: {0}")]
    RequestBuild(String),
    #[error("{provider} request failed: {message}")]
    Http { provider: String, message: String },
    #[error("{provider} returned HTTP {status}: {body_summary}")]
    HttpStatus {
        provider: String,
        status: u16,
        body_summary: String,
    },
    #[error("{provider} response could not be parsed: {body_summary}")]
    ResponseParse {
        provider: String,
        body_summary: String,
    },
    #[error("{0}")]
    NotImplemented(&'static str),
}

#[async_trait]
pub trait LlmProvider: Send + Sync {
    async fn chat(&self, request: ChatRequest) -> Result<ChatResponse>;
    async fn stream_chat(&self, request: ChatRequest) -> Result<ChatStream>;
    async fn models(&self) -> Result<Vec<ModelInfo>>;
    fn provider_name(&self) -> &str;
}
