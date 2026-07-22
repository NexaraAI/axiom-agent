use std::{collections::BTreeSet, fs, path::Path};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestCommand {
    pub command: String,
    pub reason: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub working_directory: Option<String>,
}

const DETECTION_FILES: &[&str] = &[
    "Cargo.toml",
    "package.json",
    "pnpm-lock.yaml",
    "yarn.lock",
    "pyproject.toml",
    "requirements.txt",
    "setup.py",
    "pytest.ini",
    "go.mod",
    "pom.xml",
    "build.gradle",
    "build.gradle.kts",
    "deno.json",
    "deno.jsonc",
    "bun.lockb",
];

pub fn detect_test_commands(root: impl AsRef<Path>) -> std::io::Result<Vec<TestCommand>> {
    let root = root.as_ref();
    let files = discover_detection_files(root, 4)?;
    let mut commands = detect_test_commands_for_files(&files);
    for command in &mut commands {
        if command.command != "npm test" {
            continue;
        }
        let package_path = command.working_directory.as_deref().map_or_else(
            || root.join("package.json"),
            |dir| root.join(dir).join("package.json"),
        );
        if package_path.is_file() {
            if let Some(script) = package_test_script(package_path)? {
                let scope = command
                    .working_directory
                    .as_deref()
                    .map(|dir| format!(" in workspace package `{dir}`"))
                    .unwrap_or_default();
                let reason = format!("package.json test script detected: {script}");
                command.reason = format!("{reason}{scope}");
            }
        }
    }
    Ok(commands)
}

pub fn detect_test_commands_for_files(files: &[String]) -> Vec<TestCommand> {
    let mut commands = Vec::new();

    for directory in project_directories(files, &["Cargo.toml"]) {
        add_scoped_command(
            &mut commands,
            "cargo test",
            "Rust project detected from Cargo.toml",
            directory,
        );
    }

    for directory in project_directories(files, &["package.json"]) {
        add_scoped_command(
            &mut commands,
            "npm test",
            "Node project detected from package.json",
            directory.clone(),
        );
        if file_applies_to_directory(files, "pnpm-lock.yaml", directory.as_deref()) {
            add_scoped_command(
                &mut commands,
                "pnpm test",
                "pnpm lockfile detected",
                directory.clone(),
            );
        }
        if file_applies_to_directory(files, "yarn.lock", directory.as_deref()) {
            add_scoped_command(
                &mut commands,
                "yarn test",
                "Yarn lockfile detected",
                directory,
            );
        }
    }

    for directory in project_directories(
        files,
        &[
            "pyproject.toml",
            "requirements.txt",
            "setup.py",
            "pytest.ini",
        ],
    ) {
        add_scoped_command(
            &mut commands,
            "python -m pytest",
            "Python project detected",
            directory.clone(),
        );
        if file_applies_to_directory(files, "pytest.ini", directory.as_deref()) {
            add_scoped_command(
                &mut commands,
                "pytest",
                "pytest configuration detected",
                directory,
            );
        }
    }

    for directory in project_directories(files, &["go.mod"]) {
        add_scoped_command(
            &mut commands,
            "go test ./...",
            "Go project detected from go.mod",
            directory,
        );
    }

    for directory in project_directories(files, &["pom.xml"]) {
        add_scoped_command(
            &mut commands,
            "mvn test",
            "Maven project detected from pom.xml",
            directory,
        );
    }

    for directory in project_directories(files, &["build.gradle", "build.gradle.kts"]) {
        add_scoped_command(
            &mut commands,
            "gradle test",
            "Gradle build file detected",
            directory,
        );
    }

    for directory in project_directories(files, &["deno.json", "deno.jsonc"]) {
        add_scoped_command(
            &mut commands,
            "deno test",
            "Deno configuration detected",
            directory,
        );
    }

    for directory in project_directories(files, &["bun.lockb"]) {
        add_scoped_command(
            &mut commands,
            "bun test",
            "Bun lockfile detected",
            directory,
        );
    }

    commands
}

fn package_test_script(path: impl AsRef<Path>) -> std::io::Result<Option<String>> {
    let content = fs::read_to_string(path)?;
    let package: serde_json::Value = match serde_json::from_str(&content) {
        Ok(package) => package,
        Err(_) => return Ok(None),
    };
    Ok(package["scripts"]["test"]
        .as_str()
        .map(str::trim)
        .filter(|script| !script.is_empty())
        .map(ToString::to_string))
}

fn add_scoped_command(
    commands: &mut Vec<TestCommand>,
    command: &str,
    reason: &str,
    working_directory: Option<String>,
) {
    let reason = working_directory
        .as_deref()
        .map(|dir| format!("{reason} in workspace package `{dir}`"))
        .unwrap_or_else(|| reason.to_string());
    if commands.iter().any(|candidate| {
        candidate.command == command && candidate.working_directory == working_directory
    }) {
        return;
    }
    commands.push(TestCommand {
        command: command.to_string(),
        reason,
        working_directory,
    });
}

fn project_directories(files: &[String], names: &[&str]) -> Vec<Option<String>> {
    let directories = files
        .iter()
        .filter_map(|file| safe_file_parts(file))
        .filter(|parts| {
            parts
                .last()
                .is_some_and(|name| names.contains(&name.as_str()))
        })
        .map(|parts| directory_from_parts(&parts))
        .collect::<BTreeSet<_>>();

    directories.into_iter().collect()
}

fn file_applies_to_directory(files: &[String], name: &str, directory: Option<&str>) -> bool {
    files
        .iter()
        .filter_map(|file| safe_file_parts(file))
        .any(|parts| {
            if parts.last().map(String::as_str) != Some(name) {
                return false;
            }
            let file_directory = directory_from_parts(&parts);
            match (file_directory.as_deref(), directory) {
                (None, _) => true,
                (Some(file_dir), Some(project_dir)) => {
                    file_dir == project_dir
                        || project_dir
                            .strip_prefix(file_dir)
                            .is_some_and(|suffix| suffix.starts_with('/'))
                }
                (Some(_), None) => false,
            }
        })
}

fn safe_file_parts(file: &str) -> Option<Vec<String>> {
    let normalized = file.replace('\\', "/");
    if normalized.starts_with('/')
        || normalized.as_bytes().get(1) == Some(&b':')
        || normalized.is_empty()
    {
        return None;
    }
    let parts = normalized
        .split('/')
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    if parts.iter().any(|part| {
        part.is_empty() || part == "." || part == ".." || part.chars().any(char::is_control)
    }) {
        return None;
    }
    Some(parts)
}

fn directory_from_parts(parts: &[String]) -> Option<String> {
    (parts.len() > 1).then(|| parts[..parts.len() - 1].join("/"))
}

fn discover_detection_files(root: &Path, max_depth: usize) -> std::io::Result<Vec<String>> {
    if !root.is_dir() {
        return Ok(Vec::new());
    }
    let mut files = Vec::new();
    collect_detection_files(root, root, 0, max_depth, &mut files)?;
    files.sort();
    Ok(files)
}

fn collect_detection_files(
    root: &Path,
    current: &Path,
    depth: usize,
    max_depth: usize,
    files: &mut Vec<String>,
) -> std::io::Result<()> {
    if depth > max_depth {
        return Ok(());
    }
    let mut entries = fs::read_dir(current)?.collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let file_type = entry.file_type()?;
        let name = entry.file_name().to_string_lossy().to_string();
        if file_type.is_dir() {
            if !ignored_directory(&name) {
                collect_detection_files(root, &entry.path(), depth + 1, max_depth, files)?;
            }
        } else if file_type.is_file() && DETECTION_FILES.contains(&name.as_str()) {
            if let Ok(relative) = entry.path().strip_prefix(root) {
                files.push(normalize_path(relative));
            }
        }
    }
    Ok(())
}

fn ignored_directory(name: &str) -> bool {
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

fn normalize_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    #[test]
    fn detects_rust_test_command() {
        let commands = detect_test_commands_for_files(&["Cargo.toml".to_string()]);

        assert_eq!(commands[0].command, "cargo test");
        assert_eq!(commands[0].working_directory, None);
    }

    #[test]
    fn detects_node_test_commands() {
        let commands = detect_test_commands_for_files(&[
            "package.json".to_string(),
            "pnpm-lock.yaml".to_string(),
            "yarn.lock".to_string(),
        ]);

        assert!(commands.iter().any(|command| command.command == "npm test"));
        assert!(commands
            .iter()
            .any(|command| command.command == "pnpm test"));
        assert!(commands
            .iter()
            .any(|command| command.command == "yarn test"));
    }

    #[test]
    fn detects_python_test_commands() {
        let commands = detect_test_commands_for_files(&[
            "pyproject.toml".to_string(),
            "pytest.ini".to_string(),
        ]);

        assert!(commands
            .iter()
            .any(|command| command.command == "python -m pytest"));
        assert!(commands.iter().any(|command| command.command == "pytest"));
    }

    #[test]
    fn detects_go_java_deno_and_bun_test_commands() {
        let commands = detect_test_commands_for_files(&[
            "go.mod".to_string(),
            "pom.xml".to_string(),
            "build.gradle.kts".to_string(),
            "deno.json".to_string(),
            "bun.lockb".to_string(),
        ]);

        for expected in [
            "go test ./...",
            "mvn test",
            "gradle test",
            "deno test",
            "bun test",
        ] {
            assert!(commands.iter().any(|command| command.command == expected));
        }
    }

    #[test]
    fn package_json_test_script_enriches_the_detection_reason() {
        let dir = unique_temp_dir();
        fs::create_dir_all(&dir).expect("create project");
        fs::write(
            dir.join("package.json"),
            r#"{"scripts":{"test":"vitest run"}}"#,
        )
        .expect("write package");

        let commands = detect_test_commands(&dir).expect("detect tests");

        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].command, "npm test");
        assert!(commands[0].reason.contains("vitest run"));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn scopes_nested_manifests_to_their_workspace_packages() {
        let commands = detect_test_commands_for_files(&[
            "pnpm-lock.yaml".to_string(),
            "packages/web/package.json".to_string(),
            "crates/worker/Cargo.toml".to_string(),
            "services/api/go.mod".to_string(),
        ]);

        assert!(has_scoped_command(&commands, "npm test", "packages/web"));
        assert!(has_scoped_command(&commands, "pnpm test", "packages/web"));
        assert!(has_scoped_command(&commands, "cargo test", "crates/worker"));
        assert!(has_scoped_command(
            &commands,
            "go test ./...",
            "services/api"
        ));
    }

    #[test]
    fn root_manifest_keeps_existing_command_and_adds_nested_package() {
        let commands = detect_test_commands_for_files(&[
            "Cargo.toml".to_string(),
            "crates/worker/Cargo.toml".to_string(),
        ]);

        assert_eq!(commands.len(), 2);
        assert_eq!(commands[0].command, "cargo test");
        assert_eq!(commands[0].working_directory, None);
        assert!(has_scoped_command(&commands, "cargo test", "crates/worker"));
    }

    #[test]
    fn unsafe_or_absolute_manifest_paths_are_not_suggested() {
        let commands = detect_test_commands_for_files(&[
            "../outside/Cargo.toml".to_string(),
            "/tmp/project/package.json".to_string(),
            "C:/outside/go.mod".to_string(),
            "terminal\nspoof/Cargo.toml".to_string(),
            "safe\\crate\\Cargo.toml".to_string(),
        ]);

        assert_eq!(commands.len(), 1);
        assert!(has_scoped_command(&commands, "cargo test", "safe/crate"));
    }

    #[test]
    fn recursive_detection_reads_nested_script_and_ignores_generated_directories() {
        let dir = unique_temp_dir();
        fs::create_dir_all(dir.join("packages").join("web")).expect("nested package");
        fs::create_dir_all(dir.join("target").join("generated")).expect("generated dir");
        fs::write(
            dir.join("packages").join("web").join("package.json"),
            r#"{"scripts":{"test":"vitest run"}}"#,
        )
        .expect("package manifest");
        fs::write(
            dir.join("target").join("generated").join("Cargo.toml"),
            "[package]",
        )
        .expect("ignored manifest");

        let commands = detect_test_commands(&dir).expect("detect nested tests");

        assert_eq!(commands.len(), 1);
        assert!(has_scoped_command(&commands, "npm test", "packages/web"));
        assert!(commands[0].reason.contains("vitest run"));
        assert!(!commands
            .iter()
            .any(|command| command.command == "cargo test"));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn legacy_serialized_test_command_defaults_to_workspace_root() {
        let command: TestCommand =
            serde_json::from_str(r#"{"command":"cargo test","reason":"Rust project detected"}"#)
                .expect("legacy test command");

        assert_eq!(command.working_directory, None);
    }

    fn has_scoped_command(commands: &[TestCommand], command: &str, directory: &str) -> bool {
        commands.iter().any(|candidate| {
            candidate.command == command
                && candidate.working_directory.as_deref() == Some(directory)
        })
    }

    fn unique_temp_dir() -> PathBuf {
        std::env::temp_dir().join(format!(
            "axiom-coder-test-runner-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time")
                .as_nanos()
        ))
    }
}
