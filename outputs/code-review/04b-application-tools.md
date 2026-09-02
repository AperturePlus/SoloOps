# 04b — Application 工具层审查（soloops-application：tools.rs + 工具/审批测试 + execution.rs 工具接口面）

- **审查节点**: 全代码库深度审查 · 第 4b 节点（应用工具层）
- **审查对象**: 当前工作区状态（分支 `feature/agent-page-polish`，含未提交变更，同 00-baseline.md）
- **审查方式**: tools.rs 1730 行分 5 段（1-400 / 401-800 / 801-1200 / 1201-1600 / 1601-1730）全部逐行精读；tests/workflow.rs 257 行穷尽、managed.rs 工具路径复看；execution.rs 工具接口面（30-94 策略/审批分支、95-260 执行主循环、262-460 execute_tool_cancellable/finish_tool_output、795-895 恢复收尾）交叉核对；storage/runtime.rs 的 start_tool_call_with_recovery/finish_tool_call/arguments_sha256 链为取证性读取（其自身缺陷归 03/04a）；hostd.rs 客户端参数构造（185-300）与 hostd 侧 MAX_TOOL_OUTPUT_BYTES 常量为契约核对取证（hostd 侧缺陷归 hostd 节点）。panic 面/阻塞调用经 ripgrep 全量定位后逐个判断。每条发现片段写入前均回读原文核实。未修改任何文件。
- **验证证据**: rust-analyzer 工作区零诊断（基线 §2）；tools.rs 生产代码（1-1325 行，`#[cfg(test)]` 之前）`unwrap/expect/panic!/unreachable!` 零命中，全部命中位于 1326-1730 的测试模块；阻塞 std::io 仅存在于 atomic_publish 的 spawn_blocking 内（:748-763）。`cargo test -p soloops-application --lib` 由 04a 同日验证 35/35 通过（基线 §1.3），本节点未重复运行（只读纪律下无代码变更）。

**范围小结（供后续节点快速对齐）**:
- tools.rs 是全仓库质量最高的文件之一：路径约束双保险（checked_join 组件级 symlink 拒绝 + canonicalize 组件前缀比对 + 末点类型校验）、原子写（tempfile + persist_noclobber + spawn_blocking）、双预算截断（字节 + 2000 行、不切半行、多字节安全）、生产零 panic 面。
- 审批精确匹配闭环成立且无 TOCTOU：审批摘要拷贝自不可变的 tool_calls 行，执行前双点校验（engine 内存比对 + storage SQL CAS，见正面确认 #1）。
- max_output_bytes 客户端/宿主端契约对齐：client 发送 config 值（1024..=10MB 校验域）= hostd 接受域（unix.rs:36/649、sandbox.rs:9/101 同为 1024..=10MB）；02-线索 8 的 managed 无下限不一致纯属 hostd 内部问题（本节点已核实 client 永不可能发出越界值）。
- 全部 8 个前序跨层线索中与 4b 相关的 4 条（02-线索 8、03-线索 6 已由 04a-F5 答、03-线索 8、04a 对 4b 的三条移交项）均已正式核查，见"前序线索核查答复"。

---

## 前序线索核查答复（不占发现编号）

| 线索 | 答复 |
|---|---|
| 02-线索 8 / 02-正面确认 5（max_output_bytes 两端规则不一致） | **客户端半边已核实无缺口**。client 在 6 个 HostdAction 变体中统一发送 `context.max_output_bytes`（hostd.rs:117-129/208/281），其值 = `SOLOOPS_MAX_TOOL_OUTPUT_BYTES`（config.rs:161-166，默认 65536，parsed 强制 1024..=10MB）；hostd process/sandbox 接受域 1024..=10MB（unix.rs:36/649、sandbox.rs:9/101）——**client 合法配置域 ⊆ hostd 接受域**，运行期不可能失配。managed 路径 hostd 只拒 0 与 >10MB（unix.rs:531）无 1024 下限——但 client 下限 1024 使该缺口不可达；不一致本身留给 hostd 节点统一（线索仍有效）。hostd 返回 OutputLimit 错误时 client 以 `limit: context.max_output_bytes` 构造 ToolError::OutputLimit（hostd.rs:229-232/:365），语义一致。 |
| 03-线索 8（recover_safe_runs 五类 risk 归宿，privileged=tools.rs:855/869） | **确认无误**。tools.rs:855（apply）/ :869（rollback）= ToolRisk::Privileged + 600s 超时；plan(:837)/status(:843) = ReadOnly + 60s。恢复接力闭环经 04a-F 线索 7 答复 + managed.rs:199-237 集成测试成立，本节点复看无补充。 |
| 04a-F1 移交项（修复时注意 tool_result.callId 与模型可见 id 一致性） | **已核对锚点**：journal tool_result payload = `{"callId","result","evidenceId"}`（runtime.rs:867），callId 即 tool_calls.call_id（同一来源 execution.rs:211 传入）；finish 的 CAS `WHERE status='running'`（runtime.rs:831）保证 journal 行与 tool_calls 行只能由同一事务产生——修复 F1（call_id 作用域化）时只需保证 start 侧 NewToolCall.call_id 与 journal callId 同源即可，当前两点均出自 `call.summary.call_id`，一致性成立。 |
| 04a 移交项（tools.rs:748 spawn_blocking 正确；:1543/:1631 同步 fs 在测试代码） | **确认**。:748 atomic_publish 的 tempfile+write+sync+persist 全部在 spawn_blocking 内（:749-763 use std::io::Write 局限在闭包内），是 tools.rs 唯一的同步 I/O，隔离正确；:1543（process_exec_truncates 测试）/ :1631（multibyte 测试）均在 `#[cfg(test)]`。生产路径其余文件操作全部 tokio::fs。 |
| 03-线索 6（ordinal） | 已由 04a-F5 正式答复（批内局部编号 + 排空不变量），本节点无补充。 |

---

## 正面确认（无发现项，供后续节点免重复核查）

1. **审批精确匹配无 TOCTOU（审查要点 1 核心结论）**。链条：① `persist_model_response` 落库时写 `arguments_sha256`（runtime.rs:446），此后**全库无任何 UPDATE tool_calls.arguments_json/arguments_sha256 的路径**（03 章节已确认 tool_calls 只有状态/结果列变更）；② `decide_tool_call` 的 tool_approvals 行**拷贝**当刻 call 行的摘要（runtime.rs:643-649 `.bind(&call.arguments_sha256)`）——批准即绑定批准时刻的参数指纹；③ 执行前 engine 内存比对 `approval_arguments_sha256 == arguments_sha256`，不等即 `fail_tool_call("approval_mismatch")`（execution.rs:71-85）；④ `start_tool_call_with_recovery` 的 SQL 再以 `AND tool_approvals.arguments_sha256 = tool_calls.arguments_sha256` CAS 双保险（runtime.rs:758-764）；⑤ hostd 侧授权查询四元匹配（03-正面确认 10）。**批准与执行之间参数不可变 ⇒ TOCTOU 窗口不存在**。拒绝路径完备：Deny → `fail_tool_call("owner_denied", model_visible=true)` 后 run 继续供模型自纠（execution.rs:86-94）；managed 预览非法（摘要/状态不符）→ `fail_tool_call_before_start("invalid_deployment_proposal")` 不进审批（execution.rs:113-123 + :311-313）。
2. **Workspace 路径约束为纵深防御（审查要点 2）**。写路径：`normalized_workspace_write_path` 拒 ParentDir/RootDir/Prefix（tools.rs:702-704）+ `resolve_new`→`checked_join` 逐组件 `symlink_metadata` 拒任何 symlink 组件（:1182-1186）+ `atomic_publish` 以 `persist_noclobber`（create）原子落盘（:758-761）；读路径：`resolve_existing` 在 checked_join 之上再 `canonicalize` 双侧 + **组件级** `starts_with`（非字符串前缀，"/ws-evil" 不会误判为 "/ws" 子路径，:1145-1148）+ 末点 symlink/类型校验（:1150-1156）；list/search 的 BFS 跳过 symlink（:287-289/:482-484）；工作区为 `workspace_root/run_id` 每运行隔离（engine.rs:135-138，run_id 为系统生成 UUID）。Windows 下 canonicalize 双侧同为 `\\?\` 前缀，比对一致。
3. **注入面干净（审查要点 3）**。全 crate 无 shell 字符串拼接：process.exec/sandbox.exec 的 arguments 以**原始结构化 JSON** 经长度上限帧（MAX_REQUEST_BYTES/MAX_RESPONSE_BYTES，protocol crate 常量）发往 hostd（hostd.rs:201-214/:274-287），args 是数组元素而非命令行文本，无引号/换行/参数注入面；程序别名与参数白名单解析全部在 hostd 特权域内执行（设计使然）。workspace 工具参数经 serde `deny_unknown_fields` 严格解析（:242/:329/:431/:539），数值 clamp（entries≤500、lines≤500、matches≤200）。
4. **输出双预算截断 + 证据完整性（审查要点 4，process/sandbox/read 三工具）**。`truncate_output`：字节预算 + MAX_OUTPUT_LINES=2000 行预算、从不切半行、单行超限时按字符边界裁（:1292-1295 + char_bounded_slice 的 `limit.min(text.len())` 与边界推进，:1306-1324，无 panic 面）；超限时**全文**写入 artifact（:1263），evidence 的 content_sha256 覆盖截断前全文（tools.rs:961/:1008 先 hash 后截断），Owner 可凭 artifact 取回全文。事件流只含 callId/summary/revision（03-正面确认 12），journal tool_result 存 bounded 内容。
5. **错误传播与超时分类正确（审查要点 5）**。工具失败一律 `fail_tool_call(category, model_visible)`，生产路径无 panic；超时 = `min(descriptor.timeout, max_tool_duration_ms)`（execution.rs:378-379），managed apply/rollback 超时 → `block_managed_change`（"outcome is unknown"）保守终结（:263-277，managed.rs:386-418 有集成测试）；RecoveryRequired 类错误同样触发 block。LeaseLost/Cancelled 以 ToolExecutionFailure 区分、不落失败记录（:233-234），失租的 in-flight 写由 spawn_blocking 分离续跑 + 恢复探查收敛（见 #6）。
6. **恢复幂等闭环（审查要点 7）**。`finish_tool_call` 单事务 + `WHERE status='running'` CAS + rows_affected 复核（runtime.rs:828-850）⇒ 重放不可能产生重复 evidence/journal 行；evidence id 与 journal payload 同事务同源派生（:851-867 注释明示意图）。恢复矩阵：workspace 写中断 → probe 三态（Completed=以恢复元数据补记结果**不重复执行** / RetrySafe=重置 pending 重执行一次 / Unsafe=Blocked），残留 temp 文件按 **call_id 摘要前缀**精确清理（execution.rs:824-840，`workspace_temp_prefix` tools.rs:737-740），不会误删他人文件；managed 中断 → durable 重放且 `executor.calls==0`（managed.rs:199-237）；process 中断 → storage 层直接 Blocked（03-线索 8），不重放非幂等操作。`persist_noclobber` 失败时 tempfile 由 Drop 自动删除，正常路径无残留。
7. **Evidence 无秘密泄漏路径（审查要点 4 脱敏面）**。ToolEvidence 只含 kind/summary/artifact_ref/content_sha256（tools.rs:84-89）；summary 为路径/退出码/摘要等非敏感文本；SecretRef 全链脱敏（04a 已核，本范围工具不触碰密钥）。stdout/stderr 原文入 journal/artifact 属设计内行为（单 Owner 机器），未见额外放大面。

---

## 发现列表

## [P2] F1：workspace.search / workspace.list 结果不经 truncate_output——单次工具值可达数百 KB，突破 max_tool_output_bytes 预算契约，且与模型可见的 "Output limit: 65536 bytes" 工具描述直接矛盾
- 位置: crates/soloops-application/src/tools.rs:501-521（search 构造 value）、:283-311（list）、:509（preview 按**字符**取 500）；crates/soloops-application/src/model.rs:235-241（output_limit 拼入描述文本）；crates/soloops-storage/src/runtime.rs:838（result_json 无上限落库）
- 置信度: 高
- 证据:
  ```rust
  // tools.rs:506-510 —— preview 按字符计，200 条上限
  matches.push(json!({
      "path": path.strip_prefix(&canonical_workspace).unwrap_or(&path).to_string_lossy().replace('\\', "/"),
      "line": index + 1,
      "preview": line.chars().take(500).collect::<String>()
  }));
  // tools.rs:520-521 —— 直接进 value，无 truncate_output
  let summary = format!("Found {} matches", matches.len());
  let value = json!({"matches": matches, "truncated": matches.len() == limit});
  ```
- 问题: process.exec / sandbox.exec / workspace.read 的输出都经 `truncate_output` 双预算约束，但 search 与 list 的聚合 value 直接进 `ToolOutput.value` → `finish_tool_call` 的 `result_json` 落库无上限（runtime.rs:838）→ journal tool_result → 模型上下文（仅受 04a 已核的 256KB journal 投影间接限制）。量级：search = 200 条 ×（500 字符 preview，CJK 每字符至多 4 字节 ≈ 2KB + 路径）≈ **100-400KB**；list = 500 条 × 路径（Linux PATH_MAX 下深路径可 ~4KB/条）≈ 最多 ~2MB。同时模型在每个工具描述里被告知 "Output limit: 65536 bytes"（model.rs:236-240 拼入 description），该承诺对这两类工具不成立（实际预算见 F5 双轨问题）。影响：模型上下文被单工具挤占、DB 单行膨胀、预算统计（token 侧）低估，且模型对输出边界的认知失真。
- 建议: ① search/list 的序列化 value 一律经 `truncate_output`（Head 保持 + artifact 全文），复用既有机制；② 或在 `finish_tool_call` 层对 result_json 设字节上限；③ 与 F5 一并修：描述文本中的 Output limit 应取真实执行预算。

## [P2] F2：managed deploy 的"结果未知"分类只覆盖 Timeout——响应阶段连接失败与 250ms tick 中 is_cancelled 的 DB 错误同样造成结局未知/半途丢弃，却被记为普通 execution 失败并放行 run 继续
- 位置: crates/soloops-application/src/hostd.rs:214-216、:287-289（read_frame 失败→Execution）；crates/soloops-application/src/engine/execution.rs:263-277（仅 resuming/RecoveryRequired/Timeout 触发 block）、:386-396（is_cancelled 错误→Tool(Execution) 且丢弃 in-flight future）
- 置信度: 高（机制确证；触发需连接中途断开或瞬时 DB 错误，低频但现实）
- 证据:
  ```rust
  // hostd.rs:214-216 —— 响应阶段失败与"未开始"同为 Execution 类
  let response: HostdResponseV3 = read_frame(&mut stream, MAX_RESPONSE_BYTES)
      .await
      .map_err(|error| ToolError::Execution(format!("hostd response failed: {error}")))?;
  // execution.rs:263-267 —— 保守 block 只认三种
  let recovery_required = matches!(error, ToolError::RecoveryRequired(_));
  if managed_change
      && (resuming_managed_change || recovery_required || matches!(error, ToolError::Timeout))
  { self.block_managed_change( ... "outcome is unknown ..." ).await?; return Ok(true); }
  // execution.rs:390-395 —— tick 中 DB 错误伪装成工具失败
  if self.is_cancelled(run_id).await
      .map_err(|error| ToolExecutionFailure::Tool(ToolError::Execution(error.to_string())))?
  { return Err(ToolExecutionFailure::Cancelled); }
  ```
- 问题: managed apply/rollback 的错误路径先 `inspect_managed_change_operation`（execution.rs:236-261），Missing（hostd 尚未持久化 operation——**对端可能仍在执行、或将在客户端断开后才落库**）时只有 Timeout/RecoveryRequired/resuming 三类触发 "outcome is unknown" 的 Blocked。两类同性质场景漏网：① **read_frame 失败**（对端半途断开/响应超帧）：请求已送达、结局未知，但映射为 Execution → inspect 此刻通常 Missing → `fail_tool_call("execution")`，run 继续下一调用；稍后 hostd 完成 apply 并把 revision 转 active——产生与 03-线索 9 / 04a-F4 相同的"失败记录 + active revision"矛盾组合，且这次是**客户端触发面**（04a-F4 只覆盖恢复判定侧）；② **execute_tool_cancellable 的 250ms tick 中 `is_cancelled` 遇瞬时 DB 错误**（SQLite busy 等）：`map_err` 把 RuntimeError 压成工具级 Execution 错误，`?` 返回时**丢弃正在执行的 future**——managed 场景即断开 socket（同①）；workspace 写场景 spawn_blocking 分离续跑、文件实际写成功但调用被记为失败，模型下轮重试 create 将撞 "already exists"。基础设施抖动被记成模型可见的工具失败，误导模型自纠方向。
- 建议: ① hostd.rs 区分"请求未发出"（connect 失败，可安全 fail）与"发出后失败"（read_frame/write_frame 错误，对 managed 应映射为 RecoveryRequired 语义或独立 UnknownOutcome 类别）；② engine 对 managed change 在 `Missing + 任何非确定失败` 时一律保守 block（与 Timeout 同路径），把"确定未开始"做成显式白名单而非默认；③ `is_cancelled` 的 Err 应原样上抛 RuntimeError（让 run_once 失败重试），不得伪装为工具失败并丢弃 in-flight future。

## [P3] F3：process.exec / sandbox.exec 参数在 application 侧零校验——schema 声明的所有边界（args≤128、timeoutMs≤60000、env、cwd）仅是给模型的提示，执行前无任何强制
- 位置: crates/soloops-application/src/tools.rs:929-935（sandbox schema）、:977-983（process schema）；:941-947（execute 只取 program 做摘要，其余原样透传）；hostd.rs:206-209/:279-282（arguments 整体下发）
- 置信度: 高
- 证据:
  ```rust
  // tools.rs:941-947 —— 仅读 program，无 schema 校验
  async fn execute(&self, context: &ToolContext, arguments: Value) -> Result<ToolOutput, ToolError> {
      let program = arguments
          .get("program")
          .and_then(Value::as_str)
          .unwrap_or("sandbox process")
          .to_owned();
      let output = self.executor.execute_sandbox(context, arguments).await?;
  ```
- 问题: workspace 系工具参数经 serde `deny_unknown_fields` + clamp 严格校验，而 host 工具的参数在 application 侧**完全不解析**（对比鲜明）：maxItems 128、timeoutMs 1..=60000、env 对象、cwd 等边界只存在于给模型看的 JSON Schema 文本中。真实执行链上唯一的强制点是 hostd（Linux 特权域）。这是有意设计（hostd 为唯一执法点），但意味着：① hostd 侧每漏检一项即成为事实缺口，无第二道防线；② 测试替身（FakeHostExecutor / e2e ScriptedHostExecutor）同样不校验，schema 与实现的一致性无任何测试钉住；③ 模型发出 10000 个 args 或 timeoutMs=999999 时，application 无法在审批预览/摘要中提示异常（审批界面展示的参数即原文）。
- 建议: 在 hostd.rs 客户端（或 tools.rs）加一层与 schema 同源的轻校验（args 数量、timeoutMs 区间、env 值为字符串），失败即 InvalidArguments 早退；至少把"hostd 必须逐项强制 schema 边界"写入 hostd 节点审查清单（见交接线索）。

## [P3] F4：workspace.read / workspace.replace 全文件载入内存且无大小预检——与 workspace.search 的 `metadata.len() > max_output_bytes` 预检不一致，预算上限仅间接来自 max_workspace_bytes
- 位置: crates/soloops-application/src/tools.rs:366-368（read：fs::read 全量 + from_utf8 二次拷贝）、:621（replace：read_to_string 全量）、:676（prepare_workspace_write 同样全量）、对照 :495-497（search 有预检）
- 置信度: 高
- 证据:
  ```rust
  // tools.rs:366-368 —— 无 metadata.len() 预检
  let bytes = fs::read(&path).await?;
  let text =
      String::from_utf8(bytes).map_err(|_| ToolError::Execution("file is not UTF-8".to_owned()))?;
  // tools.rs:495-497 —— search 有预检（对比）
  if metadata.len() > context.max_output_bytes as u64 {
      continue;
  }
  ```
- 问题: read/replace/prepare 三处把整个文件读入内存（read 还有 Vec→String 双份）。文件大小无直接上限，仅受工作区总预算兜底：默认 100MB（config.rs:167-172）下最坏 ~200MB 瞬时内存，operator 调到上限 10GB 后，sandbox.exec（挂载工作区）产出的巨型文件可让一次 workspace.read 直接 OOM worker——崩溃→租约过期→恢复重扫，反复在同一步骤上撞墙。search 已示范了正确的预检模式（跳过大文件），read 只需拒绝或分页。
- 建议: read 在 fs::read 前先 `metadata.len()` 预检（超过 max_output_bytes 时返回指向分页读取的错误/提示，或直接读前 N 字节按行截断）；replace/prepare 可用上限 + 流式 hash 计算替代全量 String。

## [P3] F5：descriptor.output_limit（64KB 硬编码常量）与真实预算 max_tool_output_bytes 双轨并行、互不联动——非默认配置下模型被告知的输出上限是错的，且该字段从不作为强制边界
- 位置: crates/soloops-application/src/tools.rs:27、:235（output_limit: 64*1024）；model.rs:235-241（唯一实质消费=拼入工具描述文本）；hostd.rs:357-365（错误映射的 limit 参数实际传 context.max_output_bytes 而非 output_limit）
- 置信度: 高
- 证据:
  ```rust
  // tools.rs:235 —— 硬编码
  output_limit: 64 * 1024,
  // model.rs:235-241 —— 仅入描述文本
  description: format!(
      "{} Risk: {:?}. Timeout: {} ms. Output limit: {} bytes.",
      descriptor.description, descriptor.risk,
      descriptor.timeout.as_millis(), descriptor.output_limit,
  ),
  ```
- 问题: 真实预算是 `config.budget.max_tool_output_bytes`（可配 1024..=10MB），而模型看到的 "Output limit" 恒为 65536——两者只在默认配置下恰好相等。operator 上调预算后，模型仍按 64KB 自我约束（浪费预算），下调后反而被 F1 揭穿的越界值打脸。output_limit 字段在执行路径上零消费（不做任何截断），是"看似有配置、实际无作用"的误导性字段。
- 建议: 二选一：① descriptor 构造时注入真实预算（ToolRegistry::standard 增加 budget 参数），使描述文本与强制值同源；② 删除 output_limit 字段，描述文本直接引用 max_tool_output_bytes。与 F1 一并修复。

## [P3] F6：Windows 目录联接（junction）可绕过 checked_join 的组件级 symlink 拒绝——list/search 遍历与 resolve_new 写路径对 junction 的处置依赖 Rust std 的 is_symlink 判定，末点 canonicalize 兜底只覆盖 read/replace
- 位置: crates/soloops-application/src/tools.rs:1182-1186（checked_join 依赖 file_type().is_symlink）、:287/:482（list/search 跳过判定）、:1160-1162（resolve_new 无 canonicalize 兜底）；对照 :1145-1148（resolve_existing 有兜底）
- 置信度: 低（取决于 Rust std 对 junction 的 is_symlink 判定口径与目标平台行为，未在本机实证；触发还需工作区内先存在 junction）
- 证据:
  ```rust
  // tools.rs:1182-1186 —— 唯一的组件级拦截依赖 is_symlink
  if let Ok(metadata) = fs::symlink_metadata(&cursor).await
      && metadata.file_type().is_symlink()
  {
      return Err(ToolError::PathViolation(relative.to_owned()));
  }
  ```
- 问题: 若 std 在 Windows 上未把 junction 计入 `is_symlink`（历史行为见 rust-lang/rust#75617 一线的长期议题），则：① list/search 的 BFS 会穿过工作区内的 junction 枚举/检索宿主机目录（`strip_prefix().unwrap_or(path)` 还会把外部路径原样发给模型）；② `resolve_new`（workspace.create）经 junction 父目录写出的文件实际落在工作区外，`atomic_publish` 的 tempfile_in(parent) 同样外泄——而 create 的 noclobber 与审批界面上的路径看起来完全无害。resolve_existing（read/replace/list 入口的 canonicalize 前缀比对）能把经 junction 解析后的真实路径拦下，是现成兜底；resolve_new 与 BFS 遍历没有这层。触发前提苛刻（需 allowlisted 程序或 Owner 先在工作区造出 junction，且生产目标平台是 Linux），故仅登记。
- 建议: 在 resolve_new 收尾补一次 `canonicalize + starts_with(canonical_root)` 与 resolve_existing 对齐（新路径此时不存在，可对**父目录**做 canonical 校验）；list/search 对 entry 的 reparse/junction 属性做显式跳过；补一条 Windows-only 单测钉住行为。

## [P3] F7：list/search 的 `truncated` 以 `len == limit` 判定——条目数恰好等于上限时误报"还有更多"
- 位置: crates/soloops-application/src/tools.rs:311（list）、:521（search）
- 置信度: 高
- 证据:
  ```rust
  // tools.rs:311 —— 恰好 200 条时 truncated=true
  let value = json!({"entries": entries, "truncated": entries.len() == limit});
  ```
- 问题: 收集循环在 `entries.len() >= limit` 时 break（:302-304），无法区分"到达上限且确实还有剩余"与"总数恰好等于上限"。后者会向模型误报 truncated=true，诱导其翻页/换参重查（对 list 是一次额外的全目录扫描）。search 同理（:511-517 break 时机与 :521 判定脱节）。
- 建议: break 前记录"是否因限额截断"（例如再 peek 一个 entry），或采用"多取一条再裁掉"的哨兵模式，使 truncated 与真实剩余严格一致。

## [P3] F8：run 工作区与 artifacts 目录无任何保留/清理策略——磁盘占用随 run 数无界增长（03-线索 10 的文件侧补充）
- 位置: crates/soloops-application/src/engine.rs:135-138（每 run 创建 `workspace_root/{run_id}`、`artifact_root/{run_id}`）；crates/soloops-application/src/tools.rs:1261-1263（truncate_output 每次截断落一个全文 artifact）；全 crate grep 无 run 目录删除路径（唯一文件删除是 cleanup_interrupted_write 的 temp 前缀，execution.rs:824-840）
- 置信度: 高
- 证据:
  ```rust
  // engine.rs:135-138 —— 只创建，无对应删除
  let workspace = self.config.workspace_root.join(&run_id);
  let artifacts = self.config.artifact_root.join(&run_id);
  tokio::fs::create_dir_all(&workspace).await?;
  tokio::fs::create_dir_all(&artifacts).await?;
  ```
- 问题: 03-线索 10 已登记"DB 无删 run 路径（events 只增不减）"；文件侧同构且更重：每个 run 的 workspace（上限 max_workspace_bytes，默认 100MB）与 artifacts（每次输出截断存全文，process 输出最大 max_output_bytes=10MB/次）永久留存，长期运行的部署磁盘无界增长。单 Owner 系统短期无感，但这是所有"预算"里唯一完全没有对账/告警的一个维度。另注：artifact 写入不 fsync（:1263），崩溃窗口内 evidence 引用的 artifact 可能缺失（恢复探查不覆盖 artifacts，Owner 侧表现为引用悬空——低概率，一并记录）。
- 建议: ① 为 workspace/artifact 目录设计 run 终态后的保留策略（如保留 N 个最近 run + 基于磁盘水位的清理，需与 03-线索 10 的 DB 行清理同批设计——evidence.artifact_ref 悬空检查应进 doctor）；② truncate_output 的 fs::write 补 fsync（artifact 是审计证据，值得持久化保证）。

## [P3] F9：审批/工具失败路径的引擎层测试缺口——Deny、approval_mismatch、invalid_arguments 在 application 测试中零覆盖，workflow.rs 仅 3 条 happy-path
- 位置: crates/soloops-application/src/tests/workflow.rs:12-257（全文仅 3 用例，全部 Approve→Succeeded）；src/tests 目录 grep `Deny|deny|approval_preview|invalid_arguments|PathViolation` 0 命中
- 置信度: 高
- 证据:
  ```text
  workflow.rs 三个用例（257 行）：
  :12  approval_workspace_evidence_and_finish_form_a_persisted_success_path  —— workspace.create 审批→执行→file_snapshot 证据→finish→Succeeded
  :104 approved_process_exec_uses_the_injected_host_executor                   —— process.exec 审批→executor.calls==1
  :179 sandbox_exec_is_registered_only_when_enabled_and_uses_exact_approval    —— sandbox 注册门控+审批+command_result 证据
  grep "Deny|deny" src/tests → 0 命中（Deny 决策路径无任何 application 层测试）
  ```
- 问题: 与 04a-F11 的四条缺口互补，本节点补充工具侧缺口：① **owner_denied 路径**（execution.rs:86-94：deny 后 fail_tool_call + run 继续供模型自纠）无测试——这是审批安全叙事的另一半；② **approval_mismatch**（:74-85）无测试（需 DB 篡改模拟，但这是防 TOCTOU 的关键断言，值得一条）；③ deny_unknown_fields 触发的 invalid_arguments 在引擎层的传播（fail_tool_call_before_start 类别与模型可见性）无测试；④ workspace.read/search 的 ReadOnly 自动放行路径无引擎层集成测试（仅 tools.rs 单测）。现有 3 条 happy-path 质量高（真实 Database + 脚本 provider + 断言到文件内容与证据 kind），扩展成本低。
- 建议: 按 ①-④ 各补一条最小集成测试（复用 workflow.rs 的 in-memory DB + ScriptedProvider 模式）；Deny 用例同时断言 run 未终结、模型收到 owner_denied 系统反馈。

---

## 覆盖清单

| 文件 | 行数 | 状态 |
|---|---|---|
| crates/soloops-application/src/tools.rs（生产 1-1325） | 1325 | 已审（穷尽，分 5 段；路径约束/原子写/截断/注册门控逐行；F1/F3/F4/F5/F6/F7/F8 触发点；生产零 panic 面、阻塞 I/O 仅 spawn_blocking 内） |
| crates/soloops-application/src/tools.rs（tests 1326-1730） | 405 | 已审（穷尽；8 测试函数 + 3 桩执行器逐行；unwrap/unreachable! 均为测试代码；覆盖评估见 F9） |
| crates/soloops-application/src/tests/workflow.rs | 257 | 已审（穷尽；3 用例质量高，覆盖缺口见 F9） |
| crates/soloops-application/src/tests/managed.rs（工具路径部分） | 复看 | 已审（interrupted replay :199-237 / timeout :386-418 与 04a 结论一致，无补充；Deny 缺口见 F9） |
| crates/soloops-application/src/engine/execution.rs（工具接口面） | 30-460/795-895 | 已审（接口取证：审批分支 57-131、prepare/start 145-216、execute_tool_cancellable 367-400、finish_tool_output 402-425、probe/cleanup/recovery 799-895；F2 触发点；其余归 04a） |
| crates/soloops-application/src/hostd.rs（客户端契约取证） | 185-300/357-365 | 已审（arguments 原样下发 + max_output_bytes 发送值/错误映射；其余归 04a） |
| crates/soloops-application/src/engine.rs（路径构造/摘要计算取证） | 135-146/356-405 | 已审（workspace/artifacts per-run 构造 F8；NewToolCall/length 截断防御与 04a-F1/F10 交叉核对一致） |
| crates/soloops-application/src/model.rs（output_limit 消费点取证） | 222-258 | 已审（F5；其余归 04a） |
| crates/soloops-application/src/config.rs（预算默认值取证） | 146-186 | 已审（max_tool_output_bytes/max_workspace_bytes 默认与合法域；其余归 04a-F9） |
| crates/soloops-storage/src/runtime.rs（审批摘要/finish 取证） | 62-141/446/620-675/744-900 | 取证性读取（arguments_sha256 链、start/finish 的 CAS 与同事务 evidence——正面确认 #1/#6 的依据；自身缺陷归 03） |
| bins/soloops-hostd/src/unix.rs / sandbox.rs（常量取证） | :36/:649 / :9/:101 | 取证性读取（MAX_TOOL_OUTPUT_BYTES=10MB 与 client 域比对——线索答复；其余归 hostd 节点） |

**发现统计**: P0 × 0；P1 × 0；P2 × 2（F1 search/list 绕过输出预算、F2 managed 结果未知分类不完整 + in-flight 丢弃）；P3 × 7（F3 host 工具参数零客户端校验、F4 read/replace 无大小预检、F5 output_limit 双轨失真、F6 Windows junction 边界[低置信度]、F7 truncated 恰等误报、F8 run 目录无保留策略、F9 审批失败路径测试缺口）。合计 9 条，编号连续无弃用。

**审查方法补充**: panic 面经 ripgrep 全量定位（56 命中）后逐个归类：生产段仅 3 处切片（:739 定长 hex 摘要、:1314/:1321 char 边界调整后切片 + :1307 limit.min(len) 防御）与 3 处 std::io（spawn_blocking 闭包内），其余全部位于 `#[cfg(test)]`；无 expect( 与 panic! 命中。审批 TOCTOU 结论依赖"tool_calls.arguments_json 无 UPDATE 路径"这一全库事实（03 章节 + 本节点 grep 复核）。e2e 失败 2 的 tools.rs 侧疑点（:826-843 plan=ReadOnly、:891-919 payload 键名）经全文精读确认与 04a-F3 的取证一致，无新增；call_id 冲突（04a-F1）在 tools.rs 无独立触发面（call_id 由 engine/storage 生成消费，tools.rs 仅经 ToolContext.call_id 用于 temp 前缀与 artifact 命名，批内重前缀理论碰撞面 = SHA-256 前 16 hex，可忽略）。未修改任何源码。
