use std::{fmt, net::IpAddr, time::Duration};

use async_trait::async_trait;
use thiserror::Error;
use tokio::time::sleep;

use crate::{ChatRequest, ChatResponse, ChatStream, ModelInfo};

pub type Result<T> = std::result::Result<T, LlmError>;

/// Provider credentials are intentionally omitted from `Debug` output. The
/// value is kept inside the HTTP provider instead of being exported through
/// the process environment, where unrelated child processes could inherit it.
#[derive(Clone)]
pub(crate) struct SecretValue(String);

impl SecretValue {
    pub(crate) fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub(crate) fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SecretValue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("[REDACTED]")
    }
}

#[derive(Debug, Error)]
pub enum LlmError {
    #[error("missing provider field: {0}")]
    MissingField(&'static str),
    #[error(
        "API key/token environment variable is not set: {env}. Set {env} before starting chat."
    )]
    MissingApiKeyEnv { env: String },
    #[error("API key/token environment variable is empty: {env}. Set {env} to a non-empty value.")]
    EmptyApiKeyEnv { env: String },
    #[error("unsafe API key/token environment variable name: {env}")]
    UnsafeCredentialEnv { env: String },
    #[error("provider request construction failed: {0}")]
    RequestBuild(String),
    #[error("unsafe provider endpoint in {field}: {reason}")]
    UnsafeEndpoint { field: &'static str, reason: String },
    #[error("unsafe provider identifier in {field}: {reason}")]
    UnsafeProviderIdentifier { field: &'static str, reason: String },
    #[error("{provider} request failed: {message}")]
    Http { provider: String, message: String },
    #[error("{provider} returned HTTP {status}: {body_summary}")]
    HttpStatus {
        provider: String,
        status: u16,
        body_summary: String,
    },
    #[error("provider request failed after {attempts} attempts: {last}")]
    Retried {
        attempts: u8,
        #[source]
        last: Box<LlmError>,
    },
    #[error("{provider} response could not be parsed: {body_summary}")]
    ResponseParse {
        provider: String,
        body_summary: String,
    },
    #[error("{provider} response exceeded the {resource} limit ({limit} {unit})")]
    ResponseLimitExceeded {
        provider: String,
        resource: &'static str,
        limit: usize,
        unit: &'static str,
    },
    #[error("{provider} stream disconnected before a terminal event")]
    StreamDisconnected { provider: String },
    #[error("{0}")]
    NotImplemented(&'static str),
}

pub(crate) fn build_provider_http_client() -> std::result::Result<reqwest::Client, String> {
    configure_provider_http_client(reqwest::Client::builder())
        .build()
        .map_err(|error| error.to_string())
}

fn configure_provider_http_client(builder: reqwest::ClientBuilder) -> reqwest::ClientBuilder {
    builder
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(60))
        .redirect(reqwest::redirect::Policy::none())
        // Provider credentials must never be forwarded to a proxy selected through ambient
        // HTTP(S)_PROXY/ALL_PROXY process state. Provider endpoints are explicit Axiom config.
        .no_proxy()
}

pub(crate) async fn read_response_body_limited(
    mut response: reqwest::Response,
    provider: &str,
    success_resource: &'static str,
    success_limit: usize,
) -> Result<(reqwest::StatusCode, Vec<u8>)> {
    let status = response.status();
    let (resource, limit) = if status.is_success() {
        (success_resource, success_limit)
    } else {
        (
            "provider error body",
            crate::limits::MAX_ERROR_HTTP_BODY_BYTES,
        )
    };
    ensure_response_content_length(&response, provider, resource, limit)?;

    let capacity = response
        .content_length()
        .and_then(|length| usize::try_from(length).ok())
        .unwrap_or_default()
        .min(limit);
    let mut body = Vec::with_capacity(capacity);
    while let Some(chunk) = response.chunk().await.map_err(|error| LlmError::Http {
        provider: provider.to_string(),
        message: error.to_string(),
    })? {
        crate::limits::ensure_additional_bytes(provider, resource, body.len(), chunk.len(), limit)?;
        body.extend_from_slice(&chunk);
    }
    Ok((status, body))
}

pub(crate) fn ensure_response_content_length(
    response: &reqwest::Response,
    provider: &str,
    resource: &'static str,
    limit: usize,
) -> Result<()> {
    if response
        .content_length()
        .is_some_and(|length| length > limit as u64)
    {
        Err(crate::limits::limit_error(
            provider, resource, limit, "bytes",
        ))
    } else {
        Ok(())
    }
}

pub fn validate_credential_env_name(env: &str) -> Result<()> {
    const RESERVED_ENV_NAMES: &[&str] = &[
        "ALL_PROXY",
        "APPDATA",
        "AXIOM_HOME",
        "COMSPEC",
        "DYLD_INSERT_LIBRARIES",
        "DYLD_LIBRARY_PATH",
        "HOME",
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "LD_LIBRARY_PATH",
        "LD_PRELOAD",
        "LOCALAPPDATA",
        "NODE_OPTIONS",
        "NO_PROXY",
        "PATH",
        "PATHEXT",
        "PWD",
        "RUSTFLAGS",
        "RUST_BACKTRACE",
        "RUST_LOG",
        "SHELL",
        "SYSTEMROOT",
        "TEMP",
        "TMP",
        "USERPROFILE",
        "WINDIR",
    ];
    let mut characters = env.chars();
    let valid_syntax = characters
        .next()
        .is_some_and(|character| character == '_' || character.is_ascii_alphabetic())
        && characters.all(|character| character == '_' || character.is_ascii_alphanumeric());
    let reserved = RESERVED_ENV_NAMES
        .iter()
        .any(|candidate| env.eq_ignore_ascii_case(candidate));
    if env.len() <= 128 && valid_syntax && !reserved {
        Ok(())
    } else {
        Err(LlmError::UnsafeCredentialEnv {
            env: env.to_string(),
        })
    }
}

pub fn validate_provider_endpoint(
    field: &'static str,
    endpoint: &str,
    allow_loopback_http: bool,
) -> Result<reqwest::Url> {
    let url = reqwest::Url::parse(endpoint).map_err(|error| LlmError::UnsafeEndpoint {
        field,
        reason: format!("invalid URL: {error}"),
    })?;
    if !url.username().is_empty() || url.password().is_some() {
        return Err(LlmError::UnsafeEndpoint {
            field,
            reason: "embedded usernames and passwords are not allowed".to_string(),
        });
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err(LlmError::UnsafeEndpoint {
            field,
            reason: "query strings and fragments are not allowed on provider base URLs".to_string(),
        });
    }

    let host = url.host_str().ok_or_else(|| LlmError::UnsafeEndpoint {
        field,
        reason: "a hostname is required".to_string(),
    })?;
    let normalized_host = host
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
        .unwrap_or(host);
    let is_literal_loopback = normalized_host.eq_ignore_ascii_case("localhost")
        || normalized_host
            .parse::<IpAddr>()
            .is_ok_and(|address| address.is_loopback());
    match url.scheme() {
        "https" => Ok(url),
        "http" if allow_loopback_http && is_literal_loopback => Ok(url),
        "http" => Err(LlmError::UnsafeEndpoint {
            field,
            reason: "plain HTTP is allowed only for localhost or a literal loopback address"
                .to_string(),
        }),
        scheme => Err(LlmError::UnsafeEndpoint {
            field,
            reason: format!("unsupported URL scheme `{scheme}`; expected HTTPS"),
        }),
    }
}

#[async_trait]
pub trait LlmProvider: Send + Sync {
    async fn chat(&self, request: ChatRequest) -> Result<ChatResponse>;
    async fn stream_chat(&self, request: ChatRequest) -> Result<ChatStream>;
    async fn models(&self) -> Result<Vec<ModelInfo>>;
    fn provider_name(&self) -> &str;
}

pub(crate) async fn retry_transient<T, F, Fut>(mut operation: F) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T>>,
{
    const MAX_ATTEMPTS: u8 = 3;
    let mut attempt = 0_u8;

    loop {
        attempt += 1;
        match operation().await {
            Ok(value) => return Ok(value),
            Err(error) if is_retryable(&error) && attempt < MAX_ATTEMPTS => {
                sleep(Duration::from_millis(
                    100 * u64::from(1_u8 << (attempt - 1)),
                ))
                .await;
            }
            Err(error) if is_retryable(&error) => {
                return Err(LlmError::Retried {
                    attempts: attempt,
                    last: Box::new(error),
                });
            }
            Err(error) => return Err(error),
        }
    }
}

fn is_retryable(error: &LlmError) -> bool {
    matches!(
        error,
        LlmError::Http { .. }
            | LlmError::HttpStatus {
                status: 429 | 500..=599,
                ..
            }
    )
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::sync::atomic::{AtomicU8, Ordering};

    use super::*;

    #[test]
    fn provider_client_configuration_removes_all_proxies() {
        let proxy = reqwest::Proxy::all("http://127.0.0.1:9").expect("test proxy");
        let proxied = reqwest::Client::builder()
            .proxy(proxy.clone())
            .build()
            .expect("proxied client");
        assert!(format!("{proxied:?}").contains("proxies"));

        let hardened = configure_provider_http_client(reqwest::Client::builder().proxy(proxy))
            .build()
            .expect("hardened client");
        assert!(!format!("{hardened:?}").contains("proxies"));
    }

    #[tokio::test]
    async fn response_reader_enforces_the_limit_without_content_length() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind server");
        let address = listener.local_addr().expect("server address");
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept request");
            let mut request = [0_u8; 2_048];
            let _ = stream.read(&mut request).expect("read request");
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n8\r\n12345678\r\n8\r\nabcdefgh\r\n0\r\n\r\n",
                )
                .expect("write response");
        });
        let response = build_provider_http_client()
            .expect("client")
            .get(format!("http://{address}/"))
            .send()
            .await
            .expect("response");

        let error = read_response_body_limited(response, "test", "test body", 10)
            .await
            .expect_err("body must be bounded");
        server.join().expect("server thread");

        assert!(matches!(
            error,
            LlmError::ResponseLimitExceeded {
                resource: "test body",
                limit: 10,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn retries_a_transient_http_status_then_returns_the_value() {
        let attempts = AtomicU8::new(0);
        let result = retry_transient(|| {
            let attempt = attempts.fetch_add(1, Ordering::SeqCst);
            async move {
                if attempt == 0 {
                    Err(LlmError::HttpStatus {
                        provider: "test".to_string(),
                        status: 503,
                        body_summary: "unavailable".to_string(),
                    })
                } else {
                    Ok("recovered")
                }
            }
        })
        .await
        .expect("retry should recover");

        assert_eq!(result, "recovered");
        assert_eq!(attempts.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn wraps_the_final_transient_error_with_attempt_count() {
        let result = retry_transient(|| async {
            Err::<(), _>(LlmError::HttpStatus {
                provider: "test".to_string(),
                status: 429,
                body_summary: "rate limited".to_string(),
            })
        })
        .await
        .expect_err("rate limit should exhaust retries");

        assert!(matches!(result, LlmError::Retried { attempts: 3, .. }));
    }

    #[test]
    fn provider_endpoint_policy_allows_https_and_loopback_development_http() {
        assert!(validate_provider_endpoint("base_url", "https://api.example.com/v1", true).is_ok());
        assert!(validate_provider_endpoint("base_url", "http://localhost:11434/v1", true).is_ok());
        assert!(validate_provider_endpoint("base_url", "http://127.0.0.1:8000/v1", true).is_ok());
        assert!(validate_provider_endpoint("base_url", "http://[::1]:8000/v1", true).is_ok());
    }

    #[test]
    fn provider_endpoint_policy_rejects_cleartext_remote_and_url_credentials() {
        for endpoint in [
            "http://api.example.com/v1",
            "https://user:secret@api.example.com/v1",
            "file:///tmp/provider",
            "https://api.example.com/v1?token=secret",
            "https://api.example.com/v1#fragment",
        ] {
            assert!(matches!(
                validate_provider_endpoint("base_url", endpoint, true),
                Err(LlmError::UnsafeEndpoint { .. })
            ));
        }
    }

    #[test]
    fn credential_environment_names_are_syntactic_and_cannot_replace_process_controls() {
        for valid in ["OPENAI_API_KEY", "GITHUB_TOKEN", "_LOCAL_MODEL_SECRET"] {
            assert!(validate_credential_env_name(valid).is_ok(), "{valid}");
        }
        for invalid in ["", "1TOKEN", "API-KEY", "PATH", "A=B", "AXIOM_HOME"] {
            assert!(matches!(
                validate_credential_env_name(invalid),
                Err(LlmError::UnsafeCredentialEnv { .. })
            ));
        }
    }
}
