use crate::{LlmError, Result};

// Provider responses are untrusted input. These limits bound both the bytes retained from HTTP
// and the decoded state accumulated across a stream. They are deliberately independent of model
// token settings because custom OpenAI-compatible endpoints need not honor those settings.
pub(crate) const MAX_CHAT_HTTP_BODY_BYTES: usize = 8 * 1024 * 1024;
pub(crate) const MAX_MODEL_CATALOG_HTTP_BODY_BYTES: usize = 4 * 1024 * 1024;
pub(crate) const MAX_ERROR_HTTP_BODY_BYTES: usize = 64 * 1024;
pub(crate) const MAX_STREAM_WIRE_BYTES: usize = 16 * 1024 * 1024;
pub(crate) const MAX_SSE_BUFFER_BYTES: usize = 2 * 1024 * 1024;
pub(crate) const MAX_SSE_EVENT_BYTES: usize = 1024 * 1024;
pub(crate) const MAX_STREAM_EVENTS: usize = 65_536;

pub(crate) const MAX_ASSISTANT_CONTENT_BYTES: usize = 4 * 1024 * 1024;
pub(crate) const MAX_MODEL_NAME_BYTES: usize = 1_024;
pub(crate) const MAX_CHAT_CHOICES: usize = 16;
pub(crate) const MAX_STREAM_CHOICES_PER_EVENT: usize = 16;
pub(crate) const MAX_TOOL_CALLS: usize = 128;
pub(crate) const MAX_TOOL_CALL_DELTAS: usize = 16_384;
pub(crate) const MAX_TOOL_CALL_DELTAS_PER_EVENT: usize = 256;
pub(crate) const MAX_TOOL_CALL_ID_BYTES: usize = 512;
pub(crate) const MAX_TOOL_NAME_BYTES: usize = 256;
pub(crate) const MAX_TOOL_ARGUMENT_BYTES: usize = 2 * 1024 * 1024;
pub(crate) const MAX_TOTAL_TOOL_ARGUMENT_BYTES: usize = 4 * 1024 * 1024;

pub(crate) const MAX_MODEL_CATALOG_ENTRIES: usize = 20_000;
pub(crate) const MAX_MODEL_ID_BYTES: usize = 1_024;
pub(crate) const MAX_MODEL_DESCRIPTION_BYTES: usize = 64 * 1024;

pub(crate) fn ensure_bytes(
    provider: &str,
    resource: &'static str,
    actual: usize,
    limit: usize,
) -> Result<()> {
    if actual <= limit {
        Ok(())
    } else {
        Err(limit_error(provider, resource, limit, "bytes"))
    }
}

pub(crate) fn ensure_additional_bytes(
    provider: &str,
    resource: &'static str,
    current: usize,
    additional: usize,
    limit: usize,
) -> Result<()> {
    if current <= limit && additional <= limit - current {
        Ok(())
    } else {
        Err(limit_error(provider, resource, limit, "bytes"))
    }
}

pub(crate) fn ensure_count(
    provider: &str,
    resource: &'static str,
    actual: usize,
    limit: usize,
) -> Result<()> {
    if actual <= limit {
        Ok(())
    } else {
        Err(limit_error(provider, resource, limit, "items"))
    }
}

pub(crate) fn ensure_additional_count(
    provider: &str,
    resource: &'static str,
    current: usize,
    additional: usize,
    limit: usize,
) -> Result<()> {
    if current <= limit && additional <= limit - current {
        Ok(())
    } else {
        Err(limit_error(provider, resource, limit, "items"))
    }
}

pub(crate) fn limit_error(
    provider: &str,
    resource: &'static str,
    limit: usize,
    unit: &'static str,
) -> LlmError {
    LlmError::ResponseLimitExceeded {
        provider: provider.to_string(),
        resource,
        limit,
        unit,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addition_checks_do_not_overflow() {
        assert!(ensure_additional_bytes("test", "body", usize::MAX, 1, 10).is_err());
        assert!(ensure_additional_count("test", "events", usize::MAX, 1, 10).is_err());
    }
}
