# ADR-0003: Execute untrusted processes in ephemeral Docker sandboxes

- Status: accepted
- Date: 2026-07-27

## Context

Phase 2a moved allowlisted host process execution behind `soloops-hostd`, but a host allowlist is not an isolation boundary for builds, tests or other untrusted repository code. Giving the Worker direct Docker access would also let a compromised Agent control the host through the daemon API.

## Decision

Protocol V2 adds `sandbox.exec`. The Worker registers it only when Sandbox support is explicitly enabled, and hostd independently verifies the running Tool Call, exact arguments, Task creator and persisted Owner approval.

Each call creates one disposable Docker container from a preloaded digest-pinned image. hostd maps a configured program alias to an absolute executable and never accepts a shell command, image choice, mount, port or network option from the model. The container is non-root, offline, capability-free, `no-new-privileges`, read-only except for `/tmp` and the current Run Workspace, and bounded by CPU, memory, PID, Workspace size, time and output limits. The Workspace limit comes from the persisted Runtime budget and is monitored while the container runs.

hostd force-removes the container after every outcome and cleans its labeled leftovers on startup. Interrupted Sandbox calls retain `Process` recovery semantics: unknown side effects block automatic retry.

## Consequences

- The Worker never receives the Docker Socket; only hostd connects to an absolute local Unix Socket.
- Images must be provisioned before execution and pinned by SHA-256 digest; the MVP never pulls them.
- Container root filesystem changes are intentionally discarded between Tool Calls. The Run Workspace remains the durable boundary.
- Browser automation, network allow rules, persistent containers and deployment operations remain out of scope.
