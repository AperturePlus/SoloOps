# Security model

## Trust zones

1. Owner browser: authenticated but still sends untrusted input.
2. API/Worker account: unprivileged application identity.
3. Sandbox: untrusted code and content.
4. Linux hostd: narrow privileged boundary for explicitly typed host actions.
5. External services: remote models, repositories and network APIs.

## Phase 0 controls

- One Owner only.
- Argon2id password hashes.
- Random 256-bit Session Tokens; only SHA-256 digests are stored.
- `HttpOnly`, `SameSite=Strict`, production `Secure` Cookie.
- Origin checks on mutations.
- Login failure rate limiting and audit events.
- Parameter-bound SQL queries.
- Explicit Run transition graph.
- No model, shell, browser, Docker or host execution.
- Stub Worker always ends in `blocked`.

## Deployment controls

- Bind to localhost by default.
- Expose through private VPN and HTTPS reverse proxy.
- Do not publish `/metrics` to the public Internet.
- Run API and Worker as non-root users.
- Restrict database and artifact directories to the service account.
- Rotate logs and protect backups as sensitive data.

## Secret handling

Passwords, Session Tokens, model keys and external credentials must never appear in logs, events, audit context or task artifacts. Model configuration uses a typed `SecretRef`; v1 resolves only `env:` references into a redacted, zeroizing value. SMTP passwords likewise use `env:` references and are never persisted; notification audit records contain delivery counts and IP transitions, not recipient addresses or SMTP errors.

The API makes outbound HTTPS requests only to the configured public-IP endpoint and outbound TLS SMTP connections only to the configured relay. Treat both endpoints as trusted deployment configuration. Public IPv4 addresses and notification recipients are operationally sensitive and must be protected with the same database and backup controls as Owner data.

## hostd rules

The Worker must not receive Docker Socket access. hostd validates every typed request independently and rejects arbitrary command execution, arbitrary filesystem access and arbitrary systemd units. Protocol V3 carries `process.exec`, `sandbox.exec`, and managed deployment actions. hostd verifies peer UID and reads the persisted Tool name, exact argument digest, policy, Task creator and Owner approval directly; a Worker-provided approval claim is never sufficient.

Sandbox images must already exist locally and be pinned by SHA-256 digest. Every Tool Call gets a fresh container with a read-only root filesystem, one writable `/workspace` bind mount, a non-root identity, all Linux capabilities dropped, `no-new-privileges`, the default seccomp profile, no network or published ports, and CPU, memory, PID, tmpfs, Workspace size, time and output limits. Only configured absolute executable aliases are accepted; shell executables and arbitrary image selection are rejected. hostd reads the immutable Workspace budget from the persisted Runtime instead of trusting the Worker, monitors it while the container runs, force-removes the container after success, failure, timeout, budget overflow or Worker disconnect, and cleans labeled stale containers on startup.

Managed deployment remains disabled by default. It accepts only digest-pinned, preloaded images and
rejects builds, host bind mounts, Docker Socket mounts, privileged containers, host namespaces,
devices, added capabilities, external networks/volumes, non-loopback publishing, and sites or ports
outside operator allowlists. Caddy may proxy only to loopback ports declared by the validated
Compose proposal. Approval previews derive from an immutable proposal and never contain Secret
values. Once a privileged change begins, hostd completes validation or compensating rollback and
persists the result under the original call ID.

## Reporting vulnerabilities

Do not attach production databases, logs containing secrets or private task artifacts to public issues. Provide the smallest sanitized reproduction.
