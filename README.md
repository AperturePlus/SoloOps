# SoloOps

SoloOps is a private, single-owner agent control plane. This repository currently contains the lightweight **Phase 0** foundation: authentication, durable Task/Run state, a safe stub Worker, and a resumable event timeline.

Phase 0 deliberately does not execute user goals. The Worker proves the persisted pipeline and then moves each Run to `blocked` with an explicit reason. Models, tools, privileged host operations, Docker, browser automation, and Linux deployment are not part of this phase.

## Requirements

- Bun 1.3.x
- Windows 10/11 for the current development baseline

No Go toolchain, Docker daemon, Chromium, Redis, or external database is required.

## Setup

```powershell
bun install --frozen-lockfile
Copy-Item .env.example .env
bun run db:migrate
bun run owner:init
```

`owner:init` is interactive, masks the password, requires at least 12 characters, and refuses to create a second Owner.

Start Web, API, and Worker together:

```powershell
bun run dev
```

The Web UI is available at `http://127.0.0.1:5173`. Fastify listens on `http://127.0.0.1:3001` and is reached through the SvelteKit development proxy.

## Verification

```powershell
bun run lint
bun run check
bun test
bun run build
```

The test suite uses isolated in-memory SQLite databases and does not download a browser runtime.

## Workspace layout

```text
apps/
  api/                 Fastify REST, Session and WebSocket API
  web/                 SvelteKit Owner UI
  worker/              Durable queue consumer and safe stub executor
packages/
  contracts/           Browser-safe Zod contracts and Run states
  db/                  bun:sqlite, Drizzle schema, migrations and repositories
  server-shared/       Environment validation and lightweight JSON logging
```

The API and Worker share one SQLite database in WAL mode. Migrations are always explicit through `bun run db:migrate`; application processes never run migrations automatically.

## Phase 0 API

- `GET /healthz`
- `POST /api/auth/login`
- `POST /api/auth/logout`
- `GET /api/auth/session`
- `POST /api/tasks`
- `GET /api/tasks`
- `GET /api/tasks/:taskId`
- `GET /api/runs/:runId`
- `GET /api/events?after=<sequence>&runId=<runId>`
- `WS /api/events?after=<sequence>&runId=<runId>`

All `/api` routes, including WebSocket upgrades, require the Owner Session except for login. Task status shown in the UI is derived from the latest Run. Event `sequence` is the durable cursor used for reconnect and replay.

## Configuration

Configuration is read from `.env`; see [.env.example](./.env.example). Defaults bind services to localhost. Production Cookies become `Secure` when `NODE_ENV=production`, and production access is expected to use a same-origin private reverse proxy.

The long-term product requirements and deferred Linux privilege boundary are documented in [设计.md](./设计.md).
