use async_trait::async_trait;

use reqwest::RequestBuilder;

use crate::{
    limits::{MAX_CHAT_HTTP_BODY_BYTES, MAX_STREAM_WIRE_BYTES},
    openai_format::{chat_request_body, parse_chat_response, read_secret_env, summarize_bytes},
    provider::{
        build_provider_http_client, ensure_response_content_length, read_response_body_limited,
        retry_transient, validate_provider_endpoint, SecretValue,
    },
    ChatRequest, ChatResponse, ChatStream, LlmError, LlmProvider, ModelInfo, Result,
};

#[derive(Debug, Clone)]
pub struct CloudflareAiGatewayProvider {
    name: String,
    account_id: String,
    gateway_id: String,
    api_token_env: String,
    api_token: Option<SecretValue>,
    base_url: String,
    client: std::result::Result<reqwest::Client, String>,
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
            api_token: None,
            base_url,
            client: build_provider_http_client(),
        }
    }

    pub fn default_base_url_template() -> &'static str {
        "https://api.cloudflare.com/client/v4/accounts/{account_id}/ai/v1"
    }

    /// Supply a token value directly so a credential loaded from the native
    /// credential manager never has to be exported into the process environment.
    pub fn with_api_token(mut self, api_token: impl Into<String>) -> Self {
        self.api_token = Some(SecretValue::new(api_token));
        self
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
        validate_cloudflare_identifier("account_id", &self.account_id, false)?;
        validate_cloudflare_identifier("gateway_id", &self.gateway_id, true)?;
        if self.api_token_env.trim().is_empty() {
            return Err(LlmError::MissingField("api_token_env"));
        }
        crate::validate_credential_env_name(&self.api_token_env)?;
        validate_provider_endpoint("base_url", &self.base_url, false)?;
        if request.model.trim().is_empty() {
            return Err(LlmError::MissingField("model"));
        }
        Ok(())
    }

    pub fn build_chat_request(&self, request: &ChatRequest) -> Result<RequestBuilder> {
        self.validate_request(request)?;
        let body = chat_request_body(request);
        let builder = self
            .client()?
            .post(self.chat_endpoint())
            .header("cf-aig-gateway-id", &self.gateway_id)
            .json(&body);
        if let Some(api_token) = self.api_token.as_ref() {
            if api_token.expose().trim().is_empty() {
                return Err(LlmError::EmptyApiKeyEnv {
                    env: self.api_token_env.clone(),
                });
            }
            Ok(builder.bearer_auth(api_token.expose()))
        } else {
            Ok(builder.bearer_auth(read_secret_env(&self.api_token_env)?))
        }
    }

    pub fn chat_request_body(&self, request: &ChatRequest) -> Result<serde_json::Value> {
        self.validate_request(request)?;
        Ok(chat_request_body(request))
    }

    fn client(&self) -> Result<&reqwest::Client> {
        self.client
            .as_ref()
            .map_err(|message| LlmError::RequestBuild(message.clone()))
    }
}

fn validate_cloudflare_identifier(field: &'static str, value: &str, allow_dot: bool) -> Result<()> {
    let allowed = |character: char| {
        character.is_ascii_alphanumeric()
            || matches!(character, '_' | '-')
            || (allow_dot && character == '.')
    };
    if value.len() <= 256 && value.chars().all(allowed) {
        Ok(())
    } else {
        Err(LlmError::UnsafeProviderIdentifier {
            field,
            reason: "use only ASCII letters, digits, `_`, `-`, and (for gateway IDs) `.`"
                .to_string(),
        })
    }
}

#[async_trait]
impl LlmProvider for CloudflareAiGatewayProvider {
    async fn chat(&self, request: ChatRequest) -> Result<ChatResponse> {
        retry_transient(|| self.chat_once(&request)).await
    }

    async fn stream_chat(&self, mut request: ChatRequest) -> Result<ChatStream> {
        request.stream = true;
        retry_transient(|| self.stream_once(&request)).await
    }

    async fn models(&self) -> Result<Vec<ModelInfo>> {
        Ok(Vec::new())
    }

    fn provider_name(&self) -> &str {
        &self.name
    }
}

impl CloudflareAiGatewayProvider {
    async fn chat_once(&self, request: &ChatRequest) -> Result<ChatResponse> {
        let response = self
            .build_chat_request(request)?
            .send()
            .await
            .map_err(|error| LlmError::Http {
                provider: self.name.clone(),
                message: error.to_string(),
            })?;

        self.handle_chat_response(response, request).await
    }

    async fn stream_once(&self, request: &ChatRequest) -> Result<ChatStream> {
        let response = self
            .build_chat_request(request)?
            .send()
            .await
            .map_err(|error| LlmError::Http {
                provider: self.name.clone(),
                message: error.to_string(),
            })?;
        self.handle_stream_response(response).await
    }

    async fn handle_chat_response(
        &self,
        response: reqwest::Response,
        request: &ChatRequest,
    ) -> Result<ChatResponse> {
        let (status, body) = read_response_body_limited(
            response,
            &self.name,
            "chat completion body",
            MAX_CHAT_HTTP_BODY_BYTES,
        )
        .await?;
        if !status.is_success() {
            return Err(LlmError::HttpStatus {
                provider: self.name.clone(),
                status: status.as_u16(),
                body_summary: summarize_bytes(&body),
            });
        }
        let body = std::str::from_utf8(&body).map_err(|_| LlmError::ResponseParse {
            provider: self.name.clone(),
            body_summary: summarize_bytes(&body),
        })?;
        parse_chat_response(&self.name, &request.model, body)
    }

    async fn handle_stream_response(&self, response: reqwest::Response) -> Result<ChatStream> {
        let status = response.status();
        if !status.is_success() {
            let (status, body) = read_response_body_limited(
                response,
                &self.name,
                "chat stream body",
                MAX_STREAM_WIRE_BYTES,
            )
            .await?;
            return Err(LlmError::HttpStatus {
                provider: self.name.clone(),
                status: status.as_u16(),
                body_summary: summarize_bytes(&body),
            });
        }
        ensure_response_content_length(
            &response,
            &self.name,
            "chat stream body",
            MAX_STREAM_WIRE_BYTES,
        )?;
        Ok(ChatStream::from_http(response, self.name.clone()))
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};

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

    #[test]
    fn direct_cloudflare_token_is_not_exported_or_exposed_by_debug() {
        let env = "AXIOM_TEST_DIRECT_CF_TOKEN_7A1D58C2";
        std::env::remove_var(env);
        let provider = CloudflareAiGatewayProvider::new(
            "cloudflare",
            "account123",
            "default",
            env,
            CloudflareAiGatewayProvider::default_base_url_template(),
        )
        .with_api_token("direct-cloudflare-secret");

        let request = provider
            .build_chat_request(&sample_request())
            .expect("direct token should build")
            .build()
            .expect("request should finalize");

        assert_eq!(
            request.headers()[reqwest::header::AUTHORIZATION],
            "Bearer direct-cloudflare-secret"
        );
        assert!(std::env::var_os(env).is_none());
        let debug = format!("{provider:?}");
        assert!(!debug.contains("direct-cloudflare-secret"));
        assert!(debug.contains("[REDACTED]"));
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
            tools: Vec::new(),
            tool_choice: None,
        }
    }

    #[test]
    fn cloudflare_gateway_rejects_cleartext_endpoint() {
        let provider = CloudflareAiGatewayProvider::new(
            "cloudflare",
            "account",
            "gateway",
            "CLOUDFLARE_API_TOKEN",
            "http://api.cloudflare.com/client/v4/accounts/{account_id}/ai/v1",
        );
        let request = sample_request();
        assert!(matches!(
            provider.validate_request(&request),
            Err(LlmError::UnsafeEndpoint { .. })
        ));
    }

    #[test]
    fn cloudflare_gateway_rejects_path_or_header_injection_identifiers() {
        for (account_id, gateway_id, field) in [
            ("account/../../other", "default", "account_id"),
            ("account", "default\r\nextra: value", "gateway_id"),
        ] {
            let provider = CloudflareAiGatewayProvider::new(
                "cloudflare",
                account_id,
                gateway_id,
                "CLOUDFLARE_API_TOKEN",
                CloudflareAiGatewayProvider::default_base_url_template(),
            );
            assert!(matches!(
                provider.validate_request(&sample_request()),
                Err(LlmError::UnsafeProviderIdentifier { field: actual, .. }) if actual == field
            ));
        }
    }

    #[tokio::test]
    async fn cloudflare_chat_and_stream_success_and_error_bodies_are_bounded() {
        let provider = CloudflareAiGatewayProvider::new(
            "cloudflare",
            "account",
            "gateway",
            "CLOUDFLARE_API_TOKEN",
            CloudflareAiGatewayProvider::default_base_url_template(),
        );
        for (streaming, status, declared_length, expected_resource) in [
            (
                false,
                "200 OK",
                MAX_CHAT_HTTP_BODY_BYTES + 1,
                "chat completion body",
            ),
            (
                false,
                "400 Bad Request",
                crate::limits::MAX_ERROR_HTTP_BODY_BYTES + 1,
                "provider error body",
            ),
            (
                true,
                "200 OK",
                MAX_STREAM_WIRE_BYTES + 1,
                "chat stream body",
            ),
            (
                true,
                "400 Bad Request",
                crate::limits::MAX_ERROR_HTTP_BODY_BYTES + 1,
                "provider error body",
            ),
        ] {
            let (response, server) = declared_response(status, declared_length).await;
            let error = if streaming {
                provider
                    .handle_stream_response(response)
                    .await
                    .expect_err("oversized stream response")
            } else {
                provider
                    .handle_chat_response(response, &sample_request())
                    .await
                    .expect_err("oversized chat response")
            };
            server.join().expect("server thread");
            assert!(matches!(
                error,
                LlmError::ResponseLimitExceeded { resource, .. } if resource == expected_resource
            ));
        }
    }

    async fn declared_response(
        status: &'static str,
        declared_length: usize,
    ) -> (reqwest::Response, std::thread::JoinHandle<()>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind test server");
        let address = listener.local_addr().expect("server address");
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept request");
            let mut request = [0_u8; 2_048];
            let _ = stream.read(&mut request).expect("read request");
            write!(
                stream,
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {declared_length}\r\nConnection: close\r\n\r\n"
            )
            .expect("write response headers");
        });
        let response = build_provider_http_client()
            .expect("client")
            .get(format!("http://{address}/"))
            .send()
            .await
            .expect("response");
        (response, server)
    }
}
