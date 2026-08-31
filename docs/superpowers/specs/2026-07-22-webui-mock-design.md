# WebUI Mock 测试替身设计

- **日期:** 2026-07-22
- **范围:** 为 SoloOps SvelteKit WebUI 提供一个无需真实后端的纯 UI 测试替身 mock
- **状态:** 设计已批准,待写实现计划

---

## 一、目标与非目标

### 目标

- 让 `bun run dev:web` 单进程即可驱动 WebUI 的全部核心交互,无需模型 API Key、`migrate`、`owner-init` 或任何 Rust 进程。
- mock 自动推进一条快路 Run 生命周期剧本,使人工测试能观察各状态 UI。
- mock 与前端同进程、同代码库,用环境变量控制开关;关闭时不影响与真实 Rust 后端的联调。

### 非目标(YAGNI)

- 不做多剧本(预算超限、needs_recovery、corrupt stream 等)。
- 不做动态模型循环。
- 不集成进生产构建(仅 dev server 生效)。
- 不做任何持久化;dev server 重启即重置内存状态。
- 不校验 `SOLOOPS_MODEL_NAME` 等真实配置。

---

## 二、集成方式

### 2.1 形态

一个 **Vite 插件**,源码放在 `apps/web/src/mock/`,通过 `VITE_MOCK=1` 环境变量开启。

- **开启时:** 插件在 `configureServer` 中注册 `/api/*` REST 处理器,并通过 `httpServer.on('upgrade')` 拦截 `/api/events` 的 WebSocket 升级。所有 `/api` 请求由 mock 直接响应、返回 `false`(或等价方式)以**阻止**现有 `:3001` 代理命中。
- **关闭时:** 插件为 no-op,`/api` 照常被 `vite.config.ts` 现有 proxy 转发到 `http://127.0.0.1:3001`。

### 2.2 开启方式

```powershell
# Windows
$env:VITE_MOCK=1; bun run dev:web
# 或 package.json 增加一个 script:
# "dev:mock": "bun run --cwd apps/web --bun dev -- --mode mock"
# 配合 apps/web/.env.mock 里 VITE_MOCK=1
```

推荐在 `apps/web/` 增加 `.env.mock`(含 `VITE_MOCK=1`,加入 `.gitignore` 或作为示例 `.env.mock.example`),并在根 `package.json` 增 `dev:mock` 脚本。具体 env 注入机制在实现计划中确定,但开关统一为 `import.meta.env.VITE_MOCK`(Vite 惯例)或 `process.env.VITE_MOCK`(server 侧)。由于插件运行在 Vite server 侧(Node/Bun 进程),读取 `process.env.VITE_MOCK`;通过 `--mode mock` 加载 `.env.mock` 可同时保证两端可见。

### 2.3 WebSocket 处理

浏览器对 `/api/events` 发起 WS 升级。mock 在 `configureServer` 拿到 `server.httpServer`,监听 `'upgrade'` 事件,当 URL 以 `/api/events` 开头时用 `ws` 库升级连接并纳入活跃连接集合;其余 upgrade 交给默认行为。

> **依赖:** 新增 `ws` 与 `@types/ws` 到 `apps/web` devDependencies。Vite server 已基于 Node http,`ws` 可直接挂载。

---

## 三、状态机驱动(单条快路剧本)

### 3.1 剧本序列

建任务后,mock 用一个**确定性定时器序列**推进状态。每一步通过 `/api/events` WebSocket 推送对应 `EventType`。时间间隔为可调常量(默认约 1.5s 一步),便于人工观察。

```text
queued
  → planning        (推送 agent.plan_updated,带 AgentPlan)
  → running         (推送 tool.call_started,一个 workspace_write Tool Call)
  → waiting_for_approval   (Run 状态切换;Tool Call 状态置 waiting_for_approval,UI 出现 Approve/Deny)
  → [用户点 Approve]
  → running         (推送 tool.call_completed,带 resultSummary)
  → verifying
  → reporting       (推送 run.reported,带 FinalReport)
  → succeeded       (推送 run.status_changed → succeeded)
```

每步同时更新 `RuntimeSnapshot` 内存状态,前端通过事件触发 `refreshRuntime()` 拉取最新快照。

### 3.2 分支收尾

- **Deny:** 用户点 Deny → Tool Call 置 `denied` → Run 转 `failed`,`statusReason` 记 "Tool call denied"。简单收尾,不实现复杂恢复。
- **Cancel:** 用户点 Cancel → 调 `/api/runs/:id/cancel` → Run 直接转 `cancelled`,停止剧本定时器。

### 3.3 事件与 sequence

- 全局单调递增 `sequence`,WS 推送与 HTTP `/api/events?after=` 补拉**共用同一事件源**。
- 每个事件附 `createdAt`(毫秒时间戳)、`runId`、`type`、`payload`。
- 这保证前端断线重连(`fetchPersistedEvents` + WS `after` 游标)逻辑可被测到。

---

## 四、组件拆分

| 文件                   | 职责                                                                                            | 依赖                         |
| ---------------------- | ----------------------------------------------------------------------------------------------- | ---------------------------- |
| `src/mock/index.ts`    | Vite 插件入口:`process.env.VITE_MOCK` 开关、`configureServer` 注册 REST 处理、挂 `upgrade` 监听 | vite, router, state, clients |
| `src/mock/state.ts`    | 内存状态:Owner/Task/Run/RuntimeSnapshot 的创建、查询、状态转换                                  | `contracts.ts` 类型          |
| `src/mock/scenario.ts` | 快路剧本:定时器序列 + 状态推进 + 事件派发                                                       | state, clients               |
| `src/mock/router.ts`   | REST 路由分发(login / tasks / runs / runtime / decision / cancel / events-HTTP)                 | state                        |
| `src/mock/clients.ts`  | 活跃 WebSocket 连接集合 + 按 runId 广播事件                                                     | ws                           |

每个文件单一职责,可独立理解与测试。文件保持小,避免单文件膨胀。

### 关键接口

`state.ts` 对外暴露(示意):

```ts
createTask(title, goal): TaskSummary      // 同时创建初始 Run 与 RuntimeSnapshot
getTask(id): TaskSummary | undefined
getRun(id): RunDetail | undefined
getRuntime(id): RuntimeSnapshot | undefined
applyDecision(runId, callId, decision): ToolCallSummary  // 推进或收尾剧本
cancelRun(id): RunDetail                  // 转 cancelled
listEvents(after, runId?): EventEnvelope[]  // HTTP 补拉用
```

`scenario.ts`:

```ts
startScenario(runId): void   // 启动定时器序列;内部记录 step
advanceTo(runId, step): void // 推进并广播事件 + 更新 runtime
```

---

## 五、接口契约对齐

mock 严格遵循 `docs/api/openapi.yaml` 与 `apps/web/src/lib/contracts.ts`:

- **登录:** `POST /api/auth/login` 任意 username/password → `200` + `SessionResponse` + `Set-Cookie: soloops_session=mock-<rand>; HttpOnly`。
- **会话:** `GET /api/auth/session` 在有 cookie 时返回 `SessionResponse`,否则 `401`。
- **登出:** `POST /api/auth/logout` → `204`,清除会话。
- **建任务:** `POST /api/tasks` → `201` + `TaskSummary`(含 `latestRunId`),触发 `scenario.startScenario`。
- **查询:** `GET /api/tasks`、`GET /api/tasks/:id`、`GET /api/runs/:id`、`GET /api/runs/:id/runtime`。
- **审批:** `POST /api/runs/:id/tool-calls/:callId/decision` → `200` + `ToolCallSummary`,推进剧本。
- **取消:** `POST /api/runs/:id/cancel` → `200` + `RunDetail`。
- **事件 HTTP:** `GET /api/events?after=&runId=` → `{ items: EventEnvelope[] }`。
- **错误:** 404/401 按 `ApiError` 结构返回(`{ error: { code, message } }`),让前端 401→跳 `/login` 逻辑可测。
- **健康检查:** `GET /healthz`、`/readyz`、`/livez`、`/metrics` 返回最小桩响应(前端不依赖,但保持同源可达)。

**故意不模拟:** `event_stream_corrupt` / WS close code `4002` 路径(单条快路不覆盖,YAGNI)。

---

## 六、错误处理与边界

- mock 拦截 `/api` 时**不调用 `next()`**,确保不被 `:3001` 代理命中;非 `/api` 路径照常交给 Vite。
- WS 连接记录每个连接的 `after` 游标;重连时 mock 先经 HTTP `/api/events?after=` 补拉增量,再继续 WS 推送,复用与真后端一致的重连语义。
- 剧本定时器在 Run 进入终态(`succeeded`/`failed`/`cancelled`)时清除,避免泄漏。
- 内存态:dev server 重启即重置,符合 mock 预期行为。
- 已有的 `vite.config.ts` proxy 配置**保留不动**(关闭 mock 时仍生效),mock 仅在开启时短路 `/api`。

---

## 七、测试

### 7.1 单元测试

- `state.ts`:状态转换正确性、`listEvents` 游标语义。
- `scenario.ts`:在假定时器下,启动后按预期顺序到达各状态并派发对应 `EventType`;Approve/Deny/Cancel 各自的收尾状态。

### 7.2 手测验收路径

1. `VITE_MOCK=1 bun run dev:web`
2. 登录页任意账号登录
3. 新建任务 → 跳转 Run timeline
4. 观察:plan 出现 → budget/usage 增长 → Tool Call 出现等待审批
5. 点 **Approve** → Tool 完成 → 验证 → 报告 → succeeded,看到 FinalReport markdown
6. 重复一次,点 **Deny** → 看到 failed
7. 重复一次,点 **Cancel** → 看到 cancelled
8. 期间断网/刷新验证 WS 重连补拉

---

## 八、文件清单(实现产出)

新增:

- `apps/web/src/mock/index.ts`
- `apps/web/src/mock/state.ts`
- `apps/web/src/mock/scenario.ts`
- `apps/web/src/mock/router.ts`
- `apps/web/src/mock/clients.ts`
- `apps/web/src/mock/scenario.test.ts`
- `apps/web/src/mock/state.test.ts`

修改:

- `apps/web/vite.config.ts` — 引入 mock 插件
- `apps/web/package.json` — 增 `ws`、`@types/ws` 依赖,可选 `dev:mock` script
- 根 `package.json` — 增 `dev:mock` script(可选)
- `apps/web/.env.mock.example`(可选)及 `.gitignore` 处理

> **构建边界:** `ws` 仅在 dev server 进程内使用,绝不进入浏览器 bundle。实现上,mock 插件只在 `command === 'serve'`(dev)时加入 `plugins` 数组,且 `ws` 的 import 在 mock 模块顶层;生产构建(`vite build`)根本不会加载 mock 插件,因此 `ws` 不会被打入 `adapter-static` 产物。
