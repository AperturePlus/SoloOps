# 04a — Application 引擎核心审查（soloops-application：engine/execution/report/model/hostd/config）

- **审查节点**: 全代码库深度审查 · 第 4a 节点（应用引擎核心）
- **审查对象**: 当前工作区状态（分支 `feature/agent-page-polish`，含未提交变更，同 00-baseline.md）
- **审查方式**: 范围内 7 个核心文件全部逐行精读（execution.rs 1056 行分 3 段读完；engine.rs 432 行分 2 段；model.rs 482 行分 2 段；config/hostd/report/lib.rs 穷尽），src/tests/ 以测试函数清单+断言抽查通读；每条发现的代码片段写入前均回读原文核实；跨层取证读取了 storage/runtime.rs 关键函数、bins/soloops-hostd 的 action 字面量、bins/soloops-worker/main.rs、server 示例 e2e_harness.rs（后两者仅为回答前序线索，其自身缺陷归属对应节点）。未修改任何文件。
- **验证证据**: `cargo test -p soloops-application --lib` 退出码 0（35 passed / 0 failed，与基线 §1.3 一致）；生产代码（engine.rs/execution.rs/report.rs/model.rs/hostd.rs/config.rs 非 test 部分）`unwrap/expect/panic!/切片` 扫描零命中（全部命中位于 `#[cfg(test)]` 模块）。
- **基线/章节关系**: 00-baseline.md §1.9 的 e2e 失败 2 已正式收录为 F3（基线来源注明）；02-domain-protocol.md 跨层线索 2/6/7 与 03-storage.md 跨层线索 1/3/6/7/9 已全部正式核查，见"前序线索核查答复"；02-F1/02-F11/03-F4/03-F15 等被引用处均标注章节编号。

**范围小结（供后续节点快速对齐）**:
- 引擎循环结构正确且测试覆盖扎实：`execute_run` = [租约/取消检查 → no-progress(3) → budget → 排空 pending 工具 → 调模型]，"排空 pending 才调模型"保证了 pending 集合永远只含单个批次的尾部（F5 的不变量）。
- 租约审批接力闭环成立：`run_once` 每轮先 `claim_approved_run` 再 `claim_next_run`（engine.rs:119-133），WaitingForApproval→Running 由引擎重新持租约执行，空窗 = worker 轮询间隔；workflow.rs:71-80、recovery.rs:82、managed.rs:151-161 有 decide→claim→run_once 的集成测试。02-线索 2 的疑虑解除。
- 恢复场景预算重放正确：`initialize_runtime` 用 `INSERT OR IGNORE`（storage runtime.rs:164），重 Claim 不重置 agent_sessions 的 usage/budget/plan——审批后/恢复后预算继续累计而非"重新装满"。
- 密钥泄漏面干净：SecretRef Debug/Display 均脱敏（config.rs:21-31）；解析错误只含 env 变量名（config.rs:95-97）；api key 仅入 Authorization header（model.rs:274）；错误消息（HTTP body 截 500 字符、reqwest 错误）不含密钥。02-线索（config 侧）无新增发现。
- 生产代码零 panic 面（见验证证据）；引擎不直接启动宿主机进程、不持 Docker Socket 的红线在本 crate 成立（hostd.rs 仅经 Unix Socket 协议桥接）。

---

## 前序线索核查答复（不占发现编号）

| 线索 | 答复 |
|---|---|
| 02-线索 2（审批后谁持租约） | **无缺口**。engine.rs:119-122 每轮先 `claim_approved_run(worker_id, lease_ms)`（03 已核实其做 WaitingForApproval→Running + 重设租约），随后才是 `claim_next_run`。审批通过到恢复执行的空窗 = 一个 worker 轮询周期（生产 worker 无任务时睡 `config.worker_poll_ms`，有任务则立即循环）。deny 同样可被 claim 终结记录（03 确认）。 |
| 02-线索 6 / 02-F11（ManagedDeploy 字符串契约） | **两端逐字一致，非 e2e 失败 2 的根因**。application/hostd.rs:114-131/158 期望 `result.action ∈ {"plan","status","apply","rollback"}`；hostd 侧产出点：`result_from_revision("plan",…)`（deployment.rs:652）、`"status"`（:665-671，含 `status:"not_deployed"` 变体）、`execute_change(…, "apply", …)`（:718→:953-954）、`"rollback"`（:736 replay / :768 execute_change）。全库 grep `"plan"`/`"apply"`/`"rollback"` 无其他 action 字面量。 |
| 02-线索 7 / 03-线索 3（事件 payload 键名） | 见文末"事件 payload 键名表"。补充引擎侧关键事实：**工具调用三类事件 payload 只含 `callId`（started）/`callId,summary,workspaceRevision`（completed）/`callId,category,summary`（failed），均不含工具名**——前端渲染 `managed.deploy.apply` 文本必须来自 run detail API（tool_calls 行）而非事件 payload，此点与 e2e 失败 2 直接相关（转 server/webui 节点）。 |
| 03-线索 7（WorkspaceWrite/Privileged 恢复接力） | **闭环成立且有集成测试**（与 03 的"无集成测试"判断相反，application 侧有覆盖）：recovery.rs:107-126（create 中断：recover_expired_runs=1 → recover_safe_runs=1 → run_once → 文件内容为期望值=Completed 路径）、:139/:168（Unsafe→Blocked 路径）、:258-315（replace 同矩阵）；managed.rs:199-237（中断 managed：重放 durable result 且 `executor.calls==0` 不重复执行）、:386-418（timeout → run Blocked 且 reason 含 "outcome is unknown"）。claim→initialize→execute_pending_tools 遇 Running 调用 → probe_interrupted_write（execution.rs:40-56）/ inspect_managed_change_operation（:177-201）的接力经真实 Database(:memory:) 验证。 |
| 审查要点 3（硬预算时点，03 无编号） | 预算检查在**循环顶部、支出之后**（engine.rs:216-221 先于 :223 pending、:240 call_model）——单次模型调用/单工具可越过限额（bounded overshoot，一次调用量级）。输出 token 侧有真正硬上限：`max_tokens = remaining_budget.max_output_tokens`（model.rs:268），提供方服务端强制。恢复重放正确（见范围小结）。详见 F2（越限批次的滞留问题）。 |
| 审查要点 4 末条（密钥入错误/日志） | 无泄漏路径，见范围小结与 config.rs 证据。 |

---

## 发现列表

## [P1] F1：tool_calls.call_id 为全局主键 + `INSERT OR IGNORE` 静默丢弃——模型复用 call_id 时工具调用无声消失，run 烧完预算才 Blocked
- 位置: crates/soloops-application/src/engine.rs:369-384、:356、:402；crates/soloops-storage/src/runtime.rs:441-452；crates/soloops-storage/migrations/0002_agent_runtime.sql:46-47
- 置信度: 高（机制逐行确证；触发的现实概率取决于提供方 id 策略——部分 OpenAI 兼容网关/本地推理服务器使用每请求局部短 id）
- 证据:
  ```rust
  // engine.rs:373-384 —— call_id 仅在为空时回退 UUID，重复/冲突无防御
  .enumerate()
  .map(|(ordinal, call)| {
      ...
      call_id: if call.call_id.is_empty() {
          Uuid::new_v4().to_string()
      } else {
          call.call_id.clone()
      },
      ordinal: ordinal as i64,
  // storage/runtime.rs:445 —— 冲突时静默忽略，无 rows_affected 检查
  "INSERT OR IGNORE INTO tool_calls
   (call_id, run_id, item_id, ordinal, name, arguments_json, ...
  // 0002_agent_runtime.sql:46-47 —— call_id 是跨 run 全局 PK
  CREATE TABLE tool_calls (
    call_id TEXT PRIMARY KEY NOT NULL,
  ```
- 问题: `tool_calls.call_id` 全库唯一（非 `(run_id, call_id)` 复合主键），而 `persist_model_response` 用 `INSERT OR IGNORE`。两类触发：① 同批内模型重复 id；② **跨 run/跨批次 id 复用**——行一旦存在（哪怕状态 completed）永远占住该 id，此后任何 run 的同 id 调用都被静默丢弃。被丢弃的调用没有 tool_result 行，模型下一轮看不到结果而重发同 id 调用 → 再次被忽略 → 死循环。且 `usage.tool_calls` 按**响应**计数（engine.rs:356 `saturating_add(response.tool_calls.len())`）、`made_progress = !calls.is_empty()`（engine.rs:402）在**落库前**判定——即使全部插入被 IGNORE，无进展守卫（consecutive_no_progress）也不会触发，run 以每轮一次模型调用的代价空转直到 model_turns 耗尽 → Blocked("budget_exhausted")。仓库内现成例证：e2e_harness.rs 的 ScriptedProvider 使用固定 call_id（"e2e-managed-plan"、:257 "e2e-managed-apply"），同一 harness 进程内出现第二个 managed 任务即触发（当前 e2e 仅一个 managed 任务故未炸）。
- 建议: ① engine 在构造 `NewToolCall` 时对重复/已存在 id 做归一化（如批内去重 + `format!("{run_id}:{call_id}")` 作用域化或检测到冲突时回退 UUID——注意需同时保证 journal 的 `tool_result.callId` 与模型可见 id 的一致性）；② storage 侧把 `INSERT OR IGNORE` 改为显式冲突检测并上抛（fail-loud）；③ 迁移上考虑 PK 改 `(run_id, call_id)`（需评估 tool_approvals/evidence FK 级联）。至少应保证"插入行数 ≠ 期望行数"时报错或记审计。

## [P2] F2：预算检查先于 pending 执行——把预算推过线的那个批次的工具调用（含已批准的）被永久滞留为 pending，run 直接 Blocked 且无任何工具级失败事件
- 位置: crates/soloops-application/src/engine.rs:216-221（预算检查）vs :223-238（pending 执行在后）；report.rs:29-35（budget_exhausted）
- 置信度: 高（机制确证；触发需批次恰好越过任一预算轴，长任务/审批等待场景现实可达）
- 证据:
  ```rust
  // engine.rs:216-238 —— 顺序：预算检查 → pending 工具 → 模型
  if budget_exhausted(&state.snapshot.budget, &state.snapshot.usage) {
      self.database
          .transition_run(run_id, RunStatus::Blocked, Some("budget_exhausted"))
          .await?;
      return Ok(());
  }
  let pending = self.database.pending_tool_calls(run_id).await?;
  if !pending.is_empty() {
  ```
- 问题: `usage.model_turns/tool_calls/input_tokens/output_tokens` 在 persist 响应时立即累加（engine.rs:346-356）。两个可观察后果：① 触发越限的批次（例如第 20 个 turn 发出的工具调用）persist 成功后，下一循环顶部即 Blocked，**该批全部调用永远停在 'pending'，没有任何 tool.call_failed/完成事件**，模型视角任务戛然而止；② 更糟的变体——若滞留调用是 RequireApproval 且已 `wait_for_tool_approval`（如第 20 个 turn 发出 managed.deploy.apply 并进入 WaitingForApproval），Owner 批准后 `claim_approved_run` 恢复执行 → execute_run 循环顶部预算检查立即再次命中 → **已获 Owner 精确批准的调用被静默跳过，run Blocked("budget_exhausted")**，审批石沉大海（approval 行仍在、调用永远 waiting_for_approval），且此后 server 审批端点因 run 非 waiting_for_approval 必然拒绝后续操作。对单用户控制平面而言这是"批准了却不执行也不告知"的误导性行为。
- 建议: ① 预算检查移到 pending 排空之后、call_model 之前（已有 pending 的批次先执行完再算账——工具执行本身另有 max_tool_duration/max_output_bytes 约束）；② 或至少在 Blocked 前对剩余 pending 调用补记 `fail_tool_call_before_start("budget_exhausted", …)`，让 Owner 与模型可见；③ 审批恢复路径上，若预算已尽应产生明确的"approved call skipped"事件/审计。

## [P2] F3：e2e 失败 2（15 秒未见 managed.deploy.apply）引擎侧排查结论——后端链路在 harness 下成立，证据指向 UI 未导航到 run 视图（基线收录）
- 位置: apps/web/e2e/happy-path.spec.ts:47（断言点）；本范围取证链：crates/soloops-server/examples/e2e_harness.rs:126-210、:347-374；crates/soloops-application/src/engine/execution.rs:100-129；crates/soloops-application/src/tools.rs:826-843、:891-919
- 置信度: 高（后端链路每环均已核实；UI 侧归因与基线快照一致，最终确认留给 webui 节点）
- 证据:
  ```text
  基线 §1.9: Locator: getByText('managed.deploy.apply') Timeout: 15000ms
  失败快照: 页面显示 "No task selected"（任务列表空态选择区）
  harness: worker 200ms 轮询 run_once（e2e_harness.rs:358-373），managed_deploy_enabled: true（:342）
  ScriptedProvider: goal 含 "managed deployment" → plan.update → managed.deploy.plan → managed.deploy.apply（:128-130, :229-263）
  tools.rs:826-843: managed.deploy.plan 风险 = ToolRisk::ReadOnly → policy Allow（无需审批，与规格单次审批假设一致）
  execution.rs:100-129: apply（Privileged）→ 审批 preview → wait_for_tool_approval → run WaitingForApproval
  ```
- 问题:（基线来源: 00-baseline.md §1.9；02-F1/线索 6/7 的链路疑点在本节点全部排除）逐环核实：① harness worker 以 200ms 间隔驱动 `engine.run_once`，ScriptedProvider/ScriptedHostExecutor 注册齐全，approval preview 所需的 revision 由 harness 执行器直写 storage；② 工具链 policy（plan=ReadOnly 自动放行、apply=Privileged 审批）与规格"单次 Approve 后 succeeded"完全一致；③ action 字面量两端逐字一致（见线索答复）；④ journal→provider 的 `result.revisionId` 键名闭环（tools.rs:899 写 "revisionId"，harness :260 读 `result.payload["result"]["revisionId"]`，storage tool_result item payload 为 `{"callId","result","evidenceId"}`，runtime.rs:867）；⑤ F1 的 call_id 冲突在本场景不成立（测试 1 在建任务前即失败，库中仅一个 run）。后端在任务创建后 ~1 秒内即会进入 WaitingForApproval 且 tool_calls 行含 `managed.deploy.apply`。失败快照的 "No task selected" 表明**复合布局重构后 "Create task" 不再导航/选中 run 视图**，与失败 1（heading "Tasks" 消失）同源的 UI 漂移——文本只存在于 run 详情的工具调用列表中（事件 payload 不含工具名，见线索答复）。
- 建议: webui 节点核对 tasks/new 提交后的路由/选中行为与旧规格的差量，并决定"改规格"还是"恢复导航"；server 节点顺带核对 create 响应体是否仍携带前端期望的 run 定位字段。**基线来源**: 00-baseline.md §1.9。

## [P3] F4：`managed_change_recovery` 信任 operation.error_category 判定 Failed，不交叉核对 revision 实际状态——"operation failed + revision active" 矛盾组合可被原样固化（03-线索 9 答复）
- 位置: crates/soloops-application/src/engine/execution.rs:868-887（判 Failed 的分支）；:427-441（block_managed_change：fail_tool_call + Blocked）
- 置信度: 高（engine 侧机制确证；触发前提是 hostd/storage 侧先产生 03-线索 9 描述的坏数据，hostd 节点需复核该前提的真实性）
- 证据:
  ```rust
  // execution.rs:878-886 —— error_category 存在即终判 Failed，无 revision 状态交叉验证
  if let Some(category) = operation.error_category {
      let summary = operation
          .result
          .as_ref()
          .and_then(|result| result.get("failure"))
          ...
      return Ok(ManagedChangeRecovery::Failed { category, summary });
  ```
- 问题: 03-线索 9 指出 `fail_managed_deployment_operation` 对已 active 的 revision 静默（UPDATE WHERE status='proposed' 0 行不报错）。engine 恢复判定在此之上**完全不查** `current_managed_deployment_revision(project_id)`：若 hostd 在"revision 已转 active → 之后才写失败"的竞态/缺陷下留下 `error_category=Some` 的 operation，engine 会把该 operation 报告为普通工具失败（fail_tool_call）并放行 run 继续——而宿主上 active 的正是该"失败"的 revision，`validate_operation_retry` 的 stale 检查也可能拒绝后续 apply。引擎是最后一道可发现矛盾的位置，目前不设防。
- 建议: 在 `managed_change_recovery` 的 Failed 分支前加一次当前 revision 交叉核对：若 `current(project).id == operation.revision_id && revision.status == "active"`，降级为 `Unresolved`（要求人工/hostd 对账）而非 Failed；顺带给 03-线索 9 的 storage 侧静默 UPDATE 补 rows_affected 审计。

## [P3] F5：tool_calls.ordinal 为批内局部编号（enumerate 0..n），两处排序键的等价性依赖"排空 pending 才调模型"这一无文档、无测试的隐式不变量（03-线索 6 答复）
- 位置: crates/soloops-application/src/engine.rs:369-384（ordinal 生成）；crates/soloops-storage/src/runtime.rs:562（`ORDER BY ordinal, created_at`）vs :1291（snapshot `ORDER BY created_at, ordinal`）
- 置信度: 高（当前无乱序；不变量的脆弱性为定性判断）
- 证据:
  ```rust
  // engine.rs:372-384 —— 每个模型响应内局部编号
  .tool_calls
  .iter()
  .enumerate()
  .map(|(ordinal, call)| { ... NewToolCall { ..., ordinal: ordinal as i64, ... } })
  ```
- 问题: 答复 03-线索 6：ordinal **确为批内局部编号**，跨批必然重复。当前不乱序的原因是 `execute_run` 的结构保证：pending 非空 → `execute_pending_tools`（要么全部处理完返回 false 继续排空，要么暂停/终结返回 true）→ 只有 pending 清空才会 `call_model`，因此 pending 集合永远只含**单个批次的尾部**，(ordinal, created_at) 与 (created_at, ordinal) 等价。但该不变量没有任何注释/测试钉住：一旦引入并行工具执行、"边执行边追加新调用"或恢复扫描语义变化，跨批 pending 共存即让 `ordinal 优先` 的 pending 查询静默乱序（批 2 的 ordinal-0 会插到批 1 的 ordinal-1 前），且由 F1 可知批间 created_at 也可能相同。
- 建议: 二选一：① persist 时生成 run 级全局递增 ordinal（`SELECT COALESCE(MAX(ordinal),-1)+1 FROM tool_calls WHERE run_id=?` 基线）；② 保持现状但把不变量写成 `execute_run` 的文档注释 + 一条"两批调用不可能同时 pending"的守卫断言/测试，并在 storage 排序键旁标注依赖关系。

## [P3] F6：worker 对恢复扫描错误只"log + 睡 1s 重试"，无退避升级/熔断——恢复扫描持续失败会饿死全部 run 的 claim（03-线索 1 答复，承接 03-F4）
- 位置: crates/soloops-application/src/engine.rs:116-118（扫描在 claim 之前，`?` 直接上抛）；bins/soloops-worker/src/main.rs:53-56
- 置信度: 高
- 证据:
  ```rust
  // engine.rs:115-118 —— 三次恢复扫描任一 Err 即整轮 run_once 失败，claim 不会发生
  pub async fn run_once(&self, worker_id: &str) -> Result<Option<String>, RuntimeError> {
      self.database.promote_due_retries().await?;
      self.database.recover_expired_runs().await?;
      self.database.recover_safe_runs().await?;
  // bins/soloops-worker/src/main.rs:53-56 —— 固定 1s 后重试
  Err(error) => {
      error!(worker_id, %error, "worker cycle failed");
      tokio::time::sleep(std::time::Duration::from_secs(1)).await;
  }
  ```
- 问题: 答复 03-线索 1：run_once 对恢复扫描错误是**逐层 `?` 上抛、由 worker 重试**（非熔断）。每次失败固定睡 1s，无指数退避、无失败计数隔离：一个持续 InvalidTransition 的 due run（03-F4 的多 worker 竞态是瞬时来源；库内状态损坏是持久来源）会让本 worker 陷入"每秒一次失败日志"，且由于扫描在 claim 之前，**所有排队 run 的执行被无限期饿死**，唯一症状是日志刷屏。
- 建议: ① 恢复扫描改为逐 run 容错（单 run 失败记日志/计数并继续处理其余 due run——promote_due_retries 已有 per-run 事务，可在 engine 层包 per-item catch）；② worker 对连续 N 次同类失败升级退避并输出"恢复通道降级"级别的显式告警；③ 该项与 03-F4 的多 worker 场景联动，修复时应一并处理。

## [P3] F7：租约续期器把任何 `renew_lease` 错误（含瞬时 DB 错误）一律视为永久失租并退出——瞬时抖动导致 run 无谓放弃，空转至租约过期恢复
- 位置: crates/soloops-application/src/engine.rs:180-193（spawn_lease_renewer）
- 置信度: 高（机制确证；实际触发频率取决于 SQLite busy/网络抖动，定性为低频）
- 证据:
  ```rust
  match database.renew_lease(&run_id, &worker_id, lease_ms).await {
      Ok(true) => {}
      Ok(false) | Err(_) => {
          lease_lost.store(true, Ordering::Release);
          break;
      }
  }
  ```
- 问题: `Ok(false)`（真失租）与 `Err(_)`（如 SQLITE_BUSY、连接池超时等瞬时故障）同分支处理。瞬时错误一次即令续期器永久退出，`lease_lost` 置位后 execute_run 在最近的检查点（循环顶部或工具 250ms tick）返回——此时租约**其实仍然有效**（默认还有 20-30s），run 被本 worker 主动放弃却无人接手，要等租约自然过期再经 recover_expired_runs → 重 claim 才继续，单次抖动平均浪费约 lease_ms 的执行延迟，且中途的工具/模型调用上下文全部作废重来（对 managed 调用还会多走一遍恢复探查）。
- 建议: 区分 `Ok(false)`（立即失租）与 `Err`（连续失败计数，如 ≥3 次或超过 lease 剩余时间的 2/3 才判失租）；至少把 `Err` 与 `Ok(false)` 分开打点日志便于排障。

## [P3] F8：模型错误重试策略双缺陷——retryable 错误 3 次尝试（总耐心 ~7s）即判 run Failed；非 retryable 错误零退避立即连调
- 位置: crates/soloops-application/src/engine.rs:300-338（分类处理）；model.rs:68-73（retryable = RateLimited|Timeout|Server|Transport）
- 置信度: 高
- 证据:
  ```rust
  if error.category.retryable() && attempt < 3 {
      let fallback = 1u64 << (attempt.saturating_sub(1) as u32);   // 1s, 2s
      let delay = error.retry_after.unwrap_or(Duration::from_secs(fallback)).min(Duration::from_secs(60));
      ... schedule_model_retry(...).await?;
      return Ok(true);
  }
  if error.category == ProviderErrorCategory::Authentication
      || (error.category.retryable() && attempt >= 3)
  { ... transition_run(run_id, RunStatus::Failed, ...) ... }
  ...
  if errors >= 3 { ... Blocked ... }
  return Ok(false);   // 非重试错误：execute_run 立即再次 call_model，无任何延迟
  ```
- 问题: ① 提供方持续限流/短暂宕机（分钟级故障很常见）时，3 次尝试（退避 1s+2s，`min(60s)` 的上限分支永远用不到）后 run 永久 **Failed**——瞬时故障被升级为终态，Owner 必须手动重跑；对"硬预算"系统而言 Failed 也不产生可续跑的状态。② 非重试类别（Protocol 畸形响应 / Refused 拒答 / ContextLimit 超长）走 `record_model_error + return Ok(false)`：execute_run 循环**立即**再次 call_model，零退避——ContextLimit 尤其荒谬（prompt 不会变短，三次注定相同失败的付费调用在毫秒级间隔内烧完）。唯一的软保护是 `consecutive_protocol_errors >= 3 → Blocked`（runtime.rs:313-322 递增、engine.rs:332 判定）。
- 建议: ① retryable 攻顶路径改为指数升级（1s/2s/4s→…→60s，尝试数或总时长上限配置化），超限降级为 RetryScheduled 循环或 Blocked 而非 Failed；② 非重试错误在两轮之间至少插入短退避；③ ContextLimit 单独处理：直接判 Blocked 并附"裁剪 journal"的指引（journal 已有 256KB 投影上限，见 report.rs + engine.rs:277-280，但仍可能超小预算）。

## [P3] F9：workspace_root / artifact_root 默认按 CWD 相对解析（`var/workspaces`、`var/artifacts`）——与 02-F3 同族但独立存在：worker 换目录启动即预算统计与工作区数据双漂移
- 位置: crates/soloops-application/src/config.rs:153、:184-185、:225-232（absolute()）
- 置信度: 高
- 证据:
  ```rust
  // config.rs:153 + 184-185 —— 以启动时 CWD 为基准
  let current = std::env::current_dir().context("failed to resolve current directory")?;
  ...
  workspace_root: absolute(&current, &env("SOLOOPS_WORKSPACE_ROOT", "var/workspaces")),
  artifact_root: absolute(&current, &env("SOLOOPS_ARTIFACT_ROOT", "var/artifacts")),
  ```
- 问题: 02-F3 已收录数据库路径的同类问题（server/http/config.rs，归 server 节点）；本 crate 的 RuntimeConfig 独立存在同一模式且后果不同：workspace 目录承载 `max_workspace_bytes` 预算统计（tools 执行期扫描）与 run 工作区内容——worker 以不同 CWD 启动（systemd WorkingDirectory 与手工 cargo run 混用）时会静默指向另一棵目录树：预算按错误目录计算（虚高或虚低），中断恢复 probe_interrupted_write 也将在错误目录下判定 "unexpected content" → Unsafe → Blocked。api/worker/hostd 三方对 workspace 路径的一致性纯靠 .env 约定。
- 建议: 与 02-F3 一并统一：默认值锚定固定数据目录或要求绝对路径，启动时校验（甚至记录指纹到 DB 供 doctor 比对）。

## [P3] F10：未知工具名在 persist 时按 Privileged 处理 → Owner 会先收到"未知工具"的审批请求，批准后才得到 unknown_tool 失败——浪费一次审批往返且语义误导
- 位置: crates/soloops-application/src/engine.rs:374-384（persist 时 risk 兜底）；engine/execution.rs:133-144（unknown_tool 失败）；engine/report.rs:19-26（policy_for）
- 置信度: 高（机制确证；触发需模型幻觉出不存在的工具名——大模型场景常见）
- 证据:
  ```rust
  // engine.rs:374-377 —— registry 查不到时默认 Privileged
  let descriptor = self.registry.get(&call.name).map(|tool| tool.descriptor());
  let risk = descriptor
      .as_ref()
      .map_or(ToolRisk::Privileged, |descriptor| descriptor.risk);
  // report.rs:22-23 —— Privileged → RequireApproval
  ToolRisk::WorkspaceWrite | ToolRisk::Process | ToolRisk::Privileged => {
      PolicyDecision::RequireApproval
  ```
- 问题: 模型输出不存在的工具名时，调用以 risk=Privileged/policy=require_approval 落库；execute_pending_tools 走 RequireApproval 分支（对非 managed 名字无 preview），`wait_for_tool_approval` 让 run 进入 WaitingForApproval，Owner 在审批界面看到一个无名实义的工具调用；批准后 registry.get 返回 None → `fail_tool_call("unknown_tool", …)`。安全方向正确（宁可多审批），但对 Owner 呈现为"请求批准一个系统根本不认识的工具"，且整轮审批等待完全无意义。
- 建议: persist 阶段（engine.rs 构造 calls 时）即对 registry 查不到的名字标记为"待失败"——要么直接 `fail_tool_call_before_start("unknown_tool")` 不进入审批流，要么把 policy 标为 Deny 使其以 model 可见错误立即失败（模型下一轮可自我纠正），避免消耗 Owner 注意力。

## [P3] F11：测试覆盖缺口——budget_exhausted 转移、租约续期器失租、call_id 冲突、恢复扫描并发竞态四条生产路径零测试
- 位置: crates/soloops-application/src/tests/（engine.rs 227 行 / providers.rs 441 / recovery.rs 318 / managed.rs 433 / workflow.rs 257）
- 置信度: 高（全量测试函数清单 + 关键断言 grep 核实）
- 证据:
  ```text
  tests 清单（15 个集成测试 + providers.rs 脚本化桩）：
  engine.rs: no-progress×3→Blocked、模型调用期取消不重排、重试复用 logical key/attempt key 唯一、length 截断调用不执行
  recovery.rs: 中断 create/replace 的 Completed/RetrySafe/Unsafe 矩阵（含 legacy 两例）
  managed.rs: 中断 managed 重放 durable result 且不重复执行（executor.calls==0）、首次 apply 超时 → Blocked("outcome is unknown")
  workflow.rs: 审批→workspace 证据→finish 成功路径、process.exec 注入执行器、sandbox 注册与精确审批
  grep "budget|lease" 于 tests/：仅见 BudgetSnapshot::default()/lease_ms 配置字面量，无 exhaustion/renew 断言
  ```
- 问题: 恢复矩阵、截断、取消、重试键、审批接力的覆盖质量**高**（真实 Database(:memory:) + 脚本 provider，断言到文件内容与 durable 状态）；但四条本节点发现的生产路径无任何测试：① budget_exhausted → Blocked 的转移（F2 的滞留行为因此不可见）；② spawn_lease_renewer 的失租/错误路径（recovery 测试全部以手工 recover_expired_runs 模拟，未驱动续期器）；③ call_id 冲突静默丢弃（F1）；④ 多 worker 恢复扫描竞态（03-F4/F6）。e2e_harness.rs:255/289 还有两处 `.expect(...)` 在 provider 桩内——测试基础设施层面的 panic 面（属 server 节点范围，此处仅登记）。
- 建议: 按上述四条各补一个最小集成测试（in-memory DB + 脚本 provider 的既有模式可直接复用）；provider 桩的 expect 改为返回 ProviderError。

---

## 事件 payload 键名表（engine 触发点 → storage 写入 → 消费端核对锚点）

事件流（events 表 / WS 回放，payload 全部 camelCase；键名由 storage 写入，engine 决定触发时机与取值来源）：

| 事件类型 | payload 键 | engine 触发点（application crate） | 取值来源 |
|---|---|---|---|
| run.created | `taskId`, `status` | （server 创建任务路径触发，非 engine） | tasks.rs:40 |
| run.status_changed | `from`, `to`, `workerId`, `reason` | initialize_runtime/engine.rs:207-220（Blocked×2）、execution.rs:51/438（Blocked）、engine.rs:324（Failed）、wait_for_tool_approval→WaitingForApproval、schedule_model_retry→RetryScheduled、execute_finish→Verifying/:632→Running | storage transition_run_on_connection 统一写入 |
| agent.plan_updated | `summary`, `steps` | execution.rs:490-526 execute_plan_update（save_plan） | 模型 plan.update 参数（经 100 步/唯一 id 校验） |
| agent.message | `preview` | engine.rs:359-368 → persist_model_response（runtime.rs:425-438） | assistant 文本前 500 字符 |
| tool.call_started | `callId` | execution.rs:172-175 start_tool_call_with_recovery | **不含工具名/参数**——前端须配 run detail API |
| tool.call_completed | `callId`, `summary`, `workspaceRevision` | execution.rs:179-181/229-232/636-645 → finish_tool_call（runtime.rs:891-898） | 工具 summary + workspace 版本号 |
| tool.call_failed | `callId`, `category`, `summary` | execution.rs:60-67/75-84/113-124/134-143/150-158/185-187/245-248/269-285/278-286/503-511/537-545/628-630 → fail_tool_call(_before_start) | ToolError.category + 摘要 |
| run.reported | `outcome`, `summary` | execution.rs:636-658 → save_final_report | outcome 恒为 "succeeded"（execute_finish:587-589 强制） |

**不存在 managed.deploy.* 事件类型**——domain/event.rs:7-24 仅 8 种 EventType；受管部署只产生上表中的工具事件 + tool_approvals 审批预览。e2e 断言的 `managed.deploy.apply` 文本只能来自 run detail 的 tool_calls 行（`name` 列）。

agent_items（journal；同时是模型上下文与 run 页时间线数据源）：

| kind | payload 键 | 写入点 |
|---|---|---|
| model_response | `stopReason`, `toolCallIds[]` | engine.rs:359-362 → persist_model_response |
| assistant_message | `text` | engine.rs:363-368 |
| system_feedback | `error.category`, `error.message`(≤500 字符) | engine.rs:320-331/330 → record_model_error（runtime.rs:302-312） |
| tool_result | `callId`, `result`(工具输出 JSON), `evidenceId`(可空) | finish_tool_call（runtime.rs:859-872） |

审批预览（tool_approvals 表 → 前端 ApprovalBanner；engine 写入，execution.rs:314-321/356-363）：

| 子命令 | preview 键 |
|---|---|
| managed.deploy.apply | `action`="apply", `projectId`, `proposalId`, `proposalSha256`, `previousRevisionId`, `changes` |
| managed.deploy.rollback | `action`="rollback", `projectId`, `expectedCurrentRevisionId`, `targetRevisionId`, `targetProposalSha256`, `changes` |

受管部署工具输出 value（工具结果/journal 的 `result` 内容；生成于 tools.rs:891-919，属 4b 范围、此处仅列键名供消费端核对）：`action`, `projectId`, `status`, `revisionId`, `previousRevisionId`, `proposalSha256`, `preview`。

审计条目（audit 表，非事件流）：`agent.start` context=`{"provider","model"}`（initialize_runtime，runtime.rs:196-211）；`tool.policy` require_approval context=`{"runId"}`（wait_for_tool_approval，runtime.rs:602-615）。

---

## 覆盖清单

| 文件 | 行数 | 状态 |
|---|---|---|
| crates/soloops-application/src/lib.rs | 19 | 已审（穷尽） |
| crates/soloops-application/src/config.rs | 315 | 已审（穷尽；SecretRef/SecretValue 脱敏与 zeroize、环境变量覆盖、legacy 兼容、F9；无密钥泄漏路径） |
| crates/soloops-application/src/engine.rs | 432 | 已审（穷尽；run_once/execute_run/call_model/续期器逐行；F1/F2/F5/F7/F8/F10 触发点） |
| crates/soloops-application/src/engine/execution.rs | 1056 | 已审（穷尽，分 3 段；审批流/恢复探查/managed 恢复/execute_finish 逐行；F4 + 线索 7/9 答复；尾部 941-1056 为内嵌单测） |
| crates/soloops-application/src/engine/report.rs | 105 | 已审（穷尽；policy_for/budget_exhausted/remaining_budget/project_journal/render_report；无新发现，budget 语义并入 F2） |
| crates/soloops-application/src/model.rs | 482 | 已审（穷尽；ProviderError 分类/HTTP 映射/解析健壮性/max_tokens=remaining/密钥路径；缺陷并入 F8；非流式确认——无 stream 字段，畸形 JSON 归 Protocol 类） |
| crates/soloops-application/src/hostd.rs | 373 | 已审（穷尽；错误码映射完整、协议版本+request_id 校验、UnixStream 无读超时但被 execute_tool_cancellable 的 deadline 兜住；字面量比对见线索答复；cfg(not(unix)) 返回 PolicyDenied 符合 Windows 事实） |
| crates/soloops-application/src/tests/（engine/providers/recovery/managed/workflow） | 1676 | 通读评估（测试函数全清单 + 关键断言抽查；覆盖质量结论见 F11） |
| crates/soloops-application/src/tools.rs | — | **跳过（按分工归 4b 节点）**；仅为 e2e-2 链路取证读取了 :826-843（plan 风险 ReadOnly）与 :891-919（payload 键名），未写其发现 |

**发现统计**: P0 × 0；P1 × 1（F1 call_id 全局 PK 静默丢弃）；P2 × 2（F2 预算时点滞留已批准调用、F3 e2e 失败 2 排查结论收录）；P3 × 8（F4 managed 恢复不交叉核对 revision、F5 ordinal 批内局部编号、F6 worker 恢复扫描饿死、F7 续期器 Err 即失租、F8 模型错误重试策略、F9 CWD 相对根、F10 未知工具先审批后失败、F11 测试覆盖缺口）。合计 11 条，编号连续无弃用。

**审查方法补充**: 引擎入口调用链经 run_once→execute_run→execute_pending_tools/call_model 人工全路径核对（rust-analyzer 零诊断基线下未重复跑 workspace 诊断，按节点纪律仅用定点读取）；storage/runtime.rs 的 initialize_runtime、persist_model_response、pending_tool_calls、wait_for_tool_approval、record_model_error、schedule_model_retry、promote_due_retries 为回答引擎语义问题所做的取证性读取（其自身缺陷归 03 章节与 storage 节点）；bins/soloops-hostd 仅做 action 字面量比对取证；bins/soloops-worker/main.rs（62 行穷尽）回答 03-线索 1；server 示例 e2e_harness.rs 为 e2e-2 取证通读，其 `.expect()` 桩问题登记给 server 节点；未修改任何源码；`cargo test -p soloops-application --lib` 35/35 通过（0.46s，与基线一致）。
