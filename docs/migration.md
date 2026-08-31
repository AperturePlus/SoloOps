# TypeScript-to-Rust migration

## Version-control checkpoints

- `0dd7591`: original Bun/Fastify/Drizzle Phase 0 baseline.
- `a76267b`: additive Rust control plane, Worker, storage and CLI.
- The final migration commit removes the TypeScript backend after parity checks.

The baseline remains reachable through Git; no compatibility code is required solely to retain deleted source.

## Database compatibility

The Rust migration runner creates `soloops_schema_migrations`. On a new database it applies the Rust baseline SQL. On an existing TypeScript Phase 0 database it:

1. detects the existing `users` table;
2. verifies all required Phase 0 tables;
3. records migration version 1 with `adopted = 1`;
4. preserves existing users, sessions, tasks, runs, events and audit logs.

The legacy `__drizzle_migrations` table may remain; Rust does not depend on it.

## Agent Runtime v2 migration

Migration version 2 is additive. It creates `agent_sessions`, `agent_items`, `model_attempts`,
`tool_calls`, `tool_approvals` and `evidence` without rewriting Phase 0 tables. Existing queued and
terminal Runs remain readable; Runtime rows are created lazily when a Worker first claims a Run.
Rollback to Phase 0 binaries requires restoring the pre-v2 database backup because older binaries
do not verify the additional Runtime Schema version.

## Workspace write recovery v3 migration

Migration version 3 adds the nullable `tool_calls.recovery_json` column. New workspace writes save
their pre-write and expected SHA-256 metadata there before entering `running`, which lets a Worker
distinguish a completed atomic publication from a safe retry or an unknown external modification
after lease recovery. Existing Tool Call rows are preserved with `NULL` recovery metadata and use
the stricter legacy recovery rules.

As with v2, rollback requires restoring a pre-v3 database backup before running an older binary;
the migration runner validates the new column as part of the current schema.

## Operator procedure

1. Stop the TypeScript API and Worker.
2. Back up the SQLite database, WAL and SHM consistently.
3. Build or install the Rust binaries.
4. Run:

   ```powershell
   cargo run -p soloopsctl -- migrate
   cargo run -p soloopsctl -- doctor
   ```

5. Start Rust API and Worker.
6. Verify login, task listing, event replay and `/readyz`.
7. Keep the backup until the acceptance window closes.

## Rollback

The migration does not rewrite Phase 0 application tables. Before future destructive migrations, rollback requires restoring the pre-migration backup and the previous binaries. Do not attempt source-level rollback while continuing to use a database changed by an incompatible later migration.

## Version 4: public IPv4 notifications

Migration v4 adds `ip_notification_settings` and `ip_notification_recipients`. It does not alter
existing task/runtime tables. Each recipient keeps independent delivery state so a partial SMTP
failure can be retried without duplicating successful deliveries. Older binaries require restoring a
pre-v4 database backup before rollback.

## Version 5: managed deployments

Migration v5 adds an approval preview to `tool_calls` plus durable managed project, revision, and
operation tables. It is additive and preserves existing Runtime and notification rows. Stop Worker
and hostd, run `soloopsctl migrate`, deploy both protocol V3 binaries, then restart hostd before
Worker. Rolling back binaries requires restoring the pre-v5 database backup.

## Version 6: managed deployment leases

Migration v6 adds operation ownership, lease expiry, recovery-attempt, and last-error fields to
managed deployment operations. Existing unfinished v5 operations have no lease and are therefore
eligible for immediate reconciliation by the upgraded hostd. Stop Worker and hostd, run
`soloopsctl migrate`, then start hostd before Worker. Older binaries must not be used against a
database after v6 recovery has begun; restore the pre-v6 backup to roll back.

## Removed TypeScript backend

The following runtime areas were removed:

- Fastify API and authentication;
- Bun Worker;
- Drizzle/bun:sqlite repositories and migration runner;
- server-shared environment/logging package;
- browser-safe contract workspace package;
- Bun multi-process development launcher.

Frontend TypeScript remains because it is compiled to static browser assets and is not a backend runtime.
