use serde::{Deserialize, Serialize};

use crate::ProjectScanSummary;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodingPlan {
    pub task: String,
    pub steps: Vec<String>,
}

pub fn build_plan_prompt(task: &str, scan: &ProjectScanSummary, skill_context: &str) -> String {
    format!(
        r#"You are Axiom Coder, a terminal coding assistant.

Create a concise implementation plan. Do not write files. Do not produce a patch yet.

Task:
{task}

Workspace:
{root}

Project type:
{project_type}

Important files:
{important_files}

Likely test commands:
{test_commands}

Selected Axiom skills:
{skill_context}

Safety rules:
- Stay inside the workspace.
- Do not edit secret files.
- Show a plan before changes.
- File writes require confirmation.
- Commands require confirmation.
"#,
        root = scan.root,
        project_type = scan.project_type,
        important_files = scan.important_files.join("\n"),
        test_commands = scan
            .likely_test_commands
            .iter()
            .map(|command| command.command.as_str())
            .collect::<Vec<_>>()
            .join("\n"),
    )
}

pub fn build_patch_prompt(task: &str, scan: &ProjectScanSummary, plan: &str) -> String {
    format!(
        r#"You are Axiom Coder.

Propose file changes for this task using only this provider-independent patch format:

```axiom-patch
{{
  "summary": "short summary",
  "test_command": "optional safe test command",
  "changes": [
    {{
      "path": "relative/path",
      "action": "create_or_update",
      "content": "full new file content"
    }}
  ]
}}
```

Use full-file replacement content. Do not delete files. Do not edit secret files. Keep paths relative to the workspace.

Task:
{task}

Project type:
{project_type}

Important files:
{important_files}

Plan:
{plan}
"#,
        project_type = scan.project_type,
        important_files = scan.important_files.join("\n"),
    )
}

pub fn build_fallback_plan(task: &str, scan: &ProjectScanSummary) -> CodingPlan {
    let mut steps = vec![
        format!("Review the {} project structure.", scan.project_type),
        "Identify the files related to the requested change.".to_string(),
        "Prepare a minimal full-file patch and show the diff before writing.".to_string(),
    ];

    if scan.likely_test_commands.is_empty() {
        steps.push("No obvious test command was detected.".to_string());
    } else {
        steps.push(format!(
            "After approval, run `{}`.",
            scan.likely_test_commands[0].command
        ));
    }

    CodingPlan {
        task: task.to_string(),
        steps,
    }
}

pub fn parse_plan_response(task: &str, response: &str) -> CodingPlan {
    let steps = response
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|line| {
            line.trim_start_matches(|character: char| {
                character.is_ascii_digit()
                    || character == '.'
                    || character == ')'
                    || character == '-'
                    || character.is_whitespace()
            })
            .trim()
            .to_string()
        })
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();

    CodingPlan {
        task: task.to_string(),
        steps,
    }
}

pub fn plan_only_flow_with_mock<F>(
    task: &str,
    scan: &ProjectScanSummary,
    mut planner: F,
) -> Result<CodingPlan, String>
where
    F: FnMut(String) -> Result<String, String>,
{
    let prompt = build_plan_prompt(task, scan, "");
    planner(prompt).map(|response| parse_plan_response(task, &response))
}

#[cfg(test)]
mod tests {
    use crate::{ProjectType, TestCommand};

    use super::*;

    #[test]
    fn plan_only_flow_uses_mocked_llm_response() {
        let scan = sample_scan();
        let plan = plan_only_flow_with_mock("fix cargo error", &scan, |prompt| {
            assert!(prompt.contains("fix cargo error"));
            Ok("1. Inspect Cargo.toml\n2. Run cargo test".to_string())
        })
        .expect("plan");

        assert_eq!(plan.steps[0], "Inspect Cargo.toml");
        assert_eq!(plan.steps[1], "Run cargo test");
    }

    fn sample_scan() -> ProjectScanSummary {
        ProjectScanSummary {
            root: "C:/Axiom".to_string(),
            project_type: ProjectType::Rust,
            files: vec!["Cargo.toml".to_string()],
            important_files: vec!["Cargo.toml".to_string()],
            ignored: Vec::new(),
            likely_test_commands: vec![TestCommand {
                command: "cargo test".to_string(),
                reason: "Rust project detected".to_string(),
            }],
            likely_format_commands: vec!["cargo fmt".to_string()],
        }
    }
}
