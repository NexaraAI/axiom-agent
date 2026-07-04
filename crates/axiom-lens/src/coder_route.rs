use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CodingTaskConfidence {
    None,
    Ambiguous,
    Obvious,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodingTaskDetection {
    pub confidence: CodingTaskConfidence,
    pub reason: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutoRouteAction {
    StayInChat,
    Ask,
    Switch,
}

pub fn detect_project_coding_task(prompt: &str) -> CodingTaskDetection {
    let lower = prompt.to_ascii_lowercase();
    if is_simple_snippet_request(&lower) {
        return CodingTaskDetection {
            confidence: CodingTaskConfidence::None,
            reason: "simple code generation request".to_string(),
        };
    }

    if contains_any(
        &lower,
        &[
            "fix this bug",
            "edit my project",
            "add a feature",
            "run tests",
            "scan this repo",
            "explain this codebase",
            "modify readme",
            "create files for this project",
            "why is my build failing",
            "refactor this function",
            "update package.json",
            "fix cargo error",
            "fix npm error",
            "make this project compile",
        ],
    ) {
        return CodingTaskDetection {
            confidence: CodingTaskConfidence::Obvious,
            reason: "project-level coding phrase detected".to_string(),
        };
    }

    let has_project_marker = contains_any(
        &lower,
        &[
            "project",
            "repo",
            "repository",
            "codebase",
            "workspace",
            "readme",
            "package.json",
            "cargo.toml",
            "build.gradle",
            "pyproject.toml",
            "compile",
            "build failing",
        ],
    );
    let has_project_action = contains_any(
        &lower,
        &[
            "fix", "edit", "modify", "create", "update", "refactor", "test", "scan", "explain",
            "debug",
        ],
    );

    if has_project_marker && has_project_action {
        return CodingTaskDetection {
            confidence: CodingTaskConfidence::Obvious,
            reason: "project marker and coding action detected".to_string(),
        };
    }

    if contains_any(&lower, &["bug", "failing", "tests", "compile error"]) {
        return CodingTaskDetection {
            confidence: CodingTaskConfidence::Ambiguous,
            reason: "possible project coding task detected".to_string(),
        };
    }

    CodingTaskDetection {
        confidence: CodingTaskConfidence::None,
        reason: "no project coding task detected".to_string(),
    }
}

pub fn auto_route_action(prompt: &str, enabled: bool, mode: &str) -> AutoRouteAction {
    if !enabled || mode == "off" {
        return AutoRouteAction::StayInChat;
    }

    match detect_project_coding_task(prompt).confidence {
        CodingTaskConfidence::None => AutoRouteAction::StayInChat,
        CodingTaskConfidence::Ambiguous => AutoRouteAction::Ask,
        CodingTaskConfidence::Obvious if mode == "smart" => AutoRouteAction::Switch,
        CodingTaskConfidence::Obvious => AutoRouteAction::Ask,
    }
}

fn is_simple_snippet_request(lower: &str) -> bool {
    contains_any(
        lower,
        &[
            "write me a simple",
            "give me a regex",
            "show an example",
            "what is npm",
            "what is a variable",
            "explain what a variable is",
            "short code snippet",
            "without using my project",
        ],
    )
}

fn contains_any(input: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| input.contains(needle))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_project_coding_intent() {
        let detection = detect_project_coding_task("fix cargo error");

        assert_eq!(detection.confidence, CodingTaskConfidence::Obvious);
    }

    #[test]
    fn simple_code_prompt_does_not_route() {
        let detection = detect_project_coding_task("write me a simple Python function");

        assert_eq!(detection.confidence, CodingTaskConfidence::None);
    }

    #[test]
    fn auto_route_off_stays_in_chat() {
        assert_eq!(
            auto_route_action("fix this bug", false, "ask"),
            AutoRouteAction::StayInChat
        );
        assert_eq!(
            auto_route_action("fix this bug", true, "off"),
            AutoRouteAction::StayInChat
        );
    }

    #[test]
    fn auto_route_ask_prompts_for_obvious_task() {
        assert_eq!(
            auto_route_action("make this project compile", true, "ask"),
            AutoRouteAction::Ask
        );
    }

    #[test]
    fn auto_route_smart_switches_for_obvious_and_asks_for_ambiguous() {
        assert_eq!(
            auto_route_action("make this project compile", true, "smart"),
            AutoRouteAction::Switch
        );
        assert_eq!(
            auto_route_action("tests are failing", true, "smart"),
            AutoRouteAction::Ask
        );
    }
}
