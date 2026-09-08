CREATE TABLE password_rotation_state (
  owner_id TEXT PRIMARY KEY NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  last_rotated_at INTEGER,
  last_email_at INTEGER,
  last_email_error TEXT,
  updated_at INTEGER NOT NULL
);
