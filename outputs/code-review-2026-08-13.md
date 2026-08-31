# SoloOps 代码审查报告 — Phase 3 受管部署 MVP + 发布加固

- **日期**: 2026-08-13
- **分支**: `codex/phase3-release-hardening`
- **审查范围**: 工作区未提交改动（`git diff HEAD`，base `337e458`），25 个文件，+1334 / −160 行
- **审查方法**: 双代理并行审查（Rust 后端 / 前端·E2E·CI·文档）+ 人工逐项核实 + 构建与测试验证

---

## 验证结果

| 项目 | 结果 |
|---|---|
| `cargo check --workspace` | ✅ 通过 |
| `cargo test --workspace` | ✅ 全部通过（88 tests，0 失败，2 ignored 为 live 验收测试） |
| `bun run --cwd apps/web build` | ✅ 通过（6.2s，adapter-static 输出到 `build/`） |

> 说明：首次前端构建失败系 WorkBuddy 安全删除保护拦截了 SvelteKit 清理自身 `.svelte-kit` 缓存目录，清理缓存后正常，非代码问题。

---

## 优点

1. **持久化 Saga 设计正确**（`hostd/deployment.rs`）：部署操作先写库再执行 side effect，引擎"无持久化操作 ⇒ 无副作用 ⇒ 可安全重放"的不变式成立，杜绝双重应用。
2. **恢复状态机保守且自洽**（`engine/execution.rs`）：持久化操作与授权调用（run_id/action/revision）交叉校验，任何歧义一律 `Unresolved` → 阻塞 Run 而非猜测；`Completed/Failed/Unresolved/Missing` 四种状态清晰映射 hostd 崩溃/回滚/对账结果。
3. **SQLite 读后校验占位优化正确且有测试**：`claim_next_run` / `claim_approved_run` 无锁读候选、`BEGIN IMMEDIATE` 下复核，无双重认领；idle 轮询不再阻塞写者，回归测试就位。
4. **超时即阻塞而非静默重试**：managed change 超时阻塞 Run，测试锁定（calls == 1）。
5. **E2E 后台错误门禁**（`global-teardown.ts`）：从 `/__e2e/shutdown` 读取 `{ok, errors}`，把 Worker/SQLite 后台错误提升为套件失败，是本轮最有价值的加固。
6. **Harness 优雅退出**：watch 信号 + worker join + DB close，teardown 轮询 `/healthz`，替代硬 `exit(0)`。
7. **文档与实现一致**：600s 描述符超时、420s 最坏预算（90s compose + 60s Caddy + 60s 健康 + 210s 补偿，重算吻合）、durable operation 恢复/阻塞语义均与代码对应；CI 分离 web/e2e job，避开 `.svelte-kit/output` 并发占用。

---

## 问题

### Critical（必须修复）

**C1. 顶层命名卷 `driver_opts` 可逃逸 "禁止宿主机 bind mount" 保证** — `bins/soloops-hostd/src/deployment.rs:976-989`（校验缺口）、`:1099-1115`（`reject_external_resources` 仅查 `external`/`name`）
- 现状：service 级 bind mount 通过检查 `volumes[*].source` 是否含 `/`、`\`、`.` 前缀来拒绝；但**顶层卷**声明 `driver: local` + `driver_opts: {type: none, o: bind, device: /etc}`，service 以 `mydata:/data` 引用（source 为 `mydata`，通过检查），`docker compose up` 后容器可读写宿主机任意路径。
- 影响：对以权限隔离为存在意义的 hostd 守护进程，这是容器逃逸级别的越界，模型可控容器可直接读写宿主机 `/etc`、`/root` 等。
- 修复：在 `reject_external_resources`（或卷级检查）拒绝 `driver_opts` 含 `type: none`/`o: bind`/`device:`（或直接拒绝任何顶层卷的 `driver_opts`/非 `local` driver），并补拒绝用例的单测。

### Important（应当修复）

**I1. 健康检查跟随重定向，存在 blind SSRF** — `deployment.rs:750-768` + `:179,224`（`reqwest::Client::new()` 默认最多跟 10 次重定向）
- 探测 `/health` 带 `Host: <site>`，若被部署应用返回 `302 Location: http://169.254.169.254/...` 等内网地址，hostd 会从宿主网络跟随，可探测任意主机并触发 GET 副作用。
- 修复：健康检查 client 设 `redirect(Policy::none())`，3xx 直接视为健康失败；顺带校验 `health_path`（拒绝 `//`、控制字符、`#`、`?`）。

**I2. `reconcile()` 在守护进程级别 fail-closed** — `deployment.rs:259-261`（`unix.rs:93-98` 接入）
- 任一未完成部署无法补偿（镜像丢失、docker 挂、caddy validate 失败）即 `anyhow::bail!` 中止 **hostd 整个启动**，殃及 `process.exec`/`sandbox.exec` 的所有 Run；且 Caddy rename 与 DB 提交之间的崩溃窗口内，新 revision 已生效但 `current_revision_id` 未推进，只有重启 hostd 才会回滚。
- 修复：按操作记录失败并继续（不 abort）；考虑周期对账而非仅启动时；文档写明崩溃窗口语义。

**I3. Compose `include:` / `extends:` 可读取宿主机任意 YAML** — `deployment.rs:929-949`（禁词表缺 `extends`）、`:907-921`（顶层未拒 `include`）
- `docker compose config` 会解析 `include: /abs/path.yaml`、`extends: {file: ...}`，规范化输出或 stderr（最长 1000 字符）会向模型泄露宿主机文件结构/内容。
- 修复：`extends` 加入 service 禁词表，顶层拒绝 `include`，补测试。

**I4. 顶层 `driver: macvlan/ipvlan` 网络绕过 loopback-only 隔离** — `deployment.rs:907-908` + `:1099-1115`
- 声明 `networks: {lan: {driver: macvlan, driver_opts: {parent: eth0}}}` 后容器直接获得宿主机局域网接口，突破"端口仅 127.0.0.1 + 白名单"策略。
- 修复：拒绝 `driver` 非 bridge/none 及一切 `driver_opts`。

**I5. 同一项目的部署操作缺少串行化** — `crates/soloops-storage/src/deployment.rs:187-236` + hostd `execute_change`
- 去重只按 `call_id`，不检查该项目是否已有未完成操作；多 worker 并发时两个 Run 可同时对同一项目 `docker compose up`，随后双 revision 均标 active、`current_revision_id` 后写覆盖——部署状态机不一致。
- 修复：`begin_managed_deployment_operation` 对同一 `project_id` 已存在未完成操作时拒绝（或排队），补测试。

**I6. 新恢复逻辑与安全边界测试缺口**（`application/src/tests.rs`、hostd 测试）
- "operation 与授权调用不匹配 → `Unresolved`"分支未测；恢复 `Missing` → 重执行路径（安全论证的核心不变式）无测试；`managed.deploy.rollback` 恢复无引擎级测试；`validate_compose`/`validate_caddy` 拒绝路径无单测（仅 `#[ignore]` live 测试）；`reconcile()` 完全未测；claim 的"候选在读写之间被抢"竞态无测试。

**I7. CI sandbox 清理用了不存在的 label** — `.github/workflows/release-acceptance.yml:91`
- 清理过滤器为 `label=soloops.sandbox=true`，实际沙箱容器打的是 `soloops.managed=true`（`bins/soloops-hostd/src/sandbox.rs:337`），仓库中不存在 `soloops.sandbox` label → 清理永不匹配，失败路径泄漏的容器只能靠临时 runner 回收。
- 修复：改 `--filter label=soloops.managed=true`。

**I8. e2e teardown 两个 fetch 无超时** — `apps/web/e2e/global-teardown.ts:4,19`
- 若 `stop_worker` 阻塞在执行中的 Tool Call（managed deploy 描述符超时可达 600s），teardown 的 `/__e2e/shutdown` POST 无限挂起，Playwright 进程不报错不结束，直接吞掉 CI 超时。
- 修复：两处 fetch 加 `AbortSignal.timeout(15_000)`。

### Minor（建议改进）

- **M1** `deployment.rs:786-798`：Caddy fragment 崩溃残留 `.{project}.{revision}.tmp`，compensate 不清理；建议 glob 清理。
- **M2** `e2e_harness.rs:426-433`：`worker.await` 无限等待，worker 处于长部署时 shutdown 挂起；加超时或宽限后 abort。
- **M3** `database/tasks.rs:120-126`、`runtime.rs:684-695`：候选在无锁读与 `BEGIN IMMEDIATE` 之间被抢时返回 None（无正确性问题，下次轮询补上），建议注释或事务内重扫。
- **M4** 工作区 `Cargo.toml` 声明 `rust-version = "1.85"`，但 `e2e_harness.rs:428` 及既有 `execution.rs:777` 使用 let-chains（需 Rust ≥ 1.88）；CI 若钉 1.85 将无法编译，需同步版本声明。
- **M5** `config.rs:160` + `runtime.rs:92`：`max_tool_duration_ms` 默认与上限同为 600s，环境变量只能收紧；引擎实际按全局配置而非 Run 预算执行，建议文档说明，避免运维困惑。
- **M6** `managed_root/<project>/revisions/<sha256>` 每 plan 累积不回收（含失败/被取代者），建议清理 superseded/rolled_back/failed bundle。
- **M7** `ci.yml:4-6`：`push:` 未限定分支，每分支 push 全量矩阵 + 与 PR 双触发；建议限定 `main` 并对 docs 加 `paths-ignore`。
- **M8** `ci.yml` 各 job 无 `timeout-minutes`（默认 360 分钟），建议 20–30 分钟。
- **M9** `playwright.config.ts:11` 配置 `trace: retain-on-failure`，但 CI 未上传 `test-results/` 制品，等于没保留；建议 `if: failure()` 上传。
- **M10** `.env.example` 缺 `SOLOOPS_MAX_TOOL_DURATION_MS` 及其他 budget 变量（`MAX_MODEL_TURNS/MAX_TOOL_CALLS/...` 均在 `config.rs` 存在），建议一次性补齐。
- **M11** `happy-path.spec.ts:11` 新断言绑定装饰性版本徽章 "Phase 3.1"，阶段升级需改 spec；建议断言稳定元素或收敛版本常量。
- **M12** `global-teardown.ts:14-25` 5s 硬失败可能偶发 flaky（如 SQLite close 略慢），建议放宽 10s 或降级 warning。
- **M13** README/development.md 将 `bun run test:e2e` 列为常规命令，但未说明本地 Linux 需 `bunx playwright install chromium`。
- **M14** hostd 未做旧 bundle GC（见 M6 关联）；建议补充 `reconcile` 状态暴露，让被阻塞 Run 的 Owner 可见原因。

---

## 建议（下一步）

1. **修复优先级**：先修 C1（真实容器逃逸），随后 I1/I3/I4（同一 `validate_compose`/`validate_caddy` 面，一次加固可同时闭合，并配套拒绝路径单测）。
2. **补安全测试**：`validate_compose`/`validate_caddy` 拒绝用例（driver_opts、macvlan、extends/include、非 loopback 端口）+ C1 回归测试，无需 Docker 的内存测试即可覆盖。
3. **补恢复路径测试**：`Unresolved` 不匹配分支、恢复 `Missing` 重执行、rollback 重放、claim 竞态。
4. **写"崩溃窗口"文档**：明确 apply_compose → apply_caddy → verify_health → completed/failed 各阶段"线上生效 vs 已提交"的差异，这是运维最易踩的坑。
5. **I2 重新设计 reconcile**：至少按操作记录失败并继续，周期对账替代仅启动对账。

---

## 结论

**不建议直接合并（With fixes）。**

本轮工作区改动本身质量过硬——引擎恢复、SQLite 并发、超时策略、测试与文档均属上乘，`cargo check`/`cargo test`/前端构建全部通过，E2E 门禁与 CI 骨架可圈可点。**但本文件是特权升级守护进程的发布加固审查**：managed-deployment 安全边界仍有 1 个 Critical 容器逃逸（C1: 卷 `driver_opts` bind mount）、4 个同面 Important 缺口（SSRF 重定向、reconcile fail-closed、include/extends、macvlan）及 1 个部署状态机一致性缺口（I5 项目内串行化）。C1 与校验器缺口闭合并补测试前不应合并；I7/I8 两处 CI/E2E 笔误为一行级修复，可与上述一并处理。

---

## 修复跟进（2026-08-13 二次审查）

对工作区当前状态逐项复核，结论：**C1 / I1–I8 的修复均已落入工作区代码**，唯一此前遗漏的是 M4（版本声明），本轮已补齐。

| 项 | 状态 | 证据 |
|---|---|---|
| C1 卷 `driver_opts` 逃逸 | ✅ 已修 | `deployment.rs` `reject_external_resources` 拒绝任何顶层卷/网络的 `driver_opts`，且卷 driver 限 `local`；测试 `rejects_unsafe_top_level_volume_and_network_drivers` 覆盖 bind/nfs/macvlan/network driver_opts |
| I1 SSRF 重定向 + health_path | ✅ 已修 | `new()` 用 `redirect(Policy::none())`；`validate_health_path` 拒绝 `//`、`..`、`\`、`#`、`?`、控制字符；测试 `health_probe_does_not_follow_redirects`、`validates_health_path_as_a_path_only` |
| I2 reconcile fail-closed | ✅ 已修 | `reconcile()` 逐操作 `record_managed_deployment_recovery_failure` 后 `continue`，不再 `bail!`；`unix.rs:93-98` 仅 `warn!` 不 abort 启动，并 `tokio::spawn(run_reconciler)` 周期对账 |
| I3 include/extends | ✅ 已修 | `extends` 进入 service 禁词表；顶层 `include` 在 `validate_compose` 与 `validate_compose_source` 双重拒绝；测试 `raw_compose_preflight_rejects_file_loading_before_docker` |
| I4 macvlan/ipvlan | ✅ 已修 | 网络 driver 限 `bridge`/`none`，任何 `driver_opts` 拒绝 |
| I5 同项目串行化 | ✅ 已修 | `begin_managed_deployment_operation` 查询 `project_id + finished_at IS NULL` 未完成操作并冲突拒绝；测试 `managed_deployment_...`（`managed-apply-concurrent` 同项目第二操作报错，首操作结束后可重入） |
| I7 CI label | ✅ 已修 | `release-acceptance.yml:91` 为 `label=soloops.managed=true` |
| I8 teardown 超时 | ✅ 已修 | `global-teardown.ts` 两处 `fetch` 均带 `AbortSignal.timeout(15_000)` |
| **M4 rust-version** | ✅ **本轮修复** | `Cargo.toml` `rust-version` `1.85` → `1.88`；`ci.yml` / `release-acceptance.yml` 的 `rustup toolchain install` 与 `cargo +` 全部 `1.85.0` → `1.88.0`；`README.md` 同步为 1.88。代码中 3 处 let-chains（`database.rs:359`、`http.rs:147`、`e2e_harness.rs:429`）需 Rust ≥ 1.88，其中 `http.rs` 属 HEAD 既有代码，说明 1.85 声明此前即不成立 |

### 仍为可选改进（未处理，不影响合并安全边界）
- **M1–M14** 各 Minor 建议：Caddy 临时 fragment 清理（M1）、worker join 超时（M2）、claim 无锁读竞态注释（M3）、bundle GC（M6）、CI `push` 分支限定 / `timeout-minutes`（M7/M8）、trace 制品上传（M9）、`.env.example` 补齐 budget 变量（M10）、版本徽章断言（M11）等，均非安全/正确性阻断项，可按需后续处理。
- **M14**「崩溃窗口」语义文档：apply_compose → apply_caddy → verify_health → completed 各阶段「线上生效 vs 已提交」差异建议补充运维文档。
