# SoloOps Phase 1 架构评审与下一步方向

- **评审日期：** 2026-07-17
- **评审范围：** 设计.md v0.4 + Phase 1 代码实现 + code-review-followup.md
- **评审角色：** 软件架构师

---

## 一、需求评审

### 1.1 产品定位 — 清晰且克制

SoloOps 的产品定义非常明确：**单 Owner 私有 Agent 控制平面**，核心闭环是 `目标 → 规划 → 执行 → 验证 → 报告`。设计文档明确排除了多租户、本地模型推理、任意 root Shell、Docker Socket 暴露等常见"范围蔓延"陷阱。

**做得好的地方：**

- 固定原则第 8 条"命令成功不等于任务成功"是整个系统的灵魂，直接驱动了 Evidence 闭环设计。
- 明确排除清单（§2.3）比正面需求更有价值——它锁定了不会做什么。
- "先加法后减法"（§3.1/3.2）的交付哲学，配合验收门槛，避免临时结构永久化。

**需要关注的张力：**

| 张力点                                       | 现状                                 | 风险                                                                                         |
| -------------------------------------------- | ------------------------------------ | -------------------------------------------------------------------------------------------- |
| 单 Owner vs 授权检查                         | code-review #1 仍为 Blocker          | 短期安全（单用户），但一旦开放多用户就是漏洞                                                 |
| Phase 1 进程内 Runtime vs 设计文档的进程边界 | 设计写的是 `soloops-worker` 独立进程 | 当前实现正确（单进程多 crate），但 §4.1 说"API 与 Worker 独立部署"已实现，hostd 未实现属预期 |
| 静态 Tool 注册 vs 动态能力扩展               | 设计明确首版不动态加载               | 合理，但意味着每加一个 Tool 都要重新编译部署                                                 |

### 1.2 路线图节奏 — 合理但 Phase 2 跨度大

```
Phase 0 ✅  Rust 基线（Auth/Task/Run/Event/Audit + 静态 WebUI）
Phase 1 ✅  可观察 Agent Runtime（Model Loop + Tool + Budget + Recovery）
Phase 2 🔜  隔离执行（hostd + Docker Sandbox + Browser + SecretRef）
Phase 3     部署闭环（Compose + Caddy + 回滚 + 报告）
```

Phase 0→1 的演进很自然：从"桩执行器总是 blocked"到"真实 Agent 闭环"。但 **Phase 2 一次引入了 4 个重大子系统**（hostd、Docker Sandbox、Browser Adapter、SecretRef），每个都有独立的复杂度和安全边界。建议拆分。

---

## 二、架构评审

### 2.1 分层与依赖方向 — 优秀

```
binary → server/application/storage → domain
```

- `soloops-domain` 零外部依赖（无 HTTP、无 DB、无 async runtime），只含纯数据类型和状态机。这是教科书级的 Clean Architecture。
- `soloops-storage` 独占所有 SQL，HTTP handler 不拼 SQL。
- `soloops-application` 不感知 HTTP 细节。

**验证结果：** 依赖方向在代码中严格保持，未发现违反。

### 2.2 双层状态机 — 设计精巧但需关注复杂度

Run 状态机（15 个状态）+ Runtime Checkpoint（8 个状态）的映射关系清晰。但状态组合空间为 15×8=120，测试矩阵已经不小。

**做得好的地方：**

- 状态转换校验在 `domain` 层，且通过 `transition_run_on_connection` 统一入口执行（code-review #2 修复后）。
- 终态不可离开的测试覆盖完整。

**需要关注：**

- `NeedsRecovery` 是一个"分流"状态，恢复路径有 `Queued`（重试）、`Blocked`（放弃）、`Failed`（致命）三个出口。恢复决策逻辑目前分散在 `recover_expired_runs` / `recover_safe_runs` / `probe_interrupted_write` 中，随着 Tool 类型增加，恢复策略会越来越复杂。建议考虑 Strategy 模式或恢复决策表。

### 2.3 持久化 Item Loop — 核心竞争力

这是整个系统最有价值的设计决策：

```
准备上下文 → 调用模型 → 原子持久化 Item → 策略检查 → 执行 Tool → 持久化 Result → 下一轮
```

**关键设计选择评审：**

| 设计选择                                                  | 评价                                                  |
| --------------------------------------------------------- | ----------------------------------------------------- |
| 不用 ReAct 文本范式，用结构化 Item                        | ✅ 正确。文本解析是脆弱的，结构化 Item 可审计、可恢复 |
| Checkpoint 在副作用前写入                                 | ✅ 关键。保证恢复时能判断"做了没做"                   |
| 同一事务提交 Run 状态 + Checkpoint + Item + Event + Audit | ✅ 强一致性保证                                       |
| Tool Call 串行执行（首版）                                | ✅ 合理的保守选择。确定性 > 并发性                    |
| 上下文从 Item Journal 投影，不是全量回放                  | ✅ 预算友好，可测试                                   |
| `run.finish` 不直接成功，需验证                           | ✅ 核心安全不变式                                     |

### 2.4 Tool Registry 与策略 — 安全分层清晰

```
read_only → allow
workspace_write / process → require_approval
network / privileged → deny
```

首版策略简单明确。`workspace.replace` 的前置条件（oldContent 或 SHA-256）设计得很好——它不是任意覆盖。

**但有一个设计盲区：** `plan.update` 和 `run.finish` 被标记为 `ToolRisk::ReadOnly`，绕过了审批。这在语义上是对的（它们不碰操作系统），但 `run.finish` 实际上是**完成认证**的入口——模型通过它声明"我做完了"。如果验证逻辑有漏洞，模型可以伪造完成。当前验证逻辑（§10.6）比较完善，但建议 `run.finish` 的风险级别单独分类为 `control`，与 `read_only` 区分，方便未来施加额外约束。

### 2.5 预算与恢复 — 严谨

- 硬预算在 Run 开始时固化快照，超限直接 `blocked`/`failed`。
- 取消信号在模型调用间、Tool 调用前检查。
- 恢复规则区分了瞬时错误/永久错误/需审批风险。
- `probe_interrupted_write` 的设计很巧妙：通过比较文件内容判断写入是否已完成，避免重复执行。

**需要关注：** `project_journal` 的上下文投影用 `item.payload.to_string().len()` 估算大小，这不等于实际 token 数。当前是保守估算（偏大），不会超预算，但可能过早截断上下文导致模型表现下降。未来应引入 tokenizer 精确计算。

---

## 三、代码实现评审

### 3.1 已实现且质量高

| 模块                 | 实现状态  | 质量                                       |
| -------------------- | --------- | ------------------------------------------ |
| Run 状态机           | 完整      | 15 状态 + 转换图 + 终态保护                |
| SQLite 迁移接管      | 完整      | 旧 Schema 兼容验证                         |
| 认证与 Session       | 完整      | Argon2id + SHA-256 摘要 + Cookie 安全      |
| 事件补拉与 WebSocket | 完整      | sequence 游标 + 断线重连 + 损坏检测        |
| Model Item Loop      | 完整      | Provider 适配 + 重试退避 + 取消检查        |
| Tool Registry        | 完整      | 8 个内置 Tool + 路径逃逸防护               |
| 策略与审批           | 完整      | 三级策略 + Owner 审批 + 参数指纹           |
| Evidence 与报告      | 完整      | 验证闭环 + Markdown/JSON 报告              |
| 预算与恢复           | 完整      | 硬预算 + 租约续期 + 中断恢复               |
| 可观测性             | 完整      | JSON 日志 + request_id + Prometheus + 审计 |
| 测试覆盖             | 33 个测试 | 状态机 + 存储 + 运行时 + HTTP              |

### 3.2 已知技术债

| #   | 问题                                | 严重度 | 来源                                                    |
| --- | ----------------------------------- | ------ | ------------------------------------------------------- |
| 1   | API 端点不校验 Run 归属             | 🟡 中  | 单 Owner 下无实际风险，多用户前必须修                   |
| 2   | `resolve_existing` TOCTOU 窗口      | 🟢 低  | 需已有 FS 访问权才能利用                                |
| 3   | 错误分类依赖字符串匹配              | 🟢 低  | `lower.contains("context") && lower.contains("length")` |
| 4   | 上下文投影用字节长度估算 token      | 🟢 低  | 保守但可能影响模型表现                                  |
| 5   | `run.finish` 风险级别为 `read_only` | 🟡 中  | 语义不精确，建议 `control` 级别                         |
| 6   | 恢复决策逻辑分散                    | 🟡 中  | 随 Tool 类型增加复杂度上升                              |

### 3.3 架构一致性验证

| 设计文档要求                | 代码实现                                         | 状态 |
| --------------------------- | ------------------------------------------------ | ---- |
| domain 不依赖 HTTP/DB/async | `soloops-domain/src/lib.rs` 纯 serde + thiserror | ✅   |
| 数据库访问只通过 storage    | server 层通过 `Database` 抽象                    | ✅   |
| HTTP handler 不拼 SQL       | handler 调 `database.xxx()`                      | ✅   |
| 迁移显式执行不自动          | `soloopsctl migrate` + `verify_schema()`         | ✅   |
| Checkpoint 在副作用前写入   | `set_runtime_checkpoint` 先于 Tool 执行          | ✅   |
| 同一事务提交状态+事件+审计  | `transition_run_on_connection`                   | ✅   |
| Tool 静态注册               | `ToolRegistry::standard()`                       | ✅   |
| API 与 Worker 独立部署      | 两个 binary 共享 DB                              | ✅   |
| 静态 WebUI                  | `adapter-static` + Rust ServeDir                 | ✅   |
| 生产无需 Bun/Node           | release binary + 静态文件                        | ✅   |

---

## 四、下一步开发方向

### 4.1 优先级排序

基于"可逆性优先"和"先解决最大不确定性"的原则：

```
P0  修复 #1 资源归属授权检查         [1-2 天]   低风险高价值
P0  端到端冒烟测试自动化              [2-3 天]   验证 Phase 1 完整性
P1  SecretRef 机制                   [3-5 天]   Phase 2 前置依赖
P1  hostd 骨架 + Unix Socket         [5-7 天]   Phase 2 核心边界
P2  Docker Sandbox 最小实现          [5-7 天]   可与 hostd 并行
P2  Browser Adapter（如需要）        [5-7 天]   视业务需求
P3  部署闭环（Compose + Caddy）     [5-7 天]   Phase 3
```

### 4.2 建议拆分 Phase 2

原设计 Phase 2 包含 4 个子系统，建议拆为两步：

**Phase 2a：隔离边界**

- `soloops-hostd` 骨架（Unix Socket + 类型化请求 + 独立校验）
- SecretRef（替代环境变量直读 API Key）
- `process.exec` 迁移到通过 hostd 执行（而非 Worker 直接 spawn）

**Phase 2b：沙箱与浏览器**

- Docker Sandbox（通过 hostd 的 `sandbox.create/exec/destroy`）
- Browser Adapter（CDP 连接受管 Chromium）
- 受管网络策略

**拆分理由：** hostd + SecretRef 是**特权边界**的重构，风险高但范围明确；Docker Sandbox 是**执行环境**的升级，依赖 hostd 但可独立验证。分开后每步都有可验收的交付物。

### 4.3 近期具体建议

**1. 补齐端到端测试**

当前测试是单元/集成级别，缺少真正的端到端：

- 构建 SPA → 启动 API + Worker → 浏览器登录 → 提交任务 → 审批 → 验证报告
- 这会暴露契约漂移、WebSocket 重连、静态文件 fallback 等集成问题

**2. 引入 `control` 风险级别**

```rust
pub enum ToolRisk {
    ReadOnly,
    WorkspaceWrite,
    Process,
    Network,
    Privileged,
    Control,  // 新增：plan.update, run.finish
}
```

策略上仍然 `allow`，但审计和日志中单独标记，为未来施加额外约束留出空间。

**3. 上下文投影引入 token 估算**

当前 `project_journal` 用 `to_string().len()` 估算，建议：

- 短期：引入 `chars().count()` 作为更好的近似（UTF-8 安全）
- 中期：对接 tokenizer（如 `tiktoken-rs`），精确计算

**4. 恢复决策表**

将分散的恢复逻辑收敛为显式决策表：

```rust
fn recovery_strategy(tool: &str, risk: ToolRisk, state: ToolCallStatus) -> RecoveryAction {
    match (tool, risk, state) {
        ("workspace.create" | "workspace.replace", WorkspaceWrite, Running) => ProbeAndRecover,
        ("process.exec", Process, Running) => NeedsRecovery,
        ("workspace.read" | "workspace.search", ReadOnly, Running) => RetryByIdempotencyKey,
        _ => NeedsRecovery,
    }
}
```

**5. ADR 记录关键决策**

已有一份 ADR-0001（Rust 后端）。建议补充：

- ADR-0002：为什么不用 ReAct 文本范式
- ADR-0003：Tool Call 串行 vs 并行
- ADR-0004：SQLite 作为唯一持久层（vs 引入消息队列）

---

## 五、总结

### 做得好的

- **设计文档质量极高**：约束清晰、排除明确、原则可执行。v0.4 的 Agentic Runtime 设计（§10）是我见过的最完整的 Agent 执行架构设计之一。
- **实现忠实于设计**：代码与设计文档的对应关系清晰，未发现重大偏离。
- **安全边界严谨**：路径逃逸防护、无 Shell 进程执行、前置条件写入、Evidence 验证闭环——每一层都有实质约束。
- **测试驱动**：33 个测试覆盖了状态机、存储竞争、运行时行为、取消恢复等关键路径。
- **代码审查闭环**：code-review-followup.md 14 个问题修复了 11 个，每个修复都有测试。

### 需要改进的

- **端到端测试缺失**：当前测试粒度在单元/集成层，缺少真正的全链路验证。
- **Phase 2 跨度过大**：建议拆为 2a（特权边界）+ 2b（沙箱浏览器）。
- **恢复逻辑分散**：随 Tool 类型增加会变得难以维护。
- **风险级别语义不够精确**：`run.finish` 不应与 `workspace.read` 同级。

### 总体评价

**这是一个设计驱动、实现忠实、安全意识强的项目。** 设计文档不是摆设，每一章都能在代码中找到对应实现。Phase 1 已完成核心闭环——从任务提交到 Agent 执行、审批、验证、报告——并且每个环节都有持久化和可审计性保证。

下一步的**最大不确定性**不在功能数量，而在 Phase 2 的特权边界重构（hostd）。建议以此为下一阶段重点，同时补齐端到端测试作为安全网。
