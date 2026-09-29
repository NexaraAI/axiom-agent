use std::time::Instant;

use super::*;

#[tokio::test]
async fn manifest_hooks_fire_around_a_successful_tool_call() {
    let provider = ToolCallProvider::new(vec![(
        "axiom_project_scan".to_string(),
        json!({"path": "."}),
    )]);
    // Hooks receive the triggering tool's arguments, so `code.list` is
    // pointed at the same directory `project.scan` was asked to scan.
    let installed = [installed_tool_with_hooks(
        "project.scan",
        Some("code.list"),
        Some("code.list"),
        None,
    )];
    let mut approval = DenyAllApprover;
    let mut agent = AgentLoop::new(
        &provider,
        "test-model",
        AgentCaps::default(),
        Vec::new(),
        Vec::new(),
        &installed,
        context(),
        &mut approval,
    );

    let completion = done_turn(
        agent
            .run_turn(user_message("read the manifest"))
            .await
            .expect("turn succeeds"),
    );

    let event = completion.tool_events.first().expect("one tool event");
    assert!(matches!(event.status, ToolExecutionStatus::Succeeded(_)));
    let phases = event
        .hooks
        .iter()
        .map(|hook| hook.phase)
        .collect::<Vec<_>>();
    assert_eq!(phases, vec![HookPhase::Pre, HookPhase::Post]);
    for hook in &event.hooks {
        assert_eq!(hook.hook_id, "code.list");
        assert_eq!(hook.status, HookStatus::Succeeded, "{hook:?}");
        assert!(hook.output.is_some(), "{hook:?}");
    }

    let observation = first_tool_observation(&completion);
    assert!(observation.contains("Manifest hooks:"));
    assert!(observation.contains("- code.list [pre]"));
    assert!(observation.contains("- code.list [post]"));
}

#[tokio::test]
async fn a_post_write_lint_check_hook_lints_the_written_file() {
    let root = std::env::temp_dir().join(format!(
        "axiom-loop-hook-lint-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).expect("root");
    let provider = ToolCallProvider::new(vec![(
        "axiom_file_write".to_string(),
        json!({"path": "messy.rs", "content": "fn main() {\n\tlet x = 1;   \n}\n"}),
    )]);
    // The registry's real post-write hook: file.write 0.2.0 ships
    // `[hooks] post = "lint.check"`, and lint.check absorbs the write's
    // arguments verbatim and lints the persisted file from disk.
    let installed = [
        installed_tool_with_hooks("file.write", None, Some("lint.check"), None),
        installed_tool("lint.check"),
    ];
    let mut approval = AllowAllApprover;
    let context = SkillExecutionContext {
        workspace_root: root.clone(),
        auto_approve_medium_risk: true,
        ..context()
    };
    let mut agent = AgentLoop::new(
        &provider,
        "test-model",
        AgentCaps::default(),
        Vec::new(),
        Vec::new(),
        &installed,
        context,
        &mut approval,
    );

    let completion = done_turn(
        agent
            .run_turn(user_message("write a file"))
            .await
            .expect("turn succeeds"),
    );

    let event = completion.tool_events.first().expect("one tool event");
    assert!(matches!(event.status, ToolExecutionStatus::Succeeded(_)));
    assert_eq!(event.hooks.len(), 1, "{event:?}");
    let hook = &event.hooks[0];
    assert_eq!(hook.hook_id, "lint.check");
    assert_eq!(hook.phase, HookPhase::Post);
    assert_eq!(hook.status, HookStatus::Succeeded, "{hook:?}");
    let output = hook.output.as_ref().expect("hook output");
    assert_eq!(output["path"], "messy.rs");
    let findings = output["findings"].as_array().expect("findings");
    assert!(
        findings.len() >= 2,
        "expected tab + trailing-whitespace findings, got {findings:?}"
    );
    let observation = first_tool_observation(&completion);
    assert!(observation.contains("lint.check [post]"), "{observation}");
    assert!(observation.contains("no-tabs"), "{observation}");
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn on_error_hooks_fire_when_the_tool_fails_and_post_does_not() {
    let provider = ToolCallProvider::new(vec![(
        "axiom_project_scan".to_string(),
        json!({"path": "no-such-directory-axiom-hooks"}),
    )]);
    let installed = [installed_tool_with_hooks(
        "project.scan",
        None,
        Some("code.list"),
        Some("code.list"),
    )];
    let mut approval = DenyAllApprover;
    let mut agent = AgentLoop::new(
        &provider,
        "test-model",
        AgentCaps::default(),
        Vec::new(),
        Vec::new(),
        &installed,
        context(),
        &mut approval,
    );

    let completion = done_turn(
        agent
            .run_turn(user_message("read a missing file"))
            .await
            .expect("turn succeeds"),
    );

    let event = completion.tool_events.first().expect("one tool event");
    assert!(matches!(event.status, ToolExecutionStatus::Failed(_)));
    assert_eq!(event.hooks.len(), 1, "post must not run for a failed tool");
    assert_eq!(event.hooks[0].phase, HookPhase::OnError);
    assert!(
        matches!(
            event.hooks[0].status,
            HookStatus::Succeeded | HookStatus::Failed
        ),
        "the on_error hook actually ran"
    );
    assert!(first_tool_observation(&completion).contains("[on_error]"));
}

#[tokio::test]
async fn manifest_hooks_fire_for_parallel_read_only_batches() {
    let provider = ToolCallProvider::new(vec![
        ("axiom_project_scan".to_string(), json!({"path": "."})),
        ("axiom_project_scan".to_string(), json!({"path": "src"})),
    ]);
    let installed = [installed_tool_with_hooks(
        "project.scan",
        Some("code.list"),
        Some("code.list"),
        None,
    )];
    let mut approval = DenyAllApprover;
    let mut agent = AgentLoop::new(
        &provider,
        "test-model",
        AgentCaps::default(),
        Vec::new(),
        Vec::new(),
        &installed,
        context(),
        &mut approval,
    );

    let completion = done_turn(
        agent
            .run_turn(user_message("read both files"))
            .await
            .expect("turn succeeds"),
    );

    assert_eq!(completion.tool_events.len(), 2);
    for event in &completion.tool_events {
        let phases = event
            .hooks
            .iter()
            .map(|hook| hook.phase)
            .collect::<Vec<_>>();
        assert_eq!(phases, vec![HookPhase::Pre, HookPhase::Post]);
        for hook in &event.hooks {
            assert_eq!(hook.status, HookStatus::Succeeded, "{hook:?}");
        }
    }
}

#[tokio::test]
async fn a_hook_that_cannot_execute_is_recorded_as_unavailable() {
    let provider = ToolCallProvider::new(vec![(
        "axiom_project_scan".to_string(),
        json!({"path": "."}),
    )]);
    let installed = [installed_tool_with_hooks(
        "project.scan",
        None,
        Some("demo.lint"),
        None,
    )];
    let mut approval = DenyAllApprover;
    let mut agent = AgentLoop::new(
        &provider,
        "test-model",
        AgentCaps::default(),
        Vec::new(),
        Vec::new(),
        &installed,
        context(),
        &mut approval,
    );

    let completion = done_turn(
        agent
            .run_turn(user_message("read the manifest"))
            .await
            .expect("turn succeeds"),
    );

    let event = completion.tool_events.first().expect("one tool event");
    assert!(matches!(event.status, ToolExecutionStatus::Succeeded(_)));
    assert_eq!(event.hooks.len(), 1);
    assert_eq!(event.hooks[0].hook_id, "demo.lint");
    assert_eq!(event.hooks[0].status, HookStatus::Unavailable);
    assert!(first_tool_observation(&completion).contains("not executable"));
}

#[tokio::test]
async fn hooks_run_through_the_side_effect_policy_and_approval_gate() {
    let provider = ToolCallProvider::new(vec![(
        "axiom_project_scan".to_string(),
        json!({"path": "."}),
    )]);
    let installed = [installed_tool_with_hooks(
        "project.scan",
        Some("code.list"),
        None,
        None,
    )];
    let mut approval = DenyAllApprover;
    let mut agent = AgentLoop::new(
        &provider,
        "test-model",
        AgentCaps::default(),
        Vec::new(),
        Vec::new(),
        &installed,
        context(),
        &mut approval,
    )
    .with_side_effect_policy(SideEffectPolicy {
        filesystem_read: PolicyAction::Ask,
        ..SideEffectPolicy::deny_all()
    });

    let completion = done_turn(
        agent
            .run_turn(user_message("read the manifest"))
            .await
            .expect("turn succeeds"),
    );

    let event = completion.tool_events.first().expect("one tool event");
    assert_eq!(event.hooks.len(), 1);
    assert_eq!(event.hooks[0].status, HookStatus::Failed);
    assert!(event.hooks[0]
        .error
        .as_deref()
        .unwrap_or_default()
        .contains("approval denied"));
    assert!(matches!(event.status, ToolExecutionStatus::Failed(_)));
    assert!(
        completion
            .policy_decisions
            .iter()
            .any(|decision| decision.evaluation.request.skill_id == "code.list"),
        "the hook's own policy decision is recorded"
    );
}

#[tokio::test]
async fn hooks_skip_on_reentrancy_and_exhausted_budget_instead_of_running() {
    let provider = MockProvider::new("mock");
    let installed = [installed_tool_with_hooks(
        "project.scan",
        Some("code.list"),
        Some("code.list"),
        None,
    )];
    let mut approval = DenyAllApprover;
    let mut agent = AgentLoop::new(
        &provider,
        "test-model",
        AgentCaps::default(),
        Vec::new(),
        Vec::new(),
        &installed,
        context(),
        &mut approval,
    );
    let request = ToolRequest {
        skill_id: "project.scan".to_string(),
        arguments: json!({"path": "."}),
    };
    let mut progress = TurnProgress::default();

    let mut exhausted = HookBudget {
        remaining: 0,
        started_at: Instant::now(),
        deadline_secs: 3600,
    };
    let hooks = agent
        .fire_hooks(HookPhase::Pre, &request, &mut exhausted, &mut progress)
        .await;
    assert_eq!(hooks[0].status, HookStatus::SkippedBudget);
    assert!(hooks[0].output.is_none());

    let mut late = HookBudget {
        remaining: 5,
        started_at: Instant::now() - std::time::Duration::from_secs(60),
        deadline_secs: 1,
    };
    let hooks = agent
        .fire_hooks(HookPhase::Pre, &request, &mut late, &mut progress)
        .await;
    assert_eq!(hooks[0].status, HookStatus::SkippedBudget);
    assert_eq!(late.remaining, 5, "a wall-clock skip spends no allowance");

    let mut nested = AgentLoop::new(
        &provider,
        "test-model",
        AgentCaps::default(),
        Vec::new(),
        Vec::new(),
        &installed,
        context(),
        &mut approval,
    )
    .with_hook_depth(1);
    let mut budget = HookBudget::new(&AgentCaps::default(), Instant::now());
    let hooks = nested
        .fire_hooks(HookPhase::Pre, &request, &mut budget, &mut progress)
        .await;
    assert_eq!(hooks[0].status, HookStatus::SkippedReentrancy);
    assert_eq!(
        budget.remaining, MAX_HOOKS_PER_TURN,
        "a re-entrancy skip spends no allowance"
    );
}

#[tokio::test]
async fn a_skill_cannot_hook_itself() {
    let provider = MockProvider::new("mock");
    let installed = [installed_tool_with_hooks(
        "project.scan",
        Some("project.scan"),
        Some("project.scan"),
        Some("project.scan"),
    )];
    let mut approval = DenyAllApprover;
    let mut agent = AgentLoop::new(
        &provider,
        "test-model",
        AgentCaps::default(),
        Vec::new(),
        Vec::new(),
        &installed,
        context(),
        &mut approval,
    );
    let request = ToolRequest {
        skill_id: "project.scan".to_string(),
        arguments: json!({"path": "."}),
    };
    let mut progress = TurnProgress::default();
    let mut budget = HookBudget::new(&AgentCaps::default(), Instant::now());

    for phase in [HookPhase::Pre, HookPhase::Post, HookPhase::OnError] {
        let hooks = agent
            .fire_hooks(phase, &request, &mut budget, &mut progress)
            .await;
        assert_eq!(hooks.len(), 1);
        assert_eq!(
            hooks[0].status,
            HookStatus::SkippedReentrancy,
            "{phase:?} must refuse to re-enter the loop"
        );
    }
    assert_eq!(budget.remaining, MAX_HOOKS_PER_TURN);
}

#[test]
fn tool_observation_formats_question_ask_as_trusted_user_instruction() {
    let custom_event = ToolExecutionEvent {
        request: ToolRequest {
            skill_id: "question.ask".to_string(),
            arguments: json!({"question": "Where to publish?"}),
        },
        latency_ms: 100,
        status: ToolExecutionStatus::Succeeded(SkillExecutionResult {
            skill_id: "question.ask".to_string(),
            output: json!({
                "selected": "Modrinth and CurseForge",
                "is_custom": true,
                "index": 3
            }),
        }),
        hooks: Vec::new(),
    };

    let obs = tool_observation(&custom_event);
    assert!(
        obs.contains("The user responded to your clarification question (custom write-in reply):")
    );
    assert!(obs.contains("Modrinth and CurseForge"));
    assert!(obs.contains("IMPORTANT: Prioritize this user answer above all else"));
}

#[test]
fn tool_observation_formats_timeout_as_autonomous_recovery_directive() {
    let timeout_event = ToolExecutionEvent {
        request: ToolRequest {
            skill_id: "shell.powershell.safe".to_string(),
            arguments: json!({"command": "scp -r user@remote:/data ."}),
        },
        latency_ms: 30000,
        status: ToolExecutionStatus::Succeeded(SkillExecutionResult {
            skill_id: "shell.powershell.safe".to_string(),
            output: json!({
                "exit_code": 124,
                "stdout": "Transferred 10 files...",
                "stderr": "Command timed out after 30s."
            }),
        }),
        hooks: Vec::new(),
    };

    let obs = tool_observation(&timeout_event);
    assert!(obs.contains("timed out after execution limit (exit code 124)"));
    assert!(obs.contains("AUTONOMOUS RECOVERY DIRECTIVE: Do not give up or abandon the task!"));
    assert!(obs.contains("Transferred 10 files..."));
}

#[test]
fn tool_observation_formats_declined_approval_as_recovery_directive() {
    let denied_event = ToolExecutionEvent {
        request: ToolRequest {
            skill_id: "shell.powershell.safe".to_string(),
            arguments: json!({"command": "rm -rf /"}),
        },
        latency_ms: 500,
        status: ToolExecutionStatus::Failed("approval denied by user policy".to_string()),
        hooks: Vec::new(),
    };

    let obs = tool_observation(&denied_event);
    assert!(obs.contains("was declined by user approval"));
    assert!(obs.contains("AUTONOMOUS RECOVERY DIRECTIVE: Do not give up. Select an alternative non-destructive approach"));
}

#[test]
fn sanitize_interrupted_content_truncates_repetitive_and_huge_text() {
    let pattern =
        "Both servers are launching. Let me verify they're actually up by checking the ports.\n";
    let repetitive_text = format!("{pattern}{pattern}{pattern}");
    let sanitized = sanitize_interrupted_content(&repetitive_text);
    assert_eq!(sanitized, pattern.trim());

    let huge = "A".repeat(3000);
    let sanitized_huge = sanitize_interrupted_content(&huge);
    assert!(sanitized_huge.len() < 1600);
    assert!(sanitized_huge.ends_with("[Output truncated on interruption]"));
}
