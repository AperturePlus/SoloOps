# 07b — WebUI 组件与质量审查（components / ui/ 库 / mock 全套 / time·utils / e2e 审计脚本）

- **审查对象**: 工作区状态（分支 `feature/agent-page-polish`）。lib/components/ 除 EventStream 外 10 个组件 + ui/ 6 组件全文件、utils.ts、time.ts、mock/ 7 文件全量、e2e/ui-audit.mjs、scripts/ui-lint.mjs、package.json、app.css、eslint.config.js、vite.config.ts
- **方法**: 全部范围内文件穷尽精读；e2e 断言文案以 grep 全 src 逐字核对（07a 移交项）；mock 保真度对照 04a 键名表/05 契约/06 preview 契约口径；`bun test --cwd apps/web src/mock/state.test.ts src/mock/scenario.test.ts` 实跑验证（10 pass / 0 fail）
- **前序事实**: 07a 已审 contracts.ts/9 路由页/EventStream/e2e spec，本节点不重复其发现（交叉问题注明 07a-xx）；00-baseline §1.5 的 12 处无扩展名导入警告在本节点正式收录（见 F12）；02-F4 死状态 draft/paused 已确认传播到前端契约（07a），组件侧仅做渲染口径核对
- **本节点纪律**: 只读未改任何源码/配置/测试；未重跑 e2e（按纪律豁免）

---

## [P2] F1：ToolCallCard 行内审批按钮无 busy/disabled 防护——决策在途期间可重复提交（与 05-F2→500、07a-F4 无 catch 叠加）
- 位置: apps/web/src/lib/components/ToolCallCard.svelte:7-13、133-145；apps/web/src/routes/runs/[runId]/+page.svelte:382-385
- 置信度: 高
- 证据:
```svelte
<!-- ToolCallCard.svelte:7-13 —— props 只有 call/onDecide，无 busy -->
  let {
    call,
    onDecide
  }: {
    call: ToolCallSummary;
    onDecide?: (call: ToolCallSummary, decision: "approve" | "deny") => void;
  } = $props();
<!-- ToolCallCard.svelte:133-134 —— 按钮无 disabled -->
          <div class="flex gap-2">
            <Button class="flex-1" size="sm" onclick={() => onDecide?.(call, "approve")}>
```
```svelte
<!-- 对照 ApprovalBanner.svelte:73、78（有防护） -->
            disabled={busy}
            onclick={() => onDecide(call, "deny")}
```
- 问题: 同一审批决策点存在两套按钮：ApprovalBanner（disabled={busy}，busy={deciding} 由 run 页 :337 传入）与 ToolCallCard 展开态内的 Approve/Deny。后者无任何禁用/防重机制——`deciding` 在途期间（含 07a-F3 的 refreshRuntime 等待窗口）用户可在卡片内再次点击，重复 POST decision。真实后端重复提交返回 500 catch-all（05-F2），而 decide() 无 catch（07a-F4）→ unhandled rejection、Owner 零反馈、无法判断是否已批准。ApprovalBanner 的防护形同虚设：它禁用的是横幅按钮，卡片按钮不受约束。
- 建议: 给 ToolCallCard 增加 `busy?: boolean` prop 并在 :134/:137-141 的 Button 上 `disabled={busy}`；或把 deciding 改为 per-callId 集合并同时传两处。根治须配合 07a-F4 补 catch。

## [P2] F2：e2e 审批段 3/4 断言文本在组件渲染中不存在——07a-F11 移交核对确证为"死断言"
- 位置: apps/web/e2e/happy-path.spec.ts:48-51；对照 ToolCallCard.svelte:96、123-125、152、ApprovalBanner.svelte:36
- 置信度: 高（grep 全 apps/web/src 逐字核对）
- 证据:
```ts
// happy-path.spec.ts:48-51
  await expect(page.getByText("privileged · waiting_for_approval")).toBeVisible();
  await expect(page.getByText("Approval preview")).toBeVisible();
  await expect(page.getByText("e2e.example.test")).toBeVisible();
  await expect(page.getByText(/arguments sha256:/)).toBeVisible();
```
```svelte
<!-- ToolCallCard.svelte:95-96 —— risk 与状态分开渲染，无 "·" 合成文本 -->
        {#if call.status === "waiting_for_approval"}
          <span class="text-amber-300/90">awaiting decision</span>
<!-- :124 —— 标签是 "Approval required" 不是 "Approval preview" -->
              Approval required
<!-- :152 —— sha256 前缀渲染，无 "arguments" 字样 -->
          sha256 {call.argumentsSha256}
```
- 问题: 07a-F11 假设 ":47 起断言依赖 ToolCallCard/ApprovalBanner 渲染文本，移交 7b 核对"。本节点确证：`privileged · waiting_for_approval`、`Approval preview`、`/arguments sha256:/` 三个断言在全部 src 中 0 命中（risk 徽章与 "awaiting decision" 是相邻独立元素；预览标题为 "Approval required"；sha256 行渲染为 `sha256 <hex>`）。即使按 07a 修复清单修好登录竞态 + 选择器消歧，spec 仍将死于 :48。:50 `e2e.example.test` 依赖 approvalPreview 的 JSON.stringify 渲染（ToolCallCard:126-131 / ApprovalBanner:61-66），该断言成立。
- 建议: spec 按现 UI 重写为 `getByText("awaiting decision")`、`getByText("Approval required")`、`getByText(/^sha256 /)`；或恢复旧文案。归并到 07a-F11 修复方案中一并处理。

## [P2] F3：time.ts formatDuration 秒数用 round 可产出 "1m 60s"
- 位置: apps/web/src/lib/time.ts:8-10
- 置信度: 高
- 证据:
```ts
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = Math.round(totalSeconds % 60);
  if (minutes < 60) return `${minutes}m ${seconds}s`;
```
- 问题: `ms = 119_500` → totalSeconds=119.5 → minutes=1，seconds=Math.round(59.5)=60 → 渲染 "1m 60s"。凡秒数小数部分 ≥ .5 的整分边界附近（59.5s、119.5s、…）都触发。ToolCallCard/ReportCard 的时长展示直接消费该函数。另注：`<60s` 分支 `totalSeconds.toFixed(<10?1:0)` 与 mock 事件 elapsedMs=1500/3000 生成值无碰撞，但 59.96s 会渲染 "60.0s"（同族边界）。
- 建议: 秒改 `Math.floor`（或先把 ms 整体 round 再拆分），保持"截断"语义一致。

## [P3] F4：StatusBadge TERMINAL 集缺 "blocked"——07a-F1 的组件侧第 4 处实例，终态徽章永久 pulse
- 位置: apps/web/src/lib/components/StatusBadge.svelte:9、54、69
- 置信度: 高
- 证据:
```ts
  const TERMINAL: ReadonlySet<RunStatus> = new Set(["succeeded", "failed", "cancelled"]);
  ...
  const live = $derived(!TERMINAL.has(status));
<!-- :69 -->
    <span class={`size-1.5 rounded-full bg-current ${live ? "animate-pulse" : "opacity-80"}`}
```
- 问题: 与 07a-F1 同根（domain `is_terminal` 含 Blocked）：blocked 在此组件被当活跃态 → 徽标圆点无限 pulse，向 Owner 暗示"仍在推进"。影响面仅动画级（该组件不驱动逻辑），故 P3；但说明"手写终端集合"已扩散到第 4 处，07a-F1 建议的"contracts.ts 导出单一 TERMINAL_STATUSES"应把这个组件一并纳入。
- 建议: 同 07a-F1——单一常量源，四处（3 页面 + 本组件）统一引用。

## [P3] F5：ReportCard 表头恒渲染绿色对勾与成功色边框——failed/partial 报告也呈"成功"视觉
- 位置: apps/web/src/lib/components/ReportCard.svelte:17、19、23-25
- 置信度: 高
- 证据:
```svelte
<section class="panel border-mint-400/20 bg-mint-400/[0.03] p-4">
  <div class="flex flex-wrap items-center gap-3">
    <span class="text-mint-400"><Icon name="check" size={14} /></span>
    ...
    <span class={`rounded-full border px-2 py-0.5 text-2xs font-medium capitalize ${outcomeTone}`}>
      {report.outcome}
```
- 问题: outcome 徽章本身按 `OUTCOME_TONES[report.outcome] ?? partial` 正确着色（对自由字符串 outcome 有兜底，符合 06 契约口径，正面），但整个面板的边框/底色（:17 mint 绿系）和表头图标（:19 硬编码 check）不随 outcome 变化——failed 报告显示"红徽章 + 绿对勾 + 绿色面板"的矛盾视觉。审批失败（deny→failed）后的终报尤其误导。
- 建议: 图标与面板色随 outcome 映射（failed→x/triangle、partial→circle），复用 OUTCOME_TONES 结构。

## [P3] F6：mock 会话保真度缺口——logout 不失效 cookie、WS 升级零认证、登录无凭据校验、Set-Cookie 缺 SameSite
- 位置: apps/web/src/mock/router.ts:43-46、83-88、92、95-99；apps/web/src/mock/index.ts:51-67
- 置信度: 高
- 证据:
```ts
// router.ts:43-46 —— 会话判定只是 cookie 前缀正则
function hasSession(req: IncomingMessage): boolean {
  const cookie = req.headers.cookie ?? "";
  return /soloops_session=mock-/.test(cookie);
}
// router.ts:92 —— logout 后 owner 已 null，但持旧 cookie 仍能拿到兜底 owner
        sendJson(res, 200, { owner: deps.state.owner ?? { id: "owner", username: "owner" } });
// index.ts:51-59 —— WS 升级不检查任何会话
      httpServer.on("upgrade", (req, socket, head) => {
        const url = req.url ?? "";
        if (!url.startsWith("/api/events")) {
```
- 问题: 四处叠加使 mock 模式完全无法演练真实认证失效流：① POST /api/auth/logout 只清 `state.owner`，浏览器 cookie 仍匹配 `mock-` 前缀 → 之后所有 API 依旧 200，且 GET /api/auth/session 因 :92 兜底返回 owner——真实后端 logout 后应 401（服务端会话删除）；② WS 升级（真实 API 是"认证 WebSocket 时间线"）无会话检查 → 前端 WS 401/拒绝重连路径 mock 不可测；③ login 接受任意用户名/密码 → 登录失败分支（login 页 catch 渲染错误）mock 不可达；④ mock Set-Cookie 无 SameSite（真实为 Strict，07a 安全小结 1），dev 下回落 Lax。
- 建议: logout 响应 Set-Cookie 清除（`Max-Age=0`）；WS upgrade 复用 hasSession；login 对空/错误凭据返回 401（可用固定口令）；Set-Cookie 补 `SameSite=Strict` 对齐真实。

## [P2] F7：agent-disclose 折叠态不切断子元素可聚焦性——ToolCallCard 折叠后 Approve/Deny 仍可 Tab 聚焦并盲触发
- 位置: apps/web/src/app.css:407-417；apps/web/src/lib/components/ToolCallCard.svelte:114、121-146、23-25
- 置信度: 高（机制确证；触发需要键盘导航 + 手动折叠，频率低但危害直达审批控制点）
- 证据:
```css
/* app.css:407-417 —— 折叠仅靠 grid 0fr + overflow hidden，无 visibility/inert */
.agent-disclose {
  display: grid;
  grid-template-rows: 0fr;
  transition: grid-template-rows 0.3s cubic-bezier(0.22, 1, 0.36, 1);
}
.agent-disclose[data-open="true"] {
  grid-template-rows: 1fr;
}
.agent-disclose > div {
  overflow: hidden;
}
```
```svelte
<!-- ToolCallCard.svelte:114 —— data-open 仅驱动 CSS，无 aria-hidden/inert -->
  <div class="agent-disclose" data-open={open} id={`call-body-${call.callId}`}>
```
- 问题: awaiting 且带 approvalPreview 的卡片自动展开（:23-25），但用户可再点表头折叠（:57 open=!open）。折叠后 Approve/Deny 按钮（:134-144）仍在 DOM 且可聚焦——grid 0fr + overflow:hidden 只是视觉裁剪，按钮未从无障碍树/Tab 序列移除。键盘用户 Tab 会落到不可见按钮上（焦点视觉消失，容器不可滚动），按 Enter 即"盲批准/盲拒绝"——产品核心控制点出现不可见激活路径。aria-expanded 有（:56）但 body 无 aria-hidden/inert 管理。
- 建议: 折叠态给 body 加 `visibility: hidden`（可配合 transition）或 `inert` 属性；Svelte 里 `inert={!open}` 一行即可（现代浏览器支持）。

## [P3] F8：mock 重复 decide 宽容重放 + cancel 无终态守卫——掩盖 05-F2 的 500 与 domain 状态机拒绝
- 位置: apps/web/src/mock/scenario.ts:250-274、276-292、294-308
- 置信度: 高
- 证据:
```ts
// scenario.ts:250-258 —— approveToolCall 不检查 call 当前状态
export function approveToolCall(deps: ScenarioDeps, runId: string, callId: string): void {
  const handle = ensureHandle(deps, runId);
  if (handle.finished) return;
  updateToolCall(deps.state, runId, callId, {
    status: "completed",
// scenario.ts:294-298 —— cancelScenario 对已 finished 的 run 仍改写状态
export function cancelScenario(deps: ScenarioDeps, runId: string): void {
  const handle = ensureHandle(deps, runId);
  if (handle.timer) clearTimeout(handle.timer);
  handle.finished = true;
  setRunStatus(deps.state, runId, "cancelled", "Cancelled by owner");
```
- 问题: ① 对同一 call 重复 approve/deny，mock 不报错：第二次 approve 把（可能已是 verifying 阶段的）run 拉回 running、重放 tool.call_completed 与后续 verify→report→succeeded 步骤，最终"看起来正常"；而真实后端重复提交返回 500 catch-all（05-F2）——**本节点 F1 的双击缺陷在 mock 模式下永远暴露不出来**。② cancel 对已终态（succeeded/failed）run 直接改写为 cancelled——真实后端 domain `can_transition_to` 会拒绝非法转换（02 章：守卫唯一强制点在 storage transition_run_on_connection），mock 掩盖"终态不可取消"约束。③ 附带：cancel 事件 payload `from` 硬编码 "planning"（:305），与实际前一状态无关。
- 建议: approve/deny 先查 `call.status === "waiting_for_approval"` 否则 409/400；cancel 先查 run 终态否则 409；`from` 取 run.status 快照。

## [P3] F9：mock 事件 payload 键名与 04a 键名表不符（4/8 事件类型）——mock 模式下 EventStream 摘要渲染与真实后端漂移
- 位置: apps/web/src/mock/scenario.ts:134、160、220、269；apps/web/src/mock/state.ts:217
- 置信度: 高（04a 键名表经 07a 表格复核；mock payload 直接回读）
- 证据:
```ts
// scenario.ts:134 —— agent.plan_updated 用 {plan} 包裹；04a 键：summary, steps
          type: "agent.plan_updated",
          payload: { plan: runtime.plan }
// scenario.ts:269 —— tool.call_completed 键为 resultSummary；04a 键：callId, summary, workspaceRevision
      type: "tool.call_completed",
      payload: { callId, resultSummary: "Applied the change to the workspace." }
// scenario.ts:220 —— run.reported 用 {report} 包裹；04a 键：outcome, summary
          type: "run.reported",
          payload: { report: runtime.report }
// state.ts:217 —— run.created 为 {taskId, runId}；04a 键：taskId, status
    type: "run.created",
    payload: { taskId, runId }
```
- 问题: EventStream 按真实契约取 `payload.summary`（07a 表：tool.call_completed 取 callId+summary、run.reported 取 summary）→ mock 模式下这两类事件行的摘要恒为空，与真实后端渲染不一致；基于 mock 截图/ui-audit 产物做的 UI 评审会低估事件行信息量。另外 mock 从不产生 `agent.message`（EventStream preview 行）、`tool.call_failed`（category/summary 行）、`tool.call_started` 的 name 键也不在 04a 表（工具名应来自 toolNames 映射）。scenario 的 5 档 risk 也只演示 workspace_write 一种（privileged 红档、network、process 从未出现）。
- 建议: mock payload 按 04a 键名表逐字对齐（plan_updated→{summary,steps}、call_completed→{callId,summary,workspaceRevision}、reported→{outcome,summary}）；场景增加 message/failed 分支。

## [P3] F10：mock ScenarioHandle.timer 从不赋值——deny/cancel 的 clearTimeout 是死代码，取消安全仅靠 finished 标志
- 位置: apps/web/src/mock/scenario.ts:15、23-26、281、296
- 置信度: 高
- 证据:
```ts
// state.ts:15 —— 字段声明
  timer: ReturnType<typeof setTimeout> | undefined;
// scenario.ts:23-26 —— next() 不回写 handle.timer
function next(deps: ScenarioDeps, runId: string, fn: () => void): void {
  const schedule = deps.schedule ?? ((f, ms) => setTimeout(f, ms));
  schedule(fn, deps.stepDelayMs ?? DEFAULT_STEP_MS);
}
// scenario.ts:281、296 —— clearTimeout(undefined)
  if (handle.timer) clearTimeout(handle.timer);
```
- 问题: `handle.timer` 全库零写入点，两处 `clearTimeout(handle.timer)` 永不执行。cancel 后挂起的步进定时器仍会触发——只是 advance 开头的 `if (handle.finished) return`（:107）兜底使其成为 no-op，行为侥幸正确。字段意图（取消挂起步进）从未实现，属"看似有防护实则没有"的误导性死代码；未来若有人在 finished 检查之前插入副作用即成真缺陷。
- 建议: next() 里 `handle.timer = schedule(...)`，clearTimeout 真正生效；或删掉 timer 字段、明确注释依赖 finished 标志。

## [P3] F11：mock POST /api/runs/:id/cancel 对不存在的 run 返回 500 而非 404——cancelScenario 在存在性检查之前抛出
- 位置: apps/web/src/mock/router.ts:195-201；scenario.ts:28-32；index.ts:33-42
- 置信度: 高
- 证据:
```ts
// router.ts:195-199 —— cancelScenario 先于 getRun 的 404 检查执行
      if (method === "POST" && segments.length === 4 && segments[3] === "cancel") {
        cancelScenario(deps.scenario, runId);          // ← 未知 runId 时 ensureHandle 直接 throw
        const run = getRun(deps.state, runId);
        if (!run) return (sendError(res, 404, "not_found", "Run not found"), true);
```
- 问题: `cancelScenario→ensureHandle` 对 scenarios 表中不存在的 runId 抛 `scenario not found`，被 index.ts 的 catch-all 接住变成 500 `{code:"internal"}`；而 :198 本来准备了正确的 404 分支却永远到不了。真实后端对不存在 run 的 cancel 是 404 not_found。前端 cancelRun（07a-F5：无错误处理）在 mock 下会展示 500 而非 404，误导排障。
- 建议: 先 `getRun` 判存在，再调 cancelScenario；或 cancelScenario 改为对缺失 handle 抛专用错误并由 router 映射 404。

## [P3] F12：src/mock 12 处无扩展名导入触发 Vite native configLoader 警告——正式收录（基线 00-baseline.md §1.5 / :154 来源）
- 位置: apps/web/vite.config.ts:4；apps/web/src/mock/index.ts:4-7；apps/web/src/mock/router.ts:2-5；apps/web/src/mock/scenario.ts:2-4
- 置信度: 高（逐文件清点 = 12，与基线计数吻合）
- 证据:
```ts
// vite.config.ts:4（目录索引导入，1 处）
import { mockPlugin } from "./src/mock";
// src/mock/index.ts:4-7（4 处）
import { createState, listEvents } from "./state";
import { createClients } from "./clients";
import type { ScenarioDeps } from "./scenario";
import { createMockRouter } from "./router";
// src/mock/router.ts:2-5（4 处：./state ×2、./scenario ×2）
// src/mock/scenario.ts:2-4（3 处：./state ×2、./clients ×1）
```
- 问题: Vite `configLoader: 'native'` 对目录索引导入与无扩展名相对导入各发一条提示，共 12 条（基线已记录但未定级）。native loader 是 Vite 的未来默认方向，一旦切换，目录索引（`./src/mock`）与省略 `.ts` 后缀的解析行为可能变化，构建面临破坏风险。当前 svelte-check 0 errors 0 warnings（00-baseline），属前瞻性兼容风险而非现行缺陷。
- 建议: 统一补 `.ts`/`/index.ts` 扩展名（12 处机械改动），消除两类提示；可在 check 脚本中留意 Vite 升级公告。

## [P3] F13：mock setRunStatus 语义漂移——queued 即写 startedAt、终态数组缺 blocked（07a-F1 的 mock 侧同源，blocked 状态在前端全链路不可见）
- 位置: apps/web/src/mock/state.ts:250-255
- 置信度: 高
- 证据:
```ts
  if (status === "queued" || status === "planning" || status === "running" || status === "leased") {
    if (run.startedAt === null) run.startedAt = nowMs();
  }
  if (["succeeded", "failed", "cancelled"].includes(status)) {
    run.finishedAt = nowMs();
  }
```
- 问题: ① 真实后端 run 插入即 'queued' 且 started_at 为 NULL（02-F4 证据 tasks.rs:29；07a 核对 RunDetail.startedAt 可空），startedAt 语义是"实际启动"；mock 在 queued 就写 startedAt → 任何基于 startedAt 的时长/排序展示在 mock 与真实后端下语义不同。② finishedAt 终态数组缺 blocked——domain `is_terminal` 含 Blocked（run.rs:65-70）；且 scenario.ts 全程不产出 blocked（grep 确认仅 waiting_for_approval/failed/succeeded/cancelled）。叠加 07a-F1（3 页面）与本节点 F4（StatusBadge）：blocked 在前端页面、组件、mock 数据三层全部缺席，即"blocked 显示错误"这类缺陷在 mock 模式与现存测试下不可能被发现。
- 建议: mock 补 blocked 分支（如 deny 前 30s 变 blocked 或加调试场景）；startedAt 只在 planning/running/leased 写入；终态数组与 domain is_terminal 对齐。

## [P3] F14：mock 请求体校验与错误码漂移——任意 title/goal 得 201、坏 JSON 得 500、after=NaN 静默空回放
- 位置: apps/web/src/mock/router.ts:83-88、111、161-166、228；apps/web/src/mock/index.ts:58
- 置信度: 高
- 证据:
```ts
// router.ts:161-166 —— title/goal 无任何校验（真实：title≤160/goal≤20000，越界 422）
      if (method === "POST" && segments.length === 2) {
        const body = JSON.parse((await readBody(req)) || "{}");
        const task: TaskSummary = createTask(
          deps.state,
          String(body.title ?? ""),
          String(body.goal ?? "")
// router.ts:111 —— JSON.parse 无 try/catch；坏 JSON 逃逸到 index.ts catch-all 变 500 internal
        const body = JSON.parse((await readBody(req)) || "{}");
// index.ts:58 —— after 参数无 NaN 防护
        const after = Number(parsed.searchParams.get("after") ?? "0");
```
- 问题: 真实 API 对请求体有 serde 校验（422 validation_error，07a 契约缺口核对引用 requests.rs），mock 全部宽容：空标题建任务 201、超长字段照单全收、坏 JSON 500（真实是 400/422）、WS/REST 的 `after=abc` 得 NaN → `e.sequence > NaN` 恒 false → 静默返回空回放（真实应 400）。前端 422/400 错误分支与"回放不丢事件"的契约行为在 mock 下不可验证。另：PUT ip-notifications 的 recipients 做 lowercase+dedupe（:113-115），真实后端是否同样归一化未经前序章节确认——若不一致，notifications 页在两种模式下显示的邮箱大小写会不同（置信度:中，标注存疑）。
- 建议: mock router 补最小校验镜像（空标题→422、越界→422、坏 JSON→400、after NaN→400）；recipients 归一化行为与 server 实现核对后对齐。

## [P3] F15：`text-muted-foreground/65` + 11px `text-2xs` 组合对比度约 3.7:1——低于 WCAG AA 的 4.5:1
- 位置: apps/web/src/app.css:7、113；消费示例 ToolCallCard.svelte:86、96、102、149
- 置信度: 高（按 WCAG 相对亮度公式计算；实际值随底色微差 ±0.3）
- 证据:
```css
--text-2xs: 0.6875rem; /* 11px micro labels, badges, timestamps */
--muted-foreground: #94a3b8;
```
```svelte
<!-- ToolCallCard.svelte:86 —— 11px + 65% 透明度的组合遍布 meta 信息 -->
      <span class="mt-0.5 flex flex-wrap items-center gap-1.5 text-2xs text-muted-foreground/65">
```
- 问题: #94a3b8 以 65% 不透明度叠加在面板底色（≈#10151d，.panel = rgb(16 21 29/.62) 落在 #080b10 上）的混合前景 ≈ rgb(102,113,130)，相对亮度 0.163，对比度 = (0.163+0.05)/(0.0073+0.05) ≈ **3.7:1**；11px 属小字号，WCAG AA 要求 4.5:1。该组合是全应用的 meta 文本标配（时间戳、sha256、risk 徽章、EventStream 副文本），低视力用户系统性受损。full-opacity 的 text-muted-foreground（7.1:1）达标，问题仅在 /65 修饰符。
- 建议: meta 文本至少提至 /80（≈5.0:1）或将 --muted-foreground 调亮一档；保留 /65 仅用于装饰性元素。`--radius`、prefers-reduced-motion（app.css:442-450）等其他 a11y 项无问题。

## [P3] F16：ui-lint.mjs 三处自身缺陷——IGNORE 前缀匹配无路径边界、"never edited" 声明与实际修改矛盾、灰阶规则漏 stone/渐变
- 位置: apps/web/scripts/ui-lint.mjs:10-11、16、50；apps/web/src/lib/components/ui/separator/separator.svelte:18
- 置信度: 高
- 证据:
```js
// ui-lint.mjs:10-11
// shadcn-svelte generated primitives are vendor code — never edited, never linted.
const IGNORE = [join(SRC_ROOT, "lib", "components", "ui")];
// ui-lint.mjs:50 —— startsWith 无分隔符边界
      if (IGNORE.some((path) => full === path || full.startsWith(path))) continue;
// ui-lint.mjs:16 —— 灰阶规则不含 stone，也无 from-/to-/via- 渐变前缀
    test: /(?:text|bg|border|divide|decoration)-(?:slate|zinc|gray|neutral)-\d+/,
```
```svelte
<!-- separator.svelte:18 —— 自述"与 shadcn 不同，我们改了" -->
    // this is different in shadcn/ui but self-stretch breaks things for us
```
- 问题: ① `full.startsWith(path)`：`components/ui-*`（如未来出现的 `ui-form`）目录会因前缀命中被整目录跳过 lint——静默扩大豁免面；② :10 注释断言 ui/ "never edited"，但 separator.svelte:18 明确自述修改过、switch.svelte 也加了 size prop 与自定义 thumb 位移——注释失实会误导后续维护者以为该目录可随 shadcn 升级直接覆盖（覆盖会冲掉本地修改）；③ 灰阶规则缺 tailwind 第 5 个灰系 stone 与渐变工具类前缀。当前 0 violation，无现行损害。
- 建议: startsWith 加 `path + sep` 边界；注释改为"含本地定制，升级勿直接覆盖"；规则补 stone 与 from-/to-/via-。

## [P3] F17：RISK_TONES 风险配色映射在 ApprovalBanner 与 ToolCallCard 各持一份相同拷贝
- 位置: apps/web/src/lib/components/ApprovalBanner.svelte:16-22；apps/web/src/lib/components/ToolCallCard.svelte:37-43
- 置信度: 高
- 证据:
```ts
// ApprovalBanner.svelte:16-22
  const RISK_TONES: Record<ToolCallSummary["risk"], string> = {
    read_only: "border-edge bg-ink-800/70 text-muted-foreground",
    workspace_write: "border-sky-400/25 bg-sky-400/10 text-sky-300",
    ...
// ToolCallCard.svelte:37-43 —— 同一字面量第二份
  const RISK_TONES: Record<ToolCallSummary["risk"], string> = {
    read_only: "border-edge bg-ink-800/70 text-muted-foreground",
```
- 问题: 5 档 risk × 4 class 的映射逐字重复。审查要点 5（"风险档即视觉契约"）下，两处漂移会造成横幅与卡片同一 call 颜色不一致；toolNames/STYLES 类映射在 StatusBadge 已是单一入口（其注释自称 single entry point），risk 配色应同等对待。
- 建议: 提取 `lib/components/risk.ts` 导出 RISK_TONES，两处引用。

## [P3] F18：switch.svelte 同一 Thumb 混用 data-checked 与 data-[state=checked] 两套选择器——其一必为死样式（bits-ui 封装一致性）
- 位置: apps/web/src/lib/components/ui/switch/switch.svelte:22、29
- 置信度: 中（按纪律未读 bits-ui 源码；但 bits-ui v2（package.json ^2.19.0）的 Switch 使用 data-checked/data-unchecked，同文件 :22 正是该约定，:29 的 data-[state=checked] 是 radix/shadcn(React) data-state 惯例残留，两者不可能同时生效）
- 证据:
```svelte
<!-- :22 —— Root 用 data-checked/data-unchecked（bits-ui v2 约定） -->
    "... data-checked:bg-primary data-unchecked:bg-input ..."
<!-- :29 —— Thumb 混入 data-state=checked（radix 惯例），RTL 分支永死 -->
    class="... rtl:data-[state=checked]:translate-x-[calc(-100%)]"
```
- 问题: RTL 布局下选中态 thumb 的反向位移永远不生效（RTL 用户看到 thumb 移动方向相反）。功能影响限于 dir=rtl 场景，当前应用未声明 RTL，属封装一致性/死样式问题。
- 建议: 改为 `rtl:data-checked:...`（与 :22 同约定）或删除；顺带核对 bits-ui 升级时的 data-* 命名。

## [P3] F19：前端测试覆盖缺口——mock 测试不进根 `test`、router 层/组件零测试、关键流仅靠 2/2 全红的 e2e
- 位置: package.json:21-22（根）；apps/web/src/mock/router.ts（236 行零测试）；state.test.ts + scenario.test.ts 全 11 用例
- 置信度: 高
- 证据:
```json
// 根 package.json:21-22 —— `test` 只跑 Rust，mock 测试须显式 test:mock
    "test": "cargo test --workspace",
    "test:mock": "bun test --cwd apps/web src/mock/state.test.ts src/mock/scenario.test.ts",
```
- 问题: 审查要点要求核对"登录、建任务、run 时间线、审批操作"四大关键流的覆盖：① 登录——无任何测试（仅 e2e，红）；② 建任务/审批 REST——mock/router.ts 的 401 拦截（:102）、404（:174/185/191）、400（:216）、decision 全链路零测试；③ run 时间线——WS 回放/sequence 游标仅 state 层测（state.test.ts:42-51），index.ts 升级流程零测试；④ 审批操作——仅 scenario.test.ts 的状态层断言（decide→succeeded/failed），F1 的双击路径、F8 的重复 decide 语义均无测试固化。唯一的集成防线 e2e 2/2 失败（07a-F11）。10 个绿单测全部位于 mock 内部层，无一覆盖前端组件/真实契约边界。
- 建议: 把 test:mock 并入根 `test`（或 CI 显式双跑）；为 mock router 补 401/404/decision 的最小 happy-path 测试（mock 模式下 30 行可覆盖）；e2e 修复后作为四大流的集成防线。

## [P3] F20：工具链大面积 "latest" 浮动版本——svelte/vite/kit/tailwindcss/eslint 全不锁版本
- 位置: apps/web/package.json:16-17、22-23、26-28；根 package.json:30-35
- 置信度: 高
- 证据:
```json
// apps/web/package.json
    "@sveltejs/kit": "latest",
    "svelte": "latest",
    "tailwindcss": "latest",
    "vite": "latest",
// 根 package.json
    "eslint": "latest",
    "prettier": "latest",
```
- 问题: bun.lock 当前钉住了具体版本，但 package.json 声明 "latest" 意味着任何 lock 重建（依赖新增、CI 缓存失效、`bun update`）都会静默跳到最新大版本——Svelte 5 runes 语法、Vite（叠加 F12 的 native loader 风险）、tailwind v4 token 行为都可能在无提示下变化。对一个以"纯静态 + 手写 CSS token 体系"为卖点的应用，工具链大版本漂移是最高频的破坏源。
- 建议: 全部改为 `^` 区间（与 @playwright/test ^1.55.0、typescript 5.9.3 的既有做法统一）。

## [P3] F21：ui-audit.mjs 自身三处瑕疵——注释指错文件、"form button 最后一个"脆弱选择器、approve 静默跳过
- 位置: apps/web/e2e/ui-audit.mjs:18、42、48-49
- 置信度: 高
- 证据:
```js
// :18 —— 登录宽容逻辑在 router.ts:83-88，不在 scenario.ts
  // Login first (mock mode accepts any credentials per scenario.ts).
// :42 —— 依赖表单内按钮顺序
  await page.locator("form button").last().click();
// :48-49 —— 审批横幅未出现时静默跳过，后续截图相位失真无提示
  const approve = page.getByRole("button", { name: "Approve", exact: true });
  if (await approve.count()) await approve.click();
```
- 问题: ① 注释指向错误文件，排障时误导；② `form button` last() 在 tasks/new 表单新增按钮（如"清空"）时点错元素——audit 产物静默错拍；③ mock 步进 1500ms 与硬编码 waitForTimeout（:44 1800/:46 4000/:50 7000）强耦合，时序漂移时 `approve.count()` 为 0 直接跳过，`run-final.png` 可能拍到 waiting 相位却无任何警告——审计截图不可信且不易察觉。
- 建议: 注释改指 router.ts；按钮加 data-testid 或 role 精确选择器；approve 跳过时 console.warn 打印。

---

## 正面确认（无发现项）

1. **Icon.svelte `{@html}` 组件级复核（07a 交移项 ②）——安全**。`name` 是封闭联合类型 IconName（:4-45），PATHS 为 `Record<IconName, string>` 手写 SVG 静态白名单（:55-108），全部调用点传静态字面量（07a 已核）；即使运行时 name 非法（TS 被绕过），`PATHS[name]` 为 undefined → `{@html undefined}` 渲染空。:123 注释自证 + :124 eslint-disable 有据。无可信数据注入通路。
2. **cn() 标准用法**（utils.ts:4-6）：twMerge(clsx(inputs)) 惯例封装；WithElementRef/WithoutChildren 类型族在 ui/ 各组件间使用一致（button/badge/input/textarea/switch 同构）。
3. **ToolCallCard 定时器清理正确**（:27-31）：$effect 内 setInterval，返回 clearInterval 清理函数，live 状态切换时正确重建/拆除——Svelte 5 runes 用法无混用（全组件群均为 $props/$state/$derived/$effect，无 legacy store/export let 残留）。
4. **WorkingIndicator**：CHECKPOINT_LABELS 为 Record<RuntimeCheckpoint,string> 8 值穷尽映射；role="status" + aria-live="polite" 语义恰当；装饰性圆点 aria-hidden。
5. **ReportCard 对自由字符串 outcome 的兜底**（:14 `?? OUTCOME_TONES.partial`）符合 06 章"status 勿枚举穷举"口径（表头图标问题另见 F5）；approvalPreview 经 JSON.stringify 文本插值渲染（ApprovalBanner:61-66 / ToolCallCard:126-131），无 {@html}，projectId/services/images/ports/volumes/site/routes/healthPath/composeSha256/caddySha256 全量字段自然呈现、未知字段不丢失——对 managed 契约零假设，正确。
6. **StatusBadge** STYLES/LABELS 为 Record<RunStatus,string> 15 值穷尽（含 draft/paused 死状态渲染兜底，符合 02-F4"不做功能假设"口径）；compact 模式有 title 全称提示。
7. **ApprovalBanner 无 preview 的普通审批也渲染按钮**（each 全量 calls，:46-84）——ToolCallCard 行内按钮仅在有 preview 时出现（:121），两路径互补，无"无 preview 审批无入口"的死角（run 页 :333 `awaitingCalls.length` 条件全量传入）。
8. **app.css prefers-reduced-motion 全局处理**（:442-450）正确；agent-disclose 的 grid-rows 动画方案现代且流畅（可聚焦性问题见 F7，属使用方式而非 CSS 本身）。
9. **BudgetPanel**：meter 以 label 为 key（唯一）、percent Math.min(100,…) 封顶、进度条 aria-hidden + 数值入 dd（读屏可达）。
10. **PageHeader** crumbs 索引 key 为无害小注（每路由静态数组，无重排）；back 链接有 aria-label。
11. **mock clients.ts** 广播/清理逻辑简洁正确（remove 遍历清理全部 run 桶）。
12. **bun test 实跑验证**：`bun test --cwd apps/web src/mock/state.test.ts src/mock/scenario.test.ts` → 10 pass / 0 fail / 30 expect。

## 覆盖清单

| 文件 | 行数 | 状态 |
|---|---|---|
| lib/components/ApprovalBanner.svelte | 86 | 已审（穷尽；F1 对照/F17） |
| lib/components/BudgetPanel.svelte | 73 | 已审（穷尽） |
| lib/components/EvidenceList.svelte | 40 | 已审（穷尽） |
| lib/components/Icon.svelte | 126 | 已审（穷尽；正面确认 1） |
| lib/components/PageHeader.svelte | 73 | 已审（穷尽；正面确认 10） |
| lib/components/PlanPanel.svelte | 102 | 已审（穷尽） |
| lib/components/ReportCard.svelte | 97 | 已审（穷尽；F5） |
| lib/components/StatusBadge.svelte | 73 | 已审（穷尽；F4） |
| lib/components/ToolCallCard.svelte | 157 | 已审（穷尽；F1/F2/F7） |
| lib/components/WorkingIndicator.svelte | 36 | 已审（穷尽；正面确认 4） |
| lib/components/ui/button/{button.svelte,index.ts} | 89+17 | 已审（穷尽） |
| lib/components/ui/badge/{badge.svelte,index.ts} | 50+2 | 已审（穷尽） |
| lib/components/ui/input/{input.svelte,index.ts} | 48+7 | 已审（穷尽） |
| lib/components/ui/separator/{separator.svelte,index.ts} | 23+7 | 已审（穷尽；F16 证据） |
| lib/components/ui/switch/{switch.svelte,index.ts} | 31+7 | 已审（穷尽；F18） |
| lib/components/ui/textarea/{textarea.svelte,index.ts} | 22+7 | 已审（穷尽） |
| lib/utils.ts | 11 | 已审（穷尽；正面确认 2） |
| lib/time.ts | 29 | 已审（穷尽；F3；relativeTime/formatDateTime 边界无问题） |
| mock/index.ts | 71 | 已审（穷尽；F6②/F11/F14） |
| mock/clients.ts | 38 | 已审（穷尽；正面确认 11） |
| mock/router.ts | 236 | 已审（穷尽；F6/F11/F14） |
| mock/scenario.ts | 308 | 已审（穷尽；F8/F9/F10） |
| mock/state.ts | 295 | 已审（穷尽；F9/F13） |
| mock/state.test.ts | 61 | 已审（穷尽；F19） |
| mock/scenario.test.ts | 101 | 已审（穷尽；F19；fakeTimers.flush 语义与 mock 步进吻合） |
| e2e/ui-audit.mjs | 58 | 已审（穷尽；F21） |
| scripts/ui-lint.mjs | 79 | 已审（穷尽；F16） |
| apps/web/package.json | 31 | 已审（穷尽；F20） |
| src/app.css | 450 | 已审（"快查"要求，实际 450 行全读；F7 证据/F15） |
| eslint.config.js（根） | 40 | 已审（穷尽；ignores/规则无问题，svelte/no-at-html 保持开启） |
| vite.config.ts | 25 | 范围外附带已审（F12 计数来源；proxy/mock 门控逻辑正确） |
| package.json（根） | 39 | 范围外附带已审（F19 证据；check 链入 ui-lint 与 ui-lint.mjs:3 声明吻合） |
| routes/runs/[runId]/+page.svelte | 点查 :325-395 | 点查（ApprovalBanner/ToolCallCard 接线事实；页面本体 07a 已审不重复） |
| e2e/happy-path.spec.ts | 点查 :36-61 | 点查（F2 断言原文；本体 07a 已审不重复） |
| lib/components/EventStream.svelte | — | 跳过（07a 已穷尽审，按节点分工不重复） |

## 发现统计

P0 × 0；P1 × 0；**P2 × 3**（F1 ToolCallCard 审批按钮无 busy 防重、F2 e2e 审批段 3/4 断言文本已死、F7 agent-disclose 折叠态可聚焦隐藏审批按钮）；**P3 × 18**（F4 StatusBadge 缺 blocked、F5 ReportCard 成功色硬编码、F6 mock 会话保真度、F8 mock 宽容 decide/cancel、F9 mock 事件键名漂移、F10 timer 死代码、F11 mock cancel 500、F12 无扩展名导入、F13 setRunStatus 漂移、F14 mock 校验缺失、F15 对比度 3.7:1、F16 ui-lint 自身、F17 RISK_TONES 重复、F18 switch 死样式、F19 测试覆盖缺口、F20 latest 浮动版本、F21 ui-audit 自身）。合计 21 条，编号 F1-F21 连续无弃用。

**与 07a 的交叉问题（供报告节点汇总）**：① F1+F2 与 07a-F4/05-F2 同链（审批双击→500→无反馈），修复必须三处联动（ToolCallCard busy + decide catch + 后端幂等）；② F4 与 07a-F1 同根（TERMINAL 缺 blocked，共 4 处）；③ F2 确证 07a-F11 的断言漂移不止 heading/Phase/Run timeline 三处，审批段再加 3 处死断言——spec 需整体重写；④ F13 与 02-F4/07a-F1 构成 blocked 三层盲区（契约有、语义有、前端+mock+测试全不可见）。

**验证命令**: `bun test --cwd apps/web src/mock/state.test.ts src/mock/scenario.test.ts` → 10 pass / 0 fail（实跑）。未重跑 e2e 与 ui-audit（需浏览器环境，按纪律豁免）；组件结论全部基于源码静态精读。

**本节点零源码修改。**
