# ADR-0002: Isolate process execution behind hostd

- Status: accepted
- Date: 2026-07-26

## Context

Phase 1 executed an allowlisted program directly in the Worker. The implementation avoided a shell and bounded paths, time, environment, and output, but the Worker still held executable paths and process-spawn authority. Future sandbox and deployment work requires a narrower privilege boundary.

## Decision

Linux `process.exec` is performed only by `soloops-hostd` over a versioned, length-prefixed Unix Socket protocol. The Worker sends the persisted Run ID, Tool Call ID, original JSON arguments, and output budget. hostd independently verifies:

- Unix peer UID;
- the Run and Tool Call are currently executing;
- the unique Owner approved the exact persisted argument digest;
- the approval owner is the Task creator;
- the program alias, workspace-relative cwd, environment, timeout, and output bounds.

The Worker has no local execution fallback. If the client disconnects, the request times out, or output exceeds its limit, hostd kills and reaps the child. Audits contain identifiers and outcome categories, never command arguments, environment values, or output.

## Consequences

- `process.exec` is unavailable unless hostd is explicitly configured.
- Windows builds retain a clear unsupported stub; the operational service is Linux-only.
- hostd receives database and Workspace access sufficient to verify policy and execute the typed request, but receives no generic shell, Docker API, or arbitrary filesystem API.
- Docker Sandbox and other privileged actions require new protocol variants and separate validation rules in later phases.
