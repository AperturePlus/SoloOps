# Architecture

## Runtime topology

SoloOps separates the control plane from task execution:

```text
Browser
  │ HTTPS / WSS
  ▼
Caddy or soloops-api static fallback
  ├─ static Svelte SPA
  └─ /api, /healthz, /readyz, /metrics
         │
         ▼
      SQLite WAL
         ▲
         │
  soloops-worker
         │ typed Unix Socket; no Docker Socket
         ▼
   soloops-hostd ─────► Docker Engine ─────► ephemeral offline Sandbox
```

Production does not run Bun or Node.js. Bun is a build-time dependency for `apps/web`.

`soloops-api` also owns the single-instance public IPv4 notification monitor. Its schedule and
per-recipient delivery state are durable in SQLite; SMTP credentials remain environment-backed
configuration and are never persisted.

## Dependency direction

```text
soloops-api ──────► soloops-server ──► soloops-storage ──► soloops-domain
soloops-worker ───► soloops-application ────────────────► soloops-domain
soloopsctl ───────► soloops-server + soloops-storage
```

- `soloops-domain` contains serializable contracts and the Run transition graph.
- `soloops-application` owns use cases that should not know HTTP details.
- `soloops-storage` owns all SQL, transactions, migration adoption and audit persistence.
- `soloops-server` owns configuration, authentication, HTTP/WebSocket transport and process metrics.
- binaries contain startup, shutdown and process-specific loops only.

## Consistency model

SQLite is the durable source of truth. API responses and WebSocket messages are projections of persisted state.

Task creation writes Task, initial Run and `run.created` in one transaction. Run transitions update the Run and append `run.status_changed` in one transaction. A WebSocket client reconnects using the last observed global `sequence`.

## Worker leasing

The Worker uses `BEGIN IMMEDIATE`, reads the oldest queued Run, conditionally changes it to `leased`, and appends an event before commit. Two Worker processes therefore cannot successfully lease the same queued Run.

The Phase 1 Worker renews the lease while a persisted Agent Item loop moves through:

```text
queued → leased → planning → running → verifying → reporting → succeeded
                                  ├─ waiting_for_approval → running
                                  ├─ retry_scheduled → queued
                                  └─ needs_recovery → queued / blocked
```

Model attempts, Tool Calls, approvals, Evidence and reports are durable SQLite records. The global
event stream is a redacted UI projection and is never used to reconstruct Runtime state.

## Static WebUI

The SvelteKit root layout disables SSR. `@sveltejs/adapter-static` writes the SPA to `apps/web/build`. The API binary can serve that directory with an `index.html` fallback; a production Caddy deployment may serve the same files directly.

## Privileged boundary and Sandbox

`soloops-hostd` is Rust and listens only on a permission-restricted Unix Socket. Protocol V3 exposes narrow process, Sandbox, and managed deployment actions. The Worker receives neither the Docker Socket nor unrestricted host shell access, and hostd revalidates persisted policy or exact approval before every action.

The Phase 2b MVP creates one temporary container per `sandbox.exec` Tool Call. The Run Workspace is the only host bind mount and remains the durable state boundary; container root filesystems are disposable. Each container is offline, non-root, capability-free and resource-bounded, then force-removed on every completion path. Browser automation and managed network access remain future Phase 2b increments.

## Managed deployment

Phase 3 adds `managed.deploy.plan`, `managed.deploy.status`, `managed.deploy.apply`, and
`managed.deploy.rollback`. Planning stores an immutable Compose/Caddy revision. Applying and rolling
back are privileged operations approved against exact arguments and a normalized, redacted preview.

hostd protocol V3 is a coordinated Worker/hostd upgrade. hostd alone owns Docker Compose and Caddy
CLI access. Deployment is a persisted Saga: Compose reaches health, Caddy validates and reloads, a
loopback route probe succeeds, then the revision becomes active. Failures compensate to the last
verified revision; the Worker never receives a Docker Socket or general host command surface.
