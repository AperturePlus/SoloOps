CREATE TABLE model_settings (
  owner_id TEXT PRIMARY KEY NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  base_url TEXT NOT NULL,
  model_name TEXT NOT NULL,
  api_key TEXT,
  updated_at INTEGER NOT NULL
);
