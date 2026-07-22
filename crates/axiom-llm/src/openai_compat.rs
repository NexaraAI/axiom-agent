use async_trait::async_trait;

use reqwest::RequestBuilder;
use serde::Deserialize;

use crate::{
    limits::{
        ensure_bytes, ensure_count, MAX_CHAT_HTTP_BODY_BYTES, MAX_MODEL_CATALOG_ENTRIES,
        MAX_MODEL_CATALOG_HTTP_BODY_BYTES, MAX_MODEL_DESCRIPTION_BYTES, MAX_MODEL_ID_BYTES,
        MAX_STREAM_WIRE_BYTES,
    },
    openai_format::{chat_request_body, parse_chat_response, read_secret_env, summarize_bytes},
    provider::{
        build_provider_http_client, ensure_response_content_length, read_response_body_limited,
        retry_transient, validate_provider_endpoint, SecretValue,
    },
    ChatRequest, ChatResponse, ChatStream, LlmError, LlmProvider, ModelInfo, Result,
};

#[derive(Debug, Clone)]
pub struct OpenAiCompatibleProvider {
    name: String,
    base_url: String,
    api_key_env: Option<String>,
    api_key: Option<SecretValue>,
    models_url: Option<String>,
    client: std::result::Result<reqwest::Client, String>,
}

impl OpenAiCompatibleProvider {
    pub fn new(
        name: impl Into<String>,
        base_url: impl Into<String>,
        api_key_env: Option<String>,
    ) -> Self {
        Self {
            name: name.into(),
            base_url: base_url.into(),
            api_key_env,
            api_key: None,
            models_url: None,
            client: build_provider_http_client(),
        }
    }

    pub fn chat_endpoint(&self) -> String {
        format!("{}/chat/completions", self.base_url.trim_end_matches('/'))
    }

    pub fn with_models_url(mut self, models_url: Option<String>) -> Self {
        self.models_url = models_url;
        self
    }

    /// Supply a credential value directly. This takes precedence over reading
    /// `api_key_env` and keeps keyring-resolved values out of the process
    /// environment inherited by child tools.
    pub fn with_api_key(mut self, api_key: impl Into<String>) -> Self {
        self.api_key = Some(SecretValue::new(api_key));
        self
    }

    pub fn models_endpoint(&self) -> String {
        self.models_url
            .clone()
            .unwrap_or_else(|| format!("{}/models", self.base_url.trim_end_matches('/')))
    }

    pub fn validate_request(&self, request: &ChatRequest) -> Result<()> {
        if self.base_url.trim().is_empty() {
            return Err(LlmError::MissingField("base_url"));
        }
        validate_provider_endpoint("base_url", &self.base_url, true)?;
        self.validate_auth_config()?;
        if request.model.trim().is_empty() {
            return Err(LlmError::MissingField("model"));
        }
        Ok(())
    }

    pub fn build_chat_request(&self, request: &ChatRequest) -> Result<RequestBuilder> {
        self.validate_request(request)?;
        let body = chat_request_body(request);
        let builder = self.client()?.post(self.chat_endpoint()).json(&body);
        self.authenticate(builder)
    }

    pub fn build_models_request(&self) -> Result<RequestBuilder> {
        if self.base_url.trim().is_empty() {
            return Err(LlmError::MissingField("base_url"));
        }
        self.validate_auth_config()?;
        validate_provider_endpoint("base_url", &self.base_url, true)?;
        let models_endpoint = self.models_endpoint();
        validate_provider_endpoint("models_url", &models_endpoint, true)?;
        let builder = self.client()?.get(models_endpoint);
        self.authenticate(builder)
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

    fn authenticate(&self, builder: RequestBuilder) -> Result<RequestBuilder> {
        if let Some(api_key) = self.api_key.as_ref() {
            if api_key.expose().trim().is_empty() {
                return Err(LlmError::EmptyApiKeyEnv {
                    env: self
                        .api_key_env
                        .clone()
                        .unwrap_or_else(|| "direct provider credential".to_string()),
                });
            }
            return Ok(builder.bearer_auth(api_key.expose()));
        }
        match self.api_key_env.as_deref() {
            Some(api_key_env) => Ok(builder.bearer_auth(read_secret_env(api_key_env)?)),
            None => Ok(builder),
        }
    }

    fn validate_auth_config(&self) -> Result<()> {
        if let Some(api_key_env) = self.api_key_env.as_deref() {
            if api_key_env.trim().is_empty() {
                return Err(LlmError::MissingField("api_key_env"));
            }
            crate::validate_credential_env_name(api_key_env)?;
        }
        Ok(())
    }
}

#[async_trait]
impl LlmProvider for OpenAiCompatibleProvider {
    async fn chat(&self, request: ChatRequest) -> Result<ChatResponse> {
        retry_transient(|| self.chat_once(&request)).await
    }

    async fn stream_chat(&self, mut request: ChatRequest) -> Result<ChatStream> {
        request.stream = true;
        retry_transient(|| self.stream_once(&request)).await
    }

    async fn models(&self) -> Result<Vec<ModelInfo>> {
        retry_transient(|| self.models_once()).await
    }

    fn provider_name(&self) -> &str {
        &self.name
    }
}

impl OpenAiCompatibleProvider {
    async fn models_once(&self) -> Result<Vec<ModelInfo>> {
        let response =
            self.build_models_request()?
                .send()
                .await
                .map_err(|error| LlmError::Http {
                    provider: self.name.clone(),
                    message: error.to_string(),
                })?;
        let (status, body) = read_response_body_limited(
            response,
            &self.name,
            "model catalog body",
            MAX_MODEL_CATALOG_HTTP_BODY_BYTES,
        )
        .await?;
        if !status.is_success() {
            return Err(LlmError::HttpStatus {
                provider: self.name.clone(),
                status: status.as_u16(),
                body_summary: summarize_bytes(&body),
            });
        }
        parse_models_response(&self.name, &body)
    }

    async fn chat_once(&self, request: &ChatRequest) -> Result<ChatResponse> {
        let response = self
            .build_chat_request(request)?
            .send()
            .await
            .map_err(|error| LlmError::Http {
                provider: self.name.clone(),
                message: error.to_string(),
            })?;

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

    async fn stream_once(&self, request: &ChatRequest) -> Result<ChatStream> {
        let response = self
            .build_chat_request(request)?
            .send()
            .await
            .map_err(|error| LlmError::Http {
                provider: self.name.clone(),
                message: error.to_string(),
            })?;
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

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum ModelsResponse {
    Envelope { data: Vec<ModelRecord> },
    Direct(Vec<ModelRecord>),
}

#[derive(Debug, Deserialize)]
struct ModelRecord {
    id: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    description: Option<String>,
}

fn parse_models_response(provider: &str, body: &[u8]) -> Result<Vec<ModelInfo>> {
    let parsed: ModelsResponse =
        serde_json::from_slice(body).map_err(|_| LlmError::ResponseParse {
            provider: provider.to_string(),
            body_summary: summarize_bytes(body),
        })?;
    let records = match parsed {
        ModelsResponse::Envelope { data } => data,
        ModelsResponse::Direct(records) => records,
    };
    ensure_count(
        provider,
        "model catalog entry count",
        records.len(),
        MAX_MODEL_CATALOG_ENTRIES,
    )?;
    let mut models = Vec::with_capacity(records.len());
    for record in records {
        ensure_bytes(provider, "model id", record.id.len(), MAX_MODEL_ID_BYTES)?;
        let description = record.description.or(record.name);
        if let Some(description) = description.as_deref() {
            ensure_bytes(
                provider,
                "model description",
                description.len(),
                MAX_MODEL_DESCRIPTION_BYTES,
            )?;
        }
        if !record.id.trim().is_empty() {
            models.push(ModelInfo {
                id: record.id,
                provider: provider.to_string(),
                description,
            });
        }
    }
    models.sort_by(|left, right| left.id.cmp(&right.id));
    models.dedup_by(|left, right| left.id == right.id);
    Ok(models)
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};

    use super::*;
    use crate::ChatMessage;

    #[test]
    fn builds_openai_compatible_endpoint() {
        let provider = OpenAiCompatibleProvider::new(
            "local",
            "http://localhost:8000/v1/",
            Some("LOCAL_KEY".to_string()),
        );

        assert_eq!(
            provider.chat_endpoint(),
            "http://localhost:8000/v1/chat/completions"
        );
    }

    #[test]
    fn request_body_uses_configured_model_and_messages() {
        let provider = OpenAiCompatibleProvider::new(
            "local",
            "http://localhost:8000/v1",
            Some("LOCAL_KEY".to_string()),
        );
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
        let provider = OpenAiCompatibleProvider::new(
            "local",
            "http://localhost:8000/v1",
            Some(env.to_string()),
        );

        let error = provider
            .build_chat_request(&sample_request())
            .expect_err("missing env should fail");

        assert!(matches!(error, LlmError::MissingApiKeyEnv { .. }));
        assert!(error.to_string().contains(env));
    }

    #[test]
    fn direct_api_key_builds_auth_without_exporting_or_debugging_the_secret() {
        let env = "AXIOM_TEST_DIRECT_OPENAI_KEY_725D99E4";
        std::env::remove_var(env);
        let provider = OpenAiCompatibleProvider::new(
            "local",
            "http://localhost:8000/v1",
            Some(env.to_string()),
        )
        .with_api_key("direct-secret-value");

        let request = provider
            .build_chat_request(&sample_request())
            .expect("direct credential should build")
            .build()
            .expect("request should finalize");

        assert_eq!(
            request.headers()[reqwest::header::AUTHORIZATION],
            "Bearer direct-secret-value"
        );
        assert!(std::env::var_os(env).is_none());
        let debug = format!("{provider:?}");
        assert!(!debug.contains("direct-secret-value"));
        assert!(debug.contains("[REDACTED]"));
    }

    #[test]
    fn local_provider_can_build_request_without_authorization() {
        let provider = OpenAiCompatibleProvider::new("ollama", "http://localhost:11434/v1", None);

        let request = provider
            .build_chat_request(&sample_request())
            .expect("local request should build")
            .build()
            .expect("request should finalize");

        assert!(!request
            .headers()
            .contains_key(reqwest::header::AUTHORIZATION));
    }

    #[test]
    fn model_catalog_parser_supports_openai_and_github_shapes() {
        let openai = parse_models_response(
            "openai",
            br#"{"data":[{"id":"model-b"},{"id":"model-a","name":"Model A"}]}"#,
        )
        .expect("OpenAI model list");
        let github = parse_models_response(
            "github-models",
            br#"[{"id":"openai/gpt-4.1","name":"GPT-4.1"}]"#,
        )
        .expect("GitHub model list");

        assert_eq!(openai[0].id, "model-a");
        assert_eq!(openai[0].description.as_deref(), Some("Model A"));
        assert_eq!(github[0].id, "openai/gpt-4.1");
    }

    #[test]
    fn custom_models_url_overrides_default_endpoint() {
        let provider = OpenAiCompatibleProvider::new(
            "github-models",
            "https://models.github.ai/inference",
            Some("GITHUB_TOKEN".to_string()),
        )
        .with_models_url(Some("https://models.github.ai/catalog/models".to_string()));

        assert_eq!(
            provider.models_endpoint(),
            "https://models.github.ai/catalog/models"
        );
    }

    #[test]
    fn rejects_remote_cleartext_and_credential_bearing_provider_urls() {
        let request = sample_request();
        for endpoint in [
            "http://api.example.com/v1",
            "https://user:secret@api.example.com/v1",
        ] {
            let provider = OpenAiCompatibleProvider::new("custom", endpoint, None);
            assert!(matches!(
                provider.validate_request(&request),
                Err(LlmError::UnsafeEndpoint { .. })
            ));
        }
    }

    #[test]
    fn rejects_remote_cleartext_model_catalog_url() {
        let provider = OpenAiCompatibleProvider::new("custom", "https://api.example.com/v1", None)
            .with_models_url(Some("http://api.example.com/models".to_string()));
        assert!(matches!(
            provider.build_models_request(),
            Err(LlmError::UnsafeEndpoint { .. })
        ));
    }

    #[tokio::test]
    async fn fetches_model_catalog_without_a_completion_request() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind test server");
        let address = listener.local_addr().expect("server address");
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept request");
            let mut request = [0_u8; 2_048];
            let bytes = stream.read(&mut request).expect("read request");
            let request = String::from_utf8_lossy(&request[..bytes]);
            assert!(request.starts_with("GET /v1/models "));
            assert!(!request.contains("chat/completions"));
            let body = r#"{"data":[{"id":"local-model","name":"Local Model"}]}"#;
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .expect("write response");
        });
        let provider = OpenAiCompatibleProvider::new("local", format!("http://{address}/v1"), None);

        let models = provider.models().await.expect("fetch models");
        server.join().expect("server thread");

        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, "local-model");
    }

    #[test]
    fn malformed_and_empty_catalogs_are_bounded_and_explicit() {
        let malformed = parse_models_response("test", b"not-json").expect_err("malformed");
        assert!(matches!(malformed, LlmError::ResponseParse { .. }));
        let empty = parse_models_response("test", br#"{"data":[]}"#).expect("empty catalog");
        assert!(empty.is_empty());
    }

    #[tokio::test]
    async fn model_catalog_surfaces_auth_rate_limit_and_size_failures() {
        for (status, requests) in [("401 Unauthorized", 1), ("429 Too Many Requests", 3)] {
            let (address, server) = serve_catalog(status, "denied".to_string(), requests);
            let provider =
                OpenAiCompatibleProvider::new("test", format!("http://{address}/v1"), None);
            let error = provider.models().await.expect_err("status error");
            server.join().expect("server");
            match (status, error) {
                ("401 Unauthorized", LlmError::HttpStatus { status: 401, .. }) => {}
                ("429 Too Many Requests", LlmError::Retried { attempts: 3, .. }) => {}
                (_, error) => panic!("unexpected catalog error: {error}"),
            }
        }

        let (address, server) = serve_declared_response(
            "GET /v1/models ",
            "200 OK",
            MAX_MODEL_CATALOG_HTTP_BODY_BYTES + 1,
        );
        let provider = OpenAiCompatibleProvider::new("test", format!("http://{address}/v1"), None);
        let error = provider.models().await.expect_err("oversized catalog");
        server.join().expect("server");
        assert!(matches!(
            error,
            LlmError::ResponseLimitExceeded {
                resource: "model catalog body",
                ..
            }
        ));
    }

    #[tokio::test]
    async fn chat_and_stream_success_and_error_bodies_are_bounded() {
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
            let (address, server) =
                serve_declared_response("POST /v1/chat/completions ", status, declared_length);
            let provider =
                OpenAiCompatibleProvider::new("test", format!("http://{address}/v1"), None);
            let error = if streaming {
                provider
                    .stream_chat(sample_request())
                    .await
                    .expect_err("oversized stream response")
            } else {
                provider
                    .chat(sample_request())
                    .await
                    .expect_err("oversized chat response")
            };
            server.join().expect("server");
            assert!(matches!(
                error,
                LlmError::ResponseLimitExceeded { resource, .. } if resource == expected_resource
            ));
        }
    }

    fn serve_catalog(
        status: &'static str,
        body: String,
        requests: usize,
    ) -> (std::net::SocketAddr, std::thread::JoinHandle<()>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind test server");
        let address = listener.local_addr().expect("server address");
        let server = std::thread::spawn(move || {
            for _ in 0..requests {
                let (mut stream, _) = listener.accept().expect("accept request");
                let mut request = [0_u8; 2_048];
                let bytes = stream.read(&mut request).expect("read request");
                let request = String::from_utf8_lossy(&request[..bytes]);
                assert!(request.starts_with("GET /v1/models "));
                write!(
                    stream,
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .expect("write response");
            }
        });
        (address, server)
    }

    fn serve_declared_response(
        expected_request_prefix: &'static str,
        status: &'static str,
        declared_length: usize,
    ) -> (std::net::SocketAddr, std::thread::JoinHandle<()>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind test server");
        let address = listener.local_addr().expect("server address");
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept request");
            let mut request = [0_u8; 8_192];
            let bytes = stream.read(&mut request).expect("read request");
            let request = String::from_utf8_lossy(&request[..bytes]);
            assert!(request.starts_with(expected_request_prefix), "{request}");
            write!(
                stream,
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {declared_length}\r\nConnection: close\r\n\r\n"
            )
            .expect("write response headers");
        });
        (address, server)
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
            tools: Vec::new(),
            tool_choice: None,
        }
    }
}
