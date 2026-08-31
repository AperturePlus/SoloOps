CREATE TABLE ip_notification_settings (
  owner_id TEXT PRIMARY KEY NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  enabled INTEGER NOT NULL DEFAULT 0 CHECK (enabled IN (0, 1)),
  current_ipv4 TEXT,
  last_checked_at INTEGER,
  last_changed_at INTEGER,
  updated_at INTEGER NOT NULL
);

CREATE TABLE ip_notification_recipients (
  owner_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  email TEXT NOT NULL COLLATE NOCASE,
  last_notified_ipv4 TEXT,
  last_notified_at INTEGER,
  last_attempt_at INTEGER,
  last_error TEXT,
  created_at INTEGER NOT NULL,
  PRIMARY KEY (owner_id, email)
);

