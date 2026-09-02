# Pi Agent 设计研究 × SoloOps 改进建议

> 研究对象：`参考/pi-main`（Pi Agent Harness，earendil-works）。核心包：`packages/agent`（运行时）、`packages/ai`（多 Provider 统一 API）、`packages/coding-agent`（编码 agent 与工具集）、`packages/protocol` + `session-backends`（会话协议与持久化）。
> 对照对象：本仓库 SoloOps（Rust 单 Owner Agent 控制平面）。

---

## 0. TL;DR 优先级矩阵

| # | 改进项 | 维度 | 优先级 | 预估工作量 |
|---|--------|------|--------|-----------|
| 1 | 工具错误改为"结果"而非"异常"（非零退出回传 stdout/stderr） | 工具/Loop | **P0** | 小 |
| 2 | 接通 `artifactRef`：截断输出全文落盘 | 工具 | **P0** | 小 |
| 3 | 上下文协议重构：原生消息序列替代单条 user JSON | Loop | **P0** | 中 |
| 4 | `expect()` 生产路径清理 | 边界 | **P0** | 小 |
| 5 | `elapsed_ms` 语义修复（排除审批等待时间） | 状态机 | **P0** | 小 |
| 6 | stopReason=length 时废弃该消息全部 tool call | Loop | P1 | 小 |
| 7 | 上下文压缩（compaction）替代固定 60 条截断 | Loop | P1 | 中 |
| 8 | Steering：Owner 运行中补充指令 | Loop/边界 | P1 | 中 |
| 9 | ControlTool 泛化（消灭名字 if 特判） | 工具/Loop | P1 | 小 |
| 10 | Worker 超时联动 hostd kill | 边界 | P1 | 中 |
| 11 | 取消通知机制（替代 250ms DB 轮询） | 边界 | P1 | 中 |
| 12 | 只读工具集（构造性边界）+ 工具并行执行 | 工具 | P2 | 中 |
| 13 | Journal 升级为 (id, parentId) 追加树，支持 fork | 状态机 | P2 | 大 |
| 14 | Operations 接口：统一 exec 工具 + host/sandbox 后端 | 工具 | P2 | 大 |

---

## 1. 先说结论：我们有哪些不用学 pi 的

pi 明确**不做权限系统**（README："No Built-in Sandbox"，隔离靠外部容器化）。SoloOps 的以下资产是 pi 没有的，改进时应保留并继续深耕：

- **硬预算五维上限**（turns/tool_calls/tokens/elapsed/duration）+ 指数退避重试分类；
- **租约续期与崩溃恢复**（lease renewer、`needs_recovery`、workspace 写三态判定 `probe_interrupted_write`）；
- **审批参数哈希绑定**（`arguments_sha256`，批准后参数不符即拒）；
- **hostd 特权边界**（Unix socket + `peer_cred` UID 白名单 + hostd 直查 DB 验证 running 状态才执行）；
- **事件模型**：pi 协议的原则是"快照为权威，增量仅供展示"——我们的 REST 补拉 + WS 时间线已经符合这一原则。

以下建议全部是在**不削弱这些边界**的前提下 borrowed from pi。

---

## 2. Agent Loop 设计

### 2.1 上下文协议重构（最大单项差距，P0）

**现状**（`crates/soloops-application/src/model.rs:251-259`）：整个 `PromptContext`（含 journal 投影）序列化为**单条 user JSON 消息**，且用 `expect("prompt context is serializable")`。模型看到的不是对话，而是一个不断膨胀的 JSON 大对象——这会显著伤害多轮工具调用的质量（模型难以区分"我上一轮做了什么"和"任务描述"），也浪费 token。

**pi 的做法**（`packages/agent` / `packages/ai/src/types.ts`）：
- 消息是三角色一等公民：`UserMessage | AssistantMessage | ToolResultMessage`；
- **工具结果是独立消息**（`ToolResultMessage { toolCallId, toolName, content, isError, timestamp }`），不是嵌在 user 消息里；
- 统一内容块 `TextContent | ThinkingContent | ImageContent | ToolCall`，在 LLM 边界由 `convertToLlm` 转换为各 provider 格式。

**建议**：把 `agent_items` journal 直接投影为 OpenAI tool calling 消息序列：
```
system  = 系统提示（由工具 promptSnippet 动态拼装，见 §3.8）
user    = 初始任务
assistant = 模型回复（含 tool_calls 数组）
tool    = 每个 tool_call_id 对应的 tool result（含 isError 标志）
```
我们的 `agent_items` 已经有 `kind`（model_response/assistant_message/tool_result）和 per-run sequence，天然支持这种投影——改的是 `PromptContext` 组装逻辑，不动存储 schema。顺带消灭 `expect()`（见 §5.4）。

### 2.2 工具错误 = 结果，不是异常（P0）

**pi 的关键设计**（`packages/agent/src/agent-loop.ts`）：工具 `execute` 抛出的任何异常都被 catch 并转成 `isError: true` 的正常 ToolResultMessage 回传模型，**循环不中断**——模型看到错误后自我纠正（改参数、换路径、重试）。这是 agent 自主性的核心机制。

**现状**：`fail_tool_call` 确实会写 `tool_result` journal item（`storage/runtime.rs:963-971`，含 error category/message），模型能看到错误——这个通道是好的。但有两个洞：

1. **process.exec / sandbox.exec 非零退出走 `Err(ToolError::Execution(summary))`**（`tools.rs:931/976`），模型只拿到 `"{program} exited with 1"` 一句话，**stdout/stderr 全部丢失**。对编码类任务，编译错误输出恰恰是模型最需要的信息。命令失败是正常业务结果，不是工具故障。
   **建议**：非零退出返回正常 `ToolOutput`（`exitCode + output`），只有 spawn 失败、socket 断连等基础设施问题才走 `ToolError`。
2. `ToolError` 8 分类可以增加"是否回传给模型 vs 是否终止 run"的语义维度：pi 的对应物是 `{block, reason, terminate}` 三态。`PolicyDenied`/`RecoveryRequired` 应终止或转审批，`InvalidArguments`/`Execution` 应作为 isError 结果让模型重试。

### 2.3 输出截断：双限制 + 保尾 + 落盘（P0，字段都是现成的）

**现状**：`truncated` 恒 `false`、`artifactRef` 恒 `None`（`tools.rs:909-994`）——字段定义了但从没接通。

**pi 的做法**（`coding-agent/src/core/tools/truncate.ts`）：行数（2000）/字节（50KB）双限制先到为准、**永不切半行**、保留尾部（错误信息通常在尾部）、`TruncationResult { totalLines, outputLines, truncatedBy }` 放 details 给 UI、全文写临时文件并在结果里给路径、文本里提示模型"用 offset/limit 继续读"。

**建议**：实现 `truncate_output()` 工具函数，超限时全文写入 evidence artifact（`NewEvidence`/`artifact_ref` 机制已存在，直接复用），`truncated: true` + 告诉模型 artifact 引用。

### 2.4 stopReason 处理（P1）

pi 有个精细的防御：assistant 消息因 `length` 截断时，`failToolCallsFromTruncatedMessage()` 把该消息的**所有 tool call 标记为错误**——因为流式 JSON salvage 可能产生"能通过校验但不完整"的参数，一律不执行。我们的模型错误处理覆盖了 retryable/auth/非 retryable，但未见对 length 截断消息中不完整 tool call 的防御。建议在 `persist_model_response` 增加 finish_reason=length 分支：该批 tool_calls 全部 fail（isError 结果），下一轮让模型重新输出。

### 2.5 上下文压缩 compaction（P1）

**现状**：journal 固定取最近 60 条 + 字节上限（`engine.rs:277-289`），旧上下文直接丢——长任务会"失忆"。

**pi 的做法**：`shouldCompact(contextTokens, contextWindow)`：token 超过 `window - reserveTokens` 时触发；`findCutPoint` 保留近期尾部（keepRecent 20000 tokens）；旧历史由 LLM 摘要为 `CompactionEntry { summary, retainedTail, tokensBefore }` 持久化。token 估算优先用 provider 返回的真实 usage。

**建议**：journal 投影改为 token-aware（用累计 usage 而非字节×4 估算），超阈值时把被裁掉的头部历史生成摘要 item（kind=compaction_summary），作为 system 附加段。这与 §2.1 的消息序列重构顺路。

### 2.6 Steering：运行中补充指令（P1）

pi 区分两条队列：**steering**（工具批执行完后注入，不打断当前轮）与 **followUp**（agent 停止后才注入，相当于自动续跑）。

SoloOps 是 headless 队列模型，Owner 目前只能干等或取消。建议加 `run_messages` 表（run_id, sequence, content, status），Owner 经 API 下发 steer 消息，engine 在**工具批执行完毕、下一次模型调用前**注入为 user item——不打断进行中的工具，天然与取消检查点同位。followUp 语义可以映射为"run 成功后自动派生续跑 run"。这是单 Owner 产品里"人还在环上"的关键体验。

### 2.7 循环终止语义

pi：工具结果带 `terminate: true` 且整批都是 → 批终止；`shouldStopAfterTurn` 回调优雅停止。我们对应物 `run.finish`（ControlTool）语义相同，但实现是名字 if 特判（见 §3.6）。

---

## 3. 工具设计

### 3.1 校验与执行分离（P2）

pi 把 schema 校验（typebox `Value.Check` + 详尽错误信息含原始 args）放在 `prepareToolCall` 阶段，校验失败直接转 immediate error result，不进 execute。我们 `finish_tool_call` 前的参数检查散在执行体内。建议 `Tool` trait 拆出 `validate(&self, args) -> Result<(), String>`，错误信息带原始 JSON 方便模型自纠。pi 还有 `prepareArguments` 兼容垫片（容忍模型把 edits 输出成 JSON 字符串等劣质输出）——对 OpenAI 兼容小模型尤其值得抄。

### 3.2 content/details 双通道（已有雏形，强化）

`ToolOutput { value, summary, evidence }` 的分离方向与 pi 一致（value→模型，summary/evidence→UI/审计）。建议 value 内部再学 pi 的结构化 details：如 read 工具返回 `{content, totalLines, outputLines, truncatedBy}`，模型和 UI 各取所需。

### 3.3 read 工具 offset/limit + 提示续读

pi 的 read：1-indexed offset/limit，截断时文本里明确提示模型"Use offset/limit for large files... continue with offset until complete"。我们的 workspace.read 若无分页参数应加上——配合 §2.3。

### 3.4 edit 工具的精确替换设计（若未来加）

pi 的 edit 一次调用多处互不重叠替换，每条 `oldText` 必须在**原始文件**唯一；BOM 剥离 → 行尾检测/归一 → 应用 → 还原行尾；同文件并发写用 `withFileMutationQueue` 串行化。我们的 workspace.replace 若要升级为多 edit 版本，这套细节（尤其行尾归一和唯一锚点）直接照抄。

### 3.5 executionMode：只读工具并行（P2）

pi 每个工具声明 `executionMode: "sequential" | "parallel"`；一批 tool call 先按序 prepare（保证 beforeToolCall 决策有序）再并发执行，**结果按 assistant 原始顺序回填**（toolCallId 对齐）。我们目前严格按 ordinal 串行。read-only 工具（workspace.list/read/search）并行执行是安全的（risk 已分类），能显著缩短探索类任务墙钟时间。注意与审批流的交互：需审批的工具天然串行。

### 3.6 ControlTool 泛化（P1）

**现状**：`ControlTool::execute` 是 no-op（`tools.rs:1053-1064`），`plan.update`/`run.finish` 的真实语义在 `execute_pending_tools` 里按名字 if 特判（`execution.rs:202-207`）——新增控制工具必须改引擎，抽象破裂。

**建议**：`Tool` trait 增加 `fn control(&self) -> Option<ControlEffect>`，或引擎识别 `ControlOutcome` 枚举（PlanUpdate/Finish/...）由工具自身返回；名字特判退化为 dispatch 表。pi 的等价物是 `terminate` 标志 + `afterToolCall` 钩子，本质都是"工具影响循环控制流"要有一等表达。

### 3.7 Operations 接口：工具逻辑与执行后端解耦（P2，长期方向）

pi 最有意思的架构点之一：每个内置工具暴露 `ReadOperations/EditOperations/BashOperations`，扩展可以把文件/命令执行**整体委托**给远端（SSH/VM/micro-VM）——Gondolin 容器化模式就是靠它实现的，"pi 和 auth 留宿主，工具执行进沙箱"。

对应到 SoloOps：现在 `process.exec` 和 `sandbox.exec` 是两个独立工具名，模型要自己选；未来可以统一为一个 exec 工具 + `backend: host | sandbox` 由**策略**决定（而非模型决定），workspace 工具同样可以有 sandbox backend。这同时是减法：模型看到的工具面更小，边界决策从模型侧移到策略侧——与我们"策略约束优先"的哲学一致。

### 3.8 只读工具集与动态 system prompt

pi 用 `createReadOnlyToolDefinitions()`（read/grep/find/ls）作为**构造性边界原语**：不注册就没有，比运行时拦截更硬。建议 run 模板支持 toolset 预设（readonly/standard/full），审批策略叠加其上。

system prompt 由各工具的 `promptSnippet` + `promptGuidelines` 动态拼装（没注册的工具不进 prompt，还自动生成"无 grep 时用 bash 替代"的指引）。我们的系统提示是静态的，建议 descriptor 加 `prompt_snippet` 字段。另外 pi 注入 AGENTS.md/CLAUDE.md（全局 + 祖先目录链，包裹为 `<project_instructions path>`）——SoloOps 可对应注入 workspace 内的 `SOLOOP.md`/`AGENTS.md`。

---

## 4. 状态机与持久化

### 4.1 StopReason 丰富化（P1）

pi 的 `StopReason = "pending" | "stop" | "length" | "toolUse" | "error" | "aborted" | "deferred"`。我们的模型响应缺少这一层（只区分成功/错误类别）。引入后：§2.4 的 length 防御、budget 的 output token 余量判断都有依据。

### 4.2 Journal → 追加树（P2）

pi 的会话存储是 **(id, parentId, seq) 追加日志树**：`Entry` 统一信封，`SessionState.applyMutation()` 校验 seq 连续/id 去重/parent 链合法；SQLite 后端（`session-backends/sqlite-node`：entries/lanes/branch-tips/records/writer-leases 表）天然支持 **fork**（`repo.fork(source, {scope: "tree"|"branch", entryId})`）与树上任意点回跳。

对 SoloOps 的价值：model retry 时可以分叉保留两条尝试路径；审计时能精确回答"这个决定基于哪条历史"。成本不小（schema + 投影逻辑），建议作为 Phase 3 目标，且仅当"重试探索/分支"成为产品需求时再做。我们的 `agent_items` 已有 per-run sequence，是树的退化形式，演进路径平滑。

### 4.3 操作级恢复记录（P1→P2）

pi 的 `LaneRecord`（operation_started/step_attempt/tool_started）+ `findOpenOperations()`（0 个=空闲，1 个=suspended 可恢复，2 个=损坏）把"崩溃恢复"做成了通用机制。

我们已有 workspace 写的三态恢复（`probe_interrupted_write`），但它是特例。建议把"副作用恢复语义"提升为工具声明：`Tool` trait 增加 `recovery: RecoverySemantics`（Idempotent / Probed(三态) / Blocking）。`recover_safe_runs` 按此分派，而不是硬编码 process→Blocked、workspace_write→probe 的映射。managed deploy 已是 Probed 语义，正好统一。

### 4.4 Paused：落地或删除（P1）

`run.rs:33` 定义了 Paused，`can_transition_to` 允许进入，但全库无写入点。建议落地：Owner 暂停 = 完成当前工具后停（区别于取消，可恢复），状态机已有位置，补 API + engine 检查点即可。若半年内不打算做，删掉死状态。

### 4.5 elapsed_ms 语义漂移（P0，bug 级）

`runtime_execution_state` 用 `now - created_at` 重算 elapsed（`storage/runtime.rs:1068`），含审批等待与 retry 等待时间——审批排队久的 run 会被 `max_duration_ms` 误杀成 Blocked。改为**累计活跃执行时长**（或 budget 检查时排除 waiting/retry 状态时段）。

---

## 5. 边界处理

### 5.1 取消通知（P1）

现状：`is_cancelled` 每轮循环 + 每 250ms 一次 SQL 查询（`execution.rs:842-848`），WS 也是每客户端独立轮询。SQLite 无 LISTEN/NOTIFY，但可以：
- 短期：取消检查降频 + 只读连接复用 prepared statement；
- 中期：API 进程写 `run_commands` 表后通过 hostd 同款思路给 worker 发通知（或干脆提供"单进程模式"让 API/worker 同进程用 tokio broadcast）。

### 5.2 超时联动 hostd kill（P1）

`execute_tool_cancellable` 超时只是 drop future，**宿主侧进程仍在运行**（靠后续 `recover_safe_runs` 判 Blocked 兜底）。hostd 协议已有 request_id/run_id/call_id 结构，增加 `kill` action 成本低、收益直接：超时后立即发 kill，进程状态收敛从"分钟级恢复周期"变"即时"。

### 5.3 审批拦截链泛化（P2）

pi 的审批是"平台提供拦截点（`tool_call` 事件返回 `{block, reason, terminate}`）+ 确认 UI，策略完全外置为扩展"。我们的 `policy_for` 是编译期 match。方向：把 policy 改造成可组合拦截链 `Vec<Interceptor>`（risk policy → owner approval → 自定义规则），每个拦截器返回 Allow/Deny/RequireApproval/Block(reason)。当前规模下收益有限，但为"按路径前缀放行只读命令"这类细粒度规则留好口子。

### 5.4 生产路径 expect 清理（P0）

`model.rs:258`（prompt 序列化）、`model.rs:371-376`、`storage/runtime.rs:874`。序列化我们自己的类型失败即 panic 不合理，改 `?` 传播为 `RuntimeError::Invariant`。

### 5.5 run_once 错误即时降级（P1）

非取消的 Storage 错误直接上抛给 worker 主循环记日志（`main.rs:53-56`），run 卡住直到租约过期。建议：连续 N 次 storage 错误 → run 立即置 Failed（带 error），而不是等 lease 超时周期。

### 5.6 Provider 兼容层备忘（P2）

若未来脱离单一 OpenAI 兼容端点：pi 的 `compat` 标志表（约 30 个开关：`requiresToolResultName`、`maxTokensField`、thinking 格式 12 种方言…按 baseUrl 自动探测）是抹平各家 OpenAI 兼容方言的最低成本方案。我们已有的 `stable_prompt_cache_key` 是对的，换 Anthropic 时注意 cache_control 三断点（system/最后工具定义/最后一条消息）的抽象。

---

## 6. 落地路线图

**Phase A（小改动、立即做）**
1. process/sandbox 非零退出 → 正常结果回传（§2.2）
2. 输出截断 + artifactRef 接通（§2.3）
3. expect() 清理（§5.4）
4. elapsed_ms 修复（§4.5）
5. length 截断防御（§2.4）

**Phase B（中改动、下个迭代）**
1. 上下文协议重构：journal → 原生消息序列（§2.1）
2. token-aware compaction（§2.5）
3. Steering 消息（§2.6）
4. ControlTool 泛化（§3.6）
5. hostd kill action（§5.2）
6. run_once 降级 + 取消检查降频（§5.5/§5.1）
7. RecoverySemantics 声明化（§4.3）

**Phase C（大改动、按需）**
1. Journal 追加树 + fork（§4.2）
2. Operations 接口统一 exec 后端（§3.7）
3. 只读工具并行（§3.5）
4. 审批拦截链（§5.3）
5. 多 provider compat 层（§5.6）

---

## 附：信息来源

- pi agent-core 循环/状态/steering：`参考/pi-main/packages/agent/src/agent-loop.ts`、`types.ts`、`harness/session/`
- pi 工具与扩展：`参考/pi-main/packages/coding-agent/src/core/tools/`（edit.ts/read.ts/truncate.ts/bash.ts）、`src/core/extensions/types.ts`、`docs/containerization.md`
- pi provider 层：`参考/pi-main/packages/ai/src/types.ts`、`api/anthropic-messages.ts`、`utils/retry.ts`、`utils/provider-retry.ts`
- pi 协议与存储：`参考/pi-main/packages/protocol/src/schemas.ts`、`packages/session-backends/sqlite-node/src/sqlite/repo.ts`
- SoloOps 现状：`crates/soloops-application/src/engine.rs`、`engine/execution.rs`、`model.rs`、`tools.rs`；`crates/soloops-domain/src/run.rs`；`crates/soloops-storage/src/runtime.rs`（关键行号见正文）
