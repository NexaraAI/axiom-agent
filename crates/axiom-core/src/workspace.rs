use std::{
    fs,
    path::{Component, Path, PathBuf},
};

use crate::{AxiomError, Result};

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

    fn unique_temp_dir() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos();
        std::env::temp_dir().join(format!("axiom-core-workspace-test-{nanos}"))
    }
}
