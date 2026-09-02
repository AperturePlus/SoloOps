# 07a — WebUI 契约与安全审查（apps/web 路由页面 / EventStream / e2e 定性）

- **审查对象**: 工作区状态（分支 `feature/agent-page-polish`，未提交变更）。contracts.ts 223 行全文 + 9 个路由页面 2464 行全文 + EventStream.svelte 183 行全文 + e2e 2 文件
- **方法**: 契约核对以 05-server.md 契约清单为基线，全部 DTO 直接回读 crates/soloops-domain（run.rs/runtime.rs/event.rs/requests.rs/error.rs/security.rs）逐字段比对（超出"抽查 2-3 个"要求，实际全量核对）；事件 payload 键名对照 04a 键名表；e2e 两失败以 00-baseline §1.9 + 04a-F3 为前置
- **前序事实**: 06-hostd.md grep "给 webui/预览字段契约" 0 命中——无专门 webui 契约段，其相关结论为 ManagedDeploy action 字面量 hostd 侧逐字一致（06 章前序线索答复行）
- **本节点纪律**: 只读未改任何源码；未跑 e2e（需浏览器）；bun check + lint 已跑（见文末）

---

## contracts.ts × 05 契约清单 核对表（首批成果）

**结论：contracts.ts 与 Rust domain/server DTO 逐字段零漂移。** 全部 27 个类型/枚举（23 个核对行）经 Rust serde 源码直接比对（非转述）：

| contracts.ts 类型 | Rust 权威源（已回读） | 核对结果 |
|---|---|---|
| RunStatus 15 值 | domain/run.rs:6-42 `RUN_STATUSES` + `rename_all="snake_case"` | ✅ 逐字吻合（含死状态 draft/paused——02-F4/05-F8 传播确认抵达前端） |
| Owner {id,username} | domain/requests.rs:5-8 | ✅ |
| IpNotificationRecipientStatus 5 字段 | domain/requests.rs:25-31 | ✅ 可空性（Option→`\| null`）全对 |
| IpNotificationSettings 6 字段 | domain/requests.rs:35-42 | ✅ |
| TestIpNotificationResponse | domain/requests.rs:53-56 | ✅ |
| SshKeyFileRole "user"\|"administrators" | domain/security.rs:6-12 `rename_all="lowercase"` | ✅ |
| SshAuthorizedKeyEntry 12 字段 | domain/security.rs:17-39 | ✅（line u32、keyBits u32、fromPatterns/forcedCommand Option 全对） |
| SshAuthorizedKeysFile 5 字段 | domain/security.rs:44-50 | ✅ |
| SshMachineSummary | domain/security.rs:55-59 | ✅ |
| SshAccessReport 8 字段 | domain/security.rs:65-75 | ✅ |
| TaskSummary {id,title,goal,status,latestRunId,createdAt} | domain/runtime.rs:5-12 | ✅（status 确为 RunStatus 类型） |
| RunDetail 7 字段 | domain/runtime.rs:16-24 | ✅（statusReason/startedAt/finishedAt 可空全对；确无 tool_calls，与 05-F1 叙述一致） |
| EventType 8 值 | domain/event.rs:7-24（serde rename 逐字 "run.created" 等） | ✅ 与 04a"仅 8 种 EventType"一致 |
| EventEnvelope {sequence,id,runId,type,payload,createdAt} | domain/event.rs:63-73 | ✅ 外层键名与 05:254 清单逐字一致；runId Option→`\| null` ✅ |
| RuntimeCheckpoint 8 值 | domain/runtime.rs:28-37 snake_case | ✅ |
| PlanStepStatus 4 值 / AgentPlan / PlanStep.required | domain/runtime.rs:41-56 | ✅ |
| BudgetSnapshot 8 字段 / UsageSnapshot 7 字段 | domain/runtime.rs:73-111 | ✅ |
| ToolRisk 5 值 / PolicyDecision 3 值 / ToolCallStatus 7 值（含 unknown） | domain/runtime.rs:115-141 | ✅ |
| ToolCallSummary 11 字段 | domain/runtime.rs:145-157 | ✅（与 05-F1 引用段一致：无 arguments 全文字段） |
| EvidenceSummary 8 字段 | domain/runtime.rs:161-170 | ✅ |
| FinalReport 9 字段 | domain/runtime.rs:174-184 | ✅ |
| RuntimeSnapshot 9 字段 | domain/runtime.rs:188-198 | ✅ |
| ApiError {error:{code,message,details?}} | domain/error.rs:5-15（details `skip_serializing_if=None`） | ✅ details 可选语义正确 |

**类型收窄注记（非漂移，2 处）**: ① `EventEnvelope.payload: Record<string, unknown>`（contracts.ts:121）对应 Rust `serde_json::Value`——Value 可为任意 JSON 标量/数组，TS 收窄为对象。实际事件 payload 按 04a 键名表恒为对象，运行时安全，但若后端未来写入非对象 payload，TS 类型在运行时为假。② `ToolCallSummary.approvalPreview: Record<string, unknown> | null`（contracts.ts:180）对应 `Option<serde_json::Value>`，同理。均属"前端比后端承诺更窄"的防御性收窄，标注 informational。

**契约缺口（TS 侧未镜像的请求体类型，需在各页面核对实际发送形状）**: ApprovalDecisionRequest {decision:"approve"|"deny", reason?:string≤1000}（runtime.rs:209-212 + normalize :214-231）、CreateTaskRequest {title≤160, goal≤20000}（requests.rs:60-63）、LoginRequest {username≤64, password≤1024}（requests.rs:12-15）、UpdateIpNotificationSettingsRequest {enabled, recipients:string[]}（requests.rs:46-49）、SessionResponse {owner}（requests.rs:19-21）。逐一在对应页面核对中。

## [P2] F1：前端 TERMINAL 状态集三处均缺 "blocked"——与 domain `is_terminal` 漂移，active 计数/isActive/断连重连行为全部失真
- 位置: apps/web/src/routes/+page.svelte:17；apps/web/src/routes/tasks/+page.svelte:66；apps/web/src/routes/runs/[runId]/+page.svelte:30
- 置信度: 高
- 证据:
```ts
// +page.svelte:17（Overview）
const TERMINAL = new Set(["succeeded", "failed", "cancelled"]);
// tasks/+page.svelte:66
tasks.filter((task) => !["succeeded", "failed", "cancelled"].includes(task.status)).length
// runs/[runId]/+page.svelte:30
const TERMINAL_STATUSES = new Set(["succeeded", "failed", "cancelled"]);
```
```rust
// crates/soloops-domain/src/run.rs:65-70（权威定义）
pub const fn is_terminal(self) -> bool {
    matches!(self, Self::Blocked | Self::Succeeded | Self::Failed | Self::Cancelled)
}
```
- 问题: domain 判定 Blocked 为终态，前端三处终端集合都漏掉它。后果：① Overview/tasks 页 "Active runs/N active" 把 blocked（终态、永不推进）任务永久计入活跃数；② run 页 `isActive`（:38）为真 → "live" 指示器常亮、Cancel 按钮常在（:292-294 用同一集合判定）；③ WS 断连后 `scheduleReconnect`（:152）对 blocked run 永不停止重连（指数退避封顶 30s 无限循环）。blocked 是审批链路真实可达状态（04a：block_managed_change → Blocked）。
- 建议: 三处统一改为含 "blocked"（更稳妥：从 `contracts.ts` 导出单一 `TERMINAL_STATUSES` 常量），或直接镜像 `is_terminal`。

## [P2] F2：login 页 fire-and-forget `goto("/tasks")`——登录响应迟到时劫持任意后续导航（e2e 失败 2 的 app 侧机制）
- 位置: apps/web/src/routes/login/+page.svelte:17-22
- 置信度: 高（e2e 快照反推的时间线闭环；真实用户触发窗口为登录往返时长）
- 证据:
```ts
async function login(event: SubmitEvent) {
  event.preventDefault();
  ...
  try {
    await api("/api/auth/login", { method: "POST", body: JSON.stringify({ username, password }) });
    await goto("/tasks");        // ← await 返回时用户可能早已离开 /login
  } catch (cause) { ... }
}
```
- 问题: Argon2 验证使 login 往返达数百 ms（05-F3 佐证其计算量）。期间用户点击左侧 rail 任意链接（如 "New task"）导航离开后，原 login 闭包在响应到达时仍执行 `goto("/tasks")`，把用户从当前页面（可能正在填表单）强行拽回 /tasks，表单内容丢失。组件卸载不会取消该导航。e2e 失败 2 的实际机制正是它（详见文末 e2e 定性小节）。
- 建议: goto 前校验当前路由仍为 /login（`page.url.pathname === "/login"`，用 `$app/state` 的 page），或引入 `onDestroy` 置位标志；顺带支持 `?redirectTo=` 回跳。

## [P2] F3：runs/[runId] `refreshRuntime` 无请求时序防护——并发触发时过期响应可覆盖新快照
- 位置: apps/web/src/routes/runs/[runId]/+page.svelte:75-81、106-124
- 置信度: 中（竞态路径成立；HTTP 并发下响应乱序真实可发生，触发频率取决于事件突发密度）
- 证据:
```ts
async function refreshRuntime() {
  try {
    runtime = await api<RuntimeSnapshot>(`/api/runs/${encodeURIComponent(data.runId)}/runtime`);
  } catch (cause) { if (!(cause instanceof ApiClientError && cause.status === 404)) throw cause; }
}
// applyEvent 内：每个 run.status_changed / agent.* / tool.* / run.reported 事件都触发一次
void refreshRuntime();   // :122 —— 无并发防护，无 AbortController/序号检查
```
- 问题: WS 突发（重连补拉尤其常见——REST 回放 forEach 逐个 applyEvent）会并发发起多个 refreshRuntime；响应乱序时旧快照后到并覆盖新快照。若最后两个事件分别是 tool.call_completed 与 run.reported，终报快照可能被前一个事件的旧快照覆盖且无后续事件自愈——审批横幅闪回（awaitingCalls 复活）、终报短暂消失。另有次生问题：`void refreshRuntime()` 吞掉非 404 异常为 unhandled rejection。
- 建议: 用"仅最后一次生效"模式（保存当前 Promise/序号，返回时校验自己仍是最新一次）或 AbortController；applyEvent 触发的刷新合并为每批一次。

## [P2] F4：审批提交 `decide()` try/finally 无 catch——审批失败（含 05-F2 重复提交→500）对 Owner 零反馈
- 位置: apps/web/src/routes/runs/[runId]/+page.svelte:83-98、338
- 置信度: 高
- 证据:
```ts
async function decide(callId: string, decision: "approve" | "deny") {
  deciding = true;
  try {
    await api(`/api/runs/${...}/tool-calls/${...}/decision`, { method: "POST", ..., body: JSON.stringify({ decision }) });
    await refreshRuntime();
  } finally { deciding = false; }   // ← 无 catch：错误直接逃逸成 unhandled rejection
}
```
- 问题: 审批是 SoloOps 的核心人工控制点。POST 失败（网络断、会话过期、05-F2 已确认的重复提交→500 catch-all）时错误静默吞掉：横幅保持 waiting_for_approval 原样、无任何错误提示，Owner 无从得知审批未生效；与 05-F2 组合成"双击按钮 → 500 → 静默 → 无法判断是否已批准"的坏体验。
- 建议: catch 中设置 error 状态并渲染（页面已有 error 横幅通道 :309-315）；对重复提交给出幂等确认提示。

## [P3] F5：cancelRun 与 logout 均无错误处理——失败时 unhandled rejection，cancel 按钮无反馈
- 位置: apps/web/src/routes/runs/[runId]/+page.svelte:100-104；apps/web/src/routes/tasks/+page.svelte:73-76
- 置信度: 高
- 证据:
```ts
async function cancelRun() {
  run = await api<RunDetail>(`/api/runs/${encodeURIComponent(data.runId)}/cancel`, { method: "POST" });
}
// tasks/+page.svelte:73-76
async function logout() {
  await api("/api/auth/logout", { method: "POST" });
  await goto("/login");
}
```
- 问题: cancel POST 失败（401/404/500）时按钮点击无任何可见效果且异常进控制台；logout 失败时用户停留在原页（goto 未执行），无提示。低频但均为可点按钮的主路径。
- 建议: 两处补 try/catch，cancel 复用 error 通道，logout 失败也应提示。

## [P3] F6：/tasks 页默认态无任何 heading——"Tasks" 标签降级为 span（e2e 失败 1 的 UI 侧缺陷）
- 位置: apps/web/src/routes/tasks/+page.svelte:105-107、231-243
- 置信度: 高
- 证据:
```svelte
<!-- 侧栏标题是 span，非 heading -->
<span class="min-w-0 flex-1 truncate text-sm font-semibold ...">Tasks</span>
<!-- 空态右区只有段落与按钮，零 heading -->
<p class="text-sm text-muted-foreground">No task selected</p>
<Button href="/tasks/new" ...>Create a task</Button>
```
- 问题: 复合布局后 /tasks 页在无选中任务时全页无 h1/h2（选中后 PageHeader 才渲染 h1，且名称是任务标题）；页面地标导航（heading rotor）无法定位到 "Tasks"。属可访问性回归 + e2e 断言 `heading "Tasks"`（spec:10）失败的直接 UI 侧原因。
- 建议: 侧栏 "Tasks" span 升级为 `<h1 class="...">Tasks</h1>`（或给右区空态加 sr-only h1），一次性修复 a11y 并让旧断言语义成立。

## [P3] F7：login 页 username 预填 "owner"——把唯一账号名硬编码进 UI
- 位置: apps/web/src/routes/login/+page.svelte:8
- 置信度: 高
- 证据:
```ts
let username = $state("owner");
```
- 问题: 单 Owner 系统 username 近公开（05-F3 同口径：枚举收益低），但硬编码默认值会随账号改名失配，且向任何打开登录页的人展示有效用户名。安全性影响小，属信息暴露便利权衡。
- 建议: 若保留，至少与 soloopsctl 引导文案解耦；或改为空输入 + placeholder 提示。

## [P3] F8：security 页 `refreshing` 是死标志——"Scanning…" 永不显示，Rescan 扫描期间不禁用
- 位置: apps/web/src/routes/settings/security/+page.svelte:16、21-31、99-102
- 置信度: 高
- 证据:
```ts
let refreshing = $state(false);          // :16 唯一写入点是 finally 里的 false
async function load() {
  try { report = await api<SshAccessReport>("/api/settings/ssh-access"); }
  ...
  finally { loading = false; refreshing = false; }   // 从未置 true
}
<!-- :99-102 -->
<Button variant="outline" type="button" onclick={load} disabled={loading || refreshing}>
  {refreshing ? "Scanning…" : "Rescan"}
```
- 问题: `refreshing` 从未被置 true，"Scanning…" 分支是死代码；首次加载后 `loading` 恒 false，Rescan 点击后按钮不禁用，可连点并发多个 GET /api/settings/ssh-access（幂等只读无数据损坏，但按钮的防重与状态反馈意图落空）。
- 建议: load 开头区分首载与 rescan（或统一 `refreshing = true`），finally 复位。

## [P3] F9：security 页 each 块 key 唯一性无保证——fingerprint 可空可重复（Windows 双角色文件天然撞键）
- 位置: apps/web/src/routes/settings/security/+page.svelte:180、164、208、224
- 置信度: 中（重复键的渲染后果依 Svelte 版本而异：开发警告/列表项错乱）
- 证据:
```svelte
{#each machineGroups as group (group.name ?? "__unknown__")}   <!-- :164 -->
  {#each group.keys as key (key.fingerprint)}                  <!-- :180 -->
  {#each key.fromPatterns ?? [] as pattern (pattern)}           <!-- :208 -->
  {#each plainOptions(key) as option (option)}                  <!-- :224 -->
```
- 问题: contracts.ts 允许 `fingerprint: string | null`（Rust Option 同步），valid 条目指纹解析失败时可全为 null；Windows 扫描同时覆盖 user 与 administrators 两个文件（SshKeyFileRole 两值），同一把公钥写进两个文件时 fingerprint 相同、machine 相同 → 同一 group 内撞 key。Svelte keyed each 重复 key 会导致渲染异常/开发警告。次要：group name 为 null 的多个来源合并为同一 "__unknown__" 键；fromPatterns/options 按值作 key，同值选项同样撞键。
- 建议: key 改用 `key.file + ":" + key.line`（文件内唯一，跨文件也唯一）；group 键加序号。

## [P3] F10：notifications 页 `lastError` 只显示"Last delivery failed"——失败原因被丢弃，契约字段形同虚设
- 位置: apps/web/src/routes/settings/notifications/+page.svelte:235-239
- 置信度: 高
- 证据:
```svelte
{#if recipient.lastError}
  <span class="text-red-300">Last delivery failed</span>
{:else}
  Last sent: {date(recipient.lastNotifiedAt)}
{/if}
```
- 问题: 契约专门提供 `lastError: string | null`（server 侧 record_ip_notification_failure 写入，05 正面确认 4），UI 只提示"失败"不展示原因；Owner 排障（邮箱拼错？SMTP 拒收？）必须去后端日志。tooltip 位置当前显示的是 lastNotifiedAt 的完整时间——失败行连时间都没有。
- 建议: 失败分支 tooltip 或次行展示截断的 lastError（SMTP 错误可能含内部主机名，注意截断）+ lastAttemptAt。

## [P2] F11：e2e spec 三重缺陷——登录后无导航等待（失败 2 直接根因）、"New task" 选择器二义、断言整体漂移于复合布局
- 位置: apps/web/e2e/happy-path.spec.ts:10-11、38-45；对照 apps/web/src/routes/+layout.svelte:58 与 tasks/+page.svelte:199-205
- 置信度: 高（失败 2 机制已由快照+代码反推闭环，详见 e2e 定性小节）
- 证据:
```ts
// spec:38-45 —— 登录点击与 "New task" 点击之间零等待/零断言
await page.getByRole("button", { name: "Sign in" }).click();
await page.getByRole("link", { name: "New task" }).click();   // /tasks 上有 2 个该名链接（rail + 页脚）
...
await page.getByRole("button", { name: "Create task" }).click();
await expect(page.getByText("managed.deploy.apply")).toBeVisible({ timeout: 15_000 });  // :47 失败点
```
```svelte
<!-- rail: +layout.svelte:58 --><a href="/tasks/new" aria-label="New task" ...>
<!-- 页脚: tasks/+page.svelte:199-205 --><a href="/tasks/new">…New task</a>
```
- 问题: ① :38 点 Sign in 后不等待登录完成即点 "New task"——Argon2 往返期间浏览器仍在 /login，rail 链接成为唯一匹配而侥幸通过（一旦修好等待、落在 /tasks 上将命中 2 个元素触发 strict mode violation）；② 主 e2e 套件 2/2 全红，回归检测能力归零——"审批是核心控制点"的产品其唯一 e2e 全部失效；③ :10 "Tasks" heading、:11 "Phase 3.1"（现徽标文本为 "3.1"）、:17 "Run timeline"（现 run 页无此 heading）全部漂移。
- 建议: 登录后 `await page.waitForURL("**/tasks")`；选择器限定 `getByRole("navigation", { name: "Primary" }).getByRole("link", { name: "New task" })`；断言按现 UI 重写（见 e2e 定性小节修复清单）。

## [P3] F12：tasks 页 loadRuntime 与 run 页 task 拉取吞掉全部错误（含 401）——会话过期静默降级
- 位置: apps/web/src/routes/tasks/+page.svelte:36-44；apps/web/src/routes/runs/[runId]/+page.svelte:171-177
- 置信度: 高
- 证据:
```ts
// tasks/+page.svelte:39-43
try { runtime = await api<RuntimeSnapshot>(`/api/runs/${...}/runtime`); }
catch { runtime = null; }              // ← 401 也被吞：不跳登录、不提示
// runs/[runId]/+page.svelte:171-177
try { task = await api<TaskSummary>(`/api/tasks/${...}`); } catch { task = null; }
```
- 问题: tasks 页切换任务时若会话已过期，"Latest run" 面板静默消失、页面无任何反馈也不跳登录（与页面级 401→goto("/login") 的既有模式不一致）；run 页 task 标题同理回退为 "Run"。非 401 错误（500）同样无痕。
- 建议: catch 中至少区分 `ApiClientError.status === 401` 走 goto("/login")，其余设置 error。

## [P3] F13：security 页对 entries/invalidLines 无渲染上限——镜像 05-F5 的服务端无界读取，前端无防线
- 位置: apps/web/src/routes/settings/security/+page.svelte:179-241、255-266
- 置信度: 高（前端侧确证；触发前提继承 05-F5——本地 authorized_keys 被写入巨型文件）
- 证据:
```svelte
{#each group.keys as key (key.fingerprint)}   <!-- 全量渲染，无虚拟化/分页 -->
...
{#each invalidEntries as entry (entry.file + entry.line)}
```
- 问题: 服务端读取 authorized_keys 无大小上限（05-F5，P3"非远程可控输入"），前端对返回的 entries/invalidLines 全量渲染 DOM——每条 4-6 个节点 + 动画 class，数万行 authorized_keys 会产出十万级节点，页面卡死。与 05-F5 组成"服务端不设限 + 前端不设防"。
- 建议: 前端每文件/每组渲染上限（如前 200 条 + "show all"），或依赖服务端修复 05-F5 时一并截断。

---

## e2e 失败最终定性（本章核心结论）

### 失败 1（spec:10 heading "Tasks" 超时）——定性：测试断言漂移为主 + UI 可访问性缺陷为次（F6），非功能性回归
- "Tasks" 在新复合布局里是侧栏 `<span>`（tasks/+page.svelte:105-107）而非 heading；无任务选中时全页零 heading（:231-243 空态只有段落/按钮），选中后唯一的 h1 是 PageHeader 渲染的任务标题（PageHeader.svelte:60）。`getByRole("heading", {name:"Tasks"})` 在任何状态下都不可能命中。
- 快照（00-baseline §1.9）证明应用渲染正常（登录/列表/徽标俱全）——**UI 无故障，规格未随重构更新**。同一用例后续断言 "Phase 3.1"(:11，现文本 "3.1")、"Run timeline"(:17，现 run 页 h1=任务标题/h2="Activity") 同为漂移，测试死在第一处。
- 修复：UI 侧按 F6 把侧栏标题升级 h1（同时修 a11y 并让断言语义成立），或 spec 侧改为断言侧栏文本；二者取其一即可转绿，推荐前者。

### 失败 2（spec:47 15 秒未见 managed.deploy.apply，快照停在 "No task selected"）——定性：**非 create→run 链路功能回归**；根因 = spec 登录后抢跑（F11①）+ login 页迟到 goto 劫持（F2）的复合竞态
**04a-F3 的假设（"Create task 不再导航/选中 run 视图"）被否证**：tasks/new:25 存在 `await goto(...)` 导航到 `/runs/${task.latestRunId}`，且 POST /api/tasks 响应确为 TaskSummary（http.rs:295-309，Json<TaskSummary> 含 latestRunId）——导航代码与数据源俱在。后端链路 04a/05/06 已三方排除，本节点在前端侧闭合最后一环：

时间线（每一环都有代码+快照强制约束）：
1. spec:38 点击 Sign in → login POST 在途（Argon2 往返数百 ms，05-F3 佐证）。
2. spec:40 立即点击 "New task"——此刻浏览器仍在 /login（**反证**：若已在 /tasks，rail（+layout.svelte:58）与页脚（tasks/+page.svelte:199-205）两个同名链接会触发 strict mode violation，失败点就不是 :47）。/login 上唯一匹配是 rail 链接 → 无报错 → goto /tasks/new。
3. spec:41-45 在 /tasks/new 填 Title/Goal、点 Create task → POST /api/tasks **无会话 cookie**（login 未返回）→ 401 → tasks/new:27 `return goto("/login")`。
4. login POST 到达：cookie 落地；**已卸载的 login 组件闭包**继续执行 `goto("/tasks")`（login/+page.svelte:22，即 F2）→ 浏览器被拽到 /tasks。
5. /tasks onMount：GET /api/tasks → `{items: []}`（任务从未建成）→ 侧栏 "Tasks 0 / No tasks / 0 active"，右区 "No task selected"。
6. spec:47 在 /tasks 上等 managed.deploy.apply 15s → 超时。

快照逐元素吻合（test-results/happy-path-owner-reviews-a-4e22d-…/error-context.md:44-57）：`Tasks 0`、`Collapse task list`、`Filter tasks…`、`No tasks`、`0 active`、`No task selected`、`Create a task`、`Tasks · SoloOps`——且无 error 横幅、无登录表单，证明最终态是**已认证的空任务列表**（GET 200），而非加载失败或未登录。快照里 "No task selected" 与 "Tasks 0" 的组合**只能**由"创建请求 401 失败 + 迟到 goto 落地 /tasks"这一条路径产生（若创建成功，列表非空且自动选中 tasks[0]；若未认证，/tasks 的 onMount 会跳回 /login）。

**同源性回答**：两条失败同族（复合布局重构未同步 spec——与 04a-F3"同源 UI 漂移"判断在族级一致），但机制不同：失败 1 是纯断言漂移；失败 2 是 spec 竞态（无登录等待）叠加 F2 的迟到 goto。失败 1 里 :10 的 expect 若存在，本可作为失败 2 的隐式登录同步屏障——它在旧布局下的存在掩盖了 spec 从不等待登录的结构性缺陷。

**修复清单**（两层，缺一不可）：
- spec：登录后 `await page.waitForURL("**/tasks")`；:40 选择器消歧为 `getByRole("navigation", { name: "Primary" }).getByRole("link", { name: "New task" })`（否则加上等待后必触发 strict mode violation）；:47 起的 managed.deploy.apply / "privileged · waiting_for_approval" / "Approval preview" / "e2e.example.test" / "arguments sha256:" 断言依赖 ToolCallCard/ApprovalBanner 的渲染文本——**移交 7b 核对文案是否仍逐字存在**。
- app：按 F2 给 goto 加守卫；顺带 tasks/new 401 跳转支持 ?redirectTo 回跳。

### global-teardown.ts——无发现
shutdown → 校验 `{ok, errors[]}`（harness 后台错误会让 teardown 失败，与 05-F11 的 JoinError 面闭环）→ 5s 轮询 healthz 不可达；AbortSignal.timeout 双保险。逻辑健全。

---

## 安全小结（凭据 / XSS / 信息泄漏）

1. **凭据存放（正面确认）**: `grep localStorage|sessionStorage|document.cookie` 于 apps/web/src **0 命中**——前端不落任何凭据，会话完全依赖服务端 httpOnly + SameSite=Strict cookie（05 正面确认 1），api() 统一 `credentials: "include"`（api.ts:15）。
2. **XSS（正面确认）**: 全 src 唯一 `{@html}` 在 Icon.svelte:125，PATHS 为手写 SVG 路径静态白名单（:123 注释自证 + 路由侧全部传静态字面量 IconName）；模型输出（agent.message preview）、工具 summary、Evidence、SSH comment/options/forcedCommand、错误信息全部经 Svelte 文本插值渲染（EventStream raw payload 走 `<pre>{JSON.stringify(...)}` 文本插值）——**无 {@html} 可达不可信数据的通路**。Icon 组件级结论归 7b 复核（保持 IconName 白名单封闭即可）。
3. **错误信息渲染**: 页面 error 均渲染 `ApiClientError.message` = 服务端 `error.message`（固定文案，05-F6："Unexpected server error"；ValidationError 固定串）——无内部信息（堆栈/路径/SQL）进入 UI。`Request failed (status)` 兜底（api.ts:23）。
4. **登录页**: `type="password"` + `autocomplete="current-password"` + `autocomplete="username"` 正确；密码仅存于组件 `$state`，无持久化；F7 记录 username 预填问题。

---

## 覆盖清单

| 文件 | 行数 | 状态 |
|---|---|---|
| apps/web/src/lib/contracts.ts | 223 | 已审（穷尽；全部类型回 Rust 逐字段比对，见文首核对表） |
| apps/web/src/routes/+layout.svelte | 86 | 已审（穷尽） |
| apps/web/src/routes/+page.svelte | 439 | 已审（穷尽） |
| apps/web/src/routes/login/+page.svelte | 170 | 已审（穷尽） |
| apps/web/src/routes/tasks/+page.svelte | 419 | 已审（穷尽） |
| apps/web/src/routes/tasks/new/+page.svelte | 167 | 已审（穷尽） |
| apps/web/src/routes/runs/[runId]/+page.svelte | 472 | 已审（穷尽；另点查 +page.ts loader 3 行确认 data.runId 来源） |
| apps/web/src/routes/settings/+layout.svelte | 93 | 已审（穷尽） |
| apps/web/src/routes/settings/security/+page.svelte | 363 | 已审（穷尽） |
| apps/web/src/routes/settings/notifications/+page.svelte | 255 | 已审（穷尽） |
| apps/web/src/lib/components/EventStream.svelte | 183 | 已审（穷尽；payload 键逐个对照 04a 键名表吻合；pageSize prop 可重赋值为反模式小注——父组件未传时安全） |
| apps/web/e2e/happy-path.spec.ts | 61 | 已审（穷尽） |
| apps/web/e2e/global-teardown.ts | 31 | 已审（穷尽） |
| test-results/×2/error-context.md | 126+ | 已读（e2e 定性权威证据） |
| 点查取证（发现归 7b） | — | lib/api.ts 33 行全文、Icon.svelte:100-126、PageHeader.svelte grep（h1:60）；06-hostd grep "给 webui" 0 命中（无专门 webui 契约段） |
| Rust 契约对照 | — | domain/run.rs 133 全文、runtime.rs:1-232、event.rs:1-73、requests.rs 105 全文、error.rs:1-40、security.rs 75 全文、server/http.rs:295-380（create/list/get handlers）、ssh_access.rs 定位 grep |

## EventStream payload 键 × 04a 键名表 核对

| 事件 | 04a 表列键 | EventStream 实际取用 | 结果 |
|---|---|---|---|
| run.created | taskId, status | 不取（固定文案） | ✅ 不越界 |
| run.status_changed | from, to, workerId, reason | to, reason | ✅ |
| agent.plan_updated | summary, steps | 不取（PlanPanel 另渲染） | ✅ 契约欠用（summary 不入流，P3 级小注） |
| agent.message | preview | preview | ✅ |
| tool.call_started | callId | callId（名称经 toolNames 映射，**正确地不向 payload 要工具名**——与 04a/05 结论一致） | ✅ |
| tool.call_completed | callId, summary, workspaceRevision | callId, summary（workspaceRevision 不取） | ✅ |
| tool.call_failed | callId, category, summary | callId, category, summary | ✅ |
| run.reported | outcome, summary | summary（outcome 不取，run 页 ReportCard 另渲染） | ✅ |
| — | — | :100-102 的 `resultSummary`/`preview` 回退键不在任何事件 payload 中（resultSummary 是 ToolCallSummary 字段）——防御性死代码，无运行时影响 | 小注 |

## 发现统计

P0 × 0；P1 × 0；**P2 × 5**（F1 TERMINAL 缺 blocked、F2 login 迟到 goto 劫持、F3 refreshRuntime 竞态、F4 decide 无 catch、F11 e2e spec 三重缺陷）；**P3 × 8**（F5 cancelRun/logout 无错误处理、F6 tasks 页无 heading、F7 username 预填、F8 refreshing 死标志、F9 each key 撞键、F10 lastError 不展示、F12 401 静默吞、F13 entries 无渲染上限）。合计 13 条，编号 F1-F13 连续无弃用。

**契约漂移清单（contracts.ts × 05 基线）**: 零字段漂移（文首核对表 23 项全绿）；唯二类型收窄注记（payload/approvalPreview 按 Record 收窄 serde_json::Value，informational）；TS 侧未镜像的 5 个请求体类型经页面逐一核对实际发送形状全部正确（ApprovalDecisionRequest {decision}✓、CreateTaskRequest {title,goal}✓、LoginRequest {username,password}✓、UpdateIpNotificationSettingsRequest {enabled,recipients}✓、GET 响应 {items}✓）；RunStatus 15 值含死状态 draft/paused 在前端逐字镜像（02-F4/05-F8 传播链在前端侧确认闭合）。

**验证命令**: `bun run --cwd apps/web check` → svelte-check 0 errors 0 warnings；`bun run lint`（根，eslint apps/web）→ 零输出全绿。与 00-baseline 静态链路绿一致——本节点全部发现均为运行时/逻辑层，无编译层问题。未重跑 e2e（需浏览器环境，按纪律豁免）；e2e 结论基于现存 test-results 快照 + 代码反推闭环。
