# Operations and observability

## Startup order

1. Back up the SQLite database.
2. Run `soloopsctl migrate`.
3. Run `soloopsctl doctor`.
4. Start `soloops-api`.
5. Start `soloops-hostd` when Process, Sandbox, or Managed Deployment Tools are enabled.
6. Start `soloops-worker`.
7. Confirm `/readyz`.

The API and Worker fail startup when the expected Schema is missing.

## Configuration

| Variable                                 | Default                        | Purpose                                                               |
| ---------------------------------------- | ------------------------------ | --------------------------------------------------------------------- |
| `SOLOOPS_ENV`                            | `development`                  | Enables secure production cookies when set to `production`            |
| `SOLOOPS_HOST`                           | `127.0.0.1`                    | API bind address                                                      |
| `SOLOOPS_API_PORT`                       | `3001`                         | API port                                                              |
| `SOLOOPS_WEB_ORIGIN`                     | `http://127.0.0.1:5173`        | Allowed mutation Origin                                               |
| `SOLOOPS_DATABASE_PATH`                  | `var/db/soloops.db`            | SQLite file                                                           |
| `SOLOOPS_WEB_DIST`                       | `apps/web/build`               | Static SPA directory                                                  |
| `SOLOOPS_SESSION_TTL_HOURS`              | `24`                           | Session lifetime                                                      |
| `SOLOOPS_EVENT_POLL_MS`                  | `500`                          | WebSocket durable event poll                                          |
| `SOLOOPS_WORKER_POLL_MS`                 | `1000`                         | Empty queue poll                                                      |
| `SOLOOPS_LOG`                            | `soloops=info,tower_http=info` | tracing filter                                                        |
| `SOLOOPS_LOG_FORMAT`                     | `json`                         | Use `pretty` for local output                                         |
| `SOLOOPS_METRICS_ENABLED`                | `true`                         | Enable `/metrics`                                                     |
| `SOLOOPS_PUBLIC_IP_ENDPOINT`             | `https://api.ipify.org`        | Plain-text public IPv4 detection endpoint                             |
| `SOLOOPS_PUBLIC_IP_POLL_SECONDS`         | `300`                          | Public IPv4 check interval, from 60 to 86400 seconds                  |
| `SOLOOPS_SMTP_HOST`                      | unset                          | SMTP relay hostname; leaving all SMTP variables unset disables mail   |
| `SOLOOPS_SMTP_PORT`                      | `465`/`587`                    | Defaults from the selected TLS mode                                   |
| `SOLOOPS_SMTP_SECURITY`                  | `tls`                          | `tls` for implicit TLS or `starttls` for required STARTTLS            |
| `SOLOOPS_SMTP_FROM`                      | required with SMTP             | Sender mailbox, optionally with a display name                        |
| `SOLOOPS_SMTP_USERNAME`                  | unset                          | Optional SMTP username; requires a password reference                 |
| `SOLOOPS_SMTP_PASSWORD_REF`              | unset                          | `env:<name>` reference paired with the SMTP username                  |
| `SOLOOPS_MODEL_BASE_URL`                 | `https://api.openai.com/v1`    | OpenAI-compatible API base URL                                        |
| `SOLOOPS_MODEL_NAME`                     | required                       | Production model name                                                 |
| `SOLOOPS_MODEL_API_KEY_REF`              | `env:OPENAI_API_KEY`           | Typed reference to the model API key; v1 supports `env:`              |
| `SOLOOPS_PROMPT_CACHE_KEY_ENABLED`       | `false`                        | Send OpenAI `prompt_cache_key`; enable only on compatible endpoints   |
| `SOLOOPS_MAX_TOOL_DURATION_MS`           | `600000`                       | Worker ceiling for one Tool Call; individual descriptors may be lower |
| `SOLOOPS_WORKSPACE_ROOT`                 | `var/workspaces`               | Per-Run isolated Workspace root                                       |
| `SOLOOPS_ARTIFACT_ROOT`                  | `var/artifacts`                | Truncated Tool output and Artifact root                               |
| `SOLOOPS_PROCESS_ALLOWLIST_JSON`         | `{}`                           | Program alias to absolute executable path mapping                     |
| `SOLOOPS_HOSTD_SOCKET`                   | unset                          | Linux Unix Socket used by Worker and hostd                            |
| `SOLOOPS_HOSTD_ALLOWED_UID`              | required by hostd              | Numeric Worker UID accepted by hostd peer-credential checks           |
| `SOLOOPS_SANDBOX_ENABLED`                | `false`                        | Register and serve the Docker-backed `sandbox.exec` Tool              |
| `SOLOOPS_DOCKER_SOCKET`                  | `/var/run/docker.sock`         | Absolute local Unix Socket read only by hostd                         |
| `SOLOOPS_SANDBOX_IMAGE`                  | required when enabled          | Preloaded image reference pinned with `@sha256:<64 hex>`              |
| `SOLOOPS_SANDBOX_PROGRAM_ALLOWLIST_JSON` | `{}`                           | Tool alias to absolute executable path inside the Sandbox image       |
| `SOLOOPS_SANDBOX_MEMORY_BYTES`           | `536870912`                    | Per-container memory limit                                            |
| `SOLOOPS_SANDBOX_NANO_CPUS`              | `1000000000`                   | Per-container CPU quota in Docker NanoCPUs                            |
| `SOLOOPS_SANDBOX_PIDS_LIMIT`             | `128`                          | Per-container process limit                                           |
| `SOLOOPS_SANDBOX_TMPFS_BYTES`            | `67108864`                     | Size limit for the container `/tmp` tmpfs                             |
| `SOLOOPS_MANAGED_DEPLOY_ENABLED`         | `false`                        | Register managed Compose/Caddy deployment Tools                       |
| `SOLOOPS_MANAGED_ROOT`                   | `/var/lib/soloops/deployments` | Immutable managed deployment bundles                                  |
| `SOLOOPS_DOCKER_CLI`                     | `/usr/bin/docker`              | Absolute Docker CLI path with Compose V2                              |
| `SOLOOPS_CADDY_CLI`                      | `/usr/bin/caddy`               | Absolute Caddy CLI path                                               |
| `SOLOOPS_CADDY_CONFIG`                   | `/etc/caddy/Caddyfile`         | Main Caddyfile importing managed fragments                            |
| `SOLOOPS_CADDY_MANAGED_DIR`              | `/etc/caddy/soloops.d`         | hostd-owned Caddy fragment directory                                  |
| `SOLOOPS_DEPLOY_SITE_ALLOWLIST_JSON`     | `[]`                           | Exact managed Caddy site names                                        |
| `SOLOOPS_DEPLOY_PORT_ALLOWLIST_JSON`     | `[]`                           | Loopback TCP ports Compose may publish                                |
| `SOLOOPS_DEPLOY_HEALTH_ORIGIN`           | `http://127.0.0.1`             | Fixed loopback health origin                                          |
| `SOLOOPS_DEPLOY_HEALTH_TIMEOUT_MS`       | `60000`                        | Compose and route health deadline                                     |
| `SOLOOPS_DEPLOY_OPERATION_LEASE_MS`      | `60000`                        | Managed deployment operation lease                                    |
| `SOLOOPS_DEPLOY_OPERATION_RENEW_MS`      | `10000`                        | Active managed deployment lease renewal interval                      |
| `SOLOOPS_DEPLOY_RECONCILE_INTERVAL_MS`   | `15000`                        | Expired managed operation reconciliation interval                     |
| `SOLOOPS_WORKER_LEASE_MS`                | `30000`                        | Run lease duration                                                    |
| `SOLOOPS_WORKER_LEASE_RENEW_MS`          | `10000`                        | Active lease renewal interval                                         |

## Endpoints

- `/healthz` and `/livez` prove the API event loop responds.
- `/readyz` verifies the database and required tables.
- `/metrics` exposes lightweight Prometheus text.

Current metrics:

- `soloops_http_requests_total`
- `soloops_http_errors_total`
- `soloops_auth_failures_total`
- `soloops_websocket_clients`
- `soloops_public_ip_checks_total`
- `soloops_public_ip_check_failures_total`
- `soloops_ip_notification_emails_sent_total`
- `soloops_ip_notification_email_failures_total`

Run snapshots and final reports expose total input tokens plus the `cachedInputTokens` and
`cacheWriteInputTokens` subsets reported by the provider. OpenAI prompt caching remains automatic
for eligible requests when the explicit cache key option is disabled. Enabling the option adds a
stable key derived from the model, system prompt, and Tool descriptors; non-OpenAI-compatible
endpoints may reject that optional request field.

## Logs

Production logs are JSON. Reverse proxies should preserve `x-request-id`; otherwise the API creates one. Never enable filters that log Cookie or Authorization contents.

Audit records are stored in SQLite and are independent of tracing log level.

## Public IPv4 notifications

Configure SMTP on `soloops-api`, run the explicit database migration, then use **Notifications** in
the Owner WebUI to save recipients and enable monitoring. The first successful detection is sent to
every recipient. Later checks send only when that recipient's last delivered IPv4 differs from the
current address. Failed recipients are retried on the next cycle without resending to recipients that
already succeeded. A missing SMTP configuration does not prevent API startup, but partially configured
SMTP or an unresolved password reference does.

An `event_stream_corrupt` API error, or WebSocket close code `4002`, is intentionally non-retryable. Restore or repair the affected SQLite event row before reconnecting clients; the UI stops automatic retries so database corruption is not hidden by a reconnect loop.

`process.exec` treats stdout and stderr as one bounded stream. When their combined size exceeds `maxToolOutputBytes`, hostd terminates and reaps the child process and records an `output_limit` Tool Call failure without persisting process output.

`process.exec` is registered only when the Worker has `SOLOOPS_HOSTD_SOCKET`. The allowlist is parsed by hostd, not by the Worker. hostd independently checks the Unix peer UID, persisted approval, Task creator, Tool argument digest, workspace path, environment allowlist, timeout, and output limit. There is no local Worker fallback. `SOLOOPS_MODEL_API_KEY_ENV` remains a one-phase compatibility alias and emits a deprecation warning.

`sandbox.exec` is registered only when both `SOLOOPS_HOSTD_SOCKET` and `SOLOOPS_SANDBOX_ENABLED=true` are set on the Worker. hostd must receive the same enable flag plus a digest-pinned image and program alias map. The image must be preloaded; hostd never pulls it. Each invocation mounts only the canonical Run Workspace, disables networking, applies the fixed non-root and resource policy, reads and monitors the persisted Run Workspace size budget, captures bounded output, audits the image digest and exit category, and force-removes the labeled container. On startup hostd removes any labeled container left by a prior crash.

## Managed deployment

Before enabling managed deployment, install Docker Engine with Compose V2 and Caddy through the
normal host provisioning process; SoloOps never downloads them. Create the managed bundle and Caddy
fragment directories owned by hostd. The main Caddyfile must import the managed directory, for
example `import /etc/caddy/soloops.d/*.caddy`. Preload every image by digest, configure exact site
and port allowlists, apply migrations through v6, then restart hostd before Worker.

`managed.deploy.plan` normalizes Compose, enforces policy, validates Caddy, and stores an immutable
bundle. `apply` and `rollback` display a redacted preview and exact argument SHA-256 before approval.
Deployment uses `docker compose up -d --pull never --remove-orphans --wait`, validates and reloads
Caddy, and probes the fixed loopback origin with the approved site Host header. A failure restores
the last verified revision. If both deployment and compensation fail, hostd leaves the Saga
unfinished, returns `recovery_required`, and the Worker blocks the Run. Each active operation owns
a renewable lease; startup and periodic reconciliation claim only expired leases and retry restoring
the last database-confirmed revision without preventing process or Sandbox requests from being
served. There is an unavoidable crash window between publishing/reloading Caddy and committing the
revision: a new revision may be live temporarily while `current_revision_id` still names the prior
revision. Lease-expiry reconciliation restores that prior confirmed revision. This MVP does not pull images, build images, mount host paths, inject
deployment secrets, or expose a general network policy.

`managed.deploy.apply` and `managed.deploy.rollback` have a 600-second descriptor timeout. With the
default 60-second deployment health timeout, a failed apply followed by full compensation has an
approximately 420-second worst-case command and health budget. Other built-in Tools retain their
60-second descriptor timeout even though the Worker ceiling defaults to 600 seconds. Configuring a
shorter Worker ceiling, or increasing the deployment health timeout beyond the remaining Saga
budget, can leave the result unknown; the Worker then uses the durable deployment operation to
recover a completed result or explicitly blocks the Run for hostd reconciliation.

Before a release that enables Sandbox or managed deployment, run the manual `Linux release
acceptance` GitHub Actions workflow. Its fixture image is preloaded by immutable digest, and its
disposable Caddy instance and loopback ports are isolated from production. Acceptance requires a
successful route probe, an unhealthy replacement that restores the prior active revision, a durable
failure result with `rollbackSucceeded=true`, and two successful cleanup passes with no remaining
Compose or Sandbox containers. Browser automation, general Sandbox networking, deployment Secret
injection, and runtime image pulling remain unsupported after Phase 3.1.

## Shutdown

API shutdown stops accepting connections and waits for Axum graceful shutdown. Worker exits on Ctrl+C after its current cycle. Dropping an active hostd request closes the Unix connection; hostd then terminates and removes the active process or Sandbox. Sandbox startup reconciliation removes crash leftovers.

## Backup

For Phase 0, stop API and Worker before copying the database, or use a SQLite-aware online backup tool. Copying only the main file while WAL is active can produce an incomplete backup.

Backups must include:

- SQLite contents;
- artifact/workspace data when introduced;
- managed configuration;
- encrypted Secret storage metadata.

## Release contents

Production packaging includes Rust release binaries and `apps/web/build`. It excludes `target`, `node_modules`, test databases, source maps if policy forbids them, compiler caches and development secrets.
