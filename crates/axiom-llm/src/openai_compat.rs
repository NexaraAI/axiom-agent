use async_trait::async_trait;
use reqwest::RequestBuilder;

use crate::{
    openai_format::{chat_request_body, parse_chat_response, read_secret_env, summarize_body},
    ChatRequest, ChatResponse, ChatStream, LlmError, LlmProvider, ModelInfo, Result,
};

#[derive(Debug, Clone)]
pub struct OpenAiCompatibleProvider {
    name: String,
    base_url: String,
    api_key_env: String,
    client: reqwest::Client,
}

impl OpenAiCompatibleProvider {
    pub fn new(
        name: impl Into<String>,
        base_url: impl Into<String>,
        api_key_env: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            base_url: base_url.into(),
            api_key_env: api_key_env.into(),
            client: reqwest::Client::new(),
        }
    }

    pub fn chat_endpoint(&self) -> String {
        format!("{}/chat/completions", self.base_url.trim_end_matches('/'))
    }

    pub fn validate_request(&self, request: &ChatRequest) -> Result<()> {
        if self.base_url.trim().is_empty() {
            return Err(LlmError::MissingField("base_url"));
        }
        if self.api_key_env.trim().is_empty() {
            return Err(LlmError::MissingField("api_key_env"));
        }
        if request.model.trim().is_empty() {
            return Err(LlmError::MissingField("model"));
        }
        Ok(())
    }

    pub fn build_chat_request(&self, request: &ChatRequest) -> Result<RequestBuilder> {
        self.validate_request(request)?;
        let api_key = read_secret_env(&self.api_key_env)?;
        let body = chat_request_body(request);

        Ok(self
            .client
            .post(self.chat_endpoint())
            .bearer_auth(api_key)
            .json(&body))
    }

    pub fn chat_request_body(&self, request: &ChatRequest) -> Result<serde_json::Value> {
        self.validate_request(request)?;
        Ok(chat_request_body(request))
    }
}

#[async_trait]
impl LlmProvider for OpenAiCompatibleProvider {
    async fn chat(&self, request: ChatRequest) -> Result<ChatResponse> {
        let response = self
            .build_chat_request(&request)?
            .send()
            .await
            .map_err(|error| LlmError::Http {
                provider: self.name.clone(),
                message: error.to_string(),
            })?;

        let status = response.status();
        let body = response.text().await.map_err(|error| LlmError::Http {
            provider: self.name.clone(),
            message: error.to_string(),
        })?;

        if !status.is_success() {
            return Err(LlmError::HttpStatus {
                provider: self.name.clone(),
                status: status.as_u16(),
                body_summary: summarize_body(&body),
            });
        }

        parse_chat_response(&self.name, &request.model, &body)
    }

    async fn stream_chat(&self, request: ChatRequest) -> Result<ChatStream> {
        let _builder = self.build_chat_request(&request)?;
        Err(LlmError::NotImplemented(
            "openai-compatible streaming is not implemented in this stage",
        ))
    }

    async fn models(&self) -> Result<Vec<ModelInfo>> {
        Ok(Vec::new())
    }

    fn provider_name(&self) -> &str {
        &self.name
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ChatMessage;

    #[test]
    fn builds_openai_compatible_endpoint() {
        let provider =
            OpenAiCompatibleProvider::new("local", "http://localhost:8000/v1/", "LOCAL_KEY");

        assert_eq!(
            provider.chat_endpoint(),
            "http://localhost:8000/v1/chat/completions"
        );
    }

    #[test]
    fn request_body_uses_configured_model_and_messages() {
        let provider =
            OpenAiCompatibleProvider::new("local", "http://localhost:8000/v1", "LOCAL_KEY");
        let request = sample_request();

        let body = provider
            .chat_request_body(&request)
            .expect("request body should build");

        assert_eq!(body["model"], "test-model");
        assert_eq!(body["messages"][0]["role"], "user");
        assert_eq!(body["messages"][0]["content"], "hello");
    }

    #[test]
    fn missing_openai_env_var_is_reported() {
        let env = "AXIOM_TEST_OPENAI_KEY_SHOULD_NOT_EXIST_6F409838";
        std::env::remove_var(env);
        let provider = OpenAiCompatibleProvider::new("local", "http://localhost:8000/v1", env);

        let error = provider
            .build_chat_request(&sample_request())
            .expect_err("missing env should fail");

        assert!(matches!(error, LlmError::MissingApiKeyEnv { .. }));
        assert!(error.to_string().contains(env));
    }

    fn sample_request() -> ChatRequest {
        ChatRequest {
            model: "test-model".to_string(),
            messages: vec![ChatMessage {
                role: "user".to_string(),
                content: "hello".to_string(),
            }],
            temperature: Some(0.2),
            max_tokens: Some(64),
            stream: false,
            metadata: None,
            provider_options: None,
        }
    }
}
