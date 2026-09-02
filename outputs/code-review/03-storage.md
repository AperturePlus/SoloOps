# 03 — Storage 层审查（crates/soloops-storage）

- **审查节点**: 全代码库深度审查 · 第 3 节点（数据层）
- **审查对象**: 当前工作区状态（分支 `feature/agent-page-polish`，含未提交变更，同 00-baseline.md §0）
- **审查方式**: 范围内 16 个 .rs 文件（含测试，共 4944 行）+ 6 个 SQL 迁移文件全部精读；每条发现的代码片段均在写入前回读原文核实；SQL 参数化与 panic 面经 ripgrep 全景扫描；sqlx 行为结论经本地 registry 源码（sqlx-core/sqlx-sqlite 0.8.6）取证；未修改任何源码。
- **验证证据**: `cargo test -p soloops-storage` 退出码 0（24 passed / 0 failed，与基线 §1.3 一致）；`lsp-diagnostics` 对 8 个非测试源文件 0 errors / 0 warnings。
- **基线关系**: 00-baseline.md 的 e2e 失败（§1.9）其根因链路穿过本层（事件 payload 写入点），本文件不重复展开、在跨层线索给出锚点；02-domain-protocol.md 的跨层线索 1/2/3 已在本节点正式核查并在正文回答。
- **编号说明**: 发现按严重性排序，编号 F1–F16；文中交叉引用一律使用 F 编号。

**范围小结（供后续节点快速对齐）**：
- 实际文件树与任务描述有差异：`recovery.rs`、`managed.rs`、`concurrency.rs` 均位于 `src/database/tests/`（测试代码），非独立生产模块；生产代码为 `database.rs`（405）、`database/{auth,notifications,schema,tasks}.rs`、`runtime.rs`（1387）、`deployment.rs`（641）。
- **非测试代码零 panic 操作符**（unwrap/expect/panic!/unreachable!/todo! 全库 grep 0 命中；unwrap_or* 变体仅 4 处且均安全）——存储层畸形数据一律映射为 `StorageError`，不 panic。
- **SQL 全参数化达成度高**：仅 3 处 `format!` 进 SQL，且拼接内容全部是编译期常量或 `?` 占位符（详见 F7），无任何用户可控数据进入 SQL 文本。
- 会话 token 以 **SHA-256 hash 落库**（`sessions.token_hash`），不存明文；Argon2id 哈希的生成与校验不在本层（server/ctl），本层只持久化 PHC 字符串。
- Run 状态机守卫 `transition_run_on_connection`（tasks.rs:271-342）是唯一状态写入收口（02 线索 1 的答复见正面确认 #5）。

---

## 发现列表

## [P2] F1：claim_next_run / claim_approved_run 手写 `BEGIN IMMEDIATE`/`COMMIT`/`ROLLBACK` 绕过 sqlx Transaction RAII——COMMIT 失败路径会把带未决事务的连接还回池，污染后续读写
- 位置: crates/soloops-storage/src/database/tasks.rs:128-185；crates/soloops-storage/src/runtime.rs:697-737
- 置信度: 高（源码 + sqlx 0.8.6 registry 源码双重取证）
- 证据:
  ```rust
  // tasks.rs:128-129 —— 手动开启事务，绕过 sqlx Transaction API
  let mut connection = self.pool.acquire().await?;
  sqlx::query("BEGIN IMMEDIATE").execute(&mut *connection).await?;
  ...
  // tasks.rs:176-184 —— COMMIT 失败用 `?` 直接返回，连接带未决事务 drop 回池
  match result {
      Ok(value) => {
          sqlx::query("COMMIT").execute(&mut *connection).await?;
          Ok(value)
      }
      Err(error) => {
          let _ = sqlx::query("ROLLBACK").execute(&mut *connection).await;
          Err(error)
      }
  }
  ```
  sqlx 0.8.6 取证（`~/.cargo/registry/src/*/sqlx-core-0.8.6/src/pool/connection.rs:274-316`）：`Floating::return_to_pool` 回收连接时只做 lifetime 检查、`after_release` hook（未配置）与 `self.raw.ping().await`；而 sqlx-sqlite 的 `ping`（`sqlx-sqlite-0.8.6/src/connection/worker.rs:405-407`）只 `oneshot_cmd(|tx| Command::Ping { tx })` 检查后台线程存活——**不回滚打开的事务**（其注释中的 "flush ... transaction rollbacks" 仅指经 `Transaction` API 登记的取消事务，由 worker 的 transaction_depth 机制处理；手动 `query("BEGIN IMMEDIATE")` 不经该机制登记）。
- 问题: COMMIT 失败（磁盘满 `SQLITE_FULL`、IO 错误等罕见但真实的场景）时 `?` 提前返回 → `connection` drop → `return_to_pool` 的 ping 探活成功 → **连接带着 open transaction 回池**。此后：经该连接的所有写入都累积在这个永不提交的幽灵事务里（进程退出时回滚丢失——静默数据丢失）；下一个经该连接执行 `BEGIN IMMEDIATE` 的调用者得到 "cannot start a transaction within a transaction" 的诡异错误。8 连接池中 1 个被污染即 1/8 流量异常。ROLLBACK 失败路径（`let _ =` 吞掉错误）同理。database.rs:351-353 已提供 `begin_write()`（`BEGIN IMMEDIATE` + Transaction RAII，Drop 自动回滚），persist_model_response/save_plan/decide_tool_call/finish_tool_call/cancel_run/save_final_report 与 deployment.rs 全部写事务都在用它——这两处是仅有的绕过者，且没有绕过的必要。
- 建议: `claim_next_run`/`claim_approved_run` 改用 `self.begin_write().await?` + Transaction API（auth.rs 三个 trigger 强制审计失败的测试证明该路径回滚可靠）；或至少在 COMMIT/ROLLBACK 失败分支显式重试 ROLLBACK 并把连接标记为不可回收。

## [P2] F2：迁移 1/2 不在事务中执行且 SQL 无 `IF NOT EXISTS`——迁移中途失败留下无法自愈的中间态；双 ctl 并发 migrate 会撞车
- 位置: crates/soloops-storage/src/database/schema.rs:21-56（版本 1/2 分支）；migrations/0001_phase_zero.sql:1-66、migrations/0002_agent_runtime.sql:1-91（全部 `CREATE TABLE` 无 IF NOT EXISTS）；对照版本 3-6 的事务包裹（schema.rs:62, 83, 104, 125）
- 置信度: 高
- 证据:
  ```rust
  // schema.rs:21-27 —— 迁移 1：raw_sql 直接在 pool 上执行，无事务
  if applied.is_none() {
      let adopted = self.table_exists("users").await?;
      if adopted {
          self.verify_phase_zero_schema().await?;
      } else {
          sqlx::raw_sql(BASELINE_SQL).execute(&self.pool).await?;
      }
  ```
  ```sql
  -- migrations/0001_phase_zero.sql:1 —— 无 IF NOT EXISTS，多语句文件
  CREATE TABLE users (
  ```
  对照迁移 3-6 的正确做法（schema.rs:62-76）：`let mut transaction = self.pool.begin().await?; raw_sql(...).execute(&mut *transaction)...; transaction.commit().await?;`
- 问题: ① `raw_sql(BASELINE_SQL)` 是多语句批处理且每条语句独立自动提交——进程被 kill / 磁盘满时部分表已建、migration 记录未写。重跑 migrate() 时 `table_exists("users")` 为 true → 走"接管旧 schema"分支 → `verify_phase_zero_schema` 校验前 6 表 → 缺表即报 `MissingTable`，**每次启动都失败且无自动恢复路径**（必须手工 DROP 或删除 .db 文件；报错信息不提示这一点）。② 版本 1/2 的"检查 version → 执行 SQL → INSERT version 记录"三步不原子（TOCTOU）：两个 `soloopsctl migrate` 并发时，后者在前者 commit INSERT 前通过 version 检查 → `CREATE TABLE users` 报 already exists（或 INSERT 撞主键）→ migrate 失败。缓解事实（如实记录）：生产迁移由 soloopsctl 独占（api/worker 启动只 `ready()` 校验并给出 "database is not migrated; run soloopsctl -- migrate" 的清晰报错，bins/soloops-api/src/main.rs:16、bins/soloops-worker/src/main.rs:21），中间态风险只影响**全新库**（尚无数据，删文件重来的代价低），撞车场景一次重试即恢复。但"迁移失败的中间态处理"作为要点问题确实存在。
- 建议: 版本 1/2 与 3-6 对齐：SQL 文件内显式 `BEGIN TRANSACTION...COMMIT`（SQLite 支持事务内 DDL），或代码侧用 `begin_write()` 把 "raw_sql + INSERT version 记录" 包成原子单元；同时给 0001/0002 的 CREATE TABLE 加 `IF NOT EXISTS` 作为防御纵深。

## [P2] F3：create_owner 的单用户约束是 DEFERRED 事务内 check-then-act——WAL 快照陈旧窗口可产生两个 Owner，破坏单用户不变量
- 位置: crates/soloops-storage/src/database/auth.rs:15-21
- 置信度: 高（机制确证；触发窗口极窄，如实说明）
- 证据:
  ```rust
  // auth.rs:15-21 —— pool.begin() = BEGIN DEFERRED；先 SELECT COUNT 再 INSERT
  let mut transaction = self.pool.begin().await?;
  let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users")
      .fetch_one(&mut *transaction)
      .await?;
  if count > 0 {
      return Err(StorageError::OwnerAlreadyExists);
  }
  ```
  对照 database.rs:351-353 已存在却未被此处使用的 `begin_write()`（`BEGIN IMMEDIATE`）。users 表约束：`username TEXT NOT NULL UNIQUE`（0001:3）——只对 username 唯一，**不阻止两行不同 username**。
- 问题: 两个 `soloopsctl owner-init` 并发执行（如两名操作员同时初始化、或脚本竞争）：A 先 BEGIN DEFERRED + SELECT（读快照 count=0）→ A INSERT 提交；B 在 A 提交前 SELECT（WAL 读者看到提交前快照，count=0），等待 busy_timeout 后获得写锁 → B 的**读快照不会推进** → B INSERT 成功。`id` 是各自 UUID、`username` 不同则不撞 UNIQUE → **users 表出现两行，两个 Owner 凭据均可登录**，单用户机制（count_users>0 拒绝重初始化，ctl main.rs:42-44）被永久绕过。审查要点 4"owner-init 拒绝第二个 Owner 在存储层是否可靠"的答案：**顺序场景可靠（有测试），并发窗口不可靠**。触发概率极低（首次部署的毫秒级窗口），但这是不变量级别的破绽且修复成本一行。
- 建议: `create_owner` 改用 `begin_write()`（IMMEDIATE 在第一条语句前取写锁，SELECT 读到的必然是已提交状态）；加一道数据库级兜底：`CREATE UNIQUE INDEX users_singleton ON users((1))`（部分索引强制全表唯一，任何写法都无法违反单行不变量）。

## [P3] F4：恢复扫描把 CAS 冲突当错误上抛，中断整个循环——多 worker 并发下恢复批量推迟、错误语义误导
- 位置: crates/soloops-storage/src/runtime.rs:356-385（promote_due_retries）、runtime.rs:1156-1200（recover_expired_runs）
- 置信度: 高
- 证据:
  ```rust
  // runtime.rs:376-381 —— due 复查通过后 transition，冲突错误用 `?` 上抛中断 for 循环
  if due {
      transition_run_on_connection(&mut transaction, &run_id, RunStatus::Queued, None, now).await?;
      ...
  }
  transaction.commit().await?;
  ```
  transition_run_on_connection 在 CAS 不命中时返回 `StorageError::InvalidTransition`（tasks.rs:318-325）。
- 问题: `engine.rs:116-118` 在每轮 `run_once` 开头无条件调用 `promote_due_retries` + `recover_expired_runs` + `recover_safe_runs`——若部署多个 worker 实例，两进程同时扫描同一 due run：A 成功转换后，B 的 CAS 不命中 → InvalidTransition 经 `?` 上抛 → **B 本轮恢复循环中断，剩余 due run 全部推迟到下一轮**，且日志呈现为"非法转换"而非"被并发处理"（误导排障）。单 worker 是当前默认部署形态（README 三进程 = api/worker/hostd 各一），故实际暴露面有限；`claim_next_run(worker_id)` 的多 worker 设计表明并发是支持场景。
- 建议: 恢复扫描内对 `InvalidTransition`/`ToolCallStateConflict` 类冲突单独 catch 并 `continue`（该 run 已被同伴处理是正常互斥结果），只对真正的非法状态转换报错；或循环体逐 run 隔离错误。

## [P3] F5：迁移记录的 checksum 写入后从不校验、`name`/`adopted` 字段无生产消费——迁移文件内容漂移无法被发现，checksum 纯装饰
- 位置: crates/soloops-storage/src/database/schema.rs:28-38, 46-55, 66-75, 87-96, 108-117, 129-138（6 处写入）；全库 grep `checksum`：storage 之外及 schema.rs 自身均无读取点（唯一读取在测试 tests/schema.rs:14-18 读 `adopted`）
- 置信度: 高
- 证据:
  ```rust
  // schema.rs:28-36 —— checksum 精心计算并入库……
  let checksum = format!("{:x}", Sha256::digest(BASELINE_SQL.as_bytes()));
  sqlx::query(
      "INSERT INTO soloops_schema_migrations
       (version, name, checksum, applied_at, adopted)
       VALUES (1, 'phase_zero', ?, ?, ?)",
  )
  ```
  而 migrate() 对已应用版本的后续判断只有 `SELECT version FROM soloops_schema_migrations WHERE version = ?` 的存在性检查（schema.rs:17-20 等），从不回读 checksum 比对。
- 问题: 迁移系统"看起来有完整性校验"（表结构含 checksum 字段、写入时计算 SHA-256），实际上任何两次部署之间若迁移 SQL 被修改（热修、分支合并），已应用版本与当前二进制内嵌 SQL 的不一致**零检测**——新旧二进制对同一 version 的 schema 期望分叉，错误只在运行时以 `MissingColumn` 之类间接暴露。对单 Owner 私有系统影响温和，但 checksum 字段的存在本身构成误导。
- 建议: migrate() 末尾（或 verify_schema 内）逐版本回读 checksum 与 `Sha256::digest(当前内嵌 SQL)` 比对，不匹配即报错；或删掉 checksum 字段与计算代码，消除误导。

## [P3] F6：事务开启方式三种并存且无选择规则——`begin_write()`(IMMEDIATE)、`pool.begin()`(DEFERRED)、手写 `BEGIN IMMEDIATE` 混用，读后写升级模式散布
- 位置: 全 crate 分布：begin_write 使用者 13 处（runtime.rs:393, 522, 629, 815, 1013, 1129 等 + deployment.rs:204, 344, 440, 529）；`pool.begin()` DEFERRED 使用者 15 处（auth.rs:15、tasks.rs:12、runtime.rs:161, 246, 288, 333, 367, 572, 936, 1050, 1060, 1169, 1215、notifications.rs:71）；手写 BEGIN IMMEDIATE 2 处（F1）
- 置信度: 高
- 证据:
  ```rust
  // database.rs:351-353 —— 正确的写事务入口
  pub(crate) async fn begin_write(&self) -> Result<Transaction<'static, Sqlite>, StorageError> {
      Ok(self.pool.begin_with("BEGIN IMMEDIATE").await?)
  }
  // runtime.rs:367 —— promote_due_retries 用 DEFERRED 且事务内先读后写
  let mut transaction = self.pool.begin().await?;
  let due: bool = sqlx::query_scalar("SELECT EXISTS(...)")
      .fetch_one(&mut *transaction).await?;   // 读
  if due {
      transition_run_on_connection(...)       // 写：DEFERRED 读后升级
  ```
- 问题: 除 F1/F3 已单独指出的两处外，`promote_due_retries`/`recover_expired_runs` 也是 DEFERRED 读后写模式（EXISTS 复查 → transition）。它们的正确性目前依赖 CAS 兜底（EXISTS 读到的可能是陈旧快照，transition 的 `WHERE status=?` 拦住），行为上正确但每处都靠"恰好有 CAS"来保平安；`create_owner`（F3）没有 CAS 兜底所以真正破防。事务模式不统一使"哪条路径防了快照陈旧、哪条没防"不可一眼审计，是系统性弱化。
- 建议: 立规矩——凡事务内含写一律 `begin_write()`；`pool.begin()` 仅限纯读快照（runtime_snapshot/runtime_execution_state）并在注释标明。现有 DEFERRED 写事务逐个迁移。

## [P3] F7：`format!` 进 SQL 共 3 处——均无用户数据、无注入风险，但违反"全部参数化"纪律且风格不一
- 位置: crates/soloops-storage/src/database/schema.rs:151, 173；src/runtime.rs:944-951；src/database/notifications.rs:99-105
- 置信度: 高（注入风险：无——拼接源全部为编译期常量或 `?` 占位符）
- 证据:
  ```rust
  // schema.rs:151 —— PRAGMA 的表名来自 REQUIRED_COLUMNS 编译期常量；PRAGMA 无法绑定参数
  let rows = sqlx::query(&format!("PRAGMA table_info({table})"))
  // runtime.rs:939-951 —— allowed_status 是两个硬编码常量字符串之二
  let allowed_status = if denied == 1 || before_start {
      "status IN ('pending', 'waiting_for_approval')"
  } else {
      "status = 'running'"
  };
  let changed = sqlx::query(&format!(... "AND {allowed_status}"))
  // notifications.rs:99-103 —— IN 列表拼接纯 `?` 占位符，值全部 bind
  let placeholders = std::iter::repeat_n("?", recipients.len()).collect::<Vec<_>>().join(",");
  let query = format!("DELETE FROM ip_notification_recipients WHERE owner_id = ? AND email NOT IN ({placeholders})");
  ```
- 问题: 三处都无法被 SQL 注入利用（PRAGMA 表名/条件常量/占位符列表），但：a) PRAGMA 处的 `table` 虽当前是常量数组，缺乏"必须来自白名单"的显式约束，未来若有人把动态表名传入 `verify_schema` 类函数会静默失去防线；b) `fail_tool_call_internal` 的条件分支拼接完全可以写成两条静态 SQL 之一（clarity + 可审计性）；c) `notifications` 的占位符拼接是安全的标准做法（予以正面确认），但 recipients 数量无上限——server 层若放行超大列表，将撞 SQLite 变量数上限（32766）报错。
- 建议: `fail_tool_call_internal` 拆两条静态 SQL；PRAGMA 处加 `const` 白名单断言或 debug_assert；`update_ip_notification_settings` 对 `recipients.len()` 做 clamp（如 ≤100）并显式报错。

## [P3] F8：测试覆盖缺口——recover_safe_runs 的 process→Blocked 安全分支零测试；并发测试只有 2 个真实竞争场景；其余测试全部跑在 `:memory:` 单连接上
- 位置: crates/soloops-storage/src/database/tests/（8 文件）；关键缺口证据：`grep -rn "Blocked|unsafe_process" tests/` 0 命中；concurrency.rs:9-40、:42-114 是仅有的两个真实竞争测试；common.rs:8-12 `Database::connect(":memory:")`（database.rs:377 对 in-memory 强制 max_connections=1）
- 置信度: 高
- 证据:
  ```rust
  // concurrency.rs:15-19 —— 唯一一个"写锁持有人 vs 并发 claim"测试（真实文件库 + 真实 BEGIN IMMEDIATE）
  let mut writer = database.pool.acquire().await.unwrap();
  sqlx::query("BEGIN IMMEDIATE").execute(&mut *writer).await.unwrap();
  ```
  同时 `tests/runtime.rs:118-145`（leases_a_queued_run_only_once）只测**顺序**二次 claim，未测并发双 claim 抢同一 run 的 TOCTOU 窗口——恰是 `claim_next_run` 两阶段设计所防御的场景。
- 问题: ① `recover_safe_runs` 的 `risk='process' → Blocked`（"Interrupted process has unknown side effects"）是**安全语义分支**（决定中断的进程工具后 run 是否还能继续），全测试套件零覆盖；② 双 worker 并发 claim 同一 queued run、并发 `create_owner`（F3 窗口）、并发 migrate（F2 窗口）均无测试；③ 除 2 个并发测试外全部 `:memory:`——单连接使 DEFERRED/IMMEDIATE 行为差异、BUSY、快照陈旧在测试中物理上不可能发生（这正是 F3 至今未暴露的原因）。concurrency.rs 的两个测试质量很高（真实文件库 + 真实写锁 + 双 Database 连接模拟多进程），值得以此为模板补齐。
- 建议: 补三类测试（模板照抄 concurrency.rs）：process risk running 调用中断 → recover_safe_runs → 断言 Blocked；双 Database 并发 claim 同一 run → 断言恰好一个成功；并发 create_owner → 断言恒为单行。

## [P3] F9：时间源为墙钟 SystemTime（非单调）；latest-run 选取的 tiebreak 是 UUID v4——同毫秒并发创建时"最新 run"的选择是随机的
- 位置: crates/soloops-storage/src/database.rs:388-395（now_ms）；crates/soloops-storage/src/database/tasks.rs:77, 94（排序 tiebreak）
- 置信度: 高（行为确证；实际影响小）
- 证据:
  ```rust
  // database.rs:388-395
  pub fn now_ms() -> i64 {
      SystemTime::now()
          .duration_since(UNIX_EPOCH)
          .unwrap_or_default()      // pre-epoch 时返回 0，安全
          .as_millis()
          .try_into()
          .unwrap_or(i64::MAX)
  }
  // tasks.rs:74-78 —— created_at 同值时按 id 排序；id 是 UUID v4（随机）
  JOIN runs ON runs.id = (
    SELECT r.id FROM runs r
    WHERE r.task_id = tasks.id
    ORDER BY r.created_at DESC, r.id DESC LIMIT 1
  )
  ```
- 问题: ① 全部 lease/重试/过期判定基于墙钟：时钟回拨只会让 lease 过期判定与 retry_at 推迟（保守方向，无害），`unwrap_or_default` 防了 pre-epoch——**回拨容忍度合格**（审查要点 5 的正面回答）；② 但 runs.id / tasks 排序的次序键是 `id DESC`，而 id 是 `Uuid::new_v4()`（auth.rs:23、tasks.rs:9-10）——同一毫秒内创建的两个 run（重试场景下可能），`list_tasks`/`get_task` 的 TaskSummary 展示的"latest run"是随机胜出者，而非后创建者。单 Owner + 毫秒级创建间隔下概率极低。
- 建议: run id 改用 UUID v7（时间有序）或在 runs 表加单调递增的 `rowid`/sequence 列做 tiebreak；若保持现状，在 ORDER BY 处注释说明 tiebreak 语义。

## [P3] F10：claim_next_run 的候选探测被并发抢走后直接返回 None，不重试下一个候选——多 worker 下高概率空转一轮
- 位置: crates/soloops-storage/src/database/tasks.rs:120-140
- 置信度: 高
- 证据:
  ```rust
  // tasks.rs:120-123 —— 事务外单候选探测
  let candidate_id: Option<String> =
      sqlx::query_scalar("SELECT id FROM runs WHERE status = 'queued' ORDER BY created_at ASC LIMIT 1")
          .fetch_optional(&self.pool).await?;
  ...
  // tasks.rs:138-139 —— 事务内复查失败（候选已被抢）→ 直接 Ok(None)
  let Some(row) = row else {
      return Ok(None);
  };
  ```
- 问题: 两阶段设计（快探测 + 事务 CAS）本身正确（避免了持写锁轮询），但事务内发现候选被抢后**只放弃、不取下一个候选**：队列里有 N 个 queued run、M 个 worker 并发时，每轮每个 worker 最多推进 1 次探测，被抢者整轮空转。单 worker 默认部署下无影响；多 worker 下队列消化变慢（不是错误，是吞吐缺陷）。
- 建议: 事务内候选被抢时（row 为 None 或 rows_affected≠1）循环取下一个候选（最多再试 2-3 次），或将候选选取移入 BEGIN IMMEDIATE 事务内。

## [P3] F11：会话生命周期三处缺陷——revoked 会话被物理删除断审计链；revoke 不校验存在性恒记 success；last_seen_at 每请求一写
- 位置: crates/soloops-storage/src/database/auth.rs:155-161、:125-153、:110-115
- 置信度: 高
- 证据:
  ```rust
  // auth.rs:156 —— 过期"或已吊销"的会话行被物理删除
  sqlx::query("DELETE FROM sessions WHERE expires_at <= ? OR revoked_at IS NOT NULL")
  // auth.rs:132 —— revoke 不带 revoked_at IS NULL 条件，也不检查 rows_affected
  sqlx::query("UPDATE sessions SET revoked_at = ? WHERE id = ?")
  // auth.rs:111 —— 每次认证请求写一次 last_seen_at（UPDATE 单独发一条写事务）
  sqlx::query("UPDATE sessions SET last_seen_at = ? WHERE id = ?")
  ```
- 问题: ① audit_logs 里 `auth.login`/`auth.logout` 行引用的 `session_id` 在 cleanup 后悬空（audit_logs 与 sessions 无 FK，审计行本身保留，但"该会话是否曾吊销/何时吊销"的**可追溯性随行删除丢失**）——对宣称审计完整的系统，吊销历史应保留行或归档；② `revoke_session_with_audit` 对不存在/已吊销的 session_id 照样写 `outcome: "success"` 审计（UPDATE 0 行不可见），审计与事实脱节；③ 每个 API 请求（find_session_owner）都对 sessions 表发一条写 UPDATE——api 进程的写事务与 worker 写竞争 WAL 写锁（busy_timeout 5s 掩盖），高频轮询 UI 下是持续写放大。三者皆非破坏性，合并为一条 P3。
- 建议: ① cleanup 只删过期未吊销、吊销行保留（或吊销时归档到 audit context）；② revoke 校验 rows_affected，0 行返回错误或审计 outcome 标注；③ last_seen_at 节流（如距上次 <60s 跳过 UPDATE）。

## [P3] F12：重复审批撞 tool_approvals 主键报裸 UNIQUE 错误；`RunNotFound` 错误类型被重用于承载 tool call 与 plan_call_id
- 位置: crates/soloops-storage/src/runtime.rs:642-654（decide_tool_call 的 INSERT）、:640（RunNotFound 携带 call_id）；src/deployment.rs:101（RunNotFound 携带 plan_call_id）
- 置信度: 高
- 证据:
  ```rust
  // runtime.rs:640 + :643 —— SELECT 要求 tc.status='waiting_for_approval'，但 decide 不修改
  // tool_calls.status，故 SELECT 条件在重复审批时依然通过；INSERT 随即撞 call_id 主键
  .ok_or_else(|| StorageError::RunNotFound(format!("{run_id}:{call_id}")))?;
  sqlx::query(
      "INSERT INTO tool_approvals (call_id, run_id, owner_id, arguments_sha256, decision, reason, created_at)
       VALUES (?, ?, ?, ?, ?, ?, ?)",
  )
  ```
- 问题: ① decide 的存在性检查条件（`tc.status = 'waiting_for_approval'`）在第一次审批后**依然成立**（decide 有意不改 tool_calls.status，恢复交由 claim_approved_run）——于是重复点击审批（前端双击、重试）第二次通过 SELECT 后 INSERT 撞 `tool_approvals` 的 call_id 主键，返回的是 sqlx 裸 UNIQUE 错误而非语义化的 `ToolCallStateConflict`，server 层若按 StorageError 映射会把它变成 500 而非 409；② `RunNotFound` 被用于"找不到 tool call"（`"{run_id}:{call_id}"`）和"找不到 revision"（plan_call_id），调用方无法按类型精确分支，错误信息与类型互相矛盾。
- 建议: decide_tool_call 在 INSERT 前用 `INSERT ... ON CONFLICT` 或捕获 UNIQUE 冲突映射为 `ToolCallStateConflict`；新增 `StorageError::ToolCallNotFound`/`RevisionNotFound` 变体替代 RunNotFound 重用。

## [P3] F13：runtime_snapshot 无界全量加载工具结果（每工具上限 ~10MB 的 JSON 存两份）——长 run 快照内存随工具数线性放大；两处 tool_calls 排序键不一致
- 位置: crates/soloops-storage/src/runtime.rs:1288-1302（snapshot 全量 fetch）、:838 + :867（result 双份存储）、:562 vs :1291（排序键不一致）
- 置信度: 高（机制）；实际影响: 中低（单 Owner + 正常 run 规模）
- 证据:
  ```rust
  // runtime.rs:1289-1291 —— 快照加载全部 tool_calls（含 result_json 全量）与全部 evidence，无 LIMIT
  "SELECT tc.*, ta.decision AS approval_decision, ... FROM tool_calls tc
   LEFT JOIN tool_approvals ta ON ta.call_id = tc.call_id
   WHERE tc.run_id = ? ORDER BY tc.created_at ASC, tc.ordinal ASC"
  // runtime.rs:838 —— finish_tool_call 把 result 全量写入 tool_calls.result_json
  .bind(result.to_string())
  // runtime.rs:867 —— 同一 result 又嵌入 agent_items 的 tool_result payload —— 双份存储
  json!({"callId": call_id, "result": result, "evidenceId": ...}).to_string()
  // 对照 runtime.rs:562 —— pending_tool_calls 的排序键是 ordinal 优先：
  "ORDER BY tc.ordinal ASC, tc.created_at ASC"
  ```
- 问题: ① hostd 协议的输出上限约 10MB（hostd-protocol `MAX_RESPONSE_BYTES = 10*1024*1024 + 64*1024`），每个工具结果都以 JSON 文本**存两份**（tool_calls.result_json + agent_items.payload），`runtime_snapshot` 又把它们全部 decode 进内存——N 个工具的 run 一次快照 ≈ 2N×结果体积；长 run（几十上百次工具调用）时 worker 内存尖峰可预期；② 同一表在 `pending_tool_calls`（ordinal 优先）与 `runtime_snapshot_on_connection`（created_at 优先）使用不同排序键——若 ordinal 是每批模型响应内局部编号（跨批可能重复），snapshot 的 `created_at, ordinal` 才是正确序而 pending 的 `ordinal, created_at` 会跨批乱序；若 ordinal 全局递增则两序等价。正确性依赖 application 层的 ordinal 生成语义（跨层线索 #6）。
- 建议: snapshot 按需分页/延迟 decode result_json；评估 tool_result 是否需要全文复制进 agent_items（重放是否读它）；统一两处排序键并在注释固化 ordinal 语义。

## [P3] F14：`verify_phase_zero_schema` 用 `REQUIRED_TABLES[..6]` / `REQUIRED_COLUMNS[..6]` 硬编码切片——"接管校验范围"与数组前 6 项隐式耦合
- 位置: crates/soloops-storage/src/database/schema.rs:167, 172
- 置信度: 高
- 证据:
  ```rust
  // schema.rs:167, 172
  for table in &REQUIRED_TABLES[..6] { ... }
  for (table, required) in &REQUIRED_COLUMNS[..6] { ... }
  ```
- 问题: `[..6]` 的语义是"REQUIRED_TABLES 的前 6 项恰好是 phase-0 表"——未来任何人向 REQUIRED_TABLES 中间插入新表、或调整顺序，接管校验的范围就静默漂移（多验/漏验），编译器无提示、无测试钉住当前 6 项。属于典型的隐式契约。
- 建议: 定义独立的 `PHASE_ZERO_TABLES`/`PHASE_ZERO_COLUMNS` 常量数组，verify_phase_zero_schema 显式引用；或加断言测试钉住前 6 项内容。

## [P3] F15：对旧 TypeScript schema 的接管校验只查"表存在 + 列名存在"，不查约束/索引/类型——列同名但结构不同的库会被静默接管
- 位置: crates/soloops-storage/src/database/schema.rs:22-24（接管判定仅凭 `table_exists("users")`）、:166-186（校验深度）
- 置信度: 高（校验深度事实）；风险评级: 低（旧 TS schema 是自家产物 + 单 Owner 私有部署）
- 证据:
  ```rust
  // schema.rs:21-24 —— 接管判定：库里有 users 表即视为旧 SoloOps schema
  if applied.is_none() {
      let adopted = self.table_exists("users").await?;
      if adopted {
          self.verify_phase_zero_schema().await?;
  ```
- 问题: ① 任何恰好含 `users` 表的外部 SQLite 文件被指向 `SOLOOPS_DATABASE_PATH` 时会被当作"旧 SoloOps schema"接管，校验仅到 6 表的列名（users/sessions/tasks/runs/events/audit_logs）；列名全对但**缺关键约束**（如 sessions.token_hash 无 UNIQUE、runs.status 无 NOT NULL）的库也能通过接管，随后行为语义悄悄变化（token 可重复、插入 NULL 状态）；② 旧 TS schema 的 users 行（Argon2 参数/盐格式）内容本身不被校验——这是跨层问题（ctl/server 层消费），本层如实记录。
- 建议: 接管校验补三类检查：列类型（`PRAGMA table_info` 的 type 字段）、关键 UNIQUE/NOT NULL 约束（对照 0001 声明）、以及接管判定至少再确认 1-2 张 phase-0 特征表（如 audit_logs 存在）而非仅 users。

## [P3] F16：`list_tasks`/`get_task` 对无 run 的 task 静默不可见（02 跨层线索 3 的正式确认）——当前无触发路径，属防御性收录
- 位置: crates/soloops-storage/src/database/tasks.rs:74-78, 91-95
- 置信度: 高（机制 + 全库无删除路径的证据）
- 证据:
  ```rust
  // tasks.rs:74-78 —— INNER JOIN：子查询无行时 task 不出现在结果中
  FROM tasks
  JOIN runs ON runs.id = (
    SELECT r.id FROM runs r WHERE r.task_id = tasks.id
    ORDER BY r.created_at DESC, r.id DESC LIMIT 1
  )
  ```
  全 crate 仅有的 DELETE 语句：sessions 清理（auth.rs:156）、ip_notification_recipients 清理（notifications.rs:94, 103）——**不存在删除 runs/tasks 的代码路径**，故"task 无 run"当前不可能发生。
- 问题: 确认 02 节点的疑问属实：任务列表对无 run 的 task 不可见。因 `create_task_with_run` 总是 task+run 同事务创建（tasks.rs:12-58），且无删除路径，**现实触发条件不存在**；但一旦未来加入"删 run 保留 task"或数据修复工具，此行为立即变成静默丢任务。作为对 02 跨层线索的正式答复收录，修复优先级低。
- 建议: 改为 LEFT JOIN + `status` 可空投影，或加一条断言测试钉住"task 必有 run"不变量，防止未来路径悄悄违反。

---

## 正面确认（无发现项，供后续节点免重复核查）

1. **panic 面：非测试代码零 panic 操作符**。`unwrap()`/`expect(`/`panic!`/`unreachable!`/`todo!` 全库 grep 0 命中（排除 tests/）；仅有的 `unwrap_or_default()`/`unwrap_or(i64::MAX)`（database.rs:391-394）、`unwrap_or_default()`（runtime.rs:103 enum_text）、`is_none_or`（deployment.rs:223）均为安全变体。畸形 DB 数据（非法 status/type/JSON）一律映射为 `StorageError::InvalidRunStatus/InvalidEventType/InvalidEventPayload/InvalidRuntimeJson`，不 panic、不吞错——有专门测试钉住（tests/runtime.rs:146-174 篡改 payload 为非法 JSON → 报 InvalidEventPayload 而非 fallback）。
2. **会话 token 不落明文**：sessions 表存 `token_hash`（0001:10，UNIQUE），auth.rs 全部按 hash 比对（:99-101）。token 生成与 sha256 在 server 层（跨层线索 #5 供 server 节点确认熵源）。
3. **业务写与审计写同事务原子**：create_task_with_run、create_session_with_audit、revoke_session_with_audit、wait_for_tool_approval、decide_tool_call 均在同一事务内同时写业务表与 audit_logs；三个 `CREATE TRIGGER RAISE(FAIL)` 测试（tests/auth.rs:98-196）证明审计失败时业务行完整回滚。
4. **迁移 3-6 的事务与幂等正确**：均事务包裹 SQL + INSERT 记录原子提交；重复 migrate 幂等（version 存在即跳过，tests/schema.rs:99 连续两次 migrate 通过）；v2→v3 列添加的数据保留有专门测试（tests/schema.rs:34-94）。
5. **（02 线索 1 答复）run 状态写入唯一守卫点确认**：全部 status 变更经 `transition_run_on_connection`（守卫 + `WHERE id=? AND status=?` CAS + rows_affected 复核 + RunStatusChanged 事件），**唯一旁路**是 `claim_next_run` 的 queued→leased 直写 UPDATE（tasks.rs:143-147），但该旁路自带 `WHERE id=? AND status='queued'` CAS、转换为合法矩阵边（02 已核对）、且补发 RunStatusChanged 事件（:158-165）——设计上可接受；`claim_approved_run` 的 lease 设置 UPDATE（runtime.rs:712-717）不写 status、状态变更走 transition（:711）。租约获取/恢复扫描无其它直写。
6. **（02 线索 2 答复）审批后有接管路径**：`claim_approved_run`（runtime.rs:677-738）将 WaitingForApproval→Running（transition）+ 重新设置 lease_owner/lease_expires_at，approve 与 deny 都可被 claim（注释明确 denied 也需 worker 终结记录）；engine.rs:121 每轮调用。Running 空转至恢复扫描的疑虑解除。
7. **（02 线索 3 答复）见 F16**。
8. **恢复风险分类闭环**（曾疑为盲区，核实后确认无缺口）：`recover_safe_runs` 对 5 个 ToolRisk 的归宿——process→Blocked（storage 层）、read_only→重置 pending（storage 层）、workspace_write→留 running 交 engine `probe_interrupted_write`（execution.rs:40-56）、privileged（仅 managed.deploy.apply/rollback，tools.rs:855/869）→留 running 交 engine `resuming_managed_change` + managed lease 恢复链（deployment.rs 的 claim_expired/renew，hostd 侧驱动）、network→policy Deny 无 running 路径（report.rs:25）。全部有归宿，engine 侧兜底分支的真实闭环留给 engine 节点核实（跨层线索 #7）。
9. **连接配置正确**：WAL（非 in-memory）+ `busy_timeout(5s)` + `synchronous(NORMAL)`（WAL 推荐档）+ `foreign_keys(true)`（sqlx 对每条新连接生效）+ in-memory 特判强制单连接（database.rs:357, 377，避免多条独立内存库）+ `disable_statement_logging()`（防 payload 泄漏进日志）。
10. **host 工具授权查询严谨**（runtime.rs:112-144）：run/call/工具名/sha256 四元匹配 + 双方 status='running' + approval 的 `owner_id = task.created_by` 校验 + 参数化；测试覆盖 tampered digest 拒绝、工具名不匹配拒绝（tests/auth.rs:23-96）。
11. **managed deployment 的 lease/token 设计**：partial unique index 强制每 project 单活跃 operation（0005:46-48 + 代码内 active 检查双保险）、token CAS 贯穿 renew/update_phase/finish/fail、过期接管互斥经 `claim_expired_managed_deployment_operation` 的事务内二次检查、`INSERT ... ON CONFLICT DO NOTHING` + 读回逐字段比对保证 revision 幂等与不可变（deployment.rs:98-115，测试覆盖内容冲突拒绝）；managed operation lease 的续租/接管由 **hostd 侧**驱动（bins/soloops-hostd/deployment.rs:188, 324, 365）——worker 持 run lease、hostd 持 operation lease 的双层契约清晰。
12. **事件流轻量设计**：AgentMessage 只存 preview 且截断 500 字符（runtime.rs:435）、工具事件 payload 不含 result 全文（只 callId/summary/revision）、错误 message 截断 500（:309）；`list_events` limit clamp(1,500)、`load_agent_items` clamp(1,200)。
13. **活跃执行时间语义正确**：`active_execution_ms_on_connection`（runtime.rs:1250-1274）按 model_attempts/tool_calls 的 started_at..completed_at 窗口求和、未完成窗口裁剪到 now，审批等待与重试退避不计入预算——有专门测试（tests/runtime.rs:257-333）。
14. **api/worker 启动不自动迁移**：只 `ready()`（verify_schema）校验，未迁移时给出"run soloopsctl migrate"的明确指引（bins/soloops-api/src/main.rs:16、bins/soloops-worker/src/main.rs:21）——生产迁移由 ctl 独占，收敛了 F2 的暴露面。

---

## 跨层线索（供后续节点核查）

| # | 线索 | 锚点 |
|---|---|---|
| 1 | **恢复扫描每轮无条件执行**：engine.rs:116-118 每轮 `run_once` 先跑 promote_due_retries + recover_expired_runs + recover_safe_runs——多 worker 部署即并发扫描（F4 的触发面）；engine 节点应确认 run_once 对这些 Err 的处理是重试还是熔断 | application/engine.rs:116-118 |
| 2 | **hostd 持 managed operation lease**：`renew_managed_deployment_lease`/`claim_expired_managed_deployment_operation` 的调用方在 bins/soloops-hostd/deployment.rs:188, 324, 365——worker 持 run lease、hostd 持 operation lease 的双层租约契约，hostd 节点需核对续租节奏与 storage 的 `lease_expires_at >= now` 严格续租条件（过期即失权，与 run lease 的宽松续租不同，runtime.rs:1105-1121 vs deployment.rs:315-336） | storage/deployment.rs:315-336 + hostd/deployment.rs:188, 324, 365 |
| 3 | **事件 payload 键名清单**（e2e 失败 2 / 02 F8 的消费端核对锚点）：storage 写入的事件 payload 键名全部 camelCase——`{"taskId","status"}`（RunCreated, tasks.rs:40）、`{"from","to","workerId","reason"}`（RunStatusChanged）、`{"preview"}`（AgentMessage, runtime.rs:435）、`{"summary","steps"}`（AgentPlanUpdated, :549）、`{"callId"}`（ToolCallStarted, :783）、`{"callId","summary","workspaceRevision"}`（ToolCallCompleted, :895）、`{"callId","category","summary"}`（ToolCallFailed, :987）、`{"outcome","summary"}`（RunReported, :1040）。server WS 回放与前端 run 页的键名消费应逐字比对 | storage/runtime.rs:435, 549, 783, 895, 987, 1040 + tasks.rs:40, 162 |
| 4 | **decide_tool_call 不修改 tool_calls.status**（waiting_for_approval 保持到 claim_approved_run 接管）——server 层的审批端点必须防重复提交（F12：第二次 INSERT 撞主键变 500），且 F2 修复时注意 server 是否已把 sqlx unique 错误映射成 409 | storage/runtime.rs:620-675 + server 审批端点 |
| 5 | **session token 生成在 server 层**：storage 只存 SHA-256 hash 且按 hash 等值比对（无恒时比较需求——hash 比较非秘密比较）。server 节点需确认：token 用 CSPRNG 生成（rand ≥128 bit）、hash 无需 pepper 的设计声明、以及 `find_session_owner` 的 SELECT+UPDATE 两步（非同事务）在 server 的调用频率 | storage/auth.rs:90-123 + server/http/auth.rs |
| 6 | **tool_calls 排序键二义**：pending_tool_calls 用 `ordinal, created_at`（runtime.rs:562）而 snapshot 用 `created_at, ordinal`（:1291）——正确性取决于 application 层 ordinal 生成是"全局递增"还是"每批模型响应内局部编号"；engine 节点核实（若局部编号则 pending 路径跨批乱序） | storage/runtime.rs:562 vs 1291 + application/engine |
| 7 | **WorkspaceWrite/Privileged 中断恢复的 engine 闭环**：storage 侧 recover_safe_runs 对这两类 risk 的 running 调用原样保留并转 run 为 Queued（runtime.rs:1226-1237 else 分支），engine 侧 execution.rs:35-56 有 probe_interrupted_write / resuming_managed_change 分支——两段代码的接力（claim→initialize→execute_pending_tools 遇 running 调用）无集成测试（storage 测试无此场景，application/tests 有部分覆盖 managed.rs:199-200），engine 节点应端到端验证 | storage/runtime.rs:1202-1243 + application/engine/execution.rs:35-56 |
| 8 | **迁移部署契约**：migrate() 生产入口仅 soloopsctl（api/worker 只 ready() 校验）——若 operator 直接启动 api/worker 于未迁移库，得到的是 `MissingTable` 类 StorageError 包上"database is not migrated"指引；hostd 生产不 migrate（deployment.rs:1945 为测试代码）。修复 F2/F5 时保持该契约 | bins/soloopsctl/main.rs:34 + schema.rs:188-191 |
| 9 | **`fail_managed_deployment_operation` 对已是 active 的 revision 静默**：`UPDATE ... WHERE status='proposed'` 0 行时不报错（deployment.rs:544-551）——恢复探活得出错误结论时会出现 "operation failed + revision active + current 指向它" 的矛盾组合；engine 节点核对恢复判定与 finish/fail 的选择逻辑是否可能产生该组合 | storage/deployment.rs:544-551 |
| 10 | **storage 无孤儿数据清理职责**：evidence/agent_items/事件随 run 级联删除（FK ON DELETE CASCADE），但全库无删 run 路径（仅 sessions/recipients 有 DELETE）——events 表只增不减，长期运行的库需要外部归档策略；deployment/hostd 节点确认 bundle_path 文件与 revision 记录的清理契约 | migrations/0001-0006 + storage grep "DELETE FROM" |

---

## 覆盖清单

| 文件 | 行数 | 状态 |
|---|---|---|
| crates/soloops-storage/src/lib.rs | 12 | 已审（穷尽；pub use 面 + pub(crate) 三函数收口核对） |
| crates/soloops-storage/src/database.rs | 405 | 已审（穷尽；REQUIRED_TABLES/COLUMNS 17 表逐列核对、连接配置、now_ms） |
| crates/soloops-storage/src/database/schema.rs | 201 | 已审（穷尽；6 迁移分支逐行、verify 双函数；F2/F5/F14/F15） |
| crates/soloops-storage/src/database/auth.rs | 162 | 已审（穷尽；F3/F11；token_hash 落库方式核对） |
| crates/soloops-storage/src/database/tasks.rs | 459 | 已审（穷尽；F1/F10/F16；claim 两阶段与 transition CAS 逐行） |
| crates/soloops-storage/src/database/notifications.rs | 192 | 已审（穷尽；F7 占位符拼接正面确认；N+1 评估为可接受） |
| crates/soloops-storage/src/runtime.rs | 1387 | 已审（穷尽；F4/F6/F7/F9/F12/F13；lease/续租/恢复/时间源/授权查询逐行） |
| crates/soloops-storage/src/deployment.rs | 641 | 已审（穷尽；lease/token CAS、finish/fail 的 revision 状态机、validate_operation_retry；跨层线索 2/9） |
| crates/soloops-storage/src/database/tests/mod.rs | 9 | 已审（穷尽） |
| crates/soloops-storage/src/database/tests/common.rs | 72 | 已审（穷尽；:memory: 单连接 fixture 的影响评估，见 F8） |
| crates/soloops-storage/src/database/tests/concurrency.rs | 114 | 已审（穷尽；2 个真实竞争测试逐行评估；F8 主体） |
| crates/soloops-storage/src/database/tests/auth.rs | 228 | 已审（穷尽；3 个 trigger 回滚测试验证 sqlx Transaction RAII 有效——F1 修复的安全性依据；owner-init 并发缺口） |
| crates/soloops-storage/src/database/tests/runtime.rs | 333 | 已审（穷尽；顺序二次 claim 的覆盖缺口、elapsed_ms 窗口测试评估） |
| crates/soloops-storage/src/database/tests/recovery.rs | 227 | 已审（穷尽；Blocked 分支零覆盖、事务回滚原子性测试确认） |
| crates/soloops-storage/src/database/tests/managed.rs | 348 | 已审（穷尽；lease 接管/互斥测试质量高） |
| crates/soloops-storage/src/database/tests/schema.rs | 154 | 已审（穷尽；TS schema 接管、v2→v3 数据保留、双 migrate 幂等） |
| migrations/0001_phase_zero.sql | 66 | 已审（穷尽；F2 无 IF NOT EXISTS；列与 REQUIRED_COLUMNS 一致性核对通过） |
| migrations/0002_agent_runtime.sql | 91 | 已审（穷尽；F2；UNIQUE 约束三处与代码 CAS 呼应） |
| migrations/0003_workspace_write_recovery.sql | 1 | 已审（穷尽） |
| migrations/0004_ip_notifications.sql | 20 | 已审（穷尽；COLLATE NOCASE 与 NOT IN 清理的 collation 语义核对） |
| migrations/0005_managed_deployments.sql | 48 | 已审（穷尽；partial unique index 双保险核对） |
| migrations/0006_managed_deployment_leases.sql | 8 | 已审（穷尽；过期 lease 部分索引与 claim 查询匹配性核对） |
| crates/soloops-storage/Cargo.toml | 19 | 已审（穷尽；依赖最小面确认：无 argon2/无 time crate——时间全靠 SystemTime，与 F9 一致） |

**发现统计**: P0 × 0；P1 × 0；P2 × 3（F1 手动事务连接池污染、F2 迁移 1/2 无事务中间态卡死、F3 owner-init 并发双 Owner 窗口）；P3 × 13（F4–F16）。合计 16 条。

**审查方法补充**: sqlx 0.8.6 池回收/事务行为结论经 `~/.cargo/registry/src/*/sqlx-core-0.8.6/src/pool/connection.rs` 与 `sqlx-sqlite-0.8.6/src/connection/worker.rs` 源码取证；migrate()/恢复函数/托管 lease 的调用方经 ripgrep 全库定位（engine.rs:116-121、hostd/deployment.rs:188/324/365、bins main.rs）；application/tools.rs、engine/execution.rs、engine/report.rs 的读取仅为核实 ToolRisk 五变体与恢复分支归属（其自身缺陷留给对应节点）；未修改任何文件；`cargo test -p soloops-storage` 24/24 通过（0.19s，与基线一致）。
