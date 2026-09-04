use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    process::Command,
};

use anyhow::{Context, Result};
use axiom_core::{atomic_write, AxiomConfig, ProviderConfig};

const KEYRING_SERVICE: &str = "nexara-ai-axiom";
const FILE_STORE_NAME: &str = "credentials.env";

trait CredentialStore {
    fn get(&self, environment_variable: &str) -> Result<Option<String>>;
    fn set(&self, environment_variable: &str, secret: &str) -> Result<()>;
}

struct OsCredentialStore;

impl CredentialStore for OsCredentialStore {
    fn get(&self, environment_variable: &str) -> Result<Option<String>> {
        match keyring_entry(environment_variable)?.get_password() {
            Ok(secret) if !secret.trim().is_empty() => Ok(Some(secret)),
            Ok(_) | Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(error).with_context(|| {
                format!("could not read {environment_variable} from the OS credential manager")
            }),
        }
    }

    fn set(&self, environment_variable: &str, secret: &str) -> Result<()> {
        keyring_entry(environment_variable)?
            .set_password(secret)
            .with_context(|| {
                format!(
                    "could not store {environment_variable} in the OS credential manager; set it as an environment variable instead"
                )
            })
    }
}

pub(crate) fn resolve_credential(environment_variable: &str) -> Result<Option<String>> {
    let credential = resolve_with_store(environment_variable, &OsCredentialStore)?;
    if let Some(secret) = credential.as_deref() {
        axiom_proof::register_secret_for_redaction(secret);
    }
    Ok(credential)
}

pub(crate) fn credential_environment_names(config: &AxiomConfig) -> Result<Vec<String>> {
    let mut names = BTreeSet::new();
    for provider in config.providers.values() {
        let environment_variable = match provider {
            ProviderConfig::OpenaiCompatible {
                api_key_env: Some(environment_variable),
                ..
            } => Some(environment_variable.as_str()),
            ProviderConfig::CloudflareAiGateway { api_token_env, .. } => {
                Some(api_token_env.as_str())
            }
            _ => None,
        };
        if let Some(environment_variable) = environment_variable {
            axiom_llm::validate_credential_env_name(environment_variable)?;
            names.insert(environment_variable.to_string());
        }
    }
    for gateway_variable in [
        config.gateway.telegram_bot_token_env.as_deref(),
        config.gateway.discord_bot_token_env.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        axiom_llm::validate_credential_env_name(gateway_variable)?;
        names.insert(gateway_variable.to_string());
    }
    Ok(names.into_iter().collect())
}

fn credentials_file_path() -> Result<PathBuf> {
    Ok(AxiomConfig::default_config_dir()?.join(FILE_STORE_NAME))
}

fn read_file_credential(environment_variable: &str) -> Result<Option<String>> {
    let path = credentials_file_path()?;
    if !path.exists() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(&path)?;
    Ok(parse_credentials_file(&content).remove(environment_variable))
}

fn parse_credentials_file(content: &str) -> BTreeMap<String, String> {
    let mut values = BTreeMap::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((name, value)) = line.split_once('=') else {
            continue;
        };
        let name = name.trim();
        if name.is_empty()
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        {
            continue;
        }
        if !value.trim().is_empty() {
            values.insert(name.to_string(), value.trim().to_string());
        }
    }
    values
}

fn write_file_credential(environment_variable: &str, secret: &str) -> Result<PathBuf> {
    let path = credentials_file_path()?;
    let mut values = if path.exists() {
        parse_credentials_file(&std::fs::read_to_string(&path)?)
    } else {
        BTreeMap::new()
    };
    values.insert(environment_variable.to_string(), secret.to_string());
    let mut content = String::from("# Axiom local credential fallback (0600). Prefer env vars or the OS keychain.\n");
    for (name, value) in &values {
        content.push_str(name);
        content.push('=');
        content.push_str(value);
        content.push('\n');
    }
    atomic_write(&path, content.as_bytes())?;
    Ok(path)
}

/// Removes a variable from the local fallback file (used when tokens are
/// revoked via `axiom gateway disable`). Returns true if something was removed.
pub(crate) fn forget_credential(environment_variable: &str) -> Result<bool> {
    let path = credentials_file_path()?;
    if !path.exists() {
        return Ok(false);
    }
    let mut values = parse_credentials_file(&std::fs::read_to_string(&path)?);
    if values.remove(environment_variable).is_none() {
        return Ok(false);
    }
    let mut content = String::from("# Axiom local credential fallback (0600). Prefer env vars or the OS keychain.\n");
    for (name, value) in &values {
        content.push_str(name);
        content.push('=');
        content.push_str(value);
        content.push('\n');
    }
    atomic_write(&path, content.as_bytes())?;
    Ok(true)
}

pub(crate) fn scrub_provider_credentials(
    command: &mut Command,
    config: &AxiomConfig,
) -> Result<()> {
    scrub_credential_names(command, &credential_environment_names(config)?);
    Ok(())
}

pub(crate) fn scrub_credential_names(command: &mut Command, names: &[String]) {
    for name in names {
        command.env_remove(name);
    }
}

/// If an error message names a credential variable (e.g. `NVIDIA_API_KEY`),
/// return a copy-paste fix for it. Used to turn keychain failures on headless
/// machines into actionable chat/onboarding output instead of a dead end.
pub(crate) fn credential_hint_for_error(error_text: &str) -> Option<String> {
    let var = error_text
        .split(|character: char| !(character.is_ascii_uppercase() || character == '_' || character.is_ascii_digit()))
        .filter(|token| {
            token.len() >= 8
                && (token.ends_with("_KEY")
                    || token.ends_with("_TOKEN")
                    || token.ends_with("_SECRET"))
        })
        .next()?;
    Some(format(
        "Fix: export {var}='paste-your-key-here' in this terminal, then retry. \
         (No OS keychain here, so env vars are the way.) \
         Get a key from your provider dashboard, or run `axiom onboarding` to switch provider."
     ))
}
pub(crate) fn prompt_for_credential(environment_variable: &str) -> Result<bool> {
    axiom_llm::validate_credential_env_name(environment_variable)?;
    if std::env::var(environment_variable).is_ok_and(|value| !value.trim().is_empty()) {
        println!("Using {environment_variable} from the current environment.");
        return Ok(true);
    }
    match OsCredentialStore.get(environment_variable) {
        Ok(Some(_)) => {
            println!("Using {environment_variable} from the OS credential manager.");
            return Ok(true);
        }
        Ok(None) => {}
        Err(error) => println!("Credential manager unavailable: {error}"),
    }

    let secret = rpassword::prompt_password(format!(
        "Paste the API key/token for {environment_variable} (hidden, blank to configure later): "
    ))?;
    if secret.trim().is_empty() {
        println!("No credential saved. Before chatting, run this in your terminal:");
        println!("  export {environment_variable}='paste-your-key-here'");
        println!("Then retry. (Or re-run `axiom onboarding` to pick another provider.)");
        return Ok(false);
    }

    match store_credential(environment_variable, &secret) {
        Ok(true) => {
            println!("Saved {environment_variable} in the OS credential manager.");
            Ok(true)
        }
        Ok(false) => {
            println!("Saved {environment_variable} locally (no keychain here, so it went to");
            println!("  {} (private 0600 file, this machine only).", credentials_file_path()?.display());
            println!("Chat will pick it up automatically — no export needed.");
            Ok(true)
        }
        Err(error) => {
            println!("Could not save the credential ({error}). As a last resort, export it each session:");
            println!("  export {environment_variable}='paste-your-key-here'");
            Ok(false)
        }
    }
}

/// Stores a secret in the OS keychain when available, otherwise in the
/// private local fallback file. Returns `true` for keychain, `false` for the
/// local file. Pasted keys are never silently dropped.
pub(crate) fn store_credential(environment_variable: &str, secret: &str) -> Result<bool> {
    axiom_llm::validate_credential_env_name(environment_variable)?;
    match OsCredentialStore.set(environment_variable, secret) {
        Ok(()) => Ok(true),
        Err(_) => {
            write_file_credential(environment_variable, secret)?;
            Ok(false)
        }
    }
}

fn resolve_with_store(
    environment_variable: &str,
    store: &dyn CredentialStore,
) -> Result<Option<String>> {
    axiom_llm::validate_credential_env_name(environment_variable)?;
    if let Ok(value) = std::env::var(environment_variable) {
        if !value.trim().is_empty() {
            return Ok(Some(value));
        }
    }
    match store.get(environment_variable) {
        Ok(Some(secret)) => Ok(Some(secret)),
        Ok(None) => Ok(read_file_credential(environment_variable)?),
        Err(keyring_error) => match read_file_credential(environment_variable)? {
            Some(secret) => Ok(Some(secret)),
            None => Err(keyring_error),
        },
    }
}

fn keyring_entry(environment_variable: &str) -> Result<keyring::Entry> {
    keyring::Entry::new(KEYRING_SERVICE, environment_variable).with_context(|| {
        format!("could not open the OS credential entry for {environment_variable}")
    })
}

#[cfg(test)]
mod tests {
    use std::{cell::Cell, ffi::OsString};

    use anyhow::bail;

    use super::*;

    struct FakeStore {
        value: Option<String>,
        fail: bool,
        reads: Cell<usize>,
    }

    impl CredentialStore for FakeStore {
        fn get(&self, _environment_variable: &str) -> Result<Option<String>> {
            self.reads.set(self.reads.get() + 1);
            if self.fail {
                bail!("credential backend unavailable")
            }
            Ok(self.value.clone())
        }

        fn set(&self, _environment_variable: &str, _secret: &str) -> Result<()> {
            if self.fail {
                bail!("credential backend unavailable")
            }
            Ok(())
        }
    }

    #[test]
    fn resolves_from_store_without_exporting_and_existing_environment_wins() {
        let key = "AXIOM_TEST_KEYRING_RESOLVE";
        let _guard = EnvGuard::remove(key);
        let store = FakeStore {
            value: Some("stored-secret".to_string()),
            fail: false,
            reads: Cell::new(0),
        };
        assert_eq!(
            resolve_with_store(key, &store).expect("resolve"),
            Some("stored-secret".to_string())
        );
        assert!(std::env::var_os(key).is_none());
        assert_eq!(store.reads.get(), 1);

        std::env::set_var(key, "environment-secret");
        assert_eq!(
            resolve_with_store(key, &store).expect("existing env"),
            Some("environment-secret".to_string())
        );
        assert_eq!(store.reads.get(), 1);
    }

    #[test]
    fn unavailable_store_is_actionable_and_does_not_set_environment() {
        let key = "AXIOM_TEST_KEYRING_UNAVAILABLE";
        let _guard = EnvGuard::remove(key);
        let store = FakeStore {
            value: None,
            fail: true,
            reads: Cell::new(0),
        };
        let error = resolve_with_store(key, &store).expect_err("unavailable");
        assert!(error.to_string().contains("credential backend unavailable"));
        assert!(std::env::var_os(key).is_none());
    }

    #[test]
    fn unsafe_environment_name_is_rejected_before_the_store_is_read() {
        let store = FakeStore {
            value: Some("secret".to_string()),
            fail: false,
            reads: Cell::new(0),
        };
        let error = resolve_with_store("PATH", &store).expect_err("PATH must be reserved");
        assert!(error.to_string().contains("unsafe API key/token"));
        assert_eq!(store.reads.get(), 0);
    }

    #[test]
    fn configured_exported_and_keyring_credentials_are_not_visible_to_children() {
        let key = "AXIOM_TEST_CHILD_PROVIDER_SECRET_68CCB21A";
        let _guard = EnvGuard::remove(key);
        let store = FakeStore {
            value: Some("keyring-secret".to_string()),
            fail: false,
            reads: Cell::new(0),
        };

        let keyring_secret = resolve_with_store(key, &store)
            .expect("keyring credential resolves")
            .expect("stored secret");
        assert_eq!(keyring_secret, "keyring-secret");
        assert!(std::env::var_os(key).is_none());

        std::env::set_var(key, "exported-secret");
        let mut config = AxiomConfig::default();
        config.providers.clear();
        config.providers.insert(
            "test".to_string(),
            ProviderConfig::OpenaiCompatible {
                base_url: "https://example.test/v1".to_string(),
                api_key_env: Some(key.to_string()),
                models_url: None,
            },
        );
        let mut child = credential_probe_command(key);
        scrub_provider_credentials(&mut child, &config).expect("scrub configured credentials");
        let status = child.status().expect("run credential probe");
        assert!(status.success(), "child observed a provider credential");
    }

    #[cfg(windows)]
    fn credential_probe_command(key: &str) -> Command {
        let mut command = Command::new("cmd");
        command.args([
            "/D",
            "/S",
            "/C",
            &format!("if defined {key} (exit /b 7) else (exit /b 0)"),
        ]);
        command
    }

    #[cfg(not(windows))]
    fn credential_probe_command(key: &str) -> Command {
        let mut command = Command::new("sh");
        command.arg("-c").arg(format!("test -z \"${{{key}:-}}\""));
        command
    }

    struct EnvGuard {
        key: &'static str,
        previous: Option<OsString>,
    }

    impl EnvGuard {
        fn remove(key: &'static str) -> Self {
            let previous = std::env::var_os(key);
            std::env::remove_var(key);
            Self { key, previous }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            if let Some(previous) = self.previous.as_ref() {
                std::env::set_var(self.key, previous);
            } else {
                std::env::remove_var(self.key);
            }
        }
    }
}
