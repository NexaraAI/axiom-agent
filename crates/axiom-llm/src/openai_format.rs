use serde::Deserialize;
use serde_json::{json, Map, Value};

use crate::{
    limits::{
        ensure_additional_bytes, ensure_bytes, ensure_count, MAX_ASSISTANT_CONTENT_BYTES,
        MAX_CHAT_CHOICES, MAX_MODEL_NAME_BYTES, MAX_TOOL_ARGUMENT_BYTES, MAX_TOOL_CALLS,
        MAX_TOOL_CALL_ID_BYTES, MAX_TOOL_NAME_BYTES, MAX_TOTAL_TOOL_ARGUMENT_BYTES,
    },
    ChatRequest, ChatResponse, LlmError, Result, TokenUsage,
};

pub fn chat_request_body(request: &ChatRequest) -> Value {
    let mut body = Map::new();
    body.insert("model".to_string(), json!(request.model));
    body.insert("messages".to_string(), json!(request.messages));
    body.insert("stream".to_string(), json!(request.stream));
    if request.stream {
        body.insert(
            "stream_options".to_string(),
            json!({ "include_usage": true }),
        );
    }

    if let Some(temperature) = request.temperature {
        body.insert("temperature".to_string(), json!(temperature));
    }

    if let Some(max_tokens) = request.max_tokens {
        body.insert("max_tokens".to_string(), json!(max_tokens));
    }

    if !request.tools.is_empty() {
        body.insert(
            "tools".to_string(),
            json!(request
                .tools
                .iter()
                .map(|tool| json!({
                    "type": "function",
                    "function": {
                        "name": tool.name,
                        "description": tool.description,
                        "parameters": tool.parameters,
                    }
                }))
                .collect::<Vec<_>>()),
        );
        body.insert(
            "tool_choice".to_string(),
            json!(request.tool_choice.as_deref().unwrap_or("auto")),
        );
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
    ensure_count(
        provider,
        "chat choice count",
        parsed.choices.len(),
        MAX_CHAT_CHOICES,
    )?;
    if let Some(model) = parsed.model.as_deref() {
        ensure_bytes(
            provider,
            "response model name",
            model.len(),
            MAX_MODEL_NAME_BYTES,
        )?;
    }

    let message = parsed
        .choices
        .first()
        .and_then(|choice| choice.message.as_ref())
        .ok_or_else(|| LlmError::ResponseParse {
            provider: provider.to_string(),
            body_summary: summarize_body(body),
        })?;
    ensure_count(
        provider,
        "tool-call count",
        message.tool_calls.len(),
        MAX_TOOL_CALLS,
    )?;
    let mut tool_calls = Vec::with_capacity(message.tool_calls.len());
    let mut total_argument_bytes = 0_usize;
    for tool_call in &message.tool_calls {
        if let Some(id) = tool_call.id.as_deref() {
            ensure_bytes(provider, "tool-call id", id.len(), MAX_TOOL_CALL_ID_BYTES)?;
        }
        ensure_bytes(
            provider,
            "tool-call name",
            tool_call.function.name.len(),
            MAX_TOOL_NAME_BYTES,
        )?;
        ensure_bytes(
            provider,
            "tool-call arguments",
            tool_call.function.arguments.len(),
            MAX_TOOL_ARGUMENT_BYTES,
        )?;
        ensure_additional_bytes(
            provider,
            "total tool-call arguments",
            total_argument_bytes,
            tool_call.function.arguments.len(),
            MAX_TOTAL_TOOL_ARGUMENT_BYTES,
        )?;
        total_argument_bytes += tool_call.function.arguments.len();
        let arguments = serde_json::from_str(&tool_call.function.arguments).map_err(|_| {
            LlmError::ResponseParse {
                provider: provider.to_string(),
                body_summary: summarize_body(body),
            }
        })?;
        tool_calls.push(crate::ChatToolCall {
            id: tool_call.id.clone(),
            name: tool_call.function.name.clone(),
            arguments,
        });
    }
    let raw_content = message.content.as_deref().unwrap_or_default();
    ensure_bytes(
        provider,
        "assistant content",
        raw_content.len(),
        MAX_ASSISTANT_CONTENT_BYTES,
    )?;
    let content = raw_content.trim();
    if content.is_empty() && tool_calls.is_empty() {
        return Err(LlmError::ResponseParse {
            provider: provider.to_string(),
            body_summary: summarize_body(body),
        });
    }

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
        tool_calls,
    })
}

pub fn summarize_body(body: &str) -> String {
    const MAX_SUMMARY_CHARS: usize = 500;
    let mut summary = String::with_capacity(MAX_SUMMARY_CHARS + 3);
    let mut characters = 0_usize;
    let mut truncated = false;

    'words: for word in body.split_whitespace() {
        if !summary.is_empty() {
            if characters == MAX_SUMMARY_CHARS {
                truncated = true;
                break;
            }
            summary.push(' ');
            characters += 1;
        }
        for character in word.chars() {
            if characters == MAX_SUMMARY_CHARS {
                truncated = true;
                break 'words;
            }
            summary.push(character);
            characters += 1;
        }
    }
    if truncated {
        summary.push_str("...");
    }
    summary
}

pub fn summarize_bytes(body: &[u8]) -> String {
    // Error reporting must remain bounded even when the provider sends invalid UTF-8. 4 KiB is
    // enough to produce the 500-character public summary without decoding the whole response.
    const MAX_SUMMARY_INPUT_BYTES: usize = 4 * 1024;
    let prefix = &body[..body.len().min(MAX_SUMMARY_INPUT_BYTES)];
    let mut summary = summarize_body(&String::from_utf8_lossy(prefix));
    if body.len() > prefix.len() && !summary.ends_with("...") {
        summary.push_str("...");
    }
    summary
}

pub fn read_secret_env(env: &str) -> Result<String> {
    crate::validate_credential_env_name(env)?;
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
    #[serde(default)]
    tool_calls: Vec<OpenAiToolCall>,
}

#[derive(Debug, Deserialize)]
struct OpenAiToolCall {
    id: Option<String>,
    function: OpenAiFunctionCall,
}

#[derive(Debug, Deserialize)]
struct OpenAiFunctionCall {
    name: String,
    arguments: String,
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
            tools: Vec::new(),
            tool_choice: None,
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
            tools: Vec::new(),
            tool_choice: None,
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
    fn parses_native_tool_calls_without_text_content() {
        let response = parse_chat_response(
            "local",
            "model",
            r#"{
                "choices": [{
                    "message": {
                        "content": null,
                        "tool_calls": [{
                            "id": "call_1",
                            "function": {
                                "name": "axiom_file_read",
                                "arguments": "{\"path\":\"README.md\"}"
                            }
                        }]
                    }
                }]
            }"#,
        )
        .expect("tool call response parses");

        assert!(response.content.is_empty());
        assert_eq!(response.tool_calls[0].name, "axiom_file_read");
        assert_eq!(response.tool_calls[0].arguments["path"], "README.md");
    }

    #[test]
    fn request_body_encodes_native_function_tools() {
        let mut request = ChatRequest {
            model: "test-model".to_string(),
            messages: Vec::new(),
            temperature: None,
            max_tokens: None,
            stream: false,
            metadata: None,
            provider_options: None,
            tools: vec![crate::ChatToolDefinition {
                name: "axiom_file_read".to_string(),
                description: "Read a workspace file".to_string(),
                parameters: json!({"type":"object"}),
            }],
            tool_choice: None,
        };

        let body = chat_request_body(&request);

        assert_eq!(body["tools"][0]["type"], "function");
        assert_eq!(body["tools"][0]["function"]["name"], "axiom_file_read");
        assert_eq!(body["tool_choice"], "auto");
        request.tools.clear();
        assert!(chat_request_body(&request).get("tools").is_none());
    }

    #[test]
    fn parse_error_summarizes_invalid_response() {
        let error = parse_chat_response("local", "model", r#"{"choices":[]}"#)
            .expect_err("empty choices should fail");

        assert!(matches!(error, LlmError::ResponseParse { .. }));
        assert!(!error.to_string().contains("Bearer"));
    }

    #[test]
    fn non_stream_content_and_tool_call_fields_are_bounded() {
        let oversized_content = json!({
            "choices": [{"message": {"content": "x".repeat(MAX_ASSISTANT_CONTENT_BYTES + 1)}}]
        })
        .to_string();
        let error = parse_chat_response("test", "model", &oversized_content)
            .expect_err("oversized content");
        assert!(matches!(
            error,
            LlmError::ResponseLimitExceeded {
                resource: "assistant content",
                ..
            }
        ));

        let calls = (0..=MAX_TOOL_CALLS)
            .map(|index| {
                json!({
                    "id": format!("call_{index}"),
                    "function": {"name": "tool", "arguments": "{}"}
                })
            })
            .collect::<Vec<_>>();
        let too_many_calls =
            json!({"choices":[{"message":{"content":null,"tool_calls":calls}}]}).to_string();
        let error =
            parse_chat_response("test", "model", &too_many_calls).expect_err("too many calls");
        assert!(matches!(
            error,
            LlmError::ResponseLimitExceeded {
                resource: "tool-call count",
                ..
            }
        ));

        let oversized_name = json!({
            "choices": [{"message": {"content": null, "tool_calls": [{
                "function": {"name": "n".repeat(MAX_TOOL_NAME_BYTES + 1), "arguments": "{}"}
            }]}}]
        })
        .to_string();
        let error =
            parse_chat_response("test", "model", &oversized_name).expect_err("oversized name");
        assert!(matches!(
            error,
            LlmError::ResponseLimitExceeded {
                resource: "tool-call name",
                ..
            }
        ));
    }

    #[test]
    fn body_summaries_do_not_scale_with_the_input() {
        let summary = summarize_body(&"word ".repeat(100_000));
        assert!(summary.len() <= 503);
        assert!(summary.ends_with("..."));

        let summary = summarize_bytes(&vec![0xff; 100_000]);
        assert!(summary.chars().count() <= 503);
        assert!(summary.ends_with("..."));
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
