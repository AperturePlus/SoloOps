# 06 — bins/soloops-hostd 特权边界审查（Unix Socket 对端认证 / Docker 沙箱 / 受管部署）

- **范围**: bins/soloops-hostd 全部 4 文件穷尽精读——main.rs（19 行）、src/unix.rs（1151 行）、src/sandbox.rs（787 行）、src/deployment.rs（2184 行），合计 4141 行；另对照 crates/soloops-hostd-protocol/src/lib.rs（254 行，02 章已审，本节点按消息枚举逐一对照）与 storage 两个定点核实（runtime.rs:112-144、deployment.rs:432-560）。
- **方法**: 纯静态精读（本 crate 目标平台 Linux，Windows 主机无法运行其测试；unix.rs/sandbox.rs/deployment.rs 测试段已全部读完，`#[cfg(unix)]`/`#[ignore]` 门控符合预期，不重复 00-baseline 的全量测试）。deployment.rs 分 6 段、unix.rs 分 4 段、sandbox.rs 分 3 段读取。
- **基线/章节关系**: 02-线索 5/6/8、02-F10/F11、03-线索 9/10 已全部正式核查，见"前序线索核查答复"。04a 对 ManagedDeploy 字符串契约的结论（04a §前序线索答复）已在本节点从 hostd 产出侧独立复核一致。
- **审查对象**: 工作区状态（分支 feature/agent-page-polish，deployment.rs 含未提交修改——本节点读到的即工作区版本）。

## 前序线索核查答复

| 线索 | 结论 |
|---|---|
| 02-线索 5/8（max_output_bytes 校验不一致） | **属实，收录为 F4**。unix.rs:531 managed 路径只拒 `==0 \|\| > MAX_TOOL_OUTPUT_BYTES`（无 1024 下限），且该校验先于授权查询；unix.rs:649（process）与 sandbox.rs:101（sandbox）均为 `1024..=MAX_TOOL_OUTPUT_BYTES`。04b 已证客户端合法配置域 1024..=10MB ⊆ hostd 接受域，运行期不可失配——不一致本身成立，属协议面收口问题。 |
| 02-线索 6 / 02-F11（ManagedDeploy 字符串契约 / e2e 失败 2） | **hostd 产出侧逐字一致，非 e2e 失败 2 根因**（与 04a 结论互证）。hostd 侧全部产出点：`result_from_revision("plan",…)`（deployment.rs:652）、`"status"` 含 `status:"not_deployed"` 变体（:665-678）、apply 经 `execute_change(…, "apply", …)`（:718→:953-954 action 透传）、rollback（:736 replay 走存量 result、:768 execute_change→:954 `"rollback"`）；失败路径写入 operations.result_json 的 `action` 也取自同一参数（:922-923）。全仓 grep 无其他 action 字面量。 |
| 02-F10（响应信封可表示非法组合） | **hostd 侧不可产生**。全 crate grep `HostdResponseV3`：28 处命中全部为 `::success`/`::failure` 构造器调用，零字面量构造——`result` 与 `error` 永远互斥且不同时为 None（protocol lib.rs:62-83 的构造器保证）。非法组合仅存在于协议类型的可表示性（02-F10 维持），消费端（client）仍按 02 记录防御。 |
| 03-线索 9（fail_managed_deployment_operation 对 active revision 静默） | **hostd 执行面不可构造该矛盾组合**。revision 变 active 仅发生在 `finish_managed_deployment_operation` 事务内（storage/deployment.rs:488-516：SET status='active' + current 指针 + operation phase='completed'/result 写入，同一事务），此后 `fail_…` 的 `WHERE finished_at IS NULL` 命中 0 行时返回 `ManagedDeploymentConflict`（storage/deployment.rs:530-543）——**是报错不是静默**。hostd 两处 fail 调用点（unix 无；deployment.rs:477 reconcile、:935 execute_change 失败路径）都发生在 compensate 成功之后、finish 从未执行（revision 仍 'proposed'，WHERE 命中）。唯一静默 no-op 场景是 **rollback** 操作失败：其目标 revision 入参要求 status∈{active,superseded,rolled_back}（deployment.rs:759），`WHERE status='proposed'` 永不匹配——但此时 DB 语义恰好自洽（current 仍指被 compensate 恢复的旧 revision），no-op 良性。04a 引擎侧"不查 current revision"的防御性缺口维持收录，但 hostd 侧无可达触发路径。 |
| 03-线索 10（bundle_path 文件清理契约） | **正式回答：无契约、文件侧无主**。hostd 全 crate 无 `remove_dir`/删除调用（grep 0 命中）；storage 无 DELETE revisions。收录为 F6。 |

---

## [P2] F1：Compose 变量插值未被禁止——plan 期 workspace `.env` 可使 Owner 审批预览与实际部署内容分叉
- 位置: bins/soloops-hostd/src/deployment.rs:1497-1559（validate_compose_source）、:545-561（plan 期 normalize）、:984-999 与 :1186-1195（apply 期以 bundle 为 project-directory）
- 置信度: 高（机制确凿；现实触发需模型组合多个前提）
- 证据:
```rust
fn validate_compose_source(source: &[u8], config: &ManagedDeployConfig) -> Result<(), ManagedDeployError> {
    ...
    for (name, service) in services {
        ...
        for forbidden in [
            "extends", "env_file", "secrets", "configs", "label_file", "provider",
        ] {
```
（:1497-1555 禁键清单中无 `$`/插值检查；plan 期 `docker compose -f <workspace 文件> config` 未指定 --project-directory，Compose V2 默认项目目录=compose 文件所在目录=run workspace，`.env` 参与插值；apply 期 `--project-directory <bundle>`（:988-989/:1188-1189），bundle 内只有 compose.yaml/site.caddy，无 `.env`。）
- 问题: 模型可先经 workspace 工具写入 `.env` 与含 `${VAR:-default}` 的 compose（如 `image: app@sha256:${DIGEST:-aaa…}`）。plan 期预览按 workspace `.env` 解析（预览显示 bbb 镜像、`docker image inspect` 校验 bbb）；apply 期按 bundle 目录重新解析（`.env` 缺失，落到 compose 默认值 aaa）——**Owner 批准的预览与实际部署的 revision 内容可以不同**。缓解面充分：apply 期 `validate_revision_bundle` 对实际生效的 normalize 结果全量重校验（digest-pinned、端口 loopback+白名单、站点白名单均不可越权），且未预载镜像会让 `up --pull never` 直接失败回滚；但两个镜像恰好都已预载时，"informed consent" 的预览契约被打破。
- 建议: validate_compose_source 直接拒绝包含 `$` 的 compose 源文本（不可变部署 bundle 本就不需要插值；可同时杜绝 `$$` 转义歧义）；或 plan/apply 期统一给 `docker compose config` 传固定 `--project-name`/`--project-directory` 并以 `--no-interpolation`（若 CLI 支持）消除两侧差异。

## [P2] F2：SystemCommandRunner 无流式输出上限——`.output()` 先全量收集再事后检查 output_limit
- 位置: bins/soloops-hostd/src/deployment.rs:238-257
- 置信度: 高
- 证据:
```rust
        let future = Command::new(program)
            .args(arguments)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .output();
        let output = tokio::time::timeout(timeout, future)
            .await
            ...
        if output.stdout.len().saturating_add(output.stderr.len()) > output_limit {
```
- 问题: 与 process/sandbox 路径的流式截断（unix.rs:750-772 drain 按 8KB 增量累加并即时中止）不同，managed 路径所有 docker/caddy 子进程（compose config/up/down、image inspect、caddy adapt/validate/reload）的 stdout+stderr 由 tokio `.output()` **无界缓冲到子进程退出后才检查**。`compose up -d --wait` 在服务大量打印时（或异常场景下 docker CLI 行为失控）可把 hostd 内存打爆——hostd 是全仓权限最高进程，其 OOM 会同时中断沙箱/受管部署服务。超时兜底仅限制时长（最长 health_timeout+30s ≈ 90s），不限制体积。
- 建议: 复用 unix.rs 的 drain/Capture 模式（tokio::process + 流式 read + 双流合计 limit 即时中止），或在 SystemCommandRunner 中用 `Child::wait_with_output` 前先以 bounded reader 包装。

## [P2] F3：沙箱 workspace 预算看门狗为 250ms 轮询全量 walk，且瞬时 IO 错误直接误杀合法沙箱
- 位置: bins/soloops-hostd/src/sandbox.rs:287-294、:264-285；bins/soloops-hostd/src/unix.rs:366-383
- 置信度: 高
- 证据:
```rust
pub async fn wait_for_workspace_limit(root: &std::path::Path, maximum: u64) -> std::io::Result<()> {
    loop {
        if workspace_size(root, maximum).await? > maximum {
            return Ok(());
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
}
```
```rust
                limit = &mut workspace_limit => match limit {
                    Ok(()) => Err((HostdErrorCode::PolicyDenied, "sandbox exceeded the persisted workspace size budget".into())),
                    Err(_) => Err((HostdErrorCode::Internal, "sandbox workspace monitoring failed".into())),
                },
```
- 问题: ① /workspace 是 RW bind mount（sandbox.rs:311-317 read_only:false），Docker 无法对 bind mount 配额；预算执行完全靠此轮询——检测窗口 250ms+walk 耗时内容器可写入不受限数据（每次过限后由 remove_sandbox 强删止损，总体受 60s 超时约束，但"预算=硬上限"并不成立）；② 更现实的是误杀：容器进程在看门狗 walk 的同时并发创建/删除文件，`symlink_metadata`/`read_dir` 的 TOCTOU NotFound 错误会沿 `?` 直接冒泡为 `Err` → unix.rs 以 Internal "sandbox workspace monitoring failed" **中止整个 sandbox 调用**——正常做目录增删 churn 的合法工作负载（构建、测试）可被随机误杀。
- 建议: workspace_size 对 NotFound 类错误跳过（条目已被并发删除属正常）；轮询间隔可按 workspace 大小自适应；文档明示 bind-mount 预算为尽力而为（或迁移到 docker volume + image-level 配额方案）。

## [P3] F4：max_output_bytes 双校验规则不一致（02-线索 5/8 正式确认）——managed 无 1024 下限且校验先于授权
- 位置: bins/soloops-hostd/src/unix.rs:531-537 vs :649、bins/soloops-hostd/src/sandbox.rs:101
- 置信度: 高
- 证据:
```rust
    if max_output_bytes == 0 || max_output_bytes > MAX_TOOL_OUTPUT_BYTES {
        return HostdResponseV3::failure(
            request_id,
            HostdErrorCode::PolicyDenied,
            "managed deployment output limit is invalid",
        );
    }
```
```rust
    if !(1024..=MAX_TOOL_OUTPUT_BYTES).contains(&max_output_bytes)
        || input.args.len() > 128
```
- 问题: managed 四个子命令接受 1..=1023（managed 路径），process（unix.rs:649）与 sandbox（sandbox.rs:101）拒收同区间；另 managed 的该检查位于 `authorize_host_tool_call` **之前**（unix.rs:531 vs :538），process/sandbox 则在授权之后——顺序也不对称。04b-F 系列已证 client 合法域恒为 1024..=10MB，故无运行期失配，属协议面收口/一致性问题。
- 建议: 抽出共享校验函数（如 `validate_output_limit(bytes) -> bool` 统一 1024..=MAX_TOOL_OUTPUT_BYTES），并统一"先校验后授权"或相反的顺序。

## [P3] F5：process.exec / sandbox.exec 无重放消耗语义——authorize 是状态窥视而非单次消费，与 managed 幂等设计不对称
- 位置: crates/soloops-storage/src/runtime.rs:119-136（authorize_host_tool_call）；bins/soloops-hostd/src/unix.rs:157-234（process 分支无幂等记录）
- 置信度: 高
- 证据:
```sql
                WHERE tc.run_id = ? AND tc.call_id = ? AND tc.name = ?
                  AND tc.status = 'running' AND r.status = 'running'
                  AND tc.arguments_sha256 = ?
```
- 问题: hostd 对 process/sandbox 的授权查询只校验"该 call 处于 running 且有匹配 digest 的 owner 审批"，无 nonce、不消耗状态；worker（唯一被授权 uid）在同一 call 的 running 窗口内可对完全相同的 (run_id, call_id, digest) 并发/重复发起多次执行——每次都会通过授权并各自 spawn 进程/容器（sandbox 容器名 sha256(call_id) 相同时第二次 create 会因重名失败，但前次完成后即可重放）。无越权放大（参数被 digest 锁死、每容器仍受资源限制），但与 managed 路径精心实现的 replay_operation_result 幂等（deployment.rs:776-819）形成不对称——威胁模型里 worker 属半信任方，"批准一次=执行一次"目前不成立。
- 建议: 授权成功后以 CAS 把 tool_calls 状态推进到执行态（或在协议请求中加一次性 nonce），使同一审批仅可驱动一次 hostd 执行。

## [P3] F6：bundle 文件与 caddy 临时片无任何清理契约（03-线索 10 正式回答：无人清理）
- 位置: bins/soloops-hostd/src/deployment.rs:622-648（plan 每次新建 bundle 目录）、:1065-1077（.tmp staging）；全 crate grep `remove_dir` 0 命中
- 置信度: 高
- 证据:
```rust
        let bundle = self
            .config
            .managed_root
            .join(&input.project_id)
            .join("revisions")
            .join(&proposal_sha256);
        fs::create_dir_all(&bundle)
            .await
            .map_err(|_| ManagedDeployError::internal("could not create the managed bundle"))?;
```
```rust
        let temporary = self
            .config
            .caddy_managed_dir
            .join(format!(".{}.{}.tmp", revision.project_id, revision.id));
```
- 问题: 每次 plan 生成 `managed_root/<project>/revisions/<proposal_sha256>/{compose.yaml,site.caddy}`（各 ≤ output_limit），revision 记录与文件均无删除路径——superseded/rolled_back/failed revision 的 bundle 永久留存；caddy `.tmp` 在 copy 与 rename 之间崩溃即残留，且无 stale `.tmp` 清理逻辑（同名重试会覆盖，但不同 revision.id 的孤儿会累积）。单 Owner 系统增长缓慢（纯文本小文件），但这是"预算"之外又一个无对账维度，与 03-线索 10（DB 侧只增不减）同构。
- 建议: 在 reconcile 循环中加保守清扫（如仅删除 DB 中不存在对应 revision 记录的 bundle、超过 N 代之前的 superseded bundle、以及早于当前时间的所有 `.*.tmp`），或至少在文档中声明文件侧留存的运维预期。

## [P3] F7：bundle 写入与 caddy 片段发布无 fsync——崩溃窗口可留下半写文件被后续 caddy 读取
- 位置: bins/soloops-hostd/src/deployment.rs:631-636（fs::write 直写）、:1069-1077（fs::copy→fs::rename）
- 置信度: 高
- 证据:
```rust
        fs::write(bundle.join("compose.yaml"), &compose_bytes)
            .await
            .map_err(|_| ManagedDeployError::internal("could not persist the managed Compose file"))?;
        fs::write(bundle.join("site.caddy"), &caddy_bytes)
```
```rust
        fs::copy(
            PathBuf::from(&revision.bundle_path).join("site.caddy"),
            &temporary,
        )
        .await
        ...
        fs::rename(&temporary, &target)
```
- 问题: ① plan 侧两文件均为截断式直写，崩溃留孤儿半文件（无 DB 记录，无害但见 F6）；② caddy 片段 copy→rename 无 fsync：rename 原子性地发布名字，但数据未落盘——崩溃后 `{project}.caddy` 可能为空/半内容，`caddy validate`（在 rename 之后执行）或宿主重启后的 Caddy 会读到坏配置；validate 在窗口内仍可能通过（页缓存尚存）。概率低但修复廉价。
- 建议: 临时文件写完 `sync_all()` 再 rename；bundle 写入同样走 temp+fsync+rename。

## [P3] F8：socket 权限收紧存在 umask 窗口——bind 后才 chmod 0o660
- 位置: bins/soloops-hostd/src/unix.rs:99-101
- 置信度: 高
- 证据:
```rust
    prepare_socket_path(&socket_path).await?;
    let listener = UnixListener::bind(&socket_path)?;
    fs::set_permissions(&socket_path, std::fs::Permissions::from_mode(0o660)).await?;
```
- 问题: `UnixListener::bind` 按 hostd 进程 umask 创建 socket 文件（典型 umask 022 → 短暂 0755，其他本地用户可 connect），随后才收紧为 0o660。该窗口内建立的连接仍会在 `handle_connection` 被 `SO_PEERCRED` 精确 uid 检查（unix.rs:135-138，peer_is_allowed 于 :976-978 恒等比较）拒绝，故**不可利用**；但作为纵深防御，创建即应私有。
- 建议: bind 前设置 `umask(0o077)`（或用 `UnixListener` 绑定到私有目录）；`prepare_socket_path` 的 exists→remove→bind 之间亦有竞态（恶意预创建 socket 文件可使启动 bind 失败退出，属可用性问题），可用"bind 失败即拒绝启动"的现状语义在文档标注。

## [P3] F9：cleanup_stale 串行且首错即断——单个坏容器可阻止整个 hostd 启动
- 位置: bins/soloops-hostd/src/sandbox.rs:415-419；bins/soloops-hostd/src/unix.rs:84-89
- 置信度: 高
- 证据:
```rust
        for container in containers {
            if let Some(id) = container.id {
                self.remove(&id).await?;
            }
        }
```
```rust
        driver
            .cleanup_stale()
            .await
            .context("failed to clean stale SoloOps sandboxes")?;
```
- 问题: 启动时清理带 `soloops.managed=true` 标签的遗留容器：串行遍历、首个 remove 错误即 `?` 中止并使 hostd **启动失败退出**——一个无法删除的容器（Docker daemon 异常、删除钩子卡死）即可让全部工具面（process/sandbox/managed）不可用。fail-closed 方向正确，但粒度过粗：后续容器永远不会被清理，也无逐一告警。
- 建议: 逐容器 best-effort（失败记录告警、继续清理其余），仅当"存在无法清理的容器"时再决定退出策略；或至少把全部失败容器列进启动错误信息。

## [P3] F10：parse_bool 存在两份语义分叉的实现（可维护性）
- 位置: bins/soloops-hostd/src/unix.rs:909-915 vs bins/soloops-hostd/src/deployment.rs:1624-1630
- 置信度: 高
- 证据:
```rust
fn parse_bool(value: &str) -> Result<bool> {
    match value.to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" | "on" => Ok(true),
        "false" | "0" | "no" | "off" => Ok(false),
```
```rust
fn parse_bool(value: &str) -> Result<bool> {
    match value {
        "true" | "1" => Ok(true),
        "false" | "0" => Ok(false),
```
- 问题: 同名函数两份实现接受集不同：`SOLOOPS_SANDBOX_ENABLED=yes` 生效而 `SOLOOPS_MANAGED_DEPLOY_ENABLED=yes` 直接启动报错。对运维是隐蔽的行为分叉（环境变量开关恰好分散在两个文件）。
- 建议: 提取到共享模块统一接受集，或在部署文档固定只写 true/false。

---

## 正面确认（后续节点免查；hostd 安全姿态总体非常扎实）

1. **对端认证**：`SO_PEERCRED` 于读任何帧之前执行（unix.rs:135-139），uid 恒等比较单值白名单（:976-978，测试 :1137-1141）；socket 权限 0o660；`prepare_socket_path` 拒绝替换非 socket 文件（含 symlink，:941-947）。
2. **帧边界与畸形输入**：长度前缀 u32 + `MAX_REQUEST_BYTES`=64KB 硬上限（read_frame，protocol lib.rs:172-187）；未知 action 由 `deny_unknown_fields` + tagged enum 在反序列化时拒绝；每连接一请求一响应，畸形输入直接断连（不回写响应）。run_id 进入路径拼接前查空与 `/`、`\`（:820-831、deployment.rs:1568-1576）。
3. **env 注入面封死**：process/sandbox 双侧一致强制 `SOLOOPS_TOOL_[A-Z0-9_]+` 命名空间 + 值 ≤4096 且无 `\0`（unix.rs:980-988、sandbox.rs:208-216），`env_clear()` 后只注入校验过的键——LD_PRELOAD/LD_LIBRARY_PATH/PATH 类加载器注入在两层均不可达（unix.rs 测试 :1143-1150 显式覆盖）。
4. **allowlist 精确匹配**：program 别名走 HashMap 精确键查找（无前缀碰撞）；宿主侧 alias→绝对路径在启动时 canonicalize 并要求 is_file（unix.rs:917-935）；容器侧别名 ≤64 字符字符集受限、可执行路径绝对且无 `//`/`.`/`..`、**最终段拒绝全部 shell 名**（sh/bash/dash/zsh/fish/pwsh/powershell/cmd，sandbox.rs:189-206）；execve argv 传递，全程无 shell。
5. **路径约束**：resolve_cwd/resolve_workspace/resolve_workspace_file 均 canonicalize+前缀校验+（文件侧）is_file，拒绝 `..`、绝对路径与 symlink 逃逸（unix.rs:819-852、deployment.rs:1578-1598；测试 :995-1009 覆盖 symlink escape）。
6. **容器加固全量硬编码**：cap_drop ALL、非 root 65532:65532、`network_mode:"none"` + `NetworkDisabled:true` 双保险、readonly_rootfs、`no-new-privileges`、memory/nano_cpus/pids_limit/tmpfs(noexec,nosuid,nodev,size) 全部来自启动配置（请求不可覆盖，build_run_spec 只 clone config.limits，sandbox.rs:140）、唯一挂载即 workspace bind、无 privileged/设备/pid/ipc 逃逸面；镜像强制 digest-pinned（:168-179，非 64-hex 拒绝）；**驱动无任何 pull 调用**——`docker image inspect` 预载校验在 plan（deployment.rs:565-582），apply 期 `--pull never`（:994-995）双保险离线闭环。
7. **输出限长**：process/sandbox 流式双流合计 ≤ max_output_bytes 即时中止并 kill+reap（unix.rs:750-817，测试覆盖超限/超时/断连三路）；sandbox 响应（stdout+stderr ≤10MB）+信封 < MAX_RESPONSE_BYTES（10MB+64KB）恒成立。
8. **失败路径清理**：sandbox 无论成败都 `remove_sandbox` 双删（force+v，404 视为成功）；容器孤儿由启动 cleanup_stale（soloops.managed 标签）兜底；process `kill_on_drop`+主动 kill+wait 三重。
9. **Compose 策略**：单 YAML 文档强制、merge key 展开后校验（锚点/别名无法走私 extends/env_file/include，测试 :1770-1786）；include/secrets/configs/env_file/environment/extends/provider/label_file/build/devices/cap_add/network_mode/pid/ipc/extra_hosts/sysctls/container_name 全禁；privileged:true 拒绝；driver_opts 拒绝（封死 driver_opts bind 绕过，测试 :1736-1768）；external 与显式 `name` 拒绝（不触碰宿主既有卷/网络）；bind mount 拒（source 含 `/`、`\` 或 `.` 前缀）；端口必须显式 published u16 且 host_ip ∈ {127.0.0.1,::1} 且 ∈ 白名单；镜像 digest-pinned；healthcheck 必须启用（配合 `up --wait`）。normalize 双跑（plan workspace 目录 + apply bundle 目录）使"校验的=部署的"。
10. **Caddyfile 策略**：白名单式解析器——仅允许 `site {`、`}`、`encode gzip|zstd|zstd gzip`、`reverse_proxy <单 token>`；站点精确 HashSet 匹配（无通配/子域逻辑可绕）；上游强制 {127.0.0.1,localhost,[::1]}:白名单 compose 端口；`import /file_server /php_fastcgi /exec /admin` 子串绊线 + `caddy adapt` 实际解析双保险；片段以 temp+rename 原子发布后整体 `caddy validate` 再 `reload`。
11. **健康验证防 SSRF/假阳性链**：health_origin 启动期强校验为裸 loopback HTTP origin（scheme/host/无 userinfo/path=/ 无 query/fragment，deployment.rs:80-92）；healthPath 校验拒 `//`、`..`、`?`、`#`、反斜杠、控制字符（:1483-1495，测试含 `//169.254.169.254/`）；运行期用 `url.set_path`（不会改 host）而非 join；redirect Policy::none（测试 :1797-1839 断言不跟随 302）；Host 头=白名单站点经 Caddy 路由探活；compose `--wait` 容器级 healthcheck 先行。
12. **回滚闭环**：apply 失败→ensure lease→compensate（重新 apply previous 或 compose down+删片段+validate+reload）→fail 落库 rollbackSucceeded；**回滚自身失败**→返回 RecoveryRequired、operation 留给 reconciler（lease 过期 claim→再 compensate，record_recovery_failure 可持续重试），宿主卡在半部署态但状态诚实可查；`prepared` 阶段被 claim 视为 aborted_before_side_effect 不做补偿；lease 由 begin/claim/renew CAS + 侧任务续租 + 每个副作用前 ensure_owned 双查。finish 为单事务（revision active + current 指针 + operation completed 原子完成）。
13. **审批预览脱敏不需要**：compose `environment`/`env_file`/secrets 整体禁用（唯一秘密可能存在的字段被策略消灭），预览仅 services/images/ports/volumes/site/routes/healthPath/hash——脱敏由"禁密"达成。
14. **02-F10**：hostd 响应仅经构造器产生（见线索答复）；**02-F11**：action 字面量与 client 逐字一致（见线索答复），e2e 失败 2 与 hostd 无涉。

## 覆盖清单

| 文件 | 行数 | 状态 |
|---|---|---|
| bins/soloops-hostd/src/main.rs | 19 | 已审（穷尽；纯模块壳+平台门控） |
| bins/soloops-hostd/src/unix.rs | 1151 | 已审（穷尽，4 段；含测试段 990-1151） |
| bins/soloops-hostd/src/sandbox.rs | 787 | 已审（穷尽，3 段；含测试段 498-787） |
| bins/soloops-hostd/src/deployment.rs | 2184 | 已审（穷尽，6 段；含测试段 1672-2184；工作区版本含未提交修改） |
| crates/soloops-hostd-protocol/src/lib.rs | 254 | 对照已审（02 章主审；本节点按枚举逐一核对，测试段重读） |
| crates/soloops-storage/src/runtime.rs:112-144 | 33 | 定点核实（authorize_host_tool_call 语义，F5） |
| crates/soloops-storage/src/deployment.rs:432-560 | 129 | 定点核实（finish/fail 事务语义，03-线索 9 答复） |

发现统计：P0×0 / P1×0 / P2×3（F1/F2/F3） / P3×7（F4-F10），合计 10 条。

## 协议消息 × 处理 × 校验对照表

| HostdAction | 处理链 | 字段/参数校验（hostd 侧全部） | 备注 |
|---|---|---|---|
| ProcessExec | dispatch（unix.rs:157-234）→ authorize(process.exec, sha256(Value::to_string)) → ExecArguments → execute_process（:642-721） | 信封：version==3 精确、deny_unknown_fields；max_output_bytes∈[1024,10MB]、args≤128、env≤128 且键 SOLOOPS_TOOL_* 值≤4096 无\0、timeout_ms∈[1,60000] 否则默认 60000、program 精确 allowlist、cwd canonicalize 受限、env_clear+kill_on_drop、流式限长+超时+断连三路 kill | F4（下限）、F5（重放） |
| SandboxExec | dispatch（unix.rs:235-430）→ authorize(sandbox.exec) → SandboxExecArguments → config/driver 缺失即 PolicyDenied（默认禁用，unix.rs:880）→ resolve_workspace → runtime_snapshot 预算读取（None→Unauthorized）→ workspace_size 预检 → resolve_cwd → build_run_spec → 超时/断连/预算看门狗 select → remove_sandbox 双删 | 信封同上；max_output_bytes∈[1024,10MB]（sandbox.rs:101）、args≤128 且单条≤16KB 无\0、env 同 process 侧、timeout∈[1,60000] 默认 60000、program 精确 allowlist+shell 名拒绝、cwd 相对且无 `..`/`\`/前导`/`；容器参数全硬编码（见正面确认 6） | F3（看门狗）、F5（重放） |
| ManagedDeployPlan | dispatch_managed(Plan)（unix.rs:431-445→522-617）→ authorize(managed.deploy.plan) → service.plan（deployment.rs:519-653） | max_output_bytes 非 0 且 ≤10MB（F4：先于授权）；PlanArguments deny_unknown_fields；validate_project_id [a-z0-9-]≤63；validate_health_path；compose/caddy 路径 resolve_workspace_file（全 Normal 分量+canonicalize）；源合计 ≤ output_limit；validate_compose_source + docker compose config normalize + validate_compose + image inspect 预载 + validate_caddy + caddy adapt；bundle 落盘→DB revision | F1（插值）、F7（fsync）、F6（清理） |
| ManagedDeployStatus | dispatch_managed(Status) → service.status（deployment.rs:655-680） | 同上信封/limit；StatusArguments；validate_project_id；DB current 或 not_deployed | 只读，无副作用 |
| ManagedDeployApply | dispatch_managed(Apply) → service.apply（deployment.rs:682-720）→ replay 幂等 → execute_change（:821-973） | ApplyArguments；replay_operation_result（run_id/action/revision/参数匹配 + result/error 语义）；proposal digest 与 status='proposed' 匹配；stale 检查（current==previous_revision_id）；lease CAS begin+读回校验+phase 机（prepared→applying_compose→applying_caddy→verifying_health）+副作用前 ensure_operation_lease；apply_revision=validate_revision_bundle（sha256 复核+全量重校验）→compose up --pull never --wait→caddy 原子发布+validate+reload→健康探活→finish 事务 | 03-线索 9 答复；F2/F6/F7 关联 |
| ManagedDeployRollback | dispatch_managed(Rollback) → service.rollback（deployment.rs:722-774） | RollbackArguments；replay；current 必须存在；expected_current_revision_id 精确匹配 current；target.project_id 匹配且 target.status∈{active,superseded,rolled_back}；execute_change 同上（compensation_revision_id=current） | 失败时 no-op 于 target 状态为良性（见线索答复） |
| （信封公共） | handle_connection（unix.rs:134-152） | SO_PEERCRED uid 精确匹配先于读帧；read_frame 64KB 上限；version≠3→UnsupportedVersion；响应仅 success/failure 构造器 | 02-F10 闭合；F8（umask 窗口） |

- **越权面核查结论**：全部 6 个 action 的请求字段中，能影响宿主执行面的变量（program、args、env、cwd、timeout、compose/caddy 源文本、healthPath、projectId、proposalId、revisionId）均已逐一核对——不存在可请求覆盖的容器 root/网络/特权/挂载开关（容器参数全部硬编码自启动配置），站点/端口白名单为精确匹配不可绕过，"默认禁用"由 SOLOOPS_SANDBOX_ENABLED / SOLOOPS_MANAGED_DEPLOY_ENABLED 默认 "false" 真实成立（unix.rs:880-882、deployment.rs:42-44）。
