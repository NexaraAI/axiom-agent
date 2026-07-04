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
    output
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
}
