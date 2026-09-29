use std::path::Path;

use anyhow::Result;

use axiom_agent::UsageLedger;
use axiom_proof::AgentRuntimeProof;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ChatRuntimeStats {
    pub iterations: u32,
    pub tool_iterations: usize,
    pub turn_usage: UsageLedger,
    pub session_usage: UsageLedger,
    pub turn_cost_microusd: Option<u64>,
    pub session_cost_microusd: Option<u64>,
    pub context_tokens_estimate: u64,
    pub compacted_messages: usize,
    pub todo_updates: u32,
    pub todo_total: usize,
    pub todo_completed: usize,
    pub todo_remaining: usize,
    pub todo_blocked: usize,
}

impl ChatRuntimeStats {
    pub(crate) fn to_proof(&self) -> AgentRuntimeProof {
        AgentRuntimeProof {
            iterations: self.iterations,
            tool_iterations: u32::try_from(self.tool_iterations).unwrap_or(u32::MAX),
            prompt_tokens: self.turn_usage.prompt_tokens,
            completion_tokens: self.turn_usage.completion_tokens,
            total_tokens: self.turn_usage.total_tokens,
            estimated_cost_microusd: self.turn_cost_microusd,
            context_tokens_estimate: self.context_tokens_estimate,
            compacted_messages: self.compacted_messages,
            todo_updates: self.todo_updates,
            todo_total: self.todo_total,
            todo_completed: self.todo_completed,
            todo_remaining: self.todo_remaining,
            todo_blocked: self.todo_blocked,
        }
    }

    pub(crate) fn status_text(&self) -> String {
        let calls = if self.iterations == 1 {
            "call"
        } else {
            "calls"
        };
        let cost = match (self.turn_cost_microusd, self.session_cost_microusd) {
            (Some(turn), Some(session)) => format!(
                " · turn ${:.6} / session ${:.6}",
                turn as f64 / 1_000_000.0,
                session as f64 / 1_000_000.0
            ),
            _ => String::new(),
        };
        let compacted = if self.compacted_messages == 0 {
            String::new()
        } else {
            format!(" · {} compacted", self.compacted_messages)
        };
        let todo = if self.todo_total == 0 {
            String::new()
        } else {
            format!(
                " · todo {}/{} done, {} blocked",
                self.todo_completed, self.todo_total, self.todo_blocked
            )
        };
        format!(
            "{} model {calls} · turn {} in / {} out · session {} tokens · context ~{}{cost}{compacted}{todo}",
            self.iterations,
            self.turn_usage.prompt_tokens,
            self.turn_usage.completion_tokens,
            self.session_usage.total_tokens,
            self.context_tokens_estimate,
        )
    }
}

pub(crate) fn load_workspace_rules(workspace_root: &Path) -> Option<String> {
    const MAX_RULES_BYTES: usize = 32 * 1024;
    const MAX_IMPORT_DEPTH: usize = 3;
    const MAX_FILES: usize = 16;

    let mut candidates: Vec<(String, std::path::PathBuf)> = Vec::new();
    if let Some(home) = std::env::var_os("AXIOM_HOME") {
        candidates.push((
            "Global AXIOM.md".to_string(),
            std::path::PathBuf::from(home).join("AXIOM.md"),
        ));
    }
    for (label, relative) in [
        ("Workspace .axiomrules", ".axiomrules"),
        ("Workspace AXIOM.md", "AXIOM.md"),
        ("Workspace AGENTS.md", "AGENTS.md"),
        ("Workspace .axiom/memory.md", ".axiom/memory.md"),
        ("Workspace MEMORY.md", "MEMORY.md"),
    ] {
        candidates.push((label.to_string(), workspace_root.join(relative)));
    }
    if let Ok(entries) = std::fs::read_dir(workspace_root.join(".axiom").join("rules")) {
        let mut rule_files = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.extension()
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("md"))
            })
            .collect::<Vec<_>>();
        rule_files.sort();
        for path in rule_files {
            let name = path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            candidates.push((format!(".axiom/rules/{name}"), path));
        }
    }

    let mut sections: Vec<String> = Vec::new();
    let mut seen: std::collections::BTreeSet<std::path::PathBuf> =
        std::collections::BTreeSet::new();
    let mut remaining = MAX_RULES_BYTES;
    for (label, path) in candidates {
        if sections.len() >= MAX_FILES || remaining == 0 {
            break;
        }
        append_instruction_file(
            &mut sections,
            &mut seen,
            &path,
            &label,
            0,
            MAX_IMPORT_DEPTH,
            &mut remaining,
        );
    }

    if sections.is_empty() {
        None
    } else {
        Some(sections.join("\n\n"))
    }
}

/// Appends one instruction file and its bounded `@import` graph to the merged ruleset.
fn append_instruction_file(
    sections: &mut Vec<String>,
    seen: &mut std::collections::BTreeSet<std::path::PathBuf>,
    path: &Path,
    label: &str,
    depth: usize,
    max_depth: usize,
    remaining: &mut usize,
) {
    if *remaining == 0 || depth > max_depth || sections.len() >= 16 {
        return;
    }
    let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    if !seen.insert(canonical) {
        return;
    }
    let Ok(content) = std::fs::read_to_string(path) else {
        return;
    };
    let mut body = String::new();
    for line in content.lines() {
        if let Some(import) = line.trim().strip_prefix("@import ") {
            let import = import.trim();
            if !import.is_empty() {
                let import_path = path.parent().unwrap_or_else(|| Path::new(".")).join(import);
                append_instruction_file(
                    sections,
                    seen,
                    &import_path,
                    &format!("{label} -> {import}"),
                    depth + 1,
                    max_depth,
                    remaining,
                );
            }
            continue;
        }
        if body.len() + line.len() + 1 > *remaining {
            break;
        }
        body.push_str(line);
        body.push('\n');
    }
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return;
    }
    *remaining = (*remaining).saturating_sub(trimmed.len());
    sections.push(format!("[{label}]\n{trimmed}"));
}
#[cfg(test)]
mod tests {
    use std::{
        fs,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::*;

    #[test]
    fn workspace_rules_merge_root_rules_with_nested_imports() {
        let dir = std::env::temp_dir().join(format!(
            "axiom-rules-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        fs::create_dir_all(dir.join(".axiom").join("rules")).expect("rules dir");
        fs::write(dir.join("AGENTS.md"), "root instruction").expect("agents");
        fs::write(
            dir.join(".axiom").join("rules").join("a.md"),
            "alpha rule\n@import extra.md\n",
        )
        .expect("alpha");
        fs::write(dir.join(".axiom").join("rules").join("b.md"), "beta rule").expect("beta");
        fs::write(
            dir.join(".axiom").join("rules").join("extra.md"),
            "imported rule",
        )
        .expect("extra");

        let merged = load_workspace_rules(&dir).expect("merged rules");

        assert!(merged.contains("root instruction"));
        assert!(merged.contains("alpha rule"));
        assert!(merged.contains("imported rule"));
        assert!(merged.contains("beta rule"));
        assert!(merged.find("alpha rule") < merged.find("beta rule"));
        let _ = fs::remove_dir_all(dir);
    }
}
