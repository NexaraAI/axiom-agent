use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

use axiom_core::Workspace;
use serde::{Deserialize, Serialize};

use crate::test_runner::{detect_test_commands_for_files, TestCommand};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectType {
    Rust,
    Node,
    Python,
    Java,
    Generic,
}

impl std::fmt::Display for ProjectType {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let value = match self {
            Self::Rust => "rust",
            Self::Node => "node",
            Self::Python => "python",
            Self::Java => "java",
            Self::Generic => "generic",
        };
        formatter.write_str(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectScanSummary {
    pub root: String,
    pub project_type: ProjectType,
    pub files: Vec<String>,
    pub important_files: Vec<String>,
    pub ignored: Vec<String>,
    pub likely_test_commands: Vec<TestCommand>,
    pub likely_format_commands: Vec<String>,
}

pub fn scan_project(
    root: impl AsRef<Path>,
    max_depth: usize,
) -> std::io::Result<ProjectScanSummary> {
    let root = root.as_ref();
    let workspace = Workspace::new(root).map_err(std::io::Error::other)?;
    fs::create_dir_all(workspace.root())?;

    let mut files = Vec::new();
    let mut ignored = BTreeSet::new();
    scan_dir(
        workspace.root(),
        workspace.root(),
        max_depth,
        0,
        &mut files,
        &mut ignored,
    )?;
    files.sort();

    let important_files = important_files(&files);
    let project_type = detect_project_type(&files);
    let likely_test_commands = detect_test_commands_for_files(&files);
    let likely_format_commands = detect_format_commands(&project_type, &files);

    Ok(ProjectScanSummary {
        root: workspace.root().display().to_string(),
        project_type,
        files,
        important_files,
        ignored: ignored.into_iter().collect(),
        likely_test_commands,
        likely_format_commands,
    })
}

fn scan_dir(
    scan_root: &Path,
    current: &Path,
    max_depth: usize,
    depth: usize,
    files: &mut Vec<String>,
    ignored: &mut BTreeSet<String>,
) -> std::io::Result<()> {
    if depth > max_depth {
        return Ok(());
    }

    for entry in fs::read_dir(current)? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if entry.file_type()?.is_dir() {
            if ignored_dir(&name) {
                ignored.insert(name);
                continue;
            }
            scan_dir(scan_root, &path, max_depth, depth + 1, files, ignored)?;
        } else if let Ok(relative) = path.strip_prefix(scan_root) {
            files.push(normalize_path(relative));
        }
    }

    Ok(())
}

fn detect_project_type(files: &[String]) -> ProjectType {
    if has_file(files, "Cargo.toml") {
        ProjectType::Rust
    } else if has_file(files, "package.json") {
        ProjectType::Node
    } else if has_any(files, &["pyproject.toml", "requirements.txt", "setup.py"]) {
        ProjectType::Python
    } else if has_any(files, &["pom.xml", "build.gradle"]) {
        ProjectType::Java
    } else {
        ProjectType::Generic
    }
}

fn important_files(files: &[String]) -> Vec<String> {
    const IMPORTANT: &[&str] = &[
        "Cargo.toml",
        "package.json",
        "pyproject.toml",
        "requirements.txt",
        "setup.py",
        "pom.xml",
        "build.gradle",
        "README.md",
        "readme.md",
    ];

    files
        .iter()
        .filter(|file| IMPORTANT.contains(&file.as_str()))
        .cloned()
        .collect()
}

fn detect_format_commands(project_type: &ProjectType, files: &[String]) -> Vec<String> {
    match project_type {
        ProjectType::Rust => vec!["cargo fmt".to_string()],
        ProjectType::Node if has_file(files, "package.json") => vec!["npm run format".to_string()],
        ProjectType::Python => vec!["python -m black .".to_string()],
        _ => Vec::new(),
    }
}

fn ignored_dir(name: &str) -> bool {
    matches!(
        name,
        ".git"
            | "node_modules"
            | "target"
            | "dist"
            | "build"
            | ".venv"
            | "venv"
            | "__pycache__"
            | ".next"
            | ".cache"
    )
}

fn has_file(files: &[String], name: &str) -> bool {
    files.iter().any(|file| file == name)
}

fn has_any(files: &[String], names: &[&str]) -> bool {
    names.iter().any(|name| has_file(files, name))
}

fn normalize_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

#[allow(dead_code)]
fn _pathbuf_for_tests(path: &str) -> PathBuf {
    PathBuf::from(path)
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    #[test]
    fn detects_rust_project_and_ignores_generated_directories() {
        let dir = unique_temp_dir();
        fs::create_dir_all(dir.join("src")).expect("src");
        fs::create_dir_all(dir.join("target")).expect("target");
        fs::write(dir.join("Cargo.toml"), "[package]\nname='x'").expect("cargo");
        fs::write(dir.join("src").join("main.rs"), "fn main() {}").expect("main");
        fs::write(dir.join("target").join("artifact"), "ignored").expect("artifact");

        let summary = scan_project(&dir, 4).expect("scan");

        assert_eq!(summary.project_type, ProjectType::Rust);
        assert!(summary.files.contains(&"Cargo.toml".to_string()));
        assert!(summary.ignored.contains(&"target".to_string()));
        assert!(summary
            .likely_test_commands
            .iter()
            .any(|command| command.command == "cargo test"));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn detects_node_and_python_project_types() {
        let node = unique_temp_dir();
        fs::create_dir_all(&node).expect("node dir");
        fs::write(node.join("package.json"), "{}").expect("package");
        assert_eq!(
            scan_project(&node, 2).expect("scan").project_type,
            ProjectType::Node
        );
        let _ = fs::remove_dir_all(node);

        let python = unique_temp_dir();
        fs::create_dir_all(&python).expect("python dir");
        fs::write(python.join("pyproject.toml"), "[project]").expect("pyproject");
        assert_eq!(
            scan_project(&python, 2).expect("scan").project_type,
            ProjectType::Python
        );
        let _ = fs::remove_dir_all(python);
    }

    fn unique_temp_dir() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        std::env::temp_dir().join(format!("axiom-coder-project-test-{nanos}"))
    }
}
