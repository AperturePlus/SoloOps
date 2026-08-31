use super::*;

#[test]
fn phase_zero_worker_transitions_are_allowed() {
    assert!(RunStatus::Queued.can_transition_to(RunStatus::Leased));
    assert!(RunStatus::Leased.can_transition_to(RunStatus::Planning));
    assert!(RunStatus::Planning.can_transition_to(RunStatus::Blocked));
}

#[test]
fn runtime_retry_approval_reporting_and_cancel_transitions_are_allowed() {
    assert!(RunStatus::Planning.can_transition_to(RunStatus::RetryScheduled));
    assert!(RunStatus::Running.can_transition_to(RunStatus::WaitingForApproval));
    assert!(RunStatus::WaitingForApproval.can_transition_to(RunStatus::Running));
    assert!(RunStatus::Verifying.can_transition_to(RunStatus::Reporting));
    for status in [
        RunStatus::Leased,
        RunStatus::Planning,
        RunStatus::Running,
        RunStatus::Verifying,
        RunStatus::Reporting,
    ] {
        assert!(status.can_transition_to(RunStatus::NeedsRecovery));
    }
    for status in RUN_STATUSES.iter().filter(|status| !status.is_terminal()) {
        assert!(status.can_transition_to(RunStatus::Cancelled));
    }
}

#[test]
fn terminal_states_cannot_be_left() {
    for status in [
        RunStatus::Blocked,
        RunStatus::Succeeded,
        RunStatus::Failed,
        RunStatus::Cancelled,
    ] {
        assert!(status.is_terminal());
        assert!(
            RUN_STATUSES
                .iter()
                .all(|target| !status.can_transition_to(*target))
        );
    }
}

#[test]
fn task_input_is_trimmed_and_bounded() {
    let normalized = CreateTaskRequest {
        title: " Demo ".into(),
        goal: " Test ".into(),
    }
    .normalize()
    .unwrap();
    assert_eq!(normalized.title, "Demo");
    assert_eq!(normalized.goal, "Test");
    assert!(
        CreateTaskRequest {
            title: String::new(),
            goal: "x".into()
        }
        .normalize()
        .is_err()
    );
}

#[test]
fn task_input_limits_count_unicode_characters_inclusively() {
    assert!(
        CreateTaskRequest {
            title: "界".repeat(160),
            goal: "界".repeat(20_000),
        }
        .normalize()
        .is_ok()
    );
    assert!(
        CreateTaskRequest {
            title: "界".repeat(161),
            goal: "x".into(),
        }
        .normalize()
        .is_err()
    );
    assert!(
        CreateTaskRequest {
            title: "x".into(),
            goal: "界".repeat(20_001),
        }
        .normalize()
        .is_err()
    );
}

#[test]
fn usage_snapshot_defaults_cache_counters_for_legacy_json() {
    let usage: UsageSnapshot = serde_json::from_str(
        r#"{"modelTurns":1,"toolCalls":2,"inputTokens":3,"outputTokens":4,"elapsedMs":5}"#,
    )
    .unwrap();
    assert_eq!(usage.cached_input_tokens, 0);
    assert_eq!(usage.cache_write_input_tokens, 0);
}

#[test]
fn default_tool_duration_budget_covers_managed_deployment_sagas() {
    assert_eq!(BudgetSnapshot::default().max_tool_duration_ms, 600_000);
}
