# SoloOps

SoloOps 是面向单一 Owner 的私有 Agent 控制平面。后端、任务运行时、数据库访问和管理命令全部使用 Rust；SvelteKit 仅用于构建静态 WebUI，生产环境不需要 Bun 或 Node.js 运行时。

当前仓库实现 Phase 1 可观察 Agent Runtime、Phase 2a 特权边界、Phase 2b Docker Sandbox MVP、Phase 3 受管部署 MVP，以及 Phase 3.1 发布加固：

- 单 Owner 初始化、Argon2id 密码和持久 Session；
- Task/Run 持久化状态机；
- SQLite WAL、显式迁移和旧 TypeScript Schema 接管；
- REST 事件补拉与经过认证的 WebSocket 时间线；
- 持久 Model Item Loop、OpenAI-compatible Tool Calling、硬预算、续租和恢复；
- 受策略约束的 Workspace/Process Tool、Owner 审批、取消、Evidence 和最终报告；
- 类型化 SecretRef，以及通过 Linux Unix Socket 隔离的可选 `soloops-hostd` Process Tool 边界；
- 经精确审批的 `sandbox.exec`，由 hostd 在默认断网、非 root、资源受限的临时 Docker 容器中执行；
- 不可变 Compose/Caddy 提案、脱敏审批预览、digest-pinned 离线部署、健康验证和自动回滚；
- JSON 结构化日志、请求 ID、审计日志、健康检查、就绪检查和 Prometheus 文本指标；
- Owner 可配置多收件邮箱，在公网 IPv4 首次检测或变化时通过 SMTP 逐一通知；
- SSH 访问审计：解析本机 `authorized_keys`，按来源机器汇总可免密登录的公钥、指纹与 `from`/`command` 限制；
- Windows/Linux 常规 CI、浏览器 E2E 后台错误门禁，以及手动 Linux Docker/Caddy 发布验收；
- 纯静态 Svelte WebUI，可由 Rust API 或 Caddy 提供。

当前版本不包含产品浏览器自动化、通用容器联网策略、镜像拉取或部署 Secret 注入。Host Process、Docker Sandbox 和 Managed Deployment Tool 默认禁用；启用时必须运行 Linux `soloops-hostd`，所有部署变更由 Owner 查看规范化预览后精确审批。Worker 不直接启动宿主机进程，也不持有 Docker Socket。

## 技术要求

- Rust 1.88 或更高版本；
- Bun 1.3.x，仅用于安装和构建 WebUI；
- Windows 10/11 或 Linux。
- Docker Sandbox 运行时额外要求 Linux Docker Engine，以及预加载并以 SHA-256 digest 固定的镜像。

## 初始化

```powershell
Copy-Item .env.example .env
# 在 .env 中设置 SOLOOPS_MODEL_NAME，并通过 SOLOOPS_MODEL_API_KEY_REF 引用密钥
bun install --frozen-lockfile
cargo run -p soloopsctl -- migrate
cargo run -p soloopsctl -- owner-init
```

`owner-init` 要求交互终端，密码至少 12 个字符，并拒绝创建第二个 Owner。

## 开发运行

分别启动三个进程：

```powershell
cargo run -p soloops-api
cargo run -p soloops-worker
bun run dev:web
```

Linux 上启用 `process.exec`、`sandbox.exec` 或受管部署时另行启动 `cargo run -p soloops-hostd`，并为 Worker 配置 `SOLOOPS_HOSTD_SOCKET`。Sandbox 还要求 `SOLOOPS_SANDBOX_ENABLED=true`、digest-pinned `SOLOOPS_SANDBOX_IMAGE` 和容器内程序别名白名单。受管部署要求 Docker Compose V2、Caddy、受管目录、站点/端口白名单和主 Caddyfile import。hostd、Worker 和数据库必须使用一致的 Workspace/数据库配置；完整变量见 `docs/operations.md`。

WebUI 默认位于 `http://127.0.0.1:5173`，Vite 将 `/api` 和 WebSocket 代理到 Rust API `http://127.0.0.1:3001`。

也可以先构建静态前端，再由 Rust API 同源提供：

```powershell
bun run --cwd apps/web build
cargo run -p soloops-api
```

此时访问 `http://127.0.0.1:3001`。

## 验证

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

常规 GitHub Actions 在 push 和 pull request 上运行；需要 Docker、Caddy 和预加载镜像的
Linux 发布验收通过手动 `Linux release acceptance` workflow 单独触发。

完整发布构建：

```powershell
bun run build
```

## 工作区

```text
apps/
  web/                    SvelteKit 静态 Owner UI
bins/
  soloops-api/            Axum HTTP、Session、WebSocket、静态文件
  soloops-worker/         持久队列消费者和 Agent Runtime
  soloops-hostd/          Linux 特权边界、受限宿主进程和临时 Docker Sandbox
  soloopsctl/             迁移、Owner 初始化和诊断
crates/
  soloops-domain/         数据契约和 Run 状态机
  soloops-application/    Model Provider、Tool Registry 与 Runtime Engine
  soloops-storage/        SQLx、SQLite、迁移、仓储和审计
  soloops-server/         配置、认证、API、可观测性
docs/
  api/openapi.yaml        Phase 1 HTTP 契约
```

API 与 Worker 共享一个 SQLite 数据库。应用进程不会自动迁移，迁移必须通过 `soloopsctl migrate` 显式执行。现有 TypeScript Phase 0 数据库会被校验并登记为已接管，不要求删除数据。

## 运维端点

- `GET /healthz`：进程存活；
- `GET /livez`：进程存活别名；
- `GET /readyz`：数据库和 Schema 就绪；
- `GET /metrics`：轻量 Prometheus 文本指标。

详细信息见：

- [系统设计](./设计.md)
- [架构说明](./docs/architecture.md)
- [开发与测试](./docs/development.md)
- [运维与可观测性](./docs/operations.md)
- [迁移说明](./docs/migration.md)
- [安全边界](./docs/security.md)
