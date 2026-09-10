pub mod cloudflare_gateway;
mod limits;
pub mod mock;
pub mod openai_compat;
pub mod openai_format;
pub mod provider;
pub mod streaming;
pub mod types;

pub use cloudflare_gateway::CloudflareAiGatewayProvider;
pub use mock::MockProvider;
pub use openai_compat::OpenAiCompatibleProvider;
pub use provider::{
    ollama_cloud_provider, validate_credential_env_name, validate_provider_endpoint, LlmError,
    LlmProvider, Result, OLLAMA_CLOUD_API_KEY_ENV, OLLAMA_CLOUD_BASE_URL, OLLAMA_CLOUD_MODELS,
};
pub use streaming::{
    detect_repetition_period, ChatChunk, ChatStream, ChatStreamUpdate, ChatToolCallDelta,
};
pub use types::{
    ChatMessage, ChatRequest, ChatResponse, ChatToolCall, ChatToolDefinition, ModelInfo, TokenUsage,
};
