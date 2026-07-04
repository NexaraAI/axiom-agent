use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntentAnalysis {
    pub original_prompt: String,
    pub task_type: String,
    pub language: Option<String>,
    pub needs_code: bool,
    pub needs_files: bool,
    pub needs_shell: bool,
    pub needs_web: bool,
    pub risk_level: String,
    pub keywords: Vec<String>,
    pub candidate_skill_ids: Vec<String>,
}

pub fn analyze_intent(prompt: &str) -> IntentAnalysis {
    let lower = prompt.to_ascii_lowercase();
    let mut keywords = Vec::new();
    let mut candidates = Vec::new();
    let mut needs_code = false;
    let mut needs_files = false;
    let mut needs_shell = false;
    let mut needs_web = false;
    let mut language = None;
    let mut task_type = "general".to_string();
    let mut risk_level = "low".to_string();

    if contains_any(&lower, &["python", ".py", "script"]) {
        task_type = "coding".to_string();
        language = Some("python".to_string());
        needs_code = true;
        push_keyword(&mut keywords, "python");
        push_candidate(&mut candidates, "python.write");
        push_candidate(&mut candidates, "python.run");
    }

    if contains_any(
        &lower,
        &[
            "website", "fetch", "url", "http://", "https://", "search", "web",
        ],
    ) {
        needs_web = true;
        push_keyword(&mut keywords, "web");
        push_candidate(&mut candidates, "web.fetch");
    }

    if contains_any(&lower, &["file", "read", "write", "save", "open"]) {
        needs_files = true;
        push_keyword(&mut keywords, "file");
        if contains_any(&lower, &["write", "save", "create"]) {
            risk_level = "medium".to_string();
            push_candidate(&mut candidates, "file.write");
        }
        if contains_any(&lower, &["file", "read", "open"]) {
            push_candidate(&mut candidates, "file.read");
        }
    }

    if contains_any(&lower, &["git", "diff", "commit", "status"]) {
        push_keyword(&mut keywords, "git");
        if contains_any(&lower, &["status", "git"]) {
            push_candidate(&mut candidates, "git.status");
        }
        if contains_any(&lower, &["diff", "changes"]) {
            push_candidate(&mut candidates, "git.diff");
        }
    }

    if contains_any(
        &lower,
        &[
            "run",
            "test",
            "command",
            "terminal",
            "shell",
            "powershell",
            "bash",
            "zsh",
        ],
    ) {
        needs_shell = true;
        risk_level = "high".to_string();
        push_keyword(&mut keywords, "shell");
        push_candidate(&mut candidates, "shell.powershell.safe");
        push_candidate(&mut candidates, "shell.bash.safe");
        push_candidate(&mut candidates, "shell.zsh.safe");
    }

    IntentAnalysis {
        original_prompt: prompt.to_string(),
        task_type,
        language,
        needs_code,
        needs_files,
        needs_shell,
        needs_web,
        risk_level,
        keywords,
        candidate_skill_ids: candidates,
    }
}

fn contains_any(input: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| input.contains(needle))
}

fn push_keyword(keywords: &mut Vec<String>, keyword: &str) {
    if !keywords.iter().any(|candidate| candidate == keyword) {
        keywords.push(keyword.to_string());
    }
}

fn push_candidate(candidates: &mut Vec<String>, skill_id: &str) {
    if !candidates.iter().any(|candidate| candidate == skill_id) {
        candidates.push(skill_id.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_python_prompt() {
        let intent = analyze_intent("write a Python script");

        assert_eq!(intent.task_type, "coding");
        assert_eq!(intent.language.as_deref(), Some("python"));
        assert!(intent
            .candidate_skill_ids
            .contains(&"python.write".to_string()));
    }
}
