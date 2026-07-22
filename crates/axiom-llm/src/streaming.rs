use std::collections::{BTreeMap, VecDeque};

use serde::{Deserialize, Serialize};

use crate::{
    limits::{
        ensure_additional_bytes, ensure_additional_count, ensure_bytes, ensure_count,
        MAX_ASSISTANT_CONTENT_BYTES, MAX_MODEL_NAME_BYTES, MAX_SSE_BUFFER_BYTES,
        MAX_SSE_EVENT_BYTES, MAX_STREAM_CHOICES_PER_EVENT, MAX_STREAM_EVENTS,
        MAX_STREAM_WIRE_BYTES, MAX_TOOL_ARGUMENT_BYTES, MAX_TOOL_CALLS, MAX_TOOL_CALL_DELTAS,
        MAX_TOOL_CALL_DELTAS_PER_EVENT, MAX_TOOL_CALL_ID_BYTES, MAX_TOOL_NAME_BYTES,
        MAX_TOTAL_TOOL_ARGUMENT_BYTES,
    },
    openai_format::{summarize_body, summarize_bytes},
    ChatResponse, ChatToolCall, LlmError, Result, TokenUsage,
};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatToolCallDelta {
    pub index: usize,
    pub id: Option<String>,
    pub name_delta: String,
    pub arguments_delta: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatChunk {
    #[serde(default)]
    pub content_delta: String,
    #[serde(default)]
    pub tool_call_deltas: Vec<ChatToolCallDelta>,
    #[serde(default)]
    pub usage: Option<TokenUsage>,
    #[serde(default)]
    pub model: Option<String>,
    pub done: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChatStreamUpdate {
    /// Assistant text that is safe to render. Axiom control blocks are
    /// removed even when their delimiters span provider chunks.
    pub visible_delta: String,
    pub content_chars_received: usize,
    pub tool_call_deltas_received: usize,
    pub done: bool,
}

pub struct ChatStream {
    source: ChatStreamSource,
}

enum ChatStreamSource {
    Buffered(VecDeque<ChatChunk>),
    Http(HttpChatStream),
}

struct HttpChatStream {
    response: reqwest::Response,
    buffer: Vec<u8>,
    provider: String,
    terminal_event_seen: bool,
    wire_bytes_received: usize,
    events_received: usize,
}

impl std::fmt::Debug for ChatStream {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ChatStream")
            .field(
                "source",
                &match &self.source {
                    ChatStreamSource::Buffered(_) => "buffered",
                    ChatStreamSource::Http(_) => "http",
                },
            )
            .finish()
    }
}

impl ChatStream {
    pub fn empty() -> Self {
        Self::from_chunks(Vec::new())
    }

    pub fn from_chunks(chunks: Vec<ChatChunk>) -> Self {
        Self {
            source: ChatStreamSource::Buffered(chunks.into()),
        }
    }

    pub fn from_response(response: ChatResponse) -> Self {
        let tool_call_deltas = response
            .tool_calls
            .into_iter()
            .enumerate()
            .map(|(index, tool_call)| ChatToolCallDelta {
                index,
                id: tool_call.id,
                name_delta: tool_call.name,
                arguments_delta: tool_call.arguments.to_string(),
            })
            .collect();
        Self::from_chunks(vec![ChatChunk {
            content_delta: response.content,
            tool_call_deltas,
            usage: response.usage,
            model: Some(response.model),
            done: true,
        }])
    }

    pub(crate) fn from_http(response: reqwest::Response, provider: impl Into<String>) -> Self {
        Self {
            source: ChatStreamSource::Http(HttpChatStream {
                response,
                buffer: Vec::new(),
                provider: provider.into(),
                terminal_event_seen: false,
                wire_bytes_received: 0,
                events_received: 0,
            }),
        }
    }

    pub async fn next_chunk(&mut self) -> Result<Option<ChatChunk>> {
        match &mut self.source {
            ChatStreamSource::Buffered(chunks) => Ok(chunks.pop_front()),
            ChatStreamSource::Http(stream) => stream.next_chunk().await,
        }
    }

    pub async fn collect_response(
        self,
        provider: &str,
        fallback_model: &str,
    ) -> Result<ChatResponse> {
        self.collect_response_with_observer(provider, fallback_model, |_| {})
            .await
    }

    pub async fn collect_response_with_observer(
        mut self,
        provider: &str,
        fallback_model: &str,
        mut observer: impl FnMut(ChatStreamUpdate),
    ) -> Result<ChatResponse> {
        let mut content = String::new();
        let mut tool_calls = BTreeMap::<usize, PartialToolCall>::new();
        let mut usage = None;
        let mut model = None;
        let mut projector = ControlBlockProjector::default();
        let mut content_chars_received = 0_usize;
        let mut tool_call_deltas_received = 0_usize;
        let mut chunks_received = 0_usize;
        let mut total_argument_bytes = 0_usize;

        while let Some(chunk) = self.next_chunk().await? {
            ensure_additional_count(
                provider,
                "stream chunk count",
                chunks_received,
                1,
                MAX_STREAM_EVENTS,
            )?;
            chunks_received += 1;
            ensure_additional_bytes(
                provider,
                "assistant content",
                content.len(),
                chunk.content_delta.len(),
                MAX_ASSISTANT_CONTENT_BYTES,
            )?;
            ensure_additional_count(
                provider,
                "tool-call delta count",
                tool_call_deltas_received,
                chunk.tool_call_deltas.len(),
                MAX_TOOL_CALL_DELTAS,
            )?;
            if let Some(chunk_model) = chunk.model.as_deref() {
                ensure_bytes(
                    provider,
                    "response model name",
                    chunk_model.len(),
                    MAX_MODEL_NAME_BYTES,
                )?;
            }

            content_chars_received += chunk.content_delta.chars().count();
            tool_call_deltas_received += chunk.tool_call_deltas.len();
            let had_tool_call_deltas = !chunk.tool_call_deltas.is_empty();
            for delta in chunk.tool_call_deltas {
                if !tool_calls.contains_key(&delta.index) {
                    ensure_additional_count(
                        provider,
                        "tool-call count",
                        tool_calls.len(),
                        1,
                        MAX_TOOL_CALLS,
                    )?;
                }
                if let Some(id) = delta.id.as_deref() {
                    ensure_bytes(provider, "tool-call id", id.len(), MAX_TOOL_CALL_ID_BYTES)?;
                }
                let partial = tool_calls.entry(delta.index).or_default();
                ensure_additional_bytes(
                    provider,
                    "tool-call name",
                    partial.name.len(),
                    delta.name_delta.len(),
                    MAX_TOOL_NAME_BYTES,
                )?;
                ensure_additional_bytes(
                    provider,
                    "tool-call arguments",
                    partial.arguments.len(),
                    delta.arguments_delta.len(),
                    MAX_TOOL_ARGUMENT_BYTES,
                )?;
                ensure_additional_bytes(
                    provider,
                    "total tool-call arguments",
                    total_argument_bytes,
                    delta.arguments_delta.len(),
                    MAX_TOTAL_TOOL_ARGUMENT_BYTES,
                )?;
                if delta.id.is_some() {
                    partial.id = delta.id;
                }
                partial.name.push_str(&delta.name_delta);
                partial.arguments.push_str(&delta.arguments_delta);
                total_argument_bytes += delta.arguments_delta.len();
            }
            let visible_delta = projector.push(&chunk.content_delta);
            if !visible_delta.is_empty() || had_tool_call_deltas {
                observer(ChatStreamUpdate {
                    visible_delta,
                    content_chars_received,
                    tool_call_deltas_received,
                    done: false,
                });
            }
            content.push_str(&chunk.content_delta);
            if let Some(chunk_usage) = chunk.usage {
                usage = Some(chunk_usage);
            }
            if let Some(chunk_model) = chunk.model {
                model = Some(chunk_model);
            }
        }
        observer(ChatStreamUpdate {
            visible_delta: projector.finish(),
            content_chars_received,
            tool_call_deltas_received,
            done: true,
        });

        let tool_calls = tool_calls
            .into_values()
            .map(|partial| {
                let arguments = serde_json::from_str(&partial.arguments).map_err(|_| {
                    LlmError::ResponseParse {
                        provider: provider.to_string(),
                        body_summary: summarize_body(&partial.arguments),
                    }
                })?;
                Ok(ChatToolCall {
                    id: partial.id,
                    name: partial.name,
                    arguments,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        if content.trim().is_empty() && tool_calls.is_empty() {
            return Err(LlmError::ResponseParse {
                provider: provider.to_string(),
                body_summary: "stream contained no assistant content or tool calls".to_string(),
            });
        }

        Ok(ChatResponse {
            content,
            usage,
            model: model.unwrap_or_else(|| fallback_model.to_string()),
            provider: provider.to_string(),
            raw: None,
            tool_calls,
        })
    }
}

#[derive(Debug, Default)]
struct ControlBlockProjector {
    pending: String,
    hidden: bool,
}

impl ControlBlockProjector {
    const OPENERS: [&'static str; 2] = ["```axiom-tool", "```axiom-todo"];

    fn push(&mut self, delta: &str) -> String {
        self.pending.push_str(delta);
        let mut visible = String::new();
        loop {
            if self.hidden {
                if let Some(end) = self.pending.find("```") {
                    self.pending.drain(..end + 3);
                    self.hidden = false;
                    continue;
                }
                let keep = self.pending.len().min(2);
                let discard = floor_char_boundary(&self.pending, self.pending.len() - keep);
                self.pending.drain(..discard);
                break;
            }

            if let Some((index, opener)) = Self::OPENERS
                .iter()
                .filter_map(|opener| self.pending.find(opener).map(|index| (index, *opener)))
                .min_by_key(|(index, _)| *index)
            {
                visible.push_str(&self.pending[..index]);
                self.pending.drain(..index + opener.len());
                self.hidden = true;
                continue;
            }

            let retained = Self::OPENERS
                .iter()
                .map(|opener| longest_suffix_prefix(&self.pending, opener))
                .max()
                .unwrap_or_default();
            let emit_bytes =
                floor_char_boundary(&self.pending, self.pending.len().saturating_sub(retained));
            if emit_bytes > 0 {
                visible.push_str(&self.pending[..emit_bytes]);
                self.pending.drain(..emit_bytes);
            }
            break;
        }
        visible
    }

    fn finish(mut self) -> String {
        if self.hidden {
            String::new()
        } else {
            std::mem::take(&mut self.pending)
        }
    }
}

fn longest_suffix_prefix(value: &str, marker: &str) -> usize {
    let max = value.len().min(marker.len().saturating_sub(1));
    (1..=max)
        .rev()
        .find(|length| {
            value.is_char_boundary(value.len() - length)
                && marker.is_char_boundary(*length)
                && value[value.len() - length..] == marker[..*length]
        })
        .unwrap_or_default()
}

fn floor_char_boundary(value: &str, requested: usize) -> usize {
    let mut boundary = requested.min(value.len());
    while boundary > 0 && !value.is_char_boundary(boundary) {
        boundary -= 1;
    }
    boundary
}

impl HttpChatStream {
    async fn next_chunk(&mut self) -> Result<Option<ChatChunk>> {
        loop {
            if self.terminal_event_seen {
                self.buffer.clear();
                return Ok(None);
            }
            if let Some((boundary, separator_len)) = event_boundary(&self.buffer) {
                ensure_bytes(&self.provider, "SSE event", boundary, MAX_SSE_EVENT_BYTES)?;
                ensure_additional_count(
                    &self.provider,
                    "SSE event count",
                    self.events_received,
                    1,
                    MAX_STREAM_EVENTS,
                )?;
                self.events_received += 1;
                let event = self.buffer.drain(..boundary).collect::<Vec<_>>();
                self.buffer.drain(..separator_len);
                if let Some(chunk) = parse_sse_event(&self.provider, &event)? {
                    if chunk.done {
                        self.terminal_event_seen = true;
                    }
                    return Ok(Some(chunk));
                }
                continue;
            }

            match self
                .response
                .chunk()
                .await
                .map_err(|error| LlmError::Http {
                    provider: self.provider.clone(),
                    message: error.to_string(),
                })? {
                Some(bytes) => {
                    ensure_additional_bytes(
                        &self.provider,
                        "chat stream wire body",
                        self.wire_bytes_received,
                        bytes.len(),
                        MAX_STREAM_WIRE_BYTES,
                    )?;
                    ensure_additional_bytes(
                        &self.provider,
                        "SSE receive buffer",
                        self.buffer.len(),
                        bytes.len(),
                        MAX_SSE_BUFFER_BYTES,
                    )?;
                    self.wire_bytes_received += bytes.len();
                    self.buffer.extend_from_slice(&bytes);
                    if event_boundary(&self.buffer).is_none() {
                        ensure_bytes(
                            &self.provider,
                            "unterminated SSE event",
                            self.buffer.len(),
                            MAX_SSE_EVENT_BYTES,
                        )?;
                    }
                }
                None if self.buffer.is_empty() => {
                    return Err(LlmError::StreamDisconnected {
                        provider: self.provider.clone(),
                    });
                }
                None => {
                    ensure_bytes(
                        &self.provider,
                        "unterminated SSE event",
                        self.buffer.len(),
                        MAX_SSE_EVENT_BYTES,
                    )?;
                    ensure_additional_count(
                        &self.provider,
                        "SSE event count",
                        self.events_received,
                        1,
                        MAX_STREAM_EVENTS,
                    )?;
                    self.events_received += 1;
                    let event = std::mem::take(&mut self.buffer);
                    if let Some(chunk) = parse_sse_event(&self.provider, &event)? {
                        if chunk.done {
                            self.terminal_event_seen = true;
                            return Ok(Some(chunk));
                        }
                    }
                    return Err(LlmError::StreamDisconnected {
                        provider: self.provider.clone(),
                    });
                }
            }
        }
    }
}

#[derive(Debug, Default)]
struct PartialToolCall {
    id: Option<String>,
    name: String,
    arguments: String,
}

fn event_boundary(buffer: &[u8]) -> Option<(usize, usize)> {
    let crlf = buffer
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|position| (position, 4));
    let lf = buffer
        .windows(2)
        .position(|window| window == b"\n\n")
        .map(|position| (position, 2));
    match (crlf, lf) {
        (Some(crlf), Some(lf)) => Some(if crlf.0 <= lf.0 { crlf } else { lf }),
        (Some(boundary), None) | (None, Some(boundary)) => Some(boundary),
        (None, None) => None,
    }
}

fn parse_sse_event(provider: &str, event: &[u8]) -> Result<Option<ChatChunk>> {
    ensure_bytes(provider, "SSE event", event.len(), MAX_SSE_EVENT_BYTES)?;
    let event_text = std::str::from_utf8(event).map_err(|_| LlmError::ResponseParse {
        provider: provider.to_string(),
        body_summary: summarize_bytes(event),
    })?;
    let mut data = String::new();
    for line in event_text.lines() {
        let Some(value) = line.strip_prefix("data:").map(str::trim_start) else {
            continue;
        };
        let separator_bytes = usize::from(!data.is_empty());
        ensure_additional_bytes(
            provider,
            "SSE data payload",
            data.len(),
            separator_bytes + value.len(),
            MAX_SSE_EVENT_BYTES,
        )?;
        if separator_bytes == 1 {
            data.push('\n');
        }
        data.push_str(value);
    }
    if data.is_empty() {
        return Ok(None);
    }
    if data.trim() == "[DONE]" {
        return Ok(Some(ChatChunk {
            done: true,
            ..ChatChunk::default()
        }));
    }

    let payload: OpenAiStreamPayload =
        serde_json::from_str(&data).map_err(|_| LlmError::ResponseParse {
            provider: provider.to_string(),
            body_summary: summarize_body(&data),
        })?;
    ensure_count(
        provider,
        "SSE choice count",
        payload.choices.len(),
        MAX_STREAM_CHOICES_PER_EVENT,
    )?;
    if let Some(model) = payload.model.as_deref() {
        ensure_bytes(
            provider,
            "response model name",
            model.len(),
            MAX_MODEL_NAME_BYTES,
        )?;
    }
    let mut chunk = ChatChunk {
        usage: payload.usage,
        model: payload.model,
        ..ChatChunk::default()
    };
    for choice in payload.choices {
        chunk.done |= choice.finish_reason.is_some();
        if let Some(delta) = choice.delta {
            if let Some(content) = delta.content {
                ensure_additional_bytes(
                    provider,
                    "SSE assistant content delta",
                    chunk.content_delta.len(),
                    content.len(),
                    MAX_ASSISTANT_CONTENT_BYTES,
                )?;
                chunk.content_delta.push_str(&content);
            }
            ensure_additional_count(
                provider,
                "SSE tool-call delta count",
                chunk.tool_call_deltas.len(),
                delta.tool_calls.len(),
                MAX_TOOL_CALL_DELTAS_PER_EVENT,
            )?;
            for call in delta.tool_calls {
                let function = call.function.unwrap_or_default();
                let name_delta = function.name.unwrap_or_default();
                let arguments_delta = function.arguments.unwrap_or_default();
                if let Some(id) = call.id.as_deref() {
                    ensure_bytes(
                        provider,
                        "tool-call id delta",
                        id.len(),
                        MAX_TOOL_CALL_ID_BYTES,
                    )?;
                }
                ensure_bytes(
                    provider,
                    "tool-call name delta",
                    name_delta.len(),
                    MAX_TOOL_NAME_BYTES,
                )?;
                ensure_bytes(
                    provider,
                    "tool-call argument delta",
                    arguments_delta.len(),
                    MAX_TOOL_ARGUMENT_BYTES,
                )?;
                chunk.tool_call_deltas.push(ChatToolCallDelta {
                    index: call.index,
                    id: call.id,
                    name_delta,
                    arguments_delta,
                });
            }
        }
    }
    Ok(Some(chunk))
}

#[derive(Debug, Deserialize)]
struct OpenAiStreamPayload {
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    choices: Vec<OpenAiStreamChoice>,
    #[serde(default)]
    usage: Option<TokenUsage>,
}

#[derive(Debug, Deserialize)]
struct OpenAiStreamChoice {
    #[serde(default)]
    delta: Option<OpenAiStreamDelta>,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OpenAiStreamDelta {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    tool_calls: Vec<OpenAiStreamToolCall>,
}

#[derive(Debug, Deserialize)]
struct OpenAiStreamToolCall {
    index: usize,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    function: Option<OpenAiStreamFunction>,
}

#[derive(Debug, Default, Deserialize)]
struct OpenAiStreamFunction {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};

    use super::*;

    #[test]
    fn parses_content_and_terminal_sse_events() {
        let content = parse_sse_event(
            "test",
            br#"data: {"model":"gpt-test","choices":[{"delta":{"content":"Hello"},"finish_reason":null}]}"#,
        )
        .expect("parse content")
        .expect("content chunk");
        let done = parse_sse_event("test", b"data: [DONE]")
            .expect("parse done")
            .expect("done chunk");

        assert_eq!(content.content_delta, "Hello");
        assert_eq!(content.model.as_deref(), Some("gpt-test"));
        assert!(!content.done);
        assert!(done.done);
    }

    #[tokio::test]
    async fn accumulates_fragmented_tool_calls_into_a_response() {
        let stream = ChatStream::from_chunks(vec![
            ChatChunk {
                tool_call_deltas: vec![ChatToolCallDelta {
                    index: 0,
                    id: Some("call_1".to_string()),
                    name_delta: "axiom_file_".to_string(),
                    arguments_delta: "{\"path\":\"READ".to_string(),
                }],
                model: Some("model".to_string()),
                ..ChatChunk::default()
            },
            ChatChunk {
                tool_call_deltas: vec![ChatToolCallDelta {
                    index: 0,
                    id: None,
                    name_delta: "read".to_string(),
                    arguments_delta: "ME.md\"}".to_string(),
                }],
                usage: Some(TokenUsage {
                    prompt_tokens: 5,
                    completion_tokens: 3,
                    total_tokens: 8,
                }),
                done: true,
                ..ChatChunk::default()
            },
        ]);

        let response = stream
            .collect_response("test", "fallback")
            .await
            .expect("collect response");

        assert_eq!(response.tool_calls[0].name, "axiom_file_read");
        assert_eq!(response.tool_calls[0].arguments["path"], "README.md");
        assert_eq!(response.usage.expect("usage").total_tokens, 8);
    }

    #[tokio::test]
    async fn live_projection_hides_fragmented_control_blocks() {
        let stream = ChatStream::from_chunks(vec![
            ChatChunk {
                content_delta: "I will inspect it.\n```axi".to_string(),
                ..ChatChunk::default()
            },
            ChatChunk {
                content_delta: "om-tool\n{\"skill_id\":\"file.read\"}\n``".to_string(),
                ..ChatChunk::default()
            },
            ChatChunk {
                content_delta: "`\nInspection complete.".to_string(),
                done: true,
                ..ChatChunk::default()
            },
        ]);
        let mut visible = String::new();

        let response = stream
            .collect_response_with_observer("test", "model", |update| {
                visible.push_str(&update.visible_delta);
            })
            .await
            .expect("collect");

        assert!(response.content.contains("skill_id"));
        assert_eq!(visible, "I will inspect it.\n\nInspection complete.");
        assert!(!visible.contains("axiom-tool"));
        assert!(!visible.contains("skill_id"));
    }

    #[tokio::test]
    async fn aggregate_assistant_content_is_bounded() {
        let stream = ChatStream::from_chunks(vec![ChatChunk {
            content_delta: "x".repeat(MAX_ASSISTANT_CONTENT_BYTES + 1),
            done: true,
            ..ChatChunk::default()
        }]);

        let error = stream
            .collect_response("test", "model")
            .await
            .expect_err("oversized content");
        assert!(matches!(
            error,
            LlmError::ResponseLimitExceeded {
                resource: "assistant content",
                ..
            }
        ));
    }

    #[tokio::test]
    async fn aggregate_tool_call_names_arguments_counts_and_deltas_are_bounded() {
        let cases = [
            (
                ChatStream::from_chunks(vec![ChatChunk {
                    tool_call_deltas: vec![ChatToolCallDelta {
                        index: 0,
                        id: None,
                        name_delta: "n".repeat(MAX_TOOL_NAME_BYTES + 1),
                        arguments_delta: "{}".to_string(),
                    }],
                    done: true,
                    ..ChatChunk::default()
                }]),
                "tool-call name",
            ),
            (
                ChatStream::from_chunks(vec![ChatChunk {
                    tool_call_deltas: vec![ChatToolCallDelta {
                        index: 0,
                        id: None,
                        name_delta: "tool".to_string(),
                        arguments_delta: "x".repeat(MAX_TOOL_ARGUMENT_BYTES + 1),
                    }],
                    done: true,
                    ..ChatChunk::default()
                }]),
                "tool-call arguments",
            ),
            (
                ChatStream::from_chunks(vec![ChatChunk {
                    tool_call_deltas: (0..=MAX_TOOL_CALLS)
                        .map(|index| ChatToolCallDelta {
                            index,
                            id: None,
                            name_delta: "tool".to_string(),
                            arguments_delta: "{}".to_string(),
                        })
                        .collect(),
                    done: true,
                    ..ChatChunk::default()
                }]),
                "tool-call count",
            ),
            (
                ChatStream::from_chunks(vec![ChatChunk {
                    tool_call_deltas: (0..=MAX_TOOL_CALL_DELTAS)
                        .map(|_| ChatToolCallDelta::default())
                        .collect(),
                    done: true,
                    ..ChatChunk::default()
                }]),
                "tool-call delta count",
            ),
        ];

        for (stream, expected_resource) in cases {
            let error = stream
                .collect_response("test", "model")
                .await
                .expect_err("tool-call state must be bounded");
            assert!(matches!(
                error,
                LlmError::ResponseLimitExceeded { resource, .. } if resource == expected_resource
            ));
        }
    }

    #[test]
    fn oversized_sse_events_and_per_event_tool_deltas_are_rejected() {
        let event = vec![b'x'; MAX_SSE_EVENT_BYTES + 1];
        let error = parse_sse_event("test", &event).expect_err("oversized event");
        assert!(matches!(
            error,
            LlmError::ResponseLimitExceeded {
                resource: "SSE event",
                ..
            }
        ));

        let calls = (0..=MAX_TOOL_CALL_DELTAS_PER_EVENT)
            .map(|index| serde_json::json!({"index": index}))
            .collect::<Vec<_>>();
        let event = format!(
            "data: {}",
            serde_json::json!({"choices":[{"delta":{"tool_calls":calls}}]})
        );
        let error = parse_sse_event("test", event.as_bytes()).expect_err("too many deltas");
        assert!(matches!(
            error,
            LlmError::ResponseLimitExceeded {
                resource: "SSE tool-call delta count",
                ..
            }
        ));
    }

    #[tokio::test]
    async fn unterminated_http_sse_event_cannot_grow_the_receive_buffer() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind test server");
        let address = listener.local_addr().expect("server address");
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().expect("accept request");
            let mut request = [0_u8; 2_048];
            let _ = socket.read(&mut request).expect("read request");
            let body = vec![b'x'; MAX_SSE_EVENT_BYTES + 1];
            write!(
                socket,
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .expect("write response headers");
            let _ = socket.write_all(&body);
        });
        let response = crate::provider::build_provider_http_client()
            .expect("client")
            .get(format!("http://{address}/"))
            .send()
            .await
            .expect("response");
        let mut stream = ChatStream::from_http(response, "test");

        let error = stream
            .next_chunk()
            .await
            .expect_err("unterminated event must be bounded");
        server.join().expect("server thread");
        assert!(matches!(
            error,
            LlmError::ResponseLimitExceeded {
                resource: "unterminated SSE event",
                ..
            }
        ));
    }

    #[test]
    fn projector_preserves_unicode_across_partial_markers() {
        let mut projector = ControlBlockProjector::default();
        let mut visible = projector.push("hi 🦀 ``");
        visible.push_str(&projector.push("not-a-control"));
        visible.push_str(&projector.finish());
        assert_eq!(visible, "hi 🦀 ``not-a-control");
    }

    #[test]
    fn detects_crlf_and_lf_event_boundaries() {
        assert_eq!(event_boundary(b"data: one\n\ndata: two"), Some((9, 2)));
        assert_eq!(event_boundary(b"data: one\r\n\r\ndata: two"), Some((9, 4)));
        assert_eq!(
            event_boundary(b"data: first\n\ndata: second\r\n\r\n"),
            Some((11, 2))
        );
    }

    #[test]
    fn every_partial_event_boundary_waits_for_a_complete_separator() {
        for event in [
            b"data: one\n\ndata: two".as_slice(),
            b"data: one\r\n\r\ndata: two".as_slice(),
        ] {
            let (boundary, separator_len) = event_boundary(event).expect("complete event");
            for split in 0..boundary + separator_len {
                assert_eq!(
                    event_boundary(&event[..split]),
                    None,
                    "premature boundary at byte {split}"
                );
            }
        }
    }

    #[test]
    fn malformed_sse_corpus_never_panics() {
        let mut state = 0xe703_7ed1_a0b4_28db_u64;
        for length in 0..512 {
            let mut event = Vec::with_capacity(length + 6);
            if length % 2 == 0 {
                event.extend_from_slice(b"data:");
            }
            for _ in 0..length {
                state ^= state >> 12;
                state ^= state << 25;
                state ^= state >> 27;
                event.push(state as u8);
            }
            let _ = parse_sse_event("corpus", &event);
        }
    }
}
