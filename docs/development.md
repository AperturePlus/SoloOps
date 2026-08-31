# Development and testing

## Setup

```powershell
Copy-Item .env.example .env
bun install --frozen-lockfile
cargo run -p soloopsctl -- migrate
cargo run -p soloopsctl -- owner-init
```

Run API, Worker and WebUI in separate terminals:

```powershell
cargo run -p soloops-api
cargo run -p soloops-worker
bun run dev:web
```

Vite proxies `/api` and WebSocket upgrades to port 3001.

## Required checks

```powershell
cargo fmt --all -- --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
bun run lint
bun run --cwd apps/web check
bun run test:mock
bun run --cwd apps/web build
bun run test:e2e
```

Avoid tests against `var/db/soloops.db`. Rust tests use isolated in-memory or temporary SQLite databases.

## Testing strategy

- Domain tests cover all terminal states and Runtime transitions.
- Storage tests cover migration adoption, ordered events, exclusive leasing, and approval writes during idle Worker polling.
- Application tests use fake providers and prove budget, progress, Tool, approval and verification behavior.
- hostd tests use a replaceable Sandbox driver and inspect the generated Docker security configuration without requiring a daemon.
- HTTP tests cover authentication, cookies, Origin rejection, rate limiting and the persisted lifecycle.
- Frontend checks cover Svelte and TypeScript diagnostics.
- A release smoke test should migrate a temporary file database, build the SPA, launch the API and query `/readyz`.
- `bun run test:e2e` builds the SPA, launches the non-release Rust E2E harness, and drives the Owner approval/report flow in Chromium. The harness reports background Worker errors to teardown and exits gracefully; a page-level pass cannot hide SQLite or Worker failures. Local Windows runs reuse the installed Chrome channel; CI installs Playwright Chromium.

The regular `CI` workflow runs on every push and pull request. Rust formatting, tests, and Clippy run
on Ubuntu and Windows; WebUI checks and browser E2E run in isolated Ubuntu jobs. Do not run a
standalone WebUI build concurrently with `bun run test:e2e` in the same checkout because both own
`.svelte-kit/output`.

The live Sandbox integration test is Linux-only, ignored by default, never pulls an image and requires an operator-provided digest-pinned image containing `/bin/cp` (or an alternative path in `SOLOOPS_TEST_SANDBOX_COPY_PROGRAM`):

```bash
SOLOOPS_TEST_SANDBOX_IMAGE='registry.example/sandbox@sha256:<64-hex-digest>' \
  cargo test -p soloops-hostd docker_exec_writes_workspace_and_leaves_no_container -- --ignored
```

The test writes through `/workspace`, verifies the result, removes the container twice to prove idempotent cleanup, and must be part of Linux release acceptance when Sandbox support is enabled.

Managed deployment release acceptance is Linux-only. Use a disposable site and loopback port,
preload the test image by digest, and configure temporary Caddy files. The release job must prove a
healthy apply, a deliberately unhealthy revision, automatic restoration of the prior revision, and
idempotent cleanup without pulling an image.

```bash
SOLOOPS_TEST_MANAGED_COMPOSE_GOOD=/tmp/soloops-good.yaml \
SOLOOPS_TEST_MANAGED_COMPOSE_BAD=/tmp/soloops-bad.yaml \
  cargo test -p soloops-hostd live_compose_failure_can_restore_the_previous_revision_without_pull -- --ignored
```

The manual `Linux release acceptance` GitHub Actions workflow is the canonical release gate. It
preloads a fixed BusyBox digest, installs a fixed checksum-verified Caddy release, runs the live
Sandbox test, then exercises the complete managed deployment Saga through Docker Compose, Caddy
reload, loopback route probing, failed health validation, automatic restoration, status verification,
and repeated cleanup. Image downloads occur only during workflow setup; the tests and deployment
code use `--pull never`.

## Adding a database migration

1. Add the new SQL file under `crates/soloops-storage/migrations`.
2. Register its version, name and checksum in the migration runner.
3. Write a test from the previous released Schema.
4. Prefer additive Schema changes and backfill before deleting old fields.
5. Update `docs/migration.md` and the data model in `设计.md`.

Application startup must continue to verify, not apply, migrations.

## Adding an API field

Update all of:

1. `crates/soloops-domain`;
2. storage mapping;
3. API handler tests;
4. `apps/web/src/lib/contracts.ts`;
5. `docs/api/openapi.yaml`.

Contract drift is considered a failing change even when Rust compilation succeeds.

## Development-only facilities

Fault injection, fake models and debug endpoints must be behind test-only code or explicit non-default Cargo features. They must not be enabled in release packaging. Tests themselves stay in the repository.

The browser harness is the `soloops-server` example gated by `e2e-harness`. Its scripted model, fixed test password, background-error status, and shutdown endpoints are therefore absent from normal and release binaries.
