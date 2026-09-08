CREATE TABLE smtp_settings (
  owner_id TEXT PRIMARY KEY NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  host TEXT NOT NULL,
  port INTEGER NOT NULL,
  security TEXT NOT NULL CHECK (security IN ('tls', 'starttls')),
  from_mailbox TEXT NOT NULL,
  username TEXT,
  password TEXT,
  updated_at INTEGER NOT NULL
);
