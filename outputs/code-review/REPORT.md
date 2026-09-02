# SoloOps 全代码库深度审查 — 最终报告（REPORT）

- **报告节点**: 第 8 节点（整合复验与最终报告）
- **审查对象**: 工作区状态（分支 `feature/agent-page-polish`，HEAD=`8609c93`，含 69 个未提交变更，与 00-baseline.md §0 一致）
- **输入**: outputs/code-review/ 下 9 份章节文件（00-baseline / 02 / 03 / 04a / 04b / 05 / 06 / 07a / 07b），全部通读
- **本节点纪律**: 只读审查，零源码修改；本报告为唯一新增产物
- **引用格式**: "章节-F编号"（如 04a-F1）指向各章节文件内的原始发现；合并组用 G1–G7 标注

---

## a. 执行摘要

### 整体评价

**架构设计（良）**：四层 crate 依赖为无环 DAG（domain/hostd-protocol 双叶子、server 对 application 仅 dev-dep），职责切分清晰；run 状态机守卫有唯一收口点（`transition_run_on_connection` 带 CAS 双保险）；审批精确匹配闭环（sha256 绑定 + 三层校验）经 04b 取证确认无 TOCTOU；hostd 特权边界（SO_PEERCRED、allowlist、容器全硬编码加固、digest-pinned、Caddy/Compose 白名单解析器）经 06 章穷尽核对为全仓最扎实的部分。缺陷集中在跨层契约的"字符串约定"地带（事件 payload 无 schema、ManagedDeploy action 字面量、状态枚举三重编码）与若干隐式不变量（ordinal 排序等价性、"排空才调模型"）无文档无测试钉住。

**安全姿态（中上，有一个结构性缺口）**：凭据链路干净（httpOnly+SameSite=Strict cookie、token SHA-256 落库、SecretRef 全链脱敏、SMTP 注入面封死、XSS 无 {@html} 可达通路、e2e 后门 feature 门控无泄漏）；但 **05-F1 是结构性缺陷——审批作为唯一人工控制点，Owner 却看不到将被执行的参数全文（只有 sha256），"精确审批"在 workspace/process/sandbox 三类工具上退化为盲批**；叠加 G1（重复审批 500 全链）与 07b-F7（折叠态隐藏审批按钮可 Tab 盲触发），审批控制点在"看不清、不可靠、可误触"三个维度各有伤口。hostd 侧 06-F1（Compose 插值使审批预览与部署内容分叉）是对"精确审批"的第四处侵蚀。

**工程质量（中上）**：Rust 侧 fmt/clippy -D warnings/106 项测试/rust-analyzer 全绿，生产代码零 panic 面（storage/application 生产段 grep 0 命中），tools.rs 路径约束与原子写是亮点；但测试有系统性缺口（预算耗尽/续期器/Deny 路径/call_id 冲突/并发窗口零覆盖，全库 106 项测试几乎全部跑在 `:memory:` 单连接上使 DEFERRED/IMMEDIATE 行为差异物理上不可能暴露）；前端质量分层明显——组件实现精致（runes 用法、a11y 细节、脱敏渲染），但测试防线近乎裸奔（10 个 mock 内部单测 + 2/2 全红的 e2e，router/组件零测试），且 UI 重构后未同步 e2e 规格与部分可访问性语义（heading 消失、TERMINAL 集漂移）。

### 风险画像

1. **审批面（最高）**：P1×2 + P2×3 直击"精确审批"这一产品核心承诺——盲批（05-F1）、重复提交零反馈（G1）、隐藏按钮可盲触发（07b-F7）、预览可分叉（06-F1）、已批准调用可被静默跳过（04a-F2）。
2. **静默失效面（次高）**：call_id 静默丢弃烧预算（04a-F1）、连接池污染静默丢写（03-F1）、相对路径致多进程静默分库/分工作区（G4）、恢复扫描单 run 失败饿死全部 claim（G6）、blocked 终态前端全链不可见（G3）——共同特征是"不报错、只慢慢坏"。
3. **运维/引导面（中）**：owner-init 长度契约缺口可造成整库锁死（02-F2）、迁移中间态卡死（03-F2）、并发双 Owner 窗口（03-F3）、hostd OOM 面（06-F2）、运行产物只增不删（G7）。
4. **回归防线（中）**：e2e 2/2 全红使唯一集成防线归零——在修好它之前，任何审批链修复都没有端到端验证手段。

### 发现统计

各章节原始统计（以章节末统计行为准；07b 按正文修正口径，见附录 f-1）：

| 章节 | P0 | P1 | P2 | P3 | 小计 |
|---|---|---|---|---|---|
| 02 domain/protocol/bins | 0 | 0 | 3 | 13 | 16 |
| 03 storage | 0 | 0 | 3 | 13 | 16 |
| 04a application-engine | 0 | 1 | 2 | 8 | 11 |
| 04b application-tools | 0 | 0 | 2 | 7 | 9 |
| 05 server | 0 | 1 | 1 | 11 | 13 |
| 06 hostd | 0 | 0 | 3 | 7 | 10 |
| 07a webui-contracts | 0 | 0 | 5 | 8 | 13 |
| 07b webui-quality | 0 | 0 | 4 | 17 | 21 |
| **原始合计** | **0** | **2** | **23** | **84** | **109** |

同根因跨章节合并 7 组（G1–G7，合计吸收 13 条）后主清单：

| 严重性 | 数量 | 说明 |
|---|---|---|
| P0 | 0 | — |
| P1 | 2 | 05-F1（审批盲批）、04a-F1（call_id 静默丢弃） |
| P2 | 18 | 含 4 个跨模块合并组（G1/G2/G3/G4） |
| P3 | 76 | 含 3 个跨模块合并组（G5/G6/G7） |
| **合计** | **96** | 原始 109 − 去重 13 |

主清单模块 × 严重性分布：

| 模块（归属章节） | P1 | P2 | P3 | 小计 |
|---|---|---|---|---|
| domain/protocol/bins（02） | 0 | 1 | 12 | 13 |
| storage（03） | 0 | 3 | 11 | 14 |
| application-engine（04a） | 1 | 1 | 6 | 8 |
| application-tools（04b） | 0 | 2 | 6 | 8 |
| server（05） | 1 | 0 | 10 | 11 |
| hostd（06） | 0 | 3 | 6 | 9 |
| webui-contracts（07a） | 0 | 2 | 7 | 9 |
| webui-quality（07b） | 0 | 2 | 15 | 17 |
| 跨模块合并组（G1–G7） | 0 | 4 | 3 | 7 |
| **合计** | **2** | **18** | **76** | **96** |

**复验结论**：P1 2/2、P2 合并组 18/18（原始 23/23，100%，超出"抽查一半"要求）全部由本节点亲自回读代码核实成立；零降级、零删除；1 处统计口径修正 + 2 处细节补充（见附录 f）。

---

## b. Top 10 优先修复项

| # | 严重性 | 发现（来源） | 位置 | 一句话影响 | 修复建议 | 工作量 |
|---|---|---|---|---|---|---|
| 1 | P1 | 05-F1 | server http.rs:378-401 + domain/runtime.rs:143-157 | workspace/process/sandbox 审批时 Owner 只能看到 sha256，无法知悉将执行的实际参数——informed consent 在真实后端下不成立 | `ToolCallSummary` 增加 `argumentsJson`（带截断上限）或提供专用审批详情端点；前端审批 UI 展示与 hash 绑定的同一份参数文本（e2e 断言 `/arguments sha256:/` 同步更新） | M |
| 2 | P1 | 04a-F1 | engine.rs:369-384 + storage/runtime.rs:441-452 + 0002:46-47 | 模型复用 call_id 时工具调用被 `INSERT OR IGNORE` 无声丢弃，模型重发再丢，run 空转烧完预算才 Blocked（部分 OpenAI 兼容网关用局部短 id，现实可达） | engine 侧 call_id 作用域化/冲突回退 UUID（保证 journal tool_result.callId 同源）；storage 侧 INSERT OR IGNORE 改显式冲突检测并 fail-loud；长期迁移 PK 改 (run_id, call_id) | M |
| 3 | P2 | G1 = 03-F12+05-F2+07a-F4+07b-F1 | storage runtime.rs:620-654 + server error.rs:81-88 + run 页 :83-98 + ToolCallCard :7-13/133-145 | 双击/重试审批 → 裸 UNIQUE 错误（或接管后 RunNotFound）→ 500 catch-all → 前端无 catch 无 busy → Owner 零反馈、无法判断是否已批准 | 三处联动：storage INSERT 前冲突映射 `ToolCallStateConflict`；server 增 409/404 映射（RunNotFound 勿再 500）；ToolCallCard 加 busy prop + decide 补 catch 渲染错误 | S |
| 4 | P2 | 07b-F7 | app.css:407-417 + ToolCallCard:114/134-144 | 折叠仅靠 CSS（grid 0fr + overflow hidden），折叠后 Approve/Deny 仍可 Tab 聚焦——键盘用户可"盲批准/盲拒绝"，核心控制点存在不可见激活路径 | 折叠态给 body 加 `inert={!open}`（一行，现代浏览器支持），或 `visibility: hidden` | S |
| 5 | P2 | 06-F1 | hostd deployment.rs:1497-1559（+ :545-561/:984-999/:1186-1195） | workspace `.env` 参与 plan 期插值而 apply 期 bundle 无 `.env`，Owner 批准的预览与实际部署的 revision 内容可分叉（两镜像恰都预载时打破 informed consent） | `validate_compose_source` 直接拒绝含 `$` 的 compose 源文本（不可变 bundle 本就不需要插值）；或 plan/apply 统一 `--project-directory`/`--no-interpolation` | S |
| 6 | P2 | 04a-F2 | engine.rs:216-238 | 预算检查先于 pending 执行：越过预算线的批次的调用（含**已获 Owner 精确批准的 managed 调用**）被永久滞留 pending，run 直接 Blocked，审批石沉大海且无事件 | 预算检查移到 pending 排空后、call_model 前；Blocked 前对滞留调用补记 `fail_tool_call_before_start("budget_exhausted")`；审批恢复路径产生"approved call skipped"事件/审计 | S |
| 7 | P2 | 06-F2 | hostd deployment.rs:238-257 | managed 路径所有 docker/caddy 子进程输出由 `.output()` 无界缓冲到退出才检查，异常打印可 OOM 全仓权限最高进程（同时中断沙箱与受管部署服务） | 复用 unix.rs:750-772 的 drain 流式截断模式（双流合计 limit 即时中止），或 bounded reader 包装 | M |
| 8 | P2 | 03-F1 | storage tasks.rs:128-185 + runtime.rs:697-737 | `claim_next_run`/`claim_approved_run` 手写 BEGIN/COMMIT 绕过 sqlx RAII：COMMIT 失败时带未决事务的连接回池，此后 1/8 流量的写入累积在永不提交的幽灵事务里静默丢失 | 两处改用现成的 `begin_write()`（BEGIN IMMEDIATE + Transaction RAII，Drop 自动回滚；trigger 回滚测试证明该路径可靠） | S |
| 9 | P2 | G2 = 02-F1+04a-F3+07a-F11+07b-F2（+07a-F2） | e2e/happy-path.spec.ts 全文 + login/+page.svelte:17-22 | e2e 2/2 全红使唯一集成防线归零；且 login 页迟到 `goto("/tasks")` 会劫持登录往返期间的用户导航（抢走正在填的表单），也是 e2e 失败 2 的 app 侧机制 | spec 重写（登录后 waitForURL、选择器消歧、按现 UI 更新全部断言）；login goto 前校验仍在 /login；修好后 `bun run test:e2e` 作为后续所有修复的集成验证 | M |
| 10 | P2 | 02-F2 | bins/soloopsctl/main.rs:45-52 vs domain requests.rs:88/94 | owner-init 创建侧无长度上限而登录侧强制 ≤64/≤1024，可创建**永远无法登录**的 Owner；`count_users>0` 永久阻止重初始化——单用户系统整库锁死，只能手工改 SQLite | 把 username/password 上限与字符集提为 domain 共享校验，ctl 与 `create_owner` 写入前强制执行（一并解决 02-F9 的 static str 限制） | S |

紧随其后（第 11–15 候选）：03-F3（并发双 Owner 窗口，修复仅一行 `begin_write`）、G4（CWD 相对路径族，多进程静默分库/分工作区）、G3（blocked 终态前端 4 处+mock 不可见）、06-F3（沙箱看门狗 TOCTOU 误杀合法负载）、04b-F2（managed 结果未知分类不完整）。

---

## c. 全部发现明细（96 条）

> 每条保留原章节的位置、核心证据与建议（P3 条目为压缩表述，原文全文见对应章节文件）；带 ★ 的条目经本节点复验亲自回读代码核实（P1/P2 全部核实，P3 中与 Top10/合并组相关的关键链路亦经抽查）。

### c-1 P1（2 条）

**P1-1 ★ 04a-F1：tool_calls.call_id 全局主键 + INSERT OR IGNORE 静默丢弃**
- 位置：crates/soloops-application/src/engine.rs:369-384、:356、:402；crates/soloops-storage/src/runtime.rs:441-452；migrations/0002_agent_runtime.sql:46-47
- 证据：`call_id: if call.call_id.is_empty() { Uuid::new_v4()… }`（仅空值回退，无重复防御）；`INSERT OR IGNORE INTO tool_calls`（无 rows_affected 检查）；`call_id TEXT PRIMARY KEY NOT NULL`（跨 run 全局唯一，非 (run_id, call_id) 复合）。
- 问题：跨 run/跨批 id 复用时行被静默忽略——被丢弃的调用无 tool_result，模型下一轮看不到结果而重发同 id → 再丢 → 死循环；`usage.tool_calls` 与 `made_progress` 均在落库前判定（`!calls.is_empty()`），无进展守卫不触发，run 以每轮一次模型调用的代价空转直到 model_turns 耗尽 → Blocked("budget_exhausted")。e2e_harness 的 ScriptedProvider 固定 call_id（"e2e-managed-plan"/"e2e-managed-apply"）是仓库内现成例证（同进程第二个 managed 任务即触发）。
- 建议：① engine 构造 NewToolCall 时对重复/已存在 id 归一化（批内去重 + `format!("{run_id}:{call_id}")` 作用域化或冲突回退 UUID，须保证 journal tool_result.callId 与模型可见 id 一致——04b 已核实现状两点同出自 call.summary.call_id）；② storage INSERT OR IGNORE 改显式冲突检测上抛；③ 长期 PK 改 (run_id, call_id)（需评估 FK 级联）。
- 复验核实：engine.rs:379-383/356/402、runtime.rs:444-452、0002:46-47 逐行回读一致；另核实 engine.rs:397-402 的 stop_reason=length 截断防御存在，但其仅覆盖 length 场景，不改变本结论。

**P1-2 ★ 05-F1：审批信息面缺口——API 不暴露工具参数全文，审批只能基于 sha256 盲批**
- 位置：crates/soloops-server/src/http.rs:378-401 + crates/soloops-domain/src/runtime.rs:143-157 + crates/soloops-storage/src/runtime.rs:1343-1365
- 证据：`ToolCallSummary` 全部 11 字段为 callId/name/argumentsSha256/approvalPreview/risk/policy/status/resultSummary/errorCategory/startedAt/completedAt——无 arguments；全库 grep `arguments_json` 8 处命中**全部**位于 soloops-storage 内部（runtime.rs:442/443/446/457/1347/1370、database.rs:148、0002:52），server/domain 任何响应 DTO 无该字段；RuntimeSnapshot.toolCalls 即 ToolCallSummary（runtime.rs:195）；approval_preview 仅 managed deploy 工具经 set_tool_call_approval_preview 写入。
- 问题：审批是唯一人工控制点。对 workspace.create/replace（写内容）、process.exec（执行命令）、sandbox.exec，Owner 能看到的仅是哈希——"批准的正是将执行的"这一承诺在真实后端下不成立；摘要绑定（sha256 不可变 + 三层闭环）反而成为反讽：绑定的正是 Owner 看不到的文本。
- 建议：ToolCallSummary 增加 `arguments_json`（可带截断上限与"全文见 detail"提示），或为 waiting_for_approval 调用提供专用审批详情端点；前端审批 UI 展示与 hash 绑定的同一份参数文本；openapi.yaml 与 e2e 断言同步。
- 复验核实：domain/runtime.rs:143-157 与 grep 结果逐条回读一致。

### c-2 P2（18 条）

#### 跨模块合并组（4）

**P2-3 ★ G1：审批重复提交全链缺陷（同根因，四处）** = 03-F12 + 05-F2 + 07a-F4 + 07b-F1
- 位置：storage runtime.rs:620-654（decide 的 SELECT+INSERT）+ :640（RunNotFound 重用）；server error.rs:81-88（catch-all 500）+ openapi.yaml:201-207（decision 端点零错误声明）；apps/web runs/[runId]/+page.svelte:83-98（decide 无 catch）；lib/components/ToolCallCard.svelte:7-13/133-145（无 busy/disabled）
- 证据与问题（四层同根因）：① storage——decide 不改 tool_calls.status，SELECT 条件（tc/runs 均 waiting_for_approval）在重复审批时依然通过，INSERT 撞 tool_approvals 主键返回**裸 UNIQUE 错误**（非语义化冲突错误）；worker 接管后重复提交则返回 RunNotFound——两路都落 ② server catch-all → **500**；③ 前端 decide() try/finally 无 catch，错误静默逃逸成 unhandled rejection；④ ToolCallCard 行内 Approve/Deny 无 disabled（ApprovalBanner 的 busy 防护管不到卡片按钮）——双击可重复 POST。组合后果：Owner 双击 → 500 → 零反馈，无法判断审批是否已生效。
- 建议：storage ON CONFLICT/捕获 UNIQUE → `ToolCallStateConflict`（03-F12 附带：RunNotFound 重用于 tool call 与 revision，应拆 ToolCallNotFound/RevisionNotFound 变体）；server 增 409（冲突）与 404（接管后）映射 + openapi 补错误声明；前端 ToolCallCard 加 busy prop、decide 补 catch 渲染 error 横幅、重复提交给幂等确认提示。
- 复验补充：decide 的 SELECT 同时要求 `runs.status = 'waiting_for_approval'`（runtime.rs:633-634）——重复审批存在两个不同的 500 窗口（接管前撞主键 / 接管后 RunNotFound），修复时需分别映射 409/404。

**P2-4 ★ G2：e2e 套件 2/2 全红（同根因调查链，四处）** = 02-F1 + 04a-F3 + 07a-F11 + 07b-F2
- 位置：apps/web/e2e/happy-path.spec.ts:10/:11/:17/:38-45/:47-51；login/+page.svelte:17-22（07a-F2，强关联独立条目）；tasks/+page.svelte:105-107（07a-F6，失败 1 的 UI 侧）；最终定性见 07a"e2e 失败最终定性"小节
- 定性（07a 为准）：失败 1 = 纯断言漂移（复合布局后 "Tasks" 是 span 非 heading，全页零 heading）+ a11y 回归为次——**非功能回归**；失败 2 = spec 登录后零等待抢跑（:38→:40 无 waitForURL）+ login 迟到 goto 劫持（07a-F2）复合竞态——**非 create→run 链路功能回归**（04a/05/06 已三方排除后端；07a 以快照法医闭环：最终态"Tasks 0 + No task selected + 无错误横幅"只能由"创建 401 失败 + 迟到 goto 落地 /tasks"产生）。07b 确证：即使修好登录竞态，spec :48（"privileged · waiting_for_approval"）、:49（"Approval preview"）、:51（/arguments sha256:/）三断言文本在全部 src 0 命中（实际渲染为 "awaiting decision"/"Approval required"/"sha256 <hex>"），:50 e2e.example.test 经 approvalPreview JSON.stringify 渲染成立。04a-F3 的"Create task 不再导航"假设被 07a 否证（tasks/new:25 存在 goto）。
- 建议：spec 重写（登录后 `waitForURL("**/tasks")`、:40 选择器限定 Primary navigation、:47-51 按现 UI 断言或恢复旧文案）+ 07a-F2 goto 守卫 + 07a-F6 侧栏 h1（一并修 a11y）；`bun run test:e2e` 转绿后作为所有后续修复的集成防线。
- 复验核实：spec 全文 61 行回读、login goto 无守卫、"Approval preview"/"arguments sha256"/"privileged · waiting" 0 命中、"awaiting decision"/"Approval required" 实际存在、tasks 侧栏确为 span。

**P2-5 ★ G3：blocked 终态前端全链盲区（同根因，3 条目覆盖前端 4 处实例 + mock 数据层）** = 07a-F1 + 07b-F4 + 07b-F13
- 位置：domain/run.rs:65-70（`is_terminal` 含 Blocked——权威定义）；前端四处手写终端集合缺 "blocked"——routes/+page.svelte:17、tasks/+page.svelte:66、runs/[runId]/+page.svelte:30、StatusBadge.svelte:9；mock/state.ts:250-255（setRunStatus 终态数组亦缺 blocked，且 queued 即写 startedAt）
- 问题：blocked 是审批/托管链路真实可达终态（block_managed_change → Blocked；managed apply 超时 → Blocked）。四处漏掉它导致：Overview/tasks 活跃计数永久虚高；run 页 isActive 为真 → "live" 指示常亮、Cancel 按钮常驻；WS 断连后 scheduleReconnect（:152 用同集合判定）对 blocked run **无限重连**；StatusBadge 圆点永久 pulse；mock 层也不产出 blocked——"blocked 显示错误"类缺陷在 mock 模式与现存测试下不可能被发现（三层盲区）。
- 建议：contracts.ts 导出单一 `TERMINAL_STATUSES` 常量（镜像 domain is_terminal），四处统一引用；mock 补 blocked 场景与终态数组、startedAt 仅 planning/running/leased 写入。
- 复验核实：is_terminal 源码与四处集合 grep 逐一命中一致；run 页 :152/:292 消费点确认。

**P2-6 ★ G4：关键路径默认值按 CWD 相对解析 + 三进程配置零交叉校验（同根因，两处）** = 02-F3 + 04a-F9
- 位置：server/http/config.rs:39/:82/:190-200（`SOLOOPS_DATABASE_PATH` 默认 `var/db/soloops.db` 相对路径 → `current_dir().join`；web_dist 默认 `apps/web/build` 同族）；application/config.rs:184-185/:225-232（`var/workspaces`/`var/artifacts` 同模式）；三个 bin 各自独立 `AppConfig::load()`（bins/soloops-api:10-12、worker:14-17、ctl:30-31）
- 问题：从不同 CWD 启动任一进程（systemd WorkingDirectory 不一致、手工 cargo run 混用）→ api/worker 静默打开不同 SQLite（展示与执行完全脱节、无报错无日志）；worker 的 workspace/artifact 根漂移使预算统计错目录、中断恢复 probe 在错误目录判定 Unsafe → Blocked。README"必须一致配置"在代码中无任何校验（doctor 也不查，见 02-F16）。
- 建议：① 数据库内记录部署实例 ID，各进程启动校验；② 默认值锚定固定数据目录或要求绝对路径；③ doctor 显式核对各进程解析出的 database_path 与 hostd socket（与 02-F16 联动修）。
- 复验核实：两处 absolute/absolute_path 拼接逻辑逐行回读一致。

#### storage（3）

**P2-7 ★ 03-F1：claim_next_run / claim_approved_run 手写事务绕过 RAII——COMMIT 失败连接带未决事务回池**
- 位置：database/tasks.rs:128-185；runtime.rs:697-737
- 证据：`sqlx::query("BEGIN IMMEDIATE")…` 手动开事务；`match result { Ok => sqlx::query("COMMIT")…? ` —— COMMIT 失败 `?` 早退，connection drop 回池；ROLLBACK 失败 `let _ =` 吞掉。sqlx 0.8.6 registry 源码取证：`return_to_pool` 的 ping 探活不回滚手动 BEGIN 的事务。
- 问题：COMMIT 失败（SQLITE_FULL/IO 错误）后该连接的后续写入全部累积在永不提交的幽灵事务里（进程退出回滚=静默数据丢失），下一个经它 BEGIN IMMEDIATE 的调用者得到 "cannot start a transaction within a transaction"。database.rs:351-353 已有 `begin_write()`（RAII），全库其余写事务都在用它——这两处是仅有的绕过者且无必要。
- 建议：改用 `begin_write()`（auth.rs 三个 trigger 测试证明该路径回滚可靠）。
- 复验核实：tasks.rs:115-186 逐行回读，COMMIT `?` / ROLLBACK 吞错与描述一致。

**P2-8 ★ 03-F2：迁移 1/2 不在事务中且无 IF NOT EXISTS——中途失败留下无法自愈的中间态；并发 migrate 撞车**
- 位置：database/schema.rs:21-56；migrations/0001/0002 全部 CREATE TABLE 无 IF NOT EXISTS；对照版本 3-6 的事务包裹（schema.rs:62 起）
- 证据：`sqlx::raw_sql(BASELINE_SQL).execute(&self.pool)`——多语句批处理逐条自动提交；检查 version → 执行 SQL → INSERT 记录三步非原子。
- 问题：kill/磁盘满时部分表已建而 migration 记录未写 → 重跑走"接管旧 schema"分支 → verify 缺表报 MissingTable，每次启动失败且无自动恢复路径（须手工 DROP）；双 ctl 并发 migrate 撞 TOCTOU。缓解：api/worker 启动只 `ready()` 不迁移（明确报错指引），风险仅限全新库，撞车一次重试即恢复。
- 建议：版本 1/2 与 3-6 对齐用 `begin_write()` 包成原子单元；CREATE TABLE 加 IF NOT EXISTS 纵深防御。
- 复验核实：schema.rs:21-56 回读，raw_sql 直跑 pool、无事务、TOCTOU 三步与描述一致。

**P2-9 ★ 03-F3：create_owner 单用户约束是 DEFERRED 事务内 check-then-act——并发窗口可产生两个 Owner**
- 位置：database/auth.rs:15-21（+ :10-13 创建侧无上限，与 02-F2 交叉）
- 证据：`pool.begin()`（DEFERRED）→ `SELECT COUNT(*)` → INSERT；users 表仅 username UNIQUE，不阻止两行不同 username。
- 问题：并发两次 `soloopsctl owner-init`：A/B 各读快照 count=0，WAL 下 B 的读快照不推进 → 双双 INSERT 成功，单用户不变量被永久绕过（两个凭据均可登录）。触发窗口极窄（首次部署毫秒级），但属不变量级破绽且修复成本一行。
- 建议：改 `begin_write()`（IMMEDIATE 第一条语句前取写锁）；兜底 `CREATE UNIQUE INDEX users_singleton ON users((1))`。
- 复验核实：auth.rs:10-35 逐行回读一致（提前 return 时事务 RAII 回滚正常，窗口机制成立）。

#### application（3）

**P2-10 ★ 04a-F2：预算检查先于 pending 执行——把预算推过线的批次（含已批准调用）永久滞留，run 直接 Blocked 且无工具级失败事件**
- 位置：engine.rs:216-238（检查顺序）；report.rs:29-35
- 证据：`if budget_exhausted(…) { transition_run(Blocked, "budget_exhausted"); return }` 在 `pending_tool_calls` 读取/执行**之前**；usage 在 persist 响应时立即累加。
- 问题：触发越限批次的调用全部停在 pending、无事件、模型视角任务戛然而止；更坏变体——已进入 WaitingForApproval 并获批准的 managed 调用，批准后 claim_approved_run 恢复 → 循环顶部预算再次命中 → **已获精确批准的调用被静默跳过**，run Blocked，审批行仍在、调用永远 waiting，且此后 server 审批端点因 run 非 waiting 状态必然拒绝后续操作。
- 建议：① 预算检查移到 pending 排空之后、call_model 之前；② Blocked 前对滞留调用补记 fail_tool_call_before_start("budget_exhausted")；③ 审批恢复路径产生 "approved call skipped" 事件/审计。
- 复验核实：engine.rs:216-238 回读，检查顺序与描述一致。

**P2-11 ★ 04b-F1：workspace.search / list 结果不经 truncate_output——单次工具值可达数百 KB~2MB，突破输出预算且与模型看到的 "Output limit: 65536 bytes" 描述直接矛盾**
- 位置：tools.rs:501-521（search 构造 value）、:283-311（list）、model.rs:235-241（描述拼入 output_limit）、storage/runtime.rs:838（result_json 无上限落库）
- 证据：search `preview: line.chars().take(500)` × 200 条上限 ≈ 100-400KB；list 500 条深路径 ≈ 最多 ~2MB；均直接进 value，无 truncate_output。
- 问题：process/sandbox/read 都过双预算截断，唯二例外是 search/list——模型上下文被单工具挤占、DB 单行膨胀、token 统计低估，且模型被告知的输出上限是假的（与 04b-F5 双轨问题一并修）。
- 建议：search/list 序列化 value 一律经 truncate_output（Head 保留 + artifact 全文）；或 finish_tool_call 层设 result_json 字节上限；描述文本取真实执行预算。
- 复验核实：tools.rs:296-311/:506-521 回读，两处 value 构造均无截断；search 的逐文件大小预检（:495-497）存在但只防单文件不防聚合。

**P2-12 ★ 04b-F2：managed "结果未知" 分类只认 Timeout——read_frame 失败与 is_cancelled 的 DB 错误同为结局未知却记普通失败放行 run 继续**
- 位置：hostd.rs:214-216/:287-289（响应失败→Execution）；execution.rs:263-277（保守 block 仅认 resuming/RecoveryRequired/Timeout 三类）、:386-396（is_cancelled Err 压成工具错误并丢弃 in-flight future）
- 证据：`read_frame(…).map_err(…ToolError::Execution(…))`——请求已送达、结局未知，与"未开始"同类；inspect Missing 时仅三类触发 "outcome is unknown" 的 Blocked；`is_cancelled().map_err(…Tool(Execution))?` 返回时正在执行的 future 被丢弃。
- 问题：① 响应阶段连接断开 → fail_tool_call("execution") run 继续 → 稍后 hostd 完成 apply 转 active——产生"失败记录 + active revision"矛盾组合（客户端触发面，与 04a-F4 恢复判定缺口互补）；② SQLite busy 等瞬时 DB 错误被记成模型可见的工具失败：managed 即断开 socket（同①），workspace 写场景 spawn_blocking 分离续跑、文件实际写成功但调用记失败，模型下轮重试撞 "already exists"。
- 建议：hostd.rs 区分"未发出"（可安全 fail）与"发出后失败"（映射 RecoveryRequired 语义）；engine 对 managed 在 Missing+任何非确定失败时保守 block；is_cancelled 的 Err 原样上抛 RuntimeError 不得伪装工具失败。
- 复验核实：execution.rs:263-277/:380-400 与 hostd.rs:211-216 回读一致（Missing 落空分支 :260 直通 fail_tool_call）。

#### hostd（3）

**P2-13 ★ 06-F1：Compose 变量插值未被禁止——plan 期 workspace `.env` 可使审批预览与实际部署内容分叉**
- 位置：deployment.rs:1497-1559（validate_compose_source 禁键清单无 `$`）、:545-561（plan 期 normalize）、:984-999/:1186-1195（apply 期 bundle 为 project-directory）
- 证据：forbidden 列表 `["extends","env_file","secrets","configs","label_file","provider"]`——无插值检查；plan 期 compose 文件所在目录（run workspace）的 `.env` 参与插值，apply 期 bundle 目录只有 compose.yaml/site.caddy。
- 问题：模型可先经 workspace 工具写入 `.env` 与 `${VAR:-default}` compose——预览/校验/`docker image inspect` 按 workspace 值，apply 按 bundle 缺省值。缓解面：apply 期 validate_revision_bundle 对实际生效结果全量重校验、`up --pull never` 拒绝未预载镜像；但两镜像恰都预载时 informed consent 被打破。
- 建议：validate_compose_source 拒绝含 `$` 的源文本；或两侧统一 --project-directory/--no-interpolation。
- 复验核实：禁键清单逐行回读（:1542-1549），确无 `$` 检查。

**P2-14 ★ 06-F2：SystemCommandRunner 无流式输出上限——`.output()` 全量收集后才检查 output_limit**
- 位置：deployment.rs:238-257
- 证据：`Command::new(program)…stdout(piped).stderr(piped).output()` 包在 timeout 里，之后 `if output.stdout.len()+output.stderr.len() > output_limit` 才检查。
- 问题：与 process/sandbox 的流式截断（unix.rs:750-772 按 8KB 增量即时中止）不同，managed 全部 docker/caddy 子进程输出无界缓冲到退出——`compose up -d --wait` 大量打印可打爆 hostd 内存，而 hostd 是全仓权限最高进程，OOM 同时中断沙箱与受管部署。超时只限时长不限体积（最长 health_timeout+30s ≈ 90s）。
- 建议：复用 drain/Capture 流式模式或 bounded reader。
- 复验核实：:238-257 逐行回读一致。

**P2-15 ★ 06-F3：沙箱 workspace 预算看门狗 250ms 轮询全量 walk，瞬时 IO 错误直接误杀合法沙箱**
- 位置：sandbox.rs:287-294（wait_for_workspace_limit）、:264-285（workspace_size 逐层 `?`）；unix.rs:366-383（Err → Internal 中止调用）
- 证据：`workspace_size(root, maximum).await? > maximum`——symlink_metadata/read_dir/next_entry 的 NotFound 沿 `?` 冒泡；看门狗 Err 分支映射 "sandbox workspace monitoring failed" 中止整个 sandbox 调用。
- 问题：① bind mount 无法配额，预算执行全靠此轮询——检测窗口内可写入不受限数据（"预算=硬上限"不成立，靠 remove_sandbox 删容器止损）；② 容器进程并发增删文件时 walk 的 TOCTOU NotFound 会把正常做 churn 的合法工作负载（构建/测试）随机误杀。
- 建议：workspace_size 对 NotFound 类跳过；轮询间隔自适应；文档明示 bind-mount 预算为尽力而为。
- 复验核实：sandbox.rs:264-294 逐行回读，`?` 冒泡链与 unix.rs 映射（章节引文）一致。

#### webui（4）+ bins/domain（1）

**P2-16 ★ 07a-F2：login 页 fire-and-forget goto——登录响应迟到时劫持任意后续导航**
- 位置：login/+page.svelte:17-22
- 证据：`await api("/api/auth/login", …); await goto("/tasks");`——无路由守卫；Argon2 往返数百 ms（05-F3 佐证），期间用户点 rail 任意链接离开后，闭包仍在响应到达时执行 goto。
- 问题：用户被从当前页面（可能正在填表单）强行拽回 /tasks，表单内容丢失；组件卸载不取消导航；亦为 e2e 失败 2 的 app 侧机制（G2）。
- 建议：goto 前校验 `page.url.pathname === "/login"`（或 onDestroy 置位）；顺带支持 ?redirectTo 回跳。
- 复验核实：login 页 :13-28 回读，goto 无条件执行。

**P2-17 ★ 07a-F3：runs/[runId] refreshRuntime 无请求时序防护——并发触发时过期响应覆盖新快照**
- 位置：runs/[runId]/+page.svelte:75-81、:106-124
- 证据：`applyEvent` 对每个 run.status_changed/agent.*/tool.*/run.reported 事件 `void refreshRuntime()`——无 AbortController/序号检查；refreshRuntime 直接 `runtime = await api(…)`。
- 问题：WS 突发（重连补拉尤甚）并发多请求，响应乱序时旧快照后到覆盖新快照——终报快照可被前一个事件的旧快照覆盖且无后续自愈（审批横幅闪回、终报短暂消失）；`void` 同时把非 404 异常吞成 unhandled rejection。
- 建议：仅最后一次生效模式（序号/Promise 校验）或 AbortController；批量事件合并为一次刷新。
- 复验核实：:106-124 回读，每个匹配事件无条件 fire-and-forget。

**P2-18 ★ 07b-F3：time.ts formatDuration 秒数用 round——可产出 "1m 60s"**
- 位置：lib/time.ts:8-10（另 :7 的 59.96s → "60.0s" 同族）
- 证据：`minutes = Math.floor(totalSeconds/60); seconds = Math.round(totalSeconds % 60)`——ms=119_500 → "1m 60s"。
- 问题：ToolCallCard/ReportCard 时长展示直接消费；整分边界 ±0.5s 内必触发。
- 建议：秒改 Math.floor（或先整体 round 再拆分），保持截断语义一致。
- 复验核实：time.ts:1-13 回读，边界计算复核成立。

**P2-19 ★ 07b-F7：agent-disclose 折叠态不切断可聚焦性——折叠的 Approve/Deny 仍可 Tab 盲触发**
- 位置：app.css:407-417（grid 0fr + overflow hidden，无 visibility/inert）；ToolCallCard:114（data-open 仅驱动 CSS）、:121-146（按钮无 aria-hidden/inert）
- 证据：卡片 awaiting 时自动展开，但用户可再折叠；折叠后按钮仍在 DOM 与 Tab 序列。
- 问题：键盘用户 Tab 落到不可见按钮（焦点视觉消失），Enter 即盲批准/盲拒绝——审批控制点的不可见激活路径；aria-expanded 有但 body 无 aria-hidden/inert 管理。
- 建议：`inert={!open}` 一行（或 visibility:hidden + transition）。
- 复验核实：app.css:407-417 与 ToolCallCard:114/133-145 回读一致。

**P2-20 ★ 02-F2：owner-init 与登录的长度契约不一致——可创建永远无法登录的 Owner（单用户系统即整库锁死）**
- 位置：bins/soloopsctl/main.rs:45-52（仅 `< 12` 下限）；domain requests.rs:88/94（登录上限 64/1024）；storage auth.rs:10-13（创建侧亦无上限）
- 证据：ctl 创建侧无 username 长度与密码上限校验；登录 normalize 处 400 拒绝；`count_users() > 0` 永久阻止重初始化（ctl main.rs:42-44）。
- 问题：操作员用超长用户名/密码初始化后，Owner 创建成功但所有登录被拒，只能手工改 SQLite 解锁——"创建路径接受、消费路径拒绝"的跨层契约缺口。
- 建议：上限与字符集提为 domain 共享校验，ctl 与 create_owner 写入前强制执行（受 02-F9 static str 限制，宜一并修）。
- 复验核实：ctl :42-52 与 requests.rs:85-105 回读一致。

### c-3 P3（76 条）

#### domain / protocol / bins（12）

| 编号 | 位置 | 问题与建议（原文要点） |
|---|---|---|
| 02-F4 | run.rs:7,13 + contract_files.rs:14-24 | RunStatus 含死状态 Draft/Paused（全库零写入点），契约测试把死状态强制传播到前端与 OpenAPI，误导开发者以为功能已存在。建议删除或落地暂停功能并标注 reserved（删除需 domain/contracts.ts/openapi.yaml/契约测试四方同步）。 |
| 02-F5 | run.rs:24-63 + event.rs:8-38 | RunStatus/EventType 手写 as_str 与 serde rename 双重编码无一致性测试，新增含数字/大写变体时 DB 文本/事件 JSON/前端契约可静默分叉。建议穷尽性 round-trip 单测。 |
| 02-F6 | requests.rs:10-15 | LoginRequest 持明文密码 derive Debug/Clone/Serialize，距违反 security.md "密码不得入日志" 红线仅一次 `debug!("{:?}")` 之遥（现行无泄漏点，已 grep 核实）。建议手写脱敏 Debug 或 zeroize 包装。 |
| 02-F8 | event.rs:63-72 | EventEnvelope.payload 为无类型 Value，8 类事件无 schema 契约，字段名漂移只能联调发现（e2e 失败 2 温床之一）。建议为 tool.call_* 先建强类型 payload/契约测试。 |
| 02-F9 | error.rs:34-39 | ValidationError field/message 均 &'static str，错误消息无法携带运行期上下文（限制 02-F2 修复质量）。建议 Cow<'static, str>。 |
| 02-F10 | hostd-protocol lib.rs:53-83 | HostdResponseV3 类型层可表示 result/error 双 None/双 Some（06 已核实 hostd 侧仅经构造器产生合法组合、消费端有防御）；风险在协议类型可表示性与未来新消费者。建议 into_outcome() 收口或自定义 Deserialize 强制恰其一。 |
| 02-F11 | hostd-protocol lib.rs:85-119 + application/hostd.rs:158 | ManagedDeployResult 响应侧以自由字符串 action/status 承载 4 子命令判别（两端字面量 04a/06 已核实逐字一致，非 e2e 根因）；max_output_bytes 在 6 变体重复。建议拆 4 变体/枚举化 + 上提信封层。 |
| 02-F12 | hostd-protocol lib.rs 全文 | 254 行零文档：信任模型（同机 Unix Socket + SO_PEERCRED）、版本策略（精确匹配 + deny_unknown_fields ⇒ 零前向兼容）、帧语义全部未记录。建议补 //! 模块文档。 |
| 02-F13 | bins/soloops-api main.rs:36-55 | 关机路径两处 expect panic；notification 错误可掩盖 server 真实错误；`?` 早退跳过 database.close()。建议日志化优雅退出 + 结构化收尾。 |
| 02-F14 | bins/soloops-worker main.rs:36-58 | Ctrl+C 仅在无任务分支被轮询（连续有活时停止信号无限推迟）；ctrl_c 的 Err 被静默当信号。建议共享停止通道 + Err 记日志。 |
| 02-F15 | bins/soloopsctl main.rs:45-58 | 明文密码 String 未 zeroize（workspace 已依赖 zeroize，与 application 层密钥处理基线不一致）。建议 Zeroizing 包装。 |
| 02-F16 | bins/soloopsctl main.rs:62-64 | doctor 只做 config 解析 + database.ready()，不核查 hostd/worker/数据库一致性（与 README 声称不符，G4 的检测缺口）。建议 doctor 报告解析路径、探测 socket、校验 key ref。 |

#### storage（11）

| 编号 | 位置 | 问题与建议 |
|---|---|---|
| 03-F5 | schema.rs:28-138 | 迁移 checksum 写入后从不校验（纯装饰），迁移 SQL 漂移零检测。建议 migrate 末尾回读比对或删除字段消除误导。 |
| 03-F6 | 全 crate | 事务开启三种并存（begin_write 13 处 / pool.begin DEFERRED 15 处 / 手写 2 处），"哪条路径防了快照陈旧"不可一眼审计。建议立规矩：事务内含写一律 begin_write，纯读快照注释标明。 |
| 03-F7 | schema.rs:151/173、runtime.rs:944-951、notifications.rs:99-105 | format! 进 SQL 3 处——均无注入（常量/占位符），但违反参数化纪律；recipients 无上限可撞 SQLite 变量数上限。建议拆静态 SQL + 白名单断言 + len clamp。 |
| 03-F8 | database/tests/ | 测试覆盖缺口：recover_safe_runs 的 process→Blocked 安全分支零测试；并发测试仅 2 个；其余全 :memory: 单连接（DEFERRED/IMMEDIATE 行为差异物理上不可能暴露——03-F3 至今未暴露的原因）。建议照 concurrency.rs 模板补三类。 |
| 03-F9 | database.rs:388-395、tasks.rs:77/94 | 时间源墙钟（回拨容忍合格、pre-epoch 防御已核）；latest-run tiebreak 用 UUID v4——同毫秒并发创建时"最新 run"随机胜出。建议 UUID v7 或单调序列列。 |
| 03-F10 | tasks.rs:120-140 | claim 候选被抢后直接 None 不试下一候选，多 worker 下高概率空转一轮。建议循环再试 2-3 个候选。 |
| 03-F11 | auth.rs:110-161 | 会话三处：吊销会话物理删除断审计可追溯链；revoke 不校验 rows_affected 恒记 success；last_seen_at 每请求一写（与 worker 竞争 WAL 写锁）。建议保留行/校验/节流。 |
| 03-F13 | runtime.rs:1288-1302/838/867/562 | runtime_snapshot 无界全量加载工具结果（~10MB/工具 JSON 存两份：result_json + agent_items.payload），长 run 内存线性放大；pending 与 snapshot 排序键不一致（与 04a-F5 交叉）。建议分页/延迟 decode/评估双份存储/统一排序。 |
| 03-F14 | schema.rs:167/172 | verify_phase_zero_schema 用 `REQUIRED_TABLES[..6]` 硬编码切片，接管校验范围与数组顺序隐式耦合。建议独立 PHASE_ZERO_* 常量。 |
| 03-F15 | schema.rs:21-24/166-186 | 旧 schema 接管校验只查表/列存在不查约束/类型；任何恰含 users 表的外部 SQLite 可被静默接管。建议补列类型+关键约束+特征表校验。 |
| 03-F16 | tasks.rs:74-78/91-95 | INNER JOIN 使无 run 的 task 不可见（当前无删除路径、不可达，防御性收录——对 02 线索 3 的正式答复）。建议 LEFT JOIN 或断言测试钉住不变量。 |

#### application-engine（6）

| 编号 | 位置 | 问题与建议 |
|---|---|---|
| 04a-F4 | execution.rs:868-887 | managed_change_recovery 信任 operation.error_category 判 Failed，不交叉核对 revision 实际状态——"operation failed + revision active" 矛盾组合可被原样固化（03-线索 9 的 engine 侧防御缺口；06 已核实 hostd 执行面不可构造该组合，属纵深防御）。建议 Failed 分支前核对 current revision，矛盾时降级 Unresolved。 |
| 04a-F5 | engine.rs:369-384 + storage runtime.rs:562 vs 1291 | ordinal 为批内局部编号，两处排序键等价性依赖"排空 pending 才调模型"这一无文档无测试不变量；引入并行执行即静默乱序。建议全局递增 ordinal 或文档+守卫测试。 |
| 04a-F7 | engine.rs:180-193 | 续期器把任何 Err（含 SQLITE_BUSY 等瞬时故障）当永久失租退出，run 无谓放弃、空转至租约过期恢复（上下文作废、managed 多走恢复探查）。建议区分 Ok(false) 与 Err 计数（≥3 次才判失租）。 |
| 04a-F8 | engine.rs:300-338 + model.rs:68-73 | 模型错误重试双缺陷：retryable 3 次（1s+2s，60s 上限分支用不到）即 Failed——分钟级故障升终态；非 retryable（ContextLimit/Refused/Protocol）零退避立即连调烧钱（唯一软保护 consecutive_protocol_errors≥3）。建议指数升级+超限转 RetryScheduled/Blocked；非重试加短退避；ContextLimit 直接 Blocked 附裁剪指引。 |
| 04a-F10 | engine.rs:374-384 + report.rs:19-26 | 未知工具名 persist 时按 Privileged 处理——Owner 先审批一个系统不认识的工具，批准后才收 unknown_tool 失败（浪费审批往返且语义误导；大模型幻觉工具名常见）。建议 persist 阶段直接 fail 或 Deny 供模型自纠。 |
| 04a-F11 | src/tests/ | 引擎层四条生产路径零测试：budget_exhausted 转移（F2 滞留不可见）、续期器失租、call_id 冲突、恢复扫描竞态。建议各补最小集成测试（复用 in-memory DB + 脚本 provider 模式）。 |

#### application-tools（6）

| 编号 | 位置 | 问题与建议 |
|---|---|---|
| 04b-F3 | tools.rs:929-983 + hostd.rs:206-287 | process/sandbox 参数 application 侧零校验——schema 边界（args≤128/timeoutMs≤60000）仅是模型提示，hostd 是唯一执法点，测试替身同样不校验。建议客户端加同源轻校验（InvalidArguments 早退）。 |
| 04b-F4 | tools.rs:366-368/621/676 vs :495-497 | read/replace/prepare 全文件载入无大小预检（search 有），最坏 ~200MB 瞬时内存，operator 上调 workspace 预算至 10GB 后巨型文件可 OOM worker 并反复撞墙。建议 metadata.len() 预检+分页/流式。 |
| 04b-F5 | tools.rs:27/235 + model.rs:235-241 + hostd.rs:357-365 | descriptor.output_limit 硬编码 64KB 与真实预算 max_tool_output_bytes 双轨脱钩：非默认配置下模型被告知错误上限，该字段执行路径零消费（与 04b-F1 一并修）。建议构造时注入真实预算或删除字段。 |
| 04b-F6 | tools.rs:1182-1186/287/482/1160-1162 | Windows junction 或绕过 checked_join 的 is_symlink 判定（置信度低，需工作区先存在 junction，生产目标 Linux）；resolve_new（create 写路径）与 BFS 遍历无 canonicalize 兜底（resolve_existing 有）。建议 resolve_new 补父目录 canonical 校验 + Windows-only 单测。 |
| 04b-F7 | tools.rs:311/521 | list/search truncated 以 `len == limit` 判定——总数恰等上限时误报"还有更多"，诱导模型额外翻页。建议哨兵模式（多取一条再裁）。 |
| 04b-F9 | src/tests/workflow.rs:12-257 | 审批/失败路径零覆盖：owner_denied、approval_mismatch（防 TOCTOU 关键断言）、invalid_arguments 传播、ReadOnly 自动放行——现仅 3 条 happy-path。建议按四类各补一条（Deny 用例同时断言 run 未终结、模型收到 owner_denied）。 |

#### server（10）

| 编号 | 位置 | 问题与建议 |
|---|---|---|
| 05-F3 | http/auth.rs:70-85/65 | login 用户不存在跳过 Argon2——timing 侧信道可枚举用户名；限流键 `username:IP` 可被用户名变体分散。单 Owner 下收益低故 P3。建议 dummy verify + 键改 IP 为主。 |
| 05-F4 | http.rs:447-500 | WS 无服务器心跳、瞬时 DB 错误与致命错误同表现（裸断开无 Close）、session 吊销不终止已建立流。建议 30s Ping + Close 4001/retryable + 周期复验 session。 |
| 05-F6 | ssh_access.rs:357-362 | home_dir 依赖 HOME/USERPROFILE——Linux systemd 服务环境常缺 HOME，SSH 审计端点静默降级"home could not be determined"。建议 getpwuid fallback 或 SOLOOPS_SSH_HOME + 顶层告警字段。 |
| 05-F7 | http/auth.rs:89-100 | login 失败路径审计写失败把 401 变 500（"拒绝"被降级为"服务器错误"）。建议审计失败仅 log 仍返回 401。 |
| 05-F8 | openapi.yaml:563-580/664 + http.rs:95/440-443 | openapi 漂移集合：①RunStatus 含死状态 draft/paused（02-F4 同根，正式收录）；②ApprovalDecisionRequest 声明 additionalProperties:false 但 Rust 未开 deny_unknown_fields（声明严格实际宽容）；③/api/events 静默截断 200 无"还有更多"标志；④/livez、429、metrics 404 未收录。建议逐项修漂移。 |
| 05-F9 | http.rs:460-467 | WS 事件序列化失败仅 break 内层 for，after 不推进——每 tick 重拉同一批坏事件，error! 日志死循环刷屏。建议视同流损坏发 Close 4002 或计数熔断。 |
| 05-F10 | http.rs:74-78 | pub `build_router` 在 SMTP 配置畸形时 `.expect` panic（生产 bin 走 build_router_and_notifications 不受影响；tests/e2e_harness 使用它）。建议返回 Result 或文档标注 panic 条件。 |
| 05-F11 | e2e_harness.rs:255/289/389-396 | harness 桩两处 `.expect` panic（journal 缺对应行时 worker task 死亡，e2e 只会超时而非快速失败）；/__e2e/* 无认证（127.0.0.1 测试进程，可接受；feature 门控确认零生产泄漏面——04a 登记项的正式收录）。建议桩改返回 ProviderError。 |
| 05-F12 | http/config.rs:77/40 | secure_cookies 仅由 `SOLOOPS_ENV == "production"` 字符串等值决定——HTTPS 反代部署忘设则 cookie 无 Secure，公网降级链路 token 可明文泄露。建议独立 SOLOOPS_SECURE_COOKIES + 启动日志输出 cookie 属性。 |
| 05-F13 | http.rs:97/166-175 + config.rs:88 | /metrics 无认证且默认启用——公网暴露部署存在与活跃度。建议公网部署关闭或反代保护（metrics 关闭时 404 行为亦未入 openapi）。 |

#### hostd（6）

| 编号 | 位置 | 问题与建议 |
|---|---|---|
| 06-F4 | unix.rs:531-537 vs :649、sandbox.rs:101 | max_output_bytes 双规则不一致：managed 接受 1..=1023 且校验先于授权，process/sandbox 拒同区间（04b 已证 client 合法域 1024..=10MB ⊆ hostd 接受域，运行期不可失配——属协议面收口）。建议抽共享校验函数 + 统一校验/授权顺序。 |
| 06-F5 | storage runtime.rs:119-136 + unix.rs:157-234 | process/sandbox 授权是状态窥视非单次消费——同一审批在 call 的 running 窗口内可重复驱动执行（无 nonce/状态推进），与 managed 的 replay 幂等设计不对称；"批准一次=执行一次"不成立（无越权放大，参数被 digest 锁死）。建议授权成功后 CAS 推进执行态或加一次性 nonce。 |
| 06-F7 | deployment.rs:631-636/1069-1077 | bundle 写入与 caddy 片段发布均无 fsync——copy→rename 原子发布名字但数据未落盘，崩溃后 caddy 可读空/半配置（validate 窗口内页缓存仍可能通过）。建议 temp+sync_all+rename。 |
| 06-F8 | unix.rs:99-101 | socket 权限收紧存在 umask 窗口（bind 后才 chmod 0o660）；SO_PEERCRED 恒等检查使窗口内连接仍被拒，不可利用，属纵深防御。建议 bind 前 umask(0o077)；exists→remove→bind 竞态在文档标注。 |
| 06-F9 | sandbox.rs:415-419 + unix.rs:84-89 | cleanup_stale 串行首错即断——单个无法删除的容器使 hostd 启动失败退出，后续容器永不被清理也无告警（fail-closed 方向对但粒度过粗）。建议逐容器 best-effort + 失败清单进启动错误。 |
| 06-F10 | unix.rs:909-915 vs deployment.rs:1624-1630 | parse_bool 两份实现接受集分叉：`SOLOOPS_SANDBOX_ENABLED=yes` 生效而 `SOLOOPS_MANAGED_DEPLOY_ENABLED=yes` 直接启动报错。建议提取共享模块统一接受集。 |

#### webui-contracts（7）

| 编号 | 位置 | 问题与建议 |
|---|---|---|
| 07a-F5 | runs/[runId]/+page.svelte:100-104；tasks/+page.svelte:73-76 | cancelRun/logout 均无错误处理——失败 unhandled rejection、按钮零反馈（logout 失败停留原页）。建议补 try/catch（cancel 复用 error 横幅）。 |
| 07a-F6 | tasks/+page.svelte:105-107/231-243 | /tasks 默认态全页零 heading（侧栏 "Tasks" 是 span）——可访问性回归 + e2e 失败 1 的 UI 侧直接原因。建议侧栏升级 h1（一并修 a11y 并让旧断言语义成立）。 |
| 07a-F7 | login/+page.svelte:8 | username 预填 "owner"——把唯一账号名硬编码进 UI（改名即失配、向访客展示有效用户名）。建议空输入 + placeholder。 |
| 07a-F8 | settings/security/+page.svelte:16/21-31/99-102 | `refreshing` 死标志（唯一写入点是 finally 的 false）："Scanning…" 永不显示、Rescan 可连点并发。建议 load 区分首载/rescan。 |
| 07a-F9 | settings/security/+page.svelte:164/180/208/224 | each 块 key 用可空可重复 fingerprint（Windows 双角色文件天然撞键；Svelte keyed each 重复键渲染异常）。建议 key 改 `file + ":" + line`。 |
| 07a-F10 | settings/notifications/+page.svelte:235-239 | lastError 只显示"Last delivery failed"不展示原因（契约字段形同虚设，排障须翻后端日志）。建议 tooltip 展示截断原因 + lastAttemptAt。 |
| 07a-F12 | tasks/+page.svelte:36-44；runs/[runId]/+page.svelte:171-177 | loadRuntime/task 拉取吞掉全部错误（含 401）——会话过期静默降级（面板消失/标题回退，不跳登录），与页面级 401→goto 模式不一致。建议区分 401 跳登录。 |

#### webui-quality（15）

| 编号 | 位置 | 问题与建议 |
|---|---|---|
| 07b-F5 | ReportCard.svelte:17/19/23-25 | 表头恒渲染绿色对勾+成功色面板，failed/partial 报告呈"红徽章+绿对勾+绿面板"矛盾视觉（outcome 徽章本身着色正确）。建议图标/面板色随 outcome 映射。 |
| 07b-F6 | mock/router.ts:43-46/83-88/92、mock/index.ts:51-67 | mock 会话保真度四缺口：logout 后 cookie 仍有效（session 兜底返回 owner）、WS 升级零认证、登录无凭据校验、Set-Cookie 无 SameSite——认证失效流 mock 完全不可演练。建议逐项对齐（logout 清 cookie、upgrade 复用 hasSession、固定口令 401、SameSite=Strict）。 |
| 07b-F8 | mock/scenario.ts:250-308 | 重复 decide 宽容重放（真实后端 500，G1 的双击缺陷 mock 下永不暴露）、cancel 无终态守卫（掩盖 domain 状态机约束）、cancel 事件 from 硬编码 "planning"。建议先查 call/run 状态否则 409、from 取快照。 |
| 07b-F9 | mock/scenario.ts:134/160/220/269、state.ts:217 | mock 事件 payload 键名 4/8 与 04a 键名表不符（plan_updated 包 {plan}、call_completed 用 resultSummary、reported 包 {report}、created 多 runId）——EventStream 摘要 mock 下恒空；agent.message/tool.call_failed 从不产生；5 档 risk 只演示 workspace_write。建议逐字对齐键名 + 补场景分支。 |
| 07b-F10 | mock/scenario.ts:15/23-26/281/296 | ScenarioHandle.timer 全库零写入点，两处 clearTimeout 死代码——取消安全仅靠 finished 标志侥幸正确，"看似有防护实则没有"。建议 next() 回写 handle.timer 或删字段明确注释。 |
| 07b-F11 | mock/router.ts:195-201 | cancel 对不存在 run 返回 500 而非 404（cancelScenario 先于 404 检查执行，ensureHandle 直接 throw 被 catch-all 接住；正确 404 分支永远到不了）。建议先判存在再 cancel。 |
| 07b-F12 | vite.config.ts:4、mock/index.ts:4-7、router.ts:2-5、scenario.ts:2-4 | 12 处无扩展名导入/目录索引导入（正式收录基线 §1.5）——Vite native loader 未来默认化时构建破坏风险。建议统一补 .ts//index.ts（12 处机械改动）。 |
| 07b-F14 | mock/router.ts:83-88/111/161-166/228、index.ts:58 | mock 校验/错误码漂移：任意 title/goal 得 201、坏 JSON 得 500（真实 400/422）、after=NaN 静默空回放——前端 422/400 分支与回放契约 mock 不可验证（recipients 归一化行为存疑待与 server 核对）。建议补最小校验镜像。 |
| 07b-F15 | app.css:7/113 + ToolCallCard:86/96/102/149 | `text-muted-foreground/65` + 11px 组合对比度 ≈3.7:1 低于 WCAG AA 4.5:1，为全应用 meta 文本标配（时间戳/sha256/风险徽章），低视力用户系统性受损。建议提至 /80（≈5.0:1）或调亮 token。 |
| 07b-F16 | scripts/ui-lint.mjs:10-11/16/50 | 三处自身缺陷：IGNORE 前缀匹配无路径边界（ui-* 目录会被整目录跳过）、"never edited" 注释与 separator/switch 的本地修改矛盾（误导 shadcn 升级直接覆盖）、灰阶规则漏 stone 与渐变前缀。建议修边界/改注释/补规则。 |
| 07b-F17 | ApprovalBanner.svelte:16-22 + ToolCallCard.svelte:37-43 | RISK_TONES 风险配色两处逐字重复拷贝——视觉契约漂移即横幅与卡片同 call 颜色不一致。建议提取 lib/components/risk.ts 单一入口。 |
| 07b-F18 | ui/switch/switch.svelte:22/29 | 同一 Thumb 混用 bits-ui data-checked 与 radix data-[state=checked] 两套选择器，RTL 反向位移分支永死（当前无 RTL，属封装一致性）。建议统一 bits-ui 约定。 |
| 07b-F19 | 根 package.json:21-22 + mock/router.ts 零测试 | 前端测试覆盖缺口：mock 测试不进根 `test`（CI 只跑 Rust）、router 401/404/decision 链零测试、组件零测试、四大关键流（登录/建任务/时间线/审批）唯一防线是 2/2 全红 e2e。建议 test:mock 并入 test + router 最小 happy-path 测试 + e2e 修复后作集成防线。 |
| 07b-F20 | apps/web/package.json:16-28 + 根:30-35 | svelte/vite/kit/tailwindcss/eslint 全 "latest" 浮动——任何 lock 重建静默跳大版本（runes 语法/Vite native loader/tailwind v4 行为可无提示变化），叠加 F12 是最高频破坏源。建议改 ^ 区间（与 playwright/typescript 既有做法统一）。 |
| 07b-F21 | e2e/ui-audit.mjs:18/42/48-49 | 三处瑕疵：注释指错文件、`form button` last() 脆弱选择器、approve 静默跳过致截图相位失真无提示——审计产物不可信且不易察觉。建议修注释/精确选择器/跳过时 console.warn。 |

#### 跨模块合并组（3）

**P3-74 ★ G5：ssh_access 审计链路无大小上限（同根因，三层）** = 02-F7 + 05-F5 + 07a-F13
- 位置：domain/security.rs:17-50（类型无不变量声明）；server/http/ssh_access.rs:79（`read_to_string` 无上限）/:215-223（base64 无界解码）/:290-302（fields 可 4x 内存放大——三层各自分配峰值 ~2-4x 文件大小）；前端 security 页 :179-241/:255-266（entries 全量渲染，数万行产出十万级 DOM 节点页面卡死）
- 问题：authorized_keys 属本地文件（注入前提是本地已有写权限），端点仅 Owner 可达且 spawn_blocking 隔离——非远程可控输入，维持 P3；但一旦本地被入侵，GB 级文件可放大内存占用；前端无渲染上限组成"服务端不设限+前端不设防"。
- 建议：三层各设上限（metadata.len() 拒 10MB+、行 len 上限、fields 累计字节上限、前端每文件渲染上限 + show all）；domain 类型文档声明不变量。

**P3-75 ★ G6：恢复扫描容错缺失（同根因，两层）** = 03-F4 + 04a-F6
- 位置：storage runtime.rs:356-385/:1156-1200（CAS 冲突 InvalidTransition 经 `?` 上抛中断整个 for 循环）；engine.rs:115-118（三次扫描任一 Err 整轮 run_once 失败）+ bins/worker main.rs:53-56（固定 1s 重试无退避）
- 问题：多 worker 并发扫描同一 due run 时一方 CAS 不命中 → 错误上抛 → 本轮恢复批量推迟且日志误导（"非法转换"而非"被并发处理"）；更重的是单 run 的持久性扫描失败（库内状态损坏）会让 worker 陷入每秒一次失败日志、**所有排队 run 的 claim 被无限期饿死**——唯一症状是刷屏。03 判 P3 + 04a 承接为同族，合并后仍是 P3（触发依赖多 worker 或状态损坏，非默认部署形态）。
- 建议：恢复扫描逐 run 容错（per-item catch + 计数），worker 连续失败退避升级并输出显式告警。

**P3-76 ★ G7：运行产物文件侧无清理/保留契约（同根因，两处）** = 04b-F8 + 06-F6（03-线索 10 的文件侧补充）
- 位置：application engine.rs:135-138（每 run 创建 workspace_root/{run_id} 与 artifact_root/{run_id}，全 crate 无删除路径；truncate_output 每次截断存全文 artifact，最多 10MB/次）；hostd deployment.rs:622-648（每次 plan 生成 revisions bundle，无删除调用——superseded/rolled_back/failed revision 永久留存）/:1065-1077（caddy .tmp 崩溃残留无清理）；附带：artifact 写入不 fsync（崩溃窗口 evidence 引用悬空——05 已确认无下载端点，无泄露面，仅数据字段）
- 问题：workspace（默认预算 100MB/run）+ artifacts + bundle + DB events（03-线索 10：只增不减）四个维度均无保留策略/对账/告警——长期部署磁盘无界增长；这是所有"预算"里唯一完全没有对账的维度。
- 建议：run 终态后保留策略（最近 N 个 + 磁盘水位清理，与 DB 行清理同批设计）；reconcile 循环保守清扫孤儿 bundle 与 stale .tmp；artifact 写入补 fsync；doctor 增加 artifact_ref 悬空检查。

---

## d. 验证与测试状况

### 基线命令结果（00-baseline §1，本链全程未改源码，结论持续有效）

| 命令 | 结果 |
|---|---|
| `cargo fmt --all -- --check` | ✅ |
| `cargo clippy --workspace --all-targets -- -D warnings` | ✅（9 成员零警告） |
| `cargo test --workspace` | ✅ 104 passed / 2 ignored / 0 failed（10 目标 + 5 doc-test） |
| `bun run lint` | ✅ |
| `bun run --cwd apps/web check` | ✅ svelte-check 0/0 |
| `bun run test:mock` | ✅ 10 tests / 30 expects（07b 实跑复核一致） |
| `bun run --cwd apps/web build` | ✅ 6.81s adapter-static |
| lsp-workspace-diagnostics（rust-analyzer） | ✅ 0 errors / 0 warnings |
| `bun run test:e2e` | ❌ **2/2 失败**（详见下） |

### e2e 两条失败的最终定性（07a 为最终结论，04a/02/07b 为佐证链）

- **失败 1**（spec:10 `heading "Tasks"` 超时）：**测试断言漂移为主 + UI 可访问性回归为次（07a-F6），非功能性回归**。复合布局后 "Tasks" 是侧栏 `<span>`（tasks/+page.svelte:105-107），无任务选中时全页零 heading，断言在任何状态下不可能命中；快照证明应用渲染正常。同用例 :11 "Phase 3.1"（现 "3.1"）、:17 "Run timeline"（现无此 heading）同为漂移。
- **失败 2**（spec:47 15 秒未见 `managed.deploy.apply`，快照停 "No task selected"）：**非 create→run 链路功能回归**。后端链路经 04a（引擎/harness 逐环：worker 200ms 轮询、ScriptedProvider 脚本、plan=ReadOnly 自动放行、apply=Privileged 审批）、05（server 契约零漂移、WS 透传）、06（hostd action 字面量逐字一致）三方排除；07a 以快照法医闭合最终定性：**spec 登录后零等待抢跑（:38→:40）+ login 页迟到 goto 劫持（07a-F2）的复合竞态**——时间线：登录 POST 在途时点击 New task（仍在 /login，rail 唯一匹配侥幸通过）→ 创建 POST 无 cookie 401 → tasks/new 跳回 /login → login 响应到达后已卸载闭包执行 goto("/tasks") → 空任务列表 + "No task selected"，与快照逐元素吻合（Tasks 0 / No tasks / 0 active / 无错误横幅）。04a-F3 的"Create task 不再导航"假设被 07a 否证（tasks/new:25 有 goto，POST 响应含 latestRunId）。07b-F2 再确证：即便修好竞态，:48/:49/:51 三个断言文本已死（grep 0 命中），spec 需整体重写。
- 修复须两层缺一不可：spec（登录等待 + 选择器消歧 + 断言更新）与 app（goto 守卫）；建议顺序上先修 app 侧守卫再重写 spec，转绿后作为后续所有修复的集成验证工具。

### 覆盖审计：基线失败/线索认领表

| 基线条目 | 认领章节 | 结论 |
|---|---|---|
| e2e 失败 1（heading Tasks） | 07a（最终定性 + F6）；07b-F2（断言文本佐证） | ✅ 已定性：断言漂移 + a11y，非功能回归 |
| e2e 失败 2（managed.deploy.apply） | 02-F1（域层线索）→ 04a-F3（后端排除）→ 07a（最终定性：spec 竞态 + F2 劫持）+ 07b-F2（死断言） | ✅ 已定性：前端/spec 竞态，非功能回归 |
| 基线 §1.5 Vite native loader 12 处提示 | 07b-F12 | ✅ 正式收录（P3） |
| 基线 §6.2 hostd 两个 live 测试 ignore | 00（平台限制记录）+ 06（静态穷尽精读补偿） | ✅ 如实记录为环境限制，非缺陷 |
| 基线 §6.6/§1.5（同 12 处导入） | 07b-F12 | ✅ 同上 |
| 基线 §5 sqlx 事实纠正（非 rusqlite） | 03（取证 sqlx-core 0.8.6 源码） | ✅ 已按实际技术栈审查 |

**跳过文件汇总**（各章覆盖清单中标注"跳过"的条目）：

| 章节 | 跳过项 | 去向 | 净缺口 |
|---|---|---|---|
| 04a | crates/soloops-application/src/tools.rs | 04b 全文穷尽精读（1730 行分 5 段） | 无 |
| 07b | lib/components/EventStream.svelte | 07a 全文穷尽精读（183 行 + payload 键名表核对） | 无 |

其余全部范围内文件在对应章节标注"已审（穷尽）"。**结论：无因跳过产生的审查净缺口**；基线 §4 模块盘点表中列出的源码文件全部被某章节穷尽或点查覆盖（docs/ 目录文档仅作为证据锚点被引用，未作为独立审查对象——见附录 f 盲区）。

### 平台限制（如实记录，非缺陷）

1. hostd 面向 Linux（Unix Socket/特权部署），Windows 审查主机无法运行其 2 个 live 测试（需真实 Docker Compose/Caddy/预载镜像）；非 live 的 13 个 hostd 测试在 Windows 全部通过。
2. e2e 在审查链中未重跑（需浏览器环境的纪律豁免）；e2e 定性基于基线留存的 test-results 快照 + 代码反推闭环（证据强度高但未复现）。
3. `.env` 按纪律未读取（防密钥泄漏）；`.env.example` 确认无真实密钥（均为 `*_REF=env:…` 间接引用模式）。
4. 仓库纯本地（无 remote、2 个提交），审查对象为工作区状态（含 69 个未提交变更），无法与远端对照。
5. rustc 1.95.0 高于 workspace 声明 rust-version 1.88；edition 2024。

### 测试覆盖评价

- **Rust 侧（质量高、有系统性缺口）**：104 项测试对 storage（trigger 强制审计回滚、恢复矩阵、CAS/租约互斥、接管校验）与 application（恢复矩阵、截断、取消、重试键、审批接力）覆盖扎实，断言到文件内容与 durable 状态。四类缺口：① 引擎生产路径（budget_exhausted 转移、续期器失租、call_id 冲突、恢复扫描竞态——04a-F11）；② 审批失败路径（owner_denied、approval_mismatch、invalid_arguments——04b-F9）；③ 并发/多进程场景（除 2 个真实竞争测试外全 `:memory:` 单连接，03-F8）；④ blocked 分支与 decide 重复提交（03-F8/05）。恢复接力闭环有集成测试（04a 对 03 的"无集成测试"判断的修正已采纳）。
- **前端侧（近乎裸奔）**：仅 10 个 mock 内部单测（state/scenario 层）；router.ts（236 行，401/404/decision 全链）零测试、组件零测试；mock 与真实契约的保真度缺口（07b-F6/F8/F9/F13/F14）使"mock 模式下验证过的行为"在真实后端下不成立；唯一集成防线 e2e 2/2 全红。四大关键流（登录/建任务/时间线/审批）当前**没有任何一项被绿色测试覆盖**。

---

## e. 修复路线图

> 顺序原则：先安全（审批核心）→ 恢复回归防线（作为后续验证工具）→ 正确性（引擎/数据）→ 一致性与质量。每批完成后执行全套验证命令：
> `cargo fmt --all -- --check`；`cargo clippy --workspace --all-targets -- -D warnings`；`cargo test --workspace`；`bun run lint`；`bun run --cwd apps/web check`；`bun run test:mock`；`bun run --cwd apps/web build`；`bun run test:e2e`

**第 1 批 · 审批安全核心（先安全）** —— Top10 #1/#3/#4/#5/#6
1. 05-F1：ToolCallSummary 增加 argumentsJson（或专用审批详情端点）+ 前端展示同源参数文本 + openapi 同步。
2. G1：storage 冲突映射 ToolCallStateConflict（含 03-F12② 拆 ToolCallNotFound/RevisionNotFound）→ server 409/404 映射 + openapi 补错误声明 → 前端 ToolCallCard busy prop + decide catch。
3. 07b-F7：折叠态 `inert={!open}`。
4. 06-F1：validate_compose_source 拒绝 `$` 插值。
5. 04a-F2：预算检查移到 pending 排空后 + 滞留调用补记失败事件 + "approved call skipped" 审计。
- 伴随：05-F12（独立 SOLOOPS_SECURE_COOKIES，生产 cookie 安全一行配置）。
- 验证重点：`cargo test --workspace`（新增冲突映射测试）+ 审批路径单测（04b-F9 的 deny/mismatch 用例此时一并补，天然覆盖本批改动）。

**第 2 批 · 恢复回归防线** —— Top10 #9
1. 07a-F2：login goto 路由守卫（+ ?redirectTo）。
2. G2/07a-F11/07b-F2：e2e spec 重写（waitForURL、选择器消歧、按现 UI 断言；如实施 05-F1 则断言改为参数全文而非 sha256）。
3. 07a-F6：侧栏 Tasks 升级 h1（修 a11y 并让断言语义成立）。
- 验收：`bun run test:e2e` **2/2 转绿**——此后它作为第 1 批及后续所有修复的集成防线。

**第 3 批 · 引擎/数据正确性** —— Top10 #2/#7/#8/#10 + 同族
1. 04a-F1：call_id 作用域化/冲突回退 + INSERT OR IGNORE 改显式冲突上抛（journal callId 同源，04b 已核实现状锚点一致）。
2. 03-F1：claim 两函数改 begin_write()。
3. 03-F2：迁移 1/2 事务化 + IF NOT EXISTS；03-F3：create_owner 改 begin_write() + singleton 索引。
4. 02-F2：domain 共享长度契约（配合 02-F9 改 Cow）。
5. 06-F2：SystemCommandRunner 流式截断。
6. 04b-F1/F2：search/list 过 truncate_output + output_limit 与真实预算同源（04b-F5）+ managed 未知分类补全。
7. G4：默认路径锚定固定数据目录/要求绝对路径 + doctor 一致性核查（02-F16）；G6：恢复扫描逐 run 容错 + worker 退避。
- 验证重点：`cargo test --workspace`（04a-F11 的四条引擎测试、03-F8 的并发/blocked 测试此时一并补，用 concurrency.rs 模板）。

**第 4 批 · 一致性与质量** —— 剩余 P2/P3 按模块清理
1. G3：contracts.ts 导出单一 TERMINAL_STATUSES 收编 4 处 + mock 补 blocked（07b-F13）。
2. 07a-F3（refreshRuntime 序号防护）、07b-F3（time floor）、07a-F5/F12（错误处理与 401 分流）。
3. mock 保真度批次：07b-F6/F8/F9/F11/F14（认证失效流、状态守卫、事件键名、校验镜像）+ test:mock 并入根 test（07b-F19）。
4. 05-F8 openapi 漂移 + 02-F4 死状态决策（删除 draft/paused 需四方同步，或标注 reserved）+ 02-F5 round-trip 测试。
5. G7：workspace/artifact/bundle 保留策略 + fsync；G5：ssh_access 三层上限。
6. 其余 P3（bin 关机路径、协议文档、测试缺口、lint/a11y/版本锁定等）按模块顺次清理。
- 验证重点：全套命令 + `bun run test:e2e` 保持绿。

**长期项（不阻塞演进）**：04a-F1 的 PK 迁移 (run_id, call_id)、02-F8 事件 payload 强类型化、06-F5 授权单次消费语义、tool_calls 双份存储精简（03-F13）、前端组件测试体系。

---

## f. 附录

### f-1 复验调整记录（本节点对原发现的修正）

| # | 条目 | 调整 | 原因 |
|---|---|---|---|
| 1 | 07b 章末统计行（P2×3/P3×18） | 修正为 **P2×4/P3×17**（F3=time.ts 按正文 [P2] 标注计为 P2；P3 清单实际仅列 17 项） | 章节统计行与正文标记矛盾；按"正文 F 编号标注为准"原则修正，总条数 21 不变 |
| 2 | G1（审批重复提交链） | 补充细节：重复审批存在**两个不同的 500 窗口**——worker 接管前 INSERT 撞 tool_approvals 主键（裸 UNIQUE）、接管后 SELECT 落空返回 RunNotFound——均落入 error.rs catch-all 500 | 本节点回读 runtime.rs:633-634 发现 SELECT 同时要求 runs.status='waiting_for_approval'；修复时 409 与 404 需分别映射 |
| 3 | 04a-F1 | 补充细节：engine.rs:397-402 存在 stop_reason=length 的截断防御（made_progress=false），但其仅覆盖 length 场景 | 不改变原结论：正常批次被全量 IGNORE 时无进展守卫仍不触发 |
| 4 | 全部 P1（2/2）与 P2 合并组（18/18，覆盖原始 23/23） | **零降级、零删除**——逐条回读引用文件行核实，代码与描述一致、推理成立 | 证据优先原则；P2 复验率 100%（要求为 ≥50%） |

### f-2 审查覆盖盲区清单

1. **hostd live 行为未执行验证**：2 个 live 测试（真实 Compose/Caddy 部署与回滚）被 ignore，Windows 主机无法运行；06 章为纯静态穷尽精读——Docker daemon 实际交互、caddy reload 真实行为、健康探活链路在真实环境的表现未经动态验证。
2. **e2e 未复现**：全链未重跑 e2e（浏览器纪律豁免），失败定性基于基线快照 + 代码反推（法医级闭环但非复现）；修复阶段应最先重放。
3. **并发窗口未动态复现**：03-F1（池污染）、03-F3（双 Owner）、G6（扫描竞态）等机制经 sqlx registry 源码取证/代码推演确认，未构造真实并发复现。
4. **部署配置未验证**：`.env` 按纪律未读——真实密钥/配置组合下的行为（模型 API key、SMTP 凭据）未验证；README/docs 与代码的一致性仅抽查（doctor 与"必须一致配置"的偏差已由 02-F16 收录）。
5. **docs/ 未作为审查对象**：docs/ui-spec.md、docs/security.md、README 仅作为证据锚点引用，文档-代码一致性未系统审计（security.md 红线被 02 用于核对，属例外）。
6. **性能/负载零测试**：无任何压测；内存放大面（03-F13 snapshot、06-F2、04b-F4）与磁盘增长（G7）为静态评估。
7. **前端运行时行为静态推定**：组件零测试 + e2e 全红，Svelte keyed each 重复键后果（07a-F9）、竞态（07a-F3）等运行为源码级推定，未经浏览器实测。
8. **平台差异**：Windows 主机上审查 Linux 目标代码（hostd 全部、unix.rs 的 SO_PEERCRED/umask、04b-F6 junction 等），平台特定行为依赖 std/文档口径，未在目标平台执行。
9. **变更差量视角缺失**：审查对象为工作区终态（69 个未提交变更叠加后），未对 HEAD 与工作区做逐 diff 审查——"哪行是本分支新引入"未区分（对回归定位略有损失，不影响终态缺陷清单）。
10. **锁文件/依赖审计未做**：bun.lock 一致性、Rust 依赖漏洞面（cargo audit）未覆盖；07b-F20 仅覆盖"版本声明策略"维度。

### f-3 同根因合并组索引（G1–G7）

| 组 | 成员（章节-F） | 根因 | 主清单严重性 |
|---|---|---|---|
| G1 | 03-F12 + 05-F2 + 07a-F4 + 07b-F1 | 审批重复提交无幂等处理：storage 裸错误 → server 500 → 前端无防护无反馈 | P2 |
| G2 | 02-F1 + 04a-F3 + 07a-F11 + 07b-F2（强关联 07a-F2/07a-F6） | e2e 套件 2/2 失败：spec 竞态 + 断言全面漂移（同一调查链的四个章节切片） | P2 |
| G3 | 07a-F1 + 07b-F4 + 07b-F13 | domain is_terminal 含 Blocked，前端 4 处手写终端集合 + mock 数据全链不可见 | P2 |
| G4 | 02-F3 + 04a-F9 | 关键路径默认值按 CWD 相对解析 + 多进程配置零交叉校验 | P2 |
| G5 | 02-F7 + 05-F5 + 07a-F13 | ssh_access 审计链路无大小上限（domain 类型/服务端解析/前端渲染三层） | P3 |
| G6 | 03-F4 + 04a-F6 | 恢复扫描无逐 run 容错：CAS 冲突/瞬时错误中断整批并饿死 claim | P3 |
| G7 | 04b-F8 + 06-F6（+03-线索 10） | 运行产物（workspace/artifact/bundle/events）只增不删、无保留契约 | P3 |

---

**报告结论**：SoloOps 的架构骨架与特权边界设计扎实，Rust 侧工程质量高于平均水平；当前**可以安全地继续演进**，但"精确审批"这一产品核心承诺存在结构性缺口（05-F1 盲批为首要修复项），且 e2e 防线全红使后续修复缺乏集成验证手段——建议按路线图先修第 1 批审批安全五项，随即恢复 e2e，再进入引擎/数据正确性批次。全部 96 条主清单发现（原始 109 条，P1×2 / P2×18 / P3×76）中无一条阻塞架构方向，绝大多数修复为 S/M 工作量。
