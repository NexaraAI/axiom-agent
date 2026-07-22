use async_trait::async_trait;

use crate::{
    ChatMessage, ChatRequest, ChatResponse, ChatStream, LlmProvider, ModelInfo, Result, TokenUsage,
};

#[derive(Debug, Clone)]
pub struct MockProvider {
    name: String,
}

impl MockProvider {
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }
}

#[async_trait]
impl LlmProvider for MockProvider {
    async fn chat(&self, request: ChatRequest) -> Result<ChatResponse> {
        let content = mock_response(&request.messages);
        let prompt_tokens = request.messages.iter().fold(0_u32, |total, message| {
            total.saturating_add(approximate_tokens(&message.content))
        });
        let completion_tokens = approximate_tokens(&content);
        Ok(ChatResponse {
            content,
            usage: Some(TokenUsage {
                prompt_tokens,
                completion_tokens,
                total_tokens: prompt_tokens.saturating_add(completion_tokens),
            }),
            model: request.model,
            provider: self.name.clone(),
            raw: None,
            tool_calls: Vec::new(),
        })
    }

    async fn stream_chat(&self, mut request: ChatRequest) -> Result<ChatStream> {
        request.stream = false;
        Ok(ChatStream::from_response(self.chat(request).await?))
    }

    async fn models(&self) -> Result<Vec<ModelInfo>> {
        Ok(vec![ModelInfo {
            id: "mock-model".to_string(),
            provider: self.name.clone(),
            description: Some("Offline deterministic model; no API key or network.".to_string()),
        }])
    }

    fn provider_name(&self) -> &str {
        &self.name
    }
}

fn approximate_tokens(text: &str) -> u32 {
    let chars = text.chars().count();
    u32::try_from(chars.div_ceil(4).max(1)).unwrap_or(u32::MAX)
}

fn mock_response(messages: &[ChatMessage]) -> String {
    let transcript = messages
        .iter()
        .map(|message| message.content.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let lower = transcript.to_ascii_lowercase();

    if lower.contains("axiom tool result") {
        return "Result verified and summarized.".to_string();
    }

    if lower.contains("propose file changes")
        || lower.contains("```axiom-patch")
        || lower.contains("patch format")
    {
        return r##"```axiom-patch
{
  "summary": "Create demo note",
  "test_command": null,
  "changes": [
    {
      "path": "AXIOM_DEMO.md",
      "action": "create_or_update",
      "content": "# Axiom Demo\n\nCreated by Axiom offline mode.\n"
    }
  ]
}
```"##
            .to_string();
    }

    if lower.contains("create a concise implementation plan")
        || lower.contains("do not produce a patch yet")
    {
        return "1. Inspect the workspace.\n2. Make the smallest safe change.\n3. Run an appropriate verification step.".to_string();
    }

    let last_user = messages
        .iter()
        .rev()
        .find(|message| message.role == "user")
        .map(|message| message.content.trim())
        .unwrap_or("");

    if asks_to_read_readme(last_user) {
        return r#"```axiom-tool
{
  "skill_id": "file.read",
  "arguments": { "path": "README.md" }
}
```"#
            .to_string();
    }

    format!("Axiom (offline): {last_user}")
}

fn asks_to_read_readme(input: &str) -> bool {
    let lower = input.to_ascii_lowercase();
    lower.contains("read") && lower.contains("readme.md")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn mock_provider_normal_response() {
        let provider = MockProvider::new("mock");
        let response = provider
            .chat(ChatRequest {
                model: "mock-model".to_string(),
                messages: vec![ChatMessage {
                    role: "user".to_string(),
                    content: "hello".to_string(),
                }],
                temperature: None,
                max_tokens: None,
                stream: false,
                metadata: None,
                provider_options: None,
                tools: Vec::new(),
                tool_choice: None,
            })
            .await
            .expect("mock response");

        assert_eq!(response.content, "Axiom (offline): hello");
        assert!(response.usage.expect("synthetic usage").total_tokens > 0);
    }

    #[test]
    fn mock_provider_tool_request_response() {
        let response = mock_response(&[ChatMessage {
            role: "user".to_string(),
            content: "read README.md".to_string(),
        }]);

        assert!(response.contains("```axiom-tool"));
        assert!(response.contains("\"file.read\""));
    }

    #[test]
    fn mock_provider_coder_plan_response() {
        let response = mock_response(&[ChatMessage {
            role: "user".to_string(),
            content: "Create a concise implementation plan. Do not produce a patch yet."
                .to_string(),
        }]);

        assert!(response.contains("Inspect the workspace"));
    }
}
