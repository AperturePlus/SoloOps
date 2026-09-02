# 00 — SoloOps 全代码库审查基线（验证与盘点）

- **采集时间**: 2026-09-02 10:16–10:22 (UTC+8)
- **审查对象**: 当前工作区状态（分支 `feature/agent-page-polish`，HEAD=`8609c93`，工作区含 69 个未提交变更）
- **提交历史**: 全分支仅 2 个提交（`8609c93` feat(webui): composite layouts across all pages (Codex-style space utilization)；`0dd7591` chore: capture phase 0 typescript baseline），无 git remote
- **工具链**: Windows + Git Bash；rustc/cargo 1.95.0 (59807616e 2026-04-14)；bun 1.3.14；workspace 声明 rust-version 1.88 / edition 2024；TypeScript 5.9.3；Playwright 1.55.0
- **本节点纪律**: 未改任何源码；唯一环境变更为安装缺失的 rust-analyzer 工具链组件（见 §6）

---

## 1. 验证命令结果

| # | 命令 | 退出码 | 结论 |
|---|------|--------|------|
| 1 | `cargo fmt --all -- --check` | 0 | ✅ 通过 |
| 2 | `cargo clippy --workspace --all-targets -- -D warnings` | 0 | ✅ 通过（9 个成员全部零警告） |
| 3 | `cargo test --workspace` | 0 | ✅ 通过（104 passed / 2 ignored / 0 failed） |
| 4 | `bun run lint` | 0 | ✅ 通过（`eslint apps/web`） |
| 5 | `bun run --cwd apps/web check` | 0 | ✅ 通过（svelte-check 0 errors, 0 warnings） |
| 6 | `bun run test:mock` | 0 | ✅ 通过（10 tests / 30 expects / 0 fail） |
| 7 | `bun run --cwd apps/web build` | 0 | ✅ 通过（vite build 6.81s，adapter-static 写入 `build/`） |
| 8 | `lsp-workspace-diagnostics`（rust-analyzer） | — | ✅ 采集成功：**0 errors / 0 warnings**（首轮因组件缺失失败，装好后重试成功，见 §2/§6） |
| 9 | `bun run test:e2e`（可选，因 1–7 全过而尝试） | 1 | ❌ **失败：2/2 用例失败**（真实回归线索，非环境缺失，详见下文） |

### 1.9 e2e 失败详情（唯一失败项，重要基线事实）

`bun run test:e2e` = `apps/web build` + `cargo build -p soloops-server --example e2e_harness --features e2e-harness` + `playwright test`。Playwright 配置（`playwright.config.ts`）：webServer 直接启动 `target\debug\examples\e2e_harness.exe`（真实 Rust 后端，静态服务 + API 于 `http://127.0.0.1:4173`），`fullyParallel: false, retries: 0`，chromium。**浏览器已安装、harness 正常启动、登录流程正常走通**——失败不是环境缺失，而是测试期望与当前 UI 的漂移。套件共 2 个用例，全部失败：

**失败 1** — `apps/web/e2e/happy-path.spec.ts:3` "owner approves a workspace change and receives a final report after reload"
```
Error: expect(locator).toBeVisible() failed
Locator: getByRole('heading', { name: 'Tasks' })
Expected: visible   Timeout: 5000ms   Error: element(s) not found
  at apps\web\e2e\happy-path.spec.ts:10:62
```
失败时页面快照（`test-results/happy-path-owner-approves--c63cf-…/error-context.md`）证明应用正常渲染：登录成功后任务页加载，含 "Primary" 导航（Overview/Tasks/New task）、侧栏 "Tasks 0 / No tasks / Create your first goal to start."、"3.1" 徽标文本——即**新复合布局里不再存在可访问名称为 "Tasks" 的 heading**，而 e2e 规格仍按旧布局断言。

**失败 2** — `apps/web/e2e/happy-path.spec.ts:34` "owner reviews and approves an exact managed deployment proposal"
```
Error: expect(locator).toBeVisible() failed
Locator: getByText('managed.deploy.apply')
Expected: visible   Timeout: 15000ms   Error: element(s) not found
  at apps\web\e2e\happy-path.spec.ts:47:56
```
流程走得更深：填表（Title/Goal）→ 点击 "Create task" 成功 → 15 秒内未出现 `managed.deploy.apply` 工具调用文本；失败时快照仍显示任务列表空态（"No task selected"）。可能是（a）运行页/工具调用渲染方式变更，或（b）create→run→工具执行链路在真实 harness 下的功能性回归——**后续节点必须深查**。

**佐证**: `apps/web/e2e/happy-path.spec.ts` 不在本分支 69 个变更列表中（仅 `global-teardown.ts` 被改），而 UI 大量重构（tasks 页、run 页、组件全套变更）——典型的「规格未随 UI 更新」漂移；mock 单测与 build 均绿，说明不是编译/类型层问题。

### 1.3 cargo test 明细（10 个测试目标 + 5 个 doc-test 段）

| 测试目标 | 用例数 | 结果 |
|---|---|---|
| soloops-api (main.rs) | 0 | ok |
| soloops-application (lib.rs) | 35 | 35 passed |
| soloops-domain (lib.rs) | 7 | 7 passed |
| soloops-domain/tests/contract_files.rs | 1 | 1 passed（`frontend_and_openapi_include_every_run_status`，前端/OpenAPI/域状态枚举三方一致性契约） |
| soloops-hostd (main.rs) | 15 | 13 passed, **2 ignored**（`live_compose_failure_can_restore_the_previous_revision_without_pull`、`live_managed_deployment_verifies_route_and_restores_the_active_revision`，需真实 Docker Compose/Caddy/预载镜像，见 §6） |
| soloops-hostd-protocol (lib.rs) | 3 | 3 passed |
| soloops-server (lib.rs) | 21 | 21 passed（2.37s，含 ssh_access/config/notifications/登录限流/CORS/WS 回放等） |
| soloops-storage (lib.rs) | 24 | 24 passed |
| soloops-worker / soloopsctl (main.rs) | 0 | ok |
| Doc-tests × 5 | 0 | ok |

合计 **106 项运行：104 通过、2 ignored、0 失败**。

### 1.5 svelte-check 附带输出（信息性，非错误）

Vite `configLoader: 'native'` 兼容性提示：`src/mock` 目录索引导入 + 12 处无扩展名导入（`./state`、`./clients`、`./scenario`、`./router`）在 `vite.config.ts`、`src/mock/index.ts`、`src/mock/router.ts`、`src/mock/scenario.ts`。`svelte-check` 本体：**0 errors, 0 warnings**。属未来 Vite 大版本的兼容风险提示，不构成本基线缺陷。

---

## 2. LSP 工作区诊断汇总（rust-analyzer）

- `lsp-workspace-diagnostics`（maxFiles=200）：**0 errors / 0 warnings / 0 infos，覆盖全部 Rust 工作区文件**（languages: go+rust；go 服务器启用但项目无 Go 源码，无影响）。
- 抽查复核（确认 rust-analyzer 确实完成分析而非空转）：
  - `crates/soloops-domain/src/security.rs`（新增未跟踪文件）：0/0/0
  - `crates/soloops-server/src/http/ssh_access.rs`（新增未跟踪文件）：0/0/0
  - `crates/soloops-application/src/engine.rs`（本分支修改）：0/0/0
- 结论：**工作区 Rust 源码在 rust-analyzer 下零诊断**，与 clippy `-D warnings` 全绿一致。
- 过程事实：首次调用失败（`ServerFailed("initialize failed: ServiceStopped")`），原因是 stable 工具链中 rust-analyzer 组件未安装（shim 存在但二进制缺失，`rust-analyzer --version` 报 "Unknown binary"）；执行 `rustup component add rust-analyzer`（退出码 0）后重试成功。

---

## 3. 卫生检查结果

| 检查项 | 结果 |
|---|---|
| `git check-ignore -q .env` | 退出码 0 → **.env 已被 git 忽略** ✅（本地存在 .env 2021B，按纪律未读取其内容以防密钥泄漏） |
| `.env.example` 内容审查 | ✅ 仅占位配置与默认值：SMTP 字段全部注释占位（`smtp.example.com`）；模型密钥通过引用间接注入 `SOLOOPS_MODEL_API_KEY_REF=env:OPENAI_API_KEY`（良好实践）；**无真实密钥** |
| 已跟踪文件密钥扫描 `git grep -nE "(sk-[A-Za-z0-9]{16,}\|BEGIN (RSA\|OPENSSH\|EC) PRIVATE KEY\|AKIA[0-9A-Z]{16})" -- ':!bun.lock'` | 退出码 1（无匹配）→ **0 命中，无待复核线索** ✅ |
| `git status --short` | 69 行：**50 个已修改（M）+ 19 个未跟踪（??）**。未跟踪中与代码相关的关键新增：`crates/soloops-domain/src/security.rs`(75行)、`crates/soloops-server/src/http/ssh_access.rs`(548行)、`apps/web/src/routes/settings/security/+page.svelte`(363行)、`apps/web/src/lib/components/ui/`（badge/button/input/separator/switch/textarea，shadcn 风格）、`apps/web/components.json`、`apps/web/src/lib/utils.ts`、`ApprovalBanner.svelte`、`PageHeader.svelte`、`apps/web/scripts/ui-lint.mjs`(79行)、`apps/web/e2e/ui-audit.mjs`(58行)、`docs/ui-spec.md`；另有 `参考/pi-main`（外部参考资料）与 `outputs/` 运行时杂项文件（.callid.txt、.run.json 等，非审查对象） |
| `git log --oneline -3` | 分支仅 2 个提交（见文首）；**无 remote**，纯本地仓库；已跟踪文件 149 个 |

**当前审查对象明确为工作区状态（含全部未提交变更）**，而非 HEAD 提交。

---

## 4. 模块盘点表（find/wc 实测）

| 模块 | 路径 | 文件数 | 行数 | 职责 |
|---|---|---|---|---|
| soloops-domain | crates/soloops-domain | 8 src | 796（+tests/contract_files.rs 25 = 821，与已知规模吻合） | 核心领域类型与任务/运行状态机、领域事件与错误；本分支新增 `security.rs`（75 行） |
| soloops-storage | crates/soloops-storage | 16 | 4944 ✅ | SQLite（sqlx）WAL 持久化：schema/迁移、运行时事件流、租约与恢复、认证/会话/审计、IP 通知、托管部署记录 |
| soloops-application | crates/soloops-application | 14 | 6195 ✅ | 应用引擎：模型循环（model.rs）、任务编排（engine.rs）、工具执行（tools.rs）、恢复逻辑；测试内嵌于 src/tests/ |
| soloops-server | crates/soloops-server | 9 src | 2620 src（crate 总计 3057 = src + examples/e2e_harness.rs 437 ✅） | HTTP/WS API 层：axum 路由与错误映射、argon2 认证+cookie 会话、WS 事件回放、SSH 访问报告（新增 ssh_access.rs 548 行）、SMTP 通知（lettre）、配置/遥测；`e2e_harness` 示例为 e2e 提供真实后端 |
| soloops-hostd-protocol | crates/soloops-hostd-protocol | 1 | 254 ✅ | hostd ↔ 应用层 Unix Socket 帧协议类型（serde 帧编解码、大小上限） |
| soloops-hostd (bin) | bins/soloops-hostd | 4 | 4141 ✅（deployment.rs 2184 / unix.rs 1151 / sandbox.rs 787 / main.rs 19） | Linux 特权守护：经 Unix Socket 提供 process.exec、sandbox.exec（bollard/Docker）、托管部署（Compose+Caddy 配对、健康探测、修订回滚） |
| soloops-api (bin) | bins/soloops-api | 1 | 63 ✅ | HTTP 入口：装配 server + storage 并监听（60–70 行级别） |
| soloops-worker (bin) | bins/soloops-worker | 1 | 62 ✅ | 任务运行时：轮询租约并驱动引擎循环 |
| soloopsctl (bin) | bins/soloopsctl | 1 | 69 ✅ | 管理 CLI：migrate / owner-init / doctor（clap + argon2 + rpassword） |
| apps/web | apps/web/src | 46（26 .svelte + 20 .ts） | 5235 ✅ | 纯静态 SvelteKit 前端（adapter-static）：routes 11 文件/2468 行（login、tasks、tasks/new、runs/[runId]、settings/{layout,notifications,security}、+layout、+page），lib 27 文件/1652 行（组件+契约+time+utils），mock 7 文件/1110 行（本地 mock 路由/场景/状态）；另有 e2e/ 208 行、scripts/ 79 行 |

技术栈补充：前端 devDeps 含 tailwindcss + bits-ui + tailwind-variants + tw-animate-css（shadcn-svelte 体系，与新增 `components.json`/`ui/` 组件一致）、`@internationalized/date`、`ws`；root devDeps：eslint 9 生态、prettier + prettier-plugin-svelte、typescript 5.9.3、@playwright/test 1.55。

---

## 5. 依赖方向说明

**Crate 内部依赖（全部 path 依赖，无环 DAG）：**

```
soloops-domain（叶子：serde/serde_json/thiserror）
  ↑                    ↑
soloops-storage       soloops-application → soloops-hostd-protocol（叶子）
（+sqlx/sha2/uuid）     （+reqwest/regex/zeroize/tracing/tempfile）
  ↑     ↑                ↑
  |     |                |
soloops-server       bins/soloops-worker（application + server + storage）
（+axum/axum-extra/argon2/lettre/tower-http/dotenvy/time/base64/rand）
  ↑
bins/soloops-api（server + storage）
bins/soloopsctl（server + storage + clap/rpassword）
bins/soloops-hostd（hostd-protocol + storage + bollard/serde_yaml_ng；dev: soloops-domain + tempfile）
```

要点：
- `soloops-domain` 与 `soloops-hostd-protocol` 是两个零内部依赖的叶子；一切向上依赖，**无反向依赖、无环**。
- `soloops-server` 只在 **dev-dependencies** 里引 `soloops-application`（供 http/tests.rs 与 e2e_harness 示例使用）——正式依赖不含 application，方向保持 storage→server 单向。
- `bins/soloops-hostd` 直接依赖 `soloops-storage`（而非经 server），并以 dev-dep 引 domain——hostd 作为独立特权域，与 HTTP 层解耦。
- `bins/soloops-worker` 同时依赖 application + server + storage（运行时引擎 + 复用 server 的装配/配置）。
- **数据库层是 `sqlx 0.8`（runtime-tokio + sqlite），不是 rusqlite**——任务描述中的 rusqlite 与实际不符，以本实测为准。
- 关键第三方：axum 0.8（macros+ws）、axum-extra 0.10（cookie）、tokio 1.47、argon2 0.5、lettre 0.11.22（smtp/rustls）、reqwest 0.12（rustls）、bollard 0.19.4（Docker）、tower-http 0.6、zeroize、serde_yaml_ng、tokio-tungstenite 0.29（dev）、sqlx 0.8、uuid 1.17、clap 4.5。
- workspace 统一 `edition 2024 / rust-version 1.88`，release profile：thin LTO + codegen-units=1 + strip symbols。

---

## 6. 平台限制与环境事实清单

1. **rust-analyzer 组件缺失（已修复）**：stable 工具链未安装 rust-analyzer 组件（`rust-analyzer --version` 报 "Unknown binary"），导致首次 LSP 采集失败（ServiceStopped）。已执行 `rustup component add rust-analyzer`（退出码 0），此后诊断正常。此为本节点唯一环境变更，不涉源码。
2. **hostd 两个 live 测试被 ignore（预期内平台/环境限制，非缺陷）**：需 operator 提供预载镜像、一次性 Compose/Caddy 实例；本机 Windows 无此环境。非 live 的 13 个 hostd 测试（含 sandbox 策略、driver 契约、workspace 预算、compose/caddy 静态校验、健康探测不跟随重定向等）在 Windows 上全部编译通过且通过。
3. **hostd 本体面向 Linux**（Unix Socket、特权部署），但其单元测试设计为离线/离线契约式，Windows 可编译可运行——符合"平台限制如实记录，不误判为缺陷"的原则。
4. **e2e 是真实后端集成测试**：`test:e2e` 会构建 `soloops-server` 的 `e2e_harness` 示例（feature `e2e-harness`）并作为 Playwright webServer。本次失败（2/2）发生在浏览器与 harness 均正常启动之后，**不属于环境缺失**；证据见 §1.9。Windows 上 webServer 命令使用 `target\debug\examples\e2e_harness.exe`（配置已含平台分支）。
5. **无 git remote**：仓库纯本地，2 个提交；基线仅反映工作区状态，无法与远端/其他分支对照。
6. **Vite configLoader 原生化警告**（§1.5）：`src/mock` 的目录索引与无扩展名导入触发 12 条提示，未来 Vite 大版本（native loader 默认化）可能破坏构建，建议后续节点关注 mock 模块导入风格。
7. **`参考/pi-main` 未跟踪目录**：外部参考项目，非本仓库代码，审查节点应排除。
8. **`outputs/` 下存在若干运行时文件**（.callid.txt、.run.json、.task.json、pi-design-improvement-study.md、ui-audit/ 等）：非源码，本基线产出 `outputs/code-review/` 与其并列，互不干扰。
9. **rustc 实际版本 1.95.0** 高于 workspace 声明的 rust-version 1.88；edition 2024 语法已大量使用。
10. **`.env` 未被读取**（避免真实密钥进入报告/日志）；`.env.example` 无真实密钥，密钥均以 `*_REF=env:…` 引用模式注入。

---

## 基线总结论

**Rust 侧完全绿**：fmt、clippy -D warnings、106 项测试（104 通过 + 2 项预期 ignore）、rust-analyzer 零诊断。**前端静态链路完全绿**：eslint、svelte-check、mock 单测、生产构建。**唯一红灯是 e2e（2/2 失败）**，证据指向 UI 重构后 e2e 规格未同步（选择器漂移），其中第 2 例（`managed.deploy.apply` 未出现）不排除 create→run 链路在真实 harness 下的功能回归，是后续节点最优先的深查对象。
