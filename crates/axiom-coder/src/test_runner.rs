use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestCommand {
    pub command: String,
    pub reason: String,
}

pub fn detect_test_commands(root: impl AsRef<Path>) -> std::io::Result<Vec<TestCommand>> {
    let root = root.as_ref();
    let mut files = Vec::new();
    for name in [
        "Cargo.toml",
        "package.json",
        "pnpm-lock.yaml",
        "yarn.lock",
        "pyproject.toml",
        "requirements.txt",
        "setup.py",
        "pytest.ini",
    ] {
        if root.join(name).exists() {
            files.push(name.to_string());
        }
    }
    Ok(detect_test_commands_for_files(&files))
}

pub fn detect_test_commands_for_files(files: &[String]) -> Vec<TestCommand> {
    let mut commands = Vec::new();

    if has(files, "Cargo.toml") {
        commands.push(TestCommand {
            command: "cargo test".to_string(),
            reason: "Rust project detected from Cargo.toml".to_string(),
        });
    }

    if has(files, "package.json") {
        commands.push(TestCommand {
            command: "npm test".to_string(),
            reason: "Node project detected from package.json".to_string(),
        });
        if has(files, "pnpm-lock.yaml") {
            commands.push(TestCommand {
                command: "pnpm test".to_string(),
                reason: "pnpm lockfile detected".to_string(),
            });
        }
        if has(files, "yarn.lock") {
            commands.push(TestCommand {
                command: "yarn test".to_string(),
                reason: "Yarn lockfile detected".to_string(),
            });
        }
    }

    if has_any(
        files,
        &[
            "pyproject.toml",
            "requirements.txt",
            "setup.py",
            "pytest.ini",
        ],
    ) {
        commands.push(TestCommand {
            command: "python -m pytest".to_string(),
            reason: "Python project detected".to_string(),
        });
        if has(files, "pytest.ini") {
            commands.push(TestCommand {
                command: "pytest".to_string(),
                reason: "pytest configuration detected".to_string(),
            });
        }
    }

    commands
}

fn has(files: &[String], name: &str) -> bool {
    files.iter().any(|file| file == name)
}

fn has_any(files: &[String], names: &[&str]) -> bool {
    names.iter().any(|name| has(files, name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_rust_test_command() {
        let commands = detect_test_commands_for_files(&["Cargo.toml".to_string()]);

        assert_eq!(commands[0].command, "cargo test");
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
}
