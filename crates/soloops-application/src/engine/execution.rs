use super::*;

enum WriteRecoveryDecision {
    Completed,
    RetrySafe,
    Unsafe(String),
}

enum ToolExecutionFailure {
    Tool(ToolError),
    LeaseLost,
    Cancelled,
}

enum ManagedChangeRecovery {
    Completed(ToolOutput),
    Failed { category: String, summary: String },
    Unresolved(String),
    Missing,
}

impl RuntimeEngine {
    pub(super) async fn execute_pending_tools(
        &self,
        run_id: &str,
        workspace: &str,
        artifacts: &str,
        calls: Vec<RuntimeToolCall>,
        lease_lost: Arc<AtomicBool>,
    ) -> Result<bool, RuntimeError> {
        for call in calls {
            if lease_lost.load(Ordering::Acquire) || self.is_cancelled(run_id).await? {
                return Ok(true);
            }
            let resuming_managed_change = call.summary.status == ToolCallStatus::Running
                && matches!(
                    call.summary.name.as_str(),
                    "managed.deploy.apply" | "managed.deploy.rollback"
                );
            if call.summary.status == ToolCallStatus::Running && call.summary.risk == ToolRisk::WorkspaceWrite
            {
                match self.probe_interrupted_write(run_id, workspace, &call).await? {
                    WriteRecoveryDecision::Completed => continue,
                    WriteRecoveryDecision::RetrySafe => {
                        self.database
                            .reset_tool_call_pending(run_id, &call.summary.call_id)
                            .await?;
                    }
                    WriteRecoveryDecision::Unsafe(reason) => {
                        self.database
                            .transition_run(run_id, RunStatus::Blocked, Some(&reason))
                            .await?;
                        return Ok(true);
                    }
                }
            }
            let decision = policy_for(call.summary.risk);
            if decision == PolicyDecision::Deny {
                self.database
                    .fail_tool_call(
                        run_id,
                        &call.summary.call_id,
                        "policy_denied",
                        "Tool is not available under the Phase 1 policy",
                        true,
                    )
                    .await?;
                continue;
            }
            if decision == PolicyDecision::RequireApproval {
                match call.approval {
                    Some(soloops_domain::ApprovalDecision::Approve)
                        if call.approval_arguments_sha256.as_deref() == Some(&call.arguments_sha256) => {}
                    Some(soloops_domain::ApprovalDecision::Approve) => {
                        self.database
                            .fail_tool_call(
                                run_id,
                                &call.summary.call_id,
                                "approval_mismatch",
                                "Approved arguments no longer match the Tool Call",
                                true,
                            )
                            .await?;
                        continue;
                    }
                    Some(soloops_domain::ApprovalDecision::Deny) => {
                        self.database
                            .fail_tool_call(
                                run_id,
                                &call.summary.call_id,
                                "owner_denied",
                                "Owner denied this tool call",
                                true,
                            )
                            .await?;
                        continue;
                    }
                    None if call.summary.status == ToolCallStatus::WaitingForApproval => return Ok(true),
                    None => {
                        if call.summary.name == "managed.deploy.apply"
                            || call.summary.name == "managed.deploy.rollback"
                        {
                            match self.managed_deployment_approval_preview(&call).await {
                                Ok(preview) => {
                                    self.database
                                        .set_tool_call_approval_preview(
                                            run_id,
                                            &call.summary.call_id,
                                            &preview,
                                        )
                                        .await?;
                                }
                                Err(error) => {
                                    self.database
                                        .fail_tool_call_before_start(
                                            run_id,
                                            &call.summary.call_id,
                                            "invalid_deployment_proposal",
                                            &error,
                                        )
                                        .await?;
                                    continue;
                                }
                            }
                        }
                        self.database
                            .wait_for_tool_approval(run_id, &call.summary.call_id)
                            .await?;
                        return Ok(true);
                    }
                }
            }
            let Some(tool) = self.registry.get(&call.summary.name) else {
                self.database
                    .fail_tool_call(
                        run_id,
                        &call.summary.call_id,
                        "unknown_tool",
                        "Unknown tool name",
                        true,
                    )
                    .await?;
                continue;
            };
            let recovery = if call.summary.risk == ToolRisk::WorkspaceWrite {
                match prepare_workspace_write(Path::new(workspace), &call.summary.name, &call.arguments).await
                {
                    Ok(recovery) => recovery,
                    Err(error) => {
                        self.database
                            .fail_tool_call_before_start(
                                run_id,
                                &call.summary.call_id,
                                error.category(),
                                &error.to_string(),
                            )
                            .await?;
                        continue;
                    }
                }
            } else {
                None
            };
            let recovery_value =
                recovery
                    .as_ref()
                    .map(serde_json::to_value)
                    .transpose()
                    .map_err(|error| {
                        RuntimeError::Invariant(format!("invalid write recovery metadata: {error}"))
                    })?;
            if !resuming_managed_change {
                self.database
                    .start_tool_call_with_recovery(run_id, &call.summary.call_id, recovery_value.as_ref())
                    .await?;
            } else {
                match self.inspect_managed_change_operation(run_id, &call).await? {
                    ManagedChangeRecovery::Completed(output) => {
                        self.finish_tool_output(run_id, &call.summary.call_id, output)
                            .await?;
                        continue;
                    }
                    ManagedChangeRecovery::Failed { category, summary } => {
                        self.database
                            .fail_tool_call(run_id, &call.summary.call_id, &category, &summary, false)
                            .await?;
                        continue;
                    }
                    ManagedChangeRecovery::Unresolved(reason) => {
                        self.block_managed_change(
                            run_id,
                            &call.summary.call_id,
                            "recovery_required",
                            &reason,
                        )
                        .await?;
                        return Ok(true);
                    }
                    ManagedChangeRecovery::Missing => {}
                }
            }
            if call.summary.name == "plan.update" {
                self.execute_plan_update(run_id, &call).await?;
                continue;
            }
            if call.summary.name == "run.finish" {
                return self.execute_finish(run_id, artifacts, &call).await;
            }
            let context = ToolContext {
                run_id: run_id.to_owned(),
                call_id: call.summary.call_id.clone(),
                workspace: PathBuf::from(workspace),
                artifacts: PathBuf::from(artifacts),
                max_output_bytes: self.config.budget.max_tool_output_bytes,
                max_workspace_bytes: self.config.budget.max_workspace_bytes,
            };
            let descriptor = tool.descriptor();
            match self
                .execute_tool_cancellable(
                    run_id,
                    tool,
                    &context,
                    call.arguments.clone(),
                    descriptor.timeout,
                    lease_lost.clone(),
                )
                .await
            {
                Ok(output) => {
                    self.finish_tool_output(run_id, &call.summary.call_id, output)
                        .await?;
                }
                Err(ToolExecutionFailure::Cancelled) => return Ok(true),
                Err(ToolExecutionFailure::LeaseLost) => return Ok(true),
                Err(ToolExecutionFailure::Tool(error)) => {
                    let managed_change = is_managed_change(&call.summary.name);
                    if managed_change {
                        match self.inspect_managed_change_operation(run_id, &call).await? {
                            ManagedChangeRecovery::Completed(output) => {
                                self.finish_tool_output(run_id, &call.summary.call_id, output)
                                    .await?;
                                continue;
                            }
                            ManagedChangeRecovery::Failed { category, summary } => {
                                self.database
                                    .fail_tool_call(run_id, &call.summary.call_id, &category, &summary, false)
                                    .await?;
                                continue;
                            }
                            ManagedChangeRecovery::Unresolved(reason) => {
                                self.block_managed_change(
                                    run_id,
                                    &call.summary.call_id,
                                    error.category(),
                                    &reason,
                                )
                                .await?;
                                return Ok(true);
                            }
                            ManagedChangeRecovery::Missing => {}
                        }
                    }
                    let recovery_required = matches!(error, ToolError::RecoveryRequired(_));
                    if managed_change
                        && (resuming_managed_change
                            || recovery_required
                            || matches!(error, ToolError::Timeout))
                    {
                        self.block_managed_change(
                            run_id,
                            &call.summary.call_id,
                            error.category(),
                            "Managed deployment outcome is unknown because no durable operation result was found; inspect hostd reconciliation",
                        )
                        .await?;
                        return Ok(true);
                    }
                    self.database
                        .fail_tool_call(
                            run_id,
                            &call.summary.call_id,
                            error.category(),
                            &error.to_string(),
                            false,
                        )
                        .await?;
                }
            }
        }
        Ok(false)
    }

    async fn managed_deployment_approval_preview(&self, call: &RuntimeToolCall) -> Result<Value, String> {
        if call.summary.name == "managed.deploy.apply" {
            let revision_id = call
                .arguments
                .get("proposalId")
                .and_then(Value::as_str)
                .ok_or_else(|| "proposalId is required".to_owned())?;
            let proposal_sha256 = call
                .arguments
                .get("proposalSha256")
                .and_then(Value::as_str)
                .ok_or_else(|| "proposalSha256 is required".to_owned())?;
            let revision = self
                .database
                .managed_deployment_revision(revision_id)
                .await
                .map_err(|error| error.to_string())?
                .ok_or_else(|| "deployment proposal was not found".to_owned())?;
            if revision.proposal_sha256 != proposal_sha256 || revision.status != "proposed" {
                return Err("deployment proposal digest or status does not match".into());
            }
            Ok(json!({
                "action": "apply",
                "projectId": revision.project_id,
                "proposalId": revision.id,
                "proposalSha256": revision.proposal_sha256,
                "previousRevisionId": revision.previous_revision_id,
                "changes": revision.preview,
            }))
        } else {
            let project_id = call
                .arguments
                .get("projectId")
                .and_then(Value::as_str)
                .ok_or_else(|| "projectId is required".to_owned())?;
            let expected_current = call
                .arguments
                .get("expectedCurrentRevisionId")
                .and_then(Value::as_str)
                .ok_or_else(|| "expectedCurrentRevisionId is required".to_owned())?;
            let target_id = call
                .arguments
                .get("targetRevisionId")
                .and_then(Value::as_str)
                .ok_or_else(|| "targetRevisionId is required".to_owned())?;
            let current = self
                .database
                .current_managed_deployment_revision(project_id)
                .await
                .map_err(|error| error.to_string())?
                .ok_or_else(|| "project has no active deployment".to_owned())?;
            let target = self
                .database
                .managed_deployment_revision(target_id)
                .await
                .map_err(|error| error.to_string())?
                .ok_or_else(|| "rollback target was not found".to_owned())?;
            if current.id != expected_current
                || target.project_id != project_id
                || !matches!(target.status.as_str(), "active" | "superseded" | "rolled_back")
            {
                return Err("current revision or verified rollback target does not match".into());
            }
            Ok(json!({
                "action": "rollback",
                "projectId": project_id,
                "expectedCurrentRevisionId": expected_current,
                "targetRevisionId": target.id,
                "targetProposalSha256": target.proposal_sha256,
                "changes": target.preview,
            }))
        }
    }

    async fn execute_tool_cancellable(
        &self,
        run_id: &str,
        tool: Arc<dyn Tool>,
        context: &ToolContext,
        arguments: Value,
        timeout: Duration,
        lease_lost: Arc<AtomicBool>,
    ) -> Result<ToolOutput, ToolExecutionFailure> {
        let future = tool.execute(context, arguments);
        tokio::pin!(future);
        let deadline =
            tokio::time::sleep(timeout.min(Duration::from_millis(self.config.budget.max_tool_duration_ms)));
        tokio::pin!(deadline);
        let mut interval = tokio::time::interval(Duration::from_millis(250));
        loop {
            tokio::select! {
                result = &mut future => return result.map_err(ToolExecutionFailure::Tool),
                _ = &mut deadline => return Err(ToolExecutionFailure::Tool(ToolError::Timeout)),
                _ = interval.tick() => {
                    if lease_lost.load(Ordering::Acquire) {
                        return Err(ToolExecutionFailure::LeaseLost);
                    }
                    if self
                        .is_cancelled(run_id)
                        .await
                        .map_err(|error| ToolExecutionFailure::Tool(ToolError::Execution(error.to_string())))?
                    {
                        return Err(ToolExecutionFailure::Cancelled);
                    }
                }
            }
        }
    }

    async fn finish_tool_output(
        &self,
        run_id: &str,
        call_id: &str,
        output: ToolOutput,
    ) -> Result<(), RuntimeError> {
        let evidence = output.evidence.as_ref().map(|item| NewEvidence {
            kind: item.kind.clone(),
            summary: item.summary.clone(),
            artifact_ref: item.artifact_ref.clone(),
            content_sha256: item.content_sha256.clone(),
        });
        self.database
            .finish_tool_call(
                run_id,
                call_id,
                &output.value,
                &output.summary,
                evidence.as_ref(),
                output.increments_workspace,
            )
            .await?;
        Ok(())
    }

    async fn block_managed_change(
        &self,
        run_id: &str,
        call_id: &str,
        category: &str,
        reason: &str,
    ) -> Result<(), RuntimeError> {
        self.database
            .fail_tool_call(run_id, call_id, category, reason, false)
            .await?;
        self.database
            .transition_run(run_id, RunStatus::Blocked, Some(reason))
            .await?;
        Ok(())
    }

    async fn inspect_managed_change_operation(
        &self,
        run_id: &str,
        call: &RuntimeToolCall,
    ) -> Result<ManagedChangeRecovery, RuntimeError> {
        if !is_managed_change(&call.summary.name) {
            return Ok(ManagedChangeRecovery::Missing);
        }
        let Some(operation) = self
            .database
            .managed_deployment_operation(&call.summary.call_id)
            .await?
        else {
            return Ok(ManagedChangeRecovery::Missing);
        };
        let expected_action = if call.summary.name == "managed.deploy.apply" {
            "apply"
        } else {
            "rollback"
        };
        let expected_revision = if expected_action == "apply" {
            call.arguments.get("proposalId").and_then(Value::as_str)
        } else {
            call.arguments.get("targetRevisionId").and_then(Value::as_str)
        };
        let expected_previous = if expected_action == "rollback" {
            call.arguments
                .get("expectedCurrentRevisionId")
                .and_then(Value::as_str)
        } else {
            None
        };
        if !managed_operation_matches(
            &operation,
            run_id,
            expected_action,
            expected_revision,
            expected_previous,
        ) {
            return Ok(ManagedChangeRecovery::Unresolved(
                "Persisted managed deployment operation does not match the authorized Tool Call; inspect hostd reconciliation"
                    .into(),
            ));
        }
        managed_change_recovery(operation, expected_action)
    }

    async fn execute_plan_update(&self, run_id: &str, call: &RuntimeToolCall) -> Result<(), RuntimeError> {
        let plan: AgentPlan = serde_json::from_value(call.arguments.clone())
            .map_err(|error| RuntimeError::Invariant(format!("invalid plan.update arguments: {error}")))?;
        let valid = plan.steps.len() <= 100
            && plan
                .steps
                .iter()
                .all(|step| !step.id.trim().is_empty() && !step.title.trim().is_empty())
            && {
                let mut ids = std::collections::HashSet::new();
                plan.steps.iter().all(|step| ids.insert(step.id.as_str()))
            };
        if !valid {
            self.database
                .fail_tool_call(
                    run_id,
                    &call.summary.call_id,
                    "invalid_arguments",
                    "Plan steps must have unique non-empty IDs and titles",
                    false,
                )
                .await?;
            return Ok(());
        }
        self.database.save_plan(run_id, &plan).await?;
        self.database
            .finish_tool_call(
                run_id,
                &call.summary.call_id,
                &json!({"accepted": true}),
                "Plan updated",
                None,
                false,
            )
            .await?;
        Ok(())
    }

    async fn execute_finish(
        &self,
        run_id: &str,
        artifacts: &str,
        call: &RuntimeToolCall,
    ) -> Result<bool, RuntimeError> {
        let request: FinishRequest = match serde_json::from_value(call.arguments.clone()) {
            Ok(value) => value,
            Err(error) => {
                self.database
                    .fail_tool_call(
                        run_id,
                        &call.summary.call_id,
                        "invalid_arguments",
                        &error.to_string(),
                        false,
                    )
                    .await?;
                return Ok(false);
            }
        };
        self.database
            .transition_run(run_id, RunStatus::Verifying, None)
            .await?;
        self.database
            .set_runtime_checkpoint(run_id, RuntimeCheckpoint::ValidatingCompletion)
            .await?;
        let snapshot = self
            .database
            .runtime_snapshot(run_id)
            .await?
            .ok_or_else(|| RuntimeError::Invariant("runtime snapshot disappeared".to_owned()))?;
        let mut gaps = Vec::new();
        if snapshot.tool_calls.iter().any(|item| {
            item.call_id != call.summary.call_id
                && matches!(
                    item.status,
                    ToolCallStatus::Pending
                        | ToolCallStatus::WaitingForApproval
                        | ToolCallStatus::Running
                        | ToolCallStatus::Unknown
                )
        }) {
            gaps.push("unresolved tool calls remain".to_owned());
        }
        if snapshot
            .plan
            .steps
            .iter()
            .any(|step| step.required && step.status != PlanStepStatus::Completed)
        {
            gaps.push("required plan steps are incomplete".to_owned());
        }
        if snapshot.plan.summary.trim().is_empty() || snapshot.plan.steps.is_empty() {
            gaps.push("a persisted execution plan is required".to_owned());
        }
        if request.evidence_ids.is_empty() {
            gaps.push("at least one Evidence ID is required".to_owned());
        }
        if request.outcome != "succeeded" {
            gaps.push("finish outcome must be succeeded".to_owned());
        }
        let referenced = snapshot
            .evidence
            .iter()
            .filter(|item| request.evidence_ids.contains(&item.id))
            .collect::<Vec<_>>();
        if referenced.len() != request.evidence_ids.len() {
            gaps.push("one or more evidence IDs are invalid".to_owned());
        }
        for item in &referenced {
            if let Some(reference) = &item.artifact_ref {
                let relative = PathBuf::from(reference);
                if relative.is_absolute()
                    || relative.components().any(|part| {
                        matches!(
                            part,
                            std::path::Component::ParentDir
                                | std::path::Component::RootDir
                                | std::path::Component::Prefix(_)
                        )
                    })
                    || !tokio::fs::metadata(PathBuf::from(artifacts).join(relative))
                        .await
                        .is_ok_and(|metadata| metadata.is_file())
                {
                    gaps.push(format!("evidence artifact {} is not readable", item.id));
                }
            }
        }
        if snapshot.workspace_revision > 0
            && !referenced.iter().any(|item| {
                item.workspace_revision == snapshot.workspace_revision
                    && matches!(item.kind.as_str(), "file_snapshot" | "command_result")
            })
        {
            gaps.push("latest workspace revision has no referenced verification evidence".to_owned());
        }
        if !gaps.is_empty() {
            let message = gaps.join("; ");
            self.database
                .fail_tool_call(run_id, &call.summary.call_id, "verification_gap", &message, false)
                .await?;
            self.database
                .transition_run(run_id, RunStatus::Running, Some(&message))
                .await?;
            return Ok(false);
        }
        self.database
            .finish_tool_call(
                run_id,
                &call.summary.call_id,
                &json!({"verified": true}),
                "Completion verified",
                None,
                false,
            )
            .await?;
        let markdown = render_report(&request, &snapshot.usage);
        let report = FinalReport {
            outcome: "succeeded".to_owned(),
            summary: request.summary,
            completed: request.completed,
            incomplete: request.incomplete,
            risks: request.risks,
            evidence_ids: request.evidence_ids,
            rollback: request.rollback,
            usage: snapshot.usage,
            markdown,
        };
        self.database.save_final_report(run_id, &report).await?;
        Ok(true)
    }

    async fn probe_interrupted_write(
        &self,
        run_id: &str,
        workspace: &str,
        call: &RuntimeToolCall,
    ) -> Result<WriteRecoveryDecision, RuntimeError> {
        let argument_path = call.arguments.get("path").and_then(Value::as_str);
        let Some(argument_path) = argument_path else {
            return Ok(WriteRecoveryDecision::Unsafe(
                "Interrupted workspace write has no valid path".to_owned(),
            ));
        };
        let recovery = match call
            .recovery
            .clone()
            .map(serde_json::from_value::<WorkspaceWriteRecovery>)
            .transpose()
        {
            Ok(recovery) => recovery,
            Err(error) => {
                return Ok(WriteRecoveryDecision::Unsafe(format!(
                    "Interrupted workspace write has invalid recovery metadata: {error}"
                )));
            }
        };
        let path = if let Some(recovery) = recovery.as_ref() {
            let expected_operation = call.summary.name.strip_prefix("workspace.");
            let normalized_argument = match normalized_workspace_write_path(argument_path) {
                Ok(path) => path,
                Err(_) => {
                    return Ok(WriteRecoveryDecision::Unsafe(
                        "Interrupted workspace write path is unsafe".to_owned(),
                    ));
                }
            };
            if !matches!(recovery.operation.as_str(), "create" | "replace")
                || expected_operation != Some(recovery.operation.as_str())
                || normalized_argument != recovery.path
            {
                return Ok(WriteRecoveryDecision::Unsafe(
                    "Interrupted workspace write recovery metadata does not match the call".to_owned(),
                ));
            }
            recovery.path.as_str()
        } else {
            argument_path
        };
        if PathBuf::from(path).is_absolute() || path.split(['/', '\\']).any(|part| part == "..") {
            return Ok(WriteRecoveryDecision::Unsafe(
                "Interrupted workspace write path is unsafe".to_owned(),
            ));
        }
        let target = PathBuf::from(workspace).join(path);

        let content = match tokio::fs::read(&target).await {
            Ok(content) => Some(content),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error.into()),
        };

        if let Some(recovery) = recovery {
            if let Some(content) = content.as_deref() {
                let current_sha256 = format!("{:x}", Sha256::digest(content));
                if current_sha256 == recovery.expected_sha256 && content.len() == recovery.bytes {
                    self.finish_recovered_write(run_id, call, &recovery).await?;
                    return Ok(WriteRecoveryDecision::Completed);
                }
                if recovery.previous_sha256.as_deref() == Some(&current_sha256) {
                    self.cleanup_interrupted_write(&target, &call.summary.call_id)
                        .await?;
                    return Ok(WriteRecoveryDecision::RetrySafe);
                }
            } else if recovery.operation == "create" {
                self.cleanup_interrupted_write(&target, &call.summary.call_id)
                    .await?;
                return Ok(WriteRecoveryDecision::RetrySafe);
            }
            return Ok(WriteRecoveryDecision::Unsafe(format!(
                "Interrupted {} for {} has unknown workspace contents",
                recovery.operation, recovery.path
            )));
        }

        let expected = call.arguments.get("content").and_then(Value::as_str);
        if call.summary.name == "workspace.create" {
            let Some(expected) = expected else {
                return Ok(WriteRecoveryDecision::Unsafe(
                    "Interrupted create has no expected content".to_owned(),
                ));
            };
            match content.as_deref() {
                None => {
                    self.cleanup_interrupted_write(&target, &call.summary.call_id)
                        .await?;
                    return Ok(WriteRecoveryDecision::RetrySafe);
                }
                Some(current) if current == expected.as_bytes() => {
                    let recovered = legacy_recovery("create", path, expected, None);
                    self.finish_recovered_write(run_id, call, &recovered).await?;
                    return Ok(WriteRecoveryDecision::Completed);
                }
                Some(current)
                    if current.len() < expected.len() && expected.as_bytes().starts_with(current) =>
                {
                    tokio::fs::remove_file(&target).await?;
                    self.cleanup_interrupted_write(&target, &call.summary.call_id)
                        .await?;
                    return Ok(WriteRecoveryDecision::RetrySafe);
                }
                Some(_) => {}
            }
            return Ok(WriteRecoveryDecision::Unsafe(format!(
                "Interrupted create for {path} found unexpected content"
            )));
        }

        if let (Some(expected), Some(current)) = (expected, content.as_deref())
            && current == expected.as_bytes()
        {
            let recovered = legacy_recovery("replace", path, expected, None);
            self.finish_recovered_write(run_id, call, &recovered).await?;
            return Ok(WriteRecoveryDecision::Completed);
        }
        if let (Some(old), Some(current)) = (
            call.arguments.get("oldContent").and_then(Value::as_str),
            content.as_deref(),
        ) && String::from_utf8_lossy(current).matches(old).count() == 1
        {
            self.cleanup_interrupted_write(&target, &call.summary.call_id)
                .await?;
            return Ok(WriteRecoveryDecision::RetrySafe);
        }
        Ok(WriteRecoveryDecision::Unsafe(format!(
            "Interrupted replace for {path} cannot be recovered safely"
        )))
    }

    async fn finish_recovered_write(
        &self,
        run_id: &str,
        call: &RuntimeToolCall,
        recovery: &WorkspaceWriteRecovery,
    ) -> Result<(), RuntimeError> {
        let evidence = NewEvidence {
            kind: "file_mutation".to_owned(),
            summary: recovery.evidence_summary(),
            artifact_ref: None,
            content_sha256: Some(recovery.expected_sha256.clone()),
        };
        self.database
            .finish_tool_call(
                run_id,
                &call.summary.call_id,
                &recovery.result(),
                &recovery.summary(),
                Some(&evidence),
                true,
            )
            .await?;
        Ok(())
    }

    async fn cleanup_interrupted_write(&self, target: &Path, call_id: &str) -> Result<(), RuntimeError> {
        let Some(parent) = target.parent() else {
            return Ok(());
        };
        let prefix = workspace_temp_prefix(call_id);
        let mut entries = match tokio::fs::read_dir(parent).await {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error.into()),
        };
        while let Some(entry) = entries.next_entry().await? {
            if entry.file_name().to_string_lossy().starts_with(&prefix) {
                tokio::fs::remove_file(entry.path()).await?;
            }
        }
        Ok(())
    }

    pub(super) async fn is_cancelled(&self, run_id: &str) -> Result<bool, RuntimeError> {
        Ok(self
            .database
            .get_run(run_id)
            .await?
            .is_some_and(|run| run.status == RunStatus::Cancelled))
    }
}

fn is_managed_change(name: &str) -> bool {
    matches!(name, "managed.deploy.apply" | "managed.deploy.rollback")
}

fn managed_operation_matches(
    operation: &ManagedDeploymentOperation,
    run_id: &str,
    expected_action: &str,
    expected_revision: Option<&str>,
    expected_previous: Option<&str>,
) -> bool {
    operation.run_id == run_id
        && operation.action == expected_action
        && expected_revision == Some(operation.revision_id.as_str())
        && (expected_action != "rollback" || expected_previous == operation.previous_revision_id.as_deref())
}

fn managed_change_recovery(
    operation: ManagedDeploymentOperation,
    expected_action: &str,
) -> Result<ManagedChangeRecovery, RuntimeError> {
    if operation.finished_at.is_none() {
        return Ok(ManagedChangeRecovery::Unresolved(format!(
            "Managed deployment operation is still in phase {}; inspect hostd reconciliation",
            operation.phase
        )));
    }
    if let Some(category) = operation.error_category {
        let summary = operation
            .result
            .as_ref()
            .and_then(|result| result.get("failure"))
            .and_then(Value::as_str)
            .map(|failure| format!("Persisted managed deployment failure: {failure}"))
            .unwrap_or_else(|| "Persisted managed deployment operation failed after compensation".into());
        return Ok(ManagedChangeRecovery::Failed { category, summary });
    }
    let Some(value) = operation.result else {
        return Ok(ManagedChangeRecovery::Unresolved(
            "Finished managed deployment operation has no durable result; inspect hostd reconciliation"
                .into(),
        ));
    };
    let result: soloops_hostd_protocol::ManagedDeployResult = match serde_json::from_value(value) {
        Ok(result) => result,
        Err(_) => {
            return Ok(ManagedChangeRecovery::Unresolved(
                "Persisted managed deployment result is invalid; inspect hostd reconciliation".into(),
            ));
        }
    };
    if result.action != expected_action
        || result.project_id != operation.project_id
        || result.revision_id.as_deref() != Some(operation.revision_id.as_str())
        || result.status != "active"
    {
        return Ok(ManagedChangeRecovery::Unresolved(
            "Persisted managed deployment result does not match its operation; inspect hostd reconciliation"
                .into(),
        ));
    }
    Ok(ManagedChangeRecovery::Completed(managed_deploy_tool_output(
        HostManagedDeployOutput {
            action: result.action,
            project_id: result.project_id,
            status: result.status,
            revision_id: result.revision_id,
            previous_revision_id: result.previous_revision_id,
            proposal_sha256: result.proposal_sha256,
            preview: result.preview,
        },
        "managed_deployment",
    )))
}

fn legacy_recovery(
    operation: &str,
    path: &str,
    content: &str,
    previous_sha256: Option<String>,
) -> WorkspaceWriteRecovery {
    WorkspaceWriteRecovery {
        operation: operation.to_owned(),
        path: path.replace('\\', "/"),
        previous_sha256,
        expected_sha256: format!("{:x}", Sha256::digest(content.as_bytes())),
        bytes: content.len(),
    }
}

#[cfg(test)]
mod managed_change_tests {
    use super::*;

    fn operation() -> ManagedDeploymentOperation {
        ManagedDeploymentOperation {
            call_id: "call-1".into(),
            run_id: "run-1".into(),
            project_id: "demo".into(),
            action: "apply".into(),
            revision_id: "revision-1".into(),
            previous_revision_id: None,
            phase: "completed".into(),
            result: Some(json!({
                "action": "apply",
                "projectId": "demo",
                "status": "active",
                "revisionId": "revision-1",
                "previousRevisionId": null,
                "proposalSha256": "a".repeat(64),
                "preview": {"site": "demo.example"},
            })),
            error_category: None,
            lease_token: None,
            lease_expires_at: None,
            recovery_attempts: 0,
            last_recovery_error: None,
            finished_at: Some(1),
        }
    }

    #[test]
    fn durable_success_is_converted_to_managed_deployment_evidence() {
        let recovery = managed_change_recovery(operation(), "apply").unwrap();
        let ManagedChangeRecovery::Completed(output) = recovery else {
            panic!("expected completed recovery");
        };
        assert_eq!(output.value["status"], "active");
        assert_eq!(output.evidence.unwrap().kind, "managed_deployment");
    }

    #[test]
    fn durable_failure_is_safe_to_report_as_an_ordinary_tool_failure() {
        let mut operation = operation();
        operation.phase = "failed".into();
        operation.error_category = Some("deployment_failed".into());
        operation.result = Some(json!({"failure": "health check failed"}));
        let recovery = managed_change_recovery(operation, "apply").unwrap();
        let ManagedChangeRecovery::Failed { category, summary } = recovery else {
            panic!("expected failed recovery");
        };
        assert_eq!(category, "deployment_failed");
        assert!(summary.contains("health check failed"));
    }

    #[test]
    fn unfinished_operation_requires_explicit_reconciliation() {
        let mut operation = operation();
        operation.phase = "verifying_health".into();
        operation.finished_at = None;
        operation.result = None;
        let recovery = managed_change_recovery(operation, "apply").unwrap();
        let ManagedChangeRecovery::Unresolved(reason) = recovery else {
            panic!("expected unresolved recovery");
        };
        assert!(reason.contains("verifying_health"));
    }

    #[test]
    fn authorized_call_must_match_the_persisted_operation() {
        let operation = operation();
        assert!(managed_operation_matches(
            &operation,
            "run-1",
            "apply",
            Some("revision-1"),
            None,
        ));
        assert!(!managed_operation_matches(
            &operation,
            "other-run",
            "apply",
            Some("revision-1"),
            None,
        ));
        assert!(!managed_operation_matches(
            &operation,
            "run-1",
            "rollback",
            Some("revision-1"),
            Some("current-revision"),
        ));
    }

    #[test]
    fn durable_rollback_success_is_recovered_as_evidence() {
        let mut operation = operation();
        operation.action = "rollback".into();
        operation.previous_revision_id = Some("current-revision".into());
        operation.result = Some(json!({
            "action": "rollback",
            "projectId": "demo",
            "status": "active",
            "revisionId": "revision-1",
            "previousRevisionId": "current-revision",
            "proposalSha256": "a".repeat(64),
            "preview": {"site": "demo.example"},
        }));
        let recovery = managed_change_recovery(operation, "rollback").unwrap();
        let ManagedChangeRecovery::Completed(output) = recovery else {
            panic!("expected completed rollback recovery");
        };
        assert_eq!(output.value["action"], "rollback");
        assert_eq!(output.evidence.unwrap().kind, "managed_deployment");
    }
}
