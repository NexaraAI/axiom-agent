use serde::Deserialize;
use serde_json::{json, Map, Value};

use crate::{ChatRequest, ChatResponse, LlmError, Result, TokenUsage};

pub fn chat_request_body(request: &ChatRequest) -> Value {
    let mut body = Map::new();
    body.insert("model".to_string(), json!(request.model));
    body.insert("messages".to_string(), json!(request.messages));
    body.insert("stream".to_string(), json!(request.stream));

    if let Some(temperature) = request.temperature {
        body.insert("temperature".to_string(), json!(temperature));
    }

    if let Some(max_tokens) = request.max_tokens {
        body.insert("max_tokens".to_string(), json!(max_tokens));
    }

    Value::Object(body)
}

pub fn parse_chat_response(
    provider: &str,
    fallback_model: &str,
    body: &str,
) -> Result<ChatResponse> {
    let parsed: OpenAiChatResponse =
        serde_json::from_str(body).map_err(|_| LlmError::ResponseParse {
            provider: provider.to_string(),
            body_summary: summarize_body(body),
        })?;

    let content = parsed
        .choices
        .first()
        .and_then(|choice| choice.message.as_ref())
        .and_then(|message| message.content.as_deref())
        .map(str::trim)
        .filter(|content| !content.is_empty())
        .ok_or_else(|| LlmError::ResponseParse {
            provider: provider.to_string(),
            body_summary: summarize_body(body),
        })?;

    let raw: Value = serde_json::from_str(body).map_err(|_| LlmError::ResponseParse {
        provider: provider.to_string(),
        body_summary: summarize_body(body),
    })?;

    Ok(ChatResponse {
        content: content.to_string(),
        usage: parsed.usage,
        model: parsed.model.unwrap_or_else(|| fallback_model.to_string()),
        provider: provider.to_string(),
        raw: Some(raw),
    })
}

pub fn summarize_body(body: &str) -> String {
    let compact = body.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.chars().count() <= 500 {
        compact
    } else {
        let summary = compact.chars().take(500).collect::<String>();
        format!("{summary}...")
    }
}

pub fn read_secret_env(env: &str) -> Result<String> {
    let value = std::env::var(env).map_err(|_| LlmError::MissingApiKeyEnv {
        env: env.to_string(),
    })?;

    if value.trim().is_empty() {
        return Err(LlmError::EmptyApiKeyEnv {
            env: env.to_string(),
        });
    }

    Ok(value)
}

#[derive(Debug, Deserialize)]
struct OpenAiChatResponse {
    model: Option<String>,
    choices: Vec<OpenAiChoice>,
    usage: Option<TokenUsage>,
}

#[derive(Debug, Deserialize)]
struct OpenAiChoice {
    message: Option<OpenAiMessage>,
}

#[derive(Debug, Deserialize)]
struct OpenAiMessage {
    content: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ChatMessage;

    #[test]
    fn request_body_contains_chat_fields_without_null_options() {
        let request = ChatRequest {
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
        };

        let body = chat_request_body(&request);

        assert_eq!(body["model"], "test-model");
        assert_eq!(body["messages"][0]["role"], "user");
        assert_eq!(body["messages"][0]["content"], "hello");
        let temperature = body["temperature"]
            .as_f64()
            .expect("temperature should be numeric");
        assert!((temperature - 0.2).abs() < 0.000_001);
        assert_eq!(body["max_tokens"], 64);
        assert_eq!(body["stream"], false);
    }

    #[test]
    fn request_body_omits_absent_optional_fields() {
        let request = ChatRequest {
            model: "test-model".to_string(),
            messages: Vec::new(),
            temperature: None,
            max_tokens: None,
            stream: false,
            metadata: None,
            provider_options: None,
        };

        let body = chat_request_body(&request);

        assert!(body.get("temperature").is_none());
        assert!(body.get("max_tokens").is_none());
    }

    #[test]
    fn parses_openai_compatible_response() {
        let response = parse_chat_response(
            "local",
            "fallback-model",
            r#"{
                "model": "actual-model",
                "choices": [
                    {
                        "message": {
                            "role": "assistant",
                            "content": "Hello from the model."
                        }
                    }
                ],
                "usage": {
                    "prompt_tokens": 4,
                    "completion_tokens": 5,
                    "total_tokens": 9
                }
            }"#,
        )
        .expect("parse response");

        assert_eq!(response.provider, "local");
        assert_eq!(response.model, "actual-model");
        assert_eq!(response.content, "Hello from the model.");
        assert_eq!(response.usage.expect("usage").total_tokens, 9);
    }

    #[test]
    fn parses_cloudflare_openai_compatible_response() {
        let response = parse_chat_response(
            "cloudflare",
            "openai/gpt-4.1-mini",
            r#"{
                "id": "chatcmpl-test",
                "object": "chat.completion",
                "model": "openai/gpt-4.1-mini",
                "choices": [
                    {
                        "index": 0,
                        "message": {
                            "role": "assistant",
                            "content": "Cloudflare gateway response."
                        },
                        "finish_reason": "stop"
                    }
                ]
            }"#,
        )
        .expect("parse cloudflare response");

        assert_eq!(response.provider, "cloudflare");
        assert_eq!(response.model, "openai/gpt-4.1-mini");
        assert_eq!(response.content, "Cloudflare gateway response.");
    }

    #[test]
    fn parse_error_summarizes_invalid_response() {
        let error = parse_chat_response("local", "model", r#"{"choices":[]}"#)
            .expect_err("empty choices should fail");

        assert!(matches!(error, LlmError::ResponseParse { .. }));
        assert!(!error.to_string().contains("Bearer"));
    }

    #[test]
    fn missing_env_var_returns_helpful_error() {
        let env = "AXIOM_TEST_ENV_THAT_SHOULD_NOT_EXIST_7B3F2C0E";
        std::env::remove_var(env);

        let error = read_secret_env(env).expect_err("missing env should fail");

        assert!(matches!(error, LlmError::MissingApiKeyEnv { .. }));
        assert!(error.to_string().contains(env));
    }
}
