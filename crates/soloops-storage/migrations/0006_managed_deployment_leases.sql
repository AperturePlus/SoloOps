ALTER TABLE managed_deployment_operations ADD COLUMN lease_token TEXT;
ALTER TABLE managed_deployment_operations ADD COLUMN lease_expires_at INTEGER;
ALTER TABLE managed_deployment_operations ADD COLUMN recovery_attempts INTEGER NOT NULL DEFAULT 0;
ALTER TABLE managed_deployment_operations ADD COLUMN last_recovery_error TEXT;

CREATE INDEX managed_deployment_expired_lease_idx
  ON managed_deployment_operations(lease_expires_at, created_at)
  WHERE finished_at IS NULL;
