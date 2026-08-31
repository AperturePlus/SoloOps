# ADR 0001: Rust-only backend

- Status: accepted
- Date: 2026-07-16

## Context

The Phase 0 prototype used Bun, Fastify, Drizzle and a TypeScript Worker, while the long-term design proposed a separate Go privileged daemon. This created multiple backend runtimes, duplicated operational knowledge and made strict resource and privilege boundaries harder to standardize.

## Decision

All backend processes and libraries use Rust:

- HTTP control plane;
- Agent and task Worker;
- database and migration layer;
- administrative CLI;
- future browser adapter;
- future privileged hostd.

SvelteKit and TypeScript remain for frontend source code only. The frontend is delivered as static assets.

## Consequences

Positive:

- one backend toolchain and type system;
- explicit ownership and error handling;
- low-overhead binaries;
- shared contracts across API, Worker and hostd;
- easier static release packaging.

Costs:

- browser and model ecosystem integrations may require custom adapters;
- compile times are higher than the original prototype;
- frontend contracts require deliberate synchronization until generation is automated.

## Migration method

Use an additive Rust implementation, verify behavior and database compatibility, switch the frontend, then delete the TypeScript backend. Git retains the original baseline.
