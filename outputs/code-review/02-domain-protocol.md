# 02 — Domain/协议层审查（soloops-domain · hostd-protocol · 三个薄 bin）

- **审查节点**: 全代码库深度审查 · 第 2 节点（基础层）
- **审查对象**: 当前工作区状态（分支 `feature/agent-page-polish`，含未提交变更，同 00-baseline.md §0）
- **审查方式**: 范围内 12 个文件全部逐行精读（约 1294 行，含 domain 集成测试 contract_files.rs）；每条发现的代码片段均在写入前回读原文核实；语义结论经 rust-analyzer LSP（lsp-diagnostics / 符号引用）与 ripgrep 双重核实；未修改任何源码。
- **验证证据**: `cargo test -p soloops-domain -p soloops-hostd-protocol` 退出码 0（domain 7+1、protocol 3 全过，与基线 §1.3 一致）；`lsp-diagnostics` 对 12 个范围内文件 0 errors / 0 warnings（唯一提示为 api main.rs 的 `#[cfg(unix)]` inactive-code hint，Windows 平台预期）。
- **基线关系**: 00-baseline.md 已记录 e2e 2/2 失败（§1.9）与 sqlx 事实（§5），本文件不重复展开，仅在发现 F1 按纪律正式收录与本范围交叉的部分并注明基线来源。
- **编号说明**: 发现按严重性排序，编号 F1–F16 显式标注；文中交叉引用一律使用 F 编号。

**范围小结（供后续节点快速对齐）**：
- domain 层保持纯净：8 个 src 文件无任何 IO/时间/随机依赖，时间一律以 `i64` 毫秒参数传入；契约测试（tests/contract_files.rs）中的 `fs::read_to_string` 属集成测试，不破坏库纯净。
- **SecretRef 并不定义在 domain**——实际位于 `crates/soloops-application/src/config.rs:9`（带自定义 Debug 脱敏 `[REDACTED]`，见 config.rs:21-23）。domain 侧唯一承载明文秘密的类型是 `LoginRequest`（F6）。
- 状态机守卫 `can_transition_to` 的**唯一强制点**是 `crates/soloops-storage/src/database/tasks.rs:287`（`transition_run_on_connection`，带 CAS 双保险），domain 自身只提供纯函数守卫，设计正确；但枚举含 2 个死状态（F4）。
- hostd-protocol 是硬版本协议（`PROTOCOL_VERSION=3` 精确匹配 + `deny_unknown_fields`），消息枚举覆盖全部 6 个命令、无未类型化命令名，但 ManagedDeploy 子命令在**响应侧退化为字符串判别**（F11），且响应信封可表示非法组合（F10）。
- 三个 bin 启动路径全部走 `anyhow::Result` 清晰报错（无启动 panic）；问题集中在关机与装配一致性路径（F2、F3、F13–F16）。

---

## 发现列表

## [P2] F1：e2e 2/2 失败中"managed.deploy.apply 未出现"一条与 domain/协议层契约相关，本节点提供排查线索（基线来源收录）
- 位置: apps/web/e2e/happy-path.spec.ts:34（失败用例）+ 本范围关联链路 crates/soloops-domain/src/event.rs:71、crates/soloops-hostd-protocol/src/lib.rs:112-119
- 置信度: 高（失败事实）；链路归因: 中
- 证据:（基线 §1.9 记录的失败）`Error: expect(locator).toBeVisible() failed  Locator: getByText('managed.deploy.apply')  ... Timeout: 15000ms`；本范围相关代码：`pub payload: Value,`（event.rs:71，事件载荷无 schema）；`pub struct ManagedDeployResult { pub action: String, ... pub status: String,`（hostd-protocol lib.rs:112-115）
- 问题: 基线已证明浏览器/harness 正常、登录走通、任务创建成功，但 create→run→工具调用链路在真实 harness 下未渲染出 `managed.deploy.apply`。该链路穿过本范围的三类弱契约：① 事件 payload 为无类型 `serde_json::Value`，工具调用事件由 engine 经 storage/runtime.rs:782/894/986 以 `json!({...})` 自由构造，字段名漂移无法在编译期暴露（F8）；② ManagedDeploy 的 4 个子命令靠 `action: String` 与 `expected_action` 字符串比对判别（application/hostd.rs:158 `HostdResult::ManagedDeploy(result) if result.action == expected_action`），两端字符串字面量约定无类型保障（F11）；③ run 事件/状态在事件流（serde rename_all）与 DB/API（手写 as_str）双重编码（F5）。
- 建议: server/引擎节点深查 create→run→tool.call_started 事件在 e2e_harness 下的实际产出（事件表 + WS 回放），重点比对 payload 字段名与前端消费键名；本节点已在"跨层线索"给出全部锚点。**基线来源**: 00-baseline.md §1.9（失败 2）。

## [P2] F2：soloopsctl owner-init 与 domain 登录校验的长度契约不一致，可创建**永远无法登录**的 Owner（单用户系统即整库锁死）
- 位置: bins/soloopsctl/src/main.rs:19-22, 45-52（创建侧无上限校验）；crates/soloops-domain/src/requests.rs:88, 94（登录侧上限）；crates/soloops-storage/src/database/auth.rs:10-14（存储侧亦无上限）
- 置信度: 高
- 证据:
  ```rust
  // bins/soloopsctl/src/main.rs:19-22
  OwnerInit {
      #[arg(long, default_value = "owner")]
      username: String,
  },
  // bins/soloopsctl/src/main.rs:50-52 —— 仅最小长度，无上限、无字符集校验
  if password.chars().count() < 12 {
      bail!("password must contain at least 12 characters");
  }
  // crates/soloops-domain/src/requests.rs:87-88 —— 登录时用户名上限 64
  let username = self.username.trim().to_lowercase();
  if username.is_empty() || username.chars().count() > 64 {
  // crates/soloops-domain/src/requests.rs:94 —— 登录时密码上限 1024
  if self.password.is_empty() || self.password.chars().count() > 1024 {
  // crates/soloops-storage/src/database/auth.rs:10-13 —— 创建时只查非空
  pub async fn create_owner(&self, username: &str, password_hash: &str) -> Result<Owner, StorageError> {
      let username = username.trim().to_lowercase();
      if username.is_empty() {
  ```
- 问题: 操作员执行 `soloopsctl owner-init --username <超过 64 字符>` 或输入超过 1024 字符的密码时，Owner 创建成功；但此后**所有**登录尝试在 `LoginRequest::normalize` 处即被 400 拒绝（server/http/auth.rs:61-63 先 normalize 再查库）。由于 `count_users() > 0` 永久阻止重新初始化（ctl main.rs:42-44），只能手工改 SQLite 解锁。这是"创建路径接受、消费路径拒绝"的典型跨层契约缺口。
- 建议: 把 username/password 的上限与字符集规则提为 domain 共享常量或校验函数，`soloopsctl owner-init` 与 `create_owner` 在写入前强制执行同一规则（修复时注意 F9 的 `ValidationError: &'static str` 限制——错误消息无法携带实际长度，最好一并解决）。

## [P2] F3：默认数据库路径相对当前工作目录解析 + 三 bin 配置一致性零校验 → api/worker/ctl 可静默指向不同数据库（"必须一致配置"仅是 README 约定）
- 位置: bins/soloops-api/src/main.rs:10-12、bins/soloops-worker/src/main.rs:14-17、bins/soloopsctl/src/main.rs:30-31（三个入口各自独立 `AppConfig::load()`）；根因代码 crates/soloops-server/src/http/config.rs:39, 82, 190-200（根因文件属节点 3 范围，此处从 bin 装配视角收录）
- 置信度: 高
- 证据:
  ```rust
  // crates/soloops-server/src/http/config.rs:82 —— 默认值是相对路径
  database_path: absolute_path(env("SOLOOPS_DATABASE_PATH", "var/db/soloops.db"))?,
  // crates/soloops-server/src/http/config.rs:194-200 —— 相对路径按 CWD 拼接
  fn absolute_path(path: String) -> Result<PathBuf> {
      let path = PathBuf::from(path);
      if path.is_absolute() {
          return Ok(path);
      }
      Ok(std::env::current_dir()?.join(path))
  }
  ```
- 问题: 三个 bin（以及 hostd，经其自身配置直连 storage）各自 `.env`/环境变量独立解析；不设 `SOLOOPS_DATABASE_PATH` 时默认值随**启动时的工作目录**变化。从不同目录启动任一进程（systemd 单元 WorkingDirectory 不一致、手工 `cargo run` 与二进制直接运行混用）→ api 与 worker 各自打开/创建不同 SQLite，api 展示与 worker 执行完全脱节，**无任何报错或日志提示**。README 声称的"hostd、Worker、数据库必须一致配置"在代码中没有任何交叉校验（无实例指纹、无心跳表、doctor 也不查，见 F16）。
- 建议: ① 数据库内记录部署实例 ID（migration 时机生成），各进程启动时校验一致性；② `SOLOOPS_DATABASE_PATH` 缺省时要求绝对路径或以可执行文件/固定数据目录为基准；③ doctor 子命令显式核对各进程解析出的 database_path（见 F16）。

## [P3] F4：RunStatus 含两个死状态 Draft、Paused：全代码库零写入点，且契约测试把死状态强制传播到前端与 OpenAPI
- 位置: crates/soloops-domain/src/run.rs:7, 13（枚举与 RUN_STATUSES 定义）；crates/soloops-domain/tests/contract_files.rs:14-24（强制传播）；storage 实际初值 crates/soloops-storage/src/database/tasks.rs:29
- 置信度: 高（ripgrep 全库核实：`RunStatus::(Paused|Draft)` 与字符串 `"draft"` 的写入点均为 0，唯一定义处在 run.rs）
- 证据:
  ```rust
  // crates/soloops-domain/src/run.rs:6-13
  pub const RUN_STATUSES: &[RunStatus] = &[
      RunStatus::Draft,
      ...
      RunStatus::Paused,
  // crates/soloops-storage/src/database/tasks.rs:29 —— run 的真实初始状态是 'queued'
  VALUES (?, ?, 'queued', NULL, NULL, ?, NULL, NULL)",
  ```
- 问题: 状态机定义了 `Draft => Queued`、`Paused => Running` 转换与取消规则，但无任何代码路径产出这两种状态（runs 表插入即 `queued`）。`frontend_and_openapi_include_every_run_status` 契约测试反而要求前端 contracts.ts 与 openapi.yaml 携带全部 15 个值，把死状态扩散到全栈契约，误导后续开发者以为"草稿/暂停"功能已存在或必须支持。状态机表面积（15 态 × 转换矩阵）被两行死代码污染。
- 建议: 要么落地暂停功能（domain 已有 `Paused => Running` 语义位），要么删除 Draft/Paused；若保留作未来占位，应在类型文档与契约测试中标注 `// reserved`。注：outputs/pi-design-improvement-study.md:167 已独立注意到 Paused 无写入点（该文件为 outputs/ 根目录外部研究文档，非本审查共享目录，此处为独立核实后的正式收录）。

## [P3] F5：RunStatus/EventType 双重编码（手写 as_str ↔ serde rename）之间无任何一致性测试，DB 文本与 API/事件 JSON 可静默分叉
- 位置: crates/soloops-domain/src/run.rs:24-63（`#[serde(rename_all = "snake_case")]` + 手写 `as_str`）；crates/soloops-domain/src/event.rs:8-38（每个事件名字面量写两遍）；缺测试证据：全库 grep `serde_json::to_(value|string)` 对 RunStatus 0 命中
- 置信度: 高
- 证据:
  ```rust
  // run.rs:24-25 + 45 以下 —— 同一状态由两套独立字面量编码
  #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
  #[serde(rename_all = "snake_case")]
  pub enum RunStatus { ... }
  pub const fn as_str(self) -> &'static str { ... Self::WaitingForApproval => "waiting_for_approval", ... }
  ```
- 问题: 存储层走 `from_str`/`as_str` 文本（storage tasks.rs:309-315、344-347），API/事件序列化走 serde rename（如 tasks.rs:330 事件 payload `json!({ "from": current.status, "to": to })`）。当前 15 个值人工比对一致，但未来新增含数字/大写缩写的变体（如 `Http429`、`AIPaused`）时 snake_case 转换与手写字面量可能漂移，且无测试兜底——届时 DB 文本、事件 JSON、前端契约三者静默分叉。
- 建议: 增加穷尽性单测：`for s in RUN_STATUSES { assert_eq!(serde_json::to_string(&s).unwrap(), format!("\"{}\"", s.as_str())); assert_eq!(s.to_string().parse::<RunStatus>().unwrap(), s); }`，EventType 同理。

## [P3] F6：LoginRequest 持明文密码却 derive Debug/Clone/Serialize——距离违反 security.md"密码不得入日志"红线仅一次 `debug!("{:?}")` 之遥
- 位置: crates/soloops-domain/src/requests.rs:10-15；红线声明 docs/security.md:35
- 置信度: 高（类型事实）；现行泄漏路径: 无（grep 核实唯一使用点 server/http/auth.rs:58 提取器 + normalize，未打印）
- 证据:
  ```rust
  #[derive(Debug, Clone, Serialize, Deserialize)]
  #[serde(rename_all = "camelCase")]
  pub struct LoginRequest {
      pub username: String,
      pub password: String,
  }
  ```
- 问题: 对照同仓库的 SecretRef 实现（application/config.rs:21-23 手写 Debug 输出 `SecretRef(env:[REDACTED])`），domain 对明文密码采用了裸 derive Debug。任何后续维护者在错误链、请求日志、tracing span 中 `{:?}` 该结构体即直接泄漏明文密码，且不会有任何编译期或测试告警。属于"SecretRef 类型化彻底性"在 domain 侧的缺口。
- 建议: 为 LoginRequest 手写 Debug（password 输出 `[REDACTED]`），或引入 zeroize 包装类型承载密码字段（workspace 已依赖 zeroize）。

## [P3] F7：security.rs SSH 审计类型不携带任何边界不变量；配套解析器（server/ssh_access.rs）无文件大小/行长/行数上限
- 位置: crates/soloops-domain/src/security.rs:17-50（类型无约束声明）；跨层证据 crates/soloops-server/src/http/ssh_access.rs:79（`std::fs::read_to_string` 无上限，解析函数 grep 无 truncate/limit/MAX 命中）
- 置信度: domain 侧: 高；server 侧结论: 中（基于两轮针对性 grep `MAX_|truncate|cap|len\(\)|limit|bail`，未见任何上限逻辑；精读属节点 3）
- 证据:
  ```rust
  // security.rs:44-49 —— entries 无数量上限，字段无长度约束，也未文档化预期规模
  pub struct SshAuthorizedKeysFile {
      pub path: String,
      pub role: SshKeyFileRole,
      pub exists: bool,
      pub entries: Vec<SshAuthorizedKeyEntry>,
      pub error: Option<String>,
  }
  // server/http/ssh_access.rs:79 —— 整文件无界读入
  match std::fs::read_to_string(&path) {
  ```
- 问题: 任务要点要求 security.rs 对畸形输入（超长行、怪字符、海量条目）健壮。domain 本身是纯类型（无可指摘的解析行为），但它未把任何不变量写进类型或文档（行长上限、entries 上限、fingerprint 格式），健壮性完全外包给 server 解析器——而后者按 grep 证据无任何上限：超长 authorized_keys 文件会被全量读入内存、全量解析、全量进 API 响应/事件流。重复键、超长 comment 等畸形输入也无截断。
- 建议: 在 domain 类型文档中声明不变量（如"entries ≤ 10_000、单行 ≤ 8 KiB"），并在 server 解析器落地文件大小/行数/行长上限与字段截断；server 节点应接管精读并复核本条。

## [P3] F8：EventEnvelope.payload 为无类型 serde_json::Value，8 类事件载荷无 schema 契约，前端消费全靠字符串键约定
- 位置: crates/soloops-domain/src/event.rs:63-72；写入点 storage/runtime.rs:434, 548, 782, 894, 986, 1039 与 tasks.rs:40, 330
- 置信度: 高
- 证据:
  ```rust
  pub struct EventEnvelope {
      pub sequence: i64,
      pub id: String,
      pub run_id: Option<String>,
      #[serde(rename = "type")]
      pub event_type: EventType,
      pub payload: Value,
      pub created_at: i64,
  }
  ```
- 问题: EventType 枚举本身类型完备（8 个变体全部有写入点，覆盖创建/状态/计划/消息/工具三态/报告，无逃生舱——这点合格）；但每类事件的 payload 是自由 JSON，生产端 `json!({...})` 与消费端（server WS 回放、前端 run 页）之间没有编译期或契约测试约束。字段名漂移（正是 e2e 失败 2 的候选根因之一）只能在人工联调中发现。
- 建议: 为每类 EventType 定义强类型 payload 结构体（serde tagged 或 EventEnvelope 按 event_type 泛型化），至少先给 `tool.call_*` 三类建契约测试比对前端消费键名。

## [P3] F9：ValidationError 的 field/message 均为 &'static str，错误消息无法携带运行期上下文
- 位置: crates/soloops-domain/src/error.rs:34-39
- 置信度: 高
- 证据:
  ```rust
  #[derive(Debug, Clone, Error, Serialize)]
  #[error("{field} {message}")]
  pub struct ValidationError {
      pub field: &'static str,
      pub message: &'static str,
  }
  ```
- 问题: 校验错误只能输出编译期常量（如 "must contain between 1 and 160 characters"），无法报告实际长度/实际值摘要；也阻碍复用于任何动态校验场景，并直接限制 F2 的修复质量（无法告诉操作员用户名实际超长多少）。
- 建议: 字段类型改为 `Cow<'static, str>` 或 String；Serializer 语义不变。

## [P3] F10：HostdResponseV3 可表示非法状态：result 与 error 可同时为 None 或同时为 Some（类型层未使非法状态不可表示）
- 位置: crates/soloops-hostd-protocol/src/lib.rs:53-60, 62-83
- 置信度: 高
- 证据:
  ```rust
  pub struct HostdResponseV3 {
      pub protocol_version: u16,
      pub request_id: String,
      pub result: Option<HostdResult>,
      pub error: Option<HostdError>,
  }
  ```
- 问题: 两个互斥语义仅以两个独立 Option 表达，构造器（success/failure）虽只产出合法组合，但**反序列化不设防**：有缺陷/被篡改的 hostd 可返回双 None（无结果也无错误）或双 Some。当前唯一客户端 application/hostd.rs:151-156 已防御（error 优先、`ok_or_else(|| ... "hostd response had no result")`），但该防御是消费方自觉而非类型保证，未来新消费者（或 e2e harness 类旁路）易直接 `unwrap`。
- 建议: 提供 `into_outcome(self) -> Result<HostdResult, HostdError>` 消费方法统一收口，或用自定义 Deserialize 强制"恰其一"。

## [P3] F11：ManagedDeployResult 以自由字符串 action/status 承载 4 个子命令的判别——协议枚举在响应侧退化出字符串逃生舱
- 位置: crates/soloops-hostd-protocol/src/lib.rs:85-91（HostdResult 仅 3 变体）+ 112-119（action/status 为 String）；消费端 crates/soloops-application/src/hostd.rs:106, 114-131, 158
- 置信度: 高
- 证据:
  ```rust
  // lib.rs:87-91 —— 6 个命令（含 4 个 ManagedDeploy 子命令）只对应 3 个结果变体
  pub enum HostdResult {
      ProcessExec(ProcessExecResult),
      SandboxExec(SandboxExecResult),
      ManagedDeploy(ManagedDeployResult),
  }
  // lib.rs:113-115
  pub struct ManagedDeployResult {
      pub action: String,
      pub project_id: String,
      pub status: String,
  // application/hostd.rs:106 + 158 —— 客户端用字符串比对还原子命令
  expected_action: &'static str,
  ...
  HostdResult::ManagedDeploy(result) if result.action == expected_action => {
  ```
- 问题: Plan/Status/Apply/Rollback 四个子命令在请求侧是类型化枚举，在响应侧坍缩为 `action: String`（连带 `status` 也是自由字符串，"proposed"/"applied" 等无类型约束），正确性完全依赖两侧字符串字面量约定；hostd 返回不匹配字符串时只能运行时报 "mismatched managed deployment action"。这正属于审查要点中"无字符串逃生舱"未达成的部分。另外 `max_output_bytes` 在 6 个 HostdAction 变体中逐个重复（lib.rs:27-50），可上提至信封层。
- 建议: HostdResult 拆出 ManagedDeployPlan/Status/Apply/Rollback 变体（或 action 改枚举 + serde rename）；status 同理枚举化；max_output_bytes 上提。

## [P3] F12：hostd-protocol 全文件 254 行零文档：信任模型、版本策略、帧上限语义全部未记录，只能从消费方反推
- 位置: crates/soloops-hostd-protocol/src/lib.rs 全文（无任何 `//!` / `///`）；实际安全前提位于 bins/soloops-hostd/src/unix.rs:135-141
- 置信度: 高
- 证据:
  ```rust
  // lib.rs:6-12 —— 版本与上限常量无一行说明
  pub const PROTOCOL_VERSION: u16 = 3;
  pub const MAX_REQUEST_BYTES: usize = 64 * 1024;
  pub const MAX_RESPONSE_BYTES: usize = 10 * 1024 * 1024 + 64 * 1024;
  pub const fn supports_protocol_version(version: u16) -> bool {
      version == PROTOCOL_VERSION
  }
  // bins/soloops-hostd/src/unix.rs:135-137 —— 对端身份假设（SO_PEERCRED UID 精确匹配）只写在 bin 里
  let credentials = stream.peer_cred()?;
  if !peer_is_allowed(credentials.uid(), state.allowed_uid) {
  ```
- 问题: 审查要点问"对端身份假设是否明确"——在协议 crate 层面不明确：Unix Socket 同机部署、单 allowed_uid、无协议内认证、版本硬匹配（`supports_protocol_version` 精确等于 3）+ 双向 `deny_unknown_fields` 意味着**双端必须同版本同源码部署**（无任何兼容窗口），`max_output_bytes` 合法范围（1024..=MAX_TOOL_OUTPUT_BYTES）由 hostd 侧另行执行而非协议定义——这些装配前提没有一处写在协议 crate 里。对安全关键协议而言，文档缺失即审计成本与误用风险。
- 建议: 补 `//!` 模块文档：信任模型（同机 Unix Socket + SO_PEERCRED，由 hostd 强制）、版本策略（精确匹配、deny_unknown_fields ⇒ 零前向兼容）、帧语义（u32 长度前缀 + 调用方传入上限 + read_frame 先查限再分配）。

## [P3] F13：soloops-api 关机路径含两处 `expect` panic，且错误分支早退跳过 `database.close()`、notification 错误可掩盖 server 错误
- 位置: bins/soloops-api/src/main.rs:44-48, 50-55（expect）；36-40（`?` 早退 + 顺序问题）
- 置信度: 高
- 证据:
  ```rust
  // main.rs:45-48
  let ctrl_c = async {
      tokio::signal::ctrl_c()
          .await
          .expect("failed to install Ctrl+C handler");
  };
  // main.rs:36-40
  notification_task
      .await
      .context("notification monitor task failed")?;
  server_result?;
  database.close().await;
  ```
- 问题: ① 信号处理器安装失败（罕见但存在，如无控制台环境）→ 生产进程以 panic 终止而非优雅退出/清晰报错；② `notification_task.await` 的 `?` 先于 `server_result?`：若 notification 任务失败且 server 也失败，server 的真实错误被吞；任一 `?` 早退都跳过 `database.close()`（进程退出兜底可接受，但 WAL 优雅收尾与统一清理语义被破坏）。
- 建议: expect 改为日志 + 优雅退出；`server_result` 与 notification 结果各自显式汇报后再 close；或用结构化收尾块统一清理。

## [P3] F14：soloops-worker 的 Ctrl+C 仅在"无任务"分支被轮询：连续有活时停止信号被无限推迟；且 ctrl_c 的 Err 被静默当作收到信号
- 位置: bins/soloops-worker/src/main.rs:36-58（Ok(Some) 分支直接进入下一轮 run_once，不 select ctrl_c；Ok(None) 分支三路 select）
- 置信度: 高
- 证据:
  ```rust
  match engine.run_once(&worker_id).await {
      Ok(Some(run_id)) => info!(worker_id, run_id, "agent runtime cycle completed"),
      Ok(None) => {
          tokio::select! {
              _ = tokio::time::sleep(std::time::Duration::from_millis(config.worker_poll_ms)) => {}
              _ = tokio::time::sleep_until(next_session_cleanup) => { ... }
              _ = tokio::signal::ctrl_c() => break,
          }
      }
  ```
- 问题: ① 引擎连续返回 Some（队列一直有活）时循环从不轮询 ctrl_c future，Ctrl+C 完全不生效，worker 关机时延不可控（单 Owner 下概率低，但行为未声明）；② select 分支把 `Result` 完成一律当信号处理——`ctrl_c()` 返回 Err（安装失败）也会静默 break，与 api 侧的 expect-panic 是同一问题的两个相反极端。
- 建议: 将 ctrl_c future 提出循环复用（共享 watch channel），或至少在 `Ok(Some)` 分支后也非阻塞地检查停止标志；对 Err 分支记录 error! 日志再退出。

## [P3] F15：soloopsctl 的明文密码 String 未 zeroize（与 workspace 已有 zeroize 依赖的安全基线不一致）
- 位置: bins/soloopsctl/src/main.rs:45-58
- 置信度: 高
- 证据:
  ```rust
  let password = rpassword::prompt_password("Owner password: ")?;
  ...
  let password_hash = Argon2::default()
      .hash_password(password.as_bytes(), &salt)
  ```
- 问题: 口令以普通 String 在堆上存续至进程退出，无 zeroize；workspace 明确依赖 zeroize 且 application 层对密钥做了零化处理，CTL 作为唯一直接收纳 Owner 口令的入口反而没有跟进（docs/security.md:35 红线同适用于进程内存卫生）。CLI 进程生命周期短、风险低，故 P3。
- 建议: `zeroize::Zeroizing<String>` 包裹 password（rpassword 输出可直接包入）。

## [P3] F16：soloopsctl doctor 检查深度与 README 声称不一致：只做 config 解析 + database.ready()，不核查 hostd/worker/数据库一致性
- 位置: bins/soloopsctl/src/main.rs:62-64；对照 README 的"hostd、Worker、数据库必须一致配置"声明
- 置信度: 高
- 证据:
  ```rust
  Command::Doctor => {
      database.ready().await?;
      println!("SoloOps configuration and database are ready.");
  }
  ```
- 问题: doctor 输出"ready"仅证明 env 可解析、单个 DB 可用；对 F3 的三类静默分裂场景（CWD 相对路径、env 未导出、hostd socket 配置漂移）一律不设防——恰好是 README 要求操作员保证的项。
- 建议: doctor 增加：报告解析出的 database_path 绝对路径与 schema 版本、探测 hostd socket 存在性（配置启用时）、校验模型 API key ref 可解析（RuntimeConfig 复用），把"一致性"从 README 约定变成可执行检查。

---

## 正面确认（无发现项，供后续节点免重复核查）

以下要点经核实**合格**，不计入缺陷：
1. **状态机守卫真实生效**：`can_transition_to` 由 storage 唯一写入点 `transition_run_on_connection`（tasks.rs:287）强制，且带 `WHERE id = ? AND status = ?` CAS 双保险与 `rows_affected != 1` 冲突复核；终态（Blocked/Succeeded/Failed/Cancelled）无出边、取消仅允许自非终态（run.rs:65-109，有测试覆盖）。
2. **协议消息枚举覆盖完整**：HostdAction 6 变体与 hostd dispatch 全部对上（unix.rs:157-486），无未处理命令、无命令名字符串分发；请求/响应信封均 `deny_unknown_fields`。
3. **协议版本闭环成立**：hostd 收到不匹配版本回 `UnsupportedVersion`（unix.rs:141-147），客户端校验响应 version + request_id 相关性（application/hostd.rs:146, 217, 290）。
4. **帧安全**：`read_frame` 先比对 limit 再分配（lib.rs:177-185），恶意长度前缀不会触发大分配；EOF/半帧由 read_exact 归入 Io 错误。
5. **max_output_bytes 有接收端强制**：hostd 对 process/sandbox 执行 `1024..=MAX_TOOL_OUTPUT_BYTES` 区间校验（unix.rs:649、sandbox.rs:101），managed 路径拒 0 与超上限（unix.rs:531）——但两条规则不统一（managed 无下限 1024），留给 hostd 节点统一。
6. **domain 纯净**：8 个文件无 IO/时钟/随机依赖；时间戳全部 `i64` 参数传入。
7. **LoginRequest.normalize 不 trim 密码**（正确保留原始字节），用户名归一化与存储侧 `find_user_by_username` 的二次归一化构成纵深防御。
8. **Argon2id 参数**：ctl 用 `Argon2::default()`（Argon2id、19 MiB、t=2）+ 16 字节随机盐，符合 OWASP 推荐档位。

---

## 跨层线索（供后续节点核查）

| # | 线索 | 锚点 |
|---|---|---|
| 1 | **状态写入唯一守卫点**：storage 节点需确认所有 status UPDATE 均经 `transition_run_on_connection`（尤其租约获取、恢复扫描是否有旁路直写 SQL） | storage/tasks.rs:271-325 |
| 2 | **进入 WaitingForApproval / RetryScheduled / NeedsRecovery 时释放租约**（tasks.rs:294-300）：引擎节点需验证审批通过（WaitingForApproval→Running）后由谁重新持租约执行；若无人接管，Running 无租约运行将停滞至恢复扫描 | storage/tasks.rs:294-300 + domain/run.rs:94 |
| 3 | **TaskSummary 内连接 SQL**：无 run 的任务在列表中不可见（tasks.rs:74-78 inner join）；当前无 run 删除路径，storage 节点确认即可 | storage/tasks.rs:74-78 |
| 4 | **登录长度契约单点在 domain normalize**：server 节点审查 http/auth.rs 时注意 JsonRejection 的 `body_text()` 不含原始 body（已核实），但新增"创建/改密"端点必须复用同一契约（F2 的根因面） | requests.rs:85-105 + ctl/main.rs |
| 5 | **ssh_access 解析无上限**（server 节点接管精读，对应 F7）：read_to_string 无界（:79）、无行数/行长/截断逻辑（两轮 grep 证实）；评估该认证后端点的内存放大面与畸形输入（超长行、重复键、怪字符）行为 | server/http/ssh_access.rs:79, 98, 186-311 |
| 6 | **ManagedDeploy 字符串契约**（对应 F11）：engine/hostd 节点应比对 application/hostd.rs:106-131 的 `"plan"/"status"/"apply"/"rollback"` 与 bins/soloops-hostd/deployment.rs 返回的 `result.action` 字面量是否逐字一致——e2e 失败 2 的链路经过这里 | application/hostd.rs:106-131, 158 + hostd deployment.rs |
| 7 | **事件 payload 无 schema**（F8 的消费端）：server WS 回放与前端 run 页对 `tool.call_started/completed/failed` payload 键名的消费方式是 e2e 失败 2 的首要排查点 | storage/runtime.rs:782, 894, 986 + server WS 回放 |
| 8 | **hostd 两条 max_output_bytes 校验规则不一致**（managed 无 1024 下限 vs process/sandbox 有）：hostd 节点统一 | bins/soloops-hostd/src/unix.rs:531 vs 649、sandbox.rs:101 |
| 9 | **AppConfig CWD 相对路径**（F3 根因面归 server 节点）：`dotenvy::dotenv()` 各进程独立加载 + `current_dir().join` 相对解析 | server/http/config.rs:39, 82, 190-200 |
| 10 | **死状态 draft/paused 已被契约测试传播到前端/OpenAPI**（F4）：UI 节点不要基于它们做功能假设；若 domain 删除这两个状态，contracts.ts 与 openapi.yaml 必须同步（否则契约测试反过来会阻止删除） | domain/tests/contract_files.rs:10-25 |

---

## 覆盖清单

| 文件 | 行数 | 状态 |
|---|---|---|
| crates/soloops-domain/src/lib.rs | 25 | 已审（穷尽） |
| crates/soloops-domain/src/error.rs | 45 | 已审（穷尽；F9） |
| crates/soloops-domain/src/event.rs | 73 | 已审（穷尽；F5、F8） |
| crates/soloops-domain/src/requests.rs | 105 | 已审（穷尽；F2、F6；normalize 规则逐条核对） |
| crates/soloops-domain/src/run.rs | 133 | 已审（穷尽；F4、F5；转换矩阵 15×15 逐边核对） |
| crates/soloops-domain/src/runtime.rs | 232 | 已审（穷尽；无缺陷发现；BudgetSnapshot 默认值、UsageSnapshot serde default 均核对） |
| crates/soloops-domain/src/security.rs | 75 | 已审（穷尽；F7——类型纯数据、解析在 server 层，已跨层取证） |
| crates/soloops-domain/src/tests.rs | 108 | 已审（穷尽；测试有效性核对，缺 serde round-trip 见 F5） |
| crates/soloops-domain/tests/contract_files.rs（范围外附带） | 25 | 已审（穷尽；F4 + 跨层线索 10） |
| crates/soloops-hostd-protocol/src/lib.rs | 254 | 已审（穷尽；F10、F11、F12；帧函数逐行核对） |
| bins/soloops-api/src/main.rs | 63 | 已审（穷尽；F13；启动路径逐行核对） |
| bins/soloops-worker/src/main.rs | 62 | 已审（穷尽；F14） |
| bins/soloopsctl/src/main.rs | 69 | 已审（穷尽；F2、F15、F16） |

**发现统计**: P0 × 0；P1 × 0；P2 × 3（F1 基线收录、F2 owner-init 契约、F3 CWD 相对 DB 路径）；P3 × 13（F4–F16）。合计 16 条。

**审查方法补充**: 语义结论均经 ripgrep 全库定位 + 关键片段回读原文核实（storage/auth.rs、storage/tasks.rs、application/hostd.rs、application/config.rs、server/http/config.rs、server/http/auth.rs、bins/soloops-hostd/unix.rs 的取证性读取均只为验证 domain/protocol/bin 侧发现的真实影响，其自身缺陷留给对应节点）；未修改任何文件；`cargo test -p soloops-domain -p soloops-hostd-protocol` 通过（与基线一致）。
