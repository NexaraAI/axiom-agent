use std::sync::{OnceLock, RwLock};

const MAX_REGISTERED_SECRETS: usize = 128;
const MAX_REGISTERED_SECRET_CHARS: usize = 16 * 1024;

static REGISTERED_SECRETS: OnceLock<RwLock<Vec<String>>> = OnceLock::new();

/// Registers an in-memory credential value so durable proof captures can
/// remove exact matches without exposing the credential through process-global
/// environment variables. Values are never serialized by this registry.
pub fn register_secret_for_redaction(secret: &str) {
    let length = secret.chars().count();
    if !(8..=MAX_REGISTERED_SECRET_CHARS).contains(&length) {
        return;
    }
    let secrets = REGISTERED_SECRETS.get_or_init(|| RwLock::new(Vec::new()));
    let mut secrets = secrets
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if secrets.len() < MAX_REGISTERED_SECRETS && !secrets.iter().any(|value| value == secret) {
        secrets.push(secret.to_string());
        secrets.sort_by_key(|value| std::cmp::Reverse(value.len()));
    }
}

pub fn summarize_text(input: &str, max_chars: usize) -> String {
    let redacted = redact_text(input);
    if redacted.chars().count() <= max_chars {
        redacted
    } else {
        let mut summary = redacted.chars().take(max_chars).collect::<String>();
        summary.push_str("...[truncated]");
        summary
    }
}

pub fn redact_text(input: &str) -> String {
    let mut output = redact_bearer_tokens(input);
    output = redact_key_value_secrets(&output);
    output = redact_private_key_blocks(&output);
    output = redact_prefixed_tokens(&output);
    output = redact_registered_secrets(&output);
    output = redact_environment_secrets(&output);
    output
}

fn redact_registered_secrets(input: &str) -> String {
    let Some(secrets) = REGISTERED_SECRETS.get() else {
        return input.to_string();
    };
    let secrets = secrets
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    secrets.iter().fold(input.to_string(), |output, secret| {
        output.replace(secret, "[REDACTED]")
    })
}

fn redact_prefixed_tokens(input: &str) -> String {
    const PREFIXES: &[&str] = &[
        "github_pat_",
        "sk-or-v1-",
        "xoxb-",
        "xoxp-",
        "xoxa-",
        "xoxr-",
        "gsk_",
        "ghp_",
        "gho_",
        "ghu_",
        "ghs_",
        "ghr_",
        "hf_",
        "AIza",
        "sk-",
    ];

    let mut output = input.to_string();
    for prefix in PREFIXES {
        output = redact_token_prefix(&output, prefix);
    }
    output
}

fn redact_token_prefix(input: &str, prefix: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut remainder = input;
    while let Some(index) = remainder.find(prefix) {
        output.push_str(&remainder[..index]);
        let candidate = &remainder[index..];
        let preceded_by_identifier = remainder[..index]
            .chars()
            .next_back()
            .is_some_and(|character| character.is_ascii_alphanumeric() || character == '_');
        if preceded_by_identifier {
            output.push_str(prefix);
            remainder = &candidate[prefix.len()..];
            continue;
        }
        let token_len = candidate
            .char_indices()
            .take_while(|(_, character)| {
                character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.')
            })
            .map(|(offset, character)| offset + character.len_utf8())
            .last()
            .unwrap_or(0);
        let token = &candidate[..token_len];
        if prefix == "sk-" && looks_like_semantic_sk_slug(token) {
            output.push_str(token);
            remainder = &candidate[token_len..];
        } else if token_len >= prefix.len() + 8 {
            output.push_str("[REDACTED]");
            remainder = &candidate[token_len..];
        } else {
            output.push_str(prefix);
            remainder = &candidate[prefix.len()..];
        }
    }
    output.push_str(remainder);
    output
}

// `sk-` is both a common provider-key prefix and a reasonable namespace for
// human-readable resource IDs. Treat a multi-word all-letter slug with short
// components as semantic text, while retaining redaction for long opaque
// payloads such as `sk-abcdefghijklmnopqrstuvwxyz` and `sk-proj-...`.
fn looks_like_semantic_sk_slug(token: &str) -> bool {
    let Some(rest) = token.strip_prefix("sk-") else {
        return false;
    };
    let mut segments = rest.split('-');
    let first = segments.next();
    first.is_some()
        && segments.clone().next().is_some()
        && rest.split('-').all(|segment| {
            !segment.is_empty()
                && segment.len() <= 16
                && segment.bytes().all(|byte| byte.is_ascii_alphabetic())
        })
}

fn redact_environment_secrets(input: &str) -> String {
    let mut secrets = std::env::vars_os()
        .filter_map(|(name, value)| {
            let name = name.to_str()?.to_ascii_uppercase();
            let is_secret = [
                "API_KEY",
                "APIKEY",
                "TOKEN",
                "SECRET",
                "PASSWORD",
                "CREDENTIAL",
                "AUTHORIZATION",
            ]
            .iter()
            .any(|marker| name.contains(marker));
            let value = value.to_str()?;
            (is_secret && value.chars().count() >= 8).then(|| value.to_string())
        })
        .collect::<Vec<_>>();
    secrets.sort_by_key(|value| std::cmp::Reverse(value.len()));
    secrets.dedup();

    secrets
        .into_iter()
        .fold(input.to_string(), |output, secret| {
            output.replace(&secret, "[REDACTED]")
        })
}

fn redact_bearer_tokens(input: &str) -> String {
    input
        .lines()
        .map(|line| {
            let lower = line.to_ascii_lowercase();
            if let Some(index) = lower.find("bearer ") {
                format!("{}Bearer [REDACTED]", &line[..index])
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn redact_key_value_secrets(input: &str) -> String {
    input
        .lines()
        .map(|line| {
            let lower = line.to_ascii_lowercase();
            let secret_like = [
                "api_key",
                "apikey",
                "api-key",
                "token",
                "secret",
                "password",
                "authorization",
                "credential",
            ]
            .iter()
            .any(|needle| lower.contains(needle));
            let assignment = line.contains('=') || line.contains(':');
            if secret_like && assignment {
                if let Some((left, _right)) = line.split_once('=') {
                    format!("{left}=[REDACTED]")
                } else if let Some((left, _right)) = line.split_once(':') {
                    format!("{left}: [REDACTED]")
                } else {
                    "[REDACTED]".to_string()
                }
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn redact_private_key_blocks(input: &str) -> String {
    let mut output = String::new();
    let mut redacting = false;
    for line in input.lines() {
        if line.contains("-----BEGIN") && line.contains("PRIVATE KEY-----") {
            redacting = true;
            output.push_str("[REDACTED PRIVATE KEY]\n");
            continue;
        }
        if redacting {
            if line.contains("-----END") && line.contains("PRIVATE KEY-----") {
                redacting = false;
            }
            continue;
        }
        output.push_str(line);
        output.push('\n');
    }
    if input.ends_with('\n') {
        output
    } else {
        output.trim_end_matches('\n').to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_api_key_like_text() {
        let redacted = redact_text("OPENAI_API_KEY=sk-test");

        assert_eq!(redacted, "OPENAI_API_KEY=[REDACTED]");
    }

    #[test]
    fn redacts_bearer_token() {
        let redacted = redact_text("Authorization: Bearer abc123");

        assert!(redacted.contains("[REDACTED]"));
        assert!(!redacted.contains("abc123"));
    }

    #[test]
    fn redacts_env_content() {
        let redacted = redact_text("TOKEN=abc\nNORMAL=value");

        assert!(redacted.contains("TOKEN=[REDACTED]"));
        assert!(redacted.contains("NORMAL=value"));
    }

    #[test]
    fn enforces_max_capture_length() {
        let summary = summarize_text("abcdef", 3);

        assert_eq!(summary, "abc...[truncated]");
    }

    #[test]
    fn redacts_common_raw_provider_token_shapes() {
        let cases = [
            "sk-abcdefghijklmnopqrstuvwxyz",
            "gsk_abcdefghijklmnopqrstuvwxyz",
            "github_pat_abcdefghijklmnopqrstuvwxyz",
            "AIzaabcdefghijklmnopqrstuvwxyz",
        ];

        for secret in cases {
            let redacted = redact_text(&format!("please use {secret} now"));
            assert!(!redacted.contains(secret), "raw token leaked: {secret}");
            assert!(redacted.contains("[REDACTED]"));
        }
    }

    #[test]
    fn provider_prefix_matching_preserves_larger_identifiers() {
        let task_id = ["task", "19f485fa5fc7"].join("-");

        assert_eq!(redact_text(&task_id), task_id);
    }

    #[test]
    fn preserves_semantic_sk_slugs_without_treating_them_as_api_keys() {
        let identifier = ["sk", "database", "migration"].join("-");

        assert_eq!(redact_text(&identifier), identifier);
        assert!(redact_text("sk-abcdefghijklmnopqrstuvwxyz").contains("[REDACTED]"));
    }

    #[test]
    fn redacts_exact_secret_values_from_environment() {
        let name = "AXIOM_PROOF_TEST_API_KEY";
        let secret = "opaque-value-without-provider-prefix";
        let previous = std::env::var_os(name);
        std::env::set_var(name, secret);

        let redacted = redact_text(&format!("credential is {secret}"));

        if let Some(previous) = previous {
            std::env::set_var(name, previous);
        } else {
            std::env::remove_var(name);
        }
        assert_eq!(redacted, "credential is [REDACTED]");
    }

    #[test]
    fn redacts_exact_registered_keyring_value_without_environment_hydration() {
        let secret = "opaque-keyring-value-for-proof-test";
        register_secret_for_redaction(secret);

        let redacted = redact_text(&format!("credential is {secret}"));

        assert_eq!(redacted, "credential is [REDACTED]");
    }

    #[test]
    fn generated_secret_corpus_is_redacted_and_bounded() {
        let secret = "axiom-corpus-secret-4f1a";
        let labels = [
            "api_key",
            "TOKEN",
            "password",
            "Authorization",
            "credential",
        ];
        let mut state = 0x243f_6a88_u32;

        for index in 0..2_048 {
            state = state.rotate_left(5) ^ 0x9e37_79b9;
            let prefix = format!("noise-{state:08x}");
            let separator = if index % 2 == 0 { '=' } else { ':' };
            let input = format!(
                "{prefix}\n{}{separator}{secret}\ntrailer-{index}",
                labels[index % labels.len()]
            );
            let redacted = redact_text(&input);
            assert!(!redacted.contains(secret));

            let max_chars = index % 64;
            let summary = summarize_text(&input, max_chars);
            assert!(!summary.contains(secret));
            assert!(summary.chars().count() <= max_chars + "...[truncated]".chars().count());
        }
    }
}
