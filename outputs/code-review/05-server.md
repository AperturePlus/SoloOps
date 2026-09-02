# 05 — soloops-server / HTTP 层 + OpenAPI 契约审查

- **审查对象**: 工作区状态（分支 `feature/agent-page-polish`），crates/soloops-server 全部源码（http.rs 503 / config.rs 246 / auth.rs 172 / ssh_access.rs 548 / notifications.rs 534 / error.rs 97 / telemetry.rs 68 / lib.rs 6 / http/tests.rs 446 / examples/e2e_harness.rs 437）+ docs/api/openapi.yaml 833 全文
- **方法**: 逐文件穷尽精读；契约核对以 Rust serde 序列化代码为准逐字段比对；`arguments_json`/`build_router`/`write_audit` 动作面经 ripgrep 全库定位后回读核实
- **前序线索**: 02-F3/F4/F7、03-线索4/线索5、04a 事件 payload 键名表、04b 交接线索①③ 均已核查，见"前序线索核查答复"
- **本节点纪律**: 未改任何源码；未跑 cargo test（00-baseline 同日 104 passed 基线沿用，本节点零代码变更）

---

## 正面确认（后续节点免重复核查）

1. **认证与 cookie 设计合格**: token 为 `rand::random::<[u8; 32]>()`（CSPRNG 256bit，auth.rs:166-168），落库仅 SHA-256 hex（auth.rs:170-172，无 pepper 但 token 熵 256bit 下合理）；cookie httpOnly + SameSite::Strict + Secure 按 environment（auth.rs:119-125）；logout 吊销 session + 审计（auth.rs:141-152）。
2. **CSRF 防御**: enforce_origin 对非 GET/HEAD/OPTIONS 的带 Origin 请求做精确同源比对（http.rs:147-155）；无 Origin 的非浏览器客户端放行——form-CSRF 不可行（axum Json 拒绝 urlencoded content-type），跨站 WS 被 SameSite=Strict cookie 阻断。
3. **SMTP 注入面封死**: recipient 双重校验（http.rs:274 `lettre::Address` + notifications.rs:112-114 `Mailbox::parse`），subject 仅由 `Ipv4Addr`/常量构造（notifications.rs:134-138），from 在 config 与 mailer 构造时各 parse 一次（config.rs:132-133, notifications.rs:105）。无 CRLF 进入 header 的路径。TLS 强制（仅 relay/starttls_relay，config.rs:134-144 无明文选项）；凭据仅 env 间接引用（config.rs:176-188）。
4. **通知失败闭环**: `record_ip_notification_failure` + 双向审计（success/failure，notifications.rs:341-376）；下轮按 `last_notified_ipv4 != current` 只重试失败者（notifications.rs:317-319），测试覆盖（notifications.rs:483-533）。
5. **WS 认证在握手前**: `require_owner` 在 `WebSocketUpgrade::from_request_parts` 之前执行（http.rs:418→426）；REST 与 WS 共用同一 `list_events(after, run_id, 200)` 游标语义，`after = event.sequence` 仅在 send 成功后推进（http.rs:468-472）——at-least-once、无乱序、重连补拉一致。
6. **错误脱敏**: AppError::internal 响应体固定 "Unexpected server error"（error.rs:33-40），细节只进日志；`event_stream_corrupt` 携带 sequence + retryable:false（error.rs:70-79），HTTP 500 与 WS Close 4002 双通道一致，测试覆盖（tests.rs:246-323）。
7. **审计覆盖完备（经全库 grep `action: "..."` 复核）**: login 成功/失败（auth.rs:94 / storage auth.rs:77）、logout、`tool.approval`（审批决策，storage runtime.rs:660）、`tool.policy`、`run.cancel`（runtime.rs:1143）、`task.create`（tasks.rs:49）、`settings.ip_notifications.update/test`、`public_ip.notification`。唯一无审计的敏感端点是 get_ssh_access（只读，可接受）。
8. **ssh_access 解析兜底**: 读失败→error 字段、home 缺失→占位报告项、畸形行→invalid entry 不 panic（ssh_access.rs:68-95/108-177）；spawn_blocking 隔离阻塞 I/O（http.rs:262-265）；报告不含公钥原文（只指纹/类型/comment/选项），暴露面可控。
9. **e2e-harness 无生产泄漏面**: example 由 `required-features = ["e2e-harness"]` 门控（Cargo.toml:8-13），lib/bins 零 `#[cfg(feature = "e2e-harness")]`；`/__e2e/*` 后门路由仅在 harness 进程追加（e2e_harness.rs:389-396），不进 build_router；临时 DB + 127.0.0.1 绑定。
10. **04b-F8（悬空 artifact_ref）影响收窄**: 全路由清单无 evidence/artifact 下载端点——悬空引用仅是数据字段，无内容泄露/404 面。

---

## [P1] F1：审批信息面缺口——全部 API DTO 不暴露工具参数全文，workspace/process/sandbox 审批只能基于 sha256 哈希盲批（04b 交接线索① server 侧核实结论）

- 位置: crates/soloops-server/src/http.rs:378-401 + crates/soloops-domain/src/runtime.rs:143-157 + crates/soloops-storage/src/runtime.rs:1343-1365
- 置信度: 高
- 证据:
```rust
// domain/runtime.rs:145-157 —— 对外审批 DTO 全部字段，无 arguments
pub struct ToolCallSummary {
    pub call_id: String,
    pub name: String,
    pub arguments_sha256: String,
    pub approval_preview: Option<serde_json::Value>,   // 仅 managed deploy 有（04b 证实）
    pub risk: ToolRisk,
    pub policy: PolicyDecision,
    pub status: ToolCallStatus,
    ...
}
// storage/runtime.rs:1347 —— arguments_json 存在于内部 RuntimeToolCall，未上 API
let arguments: String = row.get("arguments_json");
```
全库 grep `arguments_json|argumentsJson` 7 处命中全部位于 soloops-storage 内部（runtime.rs:442/443/446/457/1347/1370 + database.rs:148），server/domain 任何响应 DTO 均无该字段；RunDetail（runtime.rs:16-24）不含 tool_calls，RuntimeSnapshot.toolCalls 即 ToolCallSummary。
- 问题: 审批是 SoloOps 唯一的人工控制点。`decide_tool_call` 审批 `workspace.create/replace`（写内容）、`process.exec`（执行命令）、`sandbox.exec` 时，Owner 经任何 API 能看到的仅是 callId/name/argumentsSha256/risk/policy；approval_preview 仅 managed deploy 工具经 `set_tool_call_approval_preview` 写入（04b 证实）。**Owner 无法知悉将执行的实际参数（路径/内容/命令行），informed consent 在真实后端下不成立**。04b"Owner 看到的参数必须是 tool_calls.arguments_json 全文"的前置假设在 server 层无数据源。摘要绑定（arguments_sha256 不可变 + 三层闭环，04b 正面确认）反而成为反讽：绑定的正是 Owner 看不到的文本。
- 建议: ToolCallSummary 增加 `arguments_json`（可带截断上限），或为 waiting_for_approval 的调用提供专用审批详情端点；前端审批 UI 必须展示与 hash 绑定的同一份参数文本。

## [P2] F2：decide_tool_call 重复提交/已决状态一律映射 500 internal_error——语义误导且 openapi 未声明任何错误响应（03-线索4 正式收录）

- 位置: crates/soloops-server/src/http.rs:389-400 + crates/soloops-server/src/http/error.rs:81-88 + docs/api/openapi.yaml:201-207
- 置信度: 高
- 证据:
```rust
// error.rs:81-88 —— StorageError catch-all：除 InvalidEventPayload 外全部 500
error => {
    error!(%error, "storage request failed");
    Self::new(
        StatusCode::INTERNAL_SERVER_ERROR,
        "internal_error",
        "Unexpected server error",
    )
}
```
openapi decision 端点 responses 只声明 `"200"`（:201-207）；同一时间戳重复 POST（前端双击/重试）时 storage decide_tool_call 返回冲突错误 → 500。
- 问题: 重复审批是真实可触发的客户端场景（双击按钮、超时重试），返回 500 让前端走"服务器故障"分支而非幂等确认/冲突提示；openapi 对该端点零错误响应声明，mock 与前端无法按契约处理。与 03-线索4 预期一致。
- 建议: error.rs 为 StorageError 的状态冲突类 variant 增加映射（409 conflict / 422），openapi 补 401/404/409 响应。

## [P3] F3：login 用户不存在时跳过 Argon2——timing 侧信道可枚举用户名；限流键 `username:IP` 可被用户名变体分散

- 位置: crates/soloops-server/src/http/auth.rs:70-85, :65
- 置信度: 高
- 证据:
```rust
let valid = if let Some(user) = user.as_ref() {
    // … Argon2 verify（数十至数百 ms）
} else {
    false            // 用户不存在：立即返回，无 Argon2 计算
};
let limiter_key = format!("{}:{remote}", input.username);
```
- 问题: 存在与不存在的用户名响应时间差 ≈ Argon2 计算时间，公网部署可远程枚举；限流桶按 `username:IP` 独立，换用户名即换桶。单 Owner 系统下 username 接近公开信息（soloopsctl owner-init 引导），实际收益低，故 P3。
- 建议: 用户不存在分支执行一次 dummy Argon2 verify；限流键改为仅 IP（或 IP 主键 + username 辅助）。

## [P3] F4：WS 事件流无服务器心跳、无 Close 帧的 DB 错误断连、session 吊销不终止已建立流

- 位置: crates/soloops-server/src/http.rs:447-500
- 置信度: 高
- 证据:
```rust
// http.rs:475-486 —— 非 InvalidEventPayload 的 DB 错误：日志后直接 break，无 Close 帧
Err(error) => {
    error!(%error, "failed to read websocket events");
    if matches!(error, StorageError::InvalidEventPayload { .. }) { …Close 4002… }
    break;
}
```
stream_events 循环内从不调用 require_owner；服务器从不发送 Ping（浏览器不会主动发），仅靠客户端消息驱动 select。
- 问题: ① 中间代理空闲超时会静默断开，客户端只能在下次事件 send 失败时察觉；② 可恢复的瞬时 DB 错误与致命错误同表现（裸断开），客户端无法区分重连策略；③ logout/吊销 session 后已建立的 WS 流继续推送事件直到自然断开——单 Owner 自用影响有限。重连语义本身健康（after=sequence 游标 + at-least-once），故 P3。
- 建议: 周期性 Ping（如 30s）；瞬时 DB 错误改发 Close 4001/retryable 让客户端退避重连；每 N 个 tick 复验 session。

## [P3] F5：02-F7 server 侧正式收录——ssh_access 读取/解码/解析三层无上限（兜底结构完整，实际触发面需要本地写权限）

- 位置: crates/soloops-server/src/http/ssh_access.rs:79, 215-223, 290-302（02-domain-protocol.md F7 来源）
- 置信度: 高
- 证据:
```rust
// :79 无大小上限的全量读取
match std::fs::read_to_string(&path) {
// :215-223 超长 base64 行无界解码分配
STANDARD.decode(data).or_else(|_| …STANDARD_NO_PAD.decode(data)…)
// :290-302 恶意 blob 可产出 ~4x 内存的 fields Vec（每 4 字节一个空 string 切片引用）
while rest.len() >= 4 { let length = u32::from_be_bytes(…) as usize; … }
```
- 问题: authorized_keys 属本地文件（注入前提是本地已有写权限），端点仅 Owner 可达且 spawn_blocking 隔离——02-F7 的"内存放大面"在 server 侧维持 P3 评级：非远程可控输入。一旦本地被入侵，一个 GB 级文件或超长行可放大内存占用（read + decode + fields 三层各自分配，峰值 ~2-4x 文件大小）。
- 建议: `read_to_string` 前按 `metadata.len()` 设上限（如 10MB）；decode 前按行 len 设上限；fields 累计字节设上限。

## [P3] F6：home_dir 依赖 HOME/USERPROFILE 环境变量——systemd 等服务环境下可能未定义，SSH 审计端点静默降级为"home could not be determined"

- 位置: crates/soloops-server/src/http/ssh_access.rs:357-362
- 置信度: 中
- 证据:
```rust
fn home_dir() -> Option<PathBuf> {
    let variable = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    std::env::var_os(variable)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}
```
- 问题: Linux systemd 服务默认环境常不含 HOME（取决于 User=/systemd 版本），api 进程的 user authorized_keys 将**永不扫描**，报告只有 error 字段提示"home directory could not be determined"——SSH 审计功能在标准部署下可能整体静默失效（Windows 侧 administrators_authorized_keys 是绝对路径，不受影响）。无 getpwuid fallback、无配置覆盖项。
- 建议: fallback `getpwuid_r` 或增加 `SOLOOPS_SSH_HOME` 配置；home 缺失时在报告顶层加显式告警字段。

## [P3] F7：login 失败路径的审计写失败会把 401 变成 500

- 位置: crates/soloops-server/src/http/auth.rs:89-100
- 置信度: 高
- 证据:
```rust
state.database.write_audit(AuditEntry { … outcome: "failure", … }).await?;  // ← ? 上抛
return Err(AppError::unauthorized("invalid_credentials", …));
```
- 问题: DB 故障时失败登录返回 500 internal_error 而非 401——语义上"拒绝"被降级为"服务器错误"，且与攻击者观测面（500 vs 401）产生混淆。触发前提是 DB 写失败（此时整体已不健康），影响小，P3。
- 建议: 审计写失败仅 log，仍返回 401。

## [P3] F8：openapi 契约漂移集合（/livez 缺失、additionalProperties 声明与现实相反、events limit 未记录、死状态收录）

- 位置: docs/api/openapi.yaml（:563-580 RunStatus、:664、整体）；http.rs:95, 440-443
- 置信度: 高
- 证据:
```yaml
# openapi.yaml:563-566 —— RunStatus 枚举含 draft（02-F4 死状态正式收录，paused 同）
RunStatus:
  type: string
  enum:
    - draft
    …
    - paused        # :572
# openapi.yaml:664 —— 严格声明
ApprovalDecisionRequest:
  …
  additionalProperties: false
```
```rust
// http.rs:95 —— /livez 路由存在，openapi 无此 path
.route("/livez", get(health))
// http.rs:440-443 —— REST 事件固定 limit 200，openapi /api/events 无 limit 说明
let items = state.database.list_events(after, query.run_id.as_deref(), 200).await?;
```
- 问题: ① **02-F4 server 侧正式收录**: RunStatus 全 15 值（含死状态 draft/paused）在 openapi.yaml:563-580 与 domain serde `rename_all="snake_case"`（run.rs:24-42）逐一吻合，契约测试把死状态强制传播到前端与 OpenAPI（domain/tests/contract_files.rs），删除死状态需三方同步；② `ApprovalDecisionRequest` 声明 `additionalProperties: false` 但 Rust serde 未开 `deny_unknown_fields`——实际宽容，契约声明严格（按 openapi 生成客户端会假设拒绝多余字段）；③ `/api/events` 服务端静默截断至 200 条且无"还有更多"标志（客户端只能靠 items<200 推断），契约未记录；④ `/livez`（http.rs:95）与 login 的 429、metrics 的 404（disabled）均未收录。
- 建议: 修漂移项①②③④；若删除 draft/paused 需同时改 domain/run.rs、openapi.yaml、contracts.ts 与契约测试。

## [P3] F9：WS 事件序列化失败进入"每轮 poll 重试"式死循环刷屏

- 位置: crates/soloops-server/src/http.rs:460-467
- 置信度: 中（逻辑成立；EventEnvelope 字段全为 String/i64/合法 Value，实际触发面趋零）
- 证据:
```rust
Err(error) => {
    error!(%error, "failed to serialize event");
    break;                     // 仅跳出 for events 循环，after 未推进
}
```
- 问题: 序列化失败只 `break` 内层 for，`after` 游标停在坏事件之前，`select!` 下一 tick 重新拉到同一批事件再次失败——每 event_poll_ms 一条 error! 日志，无限循环。语义上应与 InvalidEventPayload 同类（不可自愈）却无 Close 帧、无计数上限。
- 建议: 序列化失败视同流损坏：发 Close 4002 并退出，或至少加连续失败计数熔断。

## [P3] F10：`build_router`（pub API）在 SMTP 配置畸形时 panic——`.expect` 语义陷阱

- 位置: crates/soloops-server/src/http.rs:74-78
- 置信度: 高
- 证据:
```rust
pub fn build_router(database: Database, config: AppConfig) -> Router {
    build_router_and_notifications(database, config)
        .expect("notification service construction failed")
        .0
}
```
- 问题: SmtpMailer::new 对畸形配置（SOLOOPS_SMTP_FROM 非法 mailbox、password env 未设等）返回 Err——`build_router` 直接 panic。生产 bin（bins/soloops-api/src/main.rs:20）走 `build_router_and_notifications`（Result，02 章已审）不受影响；但该 pub 函数是 crate 对外的"便捷"入口，tests.rs:24/258 与 e2e_harness.rs:388 使用它——第三方集成者拿到的是启动期 panic 而非错误。
- 建议: `build_router` 返回 `Result<Router>` 或文档标注 panic 条件；内部统一。

## [P3] F11：e2e_harness 桩内两处 `.expect` panic 面 + `/__e2e/*` 无认证（测试基础设施，收录 04a 登记）

- 位置: crates/soloops-server/examples/e2e_harness.rs:255, 289, 389-396
- 置信度: 高
- 证据:
```rust
// :255（provider 桩内）
.expect("managed deployment proposal result exists");
// :289
.expect("managed deployment evidence exists");
```
- 问题: goal 含 "managed deployment" 而 journal 缺对应 tool_result/evidence 时 provider 桩 panic——worker task 死亡（JoinError 在 stop_worker 时记入 errors），`/__e2e/status` 可观测但 harness 不会主动退出，e2e 用例只会超时而非快速失败。`/__e2e/shutdown` 任何人可 POST（测试进程，127.0.0.1，可接受）。**feature 泄漏面结论: 无**——examples 由 `required-features` 门控，lib/bins 零 cfg 引用（见正面确认 9）。
- 建议: 桩内 expect 改为返回 ProviderError，让 e2e 快速失败。

## [P3] F12：secure_cookies 与生产语义仅由 `SOLOOPS_ENV` 字符串等值决定——默认 development 即无 Secure 标志

- 位置: crates/soloops-server/src/http/config.rs:77, :40
- 置信度: 高
- 证据:
```rust
Ok(Self {
    secure_cookies: environment == "production",
    environment,            // env("SOLOOPS_ENV", "development")
```
- 问题: HTTPS 反代部署但忘设 `SOLOOPS_ENV=production` 时，session cookie 不带 Secure——公网链路降级时 token 可经明文泄露。`secure_cookies` 无独立开关，且 readiness/doctor 不校验"HTTPS 却非 production"组合。
- 建议: 独立 `SOLOOPS_SECURE_COOKIES` 覆盖项 + 启动日志显式输出 cookie 安全属性。

## [P3] F13：`/metrics` 无认证且默认启用（SOLOOPS_METRICS_ENABLED 默认 true）

- 位置: crates/soloops-server/src/http.rs:97, 166-175 + config.rs:88
- 置信度: 高
- 证据:
```rust
.route("/metrics", get(metrics))
…
async fn metrics(State(state): State<AppState>) -> Result<Response, AppError> {
    if !state.config.metrics_enabled { return Err(AppError::not_found(…)); }
```
- 问题: 公网部署时任何人可读请求计数/认证失败计数/WS 客户端数——暴露部署存在与活跃度（信息面小，无任务/数据内容）。metrics_enabled=false 时返回 404（此行为 openapi 未记录）。
- 建议: 文档要求公网部署关闭或经反代路径保护；metrics 端点可要求 owner session 或本地绑定。

---

## 前序线索核查答复

| 线索 | 答复 |
|---|---|
| 02-F3（AppConfig CWD 相对路径） | **根因确认**: `absolute_path`（config.rs:194-200）对相对路径一律 `std::env::current_dir()?.join(path)`，默认值 `var/db/soloops.db`、`apps/web/build`（:82-83）均受影响；`dotenvy::dotenv()`（:39）自 CWD 向上找 .env，换目录启动 = 换 .env/换 DB。02-F3 成立，本 crate 侧证据已闭合（bins 侧 02 章已审）。e2e_harness 的 web_dist 默认 `apps/web/build`（e2e_harness.rs:324）为同族测试便利。 |
| 02-F4（draft/paused 死状态传播） | **正式收录为 F8 子项①**: openapi.yaml:563-580 枚举 15 值与 run.rs serde `rename_all="snake_case"` 逐字吻合，含 draft/paused。 |
| 02-F7（ssh_access 无上限） | **正式收录为 F5**: server 侧读取/解码/解析三层均无界，维持 P3（本地文件前提）；兜底结构完整（正面确认 8）。 |
| 03-线索4（decide 重复提交→500） | **正式收录为 F2**（升 P2）: error.rs:81-88 catch-all 映射确认；openapi decision 端点零错误响应声明。 |
| 03-线索5（session 生成） | **全部达标**: CSPRNG `rand::random::<[u8;32]>` = 256bit（auth.rs:166-168）；SHA-256 hex 落库（:170-172）；`find_session_owner` SELECT+UPDATE 非同事务在 server 的调用频率 = 每认证请求 1 次 + WS 升级 1 次（升级后不再调用，F4 子项③）；写放大影响归 03-F13。 |
| 04a 事件 payload 键名表 | **server 侧零漂移**: WS 与 REST 均对 storage EventEnvelope 原样 `serde_json::to_string`/Json 序列化（http.rs:461/440-444），payload 为透传 Value 无二次映射——04a 表列出的 `{"taskId","status"}`/`{"from","to","workerId","reason"}`/`{"preview"}`/`{"summary","steps"}`/`{"callId"}`/`{"callId","summary","workspaceRevision"}`/`{"callId","category","summary"}`/`{"outcome","summary"}` 与 EventEnvelope 外层键（sequence/id/runId/type/payload/createdAt）即前端可依赖的全部契约。**工具名（如 managed.deploy.apply）不在事件 payload 中**——只存在于 RuntimeSnapshot.toolCalls[].name，e2e 失败 2 归因不变（04a-F3）。 |
| 04b 交接线索①（审批 UI 数据源） | **server 侧结论升级为 F1[P1]**: arguments_json 无任何 API 暴露面，非 managed 审批仅 sha256 可见。 |
| 04b 交接线索③（evidence 下载端点） | **不存在下载端点**——04b-F8 悬空 artifact_ref 无内容泄露面（正面确认 10）。 |

---

## 路由 × 认证 × 校验 × 审计 核对表

| 路由 | 方法 | 认证 | 输入校验 | 审计 | 错误映射 |
|---|---|---|---|---|---|
| /healthz, /livez | GET | 无（健康探针，设计如此） | — | — | 200 固定 |
| /readyz | GET | 无 | — | — | DB 错→500 |
| /metrics | GET | 无（F13） | — | — | disabled→404 |
| /api/auth/login | POST | 无（自身验证）+ 限流 5/60s | normalize: username≤64/password≤1024 | auth.login 成/败（F7） | 401/429 |
| /api/auth/logout | POST | require_owner | — | auth.logout | 401 |
| /api/auth/session | GET | require_owner | — | — | 401 |
| /api/tasks | GET/POST | require_owner | CreateTaskRequest.normalize title≤160/goal≤20000 | task.create | 400/401/201 |
| /api/tasks/{taskId} | GET | require_owner | path string | — | 401/404 |
| /api/runs/{runId} | GET | require_owner | path string | — | 401/404 |
| /api/runs/{runId}/runtime | GET | require_owner | path string | — | 401/404 |
| /api/runs/{runId}/cancel | POST | require_owner | path string | run.cancel | 401 |
| /api/runs/{runId}/tool-calls/{callId}/decision | POST | require_owner | ApprovalDecisionRequest.normalize reason≤1000 | tool.approval（storage） | **500 catch-all（F2）** |
| /api/events（REST+WS 同 URL） | GET | require_owner（WS 在握手前，:418） | after≥0 clamp; runId 可选；固定 limit 200 | — | corrupt→500+retryable:false / WS Close 4002 |
| /api/settings/ip-notifications | GET/PUT | require_owner | recipients: lettre 校验+去重+≤20；enabled 需 SMTP | settings.ip_notifications.update | 400/401/409 |
| /api/settings/ip-notifications/test | POST | require_owner | settings.recipients 非空前置 | settings.ip_notifications.test | 401/409/502 |
| /api/settings/ssh-access | GET | require_owner | —（读本机文件） | 无（只读） | 401/500 |
| 静态 fallback | GET | 无 | ServeDir(存在时) | — | 404→index.html |

中间件顺序（外→内）: CatchPanic → Trace → SetRequestId(x-request-id) → PropagateRequestId → enforce_origin → observe_request → handler。请求 ID 贯穿成立。

---

## 覆盖清单

| 文件 | 行数 | 状态 |
|---|---|---|
| crates/soloops-server/src/http.rs | 503 | 已审（穷尽；路由/WS/decide/events/normalize_recipients 逐行） |
| crates/soloops-server/src/http/config.rs | 246 | 已审（穷尽；02-F3 根因闭合；test 段略读） |
| crates/soloops-server/src/http/auth.rs | 172 | 已审（穷尽；限流/token/cookie/logout 逐行） |
| crates/soloops-server/src/http/ssh_access.rs | 548 | 已审（穷尽，含测试段；02-F7 收录） |
| crates/soloops-server/src/http/notifications.rs | 534 | 已审（穷尽，含测试段；SMTP 注入面闭合） |
| crates/soloops-server/src/http/error.rs | 97 | 已审（穷尽） |
| crates/soloops-server/src/http/telemetry.rs | 68 | 已审（穷尽） |
| crates/soloops-server/src/lib.rs | 6 | 已审 |
| crates/soloops-server/src/http/tests.rs | 446 | 已审（穷尽；覆盖质量评估见下） |
| crates/soloops-server/examples/e2e_harness.rs | 437 | 已审（穷尽；后门/泄漏面结论见正面确认 9 + F11） |
| crates/soloops-server/Cargo.toml | 45 | 已审（feature 门控确认） |
| docs/api/openapi.yaml | 833 | 已审（穷尽；与 serde 逐字段 diff，仅 F8 所列漂移） |
| domain 对照（runtime.rs/event.rs/requests.rs/error.rs/run.rs 枚举段） | — | 已读核对（02 章已审，此处仅取 serde 契约） |
| bins 调用面（soloops-api/main.rs） | — | grep 确认走 build_router_and_notifications |

**tests.rs 覆盖质量**: 高——未认证 401、跨源 403、限流 429（含 limiter 清理）、cancel、runtime 未启动 404、WS 回放（断言 event.type=run.created）、corrupt 流双通道（500+retryable:false / Close 4002）、recipients 归一化去重小写、ssh-access 401+自洽性+指纹格式。缺口: decide_tool_call 重复提交（F2 触发面）、logout、session 端点、enforce_origin 对 GET 的豁免边界、WS 游标推进/断线重连。

**发现统计**: P1 × 1（F1 审批无参数数据源）；P2 × 1（F2 decide 冲突→500）；P3 × 11（F3 login timing、F4 WS 保活/吊销、F5 ssh 无上限[02-F7 收录]、F6 HOME 依赖、F7 审计失败掩盖 401、F8 契约漂移[02-F4 收录]、F9 序列化死循环、F10 build_router expect、F11 e2e 桩 expect、F12 secure_cookies、F13 metrics 默认开）。合计 13 条，编号 F1-F13 连续无弃用。
