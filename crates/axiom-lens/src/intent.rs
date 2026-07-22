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
    if is_identity_or_smalltalk_prompt(&lower) {
        return IntentAnalysis {
            original_prompt: prompt.to_string(),
            task_type: "identity".to_string(),
            language: None,
            needs_code: false,
            needs_files: false,
            needs_shell: false,
            needs_web: false,
            risk_level: "low".to_string(),
            keywords: Vec::new(),
            candidate_skill_ids: Vec::new(),
        };
    }

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
    needles.iter().any(|needle| matches_word(input, needle))
}

fn is_identity_or_smalltalk_prompt(input: &str) -> bool {
    const IDENTITY_PHRASES: &[&str] = &[
        "who are you",
        "what can you do",
        "what do you do",
        "tell me about yourself",
        "help me understand",
    ];
    const STANDALONE_SMALLTALK: &[&str] = &["hi", "hello", "hey", "thanks", "thank you"];

    IDENTITY_PHRASES
        .iter()
        .any(|phrase| matches_word(input, phrase))
        || STANDALONE_SMALLTALK
            .iter()
            .any(|phrase| trim_terminal_punctuation(input) == *phrase)
}

fn matches_word(input: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return false;
    }

    let mut search_from = 0;
    while let Some(found_at) = input[search_from..].find(needle) {
        let start = search_from + found_at;
        let end = start + needle.len();
        let before = input[..start].chars().next_back();
        let after = input[end..].chars().next();
        if !before.is_some_and(is_word_continuation) && !after.is_some_and(is_word_continuation) {
            return true;
        }
        search_from = end;
    }

    false
}

fn is_word_continuation(character: char) -> bool {
    character.is_ascii_alphanumeric() || matches!(character, '_' | '-')
}

fn trim_terminal_punctuation(input: &str) -> &str {
    input.trim_matches(|character: char| {
        character.is_ascii_whitespace() || ".,!?;:".contains(character)
    })
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

    #[test]
    fn identity_prompt_bypasses_skill_selection_even_with_substring_traps() {
        let intent = analyze_intent("who are you, you legit digital agent?");

        assert_eq!(intent.task_type, "identity");
        assert!(intent.candidate_skill_ids.is_empty());
    }

    #[test]
    fn word_matching_does_not_select_file_read_for_hyphenated_open() {
        let intent = analyze_intent("I'm open-minded and looking for ideas.");

        assert!(!intent
            .candidate_skill_ids
            .contains(&"file.read".to_string()));
    }

    #[test]
    fn word_matching_does_not_select_git_for_embedded_text() {
        let intent = analyze_intent("This legitimate digital workflow needs discussion.");

        assert!(!intent
            .candidate_skill_ids
            .contains(&"git.status".to_string()));
    }
}
