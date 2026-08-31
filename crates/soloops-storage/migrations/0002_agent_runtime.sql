CREATE TABLE agent_sessions (
  run_id TEXT PRIMARY KEY NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
  provider TEXT NOT NULL,
  model TEXT NOT NULL,
  prompt_version TEXT NOT NULL,
  workspace_path TEXT NOT NULL,
  artifact_path TEXT NOT NULL,
  checkpoint TEXT NOT NULL,
  plan_json TEXT NOT NULL,
  budget_json TEXT NOT NULL,
  usage_json TEXT NOT NULL,
  workspace_revision INTEGER NOT NULL DEFAULT 0,
  retry_at INTEGER,
  consecutive_no_progress INTEGER NOT NULL DEFAULT 0,
  consecutive_protocol_errors INTEGER NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);

CREATE TABLE agent_items (
  id TEXT PRIMARY KEY NOT NULL,
  run_id TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
  sequence INTEGER NOT NULL,
  kind TEXT NOT NULL,
  payload TEXT NOT NULL,
  provider_request_id TEXT,
  created_at INTEGER NOT NULL,
  UNIQUE(run_id, sequence)
);
CREATE INDEX agent_items_run_sequence_idx ON agent_items(run_id, sequence);

CREATE TABLE model_attempts (
  id TEXT PRIMARY KEY NOT NULL,
  run_id TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
  request_key TEXT NOT NULL,
  attempt INTEGER NOT NULL,
  status TEXT NOT NULL,
  provider_request_id TEXT,
  error_category TEXT,
  started_at INTEGER NOT NULL,
  completed_at INTEGER,
  UNIQUE(run_id, request_key, attempt)
);
CREATE INDEX model_attempts_run_started_idx ON model_attempts(run_id, started_at);

CREATE TABLE tool_calls (
  call_id TEXT PRIMARY KEY NOT NULL,
  run_id TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
  item_id TEXT REFERENCES agent_items(id) ON DELETE SET NULL,
  ordinal INTEGER NOT NULL,
  name TEXT NOT NULL,
  arguments_json TEXT NOT NULL,
  arguments_sha256 TEXT NOT NULL,
  risk TEXT NOT NULL,
  policy_decision TEXT NOT NULL,
  status TEXT NOT NULL,
  idempotency_key TEXT NOT NULL,
  result_summary TEXT,
  result_json TEXT,
  error_category TEXT,
  workspace_revision_before INTEGER NOT NULL,
  workspace_revision_after INTEGER,
  created_at INTEGER NOT NULL,
  started_at INTEGER,
  completed_at INTEGER
);
CREATE INDEX tool_calls_run_created_idx ON tool_calls(run_id, created_at);

CREATE TABLE tool_approvals (
  call_id TEXT PRIMARY KEY NOT NULL REFERENCES tool_calls(call_id) ON DELETE CASCADE,
  run_id TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
  owner_id TEXT NOT NULL REFERENCES users(id),
  arguments_sha256 TEXT NOT NULL,
  decision TEXT NOT NULL,
  reason TEXT,
  created_at INTEGER NOT NULL
);
CREATE INDEX tool_approvals_run_created_idx ON tool_approvals(run_id, created_at);

CREATE TABLE evidence (
  id TEXT PRIMARY KEY NOT NULL,
  run_id TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
  tool_call_id TEXT NOT NULL REFERENCES tool_calls(call_id) ON DELETE CASCADE,
  kind TEXT NOT NULL,
  summary TEXT NOT NULL,
  artifact_ref TEXT,
  content_sha256 TEXT,
  workspace_revision INTEGER NOT NULL,
  created_at INTEGER NOT NULL
);
CREATE INDEX evidence_run_created_idx ON evidence(run_id, created_at);
