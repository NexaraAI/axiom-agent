use std::{collections::BTreeSet, process::Command};

use anyhow::{Context, Result};
use axiom_core::{AxiomConfig, ProviderConfig};

const KEYRING_SERVICE: &str = "nexara-ai-axiom";

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

/// Resolve one configured provider credential without copying a keyring value
/// into the process environment. Environment variables remain the explicit
/// headless override; otherwise the native credential manager is consulted.
pub(crate) fn resolve_credential(environment_variable: &str) -> Result<Option<String>> {
    let credential = resolve_with_store(environment_variable, &OsCredentialStore)?;
    if let Some(secret) = credential.as_deref() {
        axiom_proof::register_secret_for_redaction(secret);
    }
    Ok(credential)
}

/// Return every credential environment name referenced by the configuration.
/// The complete set is scrubbed from child processes, not only the active
/// provider, because a model-controlled command must not learn dormant keys.
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
    Ok(names.into_iter().collect())
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
        println!("No credential saved. Set {environment_variable} before using this provider.");
        return Ok(false);
    }

    match store_credential(environment_variable, &secret) {
        Ok(()) => {
            println!("Saved {environment_variable} in the OS credential manager.");
            Ok(true)
        }
        Err(error) => {
            println!(
                "Could not persist the credential ({error}). Set {environment_variable} in the environment and rerun setup."
            );
            Ok(false)
        }
    }
}

pub(crate) fn store_credential(environment_variable: &str, secret: &str) -> Result<()> {
    axiom_llm::validate_credential_env_name(environment_variable)?;
    OsCredentialStore.set(environment_variable, secret)
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
    store.get(environment_variable)
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
