use axiom_engine::{SkillExecutionResult, ToolRequest};
use serde_json::{json, Value};

use super::observation::tool_observation;
use super::{ToolExecutionEvent, ToolExecutionStatus};
use crate::TOOL_RESULT_BUDGET_CHARS;

/// A payload shaped like a real `github.search` organization listing, before
/// projection: fifteen repositories, each carrying the full 83-field GitHub
/// API object.
fn github_search_payload(repo_count: usize) -> Value {
    let repos: Vec<Value> = (0..repo_count)
        .map(|index| {
            let mut repo = json!({
                "id": 1000 + index,
                "node_id": format!("R_kgDOABC{index}"),
                "name": format!("project-{index}"),
                "full_name": format!("demonzdevelopment/project-{index}"),
                "private": false,
                "html_url": format!("https://github.com/demonzdevelopment/project-{index}"),
                "description": format!("A Minecraft mod project number {index}"),
                "fork": false,
                "language": "Java",
                "stargazers_count": 12,
                "forks_count": 1,
                "open_issues_count": 2,
                "topics": ["minecraft", "mod"],
                "license": {"key": "mit", "name": "MIT License", "spdx_id": "MIT"},
                "default_branch": "main",
                "created_at": "2024-01-02T03:04:05Z",
                "updated_at": "2026-05-23T22:04:09Z",
                "pushed_at": "2026-09-20T12:05:08Z",
                "size": 4096,
                "visibility": "public",
            });
            // The derived fields that make the real object 83 wide.
            for key in [
                "allow_forking",
                "archive_url",
                "archived",
                "assignees_url",
                "blobs_url",
                "branches_url",
                "clone_url",
                "collaborators_url",
                "comments_url",
                "commits_url",
                "compare_url",
                "contents_url",
                "contributors_url",
                "deployments_url",
                "downloads_url",
                "events_url",
                "forks_url",
                "git_commits_url",
                "git_refs_url",
                "git_tags_url",
                "git_url",
                "hooks_url",
                "issue_comment_url",
                "issue_events_url",
                "issues_url",
                "keys_url",
                "labels_url",
                "languages_url",
                "merges_url",
                "milestones_url",
                "mirror_url",
                "network_count",
                "notifications_url",
                "pulls_url",
                "releases_url",
                "role_name",
                "stargazers_url",
                "statuses_url",
                "subscribers_url",
                "subscription_url",
                "svn_url",
                "teams_url",
                "trees_url",
            ] {
                repo[key] = json!(format!(
                    "https://api.github.com/repos/demo/project-{index}/{key}"
                ));
            }
            repo
        })
        .collect();

    json!({
        "source": "github",
        "mode": "org_repos",
        "status": 200,
        "results": repos,
    })
}

/// Regression test for the context blow-up this was written to stop.
///
/// A fifteen-repository organization listing is about 110 kB of raw API JSON,
/// roughly 89% derived URLs and flags. Fed into the transcript uncapped, one
/// call turned a 1 kB answer into 79 kB of prompt tokens, because every later
/// model call in the turn resends it.
#[test]
fn tool_observation_caps_a_large_tool_result() {
    let payload = github_search_payload(15);
    let raw = payload.to_string();
    assert!(
        raw.chars().count() > 50_000,
        "fixture should be large, was {}",
        raw.chars().count()
    );

    let event = succeeded("github.search", payload);
    let observation = tool_observation(&event);

    assert!(
        observation.chars().count() <= TOOL_RESULT_BUDGET_CHARS + 400,
        "observation was {} chars, budget is {TOOL_RESULT_BUDGET_CHARS}",
        observation.chars().count()
    );
    assert!(
        observation.contains("[truncated:"),
        "expected a truncation note, got: {}",
        &observation[observation.len().saturating_sub(200)..]
    );
    assert!(
        observation.contains("github.search"),
        "the truncation note should name the tool"
    );
}

/// A fetch-style envelope should reach the model as text, not as re-escaped
/// JSON. Presenting `{"status":200,"text":"a\nb"}` verbatim makes the model
/// read `\n` escapes and spend tokens on the envelope.
#[test]
fn fetch_envelope_is_unwrapped_into_plain_text() {
    let payload = json!({
        "url": "https://example.com/page",
        "status": 200,
        "content_type": "text/markdown",
        "bytes": 1234,
        "text": "## Heading\n\nBody line one.\nBody line two.",
    });
    let event = succeeded("web.fetch", payload);
    let observation = tool_observation(&event);

    assert!(
        observation.contains("Body line one.\nBody line two."),
        "newlines should be real, not escaped: {observation}"
    );
    assert!(
        !observation.contains("\\n"),
        "payload should not be double-escaped: {observation}"
    );
    assert!(
        observation.contains("HTTP 200"),
        "provenance should be kept in the header: {observation}"
    );
    assert!(
        !observation.contains("\"content_type\""),
        "the JSON envelope should not be shown as JSON: {observation}"
    );
}

/// Small payloads must pass through untouched, or ordinary tools break.
#[test]
fn small_results_are_not_disturbed() {
    let payload = json!({"path": "hello.txt", "bytes": 5});
    let event = succeeded("file.read", payload);
    let observation = tool_observation(&event);
    assert!(observation.contains("\"path\":\"hello.txt\""));
    assert!(
        !observation.contains("[truncated:"),
        "small payloads must not be truncated"
    );
    assert!(
        observation.contains("```json"),
        "JSON results keep their fence"
    );
}

fn succeeded(skill_id: &str, output: Value) -> ToolExecutionEvent {
    ToolExecutionEvent {
        request: ToolRequest {
            skill_id: skill_id.to_string(),
            arguments: json!({}),
        },
        latency_ms: 1,
        status: ToolExecutionStatus::Succeeded(SkillExecutionResult {
            skill_id: skill_id.to_string(),
            output,
        }),
        hooks: Vec::new(),
    }
}
