//! Validation for environment variable names that Axiom will read a secret
//! from.
//!
//! This is the single implementation. It previously existed twice with
//! different rules: `axiom-llm::validate_credential_env_name` rejected 27
//! reserved names case-insensitively, while
//! `axiom-core::config::validate_gateway_token_env_name` rejected 11
//! case-sensitively. A config naming `TEMP` or `COMSPEC` as a gateway or MCP
//! secret variable therefore passed config validation and failed later, at
//! runtime, when the credential path checked it against the stricter list.
//!
//! The strict list wins. A false accept here means a config that loads fine
//! and then breaks the first time the gateway or MCP server starts.

/// Environment variables that must never hold a credential Axiom reads.
///
/// The cases that matter most are the loader-hijack and code-injection ones
/// (`LD_PRELOAD`, `LD_LIBRARY_PATH`, `DYLD_INSERT_LIBRARIES`), the ones that
/// change how child processes resolve binaries (`PATH`, `PATHEXT`,
/// `COMSPEC`, `NODE_OPTIONS`), and the ones that redirect or suppress
/// diagnostics (`RUST_LOG`, `RUST_BACKTRACE`, `RUSTFLAGS`, the proxy
/// variables). A gateway token dropped into any of them would not just be
/// stored in the wrong place, it would change program behavior.
pub const RESERVED_CREDENTIAL_ENV_NAMES: &[&str] = &[
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

const MAX_ENV_NAME_LEN: usize = 128;

/// Why an environment variable name was rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CredentialEnvNameError {
    /// The name is empty, too long, or contains characters no shell can set
    /// as a variable name.
    InvalidSyntax { name: String },
    /// The name collides with a variable that changes process behavior.
    Reserved { name: String },
}

impl std::fmt::Display for CredentialEnvNameError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidSyntax { name } => write!(
                formatter,
                "`{name}` is not a valid environment variable name; \
                 use letters, digits, and underscores, starting with a letter or underscore"
            ),
            Self::Reserved { name } => write!(
                formatter,
                "`{name}` is reserved; use a dedicated variable name for the credential, \
                 for example AXIOM_TELEGRAM_BOT_TOKEN"
            ),
        }
    }
}

impl std::error::Error for CredentialEnvNameError {}

/// Returns `Ok(())` when `name` is safe to read a credential from.
///
/// Syntax is POSIX-shell-compatible: an initial letter or underscore,
/// then letters, digits, and underscores. Reserved names are matched
/// case-insensitively, because the environment on Windows is
/// case-insensitive and a name that collides on one platform should not
/// silently be allowed on another.
pub fn validate_credential_env_name(name: &str) -> Result<(), CredentialEnvNameError> {
    let mut characters = name.chars();
    let valid_syntax = characters
        .next()
        .is_some_and(|character| character == '_' || character.is_ascii_alphabetic())
        && characters.all(|character| character == '_' || character.is_ascii_alphanumeric());

    if !valid_syntax || name.len() > MAX_ENV_NAME_LEN {
        return Err(CredentialEnvNameError::InvalidSyntax {
            name: name.to_string(),
        });
    }
    if RESERVED_CREDENTIAL_ENV_NAMES
        .iter()
        .any(|candidate| name.eq_ignore_ascii_case(candidate))
    {
        return Err(CredentialEnvNameError::Reserved {
            name: name.to_string(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_dedicated_credential_names() {
        for name in [
            "TELEGRAM_BOT_TOKEN",
            "DISCORD_BOT_TOKEN",
            "OPENROUTER_API_KEY",
            "AXIOM_TELEGRAM_BOT_TOKEN",
            "_PRIVATE",
            "a",
            "MCP_GITHUB_TOKEN",
        ] {
            assert!(validate_credential_env_name(name).is_ok(), "{name}");
        }
    }

    #[test]
    fn rejects_reserved_names_case_insensitively() {
        // These are the names the config-time check used to accept because it
        // compared case-sensitively against a shorter list.
        for name in ["TEMP", "COMSPEC", "RUST_LOG", "NODE_OPTIONS", "PATHEXT"] {
            assert!(
                matches!(
                    validate_credential_env_name(name),
                    Err(CredentialEnvNameError::Reserved { .. })
                ),
                "{name} should be reserved"
            );
        }
        for name in ["temp", "Path", "HOME"] {
            assert!(validate_credential_env_name(name).is_err(), "{name}");
        }
    }

    #[test]
    fn rejects_invalid_syntax() {
        for name in ["", "1TOKEN", "HAS-DASH", "HAS SPACE", "DOTS.HERE"] {
            assert!(
                matches!(
                    validate_credential_env_name(name),
                    Err(CredentialEnvNameError::InvalidSyntax { .. })
                ),
                "{name} should be invalid"
            );
        }
    }

    #[test]
    fn rejects_overlong_names() {
        let long = "A".repeat(129);
        assert!(validate_credential_env_name(&long).is_err());
        let ok = "A".repeat(128);
        assert!(validate_credential_env_name(&ok).is_ok());
    }
}
