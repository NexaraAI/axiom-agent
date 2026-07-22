use std::{
    fs,
    path::{Component, Path, PathBuf},
};

use crate::{AxiomError, Result};

/// Git exclusion pathspecs mirroring [`is_secret_path`] for commands that
/// would otherwise place file contents into child-process output.
pub const SECRET_GIT_PATHSPEC_EXCLUSIONS: &[&str] = &[
    ":(exclude,icase,glob)**/.env*",
    ":(exclude,icase,glob)**/.env*/**",
    ":(exclude,icase,glob)**/credentials.json",
    ":(exclude,icase,glob)**/credentials.json/**",
    ":(exclude,icase,glob)**/token.json",
    ":(exclude,icase,glob)**/token.json/**",
    ":(exclude,icase,glob)**/id_rsa",
    ":(exclude,icase,glob)**/id_rsa/**",
    ":(exclude,icase,glob)**/id_dsa",
    ":(exclude,icase,glob)**/id_dsa/**",
    ":(exclude,icase,glob)**/id_ecdsa",
    ":(exclude,icase,glob)**/id_ecdsa/**",
    ":(exclude,icase,glob)**/id_ed25519",
    ":(exclude,icase,glob)**/id_ed25519/**",
    ":(exclude,icase,glob)**/.npmrc",
    ":(exclude,icase,glob)**/.npmrc/**",
    ":(exclude,icase,glob)**/.pypirc",
    ":(exclude,icase,glob)**/.pypirc/**",
    ":(exclude,icase,glob)**/.netrc",
    ":(exclude,icase,glob)**/.netrc/**",
    ":(exclude,icase,glob)**/_netrc",
    ":(exclude,icase,glob)**/_netrc/**",
    ":(exclude,icase,glob)**/.git-credentials",
    ":(exclude,icase,glob)**/.git-credentials/**",
    ":(exclude,icase,glob)**/.vault-token",
    ":(exclude,icase,glob)**/.vault-token/**",
    ":(exclude,icase,glob)**/.pgpass",
    ":(exclude,icase,glob)**/.pgpass/**",
    ":(exclude,icase,glob)**/.my.cnf",
    ":(exclude,icase,glob)**/.my.cnf/**",
    ":(exclude,icase,glob)**/auth.json",
    ":(exclude,icase,glob)**/auth.json/**",
    ":(exclude,icase,glob)**/application_default_credentials.json",
    ":(exclude,icase,glob)**/application_default_credentials.json/**",
    ":(exclude,icase,glob)**/*.pem",
    ":(exclude,icase,glob)**/*.pem/**",
    ":(exclude,icase,glob)**/*.key",
    ":(exclude,icase,glob)**/*.key/**",
    ":(exclude,icase,glob)**/*.p8",
    ":(exclude,icase,glob)**/*.p8/**",
    ":(exclude,icase,glob)**/*.p12",
    ":(exclude,icase,glob)**/*.p12/**",
    ":(exclude,icase,glob)**/*.pfx",
    ":(exclude,icase,glob)**/*.pfx/**",
    ":(exclude,icase,glob)**/*.pkcs12",
    ":(exclude,icase,glob)**/*.pkcs12/**",
    ":(exclude,icase,glob)**/*private_key*",
    ":(exclude,icase,glob)**/*private_key*/**",
    ":(exclude,icase,glob)**/*private-key*",
    ":(exclude,icase,glob)**/*private-key*/**",
    ":(exclude,icase,glob)**/service-account*.json",
    ":(exclude,icase,glob)**/service-account*.json/**",
    ":(exclude,icase,glob)**/service_account*.json",
    ":(exclude,icase,glob)**/service_account*.json/**",
    ":(exclude,icase,glob)**/client-secret*.json",
    ":(exclude,icase,glob)**/client-secret*.json/**",
    ":(exclude,icase,glob)**/client_secret*.json",
    ":(exclude,icase,glob)**/client_secret*.json/**",
    ":(exclude,icase,glob)**/.aws/credentials",
    ":(exclude,icase,glob)**/.aws/credentials/**",
    ":(exclude,icase,glob)**/.docker/config.json",
    ":(exclude,icase,glob)**/.docker/config.json/**",
];

/// Returns whether a path names a file that commonly contains credentials or
/// private key material.
///
/// Both slash styles are recognized so callers can validate untrusted model
/// output before the host platform interprets it. Callers that resolve paths
/// through [`Workspace`] must check both the supplied path and the resolved
/// path, because a benign-looking symlink can resolve to a secret file.
pub fn is_secret_path(path: impl AsRef<Path>) -> bool {
    let normalized = path
        .as_ref()
        .as_os_str()
        .to_string_lossy()
        .replace('\\', "/")
        .to_ascii_lowercase();
    let components = normalized
        .trim_end_matches('/')
        .split('/')
        .filter(|component| !component.is_empty())
        .map(normalize_secret_component)
        .collect::<Vec<_>>();
    if components.is_empty() {
        return false;
    }

    let blocked_scoped_credentials = components.windows(2).any(|pair| {
        (pair[0] == ".aws" && pair[1] == "credentials")
            || (pair[0] == ".docker" && pair[1] == "config.json")
    });

    blocked_scoped_credentials
        || components
            .iter()
            .any(|component| is_secret_component(component))
}

fn normalize_secret_component(component: &str) -> String {
    // Treat NTFS alternate-data-stream and Win32 trailing-dot/space spellings
    // as the underlying name. On other platforms this is intentionally
    // conservative for credential-like names containing those characters.
    component
        .split(':')
        .next()
        .unwrap_or(component)
        .trim_end_matches(['.', ' '])
        .to_string()
}

fn is_secret_component(file_name: &str) -> bool {
    let blocked_name = matches!(
        file_name,
        ".env"
            | "credentials.json"
            | "token.json"
            | "id_rsa"
            | "id_dsa"
            | "id_ecdsa"
            | "id_ed25519"
            | ".npmrc"
            | ".pypirc"
            | ".netrc"
            | "_netrc"
            | ".git-credentials"
            | ".vault-token"
            | ".pgpass"
            | ".my.cnf"
            | "auth.json"
            | "application_default_credentials.json"
    );
    let blocked_env_variant = file_name.starts_with(".env.");
    let blocked_key_material = [".pem", ".key", ".p8", ".p12", ".pfx", ".pkcs12"]
        .iter()
        .any(|extension| file_name.ends_with(extension));
    let blocked_private_key_name =
        file_name.contains("private_key") || file_name.contains("private-key");
    let blocked_service_account = json_name_family(file_name, "service-account")
        || json_name_family(file_name, "service_account")
        || json_name_family(file_name, "client-secret")
        || json_name_family(file_name, "client_secret");

    blocked_name
        || blocked_env_variant
        || blocked_key_material
        || blocked_private_key_name
        || blocked_service_account
}

fn json_name_family(file_name: &str, stem: &str) -> bool {
    let Some(name) = file_name.strip_suffix(".json") else {
        return false;
    };
    name == stem
        || name
            .strip_prefix(stem)
            .is_some_and(|suffix| suffix.starts_with(['-', '_']))
}

#[derive(Debug, Clone)]
pub struct Workspace {
    root: PathBuf,
}

impl Workspace {
    pub fn new(root: impl AsRef<Path>) -> Result<Self> {
        fs::create_dir_all(root.as_ref())?;
        let root = fs::canonicalize(root)?;
        Ok(Self { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn resolve_inside(&self, path: impl AsRef<Path>) -> Result<PathBuf> {
        let raw_path = path.as_ref();
        let joined = if raw_path.is_absolute() {
            raw_path.to_path_buf()
        } else {
            self.root.join(raw_path)
        };

        let normalized = normalize_lexically(&joined)?;
        if !normalized.starts_with(&self.root) {
            return Err(AxiomError::UnsafeWorkspacePath { path: normalized });
        }

        let existing_ancestor = nearest_existing_ancestor(&normalized);
        let canonical_ancestor = fs::canonicalize(&existing_ancestor)?;
        if !canonical_ancestor.starts_with(&self.root) {
            return Err(AxiomError::UnsafeWorkspacePath {
                path: canonical_ancestor,
            });
        }

        let missing_suffix =
            normalized
                .strip_prefix(&existing_ancestor)
                .map_err(|_| AxiomError::InvalidPath {
                    path: normalized.clone(),
                })?;

        if missing_suffix.as_os_str().is_empty() {
            Ok(canonical_ancestor)
        } else {
            Ok(canonical_ancestor.join(missing_suffix))
        }
    }

    pub fn contains(&self, path: impl AsRef<Path>) -> bool {
        self.resolve_inside(path).is_ok()
    }
}

fn nearest_existing_ancestor(path: &Path) -> PathBuf {
    let mut current = path;
    loop {
        if current.exists() {
            return current.to_path_buf();
        }
        match current.parent() {
            Some(parent) => current = parent,
            None => return PathBuf::from("."),
        }
    }
}

fn normalize_lexically(path: &Path) -> Result<PathBuf> {
    let mut normalized = PathBuf::new();

    for component in path.components() {
        match component {
            Component::Prefix(_) | Component::RootDir | Component::Normal(_) => {
                normalized.push(component.as_os_str());
            }
            Component::CurDir => {}
            Component::ParentDir => {
                if !normalized.pop() {
                    return Err(AxiomError::InvalidPath {
                        path: path.to_path_buf(),
                    });
                }
            }
        }
    }

    Ok(normalized)
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    #[test]
    fn classifies_secret_names_portably() {
        for path in [
            ".env",
            ".ENV.production",
            r"nested\.env.local",
            "nested/credentials.json",
            r"nested\TOKEN.JSON",
            r"nested\.env::$DATA",
            "credentials.json:backup",
            "token.json. ",
            "id_rsa",
            "keys/id_dsa",
            "keys/id_ecdsa",
            "keys/id_ed25519",
            "certificates/client.pem",
            r"certificates\signing.KEY",
            "keys/signing.p12",
            "keys/my-private-key.txt",
            "keys/my_private_key",
            ".npmrc",
            ".pypirc",
            ".netrc",
            "_netrc",
            ".git-credentials",
            ".vault-token",
            ".pgpass",
            ".my.cnf",
            "auth.json",
            "application_default_credentials.json",
            "service-account-production.json",
            "service_account_dev.json",
            "client_secret_123.json",
            ".aws/credentials",
            r".docker\config.json",
            ".env/notes.txt",
            "nested/KEYS.PEM/notes.txt",
            "nested/.AWS/CREDENTIALS/notes.txt",
        ] {
            assert!(is_secret_path(path), "expected secret path: {path}");
        }
    }

    #[test]
    fn allows_similarly_named_non_secret_files() {
        for path in [
            "README.md",
            ".envrc",
            "environment.md",
            "id_rsa.pub",
            "credentials.example.json",
            "tokenizer.json",
            "config.json",
            "docker/config.json",
            ".aws/config",
            "service-accounting.json",
            "client_secretary.json",
            "docs/key-management.md",
        ] {
            assert!(!is_secret_path(path), "unexpected secret path: {path}");
        }
    }

    #[test]
    fn resolves_relative_paths_inside_workspace() {
        let root = unique_temp_dir();
        let workspace = Workspace::new(&root).expect("workspace");

        let resolved = workspace
            .resolve_inside(Path::new("src").join("main.rs"))
            .expect("resolve inside");

        assert!(resolved.starts_with(workspace.root()));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn rejects_parent_traversal_outside_workspace() {
        let root = unique_temp_dir();
        let workspace = Workspace::new(&root).expect("workspace");

        let error = workspace
            .resolve_inside("../outside.txt")
            .expect_err("outside path should fail");

        assert!(matches!(error, AxiomError::UnsafeWorkspacePath { .. }));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn rejects_absolute_paths_outside_workspace() {
        let root = unique_temp_dir();
        let outside = unique_temp_dir();
        fs::create_dir_all(&outside).expect("outside dir");
        let workspace = Workspace::new(&root).expect("workspace");

        let error = workspace
            .resolve_inside(outside.join("file.txt"))
            .expect_err("outside absolute path should fail");

        assert!(matches!(error, AxiomError::UnsafeWorkspacePath { .. }));
        let _ = fs::remove_dir_all(root);
        let _ = fs::remove_dir_all(outside);
    }

    #[test]
    fn generated_relative_paths_never_resolve_outside_workspace() {
        let root = unique_temp_dir();
        let workspace = Workspace::new(&root).expect("workspace");
        let components = ["src", "nested", ".", "..", "target", "file.txt"];
        let mut state = 0x9e37_79b9_u32;

        for _ in 0..4_096 {
            let mut candidate = PathBuf::new();
            for _ in 0..6 {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                candidate.push(components[state as usize % components.len()]);
            }
            if let Ok(resolved) = workspace.resolve_inside(&candidate) {
                assert!(
                    resolved.starts_with(workspace.root()),
                    "path escaped workspace: {} -> {}",
                    candidate.display(),
                    resolved.display()
                );
            }
        }

        let _ = fs::remove_dir_all(root);
    }

    fn unique_temp_dir() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos();
        std::env::temp_dir().join(format!("axiom-core-workspace-test-{nanos}"))
    }
}
