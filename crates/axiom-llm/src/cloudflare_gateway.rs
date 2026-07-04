use async_trait::async_trait;
use reqwest::RequestBuilder;

use crate::{
    openai_format::{chat_request_body, parse_chat_response, read_secret_env, summarize_body},
    ChatRequest, ChatResponse, ChatStream, LlmError, LlmProvider, ModelInfo, Result,
};

#[derive(Debug, Clone)]
pub struct CloudflareAiGatewayProvider {
    name: String,
    account_id: String,
    gateway_id: String,
    api_token_env: String,
    base_url: String,
    client: reqwest::Client,
}

impl CloudflareAiGatewayProvider {
    pub fn new(
        name: impl Into<String>,
        account_id: impl Into<String>,
        gateway_id: impl Into<String>,
        api_token_env: impl Into<String>,
        base_url_template: impl Into<String>,
    ) -> Self {
        let account_id = account_id.into();
        let base_url = base_url_template
            .into()
            .replace("{account_id}", &account_id);

        Self {
            name: name.into(),
            account_id,
            gateway_id: gateway_id.into(),
            api_token_env: api_token_env.into(),
            base_url,
            client: reqwest::Client::new(),
        }
    }

    pub fn default_base_url_template() -> &'static str {
        "https://api.cloudflare.com/client/v4/accounts/{account_id}/ai/v1"
    }

    pub fn chat_endpoint(&self) -> String {
        format!("{}/chat/completions", self.base_url.trim_end_matches('/'))
    }

    pub fn validate_request(&self, request: &ChatRequest) -> Result<()> {
        if self.account_id.trim().is_empty() {
            return Err(LlmError::MissingField("account_id"));
        }
        if self.gateway_id.trim().is_empty() {
            return Err(LlmError::MissingField("gateway_id"));
        }
        if self.api_token_env.trim().is_empty() {
            return Err(LlmError::MissingField("api_token_env"));
        }
        if request.model.trim().is_empty() {
            return Err(LlmError::MissingField("model"));
        }
        Ok(())
    }

    pub fn build_chat_request(&self, request: &ChatRequest) -> Result<RequestBuilder> {
        self.validate_request(request)?;
        let token = read_secret_env(&self.api_token_env)?;
        let body = chat_request_body(request);

        Ok(self
            .client
            .post(self.chat_endpoint())
            .bearer_auth(token)
            .header("cf-aig-gateway-id", &self.gateway_id)
            .json(&body))
    }

    pub fn chat_request_body(&self, request: &ChatRequest) -> Result<serde_json::Value> {
        self.validate_request(request)?;
        Ok(chat_request_body(request))
    }
}

#[async_trait]
impl LlmProvider for CloudflareAiGatewayProvider {
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
            "cloudflare ai gateway streaming is not implemented in this stage",
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
    fn builds_cloudflare_chat_endpoint_from_account_id() {
        let provider = CloudflareAiGatewayProvider::new(
            "cloudflare",
            "account123",
            "default",
            "CF_TOKEN",
            CloudflareAiGatewayProvider::default_base_url_template(),
        );

        assert_eq!(
            provider.chat_endpoint(),
            "https://api.cloudflare.com/client/v4/accounts/account123/ai/v1/chat/completions"
        );
    }

    #[test]
    fn cloudflare_request_body_preserves_configured_model_name() {
        let provider = CloudflareAiGatewayProvider::new(
            "cloudflare",
            "account123",
            "default",
            "CF_TOKEN",
            CloudflareAiGatewayProvider::default_base_url_template(),
        );
        let request = sample_request();

        let body = provider
            .chat_request_body(&request)
            .expect("request body should build");

        assert_eq!(body["model"], "openai/gpt-4.1-mini");
        assert_eq!(body["messages"][0]["content"], "hello");
    }

    #[test]
    fn missing_cloudflare_token_env_var_is_reported() {
        let env = "AXIOM_TEST_CLOUDFLARE_TOKEN_SHOULD_NOT_EXIST_16FBD76B";
        std::env::remove_var(env);
        let provider = CloudflareAiGatewayProvider::new(
            "cloudflare",
            "account123",
            "default",
            env,
            CloudflareAiGatewayProvider::default_base_url_template(),
        );

        let error = provider
            .build_chat_request(&sample_request())
            .expect_err("missing env should fail");

        assert!(matches!(error, LlmError::MissingApiKeyEnv { .. }));
        assert!(error.to_string().contains(env));
    }

    fn sample_request() -> ChatRequest {
        ChatRequest {
            model: "openai/gpt-4.1-mini".to_string(),
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
