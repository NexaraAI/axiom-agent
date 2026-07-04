use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatChunk {
    pub content_delta: String,
    pub done: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChatStream {
    chunks: Vec<ChatChunk>,
}

impl ChatStream {
    pub fn empty() -> Self {
        Self { chunks: Vec::new() }
    }

    pub fn from_chunks(chunks: Vec<ChatChunk>) -> Self {
        Self { chunks }
    }

    pub fn chunks(&self) -> &[ChatChunk] {
        &self.chunks
    }
}
