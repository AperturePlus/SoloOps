ALTER TABLE tool_calls ADD COLUMN approval_preview_json TEXT;

CREATE TABLE managed_deployments (
  project_id TEXT PRIMARY KEY NOT NULL,
  current_revision_id TEXT,
  updated_at INTEGER NOT NULL
);

CREATE TABLE managed_deployment_revisions (
  id TEXT PRIMARY KEY NOT NULL,
  project_id TEXT NOT NULL REFERENCES managed_deployments(project_id) ON DELETE CASCADE,
  run_id TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
  plan_call_id TEXT NOT NULL REFERENCES tool_calls(call_id) ON DELETE CASCADE,
  previous_revision_id TEXT,
  proposal_sha256 TEXT NOT NULL,
  compose_sha256 TEXT NOT NULL,
  caddy_sha256 TEXT NOT NULL,
  source_json TEXT NOT NULL,
  preview_json TEXT NOT NULL,
  bundle_path TEXT NOT NULL,
  status TEXT NOT NULL,
  health_json TEXT,
  created_at INTEGER NOT NULL,
  activated_at INTEGER,
  finished_at INTEGER
);
CREATE INDEX managed_deployment_revisions_project_created_idx
  ON managed_deployment_revisions(project_id, created_at);
CREATE UNIQUE INDEX managed_deployment_revision_plan_call_idx
  ON managed_deployment_revisions(plan_call_id);

CREATE TABLE managed_deployment_operations (
  call_id TEXT PRIMARY KEY NOT NULL REFERENCES tool_calls(call_id) ON DELETE CASCADE,
  run_id TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
  project_id TEXT NOT NULL REFERENCES managed_deployments(project_id) ON DELETE CASCADE,
  action TEXT NOT NULL,
  revision_id TEXT NOT NULL REFERENCES managed_deployment_revisions(id),
  previous_revision_id TEXT,
  phase TEXT NOT NULL,
  result_json TEXT,
  error_category TEXT,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  finished_at INTEGER
);
CREATE UNIQUE INDEX managed_deployment_active_operation_idx
  ON managed_deployment_operations(project_id)
  WHERE finished_at IS NULL;
