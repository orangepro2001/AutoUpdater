This file is a merged representation of the entire codebase, combined into a single document by Repomix.

# File Summary

## Purpose
This file contains a packed representation of the entire repository's contents.
It is designed to be easily consumable by AI systems for analysis, code review,
or other automated processes.

## File Format
The content is organized as follows:
1. This summary section
2. Repository information
3. Directory structure
4. Repository files (if enabled)
5. Multiple file entries, each consisting of:
  a. A header with the file path (## File: path/to/file)
  b. The full contents of the file in a code block

## Usage Guidelines
- This file should be treated as read-only. Any changes should be made to the
  original repository files, not this packed version.
- When processing this file, use the file path to distinguish
  between different files in the repository.
- Be aware that this file may contain sensitive information. Handle it with
  the same level of security as you would the original repository.

## Notes
- Some files may have been excluded based on .gitignore rules and Repomix's configuration
- Binary files are not included in this packed representation. Please refer to the Repository Structure section for a complete list of file paths, including binary files
- Files matching patterns in .gitignore are excluded
- Files matching default ignore patterns are excluded
- Files are sorted by Git change count (files with more changes are at the bottom)

# Directory Structure
````
.zcode/
  plans/
    plan-sess_91c3196a-360a-4b3e-9e9a-45b719bb5c77.md
crates/
  demo-app/
    src/
      main.rs
    Cargo.toml
  ghost-updater-cli/
    src/
      lib.rs
      main.rs
    Cargo.toml
  ghost-updater-core/
    src/
      lib.rs
    Cargo.toml
  ghost-updater-net/
    src/
      lib.rs
    Cargo.toml
  ghost-updater-tauri/
    src/
      app.css
      App.svelte
      main.ts
    src-tauri/
      capabilities/
        default.json
      gen/
        schemas/
          acl-manifests.json
          capabilities.json
          desktop-schema.json
          windows-schema.json
      icons/
        icon.ico
      src/
        main.rs
      build.rs
      Cargo.toml
      tauri.conf.json
    index.html
    package.json
    svelte.config.js
    tsconfig.json
    vite.config.ts
plan/
  design.md
  feature-requests-from-model-copy.md
  TODO.md
tools/
  e2e.ps1
  gui.cmd
  gui.ps1
.gitignore
Cargo.toml
HowToUseAutoUpdate.md
README.md
````

# Files

## File: .zcode/plans/plan-sess_91c3196a-360a-4b3e-9e9a-45b719bb5c77.md
````markdown
# 核验结论（需求 vs 代码，全部属实）

1. **REQ-2 属实**：`core/src/lib.rs:211-216` swap 失败路径确实只 log+write_result+return，不 `restore_backup`、不 `release_lock`。且**核验发现泄漏面比需求说的更大**：健康检查失败路径（:239-245）、拉起失败路径（:247-253）、pid 超时路径（:199-206）共 4 条失败路径全部泄漏 `.apply-lock`。
2. **REQ-3 属实但范围要修正**：`net/src/lib.rs:136/278` 的 `check_auth` 确实在读 body 前拒绝。但 apply 的 `body: String` 是 axum 提取器，**handler 执行前 body 已被读完**——apply 天然无此问题，需 drain 的只有 **stage**（body 是 `Body`，未消费）。stage 上 `sha256 非法`/`main 非法` 两个 400 分支（:139-144）同样在 body 消费前拒绝，一并修。
3. **REQ-1 属实**：worker 只等 `--pid`（core:199）；`UpdateConfig`（net:41-52）加字段后，经查全仓库仅 `UpdateConfig::new()`（net:54-58）与 demo-app:27（`..Default::default()`，不破）需同步，cli/tauri 零引用。

# 实施方案

## A. ghost-updater-core（REQ-2 + REQ-1 worker 侧）

**A1. 失败路径统一释放锁**：`run_apply` 拆为 `parse → run_apply_inner(&ApplyArgs) → release_lock(staging)` 无条件兜底，覆盖全部 4 条失败路径（比逐路径补更稳）。

**A2. swap 分两段（关键修正）**：`swap_files` 拆出 `ensure_staged`（预检）+ `backup_files`（备份段）+ `overwrite_files`（覆盖段），`swap_files` 变为三者的组合壳（既有 4 个 swap 单测不动、保持绿）。理由：**备份段失败时文件未动，绝不能调 `restore_backup`**——restore 会把"备份里没有"的现存旧文件当"新增"删掉（数据丢失）；只有覆盖段失败才回滚。`run_apply_inner` 按段处理：
- 备份段失败 → `msg 含"文件未动"`，log+result+Err（锁由外层释放）；
- 覆盖段失败 → `rollback()`（复用现有：restore+重拉旧 exe）→ `msg 含"已回滚"`（满足验收）。

**A3. 进程编排原语**（供 net/worker 共用）：
- `validate_process_names`：仅允许 `[A-Za-z0-9._-]`（防 taskkill /IM 参数注入）；
- `process_running_by_name`（Windows：`tasklist /FI IMAGENAME eq /FO CSV /NH` 解析首列；非 Windows false）；
- `close_processes_graceful(names, grace=5s)`（Windows：`taskkill /IM` 无 /F → 轮询 grace → 残留则 `/F` 兜底；非 Windows no-op，按 REQ"非 Windows 可先留空"）；
- `wait_processes_exit(names, timeout=10s)`（轮询 + 超时 `/F` 兜底，镜像 `wait_pid_exit` 语义；非 Windows true）；
- `append_apply_log(install, msg)`（把私有 log_line 开个 pub 口，net 关进程动作可落 apply.log）。

**A4. worker 编排**：`parse_apply_args` 增 `--close <a.exe,b.exe>`（逗号分隔，validate）；`run_apply_inner` 在等完 `--pid` 后、备份前：`close` 非空则 `wait_processes_exit`，超时按 pid 超时同款"放弃更新(文件未动)"。

**A5. 单测**（现有 7 个全保绿）：parse `--close`；`validate_process_names` 拒绝空格/引号/斜杠；`process_running_by_name("cmd.exe")` 真假两断言（windows）；`wait_processes_exit` 不存在名快速 true；**swap 失败回滚集成测试**（windows：`OpenOptionsExt::share_mode(1)` 锁住 install 第二个文件 → 覆盖段必败 → 断言 Err 含"已回滚"、install 回旧内容、staging 无 `.apply-lock`、result.json ok:false）。

## B. ghost-updater-net（REQ-3 + REQ-1 agent 侧）

**B1. drain**：新增 `drain_body(body)`（逐帧读到 EOF，上限 `MAX_PACKAGE_BYTES`，整体套 30s 超时防慢速攻击）；stage 的 check_auth 拒绝（401/503）与 sha256/main 两个 400 分支返回前 `drain_body(body).await`。apply 不动（提取器已消费 body）。

**B2. close_processes**：`UpdateConfig` 加 `pub close_processes: Vec<String>`（`new()`/Default 补空）；`routes()` 启动时 `validate_process_names` 非法即 panic（与 pubkey 同风格）；apply handler 构造 worker argv 时（`--` 分隔符前）非空则追加 `--close <csv>`；自退任务改为 `sleep 800ms → spawn_blocking(close_processes_graceful) → exit(0)`，close 动作经 `append_apply_log` 落日志。关进程在 `check_auth` 通过之后，天然满足"错 token 不触发关闭"。

**B3. 单测**（现有 7 个全保绿）：真实 TCP 集成测试——`TcpListener::bind(127.0.0.1:0)` + `axum::serve`，客户端裸 `TcpStream` 发 Content-Length=3MiB + 错 token，边写边等，断言读到 `HTTP/1.1 401`（无 drain 时此测试必挂）；worker argv 含 `--close`。

## C. demo-app

加 `--close-processes <a.exe,b.exe>` 参数（沿用手写 args 扫描风格），传入 `UpdateConfig.close_processes`。

## D. tools/e2e.ps1 扩展三场景（保持 PS5.1/BOM 约定）

- **REQ-2**：装 v2 成功后组 dist-v3(3.0.0)；`[IO.File]::Open(NOTES.txt,'Open','Read','Read')` 锁写不锁读 → 推 v3 → 备份可读、覆盖必败在 NOTES → 断言 ghost-push 退出码≠0、install VERSION 仍 2.0.0、result.json ok:false 含"已回滚"、所有 stage-* 无 `.apply-lock`；释放句柄重推 v3 成功。（注：回滚重拉是无参启动，轮后需 Stop-Process 并手动带参重启，e2e 显式管理）
- **REQ-3**：dist-v3-big = 拷贝 v3 + `junk.bin` 17MB（.NET WriteAllBytes）；错 token 推 10 次，每次断言输出含 `stage 被拒`+`401` 且**不含** `stage 上传失败`（cli 文案 lib.rs:309/315 恰好可断言）。
- **REQ-1**：拷 ghost-demo.exe 为 install\demo-gui.exe（v3 字节）并起在 7701（占锁自身 exe）；带 `--close-processes demo-gui.exe` 手动重启主程序；组 dist-v4(4.0.0, 含 demo-gui.exe) 推送 → 断言成功、`Get-Process demo-gui` 无进程（优雅→5s→/F 阶梯全走到）、demo-gui.exe 被换新。

## E. 文档同步

- `plan/TODO.md`：3 项标 done + 核验记录；`plan/design.md`：追加变更小节（阶段化回滚的取舍、drain 范围修正、tasklist/taskkill 选型与名称白名单、非 Windows no-op）+ 错误记录表；`README.md:31-35/52-56` 与 `HowToUseAutoUpdate.md:57-67/158-170`：字段表、回滚行为、排障表同步；`feature-requests-from-model-copy.md` 状态改"已实现"。

## 验证

`cargo build --workspace` → `cargo test --workspace`（存量 14 + 新增 ~10 全绿）→ `tools/e2e.ps1` 全绿。GUI（tauri）不依赖 net/core 新接口且本次零改动，不重跑 gui check。不改 ghost-push（推送端属非目标）。wire 协议零变化（无新路由/头部/参数），零配置默认行为不变。
````

## File: plan/feature-requests-from-model-copy.md
````markdown
# AutoUpdater 需求提案：换版健壮性与多进程宿主支持

| 元信息 | 内容 |
|---|---|
| 提出方 | Model Copy（DMT-Model-Copy，AFVI 产线模型复制工具，ghost-updater 下游集成方） |
| 日期 | 2026-10-10 |
| 依据 | 本地仓库 commit `33641e5` 源码 + 2026-10-10 真机推送实测（事故与 E2E 日志均在） |
| 状态 | 已实现（2026-10-10，三项全落地；实施取舍见 plan/design.md 追加变更节。REQ-2 回滚语义比原案更稳：只还原已覆盖文件、不碰未覆盖的旧文件；REQ-3 核验后确认 apply 因 body 提取器先行消费、天然无此问题，drain 只落在 stage） |
| 涉及 | `ghost-updater-core`（REQ-2）、`ghost-updater-net`（REQ-1 / REQ-3）；wire 协议不变 |

## 0. 摘要

Model Copy 是"GUI + 常驻更新代理"双进程产品，接入 ghost-updater 后在真实推送中踩到
三个问题：**① swap 半途失败不回滚、不释放锁；② stage/apply 拒绝时不 drain 请求体，
401 被客户端吞成传输错误；③ 多进程宿主缺少"apply 前关闭其他进程"的编排能力**。
三项均为增量改动，默认零配置行为不变，不影响 wire 协议。

---

## 1. 背景与事故时间线

Model Copy 安装形态：`model-copy.exe`（Tauri GUI）+ `mcpy.exe`（CLI + 常驻更新代理，
承载 7600 的 `/update/*` 路由）+ `VERSION`，同目录部署，包内 3 个文件为顶层扁平文件集。

2026-10-10 真机推送（GUI 开着）时间线：

| 时刻 | 事件 |
|---|---|
| 11:10:05 | push → stage 成功 → apply 受理，代理应答后 800ms 自退 |
| 11:10:07 | worker 备份 3 文件后覆盖：VERSION ✓、mcpy.exe ✓、**model-copy.exe ✗（GUI 进程持有文件锁，os error 32）→ swap 半途失败** |
| 之后 | 代理已退、worker 不拉起任何进程 → 7600 死亡，推送台轮询超时；`.apply-lock` 残留 |
| 11:20:57 | 修复主程序选项后重推，同样死在 model-copy.exe 覆盖 |
| 最终 | 宿主侧临时方案（见 REQ-1）落地后 E2E 全过：GUI 被优雅关闭 → 3 文件全换 → 代理自恢复，apply 应答后 ~1s 重启确认 |

---

## 2. 需求详述

### REQ-2（P1）：swap 失败的回滚与锁清理

**问题**：`ghost-updater-core/src/lib.rs` `run_apply()` 中 `swap_files` 的 Err 路径
（L211-L216）只 `log_line` + `write_result` + `return Err`。后果：

1. 安装目录处于**部分覆盖**状态（files 按序覆盖，失败点之前的已换、之后的没换）；
2. `.update_backup\backup-<ts>\` 保留但**无人负责还原**；
3. **`.apply-lock` 泄漏**——锁仅在成功路径释放（L235）；swap 失败与拉起失败两条
   失败路径都不 `release_lock`，该暂存目录永远 409。

错误信息"文件未动或部分覆盖，见备份"实质是把恢复责任推给人工。

**期望**：swap 失败 → 尽力 `restore_backup`（还原失败要 log"需人工介入"）→
`release_lock` → `write_result` 标注"已回滚"；与现有"拉起/健康检查失败"路径的回滚
行为对齐。拉起失败路径顺带补 `release_lock`。

**验收**：

- 锁住安装目录某文件触发 swap 失败 → 安装目录回到一致旧状态；重推同包可成功；
- 失败后暂存区无 `.apply-lock`；`result.json` `ok:false` 且 error 含"已回滚"。

### REQ-3（P2）：stage/apply 拒绝时 drain 请求体，让 401 可见

**问题**：`ghost-updater-net/src/lib.rs` 中 stage/apply 的 `check_auth` 在读 body
**之前**拒绝（鉴权前置是正确的安全设计，保留）。但 axum/hyper 对未消费 body 的请求
发出响应后即关连接：客户端若仍在流式上传大包体（Model Copy 实测 17MB），收到的是
连接重置，reqwest 报 `error sending request for url (...)` 而**不是 401**。
客户端恰好写完 body 时又能读到 401——同一配置下错误表现随机，E2E 无法稳定断言，
真实排障中曾把鉴权问题误判为网络问题。

**期望**：拒绝分支（401/503）在返回前把请求体读完（drain 上限 = `MAX_PACKAGE_BYTES`，
超限直接断连，此时传输错误可接受）。

**验收**：错 token 推 17MB 包 10 次 → 10 次都是 `stage 被拒: 401`（`error_for_status`
可见），0 次传输错误；E2E 增加该断言。

### REQ-1（P2）：多进程宿主的换版编排（apply 前关闭目标进程）

**问题**：apply worker 只等待"受理请求的进程"（`--pid`）退出（`wait_pid_exit`）。
多进程产品的其他常驻进程若持有包内文件，swap 必败（见时间线）。Model Copy 目前以
宿主侧 axum 中间件规避：`/update/apply` 且 Bearer 校验通过时 `taskkill` GUI 再放行
（`mc-cli::apply_kill_gui`，已 E2E 验证）。但这套逻辑**每个多进程宿主都要重写一遍**，
且"多 pid 等待"的本质问题仍在宿主侧无解（worker 不等 GUI 退净，存在关了又没关净的
窗口期）。

**期望**：`UpdateConfig` 增加可选字段 `close_processes: Vec<String>`（进程映像名列表）：

- apply 受理后、自退前，对列表内进程优雅关闭（Windows：`taskkill` 无 `/F` 即
  WM_CLOSE → 限时 5s → `/F` 兜底；非 Windows 可先留空实现）；
- core worker 等待 `--pid` **与列表内全部进程**退出后再 swap；
- 安全：与鉴权同生命周期（未授权 apply 到不了关闭逻辑）——net 侧实现天然满足，
  宿主自实现方案无此保证；
- `close_processes` 为空 = 现状，零配置行为不变。

**非目标（建议）**：换版后被关进程的"自动恢复/重启列表"建议留给宿主（各产品启动
策略差异大，Model Copy 计划用"GUI 原在运行"标记 + 新代理启动时拉起解决）。

**验收**：双进程产品（GUI 开着）推送 → GUI 优雅关闭 → 全量 swap 成功 → main 自恢复；
GUI 未运行时推送行为与现状完全一致；错 token 的 apply 不触发任何进程关闭。

---

## 3. 非目标（本批不做）

- 打包子目录支持（顶层扁平边界维持，Model Copy 暂无子目录资源需求）；
- 推送端（ghost-push / 推送台）改动；
- 多版本并存、回滚 UI、灰度策略。

## 4. 兼容性与安全

- 三项均为增量：REQ-1 新增可选字段，REQ-2/3 是失败路径行为修正；
- wire 协议不变（`close_processes` 是服务端本地配置，不新增路由/头部/查询参数）；
- 现有 14 项协议单测为回归基线，全部应保持绿色。

## 5. 附录：本机实证日志摘录

```text
# apply.log（swap 失败，两次）
[1791598207] 文件替换失败: 覆盖 model-copy.exe 失败: 另一个程序正在使用此文件，
             进程无法访问。 (os error 32)(文件未动或部分覆盖,见备份)
[1791598858] （同上，重推复现）
# apply.log（宿主侧 workaround 后，成功）
[1791599470] 文件替换完成(3 个),备份: ...backup-1791599470
[1791599472] 更新成功,新进程 pid=25824 存活
# 推送台侧（REQ-3 症状）
stage 上传失败: error sending request for url (http://.../update/stage?...)   ← 实为 401
stage 被拒: HTTP status client error (401 Unauthorized)                       ← 同配置偶发
```

---
---

# (EN) Feature Proposal: Apply Robustness & Multi-Process Host Support

| Meta | Content |
|---|---|
| Proposed by | Model Copy (DMT-Model-Copy, AFVI production-line model copy tool; ghost-updater downstream integrator) |
| Date | 2026-10-10 |
| Basis | Local repo commit `33641e5` + real-machine push tests on 2026-10-10 (incident & E2E logs available) |
| Status | Draft, pending review |
| Touches | `ghost-updater-core` (REQ-2), `ghost-updater-net` (REQ-1 / REQ-3); wire protocol unchanged |

## 0. Summary

Model Copy is a dual-process product (GUI + resident update agent). Real-world pushes
exposed three issues: **① a mid-swap failure performs no rollback and leaks the apply
lock; ② stage/apply rejections don't drain the request body, so clients see a transport
error instead of 401; ③ multi-process hosts have no orchestration for closing sibling
processes before apply**. All three are incremental changes; default zero-config
behavior is unchanged and the wire protocol is untouched.

## 1. Background & incident timeline

Deployment: `model-copy.exe` (Tauri GUI) + `mcpy.exe` (CLI + resident update agent
serving `/update/*` on 7600) + `VERSION`, flat 3-file package.

| Time | Event |
|---|---|
| 11:10:05 | push → stage OK → apply accepted; agent self-exits 800ms after ack |
| 11:10:07 | worker backs up 3 files, overwrites: VERSION ✓, mcpy.exe ✓, **model-copy.exe ✗ (locked by running GUI, os error 32) → swap fails midway** |
| after | agent gone, worker relaunches nothing → 7600 dead, pusher polls till timeout; `.apply-lock` left behind |
| 11:20:57 | re-push with corrected `main` — same death on model-copy.exe |
| final | after host-side workaround (see REQ-1) E2E passes: GUI gracefully closed → all 3 files swapped → agent self-recovers, restart confirmed ~1s after apply ack |

## 2. Requirements

### REQ-2 (P1): Rollback & lock cleanup on swap failure

**Problem**: in `ghost-updater-core/src/lib.rs` `run_apply()`, the `swap_files` error
path (L211-L216) only logs, writes the result, and returns. Consequences:

1. the install dir is left **partially overwritten** (files are copied in order);
2. `.update_backup\backup-<ts>\` remains with **no one responsible for restoring**;
3. **`.apply-lock` leaks** — the lock is released only on the success path (L235);
   neither the swap-failure nor the relaunch-failure path calls `release_lock`, so the
   staging dir answers 409 forever.

The message "文件未动或部分覆盖，见备份" ("untouched or partially overwritten, see
backup") effectively hands recovery to a human.

**Expected**: on swap failure → best-effort `restore_backup` (log "manual intervention
needed" if restore itself fails) → `release_lock` → mark "rolled back" in the result;
align with the existing relaunch/health-check failure path. Also add `release_lock` to
the relaunch-failure path.

**Acceptance**:

- Locking a file in the install dir to force a swap failure → install dir returns to a
  consistent old state; re-pushing the same package succeeds;
- no `.apply-lock` left after the failure; `result.json` `ok:false` with "rolled back"
  in the error.

### REQ-3 (P2): Drain request body on stage/apply rejection so 401 is visible

**Problem**: stage/apply `check_auth` rejects **before** reading the body (auth-first
is the right security design — keep it). But axum/hyper closes the connection after
responding to a request whose body was never consumed: a client still streaming a large
body (17 MB in our tests) gets a connection reset and reqwest reports
`error sending request for url (...)` **instead of 401**. If the client happens to
finish writing first, it does read the 401 — so under identical configuration the
failure surfaces randomly, E2E cannot assert it, and a real diagnosis initially
mistook the auth problem for a network problem.

**Expected**: on the rejection path (401/503), drain the body before responding
(cap = `MAX_PACKAGE_BYTES`; beyond that, dropping the connection is acceptable).

**Acceptance**: push a 17 MB package with a wrong token 10 times → 10 × `stage rejected:
401` (visible via `error_for_status`), 0 transport errors; add this assertion to E2E.

### REQ-1 (P2): Multi-process orchestration — close target processes before apply

**Problem**: the apply worker only waits for the process that accepted the request
(`--pid`, via `wait_pid_exit`). If any other resident process of the product holds a
packaged file, the swap necessarily fails (see timeline). Model Copy currently works
around this with a host-side axum middleware: on `/update/apply` with a valid Bearer,
gracefully kill the GUI before forwarding (`mc-cli::apply_kill_gui`, E2E-verified).
But **every multi-process host would have to rewrite this**, and the underlying
"wait for multiple pids" problem remains unsolved on the host side (the worker doesn't
wait for the GUI to fully exit).

**Expected**: optional `UpdateConfig` field `close_processes: Vec<String>` (image names):

- after apply is accepted and before self-exit, gracefully close listed processes
  (Windows: `taskkill` without `/F` (WM_CLOSE) → 5 s grace → `/F` fallback;
  non-Windows may start as a no-op);
- the worker waits for `--pid` **and all listed processes** to exit before swapping;
- security: lifecycle-bound to auth (an unauthorized apply never reaches the closing
  logic) — natural in a net-side implementation, not guaranteed by host-made ones;
- empty list = current behavior; zero-config unchanged.

**Suggested non-goal**: auto-restarting closed processes after the update is best left
to hosts (startup policies differ per product; Model Copy plans a "GUI was running"
marker + relaunch-on-agent-start approach).

**Acceptance**: pushing to a dual-process product with the GUI open → GUI gracefully
closed → full swap succeeds → `main` self-recovers; with the GUI not running, behavior
is identical to today; an apply with a wrong token never closes anything.

## 3. Non-goals (this batch)

- subdirectory packaging (flat top-level boundary stays; Model Copy needs none yet);
- pusher-side changes (ghost-push / console);
- multi-version coexistence, rollback UI, canary strategies.

## 4. Compatibility & security

- All three are additive: REQ-1 adds an optional field; REQ-2/3 fix failure-path
  behavior;
- wire protocol unchanged (`close_processes` is server-local config — no new routes,
  headers, or query params);
- the existing 14 protocol unit tests are the regression baseline and must stay green.

## 5. Appendix: log excerpts

```text
# apply.log (swap failure, twice)
[1791598207] 文件替换失败: 覆盖 model-copy.exe 失败: ... (os error 32)(文件未动或部分覆盖,见备份)
[1791598858] (reproduced on re-push)
# apply.log (after host-side workaround, success)
[1791599470] 文件替换完成(3 个)
[1791599472] 更新成功,新进程 pid=25824 存活
# pusher side (REQ-3 symptom)
stage 上传失败: error sending request for url (...)   ← actually a 401
stage 被拒: HTTP status client error (401 Unauthorized) ← same config, intermittent
```

---
---

# (한국어) 기능 제안: 적용(apply) 견고성 및 다중 프로세스 호스트 지원

| 메타 | 내용 |
|---|---|
| 제안자 | Model Copy (DMT-Model-Copy, AFVI 생산라인 모델 복제 도구, ghost-updater 다운스트림 통합자) |
| 날짜 | 2026-10-10 |
| 근거 | 로컬 저장소 commit `33641e5` 및 2026-10-10 실기 푸시 테스트(장애·E2E 로그 보유) |
| 상태 | 초안, 검토 대기 |
| 대상 | `ghost-updater-core`(REQ-2), `ghost-updater-net`(REQ-1 / REQ-3), wire 프로토콜 변경 없음 |

## 0. 요약

Model Copy는 GUI + 상주 업데이트 에이전트의 이중 프로세스 제품입니다. 실제 푸시에서
세 가지 문제가 확인되었습니다. **① swap 도중 실패 시 롤백 없음 및 apply 잠금 누수,
② stage/apply 거부 시 요청 본문을 drain하지 않아 클라이언트가 401 대신 전송 오류를
보게 됨, ③ 다중 프로세스 호스트에 apply 전 타 프로세스 종료 오케스트레이션 부재**.
세 항목 모두 점진적 변경이며, 기본 제로 구성 동작은 불변이고 wire 프로토콜에 영향이
없습니다.

## 1. 배경 및 장애 타임라인

배포 형태: `model-copy.exe`(Tauri GUI) + `mcpy.exe`(CLI + 상주 업데이트 에이전트,
7600 포트의 `/update/*` 라우트 담당) + `VERSION`, 동일 디렉터리의 플랫 3파일 패키지.

| 시각 | 이벤트 |
|---|---|
| 11:10:05 | push → stage 성공 → apply 수락, 에이전트는 ack 800ms 후 자체 종료 |
| 11:10:07 | worker가 3파일 백업 후 덮어쓰기: VERSION ✓, mcpy.exe ✓, **model-copy.exe ✗(실행 중 GUI가 파일 잠금, os error 32) → swap 중도 실패** |
| 이후 | 에이전트 종료, worker는 아무 프로세스도 기동하지 않음 → 7600 사망, 푸셔는 타임아웃까지 폴링, `.apply-lock` 잔존 |
| 11:20:57 | main 수정 후 재푸시 — model-copy.exe에서 동일하게 실패 |
| 최종 | 호스트 측 워크어라운드(REQ-1 참조) 적용 후 E2E 통과: GUI 우아하게 종료 → 3파일 전부 교체 → 에이전트 자가 복구, apply ack 후 약 1초에 재시작 확인 |

## 2. 요구사항 상세

### REQ-2 (P1): swap 실패 시 롤백 및 잠금 정리

**문제**: `ghost-updater-core/src/lib.rs`의 `run_apply()`에서 `swap_files` 오류 경로
(L211-L216)는 로그 기록 + 결과 저장 + 반환만 합니다. 결과:

1. 설치 디렉터리가 **부분 덮어쓰기** 상태로 남음(파일은 순서대로 복사됨);
2. `.update_backup\backup-<ts>\`가 남지만 **복원을 담당하는 주체가 없음**;
3. **`.apply-lock` 누수** — 잠금은 성공 경로에서만 해제됨(L235). swap 실패와 기동
   실패 어느 쪽도 `release_lock`을 호출하지 않아 해당 스테이징 디렉터리는 영원히 409.

"파일 미변경 또는 부분 덮어쓰기, 백업 참조"라는 메시지는 사실상 복구 책임을 사람에게
넘기는 것입니다.

**기대**: swap 실패 시 → 최선을 다해 `restore_backup`(복원 자체 실패 시 "수동 개입
필요" 로그) → `release_lock` → 결과에 "롤백됨" 표기. 기존 기동/헬스체크 실패 경로의
롤백 동작과 정렬. 기동 실패 경로에도 `release_lock` 추가.

**수용 기준**:

- 설치 디렉터리의 파일을 잠가 swap 실패를 유발 → 설치 디렉터리가 일관된 이전 상태로
  복원, 동일 패키지 재푸시 성공;
- 실패 후 스테이징에 `.apply-lock` 없음, `result.json` `ok:false` 및 error에
  "롤백됨" 포함.

### REQ-3 (P2): stage/apply 거부 시 요청 본문 drain으로 401 가시화

**문제**: stage/apply의 `check_auth`는 본문을 읽기 **전에** 거부합니다(인증 우선은
올바른 보안 설계이므로 유지). 그러나 axum/hyper는 본문이 소비되지 않은 요청에 응답한
후 연결을 닫습니다. 클라이언트가 대용량 본문(테스트에서 17MB)을 스트리밍 중이면 연결
리셋을 받고 reqwest는 401이 아닌 `error sending request for url (...)`을 보고합니다.
클라이언트가 우연히 먼저 쓰기를 끝내면 401을 읽으므로 — 동일한 설정에서 오류 표현이
무작위로 바뀌어 E2E 단정이 불가능하고, 실제 장애 분석에서 인증 문제를 네트워크 문제로
오인한 사례가 있었습니다.

**기대**: 거부 경로(401/503)에서는 응답 전에 본문을 끝까지 읽음(drain 상한 =
`MAX_PACKAGE_BYTES`, 초과 시 연결 끊기 허용).

**수용 기준**: 잘못된 토큰으로 17MB 패키지를 10회 푸시 → 10회 모두 `stage 거부: 401`
(`error_for_status`로 가시), 전송 오류 0회. E2E에 해당 단언 추가.

### REQ-1 (P2): 다중 프로세스 오케스트레이션 — apply 전 대상 프로세스 종료

**문제**: apply worker는 요청을 수락한 프로세스(`--pid`, `wait_pid_exit`)만 대기합니다.
제품의 다른 상주 프로세스가 패키지 파일을 잠고 있으면 swap은 반드시 실패합니다(타임라인
참조). Model Copy는 현재 호스트 측 axum 미들웨어로 회피합니다: 유효한 Bearer와 함께
`/update/apply`가 오면 전달 전에 GUI를 우아하게 종료(`mc-cli::apply_kill_gui`,
E2E 검증 완료). 그러나 **모든 다중 프로세스 호스트가 이를 재작성해야 하며**, 근본적인
"다중 pid 대기" 문제는 호스트 측에서 해결되지 않습니다(worker는 GUI의 완전한 종료를
기다리지 않음).

**기대**: `UpdateConfig`에 선택 필드 `close_processes: Vec<String>`(프로세스 이미지명
목록) 추가:

- apply 수락 후 자체 종료 전에 목록의 프로세스를 우아하게 종료(Windows: `/F` 없는
  `taskkill`(WM_CLOSE) → 5초 유예 → `/F` 폴백, 비 Windows는 우선 no-op 가능);
- worker는 swap 전에 `--pid` **및 목록의 모든 프로세스** 종료를 대기;
- 보안: 인증과 수명주기가 동일(미인증 apply는 종료 로직에 도달하지 않음) — net 측
  구현에서는 자연스럽게 보장되지만 호스트 자체 구현에서는 보장되지 않음;
- 빈 목록 = 현재 동작, 제로配置 변경 없음.

**권장 비목표**: 업데이트 후 종료된 프로세스의 자동 재시작은 호스트에 맡기는 것이
좋음(제품별 기동 정책이 다름. Model Copy는 "GUI 실행 중이었음" 마커 + 에이전트 시작 시
기동 방식으로 계획).

**수용 기준**: GUI가 열린 이중 프로세스 제품에 푸시 → GUI 우아하게 종료 → 전체 swap
성공 → `main` 자가 복구. GUI가 실행 중이 아닐 때는 오늘날과 동일한 동작. 잘못된 토큰의
apply는 어떤 프로세스도 종료하지 않음.

## 3. 비목표 (이번 배치)

- 하위 디렉터리 패키징(플랫 최상위 경계 유지, Model Copy는 아직 필요 없음);
- 푸셔 측 변경(ghost-push / 콘솔);
- 다중 버전 공존, 롤백 UI, 카나리아 전략.

## 4. 호환성 및 보안

- 세 항목 모두 점진적: REQ-1은 선택 필드 추가, REQ-2/3은 실패 경로 동작 수정;
- wire 프로토콜 불변(`close_processes`는 서버 로컬 설정 — 새 라우트/헤더/쿼리 파라미터
  없음);
- 기존 14개 프로토콜 단위 테스트가 회귀 기준이며 모두 녹색 유지여야 함.

## 5. 부록: 로그 발췌

```text
# apply.log (swap 실패, 두 번)
[1791598207] 文件替换失败: 覆盖 model-copy.exe 失败: ... (os error 32)(文件未动或部分覆盖,见备份)
[1791598858] (재푸시로 재현)
# apply.log (호스트 측 워크어라운드 후, 성공)
[1791599470] 文件替换完成(3 个)
[1791599472] 更新成功,新进程 pid=25824 存活
# 푸셔 측 (REQ-3 증상)
stage 上传失败: error sending request for url (...)   ← 실제로는 401
stage 被拒: HTTP status client error (401 Unauthorized) ← 동일 설정에서 간헐적
```
````

## File: tools/gui.cmd
````batch
@echo off
rem Thin wrapper for tools\gui.ps1 so you can skip -ExecutionPolicy Bypass -File.
rem Usage: tools\gui.cmd dev^|run^|build^|check^|installer^|verify
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0gui.ps1" %*
exit /b %ERRORLEVEL%
````

## File: tools/gui.ps1
````powershell
# gui.ps1 — ghost-updater 推送台(Tauri GUI)构建 / 运行 / 打包统一入口
#
# 为什么需要它:debug 构建下 Tauri 会忽略已构建的 webdist,改让窗口加载
# tauri.conf.json 的 devUrl(http://localhost:5173),而那个 Vite 开发服务器
# 只有 `tauri dev` 才会自动拉起。直接 `cargo run` 必然报 ERR_CONNECTION_REFUSED。
# 所以想跑窗口,要么走 dev(要热更新),要么走 run(用已构建前端,官方路径)。
#
# 用法: powershell -ExecutionPolicy Bypass -File tools\gui.ps1 <命令>
#       或直接用薄包装: tools\gui.cmd <命令>
$ErrorActionPreference = "Stop"

$Root = Split-Path -Parent $PSScriptRoot              # 仓库根
$Gui  = Join-Path $Root "crates\ghost-updater-tauri"  # 前端工程(vite/npm 在这里)
$App  = Join-Path $Gui "src-tauri"                    # Rust 工程
$Exe  = Join-Path $App "target\debug\ghost-updater-tauri.exe"

function Fail($msg) { Write-Host "[FAIL] $msg" -ForegroundColor Red; exit 1 }
function Ok($msg)   { Write-Host "[OK]   $msg" -ForegroundColor Green }

function Need-Npm {
    if (-not (Test-Path (Join-Path $Gui "node_modules"))) {
        Fail "缺少 node_modules,先执行: cd crates\ghost-updater-tauri; npm install"
    }
}

# npm/cargo 的 stderr 在 $ErrorActionPreference=Stop 下可能变 terminating error,
# 统一经 cmd 调用只看退出码
function Invoke-Cmd([string]$workDir, [string]$argLine) {
    Write-Host "> $argLine" -ForegroundColor DarkGray
    Push-Location $workDir
    try { cmd /c "$argLine 2>&1"; $code = $LASTEXITCODE } finally { Pop-Location }
    if ($code -ne 0) { Fail "$argLine 退出码 $code" }
}

function Build-Frontend { Need-Npm; Invoke-Cmd $Gui "npm run build" }

# 上一次运行的推送台实例会锁住目标 exe,导致 cargo 覆盖时报 os error 5(拒绝访问)
function Stop-Running {
    $p = Get-Process ghost-updater-tauri -ErrorAction SilentlyContinue
    if ($p) {
        Write-Host "先关闭仍在运行的推送台实例(PID $($p.Id -join ','))" -ForegroundColor Yellow
        $p | Stop-Process -Force -ErrorAction SilentlyContinue
        Start-Sleep -Milliseconds 500
    }
}

# 开发模式:CLI 会先跑 beforeDevCommand(npm run dev)起 Vite,再开窗口,改前端热更新
function Cmd-Dev {
    Need-Npm
    Stop-Running
    Invoke-Cmd $Gui "npm run tauri -- dev"
}

# 本地跑一次"打完包的界面":Rust 用 debug 编译,前端用已构建的 webdist(不走 devUrl)
function Cmd-Run {
    Need-Npm
    Stop-Running
    Invoke-Cmd $Gui "npm run tauri -- build --debug --no-bundle"
    if (-not (Test-Path $Exe)) { Fail "未找到 $Exe" }
    Ok "启动 $Exe"
    & $Exe
}

# 只重新构建前端 → webdist
function Cmd-Build {
    Build-Frontend
    Ok "webdist 就绪: $(Join-Path $Gui 'webdist')"
}

# 前端构建 + GUI crate 类型检查(generate_context! 编译期要读 webdist,故不能省前端这步)
function Cmd-Check {
    Build-Frontend
    Invoke-Cmd $App "cargo check"
    Ok "GUI crate cargo check 通过"
}

# 正式安装包(NSIS)
function Cmd-Installer {
    Need-Npm
    Stop-Running
    Invoke-Cmd $Gui "npm run tauri -- build"
    $dir = Join-Path $App "target\release\bundle\nsis"
    Ok "安装包输出目录: $dir"
    Get-ChildItem $dir -Filter *.exe -ErrorAction SilentlyContinue | ForEach-Object { Write-Host "       $($_.Name)" }
}

# 主 workspace 全量核验 + GUI check
function Cmd-Verify {
    Invoke-Cmd $Root "cargo build --workspace"
    Invoke-Cmd $Root "cargo test --workspace"
    Cmd-Check
    Ok "workspace build/test + GUI check 全绿"
}

function Usage {
    Write-Host "ghost-updater 推送台(GUI)入口脚本"
    Write-Host ""
    Write-Host "  dev        开发模式:起 Vite + 开窗口,前端热更新"
    Write-Host "  run        跑打包版界面:debug 编译 + 内嵌 webdist(绕过 devUrl)"
    Write-Host "  build      只构建前端 → webdist"
    Write-Host "  check      构建前端 + GUI crate cargo check"
    Write-Host "  installer  正式打包 → NSIS 安装包"
    Write-Host "  verify     主 workspace build/test + GUI check"
    Write-Host ""
    Write-Host "不要直接 cargo run:debug 会去连 devUrl,无人起 Vite 必报连接被拒。"
}

switch ($args[0]) {
    "dev"       { Cmd-Dev }
    "run"       { Cmd-Run }
    "build"     { Cmd-Build }
    "check"     { Cmd-Check }
    "installer" { Cmd-Installer }
    "verify"    { Cmd-Verify }
    default     { Usage; if ($args[0]) { exit 1 } }
}
````

## File: crates/demo-app/src/main.rs
````rust
//! demo-app:ghost-updater 嵌入式集成的最小宿主示例。
//!
//! 接入总量 = main 顶部一行 guard + Router 合并一行 routes(),其余全是业务代码。

use axum::routing::get;

fn main() {
    // 拦截内部触发的更新替换模式:如果是 apply worker,它会等旧进程退出、
    // 换文件、重启新版,然后直接 exit,永不返回到这里
    ghost_updater_core::guard_apply_update();

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async_main());
}

async fn async_main() {
    let addr: std::net::SocketAddr = parse_addr();
    let token = std::env::args()
        .position(|a| a == "--token")
        .and_then(|i| std::env::args().nth(i + 1));
    // REQ-1 多进程宿主:apply 受理后需优雅关闭的进程映像名,逗号分隔
    let close_processes: Vec<String> = std::env::args()
        .position(|a| a == "--close-processes")
        .and_then(|i| std::env::args().nth(i + 1))
        .map(|s| {
            s.split(',')
                .map(|n| n.trim().to_string())
                .filter(|n| !n.is_empty())
                .collect()
        })
        .unwrap_or_default();

    let app = axum::Router::new()
        .route("/hello", get(|| async { "world" }))
        .merge(ghost_updater_net::routes(ghost_updater_net::UpdateConfig {
            version: app_version(),
            auth_token: token,
            close_processes,
            ..Default::default()
        }));
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    println!("[demo] v{} 监听 {addr}", app_version());
    axum::serve(listener, app).await.unwrap();
}

fn parse_addr() -> std::net::SocketAddr {
    let a = std::env::args()
        .position(|arg| arg == "--addr")
        .and_then(|i| std::env::args().nth(i + 1))
        .unwrap_or_else(|| "127.0.0.1:7600".into());
    a.parse().unwrap_or_else(|_| panic!("--addr 非法: {a}"))
}

/// 版本号从 exe 旁 VERSION 文件读(换版即变化);缺省用包版本
fn app_version() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|p| std::fs::read_to_string(p.with_file_name("VERSION")).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string())
}
````

## File: crates/demo-app/Cargo.toml
````toml
[package]
name = "demo-app"
description = "演示宿主:main 顶部一行 guard + axum 合并 /update/* 路由,即完成一键更新接入"
version.workspace = true
edition.workspace = true
license.workspace = true

[[bin]]
name = "ghost-demo"
path = "src/main.rs"

[dependencies]
ghost-updater-core = { path = "../ghost-updater-core" }
ghost-updater-net = { path = "../ghost-updater-net" }
axum = { workspace = true }
tokio = { workspace = true }
````

## File: crates/ghost-updater-cli/src/lib.rs
````rust
//! ghost-updater-cli 推送端编排:探测 → 暂存 → 触发 → 轮询确认。
//! GUI(Tauri)与 CLI(ghost-push)共用本库;进度全部走 on_event 文本行。

use ed25519_dalek::{Signer, SigningKey};
use ghost_updater_core::{unix_secs, StageManifest};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// 与 ghost-updater-net 对齐的默认端口(避开常见 Hyper-V 保留段,如 9595-9694)
pub const DEFAULT_PORT: &str = "7600";
/// 默认不覆盖的宿主自定义配置
pub const PROTECTED_FILES: &[&str] = &["config.json"];

// ---------- wire 类型(与 ghost-updater-net 响应对齐) ----------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProbeInfo {
    pub version: String,
    pub started_at: u64,
    pub os: String,
    pub arch: String,
    #[serde(default)]
    pub pubkey_fingerprint: String,
}

#[derive(Debug, Deserialize)]
struct StageAck {
    #[serde(default)]
    staging: String,
}

#[derive(Serialize)]
struct ApplyBody<'a> {
    staging: &'a str,
    #[serde(default)]
    relaunch_args: Vec<String>,
}

// ---------- 编排配置与结果 ----------

#[derive(Clone)]
pub struct PushUpdateCfg {
    /// 目标地址 host[:9600]
    pub addr: String,
    /// 本机新版 dist 目录(顶层普通文件即更新文件集)
    pub dist_dir: PathBuf,
    /// 主程序文件名;None = dist 内唯一 *.exe
    pub main: Option<String>,
    pub token: Option<String>,
    /// ed25519 私钥 hex(64 字符);Some 时对整包摘要签名(X-Signature)
    pub sign_key_hex: Option<String>,
    /// true = 连 config.json 也覆盖(默认保留目标机配置)
    pub overwrite_config: bool,
    /// 轮询确认总超时
    pub poll_timeout: Duration,
    /// 进度行接收端(每行一条;None = 静默)
    pub on_event: Option<tokio::sync::mpsc::Sender<String>>,
}

impl PushUpdateCfg {
    pub fn new(addr: impl Into<String>, dist_dir: PathBuf) -> Self {
        Self {
            addr: addr.into(),
            dist_dir,
            main: None,
            token: None,
            sign_key_hex: None,
            overwrite_config: false,
            poll_timeout: Duration::from_secs(120),
            on_event: None,
        }
    }
}

#[derive(Debug)]
pub struct UpdateReport {
    pub addr: String,
    pub ok: bool,
    pub old_version: String,
    pub new_version: String,
    pub detail: String,
}

pub struct PushCtx {
    pub client: reqwest::Client,
    signing_key: Option<SigningKey>,
}

impl PushCtx {
    pub fn new(sign_key_hex: Option<&str>) -> Result<Self, String> {
        let signing_key = match sign_key_hex {
            Some(h) => {
                let b = hex::decode(h.trim()).map_err(|e| format!("sign-key hex 非法: {e}"))?;
                let b: [u8; 32] = b.try_into().map_err(|_| "sign-key 必须 32 字节 hex")?;
                Some(SigningKey::from_bytes(&b))
            }
            None => None,
        };
        Ok(Self {
            client: reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(10))
                .build()
                .map_err(|e| format!("构建 HTTP 客户端失败: {e}"))?,
            signing_key,
        })
    }
}

/// "192.168.1.60" → "192.168.1.60:9600";已带端口原样
pub fn normalize_addr(input: &str) -> String {
    let s = input.trim();
    if s.is_empty() || s.contains(':') {
        s.to_string()
    } else {
        format!("{s}:{DEFAULT_PORT}")
    }
}

fn base_url(addr: &str) -> String {
    format!("http://{}", normalize_addr(addr))
}

// ---------- 探测 ----------

pub async fn probe(ctx: &PushCtx, addr: &str, token: Option<&str>) -> Result<ProbeInfo, String> {
    let url = format!("{}/update/probe", base_url(addr));
    let mut req = ctx.client.get(&url).timeout(Duration::from_secs(10));
    if let Some(t) = token {
        req = req.bearer_auth(t);
    }
    req.send()
        .await
        .map_err(|e| format!("连接 {addr} 失败: {e}"))?
        .error_for_status()
        .map_err(|e| format!("probe {addr}: {e}"))?
        .json::<ProbeInfo>()
        .await
        .map_err(|e| format!("probe 响应解析失败: {e}"))
}

// ---------- dist 打包 ----------

/// 收集 dist 顶层普通文件(跳过目录、点开头文件;config.json 默认保护)。
/// 返回 (文件名列表, 主程序名)。
pub fn collect_dist_files(dist: &Path, main: Option<&str>, overwrite_config: bool) -> Result<(Vec<String>, String), String> {
    let mut files: Vec<String> = Vec::new();
    for e in std::fs::read_dir(dist).map_err(|e| format!("读 dist 失败: {}", e))? {
        let e = e.map_err(|e| e.to_string())?;
        let name = e.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }
        if !e.file_type().map_err(|e| e.to_string())?.is_file() {
            continue;
        }
        if name == "config.json" && !overwrite_config {
            continue;
        }
        files.push(name);
    }
    files.sort();
    let main = match main {
        Some(m) => m.to_string(),
        None => {
            let exes: Vec<&String> = files.iter().filter(|f| f.to_ascii_lowercase().ends_with(".exe")).collect();
            match exes.len() {
                1 => exes[0].clone(),
                0 => return Err("dist 内没有 *.exe,需用 --main 指定主程序".into()),
                _ => return Err(format!("dist 内有多个 *.exe({}),需用 --main 指定主程序", exes.len())),
            }
        }
    };
    if !files.contains(&main) {
        return Err(format!("dist 内没有主程序 {main}"));
    }
    Ok((files, main))
}

/// 打包 dist 为 tar(临时文件),返回 (路径, 整包 SHA256 hex)。
pub fn pack_dist(dist: &Path, files: &[String]) -> Result<(PathBuf, String), String> {
    let (tar_path, sha256) = pack_dist_to_file(dist, files)?;
    Ok((tar_path, sha256))
}

fn pack_dist_to_file(dist: &Path, files: &[String]) -> Result<(PathBuf, String), String> {
    let tar_path = std::env::temp_dir().join(format!("ghost-push-{}-{}.tar", std::process::id(), unix_secs()));
    let f = std::fs::File::create(&tar_path).map_err(|e| format!("建临时包失败: {e}"))?;
    let mut b = tar::Builder::new(f);
    for name in files {
        let mut src = std::fs::File::open(dist.join(name)).map_err(|e| format!("打开 {name}: {e}"))?;
        let mut h = tar::Header::new_gnu();
        let meta = src.metadata().map_err(|e| format!("stat {name}: {e}"))?;
        h.set_size(meta.len());
        h.set_mode(0o644);
        h.set_cksum();
        b.append_data(&mut h, name, &mut src).map_err(|e| format!("打包 {name}: {e}"))?;
    }
    b.into_inner().map_err(|e| format!("收尾 tar: {e}"))?;
    // 写完整包再算哈希:整包读一遍比往 Builder 里插 tee 层直白可靠
    let mut hasher = Sha256::new();
    let mut r = std::fs::File::open(&tar_path).map_err(|e| format!("读回临时包: {e}"))?;
    std::io::copy(&mut r, &mut hasher).map_err(|e| format!("算哈希: {e}"))?;
    Ok((tar_path, hex::encode(hasher.finalize())))
}

/// dist 打包进内存,返回 (tar 字节, 整包 SHA256 hex);GUI 选包预览用(push 走文件版)
pub fn pack_dist_in_memory(dist: &Path, files: &[String]) -> Result<(Vec<u8>, String), String> {
    let mut b = tar::Builder::new(Vec::new());
    for name in files {
        let mut src = std::fs::File::open(dist.join(name)).map_err(|e| format!("打开 {name}: {e}"))?;
        let mut h = tar::Header::new_gnu();
        let meta = src.metadata().map_err(|e| format!("stat {name}: {e}"))?;
        h.set_size(meta.len());
        h.set_mode(0o644);
        h.set_cksum();
        b.append_data(&mut h, name, &mut src).map_err(|e| format!("打包 {name}: {e}"))?;
    }
    let buf = b.into_inner().map_err(|e| format!("收尾 tar: {e}"))?;
    let sha = hex::encode(Sha256::digest(&buf));
    Ok((buf, sha))
}

// ---------- 完整推送流程 ----------

pub async fn push_update(ctx: &PushCtx, cfg: PushUpdateCfg) -> UpdateReport {
    let mut report = UpdateReport {
        addr: normalize_addr(&cfg.addr),
        ok: false,
        old_version: String::new(),
        new_version: String::new(),
        detail: String::new(),
    };
    let say = |m: String| {
        if let Some(tx) = &cfg.on_event {
            let _ = tx.try_send(m);
        }
    };

    // dist 校验 + 打包
    let (files, main) = match collect_dist_files(&cfg.dist_dir, cfg.main.as_deref(), cfg.overwrite_config) {
        Ok(r) => r,
        Err(e) => {
            report.detail = e;
            return report;
        }
    };
    let expected_version = std::fs::read_to_string(cfg.dist_dir.join("VERSION"))
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let (tar_path, sha256) = match pack_dist(&cfg.dist_dir, &files) {
        Ok(r) => r,
        Err(e) => {
            report.detail = e;
            return report;
        }
    };
    say(format!("[pack] {} 个文件,{} 字节,sha256={sha256}", files.len(),
        std::fs::metadata(&tar_path).map(|m| m.len()).unwrap_or(0)));

    // 探测
    say(format!("[push] 连接 {} ...", report.addr));
    let old = match probe(ctx, &cfg.addr, cfg.token.as_deref()).await {
        Ok(p) => p,
        Err(e) => {
            let _ = std::fs::remove_file(&tar_path);
            report.detail = e;
            return report;
        }
    };
    report.old_version = old.version.clone();
    let old_started_at = old.started_at;
    say(format!(
        "[push] 目标在线 version={} started_at={} os={} arch={}",
        old.version, old.started_at, old.os, old.arch
    ));

    // 暂存(流式上传 + 整包哈希)
    let stage_url = format!("{}/update/stage?sha256={sha256}&main={main}", base_url(&cfg.addr));
    say("[push] 上传暂存 ...".into());
    let file = match tokio::fs::File::open(&tar_path).await {
        Ok(f) => f,
        Err(e) => {
            let _ = std::fs::remove_file(&tar_path);
            report.detail = format!("打开临时包失败: {e}");
            return report;
        }
    };
    let stream = tokio_util::codec::FramedRead::new(file, tokio_util::codec::BytesCodec::new());
    let mut req = ctx
        .client
        .post(&stage_url)
        .timeout(Duration::from_secs(600))
        .header("content-type", "application/x-tar");
    if let Some(t) = &cfg.token {
        req = req.bearer_auth(t);
    }
    if let Some(sk) = &ctx.signing_key {
        let digest = hex::decode(&sha256).expect("sha256 已是合法 hex");
        req = req.header("x-signature", hex::encode(sk.sign(&digest).to_bytes()));
    }
    let stage_resp = match req.body(reqwest::Body::wrap_stream(stream)).send().await {
        Ok(r) => match r.error_for_status() {
            Ok(r) => r,
            Err(e) => {
                let _ = std::fs::remove_file(&tar_path);
                report.detail = format!("stage 被拒: {e}");
                return report;
            }
        },
        Err(e) => {
            let _ = std::fs::remove_file(&tar_path);
            report.detail = format!("stage 上传失败: {e}");
            return report;
        }
    };
    let ack: StageAck = match stage_resp.json().await {
        Ok(a) => a,
        Err(e) => {
            report.detail = format!("stage 响应解析失败: {e}");
            return report;
        }
    };
    say(format!("[push] 已暂存 {}({} 个文件)", ack.staging, files.len()));

    // 触发自替换
    say("[push] apply(目标即将自替换重启)...".into());
    let apply_url = format!("{}/update/apply", base_url(&cfg.addr));
    let mut req = ctx.client.post(&apply_url).timeout(Duration::from_secs(15));
    if let Some(t) = &cfg.token {
        req = req.bearer_auth(t);
    }
    let resp = req
        .json(&ApplyBody { staging: &ack.staging, relaunch_args: vec![] })
        .send()
        .await;
    match resp {
        Ok(r) if r.status().is_success() => {}
        Ok(r) => {
            let code = r.status();
            let body = r.text().await.unwrap_or_default();
            report.detail = format!("apply 失败({code}): {body}");
            return report;
        }
        Err(e) => {
            report.detail = format!("apply 请求失败: {e}");
            return report;
        }
    }

    // 轮询确认:started_at 变化才算重启(网络空窗判定是竞态,已实测踩坑)。
    // 版本规则:dist 声明了不同版本 → 必须看到新版本;同版本重部署/无 VERSION → 重启即成功。
    let deadline = tokio::time::Instant::now() + cfg.poll_timeout;
    let t0 = std::time::Instant::now();
    loop {
        tokio::time::sleep(Duration::from_secs(1)).await;
        if tokio::time::Instant::now() >= deadline {
            report.detail = "超时未确认到新版本(目标可能仍在重启或已回滚;查目标机 .update_backup/apply.log)".into();
            return report;
        }
        let at = t0.elapsed().as_millis();
        let cur = match probe(ctx, &cfg.addr, cfg.token.as_deref()).await {
            Ok(p) => p,
            Err(e) => {
                say(format!("[poll {at}ms] 连接失败(重启窗口): {e}"));
                continue;
            }
        };
        let restarted = cur.started_at != old_started_at;
        say(format!(
            "[poll {at}ms] 在线 version={} started_at={} restarted={restarted}",
            cur.version, cur.started_at
        ));
        let success = match &expected_version {
            Some(ev) if ev != &report.old_version => cur.version == *ev,
            Some(ev) => restarted && cur.version == *ev,
            None => restarted,
        };
        if success {
            report.ok = true;
            report.new_version = cur.version;
            say(format!("[push] 更新成功: {} → {}(重启确认)", report.old_version, report.new_version));
            return report;
        }
    }
}

// ---------- keygen ----------

/// 生成 ed25519 密钥对(服务端配公钥,推送端持私钥签名)
pub fn keygen() -> Result<(String, String), String> {
    let mut seed = [0u8; 32];
    rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut seed);
    let sk = SigningKey::from_bytes(&seed);
    Ok((hex::encode(seed), hex::encode(sk.verifying_key().as_bytes())))
}

/// StageManifest 的便捷重导出(GUI 显示文件集用)
pub type Manifest = StageManifest;
````

## File: crates/ghost-updater-cli/src/main.rs
````rust
//! ghost-push:ghost-updater 推送端命令行。
//!
//!   ghost-push push --target <host[:9600]> ... --dist <dir> [--main x.exe]
//!                   [--token t] [--sign-key <hex>] [--overwrite-config]
//!                   [--jobs N] [--timeout-secs 120]
//!   ghost-push keygen
//!
//! 多目标并发推送,每目标进度行实时滚动,全部成功退出码 0。

use ghost_updater_cli::{keygen, normalize_addr, probe, push_update, PushCtx, PushUpdateCfg};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = run(args);
    std::process::exit(code);
}

fn run(args: Vec<String>) -> i32 {
    match args.first().map(|s| s.as_str()) {
        Some("keygen") => match keygen() {
            Ok((seed, pubkey)) => {
                println!("sign-key(seed)  = {seed}");
                println!("pubkey(hex)     = {pubkey}");
                println!("服务端 pubkey_hex 配公钥;推送端 --sign-key 配私钥。");
                0
            }
            Err(e) => {
                eprintln!("keygen 失败: {e}");
                1
            }
        },
        Some("push") => tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(cmd_push(&args[1..])),
        _ => {
            print_usage();
            2
        }
    }
}

fn print_usage() {
    eprintln!(
        "用法:\n  ghost-push push --target <host[:9600]> [--target ...] [--targets-file <f>]\n                  --dist <dir> [--main x.exe] [--token <t>] [--sign-key <hex>]\n                  [--overwrite-config] [--jobs <N>] [--timeout-secs <s>]\n  ghost-push keygen"
    );
}

#[derive(Default)]
struct Opts {
    targets: Vec<String>,
    dist: Option<PathBuf>,
    main: Option<String>,
    token: Option<String>,
    sign_key: Option<String>,
    overwrite_config: bool,
    jobs: usize,
    timeout_secs: u64,
}

fn parse_opts(args: &[String]) -> Result<Opts, String> {
    let mut o = Opts { jobs: 4, timeout_secs: 120, ..Default::default() };
    let mut i = 0;
    while i < args.len() {
        let need = |i: usize| -> Result<String, String> {
            args.get(i + 1).cloned().ok_or_else(|| format!("{} 缺值", args[i]))
        };
        match args[i].as_str() {
            "--target" => o.targets.push(normalize_addr(&need(i)?)),
            "--targets-file" => {
                let f = need(i)?;
                for line in std::fs::read_to_string(&f).map_err(|e| format!("读 {f}: {e}"))?.lines() {
                    let l = line.trim();
                    if !l.is_empty() && !l.starts_with('#') {
                        o.targets.push(normalize_addr(l));
                    }
                }
            }
            "--dist" => o.dist = Some(PathBuf::from(need(i)?)),
            "--main" => o.main = Some(need(i)?),
            "--token" => o.token = Some(need(i)?),
            "--sign-key" => o.sign_key = Some(need(i)?),
            "--overwrite-config" => o.overwrite_config = true,
            "--jobs" => o.jobs = need(i)?.parse().map_err(|_| "--jobs 非法")?,
            "--timeout-secs" => o.timeout_secs = need(i)?.parse().map_err(|_| "--timeout-secs 非法")?,
            other => return Err(format!("未知参数: {other}")),
        }
        i += 2;
    }
    if o.dist.is_none() {
        return Err("缺少 --dist".into());
    }
    if o.targets.is_empty() {
        return Err("缺少 --target / --targets-file".into());
    }
    o.targets.sort();
    o.targets.dedup();
    Ok(o)
}

async fn cmd_push(args: &[String]) -> i32 {
    let opts = match parse_opts(args) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("参数错误: {e}\n");
            print_usage();
            return 2;
        }
    };
    let ctx = match PushCtx::new(opts.sign_key.as_deref()) {
        Ok(c) => Arc::new(c),
        Err(e) => {
            eprintln!("{e}");
            return 2;
        }
    };
    let dist = opts.dist.clone().unwrap();
    let (targets, jobs, token, main, overwrite_config, poll_timeout) = (
        opts.targets.clone(),
        opts.jobs.max(1),
        opts.token.clone(),
        opts.main.clone(),
        opts.overwrite_config,
        Duration::from_secs(opts.timeout_secs),
    );

    println!("== 目标 {} 台,并发 {jobs} ==", targets.len());
    for t in &targets {
        match probe(&ctx, t, token.as_deref()).await {
            Ok(p) => println!("  [在线] {t}  v{} ({} {})", p.version, p.os, p.arch),
            Err(e) => println!("  [离线] {t}  {e}"),
        }
    }

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<(String, String)>();
    let printer = tokio::spawn(async move {
        while let Some((addr, line)) = rx.recv().await {
            println!("[{addr}] {line}");
        }
    });

    let sem = Arc::new(tokio::sync::Semaphore::new(jobs));
    let mut handles = Vec::new();
    for target in targets {
        let permit = sem.clone().acquire_owned().await.unwrap();
        let ctx = ctx.clone();
        let tx = tx.clone();
        let dist = dist.clone();
        let (main, token, overwrite_config, poll_timeout) =
            (main.clone(), token.clone(), overwrite_config, poll_timeout);
        handles.push(tokio::spawn(async move {
            let (etx, mut erx) = tokio::sync::mpsc::channel::<String>(256);
            let fwd = {
                let tx = tx.clone();
                let addr = target.clone();
                tokio::spawn(async move {
                    while let Some(line) = erx.recv().await {
                        let _ = tx.send((addr.clone(), line));
                    }
                })
            };
            let cfg = PushUpdateCfg {
                addr: target.clone(),
                dist_dir: dist,
                main,
                token,
                sign_key_hex: None, // 签名在 PushCtx 统一持有
                overwrite_config,
                poll_timeout,
                on_event: Some(etx),
            };
            let report = push_update(&ctx, cfg).await;
            let _ = fwd.await;
            drop(permit);
            report
        }));
    }

    let mut results = Vec::new();
    for h in handles {
        if let Ok(r) = h.await {
            results.push(r);
        }
    }
    drop(tx);
    let _ = printer.await;

    println!("== 结果 ==");
    let mut failed = 0;
    for r in &results {
        if r.ok {
            println!("  [成功] {}  {} → {}", r.addr, r.old_version, r.new_version);
        } else {
            failed += 1;
            println!("  [失败] {}  {}  {}", r.addr, r.old_version, r.detail);
        }
    }
    if failed > 0 {
        println!("失败 {failed}/{};可用 --target 单独重试失败节点", results.len());
        1
    } else {
        println!("全部成功({})", results.len());
        0
    }
}
````

## File: crates/ghost-updater-cli/Cargo.toml
````toml
[package]
name = "ghost-updater-cli"
description = "ghost-updater 推送端:探测/暂存/触发/轮询编排,GUI 与 CLI 共用的库 + ghost-push 命令行"
version.workspace = true
edition.workspace = true
license.workspace = true

[[bin]]
name = "ghost-push"
path = "src/main.rs"

[dependencies]
ghost-updater-core = { path = "../ghost-updater-core" }
tokio = { workspace = true }
serde = { workspace = true }
serde_json = { workspace = true }
sha2 = { workspace = true }
hex = { workspace = true }
tar = { workspace = true }
ed25519-dalek = { workspace = true }
rand = { workspace = true }
reqwest = { workspace = true }
tokio-util = { workspace = true }
````

## File: crates/ghost-updater-core/src/lib.rs
````rust
//! ghost-updater-core:远程自替换核心(apply 侧)。
//!
//! 宿主 main 顶部一行 `ghost_updater_core::guard_apply_update();` 即完成接入:
//! 若本进程是被 `/update/apply` 从暂存区拉起的 apply worker(参数含
//! `--internal-apply-update`),它会在本函数内完成整个换版流程然后 exit;
//! 否则立即返回,走正常业务逻辑。
//!
//! apply worker 流程(全程阻塞 std,必须在 tokio runtime 建立前执行):
//!   1. 解析 --pid/--install/--staging(/--from/--close),读暂存区 manifest.json
//!   2. 等旧进程(pid)退出:轮询 process_alive,超时 taskkill /F 兜底
//!   2.5 等 --close 列出的关联进程退出(多进程宿主:agent 侧已优雅关闭,这里兜底等待/强杀)
//!   3. 备份安装目录将被覆盖的文件 → <install>/.update_backup/backup-<ts>/
//!   4. 用暂存文件覆盖安装目录(整体预校验后再动,保留未列入清单的配置文件;
//!      覆盖段中途失败当场按备份逐文件还原,Err 时 install 已回到旧状态)
//!   5. 分离拉起安装目录的新 exe(DETACHED|CREATE_NO_WINDOW,stdio 直弃,
//!      严禁管道捕获 —— 孙进程继承管道写句柄会导致等 EOF 永久卡死),1.5s
//!      存活判定为健康
//!   6. 失败回滚:还原备份、删本次新增文件、重拉旧 exe;apply.log/result.json 记录全程;
//!      所有失败路径(含备份/覆盖段)统一由 run_apply 收口释放 .apply-lock

use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// 本进程启动时刻(Unix 秒)。懒初始化一次,此后不变;
/// 推送端凭 started_at 变化确认目标已重启(网络空窗判定是竞态,不可靠)。
static STARTED_AT: AtomicU64 = AtomicU64::new(0);

pub fn started_at() -> u64 {
    let mut v = STARTED_AT.load(Ordering::Relaxed);
    if v == 0 {
        v = unix_secs();
        STARTED_AT.store(v, Ordering::Relaxed);
    }
    v
}

/// 更新数据目录名(位于安装目录下;备份/日志/结果都在这)
pub const UPDATE_DIR_NAME: &str = ".update_backup";
/// 锁文件名(写入暂存目录,create_new 原子防并发 apply)
pub const APPLY_LOCK_NAME: &str = ".apply-lock";
/// apply worker 的标记参数
pub const APPLY_FLAG: &str = "--internal-apply-update";

/// 暂存区清单(stage 侧由 ghost-updater-net 写入)
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct StageManifest {
    /// 扁平文件名(纯文件名,无路径分隔符)
    pub files: Vec<String>,
    /// 整包(tar)SHA256 hex
    pub sha256: String,
    /// 主可执行文件名(apply 后用它重启;必须 ∈ files)
    pub main: String,
    pub staged_at: u64,
}

/// 等待 pid 退出:轮询 process_alive,超时后 Windows 用 taskkill /F 兜底再等 1s。
/// 返回 true = 已确认退出。非 Windows 无 taskkill,超时即放弃(返回 false,放弃更新)。
pub fn wait_pid_exit(pid: u32, timeout: Duration) -> bool {
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        if !process_alive(pid) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("taskkill")
            .args(["/F", "/PID", &pid.to_string()])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
        std::thread::sleep(Duration::from_secs(1));
        !process_alive(pid)
    }
    #[cfg(not(windows))]
    {
        let _ = pid;
        false
    }
}

/// 进程是否存活。Windows: OpenProcess + GetExitCodeProcess == STILL_ACTIVE;
/// Unix: kill(pid, 0),EPERM 也算存活(存在但无权限发信号)。
pub fn process_alive(pid: u32) -> bool {
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE, STILL_ACTIVE};
        use windows_sys::Win32::System::Threading::{
            GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
        };
        // SAFETY:句柄 API,只查询不写入;h 用后即关
        unsafe {
            let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if h.is_null() || h == INVALID_HANDLE_VALUE {
                return false;
            }
            let mut code: u32 = 0;
            let ok = GetExitCodeProcess(h, &mut code);
            CloseHandle(h);
            ok != 0 && code == STILL_ACTIVE as u32
        }
    }
    #[cfg(unix)]
    {
        // SAFETY: kill(2) 是纯信号系统调用,只读 pid
        let r = unsafe { libc::kill(pid as i32, 0) };
        r == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }
}

/// 进程映像名白名单:只允许 ASCII 字母数字与 `. _ -`,杜绝 taskkill/tasklist 参数注入。
pub fn validate_process_names(names: &[String]) -> Result<(), String> {
    for n in names {
        if n.is_empty()
            || !n
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-')
        {
            return Err(format!("进程名只允许字母数字与 ._-: {n:?}"));
        }
    }
    Ok(())
}

/// 进程映像名是否在运行。Windows: tasklist CSV 首列精确比对(GBK 输出里文件名段
/// 是 ASCII,utf8_lossy 不损伤);非 Windows 恒 false(close/wait 均为 no-op)。
pub fn process_running_by_name(name: &str) -> bool {
    #[cfg(windows)]
    {
        let Ok(out) = std::process::Command::new("tasklist")
            .args(["/FI", &format!("IMAGENAME eq {name}"), "/FO", "CSV", "/NH"])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .output()
        else {
            return false;
        };
        String::from_utf8_lossy(&out.stdout).lines().any(|line| {
            let first = line.split(',').next().unwrap_or("").trim_matches('"');
            first.eq_ignore_ascii_case(name)
        })
    }
    #[cfg(not(windows))]
    {
        let _ = name;
        false
    }
}

fn taskkill_image(name: &str, force: bool) {
    let mut args = vec!["/IM".to_string(), name.to_string()];
    if force {
        args.push("/F".to_string());
    }
    let _ = std::process::Command::new("taskkill")
        .args(&args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
}

/// 优雅关闭一批进程(REQ-1):Windows 先 `taskkill /IM`(无 /F = WM_CLOSE),
/// grace 内轮询,残留再 `/F` 兜底。Err = 仍有进程关不掉(需人工介入)。
/// 非 Windows no-op(REQ:可先留空实现)。
pub fn close_processes_graceful(names: &[String], grace: Duration) -> Result<(), String> {
    validate_process_names(names)?;
    #[cfg(windows)]
    {
        let mut stubborn = Vec::new();
        for n in names {
            taskkill_image(n, false);
            let deadline = std::time::Instant::now() + grace;
            while process_running_by_name(n) && std::time::Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(300));
            }
            if process_running_by_name(n) {
                taskkill_image(n, true);
                std::thread::sleep(Duration::from_millis(500));
                if process_running_by_name(n) {
                    stubborn.push(n.clone());
                }
            }
        }
        if stubborn.is_empty() {
            Ok(())
        } else {
            Err(format!("以下进程未能关闭: {}", stubborn.join(", ")))
        }
    }
    #[cfg(not(windows))]
    {
        let _ = (names, grace);
        Ok(())
    }
}

/// 等待一批进程映像名全部退出:轮询,超时逐个 taskkill /F 兜底(镜像 wait_pid_exit 语义)。
/// 名单非法视为配置错误,返回 false(放弃更新)。非 Windows 恒 true(与 no-op close 配套)。
pub fn wait_processes_exit(names: &[String], timeout: Duration) -> bool {
    if validate_process_names(names).is_err() {
        return false;
    }
    #[cfg(windows)]
    {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            if names.iter().all(|n| !process_running_by_name(n)) {
                return true;
            }
            if std::time::Instant::now() >= deadline {
                break;
            }
            std::thread::sleep(Duration::from_millis(300));
        }
        for n in names {
            taskkill_image(n, true);
        }
        std::thread::sleep(Duration::from_secs(1));
        names.iter().all(|n| !process_running_by_name(n))
    }
    #[cfg(not(windows))]
    {
        let _ = (names, timeout);
        true
    }
}

/// main 顶部拦截:检测 `--internal-apply-update`,是 apply worker 就地完成换版并 exit,
/// 永不返回;否则直接 return,宿主继续正常启动。
pub fn guard_apply_update() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some((worker_args, relaunch_args)) = split_at_flag(&args) else { return };
    let code = match run_apply(&worker_args, &relaunch_args) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("ghost-updater: apply 失败: {e}");
            1
        }
    };
    std::process::exit(code);
}

/// argv 按 APPLY_FLAG 切成 (worker 参数, `--` 后的重启透传参数)。
pub fn split_at_flag(args: &[String]) -> Option<(Vec<String>, Vec<String>)> {
    let pos = args.iter().position(|a| a == APPLY_FLAG)?;
    let rest = &args[pos + 1..];
    match rest.iter().position(|a| a == "--") {
        Some(sep) => Some((rest[..sep].to_vec(), rest[sep + 1..].to_vec())),
        None => Some((rest.to_vec(), Vec::new())),
    }
}

struct ApplyArgs {
    pid: Option<u32>,
    install: PathBuf,
    staging: PathBuf,
    from_version: String,
    /// 需一并等待退出的关联进程映像名(net 侧 --close 传入;agent 已做过优雅关闭)
    close: Vec<String>,
}

fn parse_apply_args(argv: &[String]) -> Result<ApplyArgs, String> {
    let mut pid = None;
    let mut install = None;
    let mut staging = None;
    let mut from = String::new();
    let mut close = Vec::new();
    let mut i = 0;
    while i < argv.len() {
        let val = |i: usize| argv.get(i + 1).ok_or_else(|| format!("{} 缺值", argv[i]));
        match argv[i].as_str() {
            "--pid" => pid = Some(val(i)?.parse().map_err(|_| "--pid 非法")?),
            "--install" => install = Some(PathBuf::from(val(i)?)),
            "--staging" => staging = Some(PathBuf::from(val(i)?)),
            "--from" => from = val(i)?.clone(),
            "--close" => {
                close = val(i)?
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
                validate_process_names(&close)?;
            }
            other => return Err(format!("未知参数: {other}")),
        }
        i += 2;
    }
    Ok(ApplyArgs {
        pid,
        install: install.ok_or("缺少 --install")?,
        staging: staging.ok_or("缺少 --staging")?,
        from_version: from,
        close,
    })
}

fn run_apply(argv: &[String], relaunch_args: &[String]) -> Result<(), String> {
    let a = parse_apply_args(argv)?;
    let r = run_apply_inner(&a, relaunch_args);
    // 锁收口:成功/任何失败路径统一在这里释放,失败不再泄漏 .apply-lock(REQ-2)
    release_lock(&a.staging);
    r
}

fn run_apply_inner(a: &ApplyArgs, relaunch_args: &[String]) -> Result<(), String> {
    let update_dir = a.install.join(UPDATE_DIR_NAME);
    std::fs::create_dir_all(&update_dir)
        .map_err(|e| format!("创建 {} 失败: {e}", update_dir.display()))?;
    let log_path = update_dir.join("apply.log");

    let manifest: StageManifest = serde_json::from_str(
        &std::fs::read_to_string(a.staging.join("manifest.json"))
            .map_err(|e| format!("读 manifest.json 失败: {e}"))?,
    )
    .map_err(|e| format!("manifest.json 解析失败: {e}"))?;
    validate_file_names(&manifest.files)?;
    if !manifest.files.iter().any(|f| *f == manifest.main) {
        return Err(format!("main exe({}) 不在文件集内", manifest.main));
    }

    log_line(
        &log_path,
        &format!(
            "apply 开始: pid={:?} install={} staging={} files={} from={}",
            a.pid,
            a.install.display(),
            a.staging.display(),
            manifest.files.len(),
            a.from_version,
        ),
    );

    // 1. 等旧进程退出(服务端 ack 后 ~0.8s 自退;这里兜底强杀)
    if let Some(pid) = a.pid {
        if !wait_pid_exit(pid, Duration::from_secs(15)) {
            let msg = format!("旧进程(pid={pid}) 15s 内未退出,放弃更新(文件未动)");
            log_line(&log_path, &msg);
            write_result(&update_dir, false, &a.from_version, &manifest.files, &msg);
            return Err(msg);
        }
    }
    std::thread::sleep(Duration::from_millis(300)); // 句柄释放缓冲

    // 1.5 等关联进程退出(REQ-1 多进程宿主:agent 侧已优雅关闭,这里兜底等待 + taskkill /F)
    if !a.close.is_empty() {
        if !wait_processes_exit(&a.close, Duration::from_secs(10)) {
            let msg = format!(
                "关联进程({}) 10s 内未退出,放弃更新(文件未动)",
                a.close.join(",")
            );
            log_line(&log_path, &msg);
            write_result(&update_dir, false, &a.from_version, &manifest.files, &msg);
            return Err(msg);
        }
    }

    // 2+3. 备份 + 覆盖(分段:备份段失败文件未动;覆盖段失败由 overwrite_files 当场还原,
    //      只还原已覆盖的文件 —— 不碰未覆盖的旧文件,避免 remove 旧文件被外部句柄锁卡死)
    let backup_dir = update_dir.join(format!("backup-{}", unix_secs()));
    if let Err(e) = ensure_staged(&a.staging, &manifest.files)
        .and_then(|_| backup_files(&a.install, &manifest.files, &backup_dir))
    {
        let msg = format!("暂存/备份失败: {e}(文件未动)");
        log_line(&log_path, &msg);
        write_result(&update_dir, false, &a.from_version, &manifest.files, &msg);
        return Err(msg);
    }
    if let Err(e) = overwrite_files(&a.install, &a.staging, &manifest.files, &backup_dir) {
        log_line(&log_path, &format!("文件替换失败: {e};重拉旧版恢复服务"));
        if !manifest.main.is_empty() {
            let _ = start_detached(&a.install.join(&manifest.main), &a.install, &[], None);
        }
        let msg = format!("文件替换失败: {e}");
        write_result(&update_dir, false, &a.from_version, &manifest.files, &msg);
        return Err(msg);
    }
    log_line(
        &log_path,
        &format!(
            "文件替换完成({} 个),备份: {}",
            manifest.files.len(),
            backup_dir.display()
        ),
    );

    // 4. 用安装目录的新 exe 重启(绝不能用暂存副本 —— 暂存区随时可能被清理)
    let new_exe = a.install.join(&manifest.main);
    let started = start_detached(&new_exe, &a.install, relaunch_args, None);
    match started {
        Ok(mut child) => {
            // 健康检查:1.5s 后仍存活才算成功(秒退 = 启动失败 → 回滚)
            std::thread::sleep(Duration::from_millis(1500));
            if child_still_alive(&mut child) {
                let _ = std::fs::remove_dir_all(&backup_dir); // 成功清备份
                log_line(&log_path, &format!("更新成功,新进程 pid={} 存活", child.id()));
                write_result(&update_dir, true, &a.from_version, &manifest.files, "");
                Ok(())
            } else {
                let msg = "新进程 1.5s 内退出(健康检查失败)".to_string();
                log_line(&log_path, &format!("{msg};回滚备份并重试启动旧版"));
                rollback(&a.install, &backup_dir, &manifest.files, &manifest.main, &log_path);
                write_result(&update_dir, false, &a.from_version, &manifest.files, &msg);
                Err(msg)
            }
        }
        Err(e) => {
            log_line(&log_path, &format!("拉起新版失败: {e};回滚备份并重试启动旧版"));
            rollback(&a.install, &backup_dir, &manifest.files, &manifest.main, &log_path);
            let msg = format!("拉起新版失败: {e}(已回滚)");
            write_result(&update_dir, false, &a.from_version, &manifest.files, &msg);
            Err(msg)
        }
    }
}

fn rollback(install: &Path, backup_dir: &Path, files: &[String], main: &str, log_path: &Path) {
    match restore_backup(install, backup_dir, files) {
        Ok(()) => log_line(log_path, "回滚完成"),
        Err(e) => log_line(
            log_path,
            &format!("回滚失败(需人工介入,备份在 {}): {e}", backup_dir.display()),
        ),
    }
    if !main.is_empty() {
        let _ = start_detached(&install.join(main), install, &[], None);
    }
}

/// 暂存区预检:文件名扁平 + 源文件齐全。失败 = 安装目录保证未动。
fn ensure_staged(staging: &Path, files: &[String]) -> Result<(), String> {
    validate_file_names(files)?;
    for f in files {
        if !staging.join(f).is_file() {
            return Err(format!("暂存区缺少文件: {}", staging.join(f).display()));
        }
    }
    Ok(())
}

/// 备份安装目录现存文件(不覆盖;失败时 install 未动过)。
pub fn backup_files(install: &Path, files: &[String], backup_dir: &Path) -> Result<(), String> {
    validate_file_names(files)?;
    std::fs::create_dir_all(backup_dir).map_err(|e| format!("创建备份目录失败: {e}"))?;
    for f in files {
        let dst = install.join(f);
        if dst.is_file() {
            std::fs::copy(&dst, backup_dir.join(f)).map_err(|e| format!("备份 {f} 失败: {e}"))?;
        }
    }
    Ok(())
}

/// 用暂存文件覆盖安装目录(调用前必须已 backup_files)。
/// 覆盖段中途失败:已覆盖的文件当场按备份逐个还原、本次新增(备份里没有的)删除,
/// 返回 Err 时 install 已回到旧状态 —— 只动确实覆盖过的文件,未覆盖的旧文件
/// (可能仍被外部句柄锁着)绝不 remove,所以还原不会被锁卡死。
pub fn overwrite_files(
    install: &Path,
    staging: &Path,
    files: &[String],
    backup_dir: &Path,
) -> Result<(), String> {
    validate_file_names(files)?;
    let mut done: Vec<&String> = Vec::new();
    for f in files {
        match std::fs::copy(staging.join(f), install.join(f)) {
            Ok(_) => done.push(f),
            Err(e) => {
                let mut bad: Vec<String> = Vec::new();
                for f2 in done {
                    let bak = backup_dir.join(f2);
                    let r = if bak.is_file() {
                        std::fs::copy(&bak, install.join(f2)).map(|_| ())
                    } else {
                        std::fs::remove_file(install.join(f2)) // 本次新增的,删掉
                    };
                    if let Err(e2) = r {
                        bad.push(format!("还原 {f2}: {e2}"));
                    }
                }
                return Err(if bad.is_empty() {
                    format!("覆盖 {f} 失败: {e}(已回滚到旧状态)")
                } else {
                    format!("覆盖 {f} 失败: {e}(回滚失败,需人工介入: {})", bad.join("; "))
                });
            }
        }
    }
    Ok(())
}

/// 备份 + 覆盖:先整体校验暂存源齐全且文件名扁平,再备份安装目录现存文件,最后覆盖。
/// 返回后 install = 暂存内容;未列入 files 的既有文件(如 config.json)不动。
/// 覆盖段失败的还原语义见 overwrite_files(Err 时 install 已回旧状态)。
pub fn swap_files(install: &Path, staging: &Path, files: &[String], backup_dir: &Path) -> Result<(), String> {
    ensure_staged(staging, files)?;
    backup_files(install, files, backup_dir)?;
    overwrite_files(install, staging, files, backup_dir)
}

/// 回滚:备份里有的恢复原样;备份里没有的(本次新增)删除。
pub fn restore_backup(install: &Path, backup_dir: &Path, files: &[String]) -> Result<(), String> {
    validate_file_names(files)?;
    for f in files {
        let dst = install.join(f);
        let src = backup_dir.join(f);
        if src.is_file() {
            if dst.is_file() {
                std::fs::remove_file(&dst).map_err(|e| format!("删新文件 {f} 失败: {e}"))?;
            }
            std::fs::copy(&src, &dst).map_err(|e| format!("还原 {f} 失败: {e}"))?;
        } else if dst.is_file() {
            std::fs::remove_file(&dst).map_err(|e| format!("删新增 {f} 失败: {e}"))?;
        }
    }
    Ok(())
}

/// 文件集安全校验:纯文件名,拒绝路径分隔符与 ..(文件必须直接落在安装目录)
pub fn validate_file_names(files: &[String]) -> Result<(), String> {
    for f in files {
        if f.is_empty()
            || f.contains('/')
            || f.contains('\\')
            || f.contains("..")
            || f.contains(':')
            || Path::new(f).is_absolute()
        {
            return Err(format!("文件名不允许包含路径成分: {f}"));
        }
    }
    Ok(())
}

/// 分离拉起 exe(cwd=workdir):DETACHED_PROCESS | CREATE_NO_WINDOW。
/// log 为 Some 时 stdout/stderr 追加到该文件(apply worker 无控制台,崩溃线索全靠它),
/// 否则 stdio 全弃。绝不能用 output()/status() 等管道捕获 —— 若新进程又派生
/// 继承句柄的孙进程,父进程永远等不到 EOF,apply 卡死。
pub fn start_detached(
    exe: &Path,
    workdir: &Path,
    args: &[String],
    log: Option<&Path>,
) -> Result<std::process::Child, String> {
    let mut cmd = std::process::Command::new(exe);
    cmd.args(args).current_dir(workdir).stdin(std::process::Stdio::null());
    match log {
        Some(p) => {
            if let Some(parent) = p.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let f = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(p)
                .map_err(|e| format!("开 worker 日志 {} 失败: {e}", p.display()))?;
            cmd.stdout(std::process::Stdio::from(f.try_clone().map_err(|e| e.to_string())?))
                .stderr(std::process::Stdio::from(f));
        }
        None => {
            cmd.stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // DETACHED_PROCESS(0x8) | CREATE_NO_WINDOW(0x0800_0000)
        cmd.creation_flags(0x0000_0008 | 0x0800_0000);
    }
    cmd.spawn().map_err(|e| format!("spawn {} 失败: {e}", exe.display()))
}

fn child_still_alive(child: &mut std::process::Child) -> bool {
    matches!(child.try_wait(), Ok(None))
}

/// apply 并发锁:create_new 原子建 `.apply-lock`;已存在 = 有 apply 在进行。
pub fn acquire_apply_lock(staging: &Path) -> Result<(), String> {
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(staging.join(APPLY_LOCK_NAME))
        .map(|_| ())
        .map_err(|e| format!("暂存区已被锁定(或锁文件创建失败): {e}"))
}

pub fn release_lock(staging: &Path) {
    let _ = std::fs::remove_file(staging.join(APPLY_LOCK_NAME));
}

fn log_line(path: &Path, msg: &str) {
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "[{}] {}", unix_secs(), msg);
    }
}

/// 宿主侧(agent)向 apply.log 追记一行(如 close_processes 关闭动作),与 worker 日志同文件。
pub fn append_apply_log(install: &Path, msg: &str) {
    let dir = install.join(UPDATE_DIR_NAME);
    let _ = std::fs::create_dir_all(&dir);
    log_line(&dir.join("apply.log"), msg);
}

fn write_result(update_dir: &Path, ok: bool, from: &str, files: &[String], error: &str) {
    let body = serde_json::json!({
        "ok": ok,
        "from": from,
        "time": unix_secs(),
        "files": files,
        "error": error,
    });
    let _ = std::fs::write(update_dir.join("result.json"), body.to_string());
}

pub fn unix_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir()
            .join(format!("ghost-core-{}-{tag}-{}", std::process::id(), unix_secs()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn put(dir: &Path, name: &str, content: &[u8]) {
        std::fs::write(dir.join(name), content).unwrap();
    }

    fn get(dir: &Path, name: &str) -> Vec<u8> {
        std::fs::read(dir.join(name)).unwrap()
    }

    #[test]
    fn swap_replaces_and_keeps_backup() {
        let install = temp_dir("swap-install");
        let staging = temp_dir("swap-staging");
        let backup = temp_dir("swap-backup");
        put(&install, "app.exe", b"old");
        put(&install, "app.dll", b"old-dll");
        put(&install, "config.json", b"keep-me");
        put(&staging, "app.exe", b"new");
        put(&staging, "app.dll", b"new-dll");
        put(&staging, "app.h", b"new-header");
        let files = ["app.exe".to_string(), "app.dll".to_string(), "app.h".to_string()];

        swap_files(&install, &staging, &files, &backup).unwrap();

        assert_eq!(get(&install, "app.exe"), b"new");
        assert_eq!(get(&install, "app.dll"), b"new-dll");
        assert_eq!(get(&install, "app.h"), b"new-header");
        assert_eq!(get(&install, "config.json"), b"keep-me"); // 配置保留
        assert_eq!(get(&backup, "app.exe"), b"old");
        assert_eq!(get(&backup, "app.dll"), b"old-dll");
        assert!(!backup.join("app.h").exists()); // 备份只含原有文件
    }

    #[test]
    fn swap_fails_cleanly_when_staging_incomplete() {
        let install = temp_dir("swapf-install");
        let staging = temp_dir("swapf-staging");
        let backup = temp_dir("swapf-backup");
        put(&install, "app.exe", b"old");
        put(&staging, "app.exe", b"new");
        let files = ["app.exe".to_string(), "app.dll".to_string()];
        // app.dll 缺失 → 整体拒绝,安装目录不动
        assert!(swap_files(&install, &staging, &files, &backup).is_err());
        assert_eq!(get(&install, "app.exe"), b"old");
        assert!(!install.join("app.dll").exists());
        assert!(!backup.join("app.exe").exists()); // 备份未落盘
    }

    #[test]
    fn swap_rejects_path_traversal() {
        let install = temp_dir("trav-install");
        let staging = temp_dir("trav-staging");
        let backup = temp_dir("trav-backup");
        put(&staging, "evil", b"x");
        assert!(swap_files(&install, &staging, &["..\\evil".to_string()], &backup).is_err());
        assert!(swap_files(&install, &staging, &["sub/evil".to_string()], &backup).is_err());
        assert!(swap_files(&install, &staging, &["C:\\evil".to_string()], &backup).is_err());
    }

    #[test]
    fn restore_recovers_old_and_removes_new() {
        let install = temp_dir("rest-install");
        let staging = temp_dir("rest-staging");
        let backup = temp_dir("rest-backup");
        put(&install, "app.exe", b"old");
        put(&staging, "app.exe", b"new");
        put(&staging, "extra.dll", b"new-extra");
        let files = vec!["app.exe".to_string(), "extra.dll".to_string()];
        swap_files(&install, &staging, &files, &backup).unwrap();
        assert_eq!(get(&install, "app.exe"), b"new");
        assert!(install.join("extra.dll").exists());

        restore_backup(&install, &backup, &files).unwrap();
        assert_eq!(get(&install, "app.exe"), b"old");
        assert!(!install.join("extra.dll").exists());
    }

    #[test]
    fn wait_pid_exit_confirms_short_lived_child() {
        let pid = dummy_child(1).id();
        assert!(wait_pid_exit(pid, Duration::from_secs(5)));
        assert!(!process_alive(pid));
    }

    #[cfg(windows)]
    #[test]
    fn taskkill_fallback_kills_stubborn_child() {
        let pid = dummy_child(30).id();
        std::thread::sleep(Duration::from_millis(300));
        assert!(process_alive(pid));
        // timeout=0:轮询直接超时 → taskkill /F 兜底强杀
        assert!(wait_pid_exit(pid, Duration::ZERO));
        assert!(!process_alive(pid));
    }

    #[cfg(not(windows))]
    #[test]
    fn wait_pid_exit_zero_timeout_gives_up() {
        assert!(!wait_pid_exit(u32::MAX - 1, Duration::ZERO));
    }

    #[test]
    fn split_at_flag_parses_worker_and_relaunch_args() {
        let args: Vec<String> = [
            "--pid",
            "42",
            APPLY_FLAG,
            "--install",
            "D:\\app",
            "--staging",
            "S",
            "--",
            "--serve",
            ":80",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let (w, r) = split_at_flag(&args).unwrap();
        assert!(w.contains(&"--install".to_string()));
        assert_eq!(r, vec!["--serve".to_string(), ":80".to_string()]);
        assert!(split_at_flag(&["--foo".to_string()]).is_none());
    }

    #[test]
    fn parse_close_processes_arg() {
        let args: Vec<String> = ["--install", "I", "--staging", "S", "--close", "gui.exe, helper.exe ,"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let a = parse_apply_args(&args).unwrap();
        assert_eq!(a.close, vec!["gui.exe".to_string(), "helper.exe".to_string()]);
        assert!(parse_apply_args(&["--close".to_string(), "a b.exe".to_string()]).is_err());
    }

    #[test]
    fn process_names_whitelist_rejects_injection() {
        validate_process_names(&["demo-gui.exe".into(), "My_App-2.exe".into()]).unwrap();
        for bad in ["a b.exe", "a\"b", "a/b", "a\\b", "a;b", "", "a|b"] {
            assert!(validate_process_names(&[bad.to_string()]).is_err(), "{bad}");
        }
    }

    #[cfg(windows)]
    #[test]
    fn process_running_by_name_detects_cmd_child() {
        let _c = dummy_child(5);
        assert!(process_running_by_name("cmd.exe"));
        assert!(!process_running_by_name("ghost-updater-no-such-proc.exe"));
    }

    #[cfg(windows)]
    #[test]
    fn wait_processes_exit_fast_for_absent_name() {
        let t = std::time::Instant::now();
        assert!(wait_processes_exit(&["ghost-updater-no-such-proc.exe".into()], Duration::from_secs(5)));
        assert!(t.elapsed() < Duration::from_secs(2), "不存在名不应等到超时");
    }

    #[cfg(windows)]
    #[test]
    fn close_processes_graceful_ok_for_absent_name() {
        assert!(close_processes_graceful(
            &["ghost-updater-no-such-proc.exe".into()],
            Duration::from_millis(500)
        )
        .is_ok());
    }

    /// REQ-2 验收:锁住安装目录文件逼停覆盖段 → Err 含"已回滚"、install 回旧状态、
    /// 暂存区无 .apply-lock、result.json ok:false。share_mode(1)=FILE_SHARE_READ:
    /// 备份可读、覆盖写必败(os error 32),正好落在覆盖段失败分支。
    #[cfg(windows)]
    #[test]
    fn swap_failure_rolls_back_and_releases_lock() {
        use std::os::windows::fs::OpenOptionsExt;
        let install = temp_dir("roll-install");
        let staging = temp_dir("roll-staging");
        put(&install, "app.txt", b"old-app");
        put(&install, "locked.bin", b"old-locked");
        put(&staging, "app.txt", b"new-app");
        put(&staging, "locked.bin", b"new-locked");
        std::fs::write(
            staging.join("manifest.json"),
            serde_json::to_string(&StageManifest {
                files: vec!["app.txt".into(), "locked.bin".into()],
                sha256: "0".into(),
                main: "app.txt".into(), // 非 exe,回滚重拉会静默失败,单测无副作用
                staged_at: 0,
            })
            .unwrap(),
        )
        .unwrap();
        acquire_apply_lock(&staging).unwrap();

        let _guard = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(1) // FILE_SHARE_READ:他人可读不可写
            .open(install.join("locked.bin"))
            .unwrap();

        let argv: Vec<String> = [
            "--install",
            install.to_str().unwrap(),
            "--staging",
            staging.to_str().unwrap(),
            "--from",
            "1.0.0",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let err = run_apply(&argv, &[]).unwrap_err();
        assert!(err.contains("已回滚"), "err={err}");
        assert_eq!(get(&install, "app.txt"), b"old-app"); // 已覆盖的被还原
        assert_eq!(get(&install, "locked.bin"), b"old-locked"); // 未覆盖的原样
        assert!(!staging.join(APPLY_LOCK_NAME).exists()); // 锁已释放,可重推
        let result = std::fs::read_to_string(install.join(UPDATE_DIR_NAME).join("result.json")).unwrap();
        assert!(result.contains("\"ok\":false") && result.contains("已回滚"), "result={result}");
    }

    fn dummy_child(sleep_secs: u32) -> std::process::Child {
        #[cfg(windows)]
        let mut cmd = {
            let mut c = std::process::Command::new("cmd");
            c.args(["/C", "ping", "-n", &sleep_secs.to_string(), "127.0.0.1"]);
            c
        };
        #[cfg(not(windows))]
        let mut cmd = {
            let mut c = std::process::Command::new("sleep");
            c.arg(sleep_secs.to_string());
            c
        };
        cmd.stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
        cmd.spawn().unwrap()
    }
}
````

## File: crates/ghost-updater-core/Cargo.toml
````toml
[package]
name = "ghost-updater-core"
description = "ghost-updater 核心运行时:幽灵自替换、进程接力、跨平台,零网络依赖"
version.workspace = true
edition.workspace = true
license.workspace = true

[dependencies]
serde = { workspace = true }
serde_json = { workspace = true }

[target.'cfg(windows)'.dependencies]
windows-sys = { workspace = true }

[target.'cfg(unix)'.dependencies]
libc = "0.2"
````

## File: crates/ghost-updater-net/src/lib.rs
````rust
//! ghost-updater-net:标准三段式更新 API,可嵌入任意 Axum 服务,也可独立起 agent。
//!
//!   GET  /update/probe  → 当前 version / started_at / os / arch / 公钥指纹
//!   POST /update/stage  → 更新包(tar)流式写入暂存目录,SHA256 整包校验
//!   POST /update/apply  → 200 确认后分离拉起暂存区新 exe 执行自替换,本进程随后退出
//!
//! 安全部(design:强制二选一):
//!   * 配了 auth_token → stage/apply 必须带 `Authorization: Bearer <token>`
//!   * 没配 token 但配了 pubkey_hex → stage 必须带有效 X-Signature(ed25519 对整包
//!     SHA256 摘要签名),apply 放行(只能触发已验签暂存的内容)
//!   * 两者都没配 → stage/apply 直接 503,拒绝裸奔;probe 永远公开
//!
//! 一键集成:
//!   Router::new().merge(ghost_updater_net::routes(UpdateConfig { .. }))
//! 或独立监听:ghost_updater_net::start_agent("0.0.0.0:9001", Some("token")).await;

use axum::body::Body;
use axum::extract::{DefaultBodyLimit, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use ghost_updater_core::{acquire_apply_lock, started_at, unix_secs, StageManifest, APPLY_FLAG};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::io::Write;

/// 暂存根目录名(cwd 相对;所有 stage-*/ 都在这下面)
pub const STAGING_ROOT: &str = ".ghost-update";
/// 单包体积上限(整包 tar 流式上传;默认 2 GiB 覆盖绝大多数安装包)
pub const MAX_PACKAGE_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// 进程内 stage 目录序号(时间戳 + 序号,防同秒并发撞名)
static STAGE_SEQ: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Default)]
pub struct UpdateConfig {
    /// 宿主当前版本号(probe 返回;宿主自己最清楚,如 CARGO_PKG_VERSION)
    pub version: String,
    /// 预共享 Token;Some 时 stage/apply 强制 Bearer 鉴权
    pub auth_token: Option<String>,
    /// ed25519 公钥 hex(64 字符);Some 时 stage 强制验签(token 未配时的另一条安全线)
    pub pubkey_hex: Option<String>,
    /// 安装目录(apply 覆盖目标;默认 current_exe 所在目录)
    pub install_dir: Option<PathBuf>,
    /// 暂存根目录(默认 cwd 相对 .ghost-update/)
    pub staging_root: Option<PathBuf>,
    /// apply 受理后需优雅关闭的本机进程映像名(REQ-1 多进程宿主编排;空 = 现状零改动)。
    /// agent 关闭 → 自退;worker 会再兜底等待名单内进程全部退出才动文件。
    pub close_processes: Vec<String>,
}

impl UpdateConfig {
    pub fn new(version: impl Into<String>) -> Self {
        Self {
            version: version.into(),
            auth_token: None,
            pubkey_hex: None,
            install_dir: None,
            staging_root: None,
            close_processes: Vec::new(),
        }
    }
}

#[derive(Clone)]
pub struct UpdateState {
    cfg: UpdateConfig,
    staging_root: PathBuf,
    pubkey: Option<VerifyingKey>,
    fingerprint: String,
}

/// 挂载到宿主 Router:`Router::new().merge(ghost_updater_net::routes(cfg))`
/// 公钥 hex 非法会在启动时直接 panic(配置错误越早暴露越好)。
pub fn routes(cfg: UpdateConfig) -> Router {
    let (pubkey, fingerprint) = match &cfg.pubkey_hex {
        Some(h) => {
            let bytes = decode_pubkey(h).unwrap_or_else(|e| panic!("pubkey_hex 非法: {e}"));
            let vk = VerifyingKey::from_bytes(&bytes).expect("pubkey_hex 非法: 不是有效 ed25519 公钥");
            let fp = {
                let d = Sha256::digest(bytes);
                hex::encode(&d[..8])
            };
            (Some(vk), fp)
        }
        None => (None, String::new()),
    };
    let staging_root = cfg.staging_root.clone().unwrap_or_else(|| PathBuf::from(STAGING_ROOT));
    if let Err(e) = ghost_updater_core::validate_process_names(&cfg.close_processes) {
        panic!("close_processes 非法: {e}");
    }
    Router::new()
        .route("/update/probe", get(probe))
        .route("/update/stage", post(stage))
        .route("/update/apply", post(apply))
        .layer(DefaultBodyLimit::max(MAX_PACKAGE_BYTES as usize))
        .with_state(UpdateState { cfg, staging_root, pubkey, fingerprint })
}

/// 独立后台 agent(纯桌面/本地工具场景,design:1 个端口控制面)
pub async fn start_agent(addr: &str, version: &str, auth_token: Option<String>) -> std::io::Result<()> {
    let app = routes(UpdateConfig { auth_token, ..UpdateConfig::new(version) });
    let listener = tokio::net::TcpListener::bind(addr).await?;
    eprintln!("[ghost-updater] agent 监听 {addr}");
    axum::serve(listener, app).await
}

// ---------- probe ----------

async fn probe(State(st): State<UpdateState>) -> Response {
    Json(json!({
        "version": st.cfg.version,
        "started_at": started_at(),
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "pubkey_fingerprint": st.fingerprint,
    }))
    .into_response()
}

// ---------- stage ----------

#[derive(Deserialize)]
struct StageQuery {
    /// 整包(tar)SHA256 hex,必填,防半包/坏包
    sha256: String,
    /// 主可执行文件名,必填,apply 用它重启
    main: String,
}

#[derive(Serialize)]
struct StageAck {
    ok: bool,
    staging: String,
    files: Vec<String>,
}

async fn stage(
    State(st): State<UpdateState>,
    Query(q): Query<StageQuery>,
    headers: HeaderMap,
    body: Body,
) -> Response {
    if let Err(r) = check_auth(&st, &headers) {
        drain_body(body).await;
        return r;
    }
    if q.sha256.len() != 64 || hex::decode(&q.sha256).is_err() {
        drain_body(body).await;
        return err(StatusCode::BAD_REQUEST, "sha256 必须是 64 位 hex");
    }
    if let Err(e) = ghost_updater_core::validate_file_names(std::slice::from_ref(&q.main)) {
        drain_body(body).await;
        return err(StatusCode::BAD_REQUEST, &format!("main 非法: {e}"));
    }

    // 流式落盘:边收边算哈希,中断只会留下垃圾暂存目录,不影响运行中的主进程
    // 必须绝对路径:apply worker 的 cwd 就是暂存目录,相对路径会在 worker 里自我套娃
    let staging = match std::path::absolute(st.staging_root.join(format!(
        "stage-{}-{}",
        unix_secs(),
        STAGE_SEQ.fetch_add(1, Ordering::Relaxed)
    ))) {
        Ok(p) => p,
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, &format!("解析暂存路径失败: {e}")),
    };
    if let Err(e) = std::fs::create_dir_all(&staging) {
        return err(StatusCode::INTERNAL_SERVER_ERROR, &format!("创建暂存目录失败: {e}"));
    }

    let pkg_path = staging.join("package.tar");
    let mut hasher = Sha256::new();
    if let Err(e) = stream_to_file(body, &pkg_path, &mut hasher).await {
        let _ = std::fs::remove_dir_all(&staging);
        return err(StatusCode::BAD_REQUEST, &format!("接收流失败: {e}"));
    }
    let digest = hex::encode(hasher.finalize());
    if !digest.eq_ignore_ascii_case(&q.sha256) {
        let _ = std::fs::remove_dir_all(&staging);
        return err(
            StatusCode::BAD_REQUEST,
            &format!("SHA256 不符(收到 {digest},声明 {}):疑似断传/坏包,暂存已清除", q.sha256),
        );
    }

    // 可选强制验签:对 32 字节摘要签名
    if let Some(vk) = &st.pubkey {
        let ok = headers
            .get("x-signature")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| hex::decode(s).ok())
            .and_then(|b| Signature::from_slice(&b).ok())
            .map(|sig| vk.verify(digest.as_bytes(), &sig).is_ok())
            .unwrap_or(false);
        if !ok {
            let _ = std::fs::remove_dir_all(&staging);
            return err(StatusCode::UNAUTHORIZED, "缺少有效 X-Signature(此节点已启用验签)");
        }
    }

    // 解包 tar(扁平文件集,拒绝目录穿越/非普通文件)
    let files = match unpack_flat(&pkg_path, &staging) {
        Ok(f) => f,
        Err(e) => {
            let _ = std::fs::remove_dir_all(&staging);
            return err(StatusCode::BAD_REQUEST, &format!("解包失败: {e}"));
        }
    };
    let _ = std::fs::remove_file(&pkg_path);
    if files.is_empty() {
        let _ = std::fs::remove_dir_all(&staging);
        return err(StatusCode::BAD_REQUEST, "包内没有文件");
    }
    if !files.iter().any(|f| *f == q.main) {
        let _ = std::fs::remove_dir_all(&staging);
        return err(StatusCode::BAD_REQUEST, &format!("包内缺少主程序 {main}", main = q.main));
    }

    let manifest = StageManifest {
        files: files.clone(),
        sha256: digest.to_ascii_lowercase(),
        main: q.main,
        staged_at: unix_secs(),
    };
    if let Err(e) = std::fs::write(
        staging.join("manifest.json"),
        serde_json::to_string(&manifest).unwrap_or_default(),
    ) {
        let _ = std::fs::remove_dir_all(&staging);
        return err(StatusCode::INTERNAL_SERVER_ERROR, &format!("写 manifest 失败: {e}"));
    }

    Json(StageAck { ok: true, staging: staging_dir_name(&staging), files }).into_response()
}

async fn stream_to_file(body: Body, path: &Path, hasher: &mut Sha256) -> Result<u64, String> {
    use http_body_util::BodyExt;
    let mut f = std::fs::File::create(path).map_err(|e| format!("建包文件失败: {e}"))?;
    let mut total: u64 = 0;
    let mut body = body;
    while let Some(frame) = body.frame().await {
        let bytes = frame.map_err(|e| format!("读 body: {e}"))?.into_data().map_err(|_| "body 帧非数据")?;
        hasher.update(&bytes);
        f.write_all(&bytes).map_err(|e| format!("写暂存: {e}"))?;
        total += bytes.len() as u64;
        if total > MAX_PACKAGE_BYTES {
            return Err(format!("超过单包上限 {MAX_PACKAGE_BYTES} 字节"));
        }
    }
    f.flush().map_err(|e| format!("flush 暂存: {e}"))?;
    Ok(total)
}

/// 把未消费的请求体读完再响应(REQ-3):axum/hyper 对未读 body 的请求发完响应即断连,
/// 客户端若仍在流式上传大包体,看到的是连接重置而非 401/4xx,错误表现随机、E2E 无法断言。
/// 上限 MAX_PACKAGE_BYTES(超限放弃、断连可接受),整体 30s 超时不伺候慢速攻击。
async fn drain_body(body: Body) {
    use http_body_util::BodyExt;
    let _ = tokio::time::timeout(std::time::Duration::from_secs(30), async move {
        let mut body = body;
        let mut total: u64 = 0;
        while let Some(frame) = body.frame().await {
            match frame {
                Ok(f) => {
                    if let Some(d) = f.data_ref() {
                        total += d.len() as u64;
                    }
                    if total > MAX_PACKAGE_BYTES {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    })
    .await;
}

/// 解包 tar 到 dir,只接受扁平普通文件;返回文件名列表。
fn unpack_flat(pkg: &Path, dir: &Path) -> Result<Vec<String>, String> {
    let mut ar = tar::Archive::new(std::fs::File::open(pkg).map_err(|e| format!("开包: {e}"))?);
    ar.set_preserve_permissions(false);
    ar.set_unpack_xattrs(false);
    let mut files = Vec::new();
    for entry in ar.entries().map_err(|e| format!("遍历 tar: {e}"))? {
        let mut e = entry.map_err(|e| format!("读 tar 条目: {e}"))?;
        let name = e.path().map_err(|e| format!("条目路径: {e}"))?.display().to_string();
        ghost_updater_core::validate_file_names(std::slice::from_ref(&name))
            .map_err(|_| format!("包内含路径成分,拒绝: {name}"))?;
        if e.header().entry_type() != tar::EntryType::Regular {
            return Err(format!("包内含非普通文件,拒绝: {name}"));
        }
        let mut out = std::fs::File::create(dir.join(&name)).map_err(|e| format!("建 {name}: {e}"))?;
        std::io::copy(&mut e, &mut out).map_err(|e| format!("解 {name}: {e}"))?;
        if !files.contains(&name) {
            files.push(name);
        }
    }
    Ok(files)
}

// ---------- apply ----------

#[derive(Deserialize, Default)]
struct ApplyReq {
    /// 暂存目录名(stage 返回的 staging;缺省 = 最新一个)
    staging: Option<String>,
    /// 透传给重启进程的参数(worker argv 的 `--` 之后);缺省 = 本进程原始启动参数
    #[serde(default)]
    relaunch_args: Vec<String>,
}

async fn apply(State(st): State<UpdateState>, headers: HeaderMap, body: String) -> Response {
    if let Err(r) = check_auth(&st, &headers) {
        return r;
    }
    let req: ApplyReq = if body.trim().is_empty() {
        ApplyReq::default()
    } else {
        match serde_json::from_str(&body) {
            Ok(r) => r,
            Err(e) => return err(StatusCode::BAD_REQUEST, &format!("请求体非法: {e}")),
        }
    };

    let staging_abs = match resolve_staging(&st.staging_root, &req.staging) {
        Ok(p) => p,
        Err(e) => return err(StatusCode::NOT_FOUND, &e),
    };
    let manifest: StageManifest = match std::fs::read_to_string(staging_abs.join("manifest.json"))
        .map_err(|e| format!("读 manifest 失败: {e}"))
        .and_then(|s| serde_json::from_str(&s).map_err(|e| format!("manifest 解析失败: {e}")))
    {
        Ok(m) => m,
        Err(e) => return err(StatusCode::CONFLICT, &format!("暂存区不可用: {e}(重新 stage)")),
    };
    if !staging_abs.join(&manifest.main).is_file() {
        return err(StatusCode::CONFLICT, &format!("暂存区缺少主程序 {}", manifest.main));
    }
    // 缺省透传本进程原始启动参数(重启后保持原 --addr/--token 等)
    let relaunch_args = if req.relaunch_args.is_empty() {
        std::env::args().skip(1).collect::<Vec<String>>()
    } else {
        req.relaunch_args.clone()
    };
    if let Err(e) = acquire_apply_lock(&staging_abs) {
        return err(StatusCode::CONFLICT, &format!("已有 apply 在进行: {e}"));
    }

    let install = st.cfg.install_dir.clone().unwrap_or_else(default_install_dir);
    let exe = staging_abs.join(&manifest.main);
    let mut argv: Vec<String> = vec![
        APPLY_FLAG.to_string(),
        "--pid".into(),
        std::process::id().to_string(),
        "--install".into(),
        install.display().to_string(),
        "--staging".into(),
        staging_abs.display().to_string(),
        "--from".into(),
        st.cfg.version.clone(),
    ];
    // REQ-1:worker 要等这些关联进程退出后才动文件,名单经 argv 交接
    if !st.cfg.close_processes.is_empty() {
        argv.push("--close".into());
        argv.push(st.cfg.close_processes.join(","));
    }
    if !relaunch_args.is_empty() {
        argv.push("--".into());
        argv.extend(relaunch_args);
    }
    if let Err(e) = ghost_updater_core::start_detached(&exe, &staging_abs, &argv, Some(&staging_abs.join("worker.log"))) {
        ghost_updater_core::release_lock(&staging_abs);
        return err(StatusCode::INTERNAL_SERVER_ERROR, &format!("拉起 apply worker 失败: {e}"));
    }

    // ack 已代表受理;留出响应回程时间后:优雅关闭 close_processes → 自退,
    // 接力棒交给暂存区里的新 exe(worker 会等 --pid 即本进程退出)
    let install_for_log = install.clone();
    let close_list = st.cfg.close_processes.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(800)).await;
        if !close_list.is_empty() {
            ghost_updater_core::append_apply_log(
                &install_for_log,
                &format!("apply 受理,优雅关闭关联进程: {}", close_list.join(", ")),
            );
            let closed = tokio::task::spawn_blocking(move || {
                ghost_updater_core::close_processes_graceful(&close_list, std::time::Duration::from_secs(5))
            })
            .await;
            match closed {
                Ok(Ok(())) => ghost_updater_core::append_apply_log(&install_for_log, "关联进程已全部关闭,代理自退"),
                Ok(Err(e)) => ghost_updater_core::append_apply_log(
                    &install_for_log,
                    &format!("关联进程关闭失败: {e}(代理自退,worker 会兜底等待/强杀)"),
                ),
                Err(e) => ghost_updater_core::append_apply_log(&install_for_log, &format!("关闭任务异常: {e}")),
            }
        }
        std::process::exit(0);
    });
    Json(json!({ "ok": true, "staging": staging_dir_name(&staging_abs), "pid": std::process::id() }))
        .into_response()
}

// ---------- 公共 ----------

fn check_auth(st: &UpdateState, headers: &HeaderMap) -> Result<(), Response> {
    if let Some(token) = &st.cfg.auth_token {
        let ok = headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            == Some(&format!("Bearer {token}"));
        if !ok {
            return Err(err(StatusCode::UNAUTHORIZED, "Bearer token 缺失或不符"));
        }
        return Ok(());
    }
    if st.pubkey.is_some() {
        // 验签模式:stage 在收完整包后强制验签;apply 只能触发已验签内容
        return Ok(());
    }
    Err(err(StatusCode::SERVICE_UNAVAILABLE, "本节点未配置任何鉴权(token/公钥),更新接口已禁用"))
}

/// 最近一次 stage 的暂存目录(名字里带时间戳,字典序即时间序;返回绝对路径)
fn resolve_staging(root: &Path, name: &Option<String>) -> Result<PathBuf, String> {
    match name {
        Some(n) => {
            if n.contains('/') || n.contains('\\') || n.contains("..") {
                return Err("staging 名非法".into());
            }
            let p = root.join(n);
            if !p.is_dir() {
                return Err(format!("暂存目录不存在: {n}"));
            }
            std::path::absolute(p).map_err(|e| format!("解析暂存路径失败: {e}"))
        }
        None => {
            let mut dirs: Vec<PathBuf> = std::fs::read_dir(root)
                .map_err(|e| format!("暂存根目录不存在: {e}"))?
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.is_dir() && p.join("manifest.json").is_file())
                .collect();
            dirs.sort();
            let latest = dirs.pop().ok_or_else(|| "没有任何已暂存的更新".to_string())?;
            std::path::absolute(latest).map_err(|e| format!("解析暂存路径失败: {e}"))
        }
    }
}

fn staging_dir_name(p: &Path) -> String {
    p.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default()
}

fn default_install_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."))
}

fn decode_pubkey(hex_str: &str) -> Result<[u8; 32], String> {
    let b = hex::decode(hex_str.trim()).map_err(|e| format!("hex: {e}"))?;
    let n = b.len();
    b.try_into().map_err(|_| format!("长度 {n} ≠ 32 字节"))
}

fn err(code: StatusCode, msg: &str) -> Response {
    (code, Json(json!({ "ok": false, "error": msg }))).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use tower::ServiceExt;

    /// 每个测试独立暂存根,避免并发测试抢 cwd / 同一目录
    fn app_with(cfg: UpdateConfig, tag: &str) -> (Router, PathBuf) {
        let root = unique_cwd(tag).join("stage-root");
        std::fs::create_dir_all(&root).unwrap();
        let app = routes(UpdateConfig { staging_root: Some(root.clone()), ..cfg });
        (app, root)
    }

    async fn probe_version(app: Router) -> (StatusCode, serde_json::Value) {
        let r = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/update/probe")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let code = r.status();
        let bytes = http_body_util::BodyExt::collect(r.into_body()).await.unwrap().to_bytes();
        (code, serde_json::from_slice(&bytes).unwrap())
    }

    fn test_tar(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut b = tar::Builder::new(Vec::new());
        for (name, data) in files {
            let mut h = tar::Header::new_gnu();
            h.set_size(data.len() as u64);
            h.set_mode(0o644);
            h.set_cksum();
            b.append_data(&mut h, name, *data).unwrap();
        }
        b.into_inner().unwrap()
    }

    /// 手工拼 512 字节 tar 头 + 数据(Builder 会拒收 `..` 等恶意名,恶意包测试用)
    fn raw_tar_entry(name: &str, data: &[u8], typeflag: u8) -> Vec<u8> {
        let mut blk = [0u8; 512];
        blk[..name.len()].copy_from_slice(name.as_bytes());
        blk[100..108].copy_from_slice(b"0000644\0");
        blk[108..116].copy_from_slice(b"0000000\0");
        blk[116..124].copy_from_slice(b"0000000\0");
        let size = format!("{:011o}\0", data.len());
        blk[124..124 + size.len()].copy_from_slice(size.as_bytes());
        blk[156] = typeflag;
        blk[257..263].copy_from_slice(b"ustar\0");
        blk[263..265].copy_from_slice(b"00");
        for b in &mut blk[148..156] {
            *b = b' ';
        }
        let sum: u32 = blk.iter().map(|b| *b as u32).sum();
        let chk = format!("{:06o}\0 ", sum);
        blk[148..156].copy_from_slice(chk.as_bytes());
        let mut out = blk.to_vec();
        out.extend_from_slice(data);
        let pad = (512 - data.len() % 512) % 512;
        out.extend(std::iter::repeat_n(0u8, pad));
        out
    }

    fn sha(data: &[u8]) -> String {
        hex::encode(Sha256::digest(data))
    }

    async fn stage_req(
        app: Router,
        uri: String,
        headers: HeaderMap,
        body: Vec<u8>,
    ) -> (StatusCode, serde_json::Value) {
        let mut builder = axum::http::Request::builder().method("POST").uri(uri);
        for (k, v) in headers.iter() {
            builder = builder.header(k, v);
        }
        let r = app.oneshot(builder.body(Body::from(body)).unwrap()).await.unwrap();
        let code = r.status();
        let bytes = http_body_util::BodyExt::collect(r.into_body()).await.unwrap().to_bytes();
        (code, serde_json::from_slice(&bytes).unwrap_or(json!(null)))
    }

    fn auth(value: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(axum::http::header::AUTHORIZATION, value.parse().unwrap());
        h
    }

    #[tokio::test]
    async fn probe_reports_version_and_platform() {
        let (app, _root) = app_with(UpdateConfig::new("9.9.9-test"), "probe");
        let (code, body) = probe_version(app).await;
        assert_eq!(code, StatusCode::OK);
        assert_eq!(body["version"], "9.9.9-test");
        assert_eq!(body["os"], std::env::consts::OS);
        assert!(body["started_at"].as_u64().unwrap() > 0);
    }

    #[tokio::test]
    async fn stage_without_auth_config_is_503() {
        let (app, _root) = app_with(UpdateConfig::new("1.0.0"), "noauth");
        let (code, _) = stage_req(app, "/update/stage?sha256=x&main=a.exe".into(), HeaderMap::new(), vec![]).await;
        assert_eq!(code, StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn stage_wrong_token_is_401() {
        let (app, _root) = app_with(UpdateConfig { auth_token: Some("secret".into()), ..UpdateConfig::new("1.0.0") }, "badtok");
        let (code, _) = stage_req(app, "/update/stage?sha256=x&main=a.exe".into(), auth("Bearer wrong"), vec![]).await;
        assert_eq!(code, StatusCode::UNAUTHORIZED);
    }

    /// REQ-3 验收:拒绝路径必须把请求体 drain 完再回 401,否则 hyper 对未消费 body
    /// 的请求发完响应即断连,仍在流式上传的客户端只会看到连接重置。真实 TCP 全双工验证
    /// (内存 oneshot 测不出连接行为):客户端边写 3MiB 边等,无 drain 时写/读必炸一个。
    #[tokio::test]
    async fn stage_wrong_token_big_body_still_401_over_tcp() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let (app, _root) = app_with(UpdateConfig { auth_token: Some("t".into()), ..UpdateConfig::new("1.0.0") }, "drain401");
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
        let head = "POST /update/stage?sha256=ab&main=a.exe HTTP/1.1\r\n\
                    Host: e2e\r\nAuthorization: Bearer wrong\r\nContent-Length: 3145728\r\n\
                    Connection: close\r\n\r\n";
        s.write_all(head.as_bytes()).await.unwrap();
        let chunk = vec![0u8; 65536];
        for _ in 0..48 {
            s.write_all(&chunk).await.unwrap();
        }
        let mut buf = Vec::new();
        let _ = s.read_to_end(&mut buf).await;
        let head_str = String::from_utf8_lossy(&buf[..buf.len().min(2048)]).to_string();
        assert!(head_str.starts_with("HTTP/1.1 401"), "应读到 401,实际: {head_str}");
    }

    #[tokio::test]
    async fn stage_sha_mismatch_rejects_and_cleans() {
        let (app, root) = app_with(UpdateConfig { auth_token: Some("t".into()), ..UpdateConfig::new("1.0.0") }, "shabad");
        let tar = test_tar(&[("app.exe", b"hello")]);
        let bad_sha = sha(b"other");
        let (code, body) = stage_req(app, format!("/update/stage?sha256={bad_sha}&main=app.exe"), auth("Bearer t"), tar).await;
        assert_eq!(code, StatusCode::BAD_REQUEST);
        assert!(body["error"].as_str().unwrap().contains("SHA256 不符"));
        // 坏包暂存已清除
        let left = std::fs::read_dir(&root).unwrap().filter_map(|e| e.ok()).count();
        assert_eq!(left, 0);
    }

    #[tokio::test]
    async fn stage_happy_path_writes_manifest() {
        let (app, root) = app_with(UpdateConfig { auth_token: Some("t".into()), ..UpdateConfig::new("1.0.0") }, "happy");
        let tar = test_tar(&[("app.exe", b"new-bin"), ("notes.txt", b"hi")]);
        let good = sha(&tar);
        let (code, body) = stage_req(app, format!("/update/stage?sha256={good}&main=app.exe"), auth("Bearer t"), tar).await;
        assert_eq!(code, StatusCode::OK, "body={body}");
        assert_eq!(body["ok"], true);
        let name = body["staging"].as_str().unwrap().to_string();
        let dir = root.join(&name);
        assert_eq!(std::fs::read(dir.join("app.exe")).unwrap(), b"new-bin");
        let m: StageManifest =
            serde_json::from_str(&std::fs::read_to_string(dir.join("manifest.json")).unwrap()).unwrap();
        assert_eq!(m.main, "app.exe");
        assert_eq!(m.files.len(), 2);
        assert!(!dir.join("package.tar").exists()); // 包文件已清理
    }

    #[tokio::test]
    async fn stage_rejects_traversal_and_non_regular_entries() {
        let (app, root) = app_with(UpdateConfig { auth_token: Some("t".into()), ..UpdateConfig::new("1.0.0") }, "evil");

        let evil = raw_tar_entry("..\\evil.exe", b"x", b'0');
        let s = sha(&evil);
        let (code, _) = stage_req(app.clone(), format!("/update/stage?sha256={s}&main=evil.exe"), auth("Bearer t"), evil).await;
        assert_eq!(code, StatusCode::BAD_REQUEST);

        let link = raw_tar_entry("link.exe", b"", b'2'); // symlink
        let s = sha(&link);
        let (code, _) = stage_req(app.clone(), format!("/update/stage?sha256={s}&main=link.exe"), auth("Bearer t"), link).await;
        assert_eq!(code, StatusCode::BAD_REQUEST);

        // 两个恶意包都不留暂存
        let left = std::fs::read_dir(&root).unwrap().filter_map(|e| e.ok()).count();
        assert_eq!(left, 0);
    }

    #[tokio::test]
    async fn apply_without_staging_is_404() {
        let (app, _root) = app_with(UpdateConfig { auth_token: Some("t".into()), ..UpdateConfig::new("1.0.0") }, "apply404");
        let (code, _) = stage_req(app, "/update/apply".into(), auth("Bearer t"), vec![]).await;
        assert_eq!(code, StatusCode::NOT_FOUND);
    }

    fn unique_cwd(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("ghost-net-{tag}-{}-{}", std::process::id(), unix_secs()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }
}
````

## File: crates/ghost-updater-net/Cargo.toml
````toml
[package]
name = "ghost-updater-net"
description = "ghost-updater 网络层:可嵌入任意 Axum 服务的 /update/* 路由(probe/stage/apply)与独立 agent"
version.workspace = true
edition.workspace = true
license.workspace = true

[dependencies]
ghost-updater-core = { path = "../ghost-updater-core" }
axum = { workspace = true }
tokio = { workspace = true }
http-body-util = { workspace = true }
serde = { workspace = true }
serde_json = { workspace = true }
sha2 = { workspace = true }
hex = { workspace = true }
tar = { workspace = true }
ed25519-dalek = { workspace = true }

[dev-dependencies]
tower = { workspace = true }
````

## File: crates/ghost-updater-tauri/src/app.css
````css
:root {
  color-scheme: dark;
  --bg: #12141a;
  --panel: #1a1d26;
  --panel2: #20242f;
  --line: #2c313e;
  --fg: #d7dae2;
  --dim: #8b91a0;
  --acc: #4f9cf9;
  --ok: #3fb96f;
  --bad: #e2564f;
  --warn: #d9a03f;
}

* { box-sizing: border-box; }

body {
  margin: 0;
  background: var(--bg);
  color: var(--fg);
  font: 13px/1.5 "Segoe UI", "Microsoft YaHei", sans-serif;
  user-select: none;
}

#app { padding: 14px; height: 100vh; display: flex; flex-direction: column; gap: 12px; }

h2 { font-size: 13px; margin: 0 0 8px; color: var(--dim); font-weight: 600; letter-spacing: 1px; }

.panel {
  background: var(--panel);
  border: 1px solid var(--line);
  border-radius: 8px;
  padding: 12px;
}

.row { display: flex; gap: 8px; align-items: center; flex-wrap: wrap; }
.grow { flex: 1; }

input[type="text"], input[type="password"], select, textarea {
  background: var(--panel2);
  border: 1px solid var(--line);
  border-radius: 6px;
  color: var(--fg);
  padding: 6px 9px;
  outline: none;
  min-width: 0;
  font: inherit;
}
textarea { resize: vertical; font-family: Consolas, "Cascadia Mono", monospace; }
input:focus, select:focus, textarea:focus { border-color: var(--acc); }

button {
  background: var(--panel2);
  border: 1px solid var(--line);
  border-radius: 6px;
  color: var(--fg);
  padding: 6px 14px;
  cursor: pointer;
  white-space: nowrap;
}
button:hover { border-color: var(--acc); color: #fff; }
button:disabled { opacity: 0.45; cursor: default; }
button.primary { background: var(--acc); border-color: var(--acc); color: #fff; }
button.danger { background: transparent; border-color: var(--bad); color: var(--bad); }

table { width: 100%; border-collapse: collapse; }
th { text-align: left; color: var(--dim); font-weight: 500; padding: 4px 8px; border-bottom: 1px solid var(--line); }
td { padding: 6px 8px; border-bottom: 1px solid var(--line); }
tr:hover td { background: rgba(79, 156, 249, 0.05); }

.badge { padding: 1px 8px; border-radius: 10px; font-size: 12px; }
.badge.on { background: rgba(63, 185, 111, 0.15); color: var(--ok); }
.badge.off { background: rgba(226, 86, 79, 0.15); color: var(--bad); }
.badge.idle { background: var(--panel2); color: var(--dim); }

.meta { color: var(--dim); font-size: 12px; }
.sha { font-family: Consolas, monospace; color: var(--warn); font-size: 12px; word-break: break-all; }

#log {
  flex: 1;
  min-height: 140px;
  overflow-y: auto;
  background: #0d0f14;
  border: 1px solid var(--line);
  border-radius: 8px;
  padding: 8px 10px;
  font: 12px/1.6 Consolas, "Cascadia Mono", monospace;
  user-select: text;
  white-space: pre-wrap;
  word-break: break-all;
}
.log-line { color: var(--dim); }
.log-line .tag { font-weight: 700; }
.err { color: var(--bad); }
.okline { color: var(--ok); }

.hint { color: var(--dim); font-size: 12px; }
````

## File: crates/ghost-updater-tauri/src/App.svelte
````svelte
<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { onMount } from "svelte";

  interface HostEntry { addr: string }
  // 一个项目 = 更新包配置 + 默认端口 + 目标主机列表,整档保存(apps.json)
  interface AppProfile {
    name: string; port: number; hosts: HostEntry[];
    dist_dir: string; main_exe: string; overwrite_config: boolean;
    token: string; sign_key: string; timeout_secs: number;
  }
  interface ProbeResult {
    addr: string; reachable: boolean; version: string; started_at: number;
    os: string; arch: string; pubkey_fingerprint: string; error: string;
  }
  interface FileEntry { name: string; size: number }
  interface DistInfo {
    ok: boolean; version: string; main: string; files: FileEntry[];
    total_bytes: number; package_sha256: string; error: string;
  }
  interface HostState {
    addr: string;
    probing: boolean;
    reachable: boolean;
    version: string;
    os: string;
    arch: string;
    lastError: string;
    lastResult: string;   // 最近一次更新结果描述
    lastOk: boolean;
    updating: boolean;
    selected: boolean;
  }
  interface LogLine { addr: string; line: string }

  // ---- 更新包配置直接读写当前项目(apps[activeIdx]),切换项目即随档切换 ----
  let dist: DistInfo | null = null;
  let distError = "";

  // ---- 项目配置(每套 = 更新包设置 + 项目名 + 默认端口 + 机器列表) ----
  function blankProject(name: string, port = 7600): AppProfile {
    return {
      name, port, hosts: [],
      dist_dir: "", main_exe: "", overwrite_config: false,
      token: "", sign_key: "", timeout_secs: 120,
    };
  }
  let apps: AppProfile[] = [blankProject("项目A")];
  let activeIdx = 0;
  $: active = apps[activeIdx];
  // 函数内一律用 currentProject():active 是 $: 派生,同步改 activeIdx 后尚未刷新,
  // switchApp 里紧接着读 active 会拿到上一个项目。
  const currentProject = (): AppProfile | undefined => apps[activeIdx];

  // ---- 当前应用的主机工作列表 ----
  let hosts: HostState[] = [];
  let newIp = "";
  let newPort = 7600;
  let batchOpen = false;
  let batchText = "";
  let importFrom = "";
  let addingApp = false;
  let newName = "";
  let renaming = false;
  let renameText = "";
  let logs: { addr: string; line: string; at: string }[] = [];
  let pushing = false;
  let logEl: HTMLElement;

  function fmtSize(n: number): string {
    if (n > 1024 * 1024) return (n / 1024 / 1024).toFixed(1) + " MB";
    if (n > 1024) return (n / 1024).toFixed(1) + " KB";
    return n + " B";
  }

  function now(): string {
    return new Date().toLocaleTimeString("zh-CN", { hour12: false });
  }

  function addrColor(addr: string): string {
    let h = 0;
    for (const c of addr) h = (h * 31 + c.charCodeAt(0)) % 360;
    return `hsl(${h}, 65%, 70%)`;
  }

  function findHost(addr: string): HostState | undefined {
    return hosts.find((h) => h.addr === addr);
  }

  function pushLog(addr: string, line: string) {
    logs = [...logs.slice(-500), { addr, line, at: now() }];
    if (logEl) logEl.scrollTop = logEl.scrollHeight;
  }

  function freshHost(addr: string): HostState {
    return {
      addr, probing: false, reachable: false, version: "", os: "", arch: "",
      lastError: "", lastResult: "", lastOk: false, updating: false, selected: false,
    };
  }

  // ip:端口 已带端口就用原样;裸 IP 补当前应用默认端口
  function makeAddr(ip: string, port: number | string): string {
    ip = ip.trim();
    if (!ip) return "";
    if (ip.includes(":")) return ip;
    const p = Number(port);
    if (!(p >= 1 && p <= 65535)) return "";
    return `${ip}:${p}`;
  }

  // ---- 项目配置持久化 ----
  async function loadApps() {
    apps = await invoke<AppProfile[]>("load_apps");
    if (apps.length === 0) apps = [blankProject("项目A")];
    await switchApp(0, false);
  }

  // 主机列表是独立工作态,保存前同步回当前项目
  async function persistApps() {
    const p = currentProject();
    if (p) p.hosts = hosts.map((h) => ({ addr: h.addr }));
    apps = apps; // 触发下拉框选项文本(台数/端口)重渲染
    await invoke("save_apps", { apps });
  }

  // 更新包输入框用:改动防抖落盘
  let saveTimer: ReturnType<typeof setTimeout> | undefined;
  function persistProject() {
    clearTimeout(saveTimer);
    saveTimer = setTimeout(() => { persistApps().catch(() => {}); }, 300);
  }

  async function switchApp(i: number, autoprobe = true) {
    activeIdx = i;
    hosts = apps[i].hosts.map((e) => freshHost(e.addr));
    newPort = apps[i].port;
    importFrom = "";
    dist = null;
    distError = "";
    if (apps[i].dist_dir?.trim()) refreshDist();
    if (autoprobe) hosts.forEach((h) => probeOne(h.addr));
  }

  function uniqueName(base: string): string {
    if (!apps.some((a) => a.name === base)) return base;
    for (let n = 2; ; n++) {
      const name = `${base}-${n}`;
      if (!apps.some((a) => a.name === name)) return name;
    }
  }

  async function createApp() {
    const name = uniqueName(newName.trim() || `项目${apps.length + 1}`);
    apps = [...apps, blankProject(name, newPort || 7600)];
    newName = "";
    addingApp = false;
    await switchApp(apps.length - 1, false);
    await persistApps();
  }

  async function renameApp() {
    const n = renameText.trim();
    if (n && active) {
      active.name = n;
      apps = apps;
      await persistApps();
    }
    renaming = false;
  }

  async function deleteApp() {
    apps = apps.filter((_, i) => i !== activeIdx);
    if (apps.length === 0) apps = [blankProject("项目A")];
    await switchApp(Math.max(0, Math.min(activeIdx, apps.length - 1)), false);
    await persistApps();
  }

  async function changePort() {
    if (!active) return;
    let p = Number(String(active.port).replace(/\D/g, ""));
    if (!(p >= 1 && p <= 65535)) p = 7600;
    active.port = p;
    newPort = p;
    apps = apps;
    await persistApps();
  }

  // ---- 主机增删 ----
  async function addHost() {
    const a = makeAddr(newIp, newPort);
    if (!a || hosts.some((h) => h.addr === a)) return;
    hosts = [...hosts, freshHost(a)];
    newIp = "";
    await persistApps();
    probeOne(a);
  }

  async function removeHost(addr: string) {
    hosts = hosts.filter((h) => h.addr !== addr);
    await persistApps();
  }

  // 批量添加:一行/一条一个 IP 或 ip:端口,逗号空格分隔也行
  async function batchAdd() {
    const tokens = batchText.split(/[\s,;，；]+/).filter(Boolean);
    const fresh: HostState[] = [];
    for (const t of tokens) {
      const a = makeAddr(t, newPort);
      if (a && !hosts.some((h) => h.addr === a) && !fresh.some((h) => h.addr === a))
        fresh.push(freshHost(a));
    }
    if (fresh.length === 0) return;
    hosts = [...hosts, ...fresh];
    batchText = "";
    await persistApps();
    pushLog("批量添加", `新增 ${fresh.length} 台,跳过无效/重复 ${tokens.length - fresh.length} 条`);
    fresh.forEach((h) => probeOne(h.addr));
  }

  // 把另一个应用配置好的机器整批导入(端口随源配置逐台保留)
  async function importHosts() {
    const src = apps.find((a) => a.name === importFrom);
    if (!src) return;
    const fresh: HostState[] = [];
    for (const e of src.hosts) {
      if (!hosts.some((h) => h.addr === e.addr) && !fresh.some((h) => h.addr === e.addr))
        fresh.push(freshHost(e.addr));
    }
    if (fresh.length === 0) return;
    hosts = [...hosts, ...fresh];
    importFrom = "";
    await persistApps();
    pushLog("导入机器", `从「${src.name}」导入 ${fresh.length} 台`);
    fresh.forEach((h) => probeOne(h.addr));
  }

  // ---- 探测 ----
  async function probeOne(addr: string) {
    const h = findHost(addr);
    if (!h || h.probing) return;
    h.probing = true;
    hosts = hosts;
    try {
      const r = await invoke<ProbeResult>("cmd_probe", { addr, token: currentProject()?.token || null });
      h.reachable = r.reachable;
      h.version = r.version;
      h.os = r.os;
      h.arch = r.arch;
      h.lastError = r.reachable ? "" : r.error;
    } catch (e) {
      h.reachable = false;
      h.lastError = String(e);
    }
    h.probing = false;
    hosts = hosts;
  }

  async function probeAll() {
    await Promise.all(hosts.map((h) => probeOne(h.addr)));
  }

  // ---- dist 预览 ----
  async function refreshDist() {
    distError = "";
    dist = null;
    const p = currentProject();
    const dir = p?.dist_dir?.trim() || "";
    if (!dir) return;
    try {
      const r = await invoke<DistInfo>("cmd_dist_info", {
        distDir: dir,
        main: p?.main_exe?.trim() || null,
        overwriteConfig: !!p?.overwrite_config,
      });
      if (r.ok) dist = r;
      else distError = r.error;
    } catch (e) {
      distError = String(e);
    }
  }

  // ---- 更新 ----
  async function updateHosts(targets: string[]) {
    if (pushing || targets.length === 0 || !dist?.ok) return;
    pushing = true;
    hosts = hosts.map((h) => (targets.includes(h.addr) ? { ...h, updating: true } : h));
    const p = currentProject();
    const jobs = targets.map(async (addr) => {
      try {
        const r = await invoke<any>("cmd_update_host", {
          addr,
          distDir: p?.dist_dir?.trim() || "",
          main: p?.main_exe?.trim() || null,
          token: p?.token || null,
          signKey: p?.sign_key?.trim() || null,
          overwriteConfig: !!p?.overwrite_config,
          timeoutSecs: Number(p?.timeout_secs) || 120,
        });
        const h = findHost(addr);
        if (h) {
          h.updating = false;
          if (r.ok) {
            h.lastOk = true;
            h.lastResult = `${r.old_version} → ${r.new_version}`;
            h.version = r.new_version;
            probeOne(addr);
          } else {
            h.lastOk = false;
            h.lastResult = r.detail;
          }
        }
      } catch (e) {
        const h = findHost(addr);
        if (h) {
          h.updating = false;
          h.lastOk = false;
          h.lastResult = String(e);
        }
      }
      hosts = hosts;
    });
    await Promise.all(jobs);
    pushing = false;
    hosts = hosts;
  }

  const updateSelected = () => updateHosts(hosts.filter((h) => h.selected).map((h) => h.addr));
  const updateAll = () => updateHosts(hosts.map((h) => h.addr));
  const retryFailed = () =>
    updateHosts(hosts.filter((h) => h.lastResult && !h.lastOk).map((h) => h.addr));

  onMount(async () => {
    await loadApps();
    await listen<LogLine>("update-log", (e) => {
      const { addr, line } = e.payload;
      pushLog(addr, line);
    });
  });
</script>

<div class="panel">
  <h2>更新包 <span class="meta">· 随项目「{active?.name || ""}」保存</span></h2>
  <div class="row">
    <input type="text" class="grow" placeholder="新版 dist 目录,如 D:\dist" bind:value={active.dist_dir} on:change={() => { refreshDist(); persistProject(); }} />
    <input type="text" style="width: 130px" placeholder="主程序(可选)" bind:value={active.main_exe} on:change={() => { refreshDist(); persistProject(); }} />
    <label class="hint"><input type="checkbox" bind:checked={active.overwrite_config} on:change={() => { refreshDist(); persistProject(); }} /> 覆盖 config.json</label>
    <button on:click={refreshDist}>重新计算</button>
  </div>
  <div class="row" style="margin-top: 8px">
    <input type="password" style="width: 200px" placeholder="Token(可选)" bind:value={active.token} on:change={persistProject} />
    <input type="password" style="width: 200px" placeholder="签名私钥 hex(可选)" bind:value={active.sign_key} on:change={persistProject} />
    <span class="hint">轮询超时</span>
    <input type="text" style="width: 60px" bind:value={active.timeout_secs} on:change={persistProject} />
    <span class="hint">秒</span>
  </div>
  {#if dist}
    <div class="row" style="margin-top: 8px">
      <span class="badge on">v{dist.version}</span>
      <span class="meta">主程序 {dist.main} · {dist.files.length} 个文件 · {fmtSize(dist.total_bytes)}</span>
      <span class="sha grow">sha256 = {dist.package_sha256}</span>
    </div>
    <div class="meta">{dist.files.map((f) => f.name).join("  ")}</div>
  {:else if distError}
    <div class="meta err" style="margin-top: 8px">{distError}</div>
  {/if}
</div>

<div class="panel">
  <div class="row">
    {#if renaming}
      <h2 class="grow">项目</h2>
      <input type="text" style="width: 220px" bind:value={renameText}
        on:keydown={(e) => { if (e.key === "Enter") renameApp(); if (e.key === "Escape") renaming = false; }} />
      <button class="primary" on:click={renameApp}>确定</button>
      <button on:click={() => (renaming = false)}>取消</button>
    {:else if addingApp}
      <h2 class="grow">项目</h2>
      <input type="text" style="width: 220px" placeholder="新项目名,如 项目C" bind:value={newName}
        on:keydown={(e) => { if (e.key === "Enter") createApp(); if (e.key === "Escape") (addingApp = false); }} />
      <button class="primary" on:click={createApp}>创建</button>
      <button on:click={() => (addingApp = false)}>取消</button>
    {:else}
      <h2 class="grow">项目(推送对象)</h2>
      <select bind:value={activeIdx} on:change={() => switchApp(activeIdx)}>
        {#each apps as a, i}
          <option value={i}>{a.name} · {a.hosts.length}台 · 端口{a.port}</option>
        {/each}
      </select>
      <button on:click={() => { renaming = true; renameText = active?.name || ""; }}>改名</button>
      <button on:click={() => { addingApp = true; newName = ""; }}>新增项目</button>
      <button class="danger" on:click={deleteApp}>删除项目</button>
      <span class="hint">默认端口</span>
      <input type="text" style="width: 60px" bind:value={active.port} on:change={changePort} />
    {/if}
  </div>

  {#if batchOpen}
    <div class="row" style="margin-top: 8px">
      <textarea class="grow" rows="3" bind:value={batchText}
        placeholder="粘贴机器列表一次添加:每行一个 IP(端口用上方默认端口),也可写 ip:端口 覆盖;空格/逗号分隔均可&#10;192.168.1.60&#10;192.168.1.61:7601"></textarea>
    </div>
    <div class="row" style="margin-top: 6px">
      <button class="primary" on:click={batchAdd}>批量添加</button>
      <select bind:value={importFrom}>
        <option value="" disabled>从其他项目整批导入已配好的机器…</option>
        {#each apps as a, i}
          {#if i !== activeIdx}
            <option value={a.name}>{a.name}({a.hosts.length}台)</option>
          {/if}
        {/each}
      </select>
      <button on:click={importHosts} disabled={!importFrom}>导入</button>
      <span class="grow"></span>
      <button on:click={() => (batchOpen = false)}>收起</button>
    </div>
  {/if}

  <div class="row" style="margin-top: 8px">
    <h2 class="grow">{active?.name || ""} 主机({hosts.length})</h2>
    <input type="text" style="width: 200px" placeholder="IP,如 192.168.1.60" bind:value={newIp}
      on:keydown={(e) => e.key === "Enter" && addHost()} />
    <span class="hint">:</span>
    <input type="text" style="width: 64px" bind:value={newPort}
      on:keydown={(e) => e.key === "Enter" && addHost()} />
    <button on:click={addHost}>添加</button>
    <button on:click={() => (batchOpen = !batchOpen)}>{batchOpen ? "收起批量" : "批量添加"}</button>
    <button on:click={probeAll} disabled={pushing}>探测全部</button>
    <button on:click={updateSelected} disabled={pushing || !dist?.ok}>更新选中</button>
    <button on:click={retryFailed} disabled={pushing || !dist?.ok}>重试失败</button>
    <button class="primary" on:click={updateAll} disabled={pushing || !dist?.ok}>全部更新</button>
  </div>
  <table>
    <thead>
      <tr>
        <th style="width: 30px"></th>
        <th>地址</th>
        <th style="width: 70px">状态</th>
        <th style="width: 90px">版本</th>
        <th style="width: 140px">平台</th>
        <th>最近更新</th>
        <th style="width: 130px">操作</th>
      </tr>
    </thead>
    <tbody>
      {#each hosts as h (h.addr)}
        <tr>
          <td><input type="checkbox" bind:checked={h.selected} disabled={h.updating} /></td>
          <td><b>{h.addr}</b></td>
          <td>
            {#if h.probing || h.updating}
              <span class="badge idle">{h.updating ? "更新中…" : "探测中…"}</span>
            {:else if h.reachable}
              <span class="badge on">在线</span>
            {:else}
              <span class="badge off">离线</span>
            {/if}
          </td>
          <td>{h.version || "—"}</td>
          <td class="meta">{h.os ? `${h.os}/${h.arch}` : "—"}</td>
          <td class={h.lastOk ? "okline" : "err"}>
            {h.lastResult || h.lastError || "—"}
          </td>
          <td>
            <button on:click={() => probeOne(h.addr)} disabled={h.updating}>探测</button>
            <button class="primary" on:click={() => updateHosts([h.addr])} disabled={pushing || h.updating || !dist?.ok}>更新</button>
            <button class="danger" on:click={() => removeHost(h.addr)} disabled={h.updating}>删</button>
          </td>
        </tr>
      {:else}
        <tr><td colspan="7" class="meta" style="text-align: center; padding: 18px">
          「{active?.name}」还没有机器:填 IP 添加,或用「批量添加」粘一排 IP 一次全加
        </td></tr>
      {/each}
    </tbody>
  </table>
</div>

<div class="row hint">
  <span>操作日志(实时)</span>
  <span class="grow"></span>
  <button on:click={() => (logs = [])}>清空</button>
</div>
<div id="log" bind:this={logEl}>
  {#each logs as l}
    <div class="log-line">
      <span class="meta">{l.at}</span>
      <span class="tag" style="color: {addrColor(l.addr)}">[{l.addr}]</span>
      <span class={l.line.includes("失败") || l.line.includes("错误") ? "err" : l.line.includes("成功") ? "okline" : ""}>{l.line}</span>
    </div>
  {:else}
    <div class="log-line meta">等待操作…</div>
  {/each}
</div>
````

## File: crates/ghost-updater-tauri/src/main.ts
````typescript
import { mount } from "svelte";
import App from "./App.svelte";
import "./app.css";

const app = mount(App, { target: document.getElementById("app")! });

export default app;
````

## File: crates/ghost-updater-tauri/src-tauri/capabilities/default.json
````json
{
  "$schema": "gen/schemas/desktop-schema.json",
  "identifier": "default",
  "description": "ghost-updater 主窗口默认能力",
  "windows": ["main"],
  "permissions": ["core:default"]
}
````

## File: crates/ghost-updater-tauri/src-tauri/gen/schemas/acl-manifests.json
````json
{"core":{"default_permission":{"identifier":"default","description":"Default core plugins set.","permissions":["core:path:default","core:event:default","core:window:default","core:webview:default","core:app:default","core:image:default","core:resources:default","core:menu:default","core:tray:default"]},"permissions":{},"permission_sets":{},"global_scope_schema":null},"core:app":{"default_permission":{"identifier":"default","description":"Default permissions for the plugin.","permissions":["allow-version","allow-name","allow-tauri-version","allow-identifier","allow-bundle-type","allow-register-listener","allow-remove-listener","allow-supports-multiple-windows"]},"permissions":{"allow-app-hide":{"identifier":"allow-app-hide","description":"Enables the app_hide command without any pre-configured scope.","commands":{"allow":["app_hide"],"deny":[]}},"allow-app-show":{"identifier":"allow-app-show","description":"Enables the app_show command without any pre-configured scope.","commands":{"allow":["app_show"],"deny":[]}},"allow-bundle-type":{"identifier":"allow-bundle-type","description":"Enables the bundle_type command without any pre-configured scope.","commands":{"allow":["bundle_type"],"deny":[]}},"allow-default-window-icon":{"identifier":"allow-default-window-icon","description":"Enables the default_window_icon command without any pre-configured scope.","commands":{"allow":["default_window_icon"],"deny":[]}},"allow-exit":{"identifier":"allow-exit","description":"Enables the exit command without any pre-configured scope.","commands":{"allow":["exit"],"deny":[]}},"allow-fetch-data-store-identifiers":{"identifier":"allow-fetch-data-store-identifiers","description":"Enables the fetch_data_store_identifiers command without any pre-configured scope.","commands":{"allow":["fetch_data_store_identifiers"],"deny":[]}},"allow-identifier":{"identifier":"allow-identifier","description":"Enables the identifier command without any pre-configured scope.","commands":{"allow":["identifier"],"deny":[]}},"allow-name":{"identifier":"allow-name","description":"Enables the name command without any pre-configured scope.","commands":{"allow":["name"],"deny":[]}},"allow-register-listener":{"identifier":"allow-register-listener","description":"Enables the register_listener command without any pre-configured scope.","commands":{"allow":["register_listener"],"deny":[]}},"allow-remove-data-store":{"identifier":"allow-remove-data-store","description":"Enables the remove_data_store command without any pre-configured scope.","commands":{"allow":["remove_data_store"],"deny":[]}},"allow-remove-listener":{"identifier":"allow-remove-listener","description":"Enables the remove_listener command without any pre-configured scope.","commands":{"allow":["remove_listener"],"deny":[]}},"allow-set-app-theme":{"identifier":"allow-set-app-theme","description":"Enables the set_app_theme command without any pre-configured scope.","commands":{"allow":["set_app_theme"],"deny":[]}},"allow-set-dock-visibility":{"identifier":"allow-set-dock-visibility","description":"Enables the set_dock_visibility command without any pre-configured scope.","commands":{"allow":["set_dock_visibility"],"deny":[]}},"allow-supports-multiple-windows":{"identifier":"allow-supports-multiple-windows","description":"Enables the supports_multiple_windows command without any pre-configured scope.","commands":{"allow":["supports_multiple_windows"],"deny":[]}},"allow-tauri-version":{"identifier":"allow-tauri-version","description":"Enables the tauri_version command without any pre-configured scope.","commands":{"allow":["tauri_version"],"deny":[]}},"allow-version":{"identifier":"allow-version","description":"Enables the version command without any pre-configured scope.","commands":{"allow":["version"],"deny":[]}},"deny-app-hide":{"identifier":"deny-app-hide","description":"Denies the app_hide command without any pre-configured scope.","commands":{"allow":[],"deny":["app_hide"]}},"deny-app-show":{"identifier":"deny-app-show","description":"Denies the app_show command without any pre-configured scope.","commands":{"allow":[],"deny":["app_show"]}},"deny-bundle-type":{"identifier":"deny-bundle-type","description":"Denies the bundle_type command without any pre-configured scope.","commands":{"allow":[],"deny":["bundle_type"]}},"deny-default-window-icon":{"identifier":"deny-default-window-icon","description":"Denies the default_window_icon command without any pre-configured scope.","commands":{"allow":[],"deny":["default_window_icon"]}},"deny-exit":{"identifier":"deny-exit","description":"Denies the exit command without any pre-configured scope.","commands":{"allow":[],"deny":["exit"]}},"deny-fetch-data-store-identifiers":{"identifier":"deny-fetch-data-store-identifiers","description":"Denies the fetch_data_store_identifiers command without any pre-configured scope.","commands":{"allow":[],"deny":["fetch_data_store_identifiers"]}},"deny-identifier":{"identifier":"deny-identifier","description":"Denies the identifier command without any pre-configured scope.","commands":{"allow":[],"deny":["identifier"]}},"deny-name":{"identifier":"deny-name","description":"Denies the name command without any pre-configured scope.","commands":{"allow":[],"deny":["name"]}},"deny-register-listener":{"identifier":"deny-register-listener","description":"Denies the register_listener command without any pre-configured scope.","commands":{"allow":[],"deny":["register_listener"]}},"deny-remove-data-store":{"identifier":"deny-remove-data-store","description":"Denies the remove_data_store command without any pre-configured scope.","commands":{"allow":[],"deny":["remove_data_store"]}},"deny-remove-listener":{"identifier":"deny-remove-listener","description":"Denies the remove_listener command without any pre-configured scope.","commands":{"allow":[],"deny":["remove_listener"]}},"deny-set-app-theme":{"identifier":"deny-set-app-theme","description":"Denies the set_app_theme command without any pre-configured scope.","commands":{"allow":[],"deny":["set_app_theme"]}},"deny-set-dock-visibility":{"identifier":"deny-set-dock-visibility","description":"Denies the set_dock_visibility command without any pre-configured scope.","commands":{"allow":[],"deny":["set_dock_visibility"]}},"deny-supports-multiple-windows":{"identifier":"deny-supports-multiple-windows","description":"Denies the supports_multiple_windows command without any pre-configured scope.","commands":{"allow":[],"deny":["supports_multiple_windows"]}},"deny-tauri-version":{"identifier":"deny-tauri-version","description":"Denies the tauri_version command without any pre-configured scope.","commands":{"allow":[],"deny":["tauri_version"]}},"deny-version":{"identifier":"deny-version","description":"Denies the version command without any pre-configured scope.","commands":{"allow":[],"deny":["version"]}}},"permission_sets":{},"global_scope_schema":null},"core:event":{"default_permission":{"identifier":"default","description":"Default permissions for the plugin, which enables all commands.","permissions":["allow-listen","allow-unlisten","allow-emit","allow-emit-to"]},"permissions":{"allow-emit":{"identifier":"allow-emit","description":"Enables the emit command without any pre-configured scope.","commands":{"allow":["emit"],"deny":[]}},"allow-emit-to":{"identifier":"allow-emit-to","description":"Enables the emit_to command without any pre-configured scope.","commands":{"allow":["emit_to"],"deny":[]}},"allow-listen":{"identifier":"allow-listen","description":"Enables the listen command without any pre-configured scope.","commands":{"allow":["listen"],"deny":[]}},"allow-unlisten":{"identifier":"allow-unlisten","description":"Enables the unlisten command without any pre-configured scope.","commands":{"allow":["unlisten"],"deny":[]}},"deny-emit":{"identifier":"deny-emit","description":"Denies the emit command without any pre-configured scope.","commands":{"allow":[],"deny":["emit"]}},"deny-emit-to":{"identifier":"deny-emit-to","description":"Denies the emit_to command without any pre-configured scope.","commands":{"allow":[],"deny":["emit_to"]}},"deny-listen":{"identifier":"deny-listen","description":"Denies the listen command without any pre-configured scope.","commands":{"allow":[],"deny":["listen"]}},"deny-unlisten":{"identifier":"deny-unlisten","description":"Denies the unlisten command without any pre-configured scope.","commands":{"allow":[],"deny":["unlisten"]}}},"permission_sets":{},"global_scope_schema":null},"core:image":{"default_permission":{"identifier":"default","description":"Default permissions for the plugin, which enables all commands.","permissions":["allow-new","allow-from-bytes","allow-from-path","allow-rgba","allow-size"]},"permissions":{"allow-from-bytes":{"identifier":"allow-from-bytes","description":"Enables the from_bytes command without any pre-configured scope.","commands":{"allow":["from_bytes"],"deny":[]}},"allow-from-path":{"identifier":"allow-from-path","description":"Enables the from_path command without any pre-configured scope.","commands":{"allow":["from_path"],"deny":[]}},"allow-new":{"identifier":"allow-new","description":"Enables the new command without any pre-configured scope.","commands":{"allow":["new"],"deny":[]}},"allow-rgba":{"identifier":"allow-rgba","description":"Enables the rgba command without any pre-configured scope.","commands":{"allow":["rgba"],"deny":[]}},"allow-size":{"identifier":"allow-size","description":"Enables the size command without any pre-configured scope.","commands":{"allow":["size"],"deny":[]}},"deny-from-bytes":{"identifier":"deny-from-bytes","description":"Denies the from_bytes command without any pre-configured scope.","commands":{"allow":[],"deny":["from_bytes"]}},"deny-from-path":{"identifier":"deny-from-path","description":"Denies the from_path command without any pre-configured scope.","commands":{"allow":[],"deny":["from_path"]}},"deny-new":{"identifier":"deny-new","description":"Denies the new command without any pre-configured scope.","commands":{"allow":[],"deny":["new"]}},"deny-rgba":{"identifier":"deny-rgba","description":"Denies the rgba command without any pre-configured scope.","commands":{"allow":[],"deny":["rgba"]}},"deny-size":{"identifier":"deny-size","description":"Denies the size command without any pre-configured scope.","commands":{"allow":[],"deny":["size"]}}},"permission_sets":{},"global_scope_schema":null},"core:menu":{"default_permission":{"identifier":"default","description":"Default permissions for the plugin, which enables all commands.","permissions":["allow-new","allow-append","allow-prepend","allow-insert","allow-remove","allow-remove-at","allow-items","allow-get","allow-popup","allow-create-default","allow-set-as-app-menu","allow-set-as-window-menu","allow-text","allow-set-text","allow-is-enabled","allow-set-enabled","allow-set-accelerator","allow-set-as-windows-menu-for-nsapp","allow-set-as-help-menu-for-nsapp","allow-is-checked","allow-set-checked","allow-set-icon"]},"permissions":{"allow-append":{"identifier":"allow-append","description":"Enables the append command without any pre-configured scope.","commands":{"allow":["append"],"deny":[]}},"allow-create-default":{"identifier":"allow-create-default","description":"Enables the create_default command without any pre-configured scope.","commands":{"allow":["create_default"],"deny":[]}},"allow-get":{"identifier":"allow-get","description":"Enables the get command without any pre-configured scope.","commands":{"allow":["get"],"deny":[]}},"allow-insert":{"identifier":"allow-insert","description":"Enables the insert command without any pre-configured scope.","commands":{"allow":["insert"],"deny":[]}},"allow-is-checked":{"identifier":"allow-is-checked","description":"Enables the is_checked command without any pre-configured scope.","commands":{"allow":["is_checked"],"deny":[]}},"allow-is-enabled":{"identifier":"allow-is-enabled","description":"Enables the is_enabled command without any pre-configured scope.","commands":{"allow":["is_enabled"],"deny":[]}},"allow-items":{"identifier":"allow-items","description":"Enables the items command without any pre-configured scope.","commands":{"allow":["items"],"deny":[]}},"allow-new":{"identifier":"allow-new","description":"Enables the new command without any pre-configured scope.","commands":{"allow":["new"],"deny":[]}},"allow-popup":{"identifier":"allow-popup","description":"Enables the popup command without any pre-configured scope.","commands":{"allow":["popup"],"deny":[]}},"allow-prepend":{"identifier":"allow-prepend","description":"Enables the prepend command without any pre-configured scope.","commands":{"allow":["prepend"],"deny":[]}},"allow-remove":{"identifier":"allow-remove","description":"Enables the remove command without any pre-configured scope.","commands":{"allow":["remove"],"deny":[]}},"allow-remove-at":{"identifier":"allow-remove-at","description":"Enables the remove_at command without any pre-configured scope.","commands":{"allow":["remove_at"],"deny":[]}},"allow-set-accelerator":{"identifier":"allow-set-accelerator","description":"Enables the set_accelerator command without any pre-configured scope.","commands":{"allow":["set_accelerator"],"deny":[]}},"allow-set-as-app-menu":{"identifier":"allow-set-as-app-menu","description":"Enables the set_as_app_menu command without any pre-configured scope.","commands":{"allow":["set_as_app_menu"],"deny":[]}},"allow-set-as-help-menu-for-nsapp":{"identifier":"allow-set-as-help-menu-for-nsapp","description":"Enables the set_as_help_menu_for_nsapp command without any pre-configured scope.","commands":{"allow":["set_as_help_menu_for_nsapp"],"deny":[]}},"allow-set-as-window-menu":{"identifier":"allow-set-as-window-menu","description":"Enables the set_as_window_menu command without any pre-configured scope.","commands":{"allow":["set_as_window_menu"],"deny":[]}},"allow-set-as-windows-menu-for-nsapp":{"identifier":"allow-set-as-windows-menu-for-nsapp","description":"Enables the set_as_windows_menu_for_nsapp command without any pre-configured scope.","commands":{"allow":["set_as_windows_menu_for_nsapp"],"deny":[]}},"allow-set-checked":{"identifier":"allow-set-checked","description":"Enables the set_checked command without any pre-configured scope.","commands":{"allow":["set_checked"],"deny":[]}},"allow-set-enabled":{"identifier":"allow-set-enabled","description":"Enables the set_enabled command without any pre-configured scope.","commands":{"allow":["set_enabled"],"deny":[]}},"allow-set-icon":{"identifier":"allow-set-icon","description":"Enables the set_icon command without any pre-configured scope.","commands":{"allow":["set_icon"],"deny":[]}},"allow-set-text":{"identifier":"allow-set-text","description":"Enables the set_text command without any pre-configured scope.","commands":{"allow":["set_text"],"deny":[]}},"allow-text":{"identifier":"allow-text","description":"Enables the text command without any pre-configured scope.","commands":{"allow":["text"],"deny":[]}},"deny-append":{"identifier":"deny-append","description":"Denies the append command without any pre-configured scope.","commands":{"allow":[],"deny":["append"]}},"deny-create-default":{"identifier":"deny-create-default","description":"Denies the create_default command without any pre-configured scope.","commands":{"allow":[],"deny":["create_default"]}},"deny-get":{"identifier":"deny-get","description":"Denies the get command without any pre-configured scope.","commands":{"allow":[],"deny":["get"]}},"deny-insert":{"identifier":"deny-insert","description":"Denies the insert command without any pre-configured scope.","commands":{"allow":[],"deny":["insert"]}},"deny-is-checked":{"identifier":"deny-is-checked","description":"Denies the is_checked command without any pre-configured scope.","commands":{"allow":[],"deny":["is_checked"]}},"deny-is-enabled":{"identifier":"deny-is-enabled","description":"Denies the is_enabled command without any pre-configured scope.","commands":{"allow":[],"deny":["is_enabled"]}},"deny-items":{"identifier":"deny-items","description":"Denies the items command without any pre-configured scope.","commands":{"allow":[],"deny":["items"]}},"deny-new":{"identifier":"deny-new","description":"Denies the new command without any pre-configured scope.","commands":{"allow":[],"deny":["new"]}},"deny-popup":{"identifier":"deny-popup","description":"Denies the popup command without any pre-configured scope.","commands":{"allow":[],"deny":["popup"]}},"deny-prepend":{"identifier":"deny-prepend","description":"Denies the prepend command without any pre-configured scope.","commands":{"allow":[],"deny":["prepend"]}},"deny-remove":{"identifier":"deny-remove","description":"Denies the remove command without any pre-configured scope.","commands":{"allow":[],"deny":["remove"]}},"deny-remove-at":{"identifier":"deny-remove-at","description":"Denies the remove_at command without any pre-configured scope.","commands":{"allow":[],"deny":["remove_at"]}},"deny-set-accelerator":{"identifier":"deny-set-accelerator","description":"Denies the set_accelerator command without any pre-configured scope.","commands":{"allow":[],"deny":["set_accelerator"]}},"deny-set-as-app-menu":{"identifier":"deny-set-as-app-menu","description":"Denies the set_as_app_menu command without any pre-configured scope.","commands":{"allow":[],"deny":["set_as_app_menu"]}},"deny-set-as-help-menu-for-nsapp":{"identifier":"deny-set-as-help-menu-for-nsapp","description":"Denies the set_as_help_menu_for_nsapp command without any pre-configured scope.","commands":{"allow":[],"deny":["set_as_help_menu_for_nsapp"]}},"deny-set-as-window-menu":{"identifier":"deny-set-as-window-menu","description":"Denies the set_as_window_menu command without any pre-configured scope.","commands":{"allow":[],"deny":["set_as_window_menu"]}},"deny-set-as-windows-menu-for-nsapp":{"identifier":"deny-set-as-windows-menu-for-nsapp","description":"Denies the set_as_windows_menu_for_nsapp command without any pre-configured scope.","commands":{"allow":[],"deny":["set_as_windows_menu_for_nsapp"]}},"deny-set-checked":{"identifier":"deny-set-checked","description":"Denies the set_checked command without any pre-configured scope.","commands":{"allow":[],"deny":["set_checked"]}},"deny-set-enabled":{"identifier":"deny-set-enabled","description":"Denies the set_enabled command without any pre-configured scope.","commands":{"allow":[],"deny":["set_enabled"]}},"deny-set-icon":{"identifier":"deny-set-icon","description":"Denies the set_icon command without any pre-configured scope.","commands":{"allow":[],"deny":["set_icon"]}},"deny-set-text":{"identifier":"deny-set-text","description":"Denies the set_text command without any pre-configured scope.","commands":{"allow":[],"deny":["set_text"]}},"deny-text":{"identifier":"deny-text","description":"Denies the text command without any pre-configured scope.","commands":{"allow":[],"deny":["text"]}}},"permission_sets":{},"global_scope_schema":null},"core:path":{"default_permission":{"identifier":"default","description":"Default permissions for the plugin, which enables all commands.","permissions":["allow-resolve-directory","allow-resolve","allow-normalize","allow-join","allow-dirname","allow-extname","allow-basename","allow-is-absolute"]},"permissions":{"allow-basename":{"identifier":"allow-basename","description":"Enables the basename command without any pre-configured scope.","commands":{"allow":["basename"],"deny":[]}},"allow-dirname":{"identifier":"allow-dirname","description":"Enables the dirname command without any pre-configured scope.","commands":{"allow":["dirname"],"deny":[]}},"allow-extname":{"identifier":"allow-extname","description":"Enables the extname command without any pre-configured scope.","commands":{"allow":["extname"],"deny":[]}},"allow-is-absolute":{"identifier":"allow-is-absolute","description":"Enables the is_absolute command without any pre-configured scope.","commands":{"allow":["is_absolute"],"deny":[]}},"allow-join":{"identifier":"allow-join","description":"Enables the join command without any pre-configured scope.","commands":{"allow":["join"],"deny":[]}},"allow-normalize":{"identifier":"allow-normalize","description":"Enables the normalize command without any pre-configured scope.","commands":{"allow":["normalize"],"deny":[]}},"allow-resolve":{"identifier":"allow-resolve","description":"Enables the resolve command without any pre-configured scope.","commands":{"allow":["resolve"],"deny":[]}},"allow-resolve-directory":{"identifier":"allow-resolve-directory","description":"Enables the resolve_directory command without any pre-configured scope.","commands":{"allow":["resolve_directory"],"deny":[]}},"deny-basename":{"identifier":"deny-basename","description":"Denies the basename command without any pre-configured scope.","commands":{"allow":[],"deny":["basename"]}},"deny-dirname":{"identifier":"deny-dirname","description":"Denies the dirname command without any pre-configured scope.","commands":{"allow":[],"deny":["dirname"]}},"deny-extname":{"identifier":"deny-extname","description":"Denies the extname command without any pre-configured scope.","commands":{"allow":[],"deny":["extname"]}},"deny-is-absolute":{"identifier":"deny-is-absolute","description":"Denies the is_absolute command without any pre-configured scope.","commands":{"allow":[],"deny":["is_absolute"]}},"deny-join":{"identifier":"deny-join","description":"Denies the join command without any pre-configured scope.","commands":{"allow":[],"deny":["join"]}},"deny-normalize":{"identifier":"deny-normalize","description":"Denies the normalize command without any pre-configured scope.","commands":{"allow":[],"deny":["normalize"]}},"deny-resolve":{"identifier":"deny-resolve","description":"Denies the resolve command without any pre-configured scope.","commands":{"allow":[],"deny":["resolve"]}},"deny-resolve-directory":{"identifier":"deny-resolve-directory","description":"Denies the resolve_directory command without any pre-configured scope.","commands":{"allow":[],"deny":["resolve_directory"]}}},"permission_sets":{},"global_scope_schema":null},"core:resources":{"default_permission":{"identifier":"default","description":"Default permissions for the plugin, which enables all commands.","permissions":["allow-close"]},"permissions":{"allow-close":{"identifier":"allow-close","description":"Enables the close command without any pre-configured scope.","commands":{"allow":["close"],"deny":[]}},"deny-close":{"identifier":"deny-close","description":"Denies the close command without any pre-configured scope.","commands":{"allow":[],"deny":["close"]}}},"permission_sets":{},"global_scope_schema":null},"core:tray":{"default_permission":{"identifier":"default","description":"Default permissions for the plugin, which enables all commands.","permissions":["allow-new","allow-get-by-id","allow-remove-by-id","allow-set-icon","allow-set-menu","allow-set-tooltip","allow-set-title","allow-set-visible","allow-set-temp-dir-path","allow-set-icon-as-template","allow-set-icon-with-as-template","allow-set-show-menu-on-left-click"]},"permissions":{"allow-get-by-id":{"identifier":"allow-get-by-id","description":"Enables the get_by_id command without any pre-configured scope.","commands":{"allow":["get_by_id"],"deny":[]}},"allow-new":{"identifier":"allow-new","description":"Enables the new command without any pre-configured scope.","commands":{"allow":["new"],"deny":[]}},"allow-remove-by-id":{"identifier":"allow-remove-by-id","description":"Enables the remove_by_id command without any pre-configured scope.","commands":{"allow":["remove_by_id"],"deny":[]}},"allow-set-icon":{"identifier":"allow-set-icon","description":"Enables the set_icon command without any pre-configured scope.","commands":{"allow":["set_icon"],"deny":[]}},"allow-set-icon-as-template":{"identifier":"allow-set-icon-as-template","description":"Enables the set_icon_as_template command without any pre-configured scope.","commands":{"allow":["set_icon_as_template"],"deny":[]}},"allow-set-icon-with-as-template":{"identifier":"allow-set-icon-with-as-template","description":"Enables the set_icon_with_as_template command without any pre-configured scope.","commands":{"allow":["set_icon_with_as_template"],"deny":[]}},"allow-set-menu":{"identifier":"allow-set-menu","description":"Enables the set_menu command without any pre-configured scope.","commands":{"allow":["set_menu"],"deny":[]}},"allow-set-show-menu-on-left-click":{"identifier":"allow-set-show-menu-on-left-click","description":"Enables the set_show_menu_on_left_click command without any pre-configured scope.","commands":{"allow":["set_show_menu_on_left_click"],"deny":[]}},"allow-set-temp-dir-path":{"identifier":"allow-set-temp-dir-path","description":"Enables the set_temp_dir_path command without any pre-configured scope.","commands":{"allow":["set_temp_dir_path"],"deny":[]}},"allow-set-title":{"identifier":"allow-set-title","description":"Enables the set_title command without any pre-configured scope.","commands":{"allow":["set_title"],"deny":[]}},"allow-set-tooltip":{"identifier":"allow-set-tooltip","description":"Enables the set_tooltip command without any pre-configured scope.","commands":{"allow":["set_tooltip"],"deny":[]}},"allow-set-visible":{"identifier":"allow-set-visible","description":"Enables the set_visible command without any pre-configured scope.","commands":{"allow":["set_visible"],"deny":[]}},"deny-get-by-id":{"identifier":"deny-get-by-id","description":"Denies the get_by_id command without any pre-configured scope.","commands":{"allow":[],"deny":["get_by_id"]}},"deny-new":{"identifier":"deny-new","description":"Denies the new command without any pre-configured scope.","commands":{"allow":[],"deny":["new"]}},"deny-remove-by-id":{"identifier":"deny-remove-by-id","description":"Denies the remove_by_id command without any pre-configured scope.","commands":{"allow":[],"deny":["remove_by_id"]}},"deny-set-icon":{"identifier":"deny-set-icon","description":"Denies the set_icon command without any pre-configured scope.","commands":{"allow":[],"deny":["set_icon"]}},"deny-set-icon-as-template":{"identifier":"deny-set-icon-as-template","description":"Denies the set_icon_as_template command without any pre-configured scope.","commands":{"allow":[],"deny":["set_icon_as_template"]}},"deny-set-icon-with-as-template":{"identifier":"deny-set-icon-with-as-template","description":"Denies the set_icon_with_as_template command without any pre-configured scope.","commands":{"allow":[],"deny":["set_icon_with_as_template"]}},"deny-set-menu":{"identifier":"deny-set-menu","description":"Denies the set_menu command without any pre-configured scope.","commands":{"allow":[],"deny":["set_menu"]}},"deny-set-show-menu-on-left-click":{"identifier":"deny-set-show-menu-on-left-click","description":"Denies the set_show_menu_on_left_click command without any pre-configured scope.","commands":{"allow":[],"deny":["set_show_menu_on_left_click"]}},"deny-set-temp-dir-path":{"identifier":"deny-set-temp-dir-path","description":"Denies the set_temp_dir_path command without any pre-configured scope.","commands":{"allow":[],"deny":["set_temp_dir_path"]}},"deny-set-title":{"identifier":"deny-set-title","description":"Denies the set_title command without any pre-configured scope.","commands":{"allow":[],"deny":["set_title"]}},"deny-set-tooltip":{"identifier":"deny-set-tooltip","description":"Denies the set_tooltip command without any pre-configured scope.","commands":{"allow":[],"deny":["set_tooltip"]}},"deny-set-visible":{"identifier":"deny-set-visible","description":"Denies the set_visible command without any pre-configured scope.","commands":{"allow":[],"deny":["set_visible"]}}},"permission_sets":{},"global_scope_schema":null},"core:webview":{"default_permission":{"identifier":"default","description":"Default permissions for the plugin.","permissions":["allow-get-all-webviews","allow-webview-position","allow-webview-size","allow-internal-toggle-devtools"]},"permissions":{"allow-clear-all-browsing-data":{"identifier":"allow-clear-all-browsing-data","description":"Enables the clear_all_browsing_data command without any pre-configured scope.","commands":{"allow":["clear_all_browsing_data"],"deny":[]}},"allow-create-webview":{"identifier":"allow-create-webview","description":"Enables the create_webview command without any pre-configured scope.","commands":{"allow":["create_webview"],"deny":[]}},"allow-create-webview-window":{"identifier":"allow-create-webview-window","description":"Enables the create_webview_window command without any pre-configured scope.","commands":{"allow":["create_webview_window"],"deny":[]}},"allow-get-all-webviews":{"identifier":"allow-get-all-webviews","description":"Enables the get_all_webviews command without any pre-configured scope.","commands":{"allow":["get_all_webviews"],"deny":[]}},"allow-internal-toggle-devtools":{"identifier":"allow-internal-toggle-devtools","description":"Enables the internal_toggle_devtools command without any pre-configured scope.","commands":{"allow":["internal_toggle_devtools"],"deny":[]}},"allow-print":{"identifier":"allow-print","description":"Enables the print command without any pre-configured scope.","commands":{"allow":["print"],"deny":[]}},"allow-reparent":{"identifier":"allow-reparent","description":"Enables the reparent command without any pre-configured scope.","commands":{"allow":["reparent"],"deny":[]}},"allow-set-webview-auto-resize":{"identifier":"allow-set-webview-auto-resize","description":"Enables the set_webview_auto_resize command without any pre-configured scope.","commands":{"allow":["set_webview_auto_resize"],"deny":[]}},"allow-set-webview-background-color":{"identifier":"allow-set-webview-background-color","description":"Enables the set_webview_background_color command without any pre-configured scope.","commands":{"allow":["set_webview_background_color"],"deny":[]}},"allow-set-webview-focus":{"identifier":"allow-set-webview-focus","description":"Enables the set_webview_focus command without any pre-configured scope.","commands":{"allow":["set_webview_focus"],"deny":[]}},"allow-set-webview-position":{"identifier":"allow-set-webview-position","description":"Enables the set_webview_position command without any pre-configured scope.","commands":{"allow":["set_webview_position"],"deny":[]}},"allow-set-webview-size":{"identifier":"allow-set-webview-size","description":"Enables the set_webview_size command without any pre-configured scope.","commands":{"allow":["set_webview_size"],"deny":[]}},"allow-set-webview-zoom":{"identifier":"allow-set-webview-zoom","description":"Enables the set_webview_zoom command without any pre-configured scope.","commands":{"allow":["set_webview_zoom"],"deny":[]}},"allow-webview-close":{"identifier":"allow-webview-close","description":"Enables the webview_close command without any pre-configured scope.","commands":{"allow":["webview_close"],"deny":[]}},"allow-webview-hide":{"identifier":"allow-webview-hide","description":"Enables the webview_hide command without any pre-configured scope.","commands":{"allow":["webview_hide"],"deny":[]}},"allow-webview-position":{"identifier":"allow-webview-position","description":"Enables the webview_position command without any pre-configured scope.","commands":{"allow":["webview_position"],"deny":[]}},"allow-webview-show":{"identifier":"allow-webview-show","description":"Enables the webview_show command without any pre-configured scope.","commands":{"allow":["webview_show"],"deny":[]}},"allow-webview-size":{"identifier":"allow-webview-size","description":"Enables the webview_size command without any pre-configured scope.","commands":{"allow":["webview_size"],"deny":[]}},"deny-clear-all-browsing-data":{"identifier":"deny-clear-all-browsing-data","description":"Denies the clear_all_browsing_data command without any pre-configured scope.","commands":{"allow":[],"deny":["clear_all_browsing_data"]}},"deny-create-webview":{"identifier":"deny-create-webview","description":"Denies the create_webview command without any pre-configured scope.","commands":{"allow":[],"deny":["create_webview"]}},"deny-create-webview-window":{"identifier":"deny-create-webview-window","description":"Denies the create_webview_window command without any pre-configured scope.","commands":{"allow":[],"deny":["create_webview_window"]}},"deny-get-all-webviews":{"identifier":"deny-get-all-webviews","description":"Denies the get_all_webviews command without any pre-configured scope.","commands":{"allow":[],"deny":["get_all_webviews"]}},"deny-internal-toggle-devtools":{"identifier":"deny-internal-toggle-devtools","description":"Denies the internal_toggle_devtools command without any pre-configured scope.","commands":{"allow":[],"deny":["internal_toggle_devtools"]}},"deny-print":{"identifier":"deny-print","description":"Denies the print command without any pre-configured scope.","commands":{"allow":[],"deny":["print"]}},"deny-reparent":{"identifier":"deny-reparent","description":"Denies the reparent command without any pre-configured scope.","commands":{"allow":[],"deny":["reparent"]}},"deny-set-webview-auto-resize":{"identifier":"deny-set-webview-auto-resize","description":"Denies the set_webview_auto_resize command without any pre-configured scope.","commands":{"allow":[],"deny":["set_webview_auto_resize"]}},"deny-set-webview-background-color":{"identifier":"deny-set-webview-background-color","description":"Denies the set_webview_background_color command without any pre-configured scope.","commands":{"allow":[],"deny":["set_webview_background_color"]}},"deny-set-webview-focus":{"identifier":"deny-set-webview-focus","description":"Denies the set_webview_focus command without any pre-configured scope.","commands":{"allow":[],"deny":["set_webview_focus"]}},"deny-set-webview-position":{"identifier":"deny-set-webview-position","description":"Denies the set_webview_position command without any pre-configured scope.","commands":{"allow":[],"deny":["set_webview_position"]}},"deny-set-webview-size":{"identifier":"deny-set-webview-size","description":"Denies the set_webview_size command without any pre-configured scope.","commands":{"allow":[],"deny":["set_webview_size"]}},"deny-set-webview-zoom":{"identifier":"deny-set-webview-zoom","description":"Denies the set_webview_zoom command without any pre-configured scope.","commands":{"allow":[],"deny":["set_webview_zoom"]}},"deny-webview-close":{"identifier":"deny-webview-close","description":"Denies the webview_close command without any pre-configured scope.","commands":{"allow":[],"deny":["webview_close"]}},"deny-webview-hide":{"identifier":"deny-webview-hide","description":"Denies the webview_hide command without any pre-configured scope.","commands":{"allow":[],"deny":["webview_hide"]}},"deny-webview-position":{"identifier":"deny-webview-position","description":"Denies the webview_position command without any pre-configured scope.","commands":{"allow":[],"deny":["webview_position"]}},"deny-webview-show":{"identifier":"deny-webview-show","description":"Denies the webview_show command without any pre-configured scope.","commands":{"allow":[],"deny":["webview_show"]}},"deny-webview-size":{"identifier":"deny-webview-size","description":"Denies the webview_size command without any pre-configured scope.","commands":{"allow":[],"deny":["webview_size"]}}},"permission_sets":{},"global_scope_schema":null},"core:window":{"default_permission":{"identifier":"default","description":"Default permissions for the plugin.","permissions":["allow-get-all-windows","allow-scale-factor","allow-inner-position","allow-outer-position","allow-inner-size","allow-outer-size","allow-is-fullscreen","allow-is-minimized","allow-is-maximized","allow-is-focused","allow-is-decorated","allow-is-resizable","allow-is-maximizable","allow-is-minimizable","allow-is-closable","allow-is-visible","allow-is-enabled","allow-title","allow-current-monitor","allow-primary-monitor","allow-monitor-from-point","allow-available-monitors","allow-cursor-position","allow-theme","allow-is-always-on-top","allow-activity-name","allow-scene-identifier","allow-internal-toggle-maximize"]},"permissions":{"allow-activity-name":{"identifier":"allow-activity-name","description":"Enables the activity_name command without any pre-configured scope.","commands":{"allow":["activity_name"],"deny":[]}},"allow-available-monitors":{"identifier":"allow-available-monitors","description":"Enables the available_monitors command without any pre-configured scope.","commands":{"allow":["available_monitors"],"deny":[]}},"allow-center":{"identifier":"allow-center","description":"Enables the center command without any pre-configured scope.","commands":{"allow":["center"],"deny":[]}},"allow-close":{"identifier":"allow-close","description":"Enables the close command without any pre-configured scope.","commands":{"allow":["close"],"deny":[]}},"allow-create":{"identifier":"allow-create","description":"Enables the create command without any pre-configured scope.","commands":{"allow":["create"],"deny":[]}},"allow-current-monitor":{"identifier":"allow-current-monitor","description":"Enables the current_monitor command without any pre-configured scope.","commands":{"allow":["current_monitor"],"deny":[]}},"allow-cursor-position":{"identifier":"allow-cursor-position","description":"Enables the cursor_position command without any pre-configured scope.","commands":{"allow":["cursor_position"],"deny":[]}},"allow-destroy":{"identifier":"allow-destroy","description":"Enables the destroy command without any pre-configured scope.","commands":{"allow":["destroy"],"deny":[]}},"allow-get-all-windows":{"identifier":"allow-get-all-windows","description":"Enables the get_all_windows command without any pre-configured scope.","commands":{"allow":["get_all_windows"],"deny":[]}},"allow-hide":{"identifier":"allow-hide","description":"Enables the hide command without any pre-configured scope.","commands":{"allow":["hide"],"deny":[]}},"allow-inner-position":{"identifier":"allow-inner-position","description":"Enables the inner_position command without any pre-configured scope.","commands":{"allow":["inner_position"],"deny":[]}},"allow-inner-size":{"identifier":"allow-inner-size","description":"Enables the inner_size command without any pre-configured scope.","commands":{"allow":["inner_size"],"deny":[]}},"allow-internal-toggle-maximize":{"identifier":"allow-internal-toggle-maximize","description":"Enables the internal_toggle_maximize command without any pre-configured scope.","commands":{"allow":["internal_toggle_maximize"],"deny":[]}},"allow-is-always-on-top":{"identifier":"allow-is-always-on-top","description":"Enables the is_always_on_top command without any pre-configured scope.","commands":{"allow":["is_always_on_top"],"deny":[]}},"allow-is-closable":{"identifier":"allow-is-closable","description":"Enables the is_closable command without any pre-configured scope.","commands":{"allow":["is_closable"],"deny":[]}},"allow-is-decorated":{"identifier":"allow-is-decorated","description":"Enables the is_decorated command without any pre-configured scope.","commands":{"allow":["is_decorated"],"deny":[]}},"allow-is-enabled":{"identifier":"allow-is-enabled","description":"Enables the is_enabled command without any pre-configured scope.","commands":{"allow":["is_enabled"],"deny":[]}},"allow-is-focused":{"identifier":"allow-is-focused","description":"Enables the is_focused command without any pre-configured scope.","commands":{"allow":["is_focused"],"deny":[]}},"allow-is-fullscreen":{"identifier":"allow-is-fullscreen","description":"Enables the is_fullscreen command without any pre-configured scope.","commands":{"allow":["is_fullscreen"],"deny":[]}},"allow-is-maximizable":{"identifier":"allow-is-maximizable","description":"Enables the is_maximizable command without any pre-configured scope.","commands":{"allow":["is_maximizable"],"deny":[]}},"allow-is-maximized":{"identifier":"allow-is-maximized","description":"Enables the is_maximized command without any pre-configured scope.","commands":{"allow":["is_maximized"],"deny":[]}},"allow-is-minimizable":{"identifier":"allow-is-minimizable","description":"Enables the is_minimizable command without any pre-configured scope.","commands":{"allow":["is_minimizable"],"deny":[]}},"allow-is-minimized":{"identifier":"allow-is-minimized","description":"Enables the is_minimized command without any pre-configured scope.","commands":{"allow":["is_minimized"],"deny":[]}},"allow-is-resizable":{"identifier":"allow-is-resizable","description":"Enables the is_resizable command without any pre-configured scope.","commands":{"allow":["is_resizable"],"deny":[]}},"allow-is-visible":{"identifier":"allow-is-visible","description":"Enables the is_visible command without any pre-configured scope.","commands":{"allow":["is_visible"],"deny":[]}},"allow-maximize":{"identifier":"allow-maximize","description":"Enables the maximize command without any pre-configured scope.","commands":{"allow":["maximize"],"deny":[]}},"allow-minimize":{"identifier":"allow-minimize","description":"Enables the minimize command without any pre-configured scope.","commands":{"allow":["minimize"],"deny":[]}},"allow-monitor-from-point":{"identifier":"allow-monitor-from-point","description":"Enables the monitor_from_point command without any pre-configured scope.","commands":{"allow":["monitor_from_point"],"deny":[]}},"allow-outer-position":{"identifier":"allow-outer-position","description":"Enables the outer_position command without any pre-configured scope.","commands":{"allow":["outer_position"],"deny":[]}},"allow-outer-size":{"identifier":"allow-outer-size","description":"Enables the outer_size command without any pre-configured scope.","commands":{"allow":["outer_size"],"deny":[]}},"allow-primary-monitor":{"identifier":"allow-primary-monitor","description":"Enables the primary_monitor command without any pre-configured scope.","commands":{"allow":["primary_monitor"],"deny":[]}},"allow-request-user-attention":{"identifier":"allow-request-user-attention","description":"Enables the request_user_attention command without any pre-configured scope.","commands":{"allow":["request_user_attention"],"deny":[]}},"allow-scale-factor":{"identifier":"allow-scale-factor","description":"Enables the scale_factor command without any pre-configured scope.","commands":{"allow":["scale_factor"],"deny":[]}},"allow-scene-identifier":{"identifier":"allow-scene-identifier","description":"Enables the scene_identifier command without any pre-configured scope.","commands":{"allow":["scene_identifier"],"deny":[]}},"allow-set-always-on-bottom":{"identifier":"allow-set-always-on-bottom","description":"Enables the set_always_on_bottom command without any pre-configured scope.","commands":{"allow":["set_always_on_bottom"],"deny":[]}},"allow-set-always-on-top":{"identifier":"allow-set-always-on-top","description":"Enables the set_always_on_top command without any pre-configured scope.","commands":{"allow":["set_always_on_top"],"deny":[]}},"allow-set-background-color":{"identifier":"allow-set-background-color","description":"Enables the set_background_color command without any pre-configured scope.","commands":{"allow":["set_background_color"],"deny":[]}},"allow-set-badge-count":{"identifier":"allow-set-badge-count","description":"Enables the set_badge_count command without any pre-configured scope.","commands":{"allow":["set_badge_count"],"deny":[]}},"allow-set-badge-label":{"identifier":"allow-set-badge-label","description":"Enables the set_badge_label command without any pre-configured scope.","commands":{"allow":["set_badge_label"],"deny":[]}},"allow-set-closable":{"identifier":"allow-set-closable","description":"Enables the set_closable command without any pre-configured scope.","commands":{"allow":["set_closable"],"deny":[]}},"allow-set-content-protected":{"identifier":"allow-set-content-protected","description":"Enables the set_content_protected command without any pre-configured scope.","commands":{"allow":["set_content_protected"],"deny":[]}},"allow-set-cursor-grab":{"identifier":"allow-set-cursor-grab","description":"Enables the set_cursor_grab command without any pre-configured scope.","commands":{"allow":["set_cursor_grab"],"deny":[]}},"allow-set-cursor-icon":{"identifier":"allow-set-cursor-icon","description":"Enables the set_cursor_icon command without any pre-configured scope.","commands":{"allow":["set_cursor_icon"],"deny":[]}},"allow-set-cursor-position":{"identifier":"allow-set-cursor-position","description":"Enables the set_cursor_position command without any pre-configured scope.","commands":{"allow":["set_cursor_position"],"deny":[]}},"allow-set-cursor-visible":{"identifier":"allow-set-cursor-visible","description":"Enables the set_cursor_visible command without any pre-configured scope.","commands":{"allow":["set_cursor_visible"],"deny":[]}},"allow-set-decorations":{"identifier":"allow-set-decorations","description":"Enables the set_decorations command without any pre-configured scope.","commands":{"allow":["set_decorations"],"deny":[]}},"allow-set-effects":{"identifier":"allow-set-effects","description":"Enables the set_effects command without any pre-configured scope.","commands":{"allow":["set_effects"],"deny":[]}},"allow-set-enabled":{"identifier":"allow-set-enabled","description":"Enables the set_enabled command without any pre-configured scope.","commands":{"allow":["set_enabled"],"deny":[]}},"allow-set-focus":{"identifier":"allow-set-focus","description":"Enables the set_focus command without any pre-configured scope.","commands":{"allow":["set_focus"],"deny":[]}},"allow-set-focusable":{"identifier":"allow-set-focusable","description":"Enables the set_focusable command without any pre-configured scope.","commands":{"allow":["set_focusable"],"deny":[]}},"allow-set-fullscreen":{"identifier":"allow-set-fullscreen","description":"Enables the set_fullscreen command without any pre-configured scope.","commands":{"allow":["set_fullscreen"],"deny":[]}},"allow-set-fullscreen-on-monitor":{"identifier":"allow-set-fullscreen-on-monitor","description":"Enables the set_fullscreen_on_monitor command without any pre-configured scope.","commands":{"allow":["set_fullscreen_on_monitor"],"deny":[]}},"allow-set-icon":{"identifier":"allow-set-icon","description":"Enables the set_icon command without any pre-configured scope.","commands":{"allow":["set_icon"],"deny":[]}},"allow-set-ignore-cursor-events":{"identifier":"allow-set-ignore-cursor-events","description":"Enables the set_ignore_cursor_events command without any pre-configured scope.","commands":{"allow":["set_ignore_cursor_events"],"deny":[]}},"allow-set-max-size":{"identifier":"allow-set-max-size","description":"Enables the set_max_size command without any pre-configured scope.","commands":{"allow":["set_max_size"],"deny":[]}},"allow-set-maximizable":{"identifier":"allow-set-maximizable","description":"Enables the set_maximizable command without any pre-configured scope.","commands":{"allow":["set_maximizable"],"deny":[]}},"allow-set-min-size":{"identifier":"allow-set-min-size","description":"Enables the set_min_size command without any pre-configured scope.","commands":{"allow":["set_min_size"],"deny":[]}},"allow-set-minimizable":{"identifier":"allow-set-minimizable","description":"Enables the set_minimizable command without any pre-configured scope.","commands":{"allow":["set_minimizable"],"deny":[]}},"allow-set-overlay-icon":{"identifier":"allow-set-overlay-icon","description":"Enables the set_overlay_icon command without any pre-configured scope.","commands":{"allow":["set_overlay_icon"],"deny":[]}},"allow-set-position":{"identifier":"allow-set-position","description":"Enables the set_position command without any pre-configured scope.","commands":{"allow":["set_position"],"deny":[]}},"allow-set-progress-bar":{"identifier":"allow-set-progress-bar","description":"Enables the set_progress_bar command without any pre-configured scope.","commands":{"allow":["set_progress_bar"],"deny":[]}},"allow-set-resizable":{"identifier":"allow-set-resizable","description":"Enables the set_resizable command without any pre-configured scope.","commands":{"allow":["set_resizable"],"deny":[]}},"allow-set-shadow":{"identifier":"allow-set-shadow","description":"Enables the set_shadow command without any pre-configured scope.","commands":{"allow":["set_shadow"],"deny":[]}},"allow-set-simple-fullscreen":{"identifier":"allow-set-simple-fullscreen","description":"Enables the set_simple_fullscreen command without any pre-configured scope.","commands":{"allow":["set_simple_fullscreen"],"deny":[]}},"allow-set-size":{"identifier":"allow-set-size","description":"Enables the set_size command without any pre-configured scope.","commands":{"allow":["set_size"],"deny":[]}},"allow-set-size-constraints":{"identifier":"allow-set-size-constraints","description":"Enables the set_size_constraints command without any pre-configured scope.","commands":{"allow":["set_size_constraints"],"deny":[]}},"allow-set-skip-taskbar":{"identifier":"allow-set-skip-taskbar","description":"Enables the set_skip_taskbar command without any pre-configured scope.","commands":{"allow":["set_skip_taskbar"],"deny":[]}},"allow-set-theme":{"identifier":"allow-set-theme","description":"Enables the set_theme command without any pre-configured scope.","commands":{"allow":["set_theme"],"deny":[]}},"allow-set-title":{"identifier":"allow-set-title","description":"Enables the set_title command without any pre-configured scope.","commands":{"allow":["set_title"],"deny":[]}},"allow-set-title-bar-style":{"identifier":"allow-set-title-bar-style","description":"Enables the set_title_bar_style command without any pre-configured scope.","commands":{"allow":["set_title_bar_style"],"deny":[]}},"allow-set-visible-on-all-workspaces":{"identifier":"allow-set-visible-on-all-workspaces","description":"Enables the set_visible_on_all_workspaces command without any pre-configured scope.","commands":{"allow":["set_visible_on_all_workspaces"],"deny":[]}},"allow-show":{"identifier":"allow-show","description":"Enables the show command without any pre-configured scope.","commands":{"allow":["show"],"deny":[]}},"allow-start-dragging":{"identifier":"allow-start-dragging","description":"Enables the start_dragging command without any pre-configured scope.","commands":{"allow":["start_dragging"],"deny":[]}},"allow-start-resize-dragging":{"identifier":"allow-start-resize-dragging","description":"Enables the start_resize_dragging command without any pre-configured scope.","commands":{"allow":["start_resize_dragging"],"deny":[]}},"allow-theme":{"identifier":"allow-theme","description":"Enables the theme command without any pre-configured scope.","commands":{"allow":["theme"],"deny":[]}},"allow-title":{"identifier":"allow-title","description":"Enables the title command without any pre-configured scope.","commands":{"allow":["title"],"deny":[]}},"allow-toggle-maximize":{"identifier":"allow-toggle-maximize","description":"Enables the toggle_maximize command without any pre-configured scope.","commands":{"allow":["toggle_maximize"],"deny":[]}},"allow-unmaximize":{"identifier":"allow-unmaximize","description":"Enables the unmaximize command without any pre-configured scope.","commands":{"allow":["unmaximize"],"deny":[]}},"allow-unminimize":{"identifier":"allow-unminimize","description":"Enables the unminimize command without any pre-configured scope.","commands":{"allow":["unminimize"],"deny":[]}},"deny-activity-name":{"identifier":"deny-activity-name","description":"Denies the activity_name command without any pre-configured scope.","commands":{"allow":[],"deny":["activity_name"]}},"deny-available-monitors":{"identifier":"deny-available-monitors","description":"Denies the available_monitors command without any pre-configured scope.","commands":{"allow":[],"deny":["available_monitors"]}},"deny-center":{"identifier":"deny-center","description":"Denies the center command without any pre-configured scope.","commands":{"allow":[],"deny":["center"]}},"deny-close":{"identifier":"deny-close","description":"Denies the close command without any pre-configured scope.","commands":{"allow":[],"deny":["close"]}},"deny-create":{"identifier":"deny-create","description":"Denies the create command without any pre-configured scope.","commands":{"allow":[],"deny":["create"]}},"deny-current-monitor":{"identifier":"deny-current-monitor","description":"Denies the current_monitor command without any pre-configured scope.","commands":{"allow":[],"deny":["current_monitor"]}},"deny-cursor-position":{"identifier":"deny-cursor-position","description":"Denies the cursor_position command without any pre-configured scope.","commands":{"allow":[],"deny":["cursor_position"]}},"deny-destroy":{"identifier":"deny-destroy","description":"Denies the destroy command without any pre-configured scope.","commands":{"allow":[],"deny":["destroy"]}},"deny-get-all-windows":{"identifier":"deny-get-all-windows","description":"Denies the get_all_windows command without any pre-configured scope.","commands":{"allow":[],"deny":["get_all_windows"]}},"deny-hide":{"identifier":"deny-hide","description":"Denies the hide command without any pre-configured scope.","commands":{"allow":[],"deny":["hide"]}},"deny-inner-position":{"identifier":"deny-inner-position","description":"Denies the inner_position command without any pre-configured scope.","commands":{"allow":[],"deny":["inner_position"]}},"deny-inner-size":{"identifier":"deny-inner-size","description":"Denies the inner_size command without any pre-configured scope.","commands":{"allow":[],"deny":["inner_size"]}},"deny-internal-toggle-maximize":{"identifier":"deny-internal-toggle-maximize","description":"Denies the internal_toggle_maximize command without any pre-configured scope.","commands":{"allow":[],"deny":["internal_toggle_maximize"]}},"deny-is-always-on-top":{"identifier":"deny-is-always-on-top","description":"Denies the is_always_on_top command without any pre-configured scope.","commands":{"allow":[],"deny":["is_always_on_top"]}},"deny-is-closable":{"identifier":"deny-is-closable","description":"Denies the is_closable command without any pre-configured scope.","commands":{"allow":[],"deny":["is_closable"]}},"deny-is-decorated":{"identifier":"deny-is-decorated","description":"Denies the is_decorated command without any pre-configured scope.","commands":{"allow":[],"deny":["is_decorated"]}},"deny-is-enabled":{"identifier":"deny-is-enabled","description":"Denies the is_enabled command without any pre-configured scope.","commands":{"allow":[],"deny":["is_enabled"]}},"deny-is-focused":{"identifier":"deny-is-focused","description":"Denies the is_focused command without any pre-configured scope.","commands":{"allow":[],"deny":["is_focused"]}},"deny-is-fullscreen":{"identifier":"deny-is-fullscreen","description":"Denies the is_fullscreen command without any pre-configured scope.","commands":{"allow":[],"deny":["is_fullscreen"]}},"deny-is-maximizable":{"identifier":"deny-is-maximizable","description":"Denies the is_maximizable command without any pre-configured scope.","commands":{"allow":[],"deny":["is_maximizable"]}},"deny-is-maximized":{"identifier":"deny-is-maximized","description":"Denies the is_maximized command without any pre-configured scope.","commands":{"allow":[],"deny":["is_maximized"]}},"deny-is-minimizable":{"identifier":"deny-is-minimizable","description":"Denies the is_minimizable command without any pre-configured scope.","commands":{"allow":[],"deny":["is_minimizable"]}},"deny-is-minimized":{"identifier":"deny-is-minimized","description":"Denies the is_minimized command without any pre-configured scope.","commands":{"allow":[],"deny":["is_minimized"]}},"deny-is-resizable":{"identifier":"deny-is-resizable","description":"Denies the is_resizable command without any pre-configured scope.","commands":{"allow":[],"deny":["is_resizable"]}},"deny-is-visible":{"identifier":"deny-is-visible","description":"Denies the is_visible command without any pre-configured scope.","commands":{"allow":[],"deny":["is_visible"]}},"deny-maximize":{"identifier":"deny-maximize","description":"Denies the maximize command without any pre-configured scope.","commands":{"allow":[],"deny":["maximize"]}},"deny-minimize":{"identifier":"deny-minimize","description":"Denies the minimize command without any pre-configured scope.","commands":{"allow":[],"deny":["minimize"]}},"deny-monitor-from-point":{"identifier":"deny-monitor-from-point","description":"Denies the monitor_from_point command without any pre-configured scope.","commands":{"allow":[],"deny":["monitor_from_point"]}},"deny-outer-position":{"identifier":"deny-outer-position","description":"Denies the outer_position command without any pre-configured scope.","commands":{"allow":[],"deny":["outer_position"]}},"deny-outer-size":{"identifier":"deny-outer-size","description":"Denies the outer_size command without any pre-configured scope.","commands":{"allow":[],"deny":["outer_size"]}},"deny-primary-monitor":{"identifier":"deny-primary-monitor","description":"Denies the primary_monitor command without any pre-configured scope.","commands":{"allow":[],"deny":["primary_monitor"]}},"deny-request-user-attention":{"identifier":"deny-request-user-attention","description":"Denies the request_user_attention command without any pre-configured scope.","commands":{"allow":[],"deny":["request_user_attention"]}},"deny-scale-factor":{"identifier":"deny-scale-factor","description":"Denies the scale_factor command without any pre-configured scope.","commands":{"allow":[],"deny":["scale_factor"]}},"deny-scene-identifier":{"identifier":"deny-scene-identifier","description":"Denies the scene_identifier command without any pre-configured scope.","commands":{"allow":[],"deny":["scene_identifier"]}},"deny-set-always-on-bottom":{"identifier":"deny-set-always-on-bottom","description":"Denies the set_always_on_bottom command without any pre-configured scope.","commands":{"allow":[],"deny":["set_always_on_bottom"]}},"deny-set-always-on-top":{"identifier":"deny-set-always-on-top","description":"Denies the set_always_on_top command without any pre-configured scope.","commands":{"allow":[],"deny":["set_always_on_top"]}},"deny-set-background-color":{"identifier":"deny-set-background-color","description":"Denies the set_background_color command without any pre-configured scope.","commands":{"allow":[],"deny":["set_background_color"]}},"deny-set-badge-count":{"identifier":"deny-set-badge-count","description":"Denies the set_badge_count command without any pre-configured scope.","commands":{"allow":[],"deny":["set_badge_count"]}},"deny-set-badge-label":{"identifier":"deny-set-badge-label","description":"Denies the set_badge_label command without any pre-configured scope.","commands":{"allow":[],"deny":["set_badge_label"]}},"deny-set-closable":{"identifier":"deny-set-closable","description":"Denies the set_closable command without any pre-configured scope.","commands":{"allow":[],"deny":["set_closable"]}},"deny-set-content-protected":{"identifier":"deny-set-content-protected","description":"Denies the set_content_protected command without any pre-configured scope.","commands":{"allow":[],"deny":["set_content_protected"]}},"deny-set-cursor-grab":{"identifier":"deny-set-cursor-grab","description":"Denies the set_cursor_grab command without any pre-configured scope.","commands":{"allow":[],"deny":["set_cursor_grab"]}},"deny-set-cursor-icon":{"identifier":"deny-set-cursor-icon","description":"Denies the set_cursor_icon command without any pre-configured scope.","commands":{"allow":[],"deny":["set_cursor_icon"]}},"deny-set-cursor-position":{"identifier":"deny-set-cursor-position","description":"Denies the set_cursor_position command without any pre-configured scope.","commands":{"allow":[],"deny":["set_cursor_position"]}},"deny-set-cursor-visible":{"identifier":"deny-set-cursor-visible","description":"Denies the set_cursor_visible command without any pre-configured scope.","commands":{"allow":[],"deny":["set_cursor_visible"]}},"deny-set-decorations":{"identifier":"deny-set-decorations","description":"Denies the set_decorations command without any pre-configured scope.","commands":{"allow":[],"deny":["set_decorations"]}},"deny-set-effects":{"identifier":"deny-set-effects","description":"Denies the set_effects command without any pre-configured scope.","commands":{"allow":[],"deny":["set_effects"]}},"deny-set-enabled":{"identifier":"deny-set-enabled","description":"Denies the set_enabled command without any pre-configured scope.","commands":{"allow":[],"deny":["set_enabled"]}},"deny-set-focus":{"identifier":"deny-set-focus","description":"Denies the set_focus command without any pre-configured scope.","commands":{"allow":[],"deny":["set_focus"]}},"deny-set-focusable":{"identifier":"deny-set-focusable","description":"Denies the set_focusable command without any pre-configured scope.","commands":{"allow":[],"deny":["set_focusable"]}},"deny-set-fullscreen":{"identifier":"deny-set-fullscreen","description":"Denies the set_fullscreen command without any pre-configured scope.","commands":{"allow":[],"deny":["set_fullscreen"]}},"deny-set-fullscreen-on-monitor":{"identifier":"deny-set-fullscreen-on-monitor","description":"Denies the set_fullscreen_on_monitor command without any pre-configured scope.","commands":{"allow":[],"deny":["set_fullscreen_on_monitor"]}},"deny-set-icon":{"identifier":"deny-set-icon","description":"Denies the set_icon command without any pre-configured scope.","commands":{"allow":[],"deny":["set_icon"]}},"deny-set-ignore-cursor-events":{"identifier":"deny-set-ignore-cursor-events","description":"Denies the set_ignore_cursor_events command without any pre-configured scope.","commands":{"allow":[],"deny":["set_ignore_cursor_events"]}},"deny-set-max-size":{"identifier":"deny-set-max-size","description":"Denies the set_max_size command without any pre-configured scope.","commands":{"allow":[],"deny":["set_max_size"]}},"deny-set-maximizable":{"identifier":"deny-set-maximizable","description":"Denies the set_maximizable command without any pre-configured scope.","commands":{"allow":[],"deny":["set_maximizable"]}},"deny-set-min-size":{"identifier":"deny-set-min-size","description":"Denies the set_min_size command without any pre-configured scope.","commands":{"allow":[],"deny":["set_min_size"]}},"deny-set-minimizable":{"identifier":"deny-set-minimizable","description":"Denies the set_minimizable command without any pre-configured scope.","commands":{"allow":[],"deny":["set_minimizable"]}},"deny-set-overlay-icon":{"identifier":"deny-set-overlay-icon","description":"Denies the set_overlay_icon command without any pre-configured scope.","commands":{"allow":[],"deny":["set_overlay_icon"]}},"deny-set-position":{"identifier":"deny-set-position","description":"Denies the set_position command without any pre-configured scope.","commands":{"allow":[],"deny":["set_position"]}},"deny-set-progress-bar":{"identifier":"deny-set-progress-bar","description":"Denies the set_progress_bar command without any pre-configured scope.","commands":{"allow":[],"deny":["set_progress_bar"]}},"deny-set-resizable":{"identifier":"deny-set-resizable","description":"Denies the set_resizable command without any pre-configured scope.","commands":{"allow":[],"deny":["set_resizable"]}},"deny-set-shadow":{"identifier":"deny-set-shadow","description":"Denies the set_shadow command without any pre-configured scope.","commands":{"allow":[],"deny":["set_shadow"]}},"deny-set-simple-fullscreen":{"identifier":"deny-set-simple-fullscreen","description":"Denies the set_simple_fullscreen command without any pre-configured scope.","commands":{"allow":[],"deny":["set_simple_fullscreen"]}},"deny-set-size":{"identifier":"deny-set-size","description":"Denies the set_size command without any pre-configured scope.","commands":{"allow":[],"deny":["set_size"]}},"deny-set-size-constraints":{"identifier":"deny-set-size-constraints","description":"Denies the set_size_constraints command without any pre-configured scope.","commands":{"allow":[],"deny":["set_size_constraints"]}},"deny-set-skip-taskbar":{"identifier":"deny-set-skip-taskbar","description":"Denies the set_skip_taskbar command without any pre-configured scope.","commands":{"allow":[],"deny":["set_skip_taskbar"]}},"deny-set-theme":{"identifier":"deny-set-theme","description":"Denies the set_theme command without any pre-configured scope.","commands":{"allow":[],"deny":["set_theme"]}},"deny-set-title":{"identifier":"deny-set-title","description":"Denies the set_title command without any pre-configured scope.","commands":{"allow":[],"deny":["set_title"]}},"deny-set-title-bar-style":{"identifier":"deny-set-title-bar-style","description":"Denies the set_title_bar_style command without any pre-configured scope.","commands":{"allow":[],"deny":["set_title_bar_style"]}},"deny-set-visible-on-all-workspaces":{"identifier":"deny-set-visible-on-all-workspaces","description":"Denies the set_visible_on_all_workspaces command without any pre-configured scope.","commands":{"allow":[],"deny":["set_visible_on_all_workspaces"]}},"deny-show":{"identifier":"deny-show","description":"Denies the show command without any pre-configured scope.","commands":{"allow":[],"deny":["show"]}},"deny-start-dragging":{"identifier":"deny-start-dragging","description":"Denies the start_dragging command without any pre-configured scope.","commands":{"allow":[],"deny":["start_dragging"]}},"deny-start-resize-dragging":{"identifier":"deny-start-resize-dragging","description":"Denies the start_resize_dragging command without any pre-configured scope.","commands":{"allow":[],"deny":["start_resize_dragging"]}},"deny-theme":{"identifier":"deny-theme","description":"Denies the theme command without any pre-configured scope.","commands":{"allow":[],"deny":["theme"]}},"deny-title":{"identifier":"deny-title","description":"Denies the title command without any pre-configured scope.","commands":{"allow":[],"deny":["title"]}},"deny-toggle-maximize":{"identifier":"deny-toggle-maximize","description":"Denies the toggle_maximize command without any pre-configured scope.","commands":{"allow":[],"deny":["toggle_maximize"]}},"deny-unmaximize":{"identifier":"deny-unmaximize","description":"Denies the unmaximize command without any pre-configured scope.","commands":{"allow":[],"deny":["unmaximize"]}},"deny-unminimize":{"identifier":"deny-unminimize","description":"Denies the unminimize command without any pre-configured scope.","commands":{"allow":[],"deny":["unminimize"]}}},"permission_sets":{},"global_scope_schema":null}}
````

## File: crates/ghost-updater-tauri/src-tauri/gen/schemas/capabilities.json
````json
{"default":{"identifier":"default","description":"ghost-updater 主窗口默认能力","local":true,"windows":["main"],"permissions":["core:default"]}}
````

## File: crates/ghost-updater-tauri/src-tauri/gen/schemas/desktop-schema.json
````json
{
  "$schema": "http://json-schema.org/draft-07/schema#",
  "title": "CapabilityFile",
  "description": "Capability formats accepted in a capability file.",
  "anyOf": [
    {
      "description": "A single capability.",
      "allOf": [
        {
          "$ref": "#/definitions/Capability"
        }
      ]
    },
    {
      "description": "A list of capabilities.",
      "type": "array",
      "items": {
        "$ref": "#/definitions/Capability"
      }
    },
    {
      "description": "A list of capabilities.",
      "type": "object",
      "required": [
        "capabilities"
      ],
      "properties": {
        "capabilities": {
          "description": "The list of capabilities.",
          "type": "array",
          "items": {
            "$ref": "#/definitions/Capability"
          }
        }
      }
    }
  ],
  "definitions": {
    "Capability": {
      "description": "A grouping and boundary mechanism developers can use to isolate access to the IPC layer.\n\nIt controls application windows' and webviews' fine grained access to the Tauri core, application, or plugin commands. If a webview or its window is not matching any capability then it has no access to the IPC layer at all.\n\nThis can be done to create groups of windows, based on their required system access, which can reduce impact of frontend vulnerabilities in less privileged windows. Windows can be added to a capability by exact name (e.g. `main-window`) or glob patterns like `*` or `admin-*`. A Window can have none, one, or multiple associated capabilities.\n\n## Example\n\n```json { \"identifier\": \"main-user-files-write\", \"description\": \"This capability allows the `main` window on macOS and Windows access to `filesystem` write related commands and `dialog` commands to enable programmatic access to files selected by the user.\", \"windows\": [ \"main\" ], \"permissions\": [ \"core:default\", \"dialog:open\", { \"identifier\": \"fs:allow-write-text-file\", \"allow\": [{ \"path\": \"$HOME/test.txt\" }] }, ], \"platforms\": [\"macOS\",\"windows\"] } ```",
      "type": "object",
      "required": [
        "identifier",
        "permissions"
      ],
      "properties": {
        "identifier": {
          "description": "Identifier of the capability.\n\n## Example\n\n`main-user-files-write`",
          "type": "string"
        },
        "description": {
          "description": "Description of what the capability is intended to allow on associated windows.\n\nIt should contain a description of what the grouped permissions should allow.\n\n## Example\n\nThis capability allows the `main` window access to `filesystem` write related commands and `dialog` commands to enable programmatic access to files selected by the user.",
          "default": "",
          "type": "string"
        },
        "remote": {
          "description": "Configure remote URLs that can use the capability permissions.\n\nThis setting is optional and defaults to not being set, as our default use case is that the content is served from our local application.\n\n:::caution Make sure you understand the security implications of providing remote sources with local system access. :::\n\n## Example\n\n```json { \"urls\": [\"https://*.mydomain.dev\"] } ```",
          "anyOf": [
            {
              "$ref": "#/definitions/CapabilityRemote"
            },
            {
              "type": "null"
            }
          ]
        },
        "local": {
          "description": "Whether this capability is enabled for local app URLs or not. Defaults to `true`.",
          "default": true,
          "type": "boolean"
        },
        "windows": {
          "description": "List of windows that are affected by this capability. Can be a glob pattern.\n\nIf a window label matches any of the patterns in this list, the capability will be enabled on all the webviews of that window, regardless of the value of [`Self::webviews`].\n\nOn multiwebview windows, prefer specifying [`Self::webviews`] and omitting [`Self::windows`] for a fine grained access control.\n\n## Example\n\n`[\"main\"]`",
          "type": "array",
          "items": {
            "type": "string"
          }
        },
        "webviews": {
          "description": "List of webviews that are affected by this capability. Can be a glob pattern.\n\nThe capability will be enabled on all the webviews whose label matches any of the patterns in this list, regardless of whether the webview's window label matches a pattern in [`Self::windows`].\n\n## Example\n\n`[\"sub-webview-one\", \"sub-webview-two\"]`",
          "type": "array",
          "items": {
            "type": "string"
          }
        },
        "permissions": {
          "description": "List of permissions attached to this capability.\n\nMust include the plugin name as prefix in the form of `${plugin-name}:${permission-name}`. For commands directly implemented in the application itself only `${permission-name}` is required.\n\n## Example\n\n```json [ \"core:default\", \"shell:allow-open\", \"dialog:open\", { \"identifier\": \"fs:allow-write-text-file\", \"allow\": [{ \"path\": \"$HOME/test.txt\" }] } ] ```",
          "type": "array",
          "items": {
            "$ref": "#/definitions/PermissionEntry"
          },
          "uniqueItems": true
        },
        "platforms": {
          "description": "Limit which target platforms this capability applies to.\n\nBy default all platforms are targeted.\n\n## Example\n\n`[\"macOS\",\"windows\"]`",
          "type": [
            "array",
            "null"
          ],
          "items": {
            "$ref": "#/definitions/Target"
          }
        }
      }
    },
    "CapabilityRemote": {
      "description": "Configuration for remote URLs that are associated with the capability.",
      "type": "object",
      "required": [
        "urls"
      ],
      "properties": {
        "urls": {
          "description": "Remote domains this capability refers to using the [URLPattern standard](https://urlpattern.spec.whatwg.org/).\n\n## Examples\n\n- \"https://*.mydomain.dev\": allows subdomains of mydomain.dev - \"https://mydomain.dev/api/*\": allows any subpath of mydomain.dev/api",
          "type": "array",
          "items": {
            "type": "string"
          }
        }
      }
    },
    "PermissionEntry": {
      "description": "An entry for a permission value in a [`Capability`] can be either a raw permission [`Identifier`] or an object that references a permission and extends its scope.",
      "anyOf": [
        {
          "description": "Reference a permission or permission set by identifier.",
          "allOf": [
            {
              "$ref": "#/definitions/Identifier"
            }
          ]
        },
        {
          "description": "Reference a permission or permission set by identifier and extends its scope.",
          "type": "object",
          "allOf": [
            {
              "properties": {
                "identifier": {
                  "description": "Identifier of the permission or permission set.",
                  "allOf": [
                    {
                      "$ref": "#/definitions/Identifier"
                    }
                  ]
                },
                "allow": {
                  "description": "Data that defines what is allowed by the scope.",
                  "type": [
                    "array",
                    "null"
                  ],
                  "items": {
                    "$ref": "#/definitions/Value"
                  }
                },
                "deny": {
                  "description": "Data that defines what is denied by the scope. This should be prioritized by validation logic.",
                  "type": [
                    "array",
                    "null"
                  ],
                  "items": {
                    "$ref": "#/definitions/Value"
                  }
                }
              }
            }
          ],
          "required": [
            "identifier"
          ]
        }
      ]
    },
    "Identifier": {
      "description": "Permission identifier",
      "oneOf": [
        {
          "description": "Default core plugins set.\n#### This default permission set includes:\n\n- `core:path:default`\n- `core:event:default`\n- `core:window:default`\n- `core:webview:default`\n- `core:app:default`\n- `core:image:default`\n- `core:resources:default`\n- `core:menu:default`\n- `core:tray:default`",
          "type": "string",
          "const": "core:default",
          "markdownDescription": "Default core plugins set.\n#### This default permission set includes:\n\n- `core:path:default`\n- `core:event:default`\n- `core:window:default`\n- `core:webview:default`\n- `core:app:default`\n- `core:image:default`\n- `core:resources:default`\n- `core:menu:default`\n- `core:tray:default`"
        },
        {
          "description": "Default permissions for the plugin.\n#### This default permission set includes:\n\n- `allow-version`\n- `allow-name`\n- `allow-tauri-version`\n- `allow-identifier`\n- `allow-bundle-type`\n- `allow-register-listener`\n- `allow-remove-listener`\n- `allow-supports-multiple-windows`",
          "type": "string",
          "const": "core:app:default",
          "markdownDescription": "Default permissions for the plugin.\n#### This default permission set includes:\n\n- `allow-version`\n- `allow-name`\n- `allow-tauri-version`\n- `allow-identifier`\n- `allow-bundle-type`\n- `allow-register-listener`\n- `allow-remove-listener`\n- `allow-supports-multiple-windows`"
        },
        {
          "description": "Enables the app_hide command without any pre-configured scope.",
          "type": "string",
          "const": "core:app:allow-app-hide",
          "markdownDescription": "Enables the app_hide command without any pre-configured scope."
        },
        {
          "description": "Enables the app_show command without any pre-configured scope.",
          "type": "string",
          "const": "core:app:allow-app-show",
          "markdownDescription": "Enables the app_show command without any pre-configured scope."
        },
        {
          "description": "Enables the bundle_type command without any pre-configured scope.",
          "type": "string",
          "const": "core:app:allow-bundle-type",
          "markdownDescription": "Enables the bundle_type command without any pre-configured scope."
        },
        {
          "description": "Enables the default_window_icon command without any pre-configured scope.",
          "type": "string",
          "const": "core:app:allow-default-window-icon",
          "markdownDescription": "Enables the default_window_icon command without any pre-configured scope."
        },
        {
          "description": "Enables the exit command without any pre-configured scope.",
          "type": "string",
          "const": "core:app:allow-exit",
          "markdownDescription": "Enables the exit command without any pre-configured scope."
        },
        {
          "description": "Enables the fetch_data_store_identifiers command without any pre-configured scope.",
          "type": "string",
          "const": "core:app:allow-fetch-data-store-identifiers",
          "markdownDescription": "Enables the fetch_data_store_identifiers command without any pre-configured scope."
        },
        {
          "description": "Enables the identifier command without any pre-configured scope.",
          "type": "string",
          "const": "core:app:allow-identifier",
          "markdownDescription": "Enables the identifier command without any pre-configured scope."
        },
        {
          "description": "Enables the name command without any pre-configured scope.",
          "type": "string",
          "const": "core:app:allow-name",
          "markdownDescription": "Enables the name command without any pre-configured scope."
        },
        {
          "description": "Enables the register_listener command without any pre-configured scope.",
          "type": "string",
          "const": "core:app:allow-register-listener",
          "markdownDescription": "Enables the register_listener command without any pre-configured scope."
        },
        {
          "description": "Enables the remove_data_store command without any pre-configured scope.",
          "type": "string",
          "const": "core:app:allow-remove-data-store",
          "markdownDescription": "Enables the remove_data_store command without any pre-configured scope."
        },
        {
          "description": "Enables the remove_listener command without any pre-configured scope.",
          "type": "string",
          "const": "core:app:allow-remove-listener",
          "markdownDescription": "Enables the remove_listener command without any pre-configured scope."
        },
        {
          "description": "Enables the set_app_theme command without any pre-configured scope.",
          "type": "string",
          "const": "core:app:allow-set-app-theme",
          "markdownDescription": "Enables the set_app_theme command without any pre-configured scope."
        },
        {
          "description": "Enables the set_dock_visibility command without any pre-configured scope.",
          "type": "string",
          "const": "core:app:allow-set-dock-visibility",
          "markdownDescription": "Enables the set_dock_visibility command without any pre-configured scope."
        },
        {
          "description": "Enables the supports_multiple_windows command without any pre-configured scope.",
          "type": "string",
          "const": "core:app:allow-supports-multiple-windows",
          "markdownDescription": "Enables the supports_multiple_windows command without any pre-configured scope."
        },
        {
          "description": "Enables the tauri_version command without any pre-configured scope.",
          "type": "string",
          "const": "core:app:allow-tauri-version",
          "markdownDescription": "Enables the tauri_version command without any pre-configured scope."
        },
        {
          "description": "Enables the version command without any pre-configured scope.",
          "type": "string",
          "const": "core:app:allow-version",
          "markdownDescription": "Enables the version command without any pre-configured scope."
        },
        {
          "description": "Denies the app_hide command without any pre-configured scope.",
          "type": "string",
          "const": "core:app:deny-app-hide",
          "markdownDescription": "Denies the app_hide command without any pre-configured scope."
        },
        {
          "description": "Denies the app_show command without any pre-configured scope.",
          "type": "string",
          "const": "core:app:deny-app-show",
          "markdownDescription": "Denies the app_show command without any pre-configured scope."
        },
        {
          "description": "Denies the bundle_type command without any pre-configured scope.",
          "type": "string",
          "const": "core:app:deny-bundle-type",
          "markdownDescription": "Denies the bundle_type command without any pre-configured scope."
        },
        {
          "description": "Denies the default_window_icon command without any pre-configured scope.",
          "type": "string",
          "const": "core:app:deny-default-window-icon",
          "markdownDescription": "Denies the default_window_icon command without any pre-configured scope."
        },
        {
          "description": "Denies the exit command without any pre-configured scope.",
          "type": "string",
          "const": "core:app:deny-exit",
          "markdownDescription": "Denies the exit command without any pre-configured scope."
        },
        {
          "description": "Denies the fetch_data_store_identifiers command without any pre-configured scope.",
          "type": "string",
          "const": "core:app:deny-fetch-data-store-identifiers",
          "markdownDescription": "Denies the fetch_data_store_identifiers command without any pre-configured scope."
        },
        {
          "description": "Denies the identifier command without any pre-configured scope.",
          "type": "string",
          "const": "core:app:deny-identifier",
          "markdownDescription": "Denies the identifier command without any pre-configured scope."
        },
        {
          "description": "Denies the name command without any pre-configured scope.",
          "type": "string",
          "const": "core:app:deny-name",
          "markdownDescription": "Denies the name command without any pre-configured scope."
        },
        {
          "description": "Denies the register_listener command without any pre-configured scope.",
          "type": "string",
          "const": "core:app:deny-register-listener",
          "markdownDescription": "Denies the register_listener command without any pre-configured scope."
        },
        {
          "description": "Denies the remove_data_store command without any pre-configured scope.",
          "type": "string",
          "const": "core:app:deny-remove-data-store",
          "markdownDescription": "Denies the remove_data_store command without any pre-configured scope."
        },
        {
          "description": "Denies the remove_listener command without any pre-configured scope.",
          "type": "string",
          "const": "core:app:deny-remove-listener",
          "markdownDescription": "Denies the remove_listener command without any pre-configured scope."
        },
        {
          "description": "Denies the set_app_theme command without any pre-configured scope.",
          "type": "string",
          "const": "core:app:deny-set-app-theme",
          "markdownDescription": "Denies the set_app_theme command without any pre-configured scope."
        },
        {
          "description": "Denies the set_dock_visibility command without any pre-configured scope.",
          "type": "string",
          "const": "core:app:deny-set-dock-visibility",
          "markdownDescription": "Denies the set_dock_visibility command without any pre-configured scope."
        },
        {
          "description": "Denies the supports_multiple_windows command without any pre-configured scope.",
          "type": "string",
          "const": "core:app:deny-supports-multiple-windows",
          "markdownDescription": "Denies the supports_multiple_windows command without any pre-configured scope."
        },
        {
          "description": "Denies the tauri_version command without any pre-configured scope.",
          "type": "string",
          "const": "core:app:deny-tauri-version",
          "markdownDescription": "Denies the tauri_version command without any pre-configured scope."
        },
        {
          "description": "Denies the version command without any pre-configured scope.",
          "type": "string",
          "const": "core:app:deny-version",
          "markdownDescription": "Denies the version command without any pre-configured scope."
        },
        {
          "description": "Default permissions for the plugin, which enables all commands.\n#### This default permission set includes:\n\n- `allow-listen`\n- `allow-unlisten`\n- `allow-emit`\n- `allow-emit-to`",
          "type": "string",
          "const": "core:event:default",
          "markdownDescription": "Default permissions for the plugin, which enables all commands.\n#### This default permission set includes:\n\n- `allow-listen`\n- `allow-unlisten`\n- `allow-emit`\n- `allow-emit-to`"
        },
        {
          "description": "Enables the emit command without any pre-configured scope.",
          "type": "string",
          "const": "core:event:allow-emit",
          "markdownDescription": "Enables the emit command without any pre-configured scope."
        },
        {
          "description": "Enables the emit_to command without any pre-configured scope.",
          "type": "string",
          "const": "core:event:allow-emit-to",
          "markdownDescription": "Enables the emit_to command without any pre-configured scope."
        },
        {
          "description": "Enables the listen command without any pre-configured scope.",
          "type": "string",
          "const": "core:event:allow-listen",
          "markdownDescription": "Enables the listen command without any pre-configured scope."
        },
        {
          "description": "Enables the unlisten command without any pre-configured scope.",
          "type": "string",
          "const": "core:event:allow-unlisten",
          "markdownDescription": "Enables the unlisten command without any pre-configured scope."
        },
        {
          "description": "Denies the emit command without any pre-configured scope.",
          "type": "string",
          "const": "core:event:deny-emit",
          "markdownDescription": "Denies the emit command without any pre-configured scope."
        },
        {
          "description": "Denies the emit_to command without any pre-configured scope.",
          "type": "string",
          "const": "core:event:deny-emit-to",
          "markdownDescription": "Denies the emit_to command without any pre-configured scope."
        },
        {
          "description": "Denies the listen command without any pre-configured scope.",
          "type": "string",
          "const": "core:event:deny-listen",
          "markdownDescription": "Denies the listen command without any pre-configured scope."
        },
        {
          "description": "Denies the unlisten command without any pre-configured scope.",
          "type": "string",
          "const": "core:event:deny-unlisten",
          "markdownDescription": "Denies the unlisten command without any pre-configured scope."
        },
        {
          "description": "Default permissions for the plugin, which enables all commands.\n#### This default permission set includes:\n\n- `allow-new`\n- `allow-from-bytes`\n- `allow-from-path`\n- `allow-rgba`\n- `allow-size`",
          "type": "string",
          "const": "core:image:default",
          "markdownDescription": "Default permissions for the plugin, which enables all commands.\n#### This default permission set includes:\n\n- `allow-new`\n- `allow-from-bytes`\n- `allow-from-path`\n- `allow-rgba`\n- `allow-size`"
        },
        {
          "description": "Enables the from_bytes command without any pre-configured scope.",
          "type": "string",
          "const": "core:image:allow-from-bytes",
          "markdownDescription": "Enables the from_bytes command without any pre-configured scope."
        },
        {
          "description": "Enables the from_path command without any pre-configured scope.",
          "type": "string",
          "const": "core:image:allow-from-path",
          "markdownDescription": "Enables the from_path command without any pre-configured scope."
        },
        {
          "description": "Enables the new command without any pre-configured scope.",
          "type": "string",
          "const": "core:image:allow-new",
          "markdownDescription": "Enables the new command without any pre-configured scope."
        },
        {
          "description": "Enables the rgba command without any pre-configured scope.",
          "type": "string",
          "const": "core:image:allow-rgba",
          "markdownDescription": "Enables the rgba command without any pre-configured scope."
        },
        {
          "description": "Enables the size command without any pre-configured scope.",
          "type": "string",
          "const": "core:image:allow-size",
          "markdownDescription": "Enables the size command without any pre-configured scope."
        },
        {
          "description": "Denies the from_bytes command without any pre-configured scope.",
          "type": "string",
          "const": "core:image:deny-from-bytes",
          "markdownDescription": "Denies the from_bytes command without any pre-configured scope."
        },
        {
          "description": "Denies the from_path command without any pre-configured scope.",
          "type": "string",
          "const": "core:image:deny-from-path",
          "markdownDescription": "Denies the from_path command without any pre-configured scope."
        },
        {
          "description": "Denies the new command without any pre-configured scope.",
          "type": "string",
          "const": "core:image:deny-new",
          "markdownDescription": "Denies the new command without any pre-configured scope."
        },
        {
          "description": "Denies the rgba command without any pre-configured scope.",
          "type": "string",
          "const": "core:image:deny-rgba",
          "markdownDescription": "Denies the rgba command without any pre-configured scope."
        },
        {
          "description": "Denies the size command without any pre-configured scope.",
          "type": "string",
          "const": "core:image:deny-size",
          "markdownDescription": "Denies the size command without any pre-configured scope."
        },
        {
          "description": "Default permissions for the plugin, which enables all commands.\n#### This default permission set includes:\n\n- `allow-new`\n- `allow-append`\n- `allow-prepend`\n- `allow-insert`\n- `allow-remove`\n- `allow-remove-at`\n- `allow-items`\n- `allow-get`\n- `allow-popup`\n- `allow-create-default`\n- `allow-set-as-app-menu`\n- `allow-set-as-window-menu`\n- `allow-text`\n- `allow-set-text`\n- `allow-is-enabled`\n- `allow-set-enabled`\n- `allow-set-accelerator`\n- `allow-set-as-windows-menu-for-nsapp`\n- `allow-set-as-help-menu-for-nsapp`\n- `allow-is-checked`\n- `allow-set-checked`\n- `allow-set-icon`",
          "type": "string",
          "const": "core:menu:default",
          "markdownDescription": "Default permissions for the plugin, which enables all commands.\n#### This default permission set includes:\n\n- `allow-new`\n- `allow-append`\n- `allow-prepend`\n- `allow-insert`\n- `allow-remove`\n- `allow-remove-at`\n- `allow-items`\n- `allow-get`\n- `allow-popup`\n- `allow-create-default`\n- `allow-set-as-app-menu`\n- `allow-set-as-window-menu`\n- `allow-text`\n- `allow-set-text`\n- `allow-is-enabled`\n- `allow-set-enabled`\n- `allow-set-accelerator`\n- `allow-set-as-windows-menu-for-nsapp`\n- `allow-set-as-help-menu-for-nsapp`\n- `allow-is-checked`\n- `allow-set-checked`\n- `allow-set-icon`"
        },
        {
          "description": "Enables the append command without any pre-configured scope.",
          "type": "string",
          "const": "core:menu:allow-append",
          "markdownDescription": "Enables the append command without any pre-configured scope."
        },
        {
          "description": "Enables the create_default command without any pre-configured scope.",
          "type": "string",
          "const": "core:menu:allow-create-default",
          "markdownDescription": "Enables the create_default command without any pre-configured scope."
        },
        {
          "description": "Enables the get command without any pre-configured scope.",
          "type": "string",
          "const": "core:menu:allow-get",
          "markdownDescription": "Enables the get command without any pre-configured scope."
        },
        {
          "description": "Enables the insert command without any pre-configured scope.",
          "type": "string",
          "const": "core:menu:allow-insert",
          "markdownDescription": "Enables the insert command without any pre-configured scope."
        },
        {
          "description": "Enables the is_checked command without any pre-configured scope.",
          "type": "string",
          "const": "core:menu:allow-is-checked",
          "markdownDescription": "Enables the is_checked command without any pre-configured scope."
        },
        {
          "description": "Enables the is_enabled command without any pre-configured scope.",
          "type": "string",
          "const": "core:menu:allow-is-enabled",
          "markdownDescription": "Enables the is_enabled command without any pre-configured scope."
        },
        {
          "description": "Enables the items command without any pre-configured scope.",
          "type": "string",
          "const": "core:menu:allow-items",
          "markdownDescription": "Enables the items command without any pre-configured scope."
        },
        {
          "description": "Enables the new command without any pre-configured scope.",
          "type": "string",
          "const": "core:menu:allow-new",
          "markdownDescription": "Enables the new command without any pre-configured scope."
        },
        {
          "description": "Enables the popup command without any pre-configured scope.",
          "type": "string",
          "const": "core:menu:allow-popup",
          "markdownDescription": "Enables the popup command without any pre-configured scope."
        },
        {
          "description": "Enables the prepend command without any pre-configured scope.",
          "type": "string",
          "const": "core:menu:allow-prepend",
          "markdownDescription": "Enables the prepend command without any pre-configured scope."
        },
        {
          "description": "Enables the remove command without any pre-configured scope.",
          "type": "string",
          "const": "core:menu:allow-remove",
          "markdownDescription": "Enables the remove command without any pre-configured scope."
        },
        {
          "description": "Enables the remove_at command without any pre-configured scope.",
          "type": "string",
          "const": "core:menu:allow-remove-at",
          "markdownDescription": "Enables the remove_at command without any pre-configured scope."
        },
        {
          "description": "Enables the set_accelerator command without any pre-configured scope.",
          "type": "string",
          "const": "core:menu:allow-set-accelerator",
          "markdownDescription": "Enables the set_accelerator command without any pre-configured scope."
        },
        {
          "description": "Enables the set_as_app_menu command without any pre-configured scope.",
          "type": "string",
          "const": "core:menu:allow-set-as-app-menu",
          "markdownDescription": "Enables the set_as_app_menu command without any pre-configured scope."
        },
        {
          "description": "Enables the set_as_help_menu_for_nsapp command without any pre-configured scope.",
          "type": "string",
          "const": "core:menu:allow-set-as-help-menu-for-nsapp",
          "markdownDescription": "Enables the set_as_help_menu_for_nsapp command without any pre-configured scope."
        },
        {
          "description": "Enables the set_as_window_menu command without any pre-configured scope.",
          "type": "string",
          "const": "core:menu:allow-set-as-window-menu",
          "markdownDescription": "Enables the set_as_window_menu command without any pre-configured scope."
        },
        {
          "description": "Enables the set_as_windows_menu_for_nsapp command without any pre-configured scope.",
          "type": "string",
          "const": "core:menu:allow-set-as-windows-menu-for-nsapp",
          "markdownDescription": "Enables the set_as_windows_menu_for_nsapp command without any pre-configured scope."
        },
        {
          "description": "Enables the set_checked command without any pre-configured scope.",
          "type": "string",
          "const": "core:menu:allow-set-checked",
          "markdownDescription": "Enables the set_checked command without any pre-configured scope."
        },
        {
          "description": "Enables the set_enabled command without any pre-configured scope.",
          "type": "string",
          "const": "core:menu:allow-set-enabled",
          "markdownDescription": "Enables the set_enabled command without any pre-configured scope."
        },
        {
          "description": "Enables the set_icon command without any pre-configured scope.",
          "type": "string",
          "const": "core:menu:allow-set-icon",
          "markdownDescription": "Enables the set_icon command without any pre-configured scope."
        },
        {
          "description": "Enables the set_text command without any pre-configured scope.",
          "type": "string",
          "const": "core:menu:allow-set-text",
          "markdownDescription": "Enables the set_text command without any pre-configured scope."
        },
        {
          "description": "Enables the text command without any pre-configured scope.",
          "type": "string",
          "const": "core:menu:allow-text",
          "markdownDescription": "Enables the text command without any pre-configured scope."
        },
        {
          "description": "Denies the append command without any pre-configured scope.",
          "type": "string",
          "const": "core:menu:deny-append",
          "markdownDescription": "Denies the append command without any pre-configured scope."
        },
        {
          "description": "Denies the create_default command without any pre-configured scope.",
          "type": "string",
          "const": "core:menu:deny-create-default",
          "markdownDescription": "Denies the create_default command without any pre-configured scope."
        },
        {
          "description": "Denies the get command without any pre-configured scope.",
          "type": "string",
          "const": "core:menu:deny-get",
          "markdownDescription": "Denies the get command without any pre-configured scope."
        },
        {
          "description": "Denies the insert command without any pre-configured scope.",
          "type": "string",
          "const": "core:menu:deny-insert",
          "markdownDescription": "Denies the insert command without any pre-configured scope."
        },
        {
          "description": "Denies the is_checked command without any pre-configured scope.",
          "type": "string",
          "const": "core:menu:deny-is-checked",
          "markdownDescription": "Denies the is_checked command without any pre-configured scope."
        },
        {
          "description": "Denies the is_enabled command without any pre-configured scope.",
          "type": "string",
          "const": "core:menu:deny-is-enabled",
          "markdownDescription": "Denies the is_enabled command without any pre-configured scope."
        },
        {
          "description": "Denies the items command without any pre-configured scope.",
          "type": "string",
          "const": "core:menu:deny-items",
          "markdownDescription": "Denies the items command without any pre-configured scope."
        },
        {
          "description": "Denies the new command without any pre-configured scope.",
          "type": "string",
          "const": "core:menu:deny-new",
          "markdownDescription": "Denies the new command without any pre-configured scope."
        },
        {
          "description": "Denies the popup command without any pre-configured scope.",
          "type": "string",
          "const": "core:menu:deny-popup",
          "markdownDescription": "Denies the popup command without any pre-configured scope."
        },
        {
          "description": "Denies the prepend command without any pre-configured scope.",
          "type": "string",
          "const": "core:menu:deny-prepend",
          "markdownDescription": "Denies the prepend command without any pre-configured scope."
        },
        {
          "description": "Denies the remove command without any pre-configured scope.",
          "type": "string",
          "const": "core:menu:deny-remove",
          "markdownDescription": "Denies the remove command without any pre-configured scope."
        },
        {
          "description": "Denies the remove_at command without any pre-configured scope.",
          "type": "string",
          "const": "core:menu:deny-remove-at",
          "markdownDescription": "Denies the remove_at command without any pre-configured scope."
        },
        {
          "description": "Denies the set_accelerator command without any pre-configured scope.",
          "type": "string",
          "const": "core:menu:deny-set-accelerator",
          "markdownDescription": "Denies the set_accelerator command without any pre-configured scope."
        },
        {
          "description": "Denies the set_as_app_menu command without any pre-configured scope.",
          "type": "string",
          "const": "core:menu:deny-set-as-app-menu",
          "markdownDescription": "Denies the set_as_app_menu command without any pre-configured scope."
        },
        {
          "description": "Denies the set_as_help_menu_for_nsapp command without any pre-configured scope.",
          "type": "string",
          "const": "core:menu:deny-set-as-help-menu-for-nsapp",
          "markdownDescription": "Denies the set_as_help_menu_for_nsapp command without any pre-configured scope."
        },
        {
          "description": "Denies the set_as_window_menu command without any pre-configured scope.",
          "type": "string",
          "const": "core:menu:deny-set-as-window-menu",
          "markdownDescription": "Denies the set_as_window_menu command without any pre-configured scope."
        },
        {
          "description": "Denies the set_as_windows_menu_for_nsapp command without any pre-configured scope.",
          "type": "string",
          "const": "core:menu:deny-set-as-windows-menu-for-nsapp",
          "markdownDescription": "Denies the set_as_windows_menu_for_nsapp command without any pre-configured scope."
        },
        {
          "description": "Denies the set_checked command without any pre-configured scope.",
          "type": "string",
          "const": "core:menu:deny-set-checked",
          "markdownDescription": "Denies the set_checked command without any pre-configured scope."
        },
        {
          "description": "Denies the set_enabled command without any pre-configured scope.",
          "type": "string",
          "const": "core:menu:deny-set-enabled",
          "markdownDescription": "Denies the set_enabled command without any pre-configured scope."
        },
        {
          "description": "Denies the set_icon command without any pre-configured scope.",
          "type": "string",
          "const": "core:menu:deny-set-icon",
          "markdownDescription": "Denies the set_icon command without any pre-configured scope."
        },
        {
          "description": "Denies the set_text command without any pre-configured scope.",
          "type": "string",
          "const": "core:menu:deny-set-text",
          "markdownDescription": "Denies the set_text command without any pre-configured scope."
        },
        {
          "description": "Denies the text command without any pre-configured scope.",
          "type": "string",
          "const": "core:menu:deny-text",
          "markdownDescription": "Denies the text command without any pre-configured scope."
        },
        {
          "description": "Default permissions for the plugin, which enables all commands.\n#### This default permission set includes:\n\n- `allow-resolve-directory`\n- `allow-resolve`\n- `allow-normalize`\n- `allow-join`\n- `allow-dirname`\n- `allow-extname`\n- `allow-basename`\n- `allow-is-absolute`",
          "type": "string",
          "const": "core:path:default",
          "markdownDescription": "Default permissions for the plugin, which enables all commands.\n#### This default permission set includes:\n\n- `allow-resolve-directory`\n- `allow-resolve`\n- `allow-normalize`\n- `allow-join`\n- `allow-dirname`\n- `allow-extname`\n- `allow-basename`\n- `allow-is-absolute`"
        },
        {
          "description": "Enables the basename command without any pre-configured scope.",
          "type": "string",
          "const": "core:path:allow-basename",
          "markdownDescription": "Enables the basename command without any pre-configured scope."
        },
        {
          "description": "Enables the dirname command without any pre-configured scope.",
          "type": "string",
          "const": "core:path:allow-dirname",
          "markdownDescription": "Enables the dirname command without any pre-configured scope."
        },
        {
          "description": "Enables the extname command without any pre-configured scope.",
          "type": "string",
          "const": "core:path:allow-extname",
          "markdownDescription": "Enables the extname command without any pre-configured scope."
        },
        {
          "description": "Enables the is_absolute command without any pre-configured scope.",
          "type": "string",
          "const": "core:path:allow-is-absolute",
          "markdownDescription": "Enables the is_absolute command without any pre-configured scope."
        },
        {
          "description": "Enables the join command without any pre-configured scope.",
          "type": "string",
          "const": "core:path:allow-join",
          "markdownDescription": "Enables the join command without any pre-configured scope."
        },
        {
          "description": "Enables the normalize command without any pre-configured scope.",
          "type": "string",
          "const": "core:path:allow-normalize",
          "markdownDescription": "Enables the normalize command without any pre-configured scope."
        },
        {
          "description": "Enables the resolve command without any pre-configured scope.",
          "type": "string",
          "const": "core:path:allow-resolve",
          "markdownDescription": "Enables the resolve command without any pre-configured scope."
        },
        {
          "description": "Enables the resolve_directory command without any pre-configured scope.",
          "type": "string",
          "const": "core:path:allow-resolve-directory",
          "markdownDescription": "Enables the resolve_directory command without any pre-configured scope."
        },
        {
          "description": "Denies the basename command without any pre-configured scope.",
          "type": "string",
          "const": "core:path:deny-basename",
          "markdownDescription": "Denies the basename command without any pre-configured scope."
        },
        {
          "description": "Denies the dirname command without any pre-configured scope.",
          "type": "string",
          "const": "core:path:deny-dirname",
          "markdownDescription": "Denies the dirname command without any pre-configured scope."
        },
        {
          "description": "Denies the extname command without any pre-configured scope.",
          "type": "string",
          "const": "core:path:deny-extname",
          "markdownDescription": "Denies the extname command without any pre-configured scope."
        },
        {
          "description": "Denies the is_absolute command without any pre-configured scope.",
          "type": "string",
          "const": "core:path:deny-is-absolute",
          "markdownDescription": "Denies the is_absolute command without any pre-configured scope."
        },
        {
          "description": "Denies the join command without any pre-configured scope.",
          "type": "string",
          "const": "core:path:deny-join",
          "markdownDescription": "Denies the join command without any pre-configured scope."
        },
        {
          "description": "Denies the normalize command without any pre-configured scope.",
          "type": "string",
          "const": "core:path:deny-normalize",
          "markdownDescription": "Denies the normalize command without any pre-configured scope."
        },
        {
          "description": "Denies the resolve command without any pre-configured scope.",
          "type": "string",
          "const": "core:path:deny-resolve",
          "markdownDescription": "Denies the resolve command without any pre-configured scope."
        },
        {
          "description": "Denies the resolve_directory command without any pre-configured scope.",
          "type": "string",
          "const": "core:path:deny-resolve-directory",
          "markdownDescription": "Denies the resolve_directory command without any pre-configured scope."
        },
        {
          "description": "Default permissions for the plugin, which enables all commands.\n#### This default permission set includes:\n\n- `allow-close`",
          "type": "string",
          "const": "core:resources:default",
          "markdownDescription": "Default permissions for the plugin, which enables all commands.\n#### This default permission set includes:\n\n- `allow-close`"
        },
        {
          "description": "Enables the close command without any pre-configured scope.",
          "type": "string",
          "const": "core:resources:allow-close",
          "markdownDescription": "Enables the close command without any pre-configured scope."
        },
        {
          "description": "Denies the close command without any pre-configured scope.",
          "type": "string",
          "const": "core:resources:deny-close",
          "markdownDescription": "Denies the close command without any pre-configured scope."
        },
        {
          "description": "Default permissions for the plugin, which enables all commands.\n#### This default permission set includes:\n\n- `allow-new`\n- `allow-get-by-id`\n- `allow-remove-by-id`\n- `allow-set-icon`\n- `allow-set-menu`\n- `allow-set-tooltip`\n- `allow-set-title`\n- `allow-set-visible`\n- `allow-set-temp-dir-path`\n- `allow-set-icon-as-template`\n- `allow-set-icon-with-as-template`\n- `allow-set-show-menu-on-left-click`",
          "type": "string",
          "const": "core:tray:default",
          "markdownDescription": "Default permissions for the plugin, which enables all commands.\n#### This default permission set includes:\n\n- `allow-new`\n- `allow-get-by-id`\n- `allow-remove-by-id`\n- `allow-set-icon`\n- `allow-set-menu`\n- `allow-set-tooltip`\n- `allow-set-title`\n- `allow-set-visible`\n- `allow-set-temp-dir-path`\n- `allow-set-icon-as-template`\n- `allow-set-icon-with-as-template`\n- `allow-set-show-menu-on-left-click`"
        },
        {
          "description": "Enables the get_by_id command without any pre-configured scope.",
          "type": "string",
          "const": "core:tray:allow-get-by-id",
          "markdownDescription": "Enables the get_by_id command without any pre-configured scope."
        },
        {
          "description": "Enables the new command without any pre-configured scope.",
          "type": "string",
          "const": "core:tray:allow-new",
          "markdownDescription": "Enables the new command without any pre-configured scope."
        },
        {
          "description": "Enables the remove_by_id command without any pre-configured scope.",
          "type": "string",
          "const": "core:tray:allow-remove-by-id",
          "markdownDescription": "Enables the remove_by_id command without any pre-configured scope."
        },
        {
          "description": "Enables the set_icon command without any pre-configured scope.",
          "type": "string",
          "const": "core:tray:allow-set-icon",
          "markdownDescription": "Enables the set_icon command without any pre-configured scope."
        },
        {
          "description": "Enables the set_icon_as_template command without any pre-configured scope.",
          "type": "string",
          "const": "core:tray:allow-set-icon-as-template",
          "markdownDescription": "Enables the set_icon_as_template command without any pre-configured scope."
        },
        {
          "description": "Enables the set_icon_with_as_template command without any pre-configured scope.",
          "type": "string",
          "const": "core:tray:allow-set-icon-with-as-template",
          "markdownDescription": "Enables the set_icon_with_as_template command without any pre-configured scope."
        },
        {
          "description": "Enables the set_menu command without any pre-configured scope.",
          "type": "string",
          "const": "core:tray:allow-set-menu",
          "markdownDescription": "Enables the set_menu command without any pre-configured scope."
        },
        {
          "description": "Enables the set_show_menu_on_left_click command without any pre-configured scope.",
          "type": "string",
          "const": "core:tray:allow-set-show-menu-on-left-click",
          "markdownDescription": "Enables the set_show_menu_on_left_click command without any pre-configured scope."
        },
        {
          "description": "Enables the set_temp_dir_path command without any pre-configured scope.",
          "type": "string",
          "const": "core:tray:allow-set-temp-dir-path",
          "markdownDescription": "Enables the set_temp_dir_path command without any pre-configured scope."
        },
        {
          "description": "Enables the set_title command without any pre-configured scope.",
          "type": "string",
          "const": "core:tray:allow-set-title",
          "markdownDescription": "Enables the set_title command without any pre-configured scope."
        },
        {
          "description": "Enables the set_tooltip command without any pre-configured scope.",
          "type": "string",
          "const": "core:tray:allow-set-tooltip",
          "markdownDescription": "Enables the set_tooltip command without any pre-configured scope."
        },
        {
          "description": "Enables the set_visible command without any pre-configured scope.",
          "type": "string",
          "const": "core:tray:allow-set-visible",
          "markdownDescription": "Enables the set_visible command without any pre-configured scope."
        },
        {
          "description": "Denies the get_by_id command without any pre-configured scope.",
          "type": "string",
          "const": "core:tray:deny-get-by-id",
          "markdownDescription": "Denies the get_by_id command without any pre-configured scope."
        },
        {
          "description": "Denies the new command without any pre-configured scope.",
          "type": "string",
          "const": "core:tray:deny-new",
          "markdownDescription": "Denies the new command without any pre-configured scope."
        },
        {
          "description": "Denies the remove_by_id command without any pre-configured scope.",
          "type": "string",
          "const": "core:tray:deny-remove-by-id",
          "markdownDescription": "Denies the remove_by_id command without any pre-configured scope."
        },
        {
          "description": "Denies the set_icon command without any pre-configured scope.",
          "type": "string",
          "const": "core:tray:deny-set-icon",
          "markdownDescription": "Denies the set_icon command without any pre-configured scope."
        },
        {
          "description": "Denies the set_icon_as_template command without any pre-configured scope.",
          "type": "string",
          "const": "core:tray:deny-set-icon-as-template",
          "markdownDescription": "Denies the set_icon_as_template command without any pre-configured scope."
        },
        {
          "description": "Denies the set_icon_with_as_template command without any pre-configured scope.",
          "type": "string",
          "const": "core:tray:deny-set-icon-with-as-template",
          "markdownDescription": "Denies the set_icon_with_as_template command without any pre-configured scope."
        },
        {
          "description": "Denies the set_menu command without any pre-configured scope.",
          "type": "string",
          "const": "core:tray:deny-set-menu",
          "markdownDescription": "Denies the set_menu command without any pre-configured scope."
        },
        {
          "description": "Denies the set_show_menu_on_left_click command without any pre-configured scope.",
          "type": "string",
          "const": "core:tray:deny-set-show-menu-on-left-click",
          "markdownDescription": "Denies the set_show_menu_on_left_click command without any pre-configured scope."
        },
        {
          "description": "Denies the set_temp_dir_path command without any pre-configured scope.",
          "type": "string",
          "const": "core:tray:deny-set-temp-dir-path",
          "markdownDescription": "Denies the set_temp_dir_path command without any pre-configured scope."
        },
        {
          "description": "Denies the set_title command without any pre-configured scope.",
          "type": "string",
          "const": "core:tray:deny-set-title",
          "markdownDescription": "Denies the set_title command without any pre-configured scope."
        },
        {
          "description": "Denies the set_tooltip command without any pre-configured scope.",
          "type": "string",
          "const": "core:tray:deny-set-tooltip",
          "markdownDescription": "Denies the set_tooltip command without any pre-configured scope."
        },
        {
          "description": "Denies the set_visible command without any pre-configured scope.",
          "type": "string",
          "const": "core:tray:deny-set-visible",
          "markdownDescription": "Denies the set_visible command without any pre-configured scope."
        },
        {
          "description": "Default permissions for the plugin.\n#### This default permission set includes:\n\n- `allow-get-all-webviews`\n- `allow-webview-position`\n- `allow-webview-size`\n- `allow-internal-toggle-devtools`",
          "type": "string",
          "const": "core:webview:default",
          "markdownDescription": "Default permissions for the plugin.\n#### This default permission set includes:\n\n- `allow-get-all-webviews`\n- `allow-webview-position`\n- `allow-webview-size`\n- `allow-internal-toggle-devtools`"
        },
        {
          "description": "Enables the clear_all_browsing_data command without any pre-configured scope.",
          "type": "string",
          "const": "core:webview:allow-clear-all-browsing-data",
          "markdownDescription": "Enables the clear_all_browsing_data command without any pre-configured scope."
        },
        {
          "description": "Enables the create_webview command without any pre-configured scope.",
          "type": "string",
          "const": "core:webview:allow-create-webview",
          "markdownDescription": "Enables the create_webview command without any pre-configured scope."
        },
        {
          "description": "Enables the create_webview_window command without any pre-configured scope.",
          "type": "string",
          "const": "core:webview:allow-create-webview-window",
          "markdownDescription": "Enables the create_webview_window command without any pre-configured scope."
        },
        {
          "description": "Enables the get_all_webviews command without any pre-configured scope.",
          "type": "string",
          "const": "core:webview:allow-get-all-webviews",
          "markdownDescription": "Enables the get_all_webviews command without any pre-configured scope."
        },
        {
          "description": "Enables the internal_toggle_devtools command without any pre-configured scope.",
          "type": "string",
          "const": "core:webview:allow-internal-toggle-devtools",
          "markdownDescription": "Enables the internal_toggle_devtools command without any pre-configured scope."
        },
        {
          "description": "Enables the print command without any pre-configured scope.",
          "type": "string",
          "const": "core:webview:allow-print",
          "markdownDescription": "Enables the print command without any pre-configured scope."
        },
        {
          "description": "Enables the reparent command without any pre-configured scope.",
          "type": "string",
          "const": "core:webview:allow-reparent",
          "markdownDescription": "Enables the reparent command without any pre-configured scope."
        },
        {
          "description": "Enables the set_webview_auto_resize command without any pre-configured scope.",
          "type": "string",
          "const": "core:webview:allow-set-webview-auto-resize",
          "markdownDescription": "Enables the set_webview_auto_resize command without any pre-configured scope."
        },
        {
          "description": "Enables the set_webview_background_color command without any pre-configured scope.",
          "type": "string",
          "const": "core:webview:allow-set-webview-background-color",
          "markdownDescription": "Enables the set_webview_background_color command without any pre-configured scope."
        },
        {
          "description": "Enables the set_webview_focus command without any pre-configured scope.",
          "type": "string",
          "const": "core:webview:allow-set-webview-focus",
          "markdownDescription": "Enables the set_webview_focus command without any pre-configured scope."
        },
        {
          "description": "Enables the set_webview_position command without any pre-configured scope.",
          "type": "string",
          "const": "core:webview:allow-set-webview-position",
          "markdownDescription": "Enables the set_webview_position command without any pre-configured scope."
        },
        {
          "description": "Enables the set_webview_size command without any pre-configured scope.",
          "type": "string",
          "const": "core:webview:allow-set-webview-size",
          "markdownDescription": "Enables the set_webview_size command without any pre-configured scope."
        },
        {
          "description": "Enables the set_webview_zoom command without any pre-configured scope.",
          "type": "string",
          "const": "core:webview:allow-set-webview-zoom",
          "markdownDescription": "Enables the set_webview_zoom command without any pre-configured scope."
        },
        {
          "description": "Enables the webview_close command without any pre-configured scope.",
          "type": "string",
          "const": "core:webview:allow-webview-close",
          "markdownDescription": "Enables the webview_close command without any pre-configured scope."
        },
        {
          "description": "Enables the webview_hide command without any pre-configured scope.",
          "type": "string",
          "const": "core:webview:allow-webview-hide",
          "markdownDescription": "Enables the webview_hide command without any pre-configured scope."
        },
        {
          "description": "Enables the webview_position command without any pre-configured scope.",
          "type": "string",
          "const": "core:webview:allow-webview-position",
          "markdownDescription": "Enables the webview_position command without any pre-configured scope."
        },
        {
          "description": "Enables the webview_show command without any pre-configured scope.",
          "type": "string",
          "const": "core:webview:allow-webview-show",
          "markdownDescription": "Enables the webview_show command without any pre-configured scope."
        },
        {
          "description": "Enables the webview_size command without any pre-configured scope.",
          "type": "string",
          "const": "core:webview:allow-webview-size",
          "markdownDescription": "Enables the webview_size command without any pre-configured scope."
        },
        {
          "description": "Denies the clear_all_browsing_data command without any pre-configured scope.",
          "type": "string",
          "const": "core:webview:deny-clear-all-browsing-data",
          "markdownDescription": "Denies the clear_all_browsing_data command without any pre-configured scope."
        },
        {
          "description": "Denies the create_webview command without any pre-configured scope.",
          "type": "string",
          "const": "core:webview:deny-create-webview",
          "markdownDescription": "Denies the create_webview command without any pre-configured scope."
        },
        {
          "description": "Denies the create_webview_window command without any pre-configured scope.",
          "type": "string",
          "const": "core:webview:deny-create-webview-window",
          "markdownDescription": "Denies the create_webview_window command without any pre-configured scope."
        },
        {
          "description": "Denies the get_all_webviews command without any pre-configured scope.",
          "type": "string",
          "const": "core:webview:deny-get-all-webviews",
          "markdownDescription": "Denies the get_all_webviews command without any pre-configured scope."
        },
        {
          "description": "Denies the internal_toggle_devtools command without any pre-configured scope.",
          "type": "string",
          "const": "core:webview:deny-internal-toggle-devtools",
          "markdownDescription": "Denies the internal_toggle_devtools command without any pre-configured scope."
        },
        {
          "description": "Denies the print command without any pre-configured scope.",
          "type": "string",
          "const": "core:webview:deny-print",
          "markdownDescription": "Denies the print command without any pre-configured scope."
        },
        {
          "description": "Denies the reparent command without any pre-configured scope.",
          "type": "string",
          "const": "core:webview:deny-reparent",
          "markdownDescription": "Denies the reparent command without any pre-configured scope."
        },
        {
          "description": "Denies the set_webview_auto_resize command without any pre-configured scope.",
          "type": "string",
          "const": "core:webview:deny-set-webview-auto-resize",
          "markdownDescription": "Denies the set_webview_auto_resize command without any pre-configured scope."
        },
        {
          "description": "Denies the set_webview_background_color command without any pre-configured scope.",
          "type": "string",
          "const": "core:webview:deny-set-webview-background-color",
          "markdownDescription": "Denies the set_webview_background_color command without any pre-configured scope."
        },
        {
          "description": "Denies the set_webview_focus command without any pre-configured scope.",
          "type": "string",
          "const": "core:webview:deny-set-webview-focus",
          "markdownDescription": "Denies the set_webview_focus command without any pre-configured scope."
        },
        {
          "description": "Denies the set_webview_position command without any pre-configured scope.",
          "type": "string",
          "const": "core:webview:deny-set-webview-position",
          "markdownDescription": "Denies the set_webview_position command without any pre-configured scope."
        },
        {
          "description": "Denies the set_webview_size command without any pre-configured scope.",
          "type": "string",
          "const": "core:webview:deny-set-webview-size",
          "markdownDescription": "Denies the set_webview_size command without any pre-configured scope."
        },
        {
          "description": "Denies the set_webview_zoom command without any pre-configured scope.",
          "type": "string",
          "const": "core:webview:deny-set-webview-zoom",
          "markdownDescription": "Denies the set_webview_zoom command without any pre-configured scope."
        },
        {
          "description": "Denies the webview_close command without any pre-configured scope.",
          "type": "string",
          "const": "core:webview:deny-webview-close",
          "markdownDescription": "Denies the webview_close command without any pre-configured scope."
        },
        {
          "description": "Denies the webview_hide command without any pre-configured scope.",
          "type": "string",
          "const": "core:webview:deny-webview-hide",
          "markdownDescription": "Denies the webview_hide command without any pre-configured scope."
        },
        {
          "description": "Denies the webview_position command without any pre-configured scope.",
          "type": "string",
          "const": "core:webview:deny-webview-position",
          "markdownDescription": "Denies the webview_position command without any pre-configured scope."
        },
        {
          "description": "Denies the webview_show command without any pre-configured scope.",
          "type": "string",
          "const": "core:webview:deny-webview-show",
          "markdownDescription": "Denies the webview_show command without any pre-configured scope."
        },
        {
          "description": "Denies the webview_size command without any pre-configured scope.",
          "type": "string",
          "const": "core:webview:deny-webview-size",
          "markdownDescription": "Denies the webview_size command without any pre-configured scope."
        },
        {
          "description": "Default permissions for the plugin.\n#### This default permission set includes:\n\n- `allow-get-all-windows`\n- `allow-scale-factor`\n- `allow-inner-position`\n- `allow-outer-position`\n- `allow-inner-size`\n- `allow-outer-size`\n- `allow-is-fullscreen`\n- `allow-is-minimized`\n- `allow-is-maximized`\n- `allow-is-focused`\n- `allow-is-decorated`\n- `allow-is-resizable`\n- `allow-is-maximizable`\n- `allow-is-minimizable`\n- `allow-is-closable`\n- `allow-is-visible`\n- `allow-is-enabled`\n- `allow-title`\n- `allow-current-monitor`\n- `allow-primary-monitor`\n- `allow-monitor-from-point`\n- `allow-available-monitors`\n- `allow-cursor-position`\n- `allow-theme`\n- `allow-is-always-on-top`\n- `allow-activity-name`\n- `allow-scene-identifier`\n- `allow-internal-toggle-maximize`",
          "type": "string",
          "const": "core:window:default",
          "markdownDescription": "Default permissions for the plugin.\n#### This default permission set includes:\n\n- `allow-get-all-windows`\n- `allow-scale-factor`\n- `allow-inner-position`\n- `allow-outer-position`\n- `allow-inner-size`\n- `allow-outer-size`\n- `allow-is-fullscreen`\n- `allow-is-minimized`\n- `allow-is-maximized`\n- `allow-is-focused`\n- `allow-is-decorated`\n- `allow-is-resizable`\n- `allow-is-maximizable`\n- `allow-is-minimizable`\n- `allow-is-closable`\n- `allow-is-visible`\n- `allow-is-enabled`\n- `allow-title`\n- `allow-current-monitor`\n- `allow-primary-monitor`\n- `allow-monitor-from-point`\n- `allow-available-monitors`\n- `allow-cursor-position`\n- `allow-theme`\n- `allow-is-always-on-top`\n- `allow-activity-name`\n- `allow-scene-identifier`\n- `allow-internal-toggle-maximize`"
        },
        {
          "description": "Enables the activity_name command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-activity-name",
          "markdownDescription": "Enables the activity_name command without any pre-configured scope."
        },
        {
          "description": "Enables the available_monitors command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-available-monitors",
          "markdownDescription": "Enables the available_monitors command without any pre-configured scope."
        },
        {
          "description": "Enables the center command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-center",
          "markdownDescription": "Enables the center command without any pre-configured scope."
        },
        {
          "description": "Enables the close command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-close",
          "markdownDescription": "Enables the close command without any pre-configured scope."
        },
        {
          "description": "Enables the create command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-create",
          "markdownDescription": "Enables the create command without any pre-configured scope."
        },
        {
          "description": "Enables the current_monitor command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-current-monitor",
          "markdownDescription": "Enables the current_monitor command without any pre-configured scope."
        },
        {
          "description": "Enables the cursor_position command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-cursor-position",
          "markdownDescription": "Enables the cursor_position command without any pre-configured scope."
        },
        {
          "description": "Enables the destroy command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-destroy",
          "markdownDescription": "Enables the destroy command without any pre-configured scope."
        },
        {
          "description": "Enables the get_all_windows command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-get-all-windows",
          "markdownDescription": "Enables the get_all_windows command without any pre-configured scope."
        },
        {
          "description": "Enables the hide command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-hide",
          "markdownDescription": "Enables the hide command without any pre-configured scope."
        },
        {
          "description": "Enables the inner_position command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-inner-position",
          "markdownDescription": "Enables the inner_position command without any pre-configured scope."
        },
        {
          "description": "Enables the inner_size command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-inner-size",
          "markdownDescription": "Enables the inner_size command without any pre-configured scope."
        },
        {
          "description": "Enables the internal_toggle_maximize command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-internal-toggle-maximize",
          "markdownDescription": "Enables the internal_toggle_maximize command without any pre-configured scope."
        },
        {
          "description": "Enables the is_always_on_top command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-is-always-on-top",
          "markdownDescription": "Enables the is_always_on_top command without any pre-configured scope."
        },
        {
          "description": "Enables the is_closable command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-is-closable",
          "markdownDescription": "Enables the is_closable command without any pre-configured scope."
        },
        {
          "description": "Enables the is_decorated command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-is-decorated",
          "markdownDescription": "Enables the is_decorated command without any pre-configured scope."
        },
        {
          "description": "Enables the is_enabled command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-is-enabled",
          "markdownDescription": "Enables the is_enabled command without any pre-configured scope."
        },
        {
          "description": "Enables the is_focused command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-is-focused",
          "markdownDescription": "Enables the is_focused command without any pre-configured scope."
        },
        {
          "description": "Enables the is_fullscreen command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-is-fullscreen",
          "markdownDescription": "Enables the is_fullscreen command without any pre-configured scope."
        },
        {
          "description": "Enables the is_maximizable command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-is-maximizable",
          "markdownDescription": "Enables the is_maximizable command without any pre-configured scope."
        },
        {
          "description": "Enables the is_maximized command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-is-maximized",
          "markdownDescription": "Enables the is_maximized command without any pre-configured scope."
        },
        {
          "description": "Enables the is_minimizable command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-is-minimizable",
          "markdownDescription": "Enables the is_minimizable command without any pre-configured scope."
        },
        {
          "description": "Enables the is_minimized command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-is-minimized",
          "markdownDescription": "Enables the is_minimized command without any pre-configured scope."
        },
        {
          "description": "Enables the is_resizable command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-is-resizable",
          "markdownDescription": "Enables the is_resizable command without any pre-configured scope."
        },
        {
          "description": "Enables the is_visible command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-is-visible",
          "markdownDescription": "Enables the is_visible command without any pre-configured scope."
        },
        {
          "description": "Enables the maximize command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-maximize",
          "markdownDescription": "Enables the maximize command without any pre-configured scope."
        },
        {
          "description": "Enables the minimize command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-minimize",
          "markdownDescription": "Enables the minimize command without any pre-configured scope."
        },
        {
          "description": "Enables the monitor_from_point command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-monitor-from-point",
          "markdownDescription": "Enables the monitor_from_point command without any pre-configured scope."
        },
        {
          "description": "Enables the outer_position command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-outer-position",
          "markdownDescription": "Enables the outer_position command without any pre-configured scope."
        },
        {
          "description": "Enables the outer_size command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-outer-size",
          "markdownDescription": "Enables the outer_size command without any pre-configured scope."
        },
        {
          "description": "Enables the primary_monitor command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-primary-monitor",
          "markdownDescription": "Enables the primary_monitor command without any pre-configured scope."
        },
        {
          "description": "Enables the request_user_attention command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-request-user-attention",
          "markdownDescription": "Enables the request_user_attention command without any pre-configured scope."
        },
        {
          "description": "Enables the scale_factor command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-scale-factor",
          "markdownDescription": "Enables the scale_factor command without any pre-configured scope."
        },
        {
          "description": "Enables the scene_identifier command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-scene-identifier",
          "markdownDescription": "Enables the scene_identifier command without any pre-configured scope."
        },
        {
          "description": "Enables the set_always_on_bottom command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-set-always-on-bottom",
          "markdownDescription": "Enables the set_always_on_bottom command without any pre-configured scope."
        },
        {
          "description": "Enables the set_always_on_top command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-set-always-on-top",
          "markdownDescription": "Enables the set_always_on_top command without any pre-configured scope."
        },
        {
          "description": "Enables the set_background_color command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-set-background-color",
          "markdownDescription": "Enables the set_background_color command without any pre-configured scope."
        },
        {
          "description": "Enables the set_badge_count command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-set-badge-count",
          "markdownDescription": "Enables the set_badge_count command without any pre-configured scope."
        },
        {
          "description": "Enables the set_badge_label command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-set-badge-label",
          "markdownDescription": "Enables the set_badge_label command without any pre-configured scope."
        },
        {
          "description": "Enables the set_closable command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-set-closable",
          "markdownDescription": "Enables the set_closable command without any pre-configured scope."
        },
        {
          "description": "Enables the set_content_protected command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-set-content-protected",
          "markdownDescription": "Enables the set_content_protected command without any pre-configured scope."
        },
        {
          "description": "Enables the set_cursor_grab command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-set-cursor-grab",
          "markdownDescription": "Enables the set_cursor_grab command without any pre-configured scope."
        },
        {
          "description": "Enables the set_cursor_icon command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-set-cursor-icon",
          "markdownDescription": "Enables the set_cursor_icon command without any pre-configured scope."
        },
        {
          "description": "Enables the set_cursor_position command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-set-cursor-position",
          "markdownDescription": "Enables the set_cursor_position command without any pre-configured scope."
        },
        {
          "description": "Enables the set_cursor_visible command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-set-cursor-visible",
          "markdownDescription": "Enables the set_cursor_visible command without any pre-configured scope."
        },
        {
          "description": "Enables the set_decorations command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-set-decorations",
          "markdownDescription": "Enables the set_decorations command without any pre-configured scope."
        },
        {
          "description": "Enables the set_effects command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-set-effects",
          "markdownDescription": "Enables the set_effects command without any pre-configured scope."
        },
        {
          "description": "Enables the set_enabled command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-set-enabled",
          "markdownDescription": "Enables the set_enabled command without any pre-configured scope."
        },
        {
          "description": "Enables the set_focus command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-set-focus",
          "markdownDescription": "Enables the set_focus command without any pre-configured scope."
        },
        {
          "description": "Enables the set_focusable command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-set-focusable",
          "markdownDescription": "Enables the set_focusable command without any pre-configured scope."
        },
        {
          "description": "Enables the set_fullscreen command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-set-fullscreen",
          "markdownDescription": "Enables the set_fullscreen command without any pre-configured scope."
        },
        {
          "description": "Enables the set_fullscreen_on_monitor command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-set-fullscreen-on-monitor",
          "markdownDescription": "Enables the set_fullscreen_on_monitor command without any pre-configured scope."
        },
        {
          "description": "Enables the set_icon command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-set-icon",
          "markdownDescription": "Enables the set_icon command without any pre-configured scope."
        },
        {
          "description": "Enables the set_ignore_cursor_events command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-set-ignore-cursor-events",
          "markdownDescription": "Enables the set_ignore_cursor_events command without any pre-configured scope."
        },
        {
          "description": "Enables the set_max_size command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-set-max-size",
          "markdownDescription": "Enables the set_max_size command without any pre-configured scope."
        },
        {
          "description": "Enables the set_maximizable command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-set-maximizable",
          "markdownDescription": "Enables the set_maximizable command without any pre-configured scope."
        },
        {
          "description": "Enables the set_min_size command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-set-min-size",
          "markdownDescription": "Enables the set_min_size command without any pre-configured scope."
        },
        {
          "description": "Enables the set_minimizable command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-set-minimizable",
          "markdownDescription": "Enables the set_minimizable command without any pre-configured scope."
        },
        {
          "description": "Enables the set_overlay_icon command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-set-overlay-icon",
          "markdownDescription": "Enables the set_overlay_icon command without any pre-configured scope."
        },
        {
          "description": "Enables the set_position command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-set-position",
          "markdownDescription": "Enables the set_position command without any pre-configured scope."
        },
        {
          "description": "Enables the set_progress_bar command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-set-progress-bar",
          "markdownDescription": "Enables the set_progress_bar command without any pre-configured scope."
        },
        {
          "description": "Enables the set_resizable command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-set-resizable",
          "markdownDescription": "Enables the set_resizable command without any pre-configured scope."
        },
        {
          "description": "Enables the set_shadow command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-set-shadow",
          "markdownDescription": "Enables the set_shadow command without any pre-configured scope."
        },
        {
          "description": "Enables the set_simple_fullscreen command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-set-simple-fullscreen",
          "markdownDescription": "Enables the set_simple_fullscreen command without any pre-configured scope."
        },
        {
          "description": "Enables the set_size command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-set-size",
          "markdownDescription": "Enables the set_size command without any pre-configured scope."
        },
        {
          "description": "Enables the set_size_constraints command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-set-size-constraints",
          "markdownDescription": "Enables the set_size_constraints command without any pre-configured scope."
        },
        {
          "description": "Enables the set_skip_taskbar command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-set-skip-taskbar",
          "markdownDescription": "Enables the set_skip_taskbar command without any pre-configured scope."
        },
        {
          "description": "Enables the set_theme command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-set-theme",
          "markdownDescription": "Enables the set_theme command without any pre-configured scope."
        },
        {
          "description": "Enables the set_title command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-set-title",
          "markdownDescription": "Enables the set_title command without any pre-configured scope."
        },
        {
          "description": "Enables the set_title_bar_style command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-set-title-bar-style",
          "markdownDescription": "Enables the set_title_bar_style command without any pre-configured scope."
        },
        {
          "description": "Enables the set_visible_on_all_workspaces command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-set-visible-on-all-workspaces",
          "markdownDescription": "Enables the set_visible_on_all_workspaces command without any pre-configured scope."
        },
        {
          "description": "Enables the show command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-show",
          "markdownDescription": "Enables the show command without any pre-configured scope."
        },
        {
          "description": "Enables the start_dragging command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-start-dragging",
          "markdownDescription": "Enables the start_dragging command without any pre-configured scope."
        },
        {
          "description": "Enables the start_resize_dragging command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-start-resize-dragging",
          "markdownDescription": "Enables the start_resize_dragging command without any pre-configured scope."
        },
        {
          "description": "Enables the theme command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-theme",
          "markdownDescription": "Enables the theme command without any pre-configured scope."
        },
        {
          "description": "Enables the title command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-title",
          "markdownDescription": "Enables the title command without any pre-configured scope."
        },
        {
          "description": "Enables the toggle_maximize command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-toggle-maximize",
          "markdownDescription": "Enables the toggle_maximize command without any pre-configured scope."
        },
        {
          "description": "Enables the unmaximize command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-unmaximize",
          "markdownDescription": "Enables the unmaximize command without any pre-configured scope."
        },
        {
          "description": "Enables the unminimize command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-unminimize",
          "markdownDescription": "Enables the unminimize command without any pre-configured scope."
        },
        {
          "description": "Denies the activity_name command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-activity-name",
          "markdownDescription": "Denies the activity_name command without any pre-configured scope."
        },
        {
          "description": "Denies the available_monitors command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-available-monitors",
          "markdownDescription": "Denies the available_monitors command without any pre-configured scope."
        },
        {
          "description": "Denies the center command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-center",
          "markdownDescription": "Denies the center command without any pre-configured scope."
        },
        {
          "description": "Denies the close command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-close",
          "markdownDescription": "Denies the close command without any pre-configured scope."
        },
        {
          "description": "Denies the create command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-create",
          "markdownDescription": "Denies the create command without any pre-configured scope."
        },
        {
          "description": "Denies the current_monitor command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-current-monitor",
          "markdownDescription": "Denies the current_monitor command without any pre-configured scope."
        },
        {
          "description": "Denies the cursor_position command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-cursor-position",
          "markdownDescription": "Denies the cursor_position command without any pre-configured scope."
        },
        {
          "description": "Denies the destroy command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-destroy",
          "markdownDescription": "Denies the destroy command without any pre-configured scope."
        },
        {
          "description": "Denies the get_all_windows command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-get-all-windows",
          "markdownDescription": "Denies the get_all_windows command without any pre-configured scope."
        },
        {
          "description": "Denies the hide command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-hide",
          "markdownDescription": "Denies the hide command without any pre-configured scope."
        },
        {
          "description": "Denies the inner_position command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-inner-position",
          "markdownDescription": "Denies the inner_position command without any pre-configured scope."
        },
        {
          "description": "Denies the inner_size command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-inner-size",
          "markdownDescription": "Denies the inner_size command without any pre-configured scope."
        },
        {
          "description": "Denies the internal_toggle_maximize command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-internal-toggle-maximize",
          "markdownDescription": "Denies the internal_toggle_maximize command without any pre-configured scope."
        },
        {
          "description": "Denies the is_always_on_top command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-is-always-on-top",
          "markdownDescription": "Denies the is_always_on_top command without any pre-configured scope."
        },
        {
          "description": "Denies the is_closable command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-is-closable",
          "markdownDescription": "Denies the is_closable command without any pre-configured scope."
        },
        {
          "description": "Denies the is_decorated command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-is-decorated",
          "markdownDescription": "Denies the is_decorated command without any pre-configured scope."
        },
        {
          "description": "Denies the is_enabled command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-is-enabled",
          "markdownDescription": "Denies the is_enabled command without any pre-configured scope."
        },
        {
          "description": "Denies the is_focused command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-is-focused",
          "markdownDescription": "Denies the is_focused command without any pre-configured scope."
        },
        {
          "description": "Denies the is_fullscreen command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-is-fullscreen",
          "markdownDescription": "Denies the is_fullscreen command without any pre-configured scope."
        },
        {
          "description": "Denies the is_maximizable command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-is-maximizable",
          "markdownDescription": "Denies the is_maximizable command without any pre-configured scope."
        },
        {
          "description": "Denies the is_maximized command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-is-maximized",
          "markdownDescription": "Denies the is_maximized command without any pre-configured scope."
        },
        {
          "description": "Denies the is_minimizable command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-is-minimizable",
          "markdownDescription": "Denies the is_minimizable command without any pre-configured scope."
        },
        {
          "description": "Denies the is_minimized command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-is-minimized",
          "markdownDescription": "Denies the is_minimized command without any pre-configured scope."
        },
        {
          "description": "Denies the is_resizable command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-is-resizable",
          "markdownDescription": "Denies the is_resizable command without any pre-configured scope."
        },
        {
          "description": "Denies the is_visible command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-is-visible",
          "markdownDescription": "Denies the is_visible command without any pre-configured scope."
        },
        {
          "description": "Denies the maximize command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-maximize",
          "markdownDescription": "Denies the maximize command without any pre-configured scope."
        },
        {
          "description": "Denies the minimize command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-minimize",
          "markdownDescription": "Denies the minimize command without any pre-configured scope."
        },
        {
          "description": "Denies the monitor_from_point command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-monitor-from-point",
          "markdownDescription": "Denies the monitor_from_point command without any pre-configured scope."
        },
        {
          "description": "Denies the outer_position command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-outer-position",
          "markdownDescription": "Denies the outer_position command without any pre-configured scope."
        },
        {
          "description": "Denies the outer_size command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-outer-size",
          "markdownDescription": "Denies the outer_size command without any pre-configured scope."
        },
        {
          "description": "Denies the primary_monitor command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-primary-monitor",
          "markdownDescription": "Denies the primary_monitor command without any pre-configured scope."
        },
        {
          "description": "Denies the request_user_attention command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-request-user-attention",
          "markdownDescription": "Denies the request_user_attention command without any pre-configured scope."
        },
        {
          "description": "Denies the scale_factor command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-scale-factor",
          "markdownDescription": "Denies the scale_factor command without any pre-configured scope."
        },
        {
          "description": "Denies the scene_identifier command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-scene-identifier",
          "markdownDescription": "Denies the scene_identifier command without any pre-configured scope."
        },
        {
          "description": "Denies the set_always_on_bottom command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-set-always-on-bottom",
          "markdownDescription": "Denies the set_always_on_bottom command without any pre-configured scope."
        },
        {
          "description": "Denies the set_always_on_top command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-set-always-on-top",
          "markdownDescription": "Denies the set_always_on_top command without any pre-configured scope."
        },
        {
          "description": "Denies the set_background_color command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-set-background-color",
          "markdownDescription": "Denies the set_background_color command without any pre-configured scope."
        },
        {
          "description": "Denies the set_badge_count command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-set-badge-count",
          "markdownDescription": "Denies the set_badge_count command without any pre-configured scope."
        },
        {
          "description": "Denies the set_badge_label command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-set-badge-label",
          "markdownDescription": "Denies the set_badge_label command without any pre-configured scope."
        },
        {
          "description": "Denies the set_closable command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-set-closable",
          "markdownDescription": "Denies the set_closable command without any pre-configured scope."
        },
        {
          "description": "Denies the set_content_protected command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-set-content-protected",
          "markdownDescription": "Denies the set_content_protected command without any pre-configured scope."
        },
        {
          "description": "Denies the set_cursor_grab command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-set-cursor-grab",
          "markdownDescription": "Denies the set_cursor_grab command without any pre-configured scope."
        },
        {
          "description": "Denies the set_cursor_icon command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-set-cursor-icon",
          "markdownDescription": "Denies the set_cursor_icon command without any pre-configured scope."
        },
        {
          "description": "Denies the set_cursor_position command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-set-cursor-position",
          "markdownDescription": "Denies the set_cursor_position command without any pre-configured scope."
        },
        {
          "description": "Denies the set_cursor_visible command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-set-cursor-visible",
          "markdownDescription": "Denies the set_cursor_visible command without any pre-configured scope."
        },
        {
          "description": "Denies the set_decorations command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-set-decorations",
          "markdownDescription": "Denies the set_decorations command without any pre-configured scope."
        },
        {
          "description": "Denies the set_effects command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-set-effects",
          "markdownDescription": "Denies the set_effects command without any pre-configured scope."
        },
        {
          "description": "Denies the set_enabled command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-set-enabled",
          "markdownDescription": "Denies the set_enabled command without any pre-configured scope."
        },
        {
          "description": "Denies the set_focus command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-set-focus",
          "markdownDescription": "Denies the set_focus command without any pre-configured scope."
        },
        {
          "description": "Denies the set_focusable command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-set-focusable",
          "markdownDescription": "Denies the set_focusable command without any pre-configured scope."
        },
        {
          "description": "Denies the set_fullscreen command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-set-fullscreen",
          "markdownDescription": "Denies the set_fullscreen command without any pre-configured scope."
        },
        {
          "description": "Denies the set_fullscreen_on_monitor command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-set-fullscreen-on-monitor",
          "markdownDescription": "Denies the set_fullscreen_on_monitor command without any pre-configured scope."
        },
        {
          "description": "Denies the set_icon command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-set-icon",
          "markdownDescription": "Denies the set_icon command without any pre-configured scope."
        },
        {
          "description": "Denies the set_ignore_cursor_events command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-set-ignore-cursor-events",
          "markdownDescription": "Denies the set_ignore_cursor_events command without any pre-configured scope."
        },
        {
          "description": "Denies the set_max_size command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-set-max-size",
          "markdownDescription": "Denies the set_max_size command without any pre-configured scope."
        },
        {
          "description": "Denies the set_maximizable command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-set-maximizable",
          "markdownDescription": "Denies the set_maximizable command without any pre-configured scope."
        },
        {
          "description": "Denies the set_min_size command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-set-min-size",
          "markdownDescription": "Denies the set_min_size command without any pre-configured scope."
        },
        {
          "description": "Denies the set_minimizable command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-set-minimizable",
          "markdownDescription": "Denies the set_minimizable command without any pre-configured scope."
        },
        {
          "description": "Denies the set_overlay_icon command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-set-overlay-icon",
          "markdownDescription": "Denies the set_overlay_icon command without any pre-configured scope."
        },
        {
          "description": "Denies the set_position command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-set-position",
          "markdownDescription": "Denies the set_position command without any pre-configured scope."
        },
        {
          "description": "Denies the set_progress_bar command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-set-progress-bar",
          "markdownDescription": "Denies the set_progress_bar command without any pre-configured scope."
        },
        {
          "description": "Denies the set_resizable command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-set-resizable",
          "markdownDescription": "Denies the set_resizable command without any pre-configured scope."
        },
        {
          "description": "Denies the set_shadow command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-set-shadow",
          "markdownDescription": "Denies the set_shadow command without any pre-configured scope."
        },
        {
          "description": "Denies the set_simple_fullscreen command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-set-simple-fullscreen",
          "markdownDescription": "Denies the set_simple_fullscreen command without any pre-configured scope."
        },
        {
          "description": "Denies the set_size command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-set-size",
          "markdownDescription": "Denies the set_size command without any pre-configured scope."
        },
        {
          "description": "Denies the set_size_constraints command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-set-size-constraints",
          "markdownDescription": "Denies the set_size_constraints command without any pre-configured scope."
        },
        {
          "description": "Denies the set_skip_taskbar command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-set-skip-taskbar",
          "markdownDescription": "Denies the set_skip_taskbar command without any pre-configured scope."
        },
        {
          "description": "Denies the set_theme command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-set-theme",
          "markdownDescription": "Denies the set_theme command without any pre-configured scope."
        },
        {
          "description": "Denies the set_title command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-set-title",
          "markdownDescription": "Denies the set_title command without any pre-configured scope."
        },
        {
          "description": "Denies the set_title_bar_style command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-set-title-bar-style",
          "markdownDescription": "Denies the set_title_bar_style command without any pre-configured scope."
        },
        {
          "description": "Denies the set_visible_on_all_workspaces command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-set-visible-on-all-workspaces",
          "markdownDescription": "Denies the set_visible_on_all_workspaces command without any pre-configured scope."
        },
        {
          "description": "Denies the show command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-show",
          "markdownDescription": "Denies the show command without any pre-configured scope."
        },
        {
          "description": "Denies the start_dragging command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-start-dragging",
          "markdownDescription": "Denies the start_dragging command without any pre-configured scope."
        },
        {
          "description": "Denies the start_resize_dragging command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-start-resize-dragging",
          "markdownDescription": "Denies the start_resize_dragging command without any pre-configured scope."
        },
        {
          "description": "Denies the theme command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-theme",
          "markdownDescription": "Denies the theme command without any pre-configured scope."
        },
        {
          "description": "Denies the title command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-title",
          "markdownDescription": "Denies the title command without any pre-configured scope."
        },
        {
          "description": "Denies the toggle_maximize command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-toggle-maximize",
          "markdownDescription": "Denies the toggle_maximize command without any pre-configured scope."
        },
        {
          "description": "Denies the unmaximize command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-unmaximize",
          "markdownDescription": "Denies the unmaximize command without any pre-configured scope."
        },
        {
          "description": "Denies the unminimize command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-unminimize",
          "markdownDescription": "Denies the unminimize command without any pre-configured scope."
        }
      ]
    },
    "Value": {
      "description": "All supported ACL values.",
      "anyOf": [
        {
          "description": "Represents a null JSON value.",
          "type": "null"
        },
        {
          "description": "Represents a [`bool`].",
          "type": "boolean"
        },
        {
          "description": "Represents a valid ACL [`Number`].",
          "allOf": [
            {
              "$ref": "#/definitions/Number"
            }
          ]
        },
        {
          "description": "Represents a [`String`].",
          "type": "string"
        },
        {
          "description": "Represents a list of other [`Value`]s.",
          "type": "array",
          "items": {
            "$ref": "#/definitions/Value"
          }
        },
        {
          "description": "Represents a map of [`String`] keys to [`Value`]s.",
          "type": "object",
          "additionalProperties": {
            "$ref": "#/definitions/Value"
          }
        }
      ]
    },
    "Number": {
      "description": "A valid ACL number.",
      "anyOf": [
        {
          "description": "Represents an [`i64`].",
          "type": "integer",
          "format": "int64"
        },
        {
          "description": "Represents a [`f64`].",
          "type": "number",
          "format": "double"
        }
      ]
    },
    "Target": {
      "description": "Platform target.",
      "oneOf": [
        {
          "description": "MacOS.",
          "type": "string",
          "enum": [
            "macOS"
          ]
        },
        {
          "description": "Windows.",
          "type": "string",
          "enum": [
            "windows"
          ]
        },
        {
          "description": "Linux.",
          "type": "string",
          "enum": [
            "linux"
          ]
        },
        {
          "description": "Android.",
          "type": "string",
          "enum": [
            "android"
          ]
        },
        {
          "description": "iOS.",
          "type": "string",
          "enum": [
            "iOS"
          ]
        }
      ]
    }
  }
}
````

## File: crates/ghost-updater-tauri/src-tauri/gen/schemas/windows-schema.json
````json
{
  "$schema": "http://json-schema.org/draft-07/schema#",
  "title": "CapabilityFile",
  "description": "Capability formats accepted in a capability file.",
  "anyOf": [
    {
      "description": "A single capability.",
      "allOf": [
        {
          "$ref": "#/definitions/Capability"
        }
      ]
    },
    {
      "description": "A list of capabilities.",
      "type": "array",
      "items": {
        "$ref": "#/definitions/Capability"
      }
    },
    {
      "description": "A list of capabilities.",
      "type": "object",
      "required": [
        "capabilities"
      ],
      "properties": {
        "capabilities": {
          "description": "The list of capabilities.",
          "type": "array",
          "items": {
            "$ref": "#/definitions/Capability"
          }
        }
      }
    }
  ],
  "definitions": {
    "Capability": {
      "description": "A grouping and boundary mechanism developers can use to isolate access to the IPC layer.\n\nIt controls application windows' and webviews' fine grained access to the Tauri core, application, or plugin commands. If a webview or its window is not matching any capability then it has no access to the IPC layer at all.\n\nThis can be done to create groups of windows, based on their required system access, which can reduce impact of frontend vulnerabilities in less privileged windows. Windows can be added to a capability by exact name (e.g. `main-window`) or glob patterns like `*` or `admin-*`. A Window can have none, one, or multiple associated capabilities.\n\n## Example\n\n```json { \"identifier\": \"main-user-files-write\", \"description\": \"This capability allows the `main` window on macOS and Windows access to `filesystem` write related commands and `dialog` commands to enable programmatic access to files selected by the user.\", \"windows\": [ \"main\" ], \"permissions\": [ \"core:default\", \"dialog:open\", { \"identifier\": \"fs:allow-write-text-file\", \"allow\": [{ \"path\": \"$HOME/test.txt\" }] }, ], \"platforms\": [\"macOS\",\"windows\"] } ```",
      "type": "object",
      "required": [
        "identifier",
        "permissions"
      ],
      "properties": {
        "identifier": {
          "description": "Identifier of the capability.\n\n## Example\n\n`main-user-files-write`",
          "type": "string"
        },
        "description": {
          "description": "Description of what the capability is intended to allow on associated windows.\n\nIt should contain a description of what the grouped permissions should allow.\n\n## Example\n\nThis capability allows the `main` window access to `filesystem` write related commands and `dialog` commands to enable programmatic access to files selected by the user.",
          "default": "",
          "type": "string"
        },
        "remote": {
          "description": "Configure remote URLs that can use the capability permissions.\n\nThis setting is optional and defaults to not being set, as our default use case is that the content is served from our local application.\n\n:::caution Make sure you understand the security implications of providing remote sources with local system access. :::\n\n## Example\n\n```json { \"urls\": [\"https://*.mydomain.dev\"] } ```",
          "anyOf": [
            {
              "$ref": "#/definitions/CapabilityRemote"
            },
            {
              "type": "null"
            }
          ]
        },
        "local": {
          "description": "Whether this capability is enabled for local app URLs or not. Defaults to `true`.",
          "default": true,
          "type": "boolean"
        },
        "windows": {
          "description": "List of windows that are affected by this capability. Can be a glob pattern.\n\nIf a window label matches any of the patterns in this list, the capability will be enabled on all the webviews of that window, regardless of the value of [`Self::webviews`].\n\nOn multiwebview windows, prefer specifying [`Self::webviews`] and omitting [`Self::windows`] for a fine grained access control.\n\n## Example\n\n`[\"main\"]`",
          "type": "array",
          "items": {
            "type": "string"
          }
        },
        "webviews": {
          "description": "List of webviews that are affected by this capability. Can be a glob pattern.\n\nThe capability will be enabled on all the webviews whose label matches any of the patterns in this list, regardless of whether the webview's window label matches a pattern in [`Self::windows`].\n\n## Example\n\n`[\"sub-webview-one\", \"sub-webview-two\"]`",
          "type": "array",
          "items": {
            "type": "string"
          }
        },
        "permissions": {
          "description": "List of permissions attached to this capability.\n\nMust include the plugin name as prefix in the form of `${plugin-name}:${permission-name}`. For commands directly implemented in the application itself only `${permission-name}` is required.\n\n## Example\n\n```json [ \"core:default\", \"shell:allow-open\", \"dialog:open\", { \"identifier\": \"fs:allow-write-text-file\", \"allow\": [{ \"path\": \"$HOME/test.txt\" }] } ] ```",
          "type": "array",
          "items": {
            "$ref": "#/definitions/PermissionEntry"
          },
          "uniqueItems": true
        },
        "platforms": {
          "description": "Limit which target platforms this capability applies to.\n\nBy default all platforms are targeted.\n\n## Example\n\n`[\"macOS\",\"windows\"]`",
          "type": [
            "array",
            "null"
          ],
          "items": {
            "$ref": "#/definitions/Target"
          }
        }
      }
    },
    "CapabilityRemote": {
      "description": "Configuration for remote URLs that are associated with the capability.",
      "type": "object",
      "required": [
        "urls"
      ],
      "properties": {
        "urls": {
          "description": "Remote domains this capability refers to using the [URLPattern standard](https://urlpattern.spec.whatwg.org/).\n\n## Examples\n\n- \"https://*.mydomain.dev\": allows subdomains of mydomain.dev - \"https://mydomain.dev/api/*\": allows any subpath of mydomain.dev/api",
          "type": "array",
          "items": {
            "type": "string"
          }
        }
      }
    },
    "PermissionEntry": {
      "description": "An entry for a permission value in a [`Capability`] can be either a raw permission [`Identifier`] or an object that references a permission and extends its scope.",
      "anyOf": [
        {
          "description": "Reference a permission or permission set by identifier.",
          "allOf": [
            {
              "$ref": "#/definitions/Identifier"
            }
          ]
        },
        {
          "description": "Reference a permission or permission set by identifier and extends its scope.",
          "type": "object",
          "allOf": [
            {
              "properties": {
                "identifier": {
                  "description": "Identifier of the permission or permission set.",
                  "allOf": [
                    {
                      "$ref": "#/definitions/Identifier"
                    }
                  ]
                },
                "allow": {
                  "description": "Data that defines what is allowed by the scope.",
                  "type": [
                    "array",
                    "null"
                  ],
                  "items": {
                    "$ref": "#/definitions/Value"
                  }
                },
                "deny": {
                  "description": "Data that defines what is denied by the scope. This should be prioritized by validation logic.",
                  "type": [
                    "array",
                    "null"
                  ],
                  "items": {
                    "$ref": "#/definitions/Value"
                  }
                }
              }
            }
          ],
          "required": [
            "identifier"
          ]
        }
      ]
    },
    "Identifier": {
      "description": "Permission identifier",
      "oneOf": [
        {
          "description": "Default core plugins set.\n#### This default permission set includes:\n\n- `core:path:default`\n- `core:event:default`\n- `core:window:default`\n- `core:webview:default`\n- `core:app:default`\n- `core:image:default`\n- `core:resources:default`\n- `core:menu:default`\n- `core:tray:default`",
          "type": "string",
          "const": "core:default",
          "markdownDescription": "Default core plugins set.\n#### This default permission set includes:\n\n- `core:path:default`\n- `core:event:default`\n- `core:window:default`\n- `core:webview:default`\n- `core:app:default`\n- `core:image:default`\n- `core:resources:default`\n- `core:menu:default`\n- `core:tray:default`"
        },
        {
          "description": "Default permissions for the plugin.\n#### This default permission set includes:\n\n- `allow-version`\n- `allow-name`\n- `allow-tauri-version`\n- `allow-identifier`\n- `allow-bundle-type`\n- `allow-register-listener`\n- `allow-remove-listener`\n- `allow-supports-multiple-windows`",
          "type": "string",
          "const": "core:app:default",
          "markdownDescription": "Default permissions for the plugin.\n#### This default permission set includes:\n\n- `allow-version`\n- `allow-name`\n- `allow-tauri-version`\n- `allow-identifier`\n- `allow-bundle-type`\n- `allow-register-listener`\n- `allow-remove-listener`\n- `allow-supports-multiple-windows`"
        },
        {
          "description": "Enables the app_hide command without any pre-configured scope.",
          "type": "string",
          "const": "core:app:allow-app-hide",
          "markdownDescription": "Enables the app_hide command without any pre-configured scope."
        },
        {
          "description": "Enables the app_show command without any pre-configured scope.",
          "type": "string",
          "const": "core:app:allow-app-show",
          "markdownDescription": "Enables the app_show command without any pre-configured scope."
        },
        {
          "description": "Enables the bundle_type command without any pre-configured scope.",
          "type": "string",
          "const": "core:app:allow-bundle-type",
          "markdownDescription": "Enables the bundle_type command without any pre-configured scope."
        },
        {
          "description": "Enables the default_window_icon command without any pre-configured scope.",
          "type": "string",
          "const": "core:app:allow-default-window-icon",
          "markdownDescription": "Enables the default_window_icon command without any pre-configured scope."
        },
        {
          "description": "Enables the exit command without any pre-configured scope.",
          "type": "string",
          "const": "core:app:allow-exit",
          "markdownDescription": "Enables the exit command without any pre-configured scope."
        },
        {
          "description": "Enables the fetch_data_store_identifiers command without any pre-configured scope.",
          "type": "string",
          "const": "core:app:allow-fetch-data-store-identifiers",
          "markdownDescription": "Enables the fetch_data_store_identifiers command without any pre-configured scope."
        },
        {
          "description": "Enables the identifier command without any pre-configured scope.",
          "type": "string",
          "const": "core:app:allow-identifier",
          "markdownDescription": "Enables the identifier command without any pre-configured scope."
        },
        {
          "description": "Enables the name command without any pre-configured scope.",
          "type": "string",
          "const": "core:app:allow-name",
          "markdownDescription": "Enables the name command without any pre-configured scope."
        },
        {
          "description": "Enables the register_listener command without any pre-configured scope.",
          "type": "string",
          "const": "core:app:allow-register-listener",
          "markdownDescription": "Enables the register_listener command without any pre-configured scope."
        },
        {
          "description": "Enables the remove_data_store command without any pre-configured scope.",
          "type": "string",
          "const": "core:app:allow-remove-data-store",
          "markdownDescription": "Enables the remove_data_store command without any pre-configured scope."
        },
        {
          "description": "Enables the remove_listener command without any pre-configured scope.",
          "type": "string",
          "const": "core:app:allow-remove-listener",
          "markdownDescription": "Enables the remove_listener command without any pre-configured scope."
        },
        {
          "description": "Enables the set_app_theme command without any pre-configured scope.",
          "type": "string",
          "const": "core:app:allow-set-app-theme",
          "markdownDescription": "Enables the set_app_theme command without any pre-configured scope."
        },
        {
          "description": "Enables the set_dock_visibility command without any pre-configured scope.",
          "type": "string",
          "const": "core:app:allow-set-dock-visibility",
          "markdownDescription": "Enables the set_dock_visibility command without any pre-configured scope."
        },
        {
          "description": "Enables the supports_multiple_windows command without any pre-configured scope.",
          "type": "string",
          "const": "core:app:allow-supports-multiple-windows",
          "markdownDescription": "Enables the supports_multiple_windows command without any pre-configured scope."
        },
        {
          "description": "Enables the tauri_version command without any pre-configured scope.",
          "type": "string",
          "const": "core:app:allow-tauri-version",
          "markdownDescription": "Enables the tauri_version command without any pre-configured scope."
        },
        {
          "description": "Enables the version command without any pre-configured scope.",
          "type": "string",
          "const": "core:app:allow-version",
          "markdownDescription": "Enables the version command without any pre-configured scope."
        },
        {
          "description": "Denies the app_hide command without any pre-configured scope.",
          "type": "string",
          "const": "core:app:deny-app-hide",
          "markdownDescription": "Denies the app_hide command without any pre-configured scope."
        },
        {
          "description": "Denies the app_show command without any pre-configured scope.",
          "type": "string",
          "const": "core:app:deny-app-show",
          "markdownDescription": "Denies the app_show command without any pre-configured scope."
        },
        {
          "description": "Denies the bundle_type command without any pre-configured scope.",
          "type": "string",
          "const": "core:app:deny-bundle-type",
          "markdownDescription": "Denies the bundle_type command without any pre-configured scope."
        },
        {
          "description": "Denies the default_window_icon command without any pre-configured scope.",
          "type": "string",
          "const": "core:app:deny-default-window-icon",
          "markdownDescription": "Denies the default_window_icon command without any pre-configured scope."
        },
        {
          "description": "Denies the exit command without any pre-configured scope.",
          "type": "string",
          "const": "core:app:deny-exit",
          "markdownDescription": "Denies the exit command without any pre-configured scope."
        },
        {
          "description": "Denies the fetch_data_store_identifiers command without any pre-configured scope.",
          "type": "string",
          "const": "core:app:deny-fetch-data-store-identifiers",
          "markdownDescription": "Denies the fetch_data_store_identifiers command without any pre-configured scope."
        },
        {
          "description": "Denies the identifier command without any pre-configured scope.",
          "type": "string",
          "const": "core:app:deny-identifier",
          "markdownDescription": "Denies the identifier command without any pre-configured scope."
        },
        {
          "description": "Denies the name command without any pre-configured scope.",
          "type": "string",
          "const": "core:app:deny-name",
          "markdownDescription": "Denies the name command without any pre-configured scope."
        },
        {
          "description": "Denies the register_listener command without any pre-configured scope.",
          "type": "string",
          "const": "core:app:deny-register-listener",
          "markdownDescription": "Denies the register_listener command without any pre-configured scope."
        },
        {
          "description": "Denies the remove_data_store command without any pre-configured scope.",
          "type": "string",
          "const": "core:app:deny-remove-data-store",
          "markdownDescription": "Denies the remove_data_store command without any pre-configured scope."
        },
        {
          "description": "Denies the remove_listener command without any pre-configured scope.",
          "type": "string",
          "const": "core:app:deny-remove-listener",
          "markdownDescription": "Denies the remove_listener command without any pre-configured scope."
        },
        {
          "description": "Denies the set_app_theme command without any pre-configured scope.",
          "type": "string",
          "const": "core:app:deny-set-app-theme",
          "markdownDescription": "Denies the set_app_theme command without any pre-configured scope."
        },
        {
          "description": "Denies the set_dock_visibility command without any pre-configured scope.",
          "type": "string",
          "const": "core:app:deny-set-dock-visibility",
          "markdownDescription": "Denies the set_dock_visibility command without any pre-configured scope."
        },
        {
          "description": "Denies the supports_multiple_windows command without any pre-configured scope.",
          "type": "string",
          "const": "core:app:deny-supports-multiple-windows",
          "markdownDescription": "Denies the supports_multiple_windows command without any pre-configured scope."
        },
        {
          "description": "Denies the tauri_version command without any pre-configured scope.",
          "type": "string",
          "const": "core:app:deny-tauri-version",
          "markdownDescription": "Denies the tauri_version command without any pre-configured scope."
        },
        {
          "description": "Denies the version command without any pre-configured scope.",
          "type": "string",
          "const": "core:app:deny-version",
          "markdownDescription": "Denies the version command without any pre-configured scope."
        },
        {
          "description": "Default permissions for the plugin, which enables all commands.\n#### This default permission set includes:\n\n- `allow-listen`\n- `allow-unlisten`\n- `allow-emit`\n- `allow-emit-to`",
          "type": "string",
          "const": "core:event:default",
          "markdownDescription": "Default permissions for the plugin, which enables all commands.\n#### This default permission set includes:\n\n- `allow-listen`\n- `allow-unlisten`\n- `allow-emit`\n- `allow-emit-to`"
        },
        {
          "description": "Enables the emit command without any pre-configured scope.",
          "type": "string",
          "const": "core:event:allow-emit",
          "markdownDescription": "Enables the emit command without any pre-configured scope."
        },
        {
          "description": "Enables the emit_to command without any pre-configured scope.",
          "type": "string",
          "const": "core:event:allow-emit-to",
          "markdownDescription": "Enables the emit_to command without any pre-configured scope."
        },
        {
          "description": "Enables the listen command without any pre-configured scope.",
          "type": "string",
          "const": "core:event:allow-listen",
          "markdownDescription": "Enables the listen command without any pre-configured scope."
        },
        {
          "description": "Enables the unlisten command without any pre-configured scope.",
          "type": "string",
          "const": "core:event:allow-unlisten",
          "markdownDescription": "Enables the unlisten command without any pre-configured scope."
        },
        {
          "description": "Denies the emit command without any pre-configured scope.",
          "type": "string",
          "const": "core:event:deny-emit",
          "markdownDescription": "Denies the emit command without any pre-configured scope."
        },
        {
          "description": "Denies the emit_to command without any pre-configured scope.",
          "type": "string",
          "const": "core:event:deny-emit-to",
          "markdownDescription": "Denies the emit_to command without any pre-configured scope."
        },
        {
          "description": "Denies the listen command without any pre-configured scope.",
          "type": "string",
          "const": "core:event:deny-listen",
          "markdownDescription": "Denies the listen command without any pre-configured scope."
        },
        {
          "description": "Denies the unlisten command without any pre-configured scope.",
          "type": "string",
          "const": "core:event:deny-unlisten",
          "markdownDescription": "Denies the unlisten command without any pre-configured scope."
        },
        {
          "description": "Default permissions for the plugin, which enables all commands.\n#### This default permission set includes:\n\n- `allow-new`\n- `allow-from-bytes`\n- `allow-from-path`\n- `allow-rgba`\n- `allow-size`",
          "type": "string",
          "const": "core:image:default",
          "markdownDescription": "Default permissions for the plugin, which enables all commands.\n#### This default permission set includes:\n\n- `allow-new`\n- `allow-from-bytes`\n- `allow-from-path`\n- `allow-rgba`\n- `allow-size`"
        },
        {
          "description": "Enables the from_bytes command without any pre-configured scope.",
          "type": "string",
          "const": "core:image:allow-from-bytes",
          "markdownDescription": "Enables the from_bytes command without any pre-configured scope."
        },
        {
          "description": "Enables the from_path command without any pre-configured scope.",
          "type": "string",
          "const": "core:image:allow-from-path",
          "markdownDescription": "Enables the from_path command without any pre-configured scope."
        },
        {
          "description": "Enables the new command without any pre-configured scope.",
          "type": "string",
          "const": "core:image:allow-new",
          "markdownDescription": "Enables the new command without any pre-configured scope."
        },
        {
          "description": "Enables the rgba command without any pre-configured scope.",
          "type": "string",
          "const": "core:image:allow-rgba",
          "markdownDescription": "Enables the rgba command without any pre-configured scope."
        },
        {
          "description": "Enables the size command without any pre-configured scope.",
          "type": "string",
          "const": "core:image:allow-size",
          "markdownDescription": "Enables the size command without any pre-configured scope."
        },
        {
          "description": "Denies the from_bytes command without any pre-configured scope.",
          "type": "string",
          "const": "core:image:deny-from-bytes",
          "markdownDescription": "Denies the from_bytes command without any pre-configured scope."
        },
        {
          "description": "Denies the from_path command without any pre-configured scope.",
          "type": "string",
          "const": "core:image:deny-from-path",
          "markdownDescription": "Denies the from_path command without any pre-configured scope."
        },
        {
          "description": "Denies the new command without any pre-configured scope.",
          "type": "string",
          "const": "core:image:deny-new",
          "markdownDescription": "Denies the new command without any pre-configured scope."
        },
        {
          "description": "Denies the rgba command without any pre-configured scope.",
          "type": "string",
          "const": "core:image:deny-rgba",
          "markdownDescription": "Denies the rgba command without any pre-configured scope."
        },
        {
          "description": "Denies the size command without any pre-configured scope.",
          "type": "string",
          "const": "core:image:deny-size",
          "markdownDescription": "Denies the size command without any pre-configured scope."
        },
        {
          "description": "Default permissions for the plugin, which enables all commands.\n#### This default permission set includes:\n\n- `allow-new`\n- `allow-append`\n- `allow-prepend`\n- `allow-insert`\n- `allow-remove`\n- `allow-remove-at`\n- `allow-items`\n- `allow-get`\n- `allow-popup`\n- `allow-create-default`\n- `allow-set-as-app-menu`\n- `allow-set-as-window-menu`\n- `allow-text`\n- `allow-set-text`\n- `allow-is-enabled`\n- `allow-set-enabled`\n- `allow-set-accelerator`\n- `allow-set-as-windows-menu-for-nsapp`\n- `allow-set-as-help-menu-for-nsapp`\n- `allow-is-checked`\n- `allow-set-checked`\n- `allow-set-icon`",
          "type": "string",
          "const": "core:menu:default",
          "markdownDescription": "Default permissions for the plugin, which enables all commands.\n#### This default permission set includes:\n\n- `allow-new`\n- `allow-append`\n- `allow-prepend`\n- `allow-insert`\n- `allow-remove`\n- `allow-remove-at`\n- `allow-items`\n- `allow-get`\n- `allow-popup`\n- `allow-create-default`\n- `allow-set-as-app-menu`\n- `allow-set-as-window-menu`\n- `allow-text`\n- `allow-set-text`\n- `allow-is-enabled`\n- `allow-set-enabled`\n- `allow-set-accelerator`\n- `allow-set-as-windows-menu-for-nsapp`\n- `allow-set-as-help-menu-for-nsapp`\n- `allow-is-checked`\n- `allow-set-checked`\n- `allow-set-icon`"
        },
        {
          "description": "Enables the append command without any pre-configured scope.",
          "type": "string",
          "const": "core:menu:allow-append",
          "markdownDescription": "Enables the append command without any pre-configured scope."
        },
        {
          "description": "Enables the create_default command without any pre-configured scope.",
          "type": "string",
          "const": "core:menu:allow-create-default",
          "markdownDescription": "Enables the create_default command without any pre-configured scope."
        },
        {
          "description": "Enables the get command without any pre-configured scope.",
          "type": "string",
          "const": "core:menu:allow-get",
          "markdownDescription": "Enables the get command without any pre-configured scope."
        },
        {
          "description": "Enables the insert command without any pre-configured scope.",
          "type": "string",
          "const": "core:menu:allow-insert",
          "markdownDescription": "Enables the insert command without any pre-configured scope."
        },
        {
          "description": "Enables the is_checked command without any pre-configured scope.",
          "type": "string",
          "const": "core:menu:allow-is-checked",
          "markdownDescription": "Enables the is_checked command without any pre-configured scope."
        },
        {
          "description": "Enables the is_enabled command without any pre-configured scope.",
          "type": "string",
          "const": "core:menu:allow-is-enabled",
          "markdownDescription": "Enables the is_enabled command without any pre-configured scope."
        },
        {
          "description": "Enables the items command without any pre-configured scope.",
          "type": "string",
          "const": "core:menu:allow-items",
          "markdownDescription": "Enables the items command without any pre-configured scope."
        },
        {
          "description": "Enables the new command without any pre-configured scope.",
          "type": "string",
          "const": "core:menu:allow-new",
          "markdownDescription": "Enables the new command without any pre-configured scope."
        },
        {
          "description": "Enables the popup command without any pre-configured scope.",
          "type": "string",
          "const": "core:menu:allow-popup",
          "markdownDescription": "Enables the popup command without any pre-configured scope."
        },
        {
          "description": "Enables the prepend command without any pre-configured scope.",
          "type": "string",
          "const": "core:menu:allow-prepend",
          "markdownDescription": "Enables the prepend command without any pre-configured scope."
        },
        {
          "description": "Enables the remove command without any pre-configured scope.",
          "type": "string",
          "const": "core:menu:allow-remove",
          "markdownDescription": "Enables the remove command without any pre-configured scope."
        },
        {
          "description": "Enables the remove_at command without any pre-configured scope.",
          "type": "string",
          "const": "core:menu:allow-remove-at",
          "markdownDescription": "Enables the remove_at command without any pre-configured scope."
        },
        {
          "description": "Enables the set_accelerator command without any pre-configured scope.",
          "type": "string",
          "const": "core:menu:allow-set-accelerator",
          "markdownDescription": "Enables the set_accelerator command without any pre-configured scope."
        },
        {
          "description": "Enables the set_as_app_menu command without any pre-configured scope.",
          "type": "string",
          "const": "core:menu:allow-set-as-app-menu",
          "markdownDescription": "Enables the set_as_app_menu command without any pre-configured scope."
        },
        {
          "description": "Enables the set_as_help_menu_for_nsapp command without any pre-configured scope.",
          "type": "string",
          "const": "core:menu:allow-set-as-help-menu-for-nsapp",
          "markdownDescription": "Enables the set_as_help_menu_for_nsapp command without any pre-configured scope."
        },
        {
          "description": "Enables the set_as_window_menu command without any pre-configured scope.",
          "type": "string",
          "const": "core:menu:allow-set-as-window-menu",
          "markdownDescription": "Enables the set_as_window_menu command without any pre-configured scope."
        },
        {
          "description": "Enables the set_as_windows_menu_for_nsapp command without any pre-configured scope.",
          "type": "string",
          "const": "core:menu:allow-set-as-windows-menu-for-nsapp",
          "markdownDescription": "Enables the set_as_windows_menu_for_nsapp command without any pre-configured scope."
        },
        {
          "description": "Enables the set_checked command without any pre-configured scope.",
          "type": "string",
          "const": "core:menu:allow-set-checked",
          "markdownDescription": "Enables the set_checked command without any pre-configured scope."
        },
        {
          "description": "Enables the set_enabled command without any pre-configured scope.",
          "type": "string",
          "const": "core:menu:allow-set-enabled",
          "markdownDescription": "Enables the set_enabled command without any pre-configured scope."
        },
        {
          "description": "Enables the set_icon command without any pre-configured scope.",
          "type": "string",
          "const": "core:menu:allow-set-icon",
          "markdownDescription": "Enables the set_icon command without any pre-configured scope."
        },
        {
          "description": "Enables the set_text command without any pre-configured scope.",
          "type": "string",
          "const": "core:menu:allow-set-text",
          "markdownDescription": "Enables the set_text command without any pre-configured scope."
        },
        {
          "description": "Enables the text command without any pre-configured scope.",
          "type": "string",
          "const": "core:menu:allow-text",
          "markdownDescription": "Enables the text command without any pre-configured scope."
        },
        {
          "description": "Denies the append command without any pre-configured scope.",
          "type": "string",
          "const": "core:menu:deny-append",
          "markdownDescription": "Denies the append command without any pre-configured scope."
        },
        {
          "description": "Denies the create_default command without any pre-configured scope.",
          "type": "string",
          "const": "core:menu:deny-create-default",
          "markdownDescription": "Denies the create_default command without any pre-configured scope."
        },
        {
          "description": "Denies the get command without any pre-configured scope.",
          "type": "string",
          "const": "core:menu:deny-get",
          "markdownDescription": "Denies the get command without any pre-configured scope."
        },
        {
          "description": "Denies the insert command without any pre-configured scope.",
          "type": "string",
          "const": "core:menu:deny-insert",
          "markdownDescription": "Denies the insert command without any pre-configured scope."
        },
        {
          "description": "Denies the is_checked command without any pre-configured scope.",
          "type": "string",
          "const": "core:menu:deny-is-checked",
          "markdownDescription": "Denies the is_checked command without any pre-configured scope."
        },
        {
          "description": "Denies the is_enabled command without any pre-configured scope.",
          "type": "string",
          "const": "core:menu:deny-is-enabled",
          "markdownDescription": "Denies the is_enabled command without any pre-configured scope."
        },
        {
          "description": "Denies the items command without any pre-configured scope.",
          "type": "string",
          "const": "core:menu:deny-items",
          "markdownDescription": "Denies the items command without any pre-configured scope."
        },
        {
          "description": "Denies the new command without any pre-configured scope.",
          "type": "string",
          "const": "core:menu:deny-new",
          "markdownDescription": "Denies the new command without any pre-configured scope."
        },
        {
          "description": "Denies the popup command without any pre-configured scope.",
          "type": "string",
          "const": "core:menu:deny-popup",
          "markdownDescription": "Denies the popup command without any pre-configured scope."
        },
        {
          "description": "Denies the prepend command without any pre-configured scope.",
          "type": "string",
          "const": "core:menu:deny-prepend",
          "markdownDescription": "Denies the prepend command without any pre-configured scope."
        },
        {
          "description": "Denies the remove command without any pre-configured scope.",
          "type": "string",
          "const": "core:menu:deny-remove",
          "markdownDescription": "Denies the remove command without any pre-configured scope."
        },
        {
          "description": "Denies the remove_at command without any pre-configured scope.",
          "type": "string",
          "const": "core:menu:deny-remove-at",
          "markdownDescription": "Denies the remove_at command without any pre-configured scope."
        },
        {
          "description": "Denies the set_accelerator command without any pre-configured scope.",
          "type": "string",
          "const": "core:menu:deny-set-accelerator",
          "markdownDescription": "Denies the set_accelerator command without any pre-configured scope."
        },
        {
          "description": "Denies the set_as_app_menu command without any pre-configured scope.",
          "type": "string",
          "const": "core:menu:deny-set-as-app-menu",
          "markdownDescription": "Denies the set_as_app_menu command without any pre-configured scope."
        },
        {
          "description": "Denies the set_as_help_menu_for_nsapp command without any pre-configured scope.",
          "type": "string",
          "const": "core:menu:deny-set-as-help-menu-for-nsapp",
          "markdownDescription": "Denies the set_as_help_menu_for_nsapp command without any pre-configured scope."
        },
        {
          "description": "Denies the set_as_window_menu command without any pre-configured scope.",
          "type": "string",
          "const": "core:menu:deny-set-as-window-menu",
          "markdownDescription": "Denies the set_as_window_menu command without any pre-configured scope."
        },
        {
          "description": "Denies the set_as_windows_menu_for_nsapp command without any pre-configured scope.",
          "type": "string",
          "const": "core:menu:deny-set-as-windows-menu-for-nsapp",
          "markdownDescription": "Denies the set_as_windows_menu_for_nsapp command without any pre-configured scope."
        },
        {
          "description": "Denies the set_checked command without any pre-configured scope.",
          "type": "string",
          "const": "core:menu:deny-set-checked",
          "markdownDescription": "Denies the set_checked command without any pre-configured scope."
        },
        {
          "description": "Denies the set_enabled command without any pre-configured scope.",
          "type": "string",
          "const": "core:menu:deny-set-enabled",
          "markdownDescription": "Denies the set_enabled command without any pre-configured scope."
        },
        {
          "description": "Denies the set_icon command without any pre-configured scope.",
          "type": "string",
          "const": "core:menu:deny-set-icon",
          "markdownDescription": "Denies the set_icon command without any pre-configured scope."
        },
        {
          "description": "Denies the set_text command without any pre-configured scope.",
          "type": "string",
          "const": "core:menu:deny-set-text",
          "markdownDescription": "Denies the set_text command without any pre-configured scope."
        },
        {
          "description": "Denies the text command without any pre-configured scope.",
          "type": "string",
          "const": "core:menu:deny-text",
          "markdownDescription": "Denies the text command without any pre-configured scope."
        },
        {
          "description": "Default permissions for the plugin, which enables all commands.\n#### This default permission set includes:\n\n- `allow-resolve-directory`\n- `allow-resolve`\n- `allow-normalize`\n- `allow-join`\n- `allow-dirname`\n- `allow-extname`\n- `allow-basename`\n- `allow-is-absolute`",
          "type": "string",
          "const": "core:path:default",
          "markdownDescription": "Default permissions for the plugin, which enables all commands.\n#### This default permission set includes:\n\n- `allow-resolve-directory`\n- `allow-resolve`\n- `allow-normalize`\n- `allow-join`\n- `allow-dirname`\n- `allow-extname`\n- `allow-basename`\n- `allow-is-absolute`"
        },
        {
          "description": "Enables the basename command without any pre-configured scope.",
          "type": "string",
          "const": "core:path:allow-basename",
          "markdownDescription": "Enables the basename command without any pre-configured scope."
        },
        {
          "description": "Enables the dirname command without any pre-configured scope.",
          "type": "string",
          "const": "core:path:allow-dirname",
          "markdownDescription": "Enables the dirname command without any pre-configured scope."
        },
        {
          "description": "Enables the extname command without any pre-configured scope.",
          "type": "string",
          "const": "core:path:allow-extname",
          "markdownDescription": "Enables the extname command without any pre-configured scope."
        },
        {
          "description": "Enables the is_absolute command without any pre-configured scope.",
          "type": "string",
          "const": "core:path:allow-is-absolute",
          "markdownDescription": "Enables the is_absolute command without any pre-configured scope."
        },
        {
          "description": "Enables the join command without any pre-configured scope.",
          "type": "string",
          "const": "core:path:allow-join",
          "markdownDescription": "Enables the join command without any pre-configured scope."
        },
        {
          "description": "Enables the normalize command without any pre-configured scope.",
          "type": "string",
          "const": "core:path:allow-normalize",
          "markdownDescription": "Enables the normalize command without any pre-configured scope."
        },
        {
          "description": "Enables the resolve command without any pre-configured scope.",
          "type": "string",
          "const": "core:path:allow-resolve",
          "markdownDescription": "Enables the resolve command without any pre-configured scope."
        },
        {
          "description": "Enables the resolve_directory command without any pre-configured scope.",
          "type": "string",
          "const": "core:path:allow-resolve-directory",
          "markdownDescription": "Enables the resolve_directory command without any pre-configured scope."
        },
        {
          "description": "Denies the basename command without any pre-configured scope.",
          "type": "string",
          "const": "core:path:deny-basename",
          "markdownDescription": "Denies the basename command without any pre-configured scope."
        },
        {
          "description": "Denies the dirname command without any pre-configured scope.",
          "type": "string",
          "const": "core:path:deny-dirname",
          "markdownDescription": "Denies the dirname command without any pre-configured scope."
        },
        {
          "description": "Denies the extname command without any pre-configured scope.",
          "type": "string",
          "const": "core:path:deny-extname",
          "markdownDescription": "Denies the extname command without any pre-configured scope."
        },
        {
          "description": "Denies the is_absolute command without any pre-configured scope.",
          "type": "string",
          "const": "core:path:deny-is-absolute",
          "markdownDescription": "Denies the is_absolute command without any pre-configured scope."
        },
        {
          "description": "Denies the join command without any pre-configured scope.",
          "type": "string",
          "const": "core:path:deny-join",
          "markdownDescription": "Denies the join command without any pre-configured scope."
        },
        {
          "description": "Denies the normalize command without any pre-configured scope.",
          "type": "string",
          "const": "core:path:deny-normalize",
          "markdownDescription": "Denies the normalize command without any pre-configured scope."
        },
        {
          "description": "Denies the resolve command without any pre-configured scope.",
          "type": "string",
          "const": "core:path:deny-resolve",
          "markdownDescription": "Denies the resolve command without any pre-configured scope."
        },
        {
          "description": "Denies the resolve_directory command without any pre-configured scope.",
          "type": "string",
          "const": "core:path:deny-resolve-directory",
          "markdownDescription": "Denies the resolve_directory command without any pre-configured scope."
        },
        {
          "description": "Default permissions for the plugin, which enables all commands.\n#### This default permission set includes:\n\n- `allow-close`",
          "type": "string",
          "const": "core:resources:default",
          "markdownDescription": "Default permissions for the plugin, which enables all commands.\n#### This default permission set includes:\n\n- `allow-close`"
        },
        {
          "description": "Enables the close command without any pre-configured scope.",
          "type": "string",
          "const": "core:resources:allow-close",
          "markdownDescription": "Enables the close command without any pre-configured scope."
        },
        {
          "description": "Denies the close command without any pre-configured scope.",
          "type": "string",
          "const": "core:resources:deny-close",
          "markdownDescription": "Denies the close command without any pre-configured scope."
        },
        {
          "description": "Default permissions for the plugin, which enables all commands.\n#### This default permission set includes:\n\n- `allow-new`\n- `allow-get-by-id`\n- `allow-remove-by-id`\n- `allow-set-icon`\n- `allow-set-menu`\n- `allow-set-tooltip`\n- `allow-set-title`\n- `allow-set-visible`\n- `allow-set-temp-dir-path`\n- `allow-set-icon-as-template`\n- `allow-set-icon-with-as-template`\n- `allow-set-show-menu-on-left-click`",
          "type": "string",
          "const": "core:tray:default",
          "markdownDescription": "Default permissions for the plugin, which enables all commands.\n#### This default permission set includes:\n\n- `allow-new`\n- `allow-get-by-id`\n- `allow-remove-by-id`\n- `allow-set-icon`\n- `allow-set-menu`\n- `allow-set-tooltip`\n- `allow-set-title`\n- `allow-set-visible`\n- `allow-set-temp-dir-path`\n- `allow-set-icon-as-template`\n- `allow-set-icon-with-as-template`\n- `allow-set-show-menu-on-left-click`"
        },
        {
          "description": "Enables the get_by_id command without any pre-configured scope.",
          "type": "string",
          "const": "core:tray:allow-get-by-id",
          "markdownDescription": "Enables the get_by_id command without any pre-configured scope."
        },
        {
          "description": "Enables the new command without any pre-configured scope.",
          "type": "string",
          "const": "core:tray:allow-new",
          "markdownDescription": "Enables the new command without any pre-configured scope."
        },
        {
          "description": "Enables the remove_by_id command without any pre-configured scope.",
          "type": "string",
          "const": "core:tray:allow-remove-by-id",
          "markdownDescription": "Enables the remove_by_id command without any pre-configured scope."
        },
        {
          "description": "Enables the set_icon command without any pre-configured scope.",
          "type": "string",
          "const": "core:tray:allow-set-icon",
          "markdownDescription": "Enables the set_icon command without any pre-configured scope."
        },
        {
          "description": "Enables the set_icon_as_template command without any pre-configured scope.",
          "type": "string",
          "const": "core:tray:allow-set-icon-as-template",
          "markdownDescription": "Enables the set_icon_as_template command without any pre-configured scope."
        },
        {
          "description": "Enables the set_icon_with_as_template command without any pre-configured scope.",
          "type": "string",
          "const": "core:tray:allow-set-icon-with-as-template",
          "markdownDescription": "Enables the set_icon_with_as_template command without any pre-configured scope."
        },
        {
          "description": "Enables the set_menu command without any pre-configured scope.",
          "type": "string",
          "const": "core:tray:allow-set-menu",
          "markdownDescription": "Enables the set_menu command without any pre-configured scope."
        },
        {
          "description": "Enables the set_show_menu_on_left_click command without any pre-configured scope.",
          "type": "string",
          "const": "core:tray:allow-set-show-menu-on-left-click",
          "markdownDescription": "Enables the set_show_menu_on_left_click command without any pre-configured scope."
        },
        {
          "description": "Enables the set_temp_dir_path command without any pre-configured scope.",
          "type": "string",
          "const": "core:tray:allow-set-temp-dir-path",
          "markdownDescription": "Enables the set_temp_dir_path command without any pre-configured scope."
        },
        {
          "description": "Enables the set_title command without any pre-configured scope.",
          "type": "string",
          "const": "core:tray:allow-set-title",
          "markdownDescription": "Enables the set_title command without any pre-configured scope."
        },
        {
          "description": "Enables the set_tooltip command without any pre-configured scope.",
          "type": "string",
          "const": "core:tray:allow-set-tooltip",
          "markdownDescription": "Enables the set_tooltip command without any pre-configured scope."
        },
        {
          "description": "Enables the set_visible command without any pre-configured scope.",
          "type": "string",
          "const": "core:tray:allow-set-visible",
          "markdownDescription": "Enables the set_visible command without any pre-configured scope."
        },
        {
          "description": "Denies the get_by_id command without any pre-configured scope.",
          "type": "string",
          "const": "core:tray:deny-get-by-id",
          "markdownDescription": "Denies the get_by_id command without any pre-configured scope."
        },
        {
          "description": "Denies the new command without any pre-configured scope.",
          "type": "string",
          "const": "core:tray:deny-new",
          "markdownDescription": "Denies the new command without any pre-configured scope."
        },
        {
          "description": "Denies the remove_by_id command without any pre-configured scope.",
          "type": "string",
          "const": "core:tray:deny-remove-by-id",
          "markdownDescription": "Denies the remove_by_id command without any pre-configured scope."
        },
        {
          "description": "Denies the set_icon command without any pre-configured scope.",
          "type": "string",
          "const": "core:tray:deny-set-icon",
          "markdownDescription": "Denies the set_icon command without any pre-configured scope."
        },
        {
          "description": "Denies the set_icon_as_template command without any pre-configured scope.",
          "type": "string",
          "const": "core:tray:deny-set-icon-as-template",
          "markdownDescription": "Denies the set_icon_as_template command without any pre-configured scope."
        },
        {
          "description": "Denies the set_icon_with_as_template command without any pre-configured scope.",
          "type": "string",
          "const": "core:tray:deny-set-icon-with-as-template",
          "markdownDescription": "Denies the set_icon_with_as_template command without any pre-configured scope."
        },
        {
          "description": "Denies the set_menu command without any pre-configured scope.",
          "type": "string",
          "const": "core:tray:deny-set-menu",
          "markdownDescription": "Denies the set_menu command without any pre-configured scope."
        },
        {
          "description": "Denies the set_show_menu_on_left_click command without any pre-configured scope.",
          "type": "string",
          "const": "core:tray:deny-set-show-menu-on-left-click",
          "markdownDescription": "Denies the set_show_menu_on_left_click command without any pre-configured scope."
        },
        {
          "description": "Denies the set_temp_dir_path command without any pre-configured scope.",
          "type": "string",
          "const": "core:tray:deny-set-temp-dir-path",
          "markdownDescription": "Denies the set_temp_dir_path command without any pre-configured scope."
        },
        {
          "description": "Denies the set_title command without any pre-configured scope.",
          "type": "string",
          "const": "core:tray:deny-set-title",
          "markdownDescription": "Denies the set_title command without any pre-configured scope."
        },
        {
          "description": "Denies the set_tooltip command without any pre-configured scope.",
          "type": "string",
          "const": "core:tray:deny-set-tooltip",
          "markdownDescription": "Denies the set_tooltip command without any pre-configured scope."
        },
        {
          "description": "Denies the set_visible command without any pre-configured scope.",
          "type": "string",
          "const": "core:tray:deny-set-visible",
          "markdownDescription": "Denies the set_visible command without any pre-configured scope."
        },
        {
          "description": "Default permissions for the plugin.\n#### This default permission set includes:\n\n- `allow-get-all-webviews`\n- `allow-webview-position`\n- `allow-webview-size`\n- `allow-internal-toggle-devtools`",
          "type": "string",
          "const": "core:webview:default",
          "markdownDescription": "Default permissions for the plugin.\n#### This default permission set includes:\n\n- `allow-get-all-webviews`\n- `allow-webview-position`\n- `allow-webview-size`\n- `allow-internal-toggle-devtools`"
        },
        {
          "description": "Enables the clear_all_browsing_data command without any pre-configured scope.",
          "type": "string",
          "const": "core:webview:allow-clear-all-browsing-data",
          "markdownDescription": "Enables the clear_all_browsing_data command without any pre-configured scope."
        },
        {
          "description": "Enables the create_webview command without any pre-configured scope.",
          "type": "string",
          "const": "core:webview:allow-create-webview",
          "markdownDescription": "Enables the create_webview command without any pre-configured scope."
        },
        {
          "description": "Enables the create_webview_window command without any pre-configured scope.",
          "type": "string",
          "const": "core:webview:allow-create-webview-window",
          "markdownDescription": "Enables the create_webview_window command without any pre-configured scope."
        },
        {
          "description": "Enables the get_all_webviews command without any pre-configured scope.",
          "type": "string",
          "const": "core:webview:allow-get-all-webviews",
          "markdownDescription": "Enables the get_all_webviews command without any pre-configured scope."
        },
        {
          "description": "Enables the internal_toggle_devtools command without any pre-configured scope.",
          "type": "string",
          "const": "core:webview:allow-internal-toggle-devtools",
          "markdownDescription": "Enables the internal_toggle_devtools command without any pre-configured scope."
        },
        {
          "description": "Enables the print command without any pre-configured scope.",
          "type": "string",
          "const": "core:webview:allow-print",
          "markdownDescription": "Enables the print command without any pre-configured scope."
        },
        {
          "description": "Enables the reparent command without any pre-configured scope.",
          "type": "string",
          "const": "core:webview:allow-reparent",
          "markdownDescription": "Enables the reparent command without any pre-configured scope."
        },
        {
          "description": "Enables the set_webview_auto_resize command without any pre-configured scope.",
          "type": "string",
          "const": "core:webview:allow-set-webview-auto-resize",
          "markdownDescription": "Enables the set_webview_auto_resize command without any pre-configured scope."
        },
        {
          "description": "Enables the set_webview_background_color command without any pre-configured scope.",
          "type": "string",
          "const": "core:webview:allow-set-webview-background-color",
          "markdownDescription": "Enables the set_webview_background_color command without any pre-configured scope."
        },
        {
          "description": "Enables the set_webview_focus command without any pre-configured scope.",
          "type": "string",
          "const": "core:webview:allow-set-webview-focus",
          "markdownDescription": "Enables the set_webview_focus command without any pre-configured scope."
        },
        {
          "description": "Enables the set_webview_position command without any pre-configured scope.",
          "type": "string",
          "const": "core:webview:allow-set-webview-position",
          "markdownDescription": "Enables the set_webview_position command without any pre-configured scope."
        },
        {
          "description": "Enables the set_webview_size command without any pre-configured scope.",
          "type": "string",
          "const": "core:webview:allow-set-webview-size",
          "markdownDescription": "Enables the set_webview_size command without any pre-configured scope."
        },
        {
          "description": "Enables the set_webview_zoom command without any pre-configured scope.",
          "type": "string",
          "const": "core:webview:allow-set-webview-zoom",
          "markdownDescription": "Enables the set_webview_zoom command without any pre-configured scope."
        },
        {
          "description": "Enables the webview_close command without any pre-configured scope.",
          "type": "string",
          "const": "core:webview:allow-webview-close",
          "markdownDescription": "Enables the webview_close command without any pre-configured scope."
        },
        {
          "description": "Enables the webview_hide command without any pre-configured scope.",
          "type": "string",
          "const": "core:webview:allow-webview-hide",
          "markdownDescription": "Enables the webview_hide command without any pre-configured scope."
        },
        {
          "description": "Enables the webview_position command without any pre-configured scope.",
          "type": "string",
          "const": "core:webview:allow-webview-position",
          "markdownDescription": "Enables the webview_position command without any pre-configured scope."
        },
        {
          "description": "Enables the webview_show command without any pre-configured scope.",
          "type": "string",
          "const": "core:webview:allow-webview-show",
          "markdownDescription": "Enables the webview_show command without any pre-configured scope."
        },
        {
          "description": "Enables the webview_size command without any pre-configured scope.",
          "type": "string",
          "const": "core:webview:allow-webview-size",
          "markdownDescription": "Enables the webview_size command without any pre-configured scope."
        },
        {
          "description": "Denies the clear_all_browsing_data command without any pre-configured scope.",
          "type": "string",
          "const": "core:webview:deny-clear-all-browsing-data",
          "markdownDescription": "Denies the clear_all_browsing_data command without any pre-configured scope."
        },
        {
          "description": "Denies the create_webview command without any pre-configured scope.",
          "type": "string",
          "const": "core:webview:deny-create-webview",
          "markdownDescription": "Denies the create_webview command without any pre-configured scope."
        },
        {
          "description": "Denies the create_webview_window command without any pre-configured scope.",
          "type": "string",
          "const": "core:webview:deny-create-webview-window",
          "markdownDescription": "Denies the create_webview_window command without any pre-configured scope."
        },
        {
          "description": "Denies the get_all_webviews command without any pre-configured scope.",
          "type": "string",
          "const": "core:webview:deny-get-all-webviews",
          "markdownDescription": "Denies the get_all_webviews command without any pre-configured scope."
        },
        {
          "description": "Denies the internal_toggle_devtools command without any pre-configured scope.",
          "type": "string",
          "const": "core:webview:deny-internal-toggle-devtools",
          "markdownDescription": "Denies the internal_toggle_devtools command without any pre-configured scope."
        },
        {
          "description": "Denies the print command without any pre-configured scope.",
          "type": "string",
          "const": "core:webview:deny-print",
          "markdownDescription": "Denies the print command without any pre-configured scope."
        },
        {
          "description": "Denies the reparent command without any pre-configured scope.",
          "type": "string",
          "const": "core:webview:deny-reparent",
          "markdownDescription": "Denies the reparent command without any pre-configured scope."
        },
        {
          "description": "Denies the set_webview_auto_resize command without any pre-configured scope.",
          "type": "string",
          "const": "core:webview:deny-set-webview-auto-resize",
          "markdownDescription": "Denies the set_webview_auto_resize command without any pre-configured scope."
        },
        {
          "description": "Denies the set_webview_background_color command without any pre-configured scope.",
          "type": "string",
          "const": "core:webview:deny-set-webview-background-color",
          "markdownDescription": "Denies the set_webview_background_color command without any pre-configured scope."
        },
        {
          "description": "Denies the set_webview_focus command without any pre-configured scope.",
          "type": "string",
          "const": "core:webview:deny-set-webview-focus",
          "markdownDescription": "Denies the set_webview_focus command without any pre-configured scope."
        },
        {
          "description": "Denies the set_webview_position command without any pre-configured scope.",
          "type": "string",
          "const": "core:webview:deny-set-webview-position",
          "markdownDescription": "Denies the set_webview_position command without any pre-configured scope."
        },
        {
          "description": "Denies the set_webview_size command without any pre-configured scope.",
          "type": "string",
          "const": "core:webview:deny-set-webview-size",
          "markdownDescription": "Denies the set_webview_size command without any pre-configured scope."
        },
        {
          "description": "Denies the set_webview_zoom command without any pre-configured scope.",
          "type": "string",
          "const": "core:webview:deny-set-webview-zoom",
          "markdownDescription": "Denies the set_webview_zoom command without any pre-configured scope."
        },
        {
          "description": "Denies the webview_close command without any pre-configured scope.",
          "type": "string",
          "const": "core:webview:deny-webview-close",
          "markdownDescription": "Denies the webview_close command without any pre-configured scope."
        },
        {
          "description": "Denies the webview_hide command without any pre-configured scope.",
          "type": "string",
          "const": "core:webview:deny-webview-hide",
          "markdownDescription": "Denies the webview_hide command without any pre-configured scope."
        },
        {
          "description": "Denies the webview_position command without any pre-configured scope.",
          "type": "string",
          "const": "core:webview:deny-webview-position",
          "markdownDescription": "Denies the webview_position command without any pre-configured scope."
        },
        {
          "description": "Denies the webview_show command without any pre-configured scope.",
          "type": "string",
          "const": "core:webview:deny-webview-show",
          "markdownDescription": "Denies the webview_show command without any pre-configured scope."
        },
        {
          "description": "Denies the webview_size command without any pre-configured scope.",
          "type": "string",
          "const": "core:webview:deny-webview-size",
          "markdownDescription": "Denies the webview_size command without any pre-configured scope."
        },
        {
          "description": "Default permissions for the plugin.\n#### This default permission set includes:\n\n- `allow-get-all-windows`\n- `allow-scale-factor`\n- `allow-inner-position`\n- `allow-outer-position`\n- `allow-inner-size`\n- `allow-outer-size`\n- `allow-is-fullscreen`\n- `allow-is-minimized`\n- `allow-is-maximized`\n- `allow-is-focused`\n- `allow-is-decorated`\n- `allow-is-resizable`\n- `allow-is-maximizable`\n- `allow-is-minimizable`\n- `allow-is-closable`\n- `allow-is-visible`\n- `allow-is-enabled`\n- `allow-title`\n- `allow-current-monitor`\n- `allow-primary-monitor`\n- `allow-monitor-from-point`\n- `allow-available-monitors`\n- `allow-cursor-position`\n- `allow-theme`\n- `allow-is-always-on-top`\n- `allow-activity-name`\n- `allow-scene-identifier`\n- `allow-internal-toggle-maximize`",
          "type": "string",
          "const": "core:window:default",
          "markdownDescription": "Default permissions for the plugin.\n#### This default permission set includes:\n\n- `allow-get-all-windows`\n- `allow-scale-factor`\n- `allow-inner-position`\n- `allow-outer-position`\n- `allow-inner-size`\n- `allow-outer-size`\n- `allow-is-fullscreen`\n- `allow-is-minimized`\n- `allow-is-maximized`\n- `allow-is-focused`\n- `allow-is-decorated`\n- `allow-is-resizable`\n- `allow-is-maximizable`\n- `allow-is-minimizable`\n- `allow-is-closable`\n- `allow-is-visible`\n- `allow-is-enabled`\n- `allow-title`\n- `allow-current-monitor`\n- `allow-primary-monitor`\n- `allow-monitor-from-point`\n- `allow-available-monitors`\n- `allow-cursor-position`\n- `allow-theme`\n- `allow-is-always-on-top`\n- `allow-activity-name`\n- `allow-scene-identifier`\n- `allow-internal-toggle-maximize`"
        },
        {
          "description": "Enables the activity_name command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-activity-name",
          "markdownDescription": "Enables the activity_name command without any pre-configured scope."
        },
        {
          "description": "Enables the available_monitors command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-available-monitors",
          "markdownDescription": "Enables the available_monitors command without any pre-configured scope."
        },
        {
          "description": "Enables the center command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-center",
          "markdownDescription": "Enables the center command without any pre-configured scope."
        },
        {
          "description": "Enables the close command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-close",
          "markdownDescription": "Enables the close command without any pre-configured scope."
        },
        {
          "description": "Enables the create command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-create",
          "markdownDescription": "Enables the create command without any pre-configured scope."
        },
        {
          "description": "Enables the current_monitor command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-current-monitor",
          "markdownDescription": "Enables the current_monitor command without any pre-configured scope."
        },
        {
          "description": "Enables the cursor_position command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-cursor-position",
          "markdownDescription": "Enables the cursor_position command without any pre-configured scope."
        },
        {
          "description": "Enables the destroy command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-destroy",
          "markdownDescription": "Enables the destroy command without any pre-configured scope."
        },
        {
          "description": "Enables the get_all_windows command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-get-all-windows",
          "markdownDescription": "Enables the get_all_windows command without any pre-configured scope."
        },
        {
          "description": "Enables the hide command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-hide",
          "markdownDescription": "Enables the hide command without any pre-configured scope."
        },
        {
          "description": "Enables the inner_position command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-inner-position",
          "markdownDescription": "Enables the inner_position command without any pre-configured scope."
        },
        {
          "description": "Enables the inner_size command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-inner-size",
          "markdownDescription": "Enables the inner_size command without any pre-configured scope."
        },
        {
          "description": "Enables the internal_toggle_maximize command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-internal-toggle-maximize",
          "markdownDescription": "Enables the internal_toggle_maximize command without any pre-configured scope."
        },
        {
          "description": "Enables the is_always_on_top command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-is-always-on-top",
          "markdownDescription": "Enables the is_always_on_top command without any pre-configured scope."
        },
        {
          "description": "Enables the is_closable command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-is-closable",
          "markdownDescription": "Enables the is_closable command without any pre-configured scope."
        },
        {
          "description": "Enables the is_decorated command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-is-decorated",
          "markdownDescription": "Enables the is_decorated command without any pre-configured scope."
        },
        {
          "description": "Enables the is_enabled command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-is-enabled",
          "markdownDescription": "Enables the is_enabled command without any pre-configured scope."
        },
        {
          "description": "Enables the is_focused command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-is-focused",
          "markdownDescription": "Enables the is_focused command without any pre-configured scope."
        },
        {
          "description": "Enables the is_fullscreen command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-is-fullscreen",
          "markdownDescription": "Enables the is_fullscreen command without any pre-configured scope."
        },
        {
          "description": "Enables the is_maximizable command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-is-maximizable",
          "markdownDescription": "Enables the is_maximizable command without any pre-configured scope."
        },
        {
          "description": "Enables the is_maximized command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-is-maximized",
          "markdownDescription": "Enables the is_maximized command without any pre-configured scope."
        },
        {
          "description": "Enables the is_minimizable command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-is-minimizable",
          "markdownDescription": "Enables the is_minimizable command without any pre-configured scope."
        },
        {
          "description": "Enables the is_minimized command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-is-minimized",
          "markdownDescription": "Enables the is_minimized command without any pre-configured scope."
        },
        {
          "description": "Enables the is_resizable command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-is-resizable",
          "markdownDescription": "Enables the is_resizable command without any pre-configured scope."
        },
        {
          "description": "Enables the is_visible command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-is-visible",
          "markdownDescription": "Enables the is_visible command without any pre-configured scope."
        },
        {
          "description": "Enables the maximize command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-maximize",
          "markdownDescription": "Enables the maximize command without any pre-configured scope."
        },
        {
          "description": "Enables the minimize command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-minimize",
          "markdownDescription": "Enables the minimize command without any pre-configured scope."
        },
        {
          "description": "Enables the monitor_from_point command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-monitor-from-point",
          "markdownDescription": "Enables the monitor_from_point command without any pre-configured scope."
        },
        {
          "description": "Enables the outer_position command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-outer-position",
          "markdownDescription": "Enables the outer_position command without any pre-configured scope."
        },
        {
          "description": "Enables the outer_size command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-outer-size",
          "markdownDescription": "Enables the outer_size command without any pre-configured scope."
        },
        {
          "description": "Enables the primary_monitor command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-primary-monitor",
          "markdownDescription": "Enables the primary_monitor command without any pre-configured scope."
        },
        {
          "description": "Enables the request_user_attention command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-request-user-attention",
          "markdownDescription": "Enables the request_user_attention command without any pre-configured scope."
        },
        {
          "description": "Enables the scale_factor command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-scale-factor",
          "markdownDescription": "Enables the scale_factor command without any pre-configured scope."
        },
        {
          "description": "Enables the scene_identifier command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-scene-identifier",
          "markdownDescription": "Enables the scene_identifier command without any pre-configured scope."
        },
        {
          "description": "Enables the set_always_on_bottom command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-set-always-on-bottom",
          "markdownDescription": "Enables the set_always_on_bottom command without any pre-configured scope."
        },
        {
          "description": "Enables the set_always_on_top command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-set-always-on-top",
          "markdownDescription": "Enables the set_always_on_top command without any pre-configured scope."
        },
        {
          "description": "Enables the set_background_color command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-set-background-color",
          "markdownDescription": "Enables the set_background_color command without any pre-configured scope."
        },
        {
          "description": "Enables the set_badge_count command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-set-badge-count",
          "markdownDescription": "Enables the set_badge_count command without any pre-configured scope."
        },
        {
          "description": "Enables the set_badge_label command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-set-badge-label",
          "markdownDescription": "Enables the set_badge_label command without any pre-configured scope."
        },
        {
          "description": "Enables the set_closable command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-set-closable",
          "markdownDescription": "Enables the set_closable command without any pre-configured scope."
        },
        {
          "description": "Enables the set_content_protected command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-set-content-protected",
          "markdownDescription": "Enables the set_content_protected command without any pre-configured scope."
        },
        {
          "description": "Enables the set_cursor_grab command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-set-cursor-grab",
          "markdownDescription": "Enables the set_cursor_grab command without any pre-configured scope."
        },
        {
          "description": "Enables the set_cursor_icon command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-set-cursor-icon",
          "markdownDescription": "Enables the set_cursor_icon command without any pre-configured scope."
        },
        {
          "description": "Enables the set_cursor_position command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-set-cursor-position",
          "markdownDescription": "Enables the set_cursor_position command without any pre-configured scope."
        },
        {
          "description": "Enables the set_cursor_visible command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-set-cursor-visible",
          "markdownDescription": "Enables the set_cursor_visible command without any pre-configured scope."
        },
        {
          "description": "Enables the set_decorations command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-set-decorations",
          "markdownDescription": "Enables the set_decorations command without any pre-configured scope."
        },
        {
          "description": "Enables the set_effects command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-set-effects",
          "markdownDescription": "Enables the set_effects command without any pre-configured scope."
        },
        {
          "description": "Enables the set_enabled command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-set-enabled",
          "markdownDescription": "Enables the set_enabled command without any pre-configured scope."
        },
        {
          "description": "Enables the set_focus command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-set-focus",
          "markdownDescription": "Enables the set_focus command without any pre-configured scope."
        },
        {
          "description": "Enables the set_focusable command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-set-focusable",
          "markdownDescription": "Enables the set_focusable command without any pre-configured scope."
        },
        {
          "description": "Enables the set_fullscreen command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-set-fullscreen",
          "markdownDescription": "Enables the set_fullscreen command without any pre-configured scope."
        },
        {
          "description": "Enables the set_fullscreen_on_monitor command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-set-fullscreen-on-monitor",
          "markdownDescription": "Enables the set_fullscreen_on_monitor command without any pre-configured scope."
        },
        {
          "description": "Enables the set_icon command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-set-icon",
          "markdownDescription": "Enables the set_icon command without any pre-configured scope."
        },
        {
          "description": "Enables the set_ignore_cursor_events command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-set-ignore-cursor-events",
          "markdownDescription": "Enables the set_ignore_cursor_events command without any pre-configured scope."
        },
        {
          "description": "Enables the set_max_size command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-set-max-size",
          "markdownDescription": "Enables the set_max_size command without any pre-configured scope."
        },
        {
          "description": "Enables the set_maximizable command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-set-maximizable",
          "markdownDescription": "Enables the set_maximizable command without any pre-configured scope."
        },
        {
          "description": "Enables the set_min_size command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-set-min-size",
          "markdownDescription": "Enables the set_min_size command without any pre-configured scope."
        },
        {
          "description": "Enables the set_minimizable command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-set-minimizable",
          "markdownDescription": "Enables the set_minimizable command without any pre-configured scope."
        },
        {
          "description": "Enables the set_overlay_icon command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-set-overlay-icon",
          "markdownDescription": "Enables the set_overlay_icon command without any pre-configured scope."
        },
        {
          "description": "Enables the set_position command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-set-position",
          "markdownDescription": "Enables the set_position command without any pre-configured scope."
        },
        {
          "description": "Enables the set_progress_bar command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-set-progress-bar",
          "markdownDescription": "Enables the set_progress_bar command without any pre-configured scope."
        },
        {
          "description": "Enables the set_resizable command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-set-resizable",
          "markdownDescription": "Enables the set_resizable command without any pre-configured scope."
        },
        {
          "description": "Enables the set_shadow command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-set-shadow",
          "markdownDescription": "Enables the set_shadow command without any pre-configured scope."
        },
        {
          "description": "Enables the set_simple_fullscreen command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-set-simple-fullscreen",
          "markdownDescription": "Enables the set_simple_fullscreen command without any pre-configured scope."
        },
        {
          "description": "Enables the set_size command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-set-size",
          "markdownDescription": "Enables the set_size command without any pre-configured scope."
        },
        {
          "description": "Enables the set_size_constraints command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-set-size-constraints",
          "markdownDescription": "Enables the set_size_constraints command without any pre-configured scope."
        },
        {
          "description": "Enables the set_skip_taskbar command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-set-skip-taskbar",
          "markdownDescription": "Enables the set_skip_taskbar command without any pre-configured scope."
        },
        {
          "description": "Enables the set_theme command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-set-theme",
          "markdownDescription": "Enables the set_theme command without any pre-configured scope."
        },
        {
          "description": "Enables the set_title command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-set-title",
          "markdownDescription": "Enables the set_title command without any pre-configured scope."
        },
        {
          "description": "Enables the set_title_bar_style command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-set-title-bar-style",
          "markdownDescription": "Enables the set_title_bar_style command without any pre-configured scope."
        },
        {
          "description": "Enables the set_visible_on_all_workspaces command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-set-visible-on-all-workspaces",
          "markdownDescription": "Enables the set_visible_on_all_workspaces command without any pre-configured scope."
        },
        {
          "description": "Enables the show command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-show",
          "markdownDescription": "Enables the show command without any pre-configured scope."
        },
        {
          "description": "Enables the start_dragging command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-start-dragging",
          "markdownDescription": "Enables the start_dragging command without any pre-configured scope."
        },
        {
          "description": "Enables the start_resize_dragging command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-start-resize-dragging",
          "markdownDescription": "Enables the start_resize_dragging command without any pre-configured scope."
        },
        {
          "description": "Enables the theme command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-theme",
          "markdownDescription": "Enables the theme command without any pre-configured scope."
        },
        {
          "description": "Enables the title command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-title",
          "markdownDescription": "Enables the title command without any pre-configured scope."
        },
        {
          "description": "Enables the toggle_maximize command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-toggle-maximize",
          "markdownDescription": "Enables the toggle_maximize command without any pre-configured scope."
        },
        {
          "description": "Enables the unmaximize command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-unmaximize",
          "markdownDescription": "Enables the unmaximize command without any pre-configured scope."
        },
        {
          "description": "Enables the unminimize command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:allow-unminimize",
          "markdownDescription": "Enables the unminimize command without any pre-configured scope."
        },
        {
          "description": "Denies the activity_name command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-activity-name",
          "markdownDescription": "Denies the activity_name command without any pre-configured scope."
        },
        {
          "description": "Denies the available_monitors command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-available-monitors",
          "markdownDescription": "Denies the available_monitors command without any pre-configured scope."
        },
        {
          "description": "Denies the center command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-center",
          "markdownDescription": "Denies the center command without any pre-configured scope."
        },
        {
          "description": "Denies the close command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-close",
          "markdownDescription": "Denies the close command without any pre-configured scope."
        },
        {
          "description": "Denies the create command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-create",
          "markdownDescription": "Denies the create command without any pre-configured scope."
        },
        {
          "description": "Denies the current_monitor command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-current-monitor",
          "markdownDescription": "Denies the current_monitor command without any pre-configured scope."
        },
        {
          "description": "Denies the cursor_position command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-cursor-position",
          "markdownDescription": "Denies the cursor_position command without any pre-configured scope."
        },
        {
          "description": "Denies the destroy command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-destroy",
          "markdownDescription": "Denies the destroy command without any pre-configured scope."
        },
        {
          "description": "Denies the get_all_windows command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-get-all-windows",
          "markdownDescription": "Denies the get_all_windows command without any pre-configured scope."
        },
        {
          "description": "Denies the hide command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-hide",
          "markdownDescription": "Denies the hide command without any pre-configured scope."
        },
        {
          "description": "Denies the inner_position command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-inner-position",
          "markdownDescription": "Denies the inner_position command without any pre-configured scope."
        },
        {
          "description": "Denies the inner_size command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-inner-size",
          "markdownDescription": "Denies the inner_size command without any pre-configured scope."
        },
        {
          "description": "Denies the internal_toggle_maximize command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-internal-toggle-maximize",
          "markdownDescription": "Denies the internal_toggle_maximize command without any pre-configured scope."
        },
        {
          "description": "Denies the is_always_on_top command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-is-always-on-top",
          "markdownDescription": "Denies the is_always_on_top command without any pre-configured scope."
        },
        {
          "description": "Denies the is_closable command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-is-closable",
          "markdownDescription": "Denies the is_closable command without any pre-configured scope."
        },
        {
          "description": "Denies the is_decorated command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-is-decorated",
          "markdownDescription": "Denies the is_decorated command without any pre-configured scope."
        },
        {
          "description": "Denies the is_enabled command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-is-enabled",
          "markdownDescription": "Denies the is_enabled command without any pre-configured scope."
        },
        {
          "description": "Denies the is_focused command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-is-focused",
          "markdownDescription": "Denies the is_focused command without any pre-configured scope."
        },
        {
          "description": "Denies the is_fullscreen command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-is-fullscreen",
          "markdownDescription": "Denies the is_fullscreen command without any pre-configured scope."
        },
        {
          "description": "Denies the is_maximizable command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-is-maximizable",
          "markdownDescription": "Denies the is_maximizable command without any pre-configured scope."
        },
        {
          "description": "Denies the is_maximized command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-is-maximized",
          "markdownDescription": "Denies the is_maximized command without any pre-configured scope."
        },
        {
          "description": "Denies the is_minimizable command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-is-minimizable",
          "markdownDescription": "Denies the is_minimizable command without any pre-configured scope."
        },
        {
          "description": "Denies the is_minimized command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-is-minimized",
          "markdownDescription": "Denies the is_minimized command without any pre-configured scope."
        },
        {
          "description": "Denies the is_resizable command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-is-resizable",
          "markdownDescription": "Denies the is_resizable command without any pre-configured scope."
        },
        {
          "description": "Denies the is_visible command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-is-visible",
          "markdownDescription": "Denies the is_visible command without any pre-configured scope."
        },
        {
          "description": "Denies the maximize command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-maximize",
          "markdownDescription": "Denies the maximize command without any pre-configured scope."
        },
        {
          "description": "Denies the minimize command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-minimize",
          "markdownDescription": "Denies the minimize command without any pre-configured scope."
        },
        {
          "description": "Denies the monitor_from_point command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-monitor-from-point",
          "markdownDescription": "Denies the monitor_from_point command without any pre-configured scope."
        },
        {
          "description": "Denies the outer_position command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-outer-position",
          "markdownDescription": "Denies the outer_position command without any pre-configured scope."
        },
        {
          "description": "Denies the outer_size command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-outer-size",
          "markdownDescription": "Denies the outer_size command without any pre-configured scope."
        },
        {
          "description": "Denies the primary_monitor command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-primary-monitor",
          "markdownDescription": "Denies the primary_monitor command without any pre-configured scope."
        },
        {
          "description": "Denies the request_user_attention command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-request-user-attention",
          "markdownDescription": "Denies the request_user_attention command without any pre-configured scope."
        },
        {
          "description": "Denies the scale_factor command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-scale-factor",
          "markdownDescription": "Denies the scale_factor command without any pre-configured scope."
        },
        {
          "description": "Denies the scene_identifier command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-scene-identifier",
          "markdownDescription": "Denies the scene_identifier command without any pre-configured scope."
        },
        {
          "description": "Denies the set_always_on_bottom command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-set-always-on-bottom",
          "markdownDescription": "Denies the set_always_on_bottom command without any pre-configured scope."
        },
        {
          "description": "Denies the set_always_on_top command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-set-always-on-top",
          "markdownDescription": "Denies the set_always_on_top command without any pre-configured scope."
        },
        {
          "description": "Denies the set_background_color command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-set-background-color",
          "markdownDescription": "Denies the set_background_color command without any pre-configured scope."
        },
        {
          "description": "Denies the set_badge_count command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-set-badge-count",
          "markdownDescription": "Denies the set_badge_count command without any pre-configured scope."
        },
        {
          "description": "Denies the set_badge_label command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-set-badge-label",
          "markdownDescription": "Denies the set_badge_label command without any pre-configured scope."
        },
        {
          "description": "Denies the set_closable command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-set-closable",
          "markdownDescription": "Denies the set_closable command without any pre-configured scope."
        },
        {
          "description": "Denies the set_content_protected command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-set-content-protected",
          "markdownDescription": "Denies the set_content_protected command without any pre-configured scope."
        },
        {
          "description": "Denies the set_cursor_grab command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-set-cursor-grab",
          "markdownDescription": "Denies the set_cursor_grab command without any pre-configured scope."
        },
        {
          "description": "Denies the set_cursor_icon command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-set-cursor-icon",
          "markdownDescription": "Denies the set_cursor_icon command without any pre-configured scope."
        },
        {
          "description": "Denies the set_cursor_position command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-set-cursor-position",
          "markdownDescription": "Denies the set_cursor_position command without any pre-configured scope."
        },
        {
          "description": "Denies the set_cursor_visible command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-set-cursor-visible",
          "markdownDescription": "Denies the set_cursor_visible command without any pre-configured scope."
        },
        {
          "description": "Denies the set_decorations command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-set-decorations",
          "markdownDescription": "Denies the set_decorations command without any pre-configured scope."
        },
        {
          "description": "Denies the set_effects command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-set-effects",
          "markdownDescription": "Denies the set_effects command without any pre-configured scope."
        },
        {
          "description": "Denies the set_enabled command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-set-enabled",
          "markdownDescription": "Denies the set_enabled command without any pre-configured scope."
        },
        {
          "description": "Denies the set_focus command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-set-focus",
          "markdownDescription": "Denies the set_focus command without any pre-configured scope."
        },
        {
          "description": "Denies the set_focusable command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-set-focusable",
          "markdownDescription": "Denies the set_focusable command without any pre-configured scope."
        },
        {
          "description": "Denies the set_fullscreen command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-set-fullscreen",
          "markdownDescription": "Denies the set_fullscreen command without any pre-configured scope."
        },
        {
          "description": "Denies the set_fullscreen_on_monitor command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-set-fullscreen-on-monitor",
          "markdownDescription": "Denies the set_fullscreen_on_monitor command without any pre-configured scope."
        },
        {
          "description": "Denies the set_icon command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-set-icon",
          "markdownDescription": "Denies the set_icon command without any pre-configured scope."
        },
        {
          "description": "Denies the set_ignore_cursor_events command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-set-ignore-cursor-events",
          "markdownDescription": "Denies the set_ignore_cursor_events command without any pre-configured scope."
        },
        {
          "description": "Denies the set_max_size command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-set-max-size",
          "markdownDescription": "Denies the set_max_size command without any pre-configured scope."
        },
        {
          "description": "Denies the set_maximizable command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-set-maximizable",
          "markdownDescription": "Denies the set_maximizable command without any pre-configured scope."
        },
        {
          "description": "Denies the set_min_size command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-set-min-size",
          "markdownDescription": "Denies the set_min_size command without any pre-configured scope."
        },
        {
          "description": "Denies the set_minimizable command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-set-minimizable",
          "markdownDescription": "Denies the set_minimizable command without any pre-configured scope."
        },
        {
          "description": "Denies the set_overlay_icon command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-set-overlay-icon",
          "markdownDescription": "Denies the set_overlay_icon command without any pre-configured scope."
        },
        {
          "description": "Denies the set_position command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-set-position",
          "markdownDescription": "Denies the set_position command without any pre-configured scope."
        },
        {
          "description": "Denies the set_progress_bar command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-set-progress-bar",
          "markdownDescription": "Denies the set_progress_bar command without any pre-configured scope."
        },
        {
          "description": "Denies the set_resizable command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-set-resizable",
          "markdownDescription": "Denies the set_resizable command without any pre-configured scope."
        },
        {
          "description": "Denies the set_shadow command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-set-shadow",
          "markdownDescription": "Denies the set_shadow command without any pre-configured scope."
        },
        {
          "description": "Denies the set_simple_fullscreen command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-set-simple-fullscreen",
          "markdownDescription": "Denies the set_simple_fullscreen command without any pre-configured scope."
        },
        {
          "description": "Denies the set_size command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-set-size",
          "markdownDescription": "Denies the set_size command without any pre-configured scope."
        },
        {
          "description": "Denies the set_size_constraints command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-set-size-constraints",
          "markdownDescription": "Denies the set_size_constraints command without any pre-configured scope."
        },
        {
          "description": "Denies the set_skip_taskbar command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-set-skip-taskbar",
          "markdownDescription": "Denies the set_skip_taskbar command without any pre-configured scope."
        },
        {
          "description": "Denies the set_theme command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-set-theme",
          "markdownDescription": "Denies the set_theme command without any pre-configured scope."
        },
        {
          "description": "Denies the set_title command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-set-title",
          "markdownDescription": "Denies the set_title command without any pre-configured scope."
        },
        {
          "description": "Denies the set_title_bar_style command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-set-title-bar-style",
          "markdownDescription": "Denies the set_title_bar_style command without any pre-configured scope."
        },
        {
          "description": "Denies the set_visible_on_all_workspaces command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-set-visible-on-all-workspaces",
          "markdownDescription": "Denies the set_visible_on_all_workspaces command without any pre-configured scope."
        },
        {
          "description": "Denies the show command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-show",
          "markdownDescription": "Denies the show command without any pre-configured scope."
        },
        {
          "description": "Denies the start_dragging command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-start-dragging",
          "markdownDescription": "Denies the start_dragging command without any pre-configured scope."
        },
        {
          "description": "Denies the start_resize_dragging command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-start-resize-dragging",
          "markdownDescription": "Denies the start_resize_dragging command without any pre-configured scope."
        },
        {
          "description": "Denies the theme command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-theme",
          "markdownDescription": "Denies the theme command without any pre-configured scope."
        },
        {
          "description": "Denies the title command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-title",
          "markdownDescription": "Denies the title command without any pre-configured scope."
        },
        {
          "description": "Denies the toggle_maximize command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-toggle-maximize",
          "markdownDescription": "Denies the toggle_maximize command without any pre-configured scope."
        },
        {
          "description": "Denies the unmaximize command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-unmaximize",
          "markdownDescription": "Denies the unmaximize command without any pre-configured scope."
        },
        {
          "description": "Denies the unminimize command without any pre-configured scope.",
          "type": "string",
          "const": "core:window:deny-unminimize",
          "markdownDescription": "Denies the unminimize command without any pre-configured scope."
        }
      ]
    },
    "Value": {
      "description": "All supported ACL values.",
      "anyOf": [
        {
          "description": "Represents a null JSON value.",
          "type": "null"
        },
        {
          "description": "Represents a [`bool`].",
          "type": "boolean"
        },
        {
          "description": "Represents a valid ACL [`Number`].",
          "allOf": [
            {
              "$ref": "#/definitions/Number"
            }
          ]
        },
        {
          "description": "Represents a [`String`].",
          "type": "string"
        },
        {
          "description": "Represents a list of other [`Value`]s.",
          "type": "array",
          "items": {
            "$ref": "#/definitions/Value"
          }
        },
        {
          "description": "Represents a map of [`String`] keys to [`Value`]s.",
          "type": "object",
          "additionalProperties": {
            "$ref": "#/definitions/Value"
          }
        }
      ]
    },
    "Number": {
      "description": "A valid ACL number.",
      "anyOf": [
        {
          "description": "Represents an [`i64`].",
          "type": "integer",
          "format": "int64"
        },
        {
          "description": "Represents a [`f64`].",
          "type": "number",
          "format": "double"
        }
      ]
    },
    "Target": {
      "description": "Platform target.",
      "oneOf": [
        {
          "description": "MacOS.",
          "type": "string",
          "enum": [
            "macOS"
          ]
        },
        {
          "description": "Windows.",
          "type": "string",
          "enum": [
            "windows"
          ]
        },
        {
          "description": "Linux.",
          "type": "string",
          "enum": [
            "linux"
          ]
        },
        {
          "description": "Android.",
          "type": "string",
          "enum": [
            "android"
          ]
        },
        {
          "description": "iOS.",
          "type": "string",
          "enum": [
            "iOS"
          ]
        }
      ]
    }
  }
}
````

## File: crates/ghost-updater-tauri/src-tauri/src/main.rs
````rust
//! ghost-updater 推送台(Tauri 2 桌面应用)。
//! 业务逻辑全部在 ghost-updater-cli 库(probe / push_update / collect_dist_files),
//! 这里只做:参数编排、进度事件转发给前端、主机列表持久化。

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use ghost_updater_cli::{collect_dist_files, normalize_addr, pack_dist_in_memory, probe, push_update, PushCtx, PushUpdateCfg};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tauri::{AppHandle, Emitter, Manager};

/// 主机列表条目(随项目配置持久化)
#[derive(Serialize, Deserialize, Clone)]
struct HostEntry {
    addr: String,
}

fn default_timeout() -> u32 {
    120
}

/// 项目配置:一个集成了更新接口的目标应用 = 更新包配置 + 默认端口 + 机器列表,整档保存
#[derive(Serialize, Deserialize, Clone)]
struct AppProfile {
    name: String,
    port: u16,
    hosts: Vec<HostEntry>,
    /// 本机新版 dist 目录
    #[serde(default)]
    dist_dir: String,
    /// 主程序文件名(可选,空 = dist 内唯一 *.exe)
    #[serde(default)]
    main_exe: String,
    /// true = 连 config.json 也覆盖
    #[serde(default)]
    overwrite_config: bool,
    #[serde(default)]
    token: String,
    /// ed25519 私钥 hex(可选)
    #[serde(default)]
    sign_key: String,
    /// 轮询确认超时(秒)
    #[serde(default = "default_timeout")]
    timeout_secs: u32,
}

impl Default for AppProfile {
    fn default() -> Self {
        Self {
            name: String::new(),
            port: 0,
            hosts: Vec::new(),
            dist_dir: String::new(),
            main_exe: String::new(),
            overwrite_config: false,
            token: String::new(),
            sign_key: String::new(),
            timeout_secs: default_timeout(),
        }
    }
}

#[derive(Serialize, Clone, Default)]
struct ProbeResult {
    addr: String,
    reachable: bool,
    version: String,
    started_at: u64,
    os: String,
    arch: String,
    pubkey_fingerprint: String,
    error: String,
}

#[derive(Serialize)]
struct FileEntry {
    name: String,
    size: u64,
}

#[derive(Serialize)]
struct DistInfo {
    ok: bool,
    version: String,
    main: String,
    files: Vec<FileEntry>,
    total_bytes: u64,
    package_sha256: String,
    error: String,
}

#[derive(Serialize, Clone)]
struct UpdateReportDto {
    ok: bool,
    addr: String,
    old_version: String,
    new_version: String,
    detail: String,
}

#[derive(Serialize, Clone)]
struct LogLine {
    addr: String,
    line: String,
}

#[tauri::command]
async fn cmd_probe(addr: String, token: Option<String>) -> ProbeResult {
    let addr = normalize_addr(&addr);
    let mut r = ProbeResult { addr: addr.clone(), ..Default::default() };
    let ctx = match PushCtx::new(None) {
        Ok(c) => c,
        Err(e) => {
            r.error = e;
            return r;
        }
    };
    match probe(&ctx, &addr, token.as_deref()).await {
        Ok(p) => {
            r.reachable = true;
            r.version = p.version;
            r.started_at = p.started_at;
            r.os = p.os;
            r.arch = p.arch;
            r.pubkey_fingerprint = p.pubkey_fingerprint;
        }
        Err(e) => r.error = e,
    }
    r
}

/// dist 预览:版本号、主程序、文件集、整包哈希(与推送时一致)
#[tauri::command]
fn cmd_dist_info(dist_dir: String, main: Option<String>, overwrite_config: bool) -> DistInfo {
    let dir = PathBuf::from(&dist_dir);
    let mut info = DistInfo {
        ok: false,
        version: String::new(),
        main: String::new(),
        files: vec![],
        total_bytes: 0,
        package_sha256: String::new(),
        error: String::new(),
    };
    let (files, main_name) = match collect_dist_files(&dir, main.as_deref(), overwrite_config) {
        Ok(f) => f,
        Err(e) => {
            info.error = e;
            return info;
        }
    };
    info.version = std::fs::read_to_string(dir.join("VERSION"))
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "(无 VERSION)".into());
    info.main = main_name;
    for f in &files {
        let size = std::fs::metadata(dir.join(f)).map(|m| m.len()).unwrap_or(0);
        info.total_bytes += size;
        info.files.push(FileEntry { name: f.clone(), size });
    }
    match pack_dist_in_memory(&dir, &files) {
        Ok((_, sha)) => info.package_sha256 = sha,
        Err(e) => {
            info.error = e;
            return info;
        }
    }
    info.ok = true;
    info
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
async fn cmd_update_host(
    app: AppHandle,
    addr: String,
    dist_dir: String,
    main: Option<String>,
    token: Option<String>,
    sign_key: Option<String>,
    overwrite_config: bool,
    timeout_secs: u64,
) -> UpdateReportDto {
    let addr = normalize_addr(&addr);
    // 进度行 → 前端 "update-log" 事件(按主机 addr 区分)
    let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(256);
    let forwarder = {
        let addr = addr.clone();
        let app = app.clone();
        tokio::spawn(async move {
            while let Some(line) = rx.recv().await {
                let _ = app.emit("update-log", LogLine { addr: addr.clone(), line });
            }
        })
    };
    let ctx = match PushCtx::new(sign_key.as_deref()) {
        Ok(c) => c,
        Err(e) => {
            return UpdateReportDto { ok: false, addr, old_version: String::new(), new_version: String::new(), detail: e };
        }
    };
    let report = push_update(
        &ctx,
        PushUpdateCfg {
            addr: addr.clone(),
            dist_dir: PathBuf::from(&dist_dir),
            main,
            token,
            sign_key_hex: None, // 签名在 ctx 里
            overwrite_config,
            poll_timeout: std::time::Duration::from_secs(timeout_secs.max(10)),
            on_event: Some(tx),
        },
    )
    .await;
    let _ = forwarder.await;
    UpdateReportDto {
        ok: report.ok,
        addr: report.addr,
        old_version: report.old_version,
        new_version: report.new_version,
        detail: report.detail,
    }
}

fn config_file(app: &AppHandle, name: &str) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|e| format!("配置目录不可用: {e}"))?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建配置目录失败: {e}"))?;
    Ok(dir.join(name))
}

/// 旧的全局"更新包"设置(仅用于一次性迁移进项目;迁移后删除该文件)
#[derive(Deserialize, Default)]
struct LegacySettings {
    #[serde(default)]
    dist_dir: String,
    #[serde(default)]
    main_exe: String,
    #[serde(default)]
    overwrite_config: bool,
    #[serde(default)]
    token: String,
    #[serde(default)]
    sign_key: String,
    #[serde(default)]
    timeout_secs: u32,
}

impl LegacySettings {
    /// 全空则视为没有可迁移的内容
    fn is_empty(&self) -> bool {
        self.dist_dir.is_empty()
            && self.main_exe.is_empty()
            && !self.overwrite_config
            && self.token.is_empty()
            && self.sign_key.is_empty()
    }

    fn apply_to(&self, a: &mut AppProfile) {
        a.dist_dir = self.dist_dir.clone();
        a.main_exe = self.main_exe.clone();
        a.overwrite_config = self.overwrite_config;
        a.token = self.token.clone();
        a.sign_key = self.sign_key.clone();
        if self.timeout_secs > 0 {
            a.timeout_secs = self.timeout_secs;
        }
    }
}

/// 读取 apps.json;不存在但旧 hosts.json 存在时,迁移为单档"默认项目"
fn read_apps(app: &AppHandle) -> Vec<AppProfile> {
    let Ok(path) = config_file(app, "apps.json") else { return vec![] };
    if let Ok(s) = std::fs::read_to_string(&path) {
        if let Ok(apps) = serde_json::from_str::<Vec<AppProfile>>(&s) {
            return apps;
        }
    }
    // 兼容旧版单列表 hosts.json
    let Ok(old) = config_file(app, "hosts.json") else { return vec![] };
    let Ok(s) = std::fs::read_to_string(old) else { return vec![] };
    let Ok(hosts) = serde_json::from_str::<Vec<HostEntry>>(&s) else { return vec![] };
    if hosts.is_empty() {
        return vec![];
    }
    vec![AppProfile { name: "默认项目".into(), port: 7600, hosts, ..Default::default() }]
}

/// 读取项目列表;首启把旧 settings.json 的更新包设置并入项目,随后删除旧文件
#[tauri::command]
fn load_apps(app: AppHandle) -> Vec<AppProfile> {
    let mut apps = read_apps(&app);
    let Ok(path) = config_file(&app, "settings.json") else { return apps };
    let Ok(s) = std::fs::read_to_string(&path) else { return apps };
    let Ok(legacy) = serde_json::from_str::<LegacySettings>(&s) else { return apps };
    if !legacy.is_empty() {
        if apps.is_empty() {
            let mut p = AppProfile { name: "默认项目".into(), port: 7600, ..Default::default() };
            legacy.apply_to(&mut p);
            apps.push(p);
        } else {
            for a in &mut apps {
                if a.dist_dir.is_empty() {
                    legacy.apply_to(a);
                }
            }
        }
        // 马上落盘,避免"还没保存"期间旧文件被删而丢设置
        if let Err(e) = write_apps(&app, &apps) {
            eprintln!("迁移项目配置落盘失败: {e}");
            return apps;
        }
    }
    // 已逐项并入项目,旧文件不再需要
    let _ = std::fs::remove_file(&path);
    apps
}

fn write_apps(app: &AppHandle, apps: &[AppProfile]) -> Result<(), String> {
    let path = config_file(app, "apps.json")?;
    let body = serde_json::to_string_pretty(apps).map_err(|e| e.to_string())?;
    std::fs::write(path, body).map_err(|e| format!("保存项目配置失败: {e}"))
}

#[tauri::command]
fn save_apps(app: AppHandle, apps: Vec<AppProfile>) -> Result<(), String> {
    write_apps(&app, &apps)
}

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            cmd_probe,
            cmd_dist_info,
            cmd_update_host,
            load_apps,
            save_apps
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
````

## File: crates/ghost-updater-tauri/src-tauri/build.rs
````rust
fn main() {
    tauri_build::build();
}
````

## File: crates/ghost-updater-tauri/src-tauri/Cargo.toml
````toml
[package]
name = "ghost-updater-tauri"
description = "ghost-updater 推送端 GUI(Tauri 2):主机列表、批量推更、实时日志"
version = "0.1.0"
edition = "2024"
license = "MIT"

# 独立于主 workspace:generate_context! 编译期读 ../webdist,需先 npm run build
[workspace]

[build-dependencies]
tauri-build = { version = "2", features = [] }

[dependencies]
ghost-updater-cli = { path = "../../ghost-updater-cli" }
tauri = { version = "2", features = [] }
tokio = { version = "1", features = ["full"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"

[profile.release]
strip = true
lto = true
````

## File: crates/ghost-updater-tauri/src-tauri/tauri.conf.json
````json
{
  "$schema": "https://schema.tauri.app/config/2",
  "productName": "ghost-updater",
  "version": "0.1.0",
  "identifier": "com.ghost.updater",
  "build": {
    "frontendDist": "../webdist",
    "devUrl": "http://localhost:5173",
    "beforeDevCommand": "npm run dev",
    "beforeBuildCommand": "npm run build"
  },
  "app": {
    "windows": [
      {
        "title": "ghost-updater 推送台",
        "width": 1120,
        "height": 800,
        "minWidth": 900,
        "minHeight": 600
      }
    ],
    "security": { "csp": null }
  },
  "bundle": {
    "active": true,
    "targets": ["nsis"],
    "icon": ["icons/icon.ico"]
  }
}
````

## File: crates/ghost-updater-tauri/index.html
````html
<!doctype html>
<html lang="zh-CN">
  <head>
    <meta charset="UTF-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1.0" />
    <title>ghost-updater 推送台</title>
  </head>
  <body>
    <div id="app"></div>
    <script type="module" src="/src/main.ts"></script>
  </body>
</html>
````

## File: crates/ghost-updater-tauri/package.json
````json
{
  "name": "ghost-updater-gui",
  "private": true,
  "version": "0.1.0",
  "type": "module",
  "scripts": {
    "dev": "vite",
    "build": "vite build",
    "preview": "vite preview",
    "tauri": "tauri"
  },
  "dependencies": {
    "@tauri-apps/api": "^2"
  },
  "devDependencies": {
    "@sveltejs/vite-plugin-svelte": "^5.0.0",
    "@tauri-apps/cli": "^2",
    "svelte": "^5.0.0",
    "typescript": "^5.6.0",
    "vite": "^6.0.0"
  }
}
````

## File: crates/ghost-updater-tauri/svelte.config.js
````javascript
import { vitePreprocess } from "@sveltejs/vite-plugin-svelte";

export default { preprocess: vitePreprocess() };
````

## File: crates/ghost-updater-tauri/tsconfig.json
````json
{
  "compilerOptions": {
    "target": "ES2021",
    "module": "ESNext",
    "moduleResolution": "bundler",
    "strict": true,
    "noEmit": true,
    "isolatedModules": true,
    "skipLibCheck": true,
    "types": ["svelte", "vite/client"]
  },
  "include": ["src/**/*.ts", "src/**/*.svelte"]
}
````

## File: crates/ghost-updater-tauri/vite.config.ts
````typescript
import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";

// 输出目录用 webdist(src-tauri 的 frontendDist 指向这里)
export default defineConfig({
  plugins: [svelte()],
  clearScreen: false,
  server: { port: 5173, strictPort: true },
  build: { outDir: "webdist", emptyOutDir: true, target: "chrome105" },
});
````

## File: plan/design.md
````markdown
1. 关于端口占用的确认：绝对不会做主动推送更新完全不需要占用很多端口，最少只需 0 到 1 个端口：0 额外端口（嵌入式集成，最推荐）： 如果你的软件本身已有 Web/RPC 接口（如 Axum 或 Tonic 服务），更新接口只是几个普通的路由（如 /v1/update/stage、/v1/update/apply），直接挂载在主服务下，新增端口为 0。0 监听端口（长连接反向推送）： 若你的客户端是内网机器且不对外暴露端口，客户端启动时只需向中控服务器建立一条出站长连接（WebSocket 或 gRPC 双向流），由云端通过该长连接下发更新指令，本地完全不需要开启任何监听端口。1 个端口（独立更新监听器）： 只有当程序本身是纯桌面/本地工具，且必须接受局域网直连管理时，才需要额外开启 1 个轻量级控制面端口（例如 1 个 TCP 端口同时承载控制指令与分块传输）。   2. 通用接口形态：如何做到“一键集成”目前开源生态中常见的更新库（如 self_update、tauri-plugin-updater）几乎都是客户端被动拉取（Pull）模型，业界并没有一个现成的、面向“主动远程换版（Push）”的通用库。但你可以封装一个通用的内部私有 crate（例如命名为 ghost-updater），只需对外暴露两个极简的接入点：(1) main.rs 顶部拦截（一键接管自替换）Rustfn main() {
    // 拦截内部触发的更新替换模式：如果是更新进程，它会等待旧进程退出、替换文件并重启，然后直接 exit(0)
    ghost_updater::guard_apply_update();

    // 正常业务逻辑
    tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap().block_on(async {
        // ...
    });
}
(2) 挂载到业务服务中（一键暴露更新路由）Rust// 如果使用 Axum：
let app = Router::new()
    .route("/hello", get(|| async { "world" }))
    .merge(ghost_updater::routes()); // 自动注入 /update/stage、/update/apply、/update/status 等路由

// 或者作为独立后台任务启动（纯桌面/工具软件）：
ghost_updater::start_agent("0.0.0.0:9001", &auth_token);
3. “通用一键更新器”开发计划本计划提炼自生产级推更方案与踩坑记录，将其标准化为通用模块。   ┌────────────────────────────────────────────────────────┐
│             ghost-updater (通用 Rust Workspace)        │
├────────────────────┬───────────────────┬───────────────┤
│ ghost-updater-core │ ghost-updater-net │ ghost-updater-cli/tauri │
│ (自替换/锁/跨平台) │ (Axum/gRPC 路由)  │ (推送端 GUI/CLI)       │
└────────────────────┴───────────────────┴───────────────┘
阶段一：核心运行时设计 (ghost-updater-core)目标： 实现跨平台的“幽灵自替换”与进程接力，零网络依赖。进程启动时间戳记录：在模块初始化时记录一个全局原子 STARTED_AT（Unix 秒）。   解决更新后确认重启的竞态问题，避免用“网络断开再连通”判定重启造成的误判。   guard_apply_update() 执行流程：检查启动参数是否包含 --internal-apply-update。   等旧进程退出： 轮询目标 PID（Windows 使用 OpenProcess + STILL_ACTIVE，10~15 秒超时，超时 taskkill /F 兜底）。   备份机制： 将将被覆盖的旧二进制移动到 .update_backup/。   文件覆盖： 将暂存区文件复制/移动到目标安装目录（必须保留目标机器的自定义配置文件）。   拉起新程序： 使用 DETACHED_PROCESS 或 CREATE_NO_WINDOW 拉起安装目录里的最终可执行文件，注意严禁使用等待 EOF 的管道捕获（避免子进程句柄继承导致调用死锁）。   自愈回滚： 若启动失败，还原备份并记录 apply.log。   阶段二：网络协议层 (ghost-updater-net)目标： 抽离出可嵌入现有框架（Axum / Tonic / 自建 TCP）的通用 Handler。标准化三段式 API：GET /update/probe：返回当前程序的 version、started_at、操作系统架构、公钥指纹。   POST /update/stage：将推送的更新包流式写入暂存目录（带 SHA256 / BLAKE3 校验，避免网络中断导致半损坏）。   POST /update/apply：触发更新。主服务向调用方返回 200 OK，并延迟 500ms~1000ms 后以分离子进程拉起暂存文件执行 --internal-apply-update，自身随后 std::process::exit(0)。   通信安全边界（重要）：由于允许远程替换可执行文件，必须内置强制性的安全校验：要么支持预共享 Token 鉴权（Bearer <TOKEN>）。要么二进制包自带签名，更新器在 apply 前使用内置公钥验签。阶段三：推送端 SDK 与 Tauri GUI (ghost-updater-tauri)目标： 打造类似你此前验证过的管理控制台，支持批量机器推更。   推送编排器（Orchestrator Engine）：对目标列表进行连接探活（Ping / Probe）。   并发向多台机器推送暂存文件（显示流式传输进度）。   并发下发 apply 指令，并进入轮询等待阶段。   Tauri 2 + Svelte 前端界面：主机列表视图： IP:Port、当前版本、运行状态、上次更新时间。   更新包配置： 选择本地打包好的可执行文件或压缩包（自动计算哈希并展示版本号）。   一键批量更新面板： 单独更新、批量全部更新、重试失败节点、操作日志实时滚动。   阶段四：极限场景测试与边界加固Windows 文件锁边界： 测试带有附加 DLL（如 sqlite3.dll 或自定义动态库）时的完整目录覆盖与还原。网络中途断开： 测试在推送阶段人为掐断连接，验证暂存文件损坏不会影响正在运行的业务主进程。同版本更新与回滚： 验证版本未变化但重新部署时，started_at 能够正确标定成功状态。   



----



一个完整的实现（已经实际投入生产使用）在：C:\develop\File-Transfer\updater中，需要的时候可以作为参考。

## 实现假设（依据见上；地图未覆盖细节按此执行）

1. 校验算法选 SHA256（design 写 "SHA256 / BLAKE3" 二选一，sha2 依赖更普适，推送端 GUI/CLI 都易算）。
2. 更新包格式 = 未压缩 tar（扁平文件集，拒绝含路径分隔符/.. 的条目），流式写暂存区边写边算哈希；stage 时把文件清单写进暂存区 manifest.json，apply 侧读 manifest，不重复传清单。
3. 签名方案：ed25519 对 "tar 包的 SHA256 摘要(32 字节)" 签名，X-Signature 头传 hex；非 minisign，内部系统自洽即可。Token 与签名按 design 二选一强制：配了 token 走 token；没配 token 但配了公钥走验签；两者都没配时 stage/apply 返回 503（拒绝服务而非裸奔）。probe 永远公开。
4. guard_apply_update 零配置（设计示例无参）：重启 = 无参拉起安装目录主 exe，cwd=安装目录；健康检查 = 分离拉起后 1.5s 仍存活，否则回滚。apply 请求体可选 relaunch_args 数组，经 `--` 透传给重启进程。
5. 备份/日志统一放 `<安装目录>/.update_backup/`（design 点名该目录）：apply.log、result.json、backup-<ts>/；成功后删当日备份，保留日志。
6. apply 锁：暂存区写 `.apply-lock`（create_new 原子）防并发 apply，worker 结束时清理。
7. 主 exe 名由推送端指定（--main，缺省 = dist 内唯一 *.exe），随 stage 请求传给服务端存入 manifest；apply 校验其在文件集内。
8. 推送端成功判定复刻 fts 生产规则：dist 有 VERSION 且与旧版本不同 → 必须看到新版本；同版本重部署/无 VERSION → started_at 变化即成功。
9. GUI 排除出主 workspace（需 npm 先产 webdist，generate_context! 编译期读 frontendDist）；镜像参考项目结构（src-tauri + svelte + vite），图标复用参考项目 icon.ico。
10. e2e 用同一 demo-app exe（VERSION/NOTES 文件差异验证换版）；exe 字节级替换语义由 core 单测覆盖；宿主版本号启动时读 exe 旁 VERSION 文件（option_env 双份构建方案，弃）。

## 错误记录表

| 时间 | 位置 | 报错/问题 | 处置 |
|---|---|---|---|
| e2e | core process_alive | windows-sys 0.59 无 Threading::STILL_ACTIVE(在 Foundation,类型 i32) | 移到 Foundation 导入 + `as u32` |
| 单测 | net tests | 并发测试 set_current_dir 互相踩,os error 183 | UpdateConfig 加 staging_root,每测试独立目录,不再动 cwd |
| 单测 | tar::Builder | Builder 拒绝 `..` 条目名,恶意包测试造不出来 | 测试里手工拼 512 字节 raw tar 头 |
| e2e | tools/e2e.ps1 | PowerShell 5.1 把无 BOM UTF-8 中文读成乱码,字符串解析炸 | 重写为带 BOM;`$ErrorActionPreference=Stop` 下 cargo 的 stderr 会变 terminating error → `cmd /c ... >nul 2>&1` |
| e2e | 端口 | 默认端口 9600 落在 Hyper-V 保留段 9595-9694,bind 10013 | 全线默认端口改 7600,CLI/文档注明 |
| e2e | cli bin | 包名 ghost-updater-cli ≠ 二进制名,ghost-push.exe 不存在 | 显式 [[bin]] name = "ghost-push" |
| e2e | 清理顺序 | 上轮残留 ghost-demo 锁住 exe,目录删不掉 | 先 Stop-Process 再 Remove-Item |
| **核心** | net apply | staging 传了相对路径,worker 的 cwd 又是暂存目录 → `--staging` 在 worker 里自我套娃,manifest 读不到(os error 3),worker 无声死掉 | stage/resolve_staging 一律 std::path::absolute;worker stdio 落 staging/worker.log 便于排障 |
| 设计补 | net apply | 重启后宿主丢了原始启动参数(默认端口起服务),推送端轮询旧地址永远失败 | apply 缺省透传本进程原始 argv 作 relaunch_args |
| e2e | 断言时序 | pusher 见到 restarted=true 时 worker 还没做健康检查后的备份清理 | e2e 轮询等备份目录消失 |
| gui | src-tauri Cargo.toml | path 依赖 `../ghost-updater-cli` 相对 src-tauri 解析,找不到 | 改 `../../ghost-updater-cli` |

---

## 追加变更:GUI 构建/运行/打包统一入口脚本

### 目标

`cargo run` 在 debug 下走 `tauri.conf.json` 的 `devUrl`(http://localhost:5173),而 `beforeDevCommand`
只有 `tauri dev` 才会执行;直接 `cargo run` 必然 WebView2 报 ERR_CONNECTION_REFUSED。
用脚本收敛成一条命令,并把"改前端 / 跑窗口 / 出安装包"三条路径分开。

### 方案取舍

- 脚本落 `tools/gui.ps1`(与既有 `tools/e2e.ps1` 同目录同风格),子命令:
  - `dev`   → `npm run tauri -- dev`(CLI 自动起 Vite,热更新,走 devUrl 是正确的)
  - `run`   → `npm run tauri -- build --debug --no-bundle`,再拉起 debug exe。
            这是"Rust 用 debug、前端用已构建 webdist"的官方 CLI 路径(不走 devUrl)。
            备选是仓库文档里的 `cargo run --features tauri/custom-protocol`(已核
            tauri-build-2.7.1/src/lib.rs:591 `is_dev()`:`dev` cfg = tauri 是否未启用
            `custom-protocol`),功能等价;脚本选 CLI 路径以免与"手动跑"两套说法打架。
            已实测:窗口正常渲染出界面,无 ERR_CONNECTION_REFUSED。
  - `build` → `npm run build`(仅 vite → webdist)
  - `check` → `npm run build` + GUI crate `cargo check`(generate_context! 编译期读 webdist,必须先有)
  - `installer` → `npm run tauri -- build` → NSIS
  - `verify` → 主 workspace `cargo build/test` + GUI `check`(复刻 TODO 完成标准)
- `run` / `installer` 前先 `Stop-Running`:上次运行的 GUI 会锁住 exe,不关掉 cargo 覆盖必报 os error 5。
- 不改 `tauri.conf.json`:devUrl 是 Tauri 的标准开发配置,问题在调用方式不在配置。
- `tools/gui.cmd` 薄包装:cmd 下免写 `-ExecutionPolicy Bypass -File`;必须纯 ASCII
  (cmd 按 GBK 读 .cmd 文件,中文注释会被嚼成命令)。

### 假设

- `npx tauri` / `npm run tauri` 解析到本地 `node_modules/.bin/tauri`(已确认存在);缺 node_modules 时
  脚本直接 FAIL 并提示 `npm install`,不自动 install(联网副作用不进脚本)。
- `npm run tauri -- build --debug --no-bundle` 会先跑 `beforeBuildCommand`(npm run build),
  所以 `run` 无需自己再构建前端。依据:`tauri build --help` 明示会执行 beforeBuildCommand。
- native 命令的 stderr 在 `$ErrorActionPreference=Stop` 下可能变 terminating error(见既有错误表),
  故 npm/cargo 一律经 `cmd /c "... 2>&1"` 调用,只看 `$LASTEXITCODE`。

### 错误记录表(追加)

| 时间 | 位置 | 报错/问题 | 处置 |
|---|---|---|---|
| 运行 | WebView2 | `cargo run` 后窗口 ERR_CONNECTION_REFUSED(localhost:5173) | debug 走 devUrl 且无人起 Vite;改用脚本 `dev` / `run` |
| 打包 | cargo 链接 | 上一次运行的推送台进程锁住 debug exe → `failed to remove file ... 拒绝访问 (os error 5)` | `run` / `installer` 前 `Stop-Running`(Stop-Process 同名进程) |
| 脚本 | tools/gui.cmd | 中文注释被 cmd 按 GBK 解读,注释里的 `\|` 触发分词,报 `'鐢ㄦ硶:' 不是内部或外部命令` | 包装脚本改纯 ASCII(中文只留在带 BOM 的 .ps1 里) |

---

## 追加变更:项目整档保存(更新包设置并入项目)

### 目标

推送台此前只把「应用名 / 默认端口 / 机器列表」存进 apps.json,而「更新包」面板的
`dist 目录 / 主程序 / 覆盖 config / Token / 签名私钥 / 轮询超时` 是全局单份(settings.json),
切换应用不跟着换。用户有多个应用各配一套更新包,需要「更新包配置 + 目标主机」整档保存、随项目切换。

### 方案取舍

- 把 6 个更新包字段并入 `AppProfile`(即"项目"):一个项目 = 更新包配置 + 默认端口 + 主机列表,
  apps.json 成为唯一持久化,不再需要 settings.json。
- 前端那 6 个输入框由全局变量改为"当前项目的工作副本":切换项目时从 `apps[idx]` 拉取,
  改动防抖写回该项目。
- 兼容:`AppProfile` 新字段全部 `#[serde(default)]`(timeout 默认 120);`load_apps` 首启一次性把旧
  settings.json 的更新包设置补进"还没有 dist 的项目"(无项目则新建"默认项目"),随后删除旧文件。
- UI 文案「应用」→「项目」;领域上仍是目标机上的一个集成应用,一个应用即一个项目。

### 影响面

- `crates/ghost-updater-tauri/src-tauri/src/main.rs`:AppProfile 扩字段;load_apps 迁移;
  删 `load_settings`/`save_settings` 命令及 handler 注册;旧 hosts.json 迁移名改「默认项目」。
- `crates/ghost-updater-tauri/src/App.svelte`:接口扩字段;更新包面板绑定 `active.*`;pull/push 项目;
  去掉全局 settings 持久化。
- `README.md` / `HowToUseAutoUpdate.md`:一句带过。

### 假设

- 单级结构:一个项目 = 一个目标应用(一套 exe/端口),不做"项目下再分多个应用"的两级。
  依据:现有「应用」已是 名称+端口+主机 的单位,用户原话"项目+接受推送的主机"亦为单级组合。
- 迁移后删除 settings.json 安全:6 项已逐项复制进项目,不丢数据。
- Token / 签名私钥随项目落 apps.json,与原先落 settings.json 的明文同级别,安全边界不变。

### 错误记录表(追加)

| 时间 | 位置 | 报错/问题 | 处置 |
|---|---|---|---|
| 项目保存 | App.svelte | `$: active = apps[activeIdx]` 是派生值,`activeIdx` 同步改后到 reactivity flush 前仍是旧项目;`switchApp` 里紧接着 `refreshDist()` 会预览到上一个项目的 dist | 函数内统一改用 `currentProject() = apps[activeIdx]`,只在模板里用 `active` |
| 项目保存 | main.rs | `#[derive(Default)]` 的 `timeout_secs` 是 `u32::default()=0`,不是 serde 的 120;Rust 侧新建的"默认项目"轮询超时显示 0 | 手写 `impl Default for AppProfile`,`timeout_secs: default_timeout()` |

---

## 追加变更:Model Copy 三需求(REQ-2 swap 失败回滚 / REQ-3 拒绝路径 drain / REQ-1 多进程编排)

### 目标

落地 plan/feature-requests-from-model-copy.md 三项:swap 半途失败自动回滚并释放 `.apply-lock`;
stage/apply 拒绝时 drain 请求体让 401 稳定可见;`UpdateConfig.close_processes` 多进程换版编排。
wire 协议零变化,空配置行为不变。

### 方案取舍

- **REQ-2 回滚不做全量 `restore_backup`,改为"只还原已覆盖的文件"**。原案(备份里没有的
  当新增删掉)有个数据丢失陷阱:若失败点之后的旧文件没被覆盖、也没进备份(备份段部分失败),
  restore 会把它们当"本次新增"删除;且 swap 失败时旧文件常仍被外部句柄锁着,restore 的
  remove+copy 会被 os error 32 卡死。实现:`swap_files` 拆成 `ensure_staged`/`backup_files`/
  `overwrite_files` 三段(组合壳保持原 API,4 个存量单测不动),`overwrite_files` 失败时仅对
  已成功覆盖的文件按备份逐个还原、对"备份里没有且刚创建"的删除——只动确实覆盖过的文件,
  Err 时 install 必已回旧状态,error 文案区分"(已回滚到旧状态)"与"(回滚失败,需人工介入:…)"。
  备份段失败 = 文件未动,不触发任何还原。`rollback()`/`restore_backup` 保留给健康检查/拉起
  失败路径(那时文件锁已释放,全量还原安全)。
- **锁收口**:`run_apply` 拆 parse → inner → `release_lock` 无条件兜底。核验发现泄漏面共 4 条
  失败路径(pid 超时/swap/健康检查/拉起),需求只点名 2 条,收口一次全覆盖。
- **REQ-3 drain 只落 stage**。核验:apply 的 `body: String` 是 axum 提取器,handler 执行前
  body 已读完,天然无此问题;需求里"stage/apply 拒绝"的 apply 半句不成立。stage 的 check_auth
  拒绝(401/503)与 sha256/main 两个 400 分支都在 body 消费前拒绝,一并 drain(上限
  MAX_PACKAGE_BYTES,整体 30s 超时防慢速攻击)。
- **REQ-1 两级编排**:agent 侧(apply 受理、鉴权后)`spawn_blocking(close_processes_graceful)`
  后再 exit(0)——自退从固定 800ms 变为"800ms + 关闭耗时",worker 的 wait_pid(15s)天然把
  关闭窗口吃进去;名单经 worker argv `--close a.exe,b.exe` 交接,worker 在等完 `--pid` 后再
  `wait_processes_exit`(轮询 tasklist,超时 taskkill /F 兜底,镜像 wait_pid_exit 语义)。
  非 Windows 全部 no-op(REQ 明示可留空)。进程名白名单 `validate_process_names`
  (仅 `[A-Za-z0-9._-]`)双端校验:routes() 启动 panic(与 pubkey 同风格)+ worker parse,
  防 taskkill/tasklist 参数注入。
- **进程操作选 tasklist/taskkill 子进程**(CSV 首列精确比对),不引 Toolhelp32 快照:
  与既有 wait_pid_exit 的 taskkill 风格一致,零新增 unsafe;GBK 输出里文件名列是 ASCII,
  utf8_lossy 无损。
- **close 动作落 apply.log**:core 开 `append_apply_log(install, msg)` pub 口,agent 关闭
  开始/成功/失败三态都记,排障与 worker 日志同文件。

### 影响面

- core/src/lib.rs:锁收口、swap 三段拆分、close/wait/validate 进程原语、`--close` 参数、单测 +6。
- net/src/lib.rs:UpdateConfig.close_processes、routes() 校验、drain_body、stage 拒绝分支 drain、
  apply argv/自退编排、单测 +1(真 TCP)。
- demo-app:`--close-processes <a.exe,b.exe>` 参数。
- tools/e2e.ps1:+REQ-2/REQ-3/REQ-1 三场景(见 TODO 核验节)。
- README/HowToUse:字段、回滚语义、排障表同步;ghost-push/tauri 零改动(推送端属非目标)。

### 假设

- worker 等关联进程超时(10s)按 pid 超时同款处理:放弃更新(文件未动),由 agent 已做过的
  优雅关闭 + /F 兜底保证实际到不了这一步。
- 换版后被关进程的自动重启留宿主(需求建议的非目标);Model Copy 计划用"GUI 原在运行"标记。
- 关联进程名单不进 manifest/不含版本概念,纯粹本机编排参数,故走 argv 不走暂存文件。

### 错误记录表(追加)

| 时间 | 位置 | 报错/问题 | 处置 |
|---|---|---|---|
| e2e | REQ-2 断言 | PS5.1 `Get-Content` 按 GBK 读 UTF-8 的 result.json,中文断言全乱码(与早前 BOM 坑同源) | `Get-Content -Raw -Encoding UTF8`;对外输出断言一律改 ASCII 锚点(如 `HTTP status client error` / `error sending request`) |
| e2e | REQ-1 推送 | dist-v4 含 2 个 exe,ghost-push 拒包"需用 --main 指定" | 推送加 `--main ghost-demo.exe`(既有规则,非 bug) |
````

## File: plan/TODO.md
````markdown
# TODO — ghost-updater 实现

目标:按 plan/design.md 实现 ghost-updater 通用 Rust workspace(远程主动推更:自替换核心 + 嵌入式 HTTP 路由 + 推送端 CLI/GUI)。

## 阶段

1. [done] workspace 脚手架:根 Cargo.toml(members: core/net/cli/demo-app;gui 排除在外)+ .gitignore
2. [done] ghost-updater-core:STARTED_AT、guard_apply_update、wait_pid_exit(OpenProcess+taskkill 兜底)、swap_files/restore_backup(.update_backup/)、分离拉起(CREATE_NO_WINDOW、禁管道、可选 worker 日志)、apply.log/result.json、apply 锁;7 个单测全绿
3. [done] ghost-updater-net:routes() 可合并进宿主 Router + start_agent();probe/stage/apply;SHA256 边收边校验;Bearer Token 强制(未配 token/公钥 → 503);可选 ed25519 验签;tar 扁平白名单+穿越拒绝;staging 绝对路径;7 个单测全绿
4. [done] demo-app:guard + axum 合并 routes(),VERSION 文件读版本,--addr/--token
5. [done] ghost-updater-cli(lib + bin ghost-push):probe/stage/apply/poll 编排,poll 成功规则复刻生产(started_at 变化 + 版本匹配),dist→tar+SHA256,--jobs 并发,进度行,targets-file,keygen
6. [done] tools/e2e.ps1:全绿 —— v1 起 → 无 token stage 401 → ghost-push 推 v2 → 版本 1.0.0→2.0.0、started_at 变化、config.json 保留、备份清理、result.json ok
7. [done] ghost-updater-tauri:Tauri 2 + Svelte 5 GUI(主机列表/持久化、dist 预览+整包哈希、单个/选中/全部/重试失败、update-log 实时滚动);npm run build ✓ + cargo check ✓
8. [done] README.md + design.md 假设/错误表收尾

## 完成标准核验

- `cargo build --workspace` ✓ / `cargo test --workspace` 14/14 ✓
- e2e.ps1 全绿 ✓(含 started_at 确认重启、401 安全边界)
- gui: `npm run build` 产 webdist ✓ + `cargo check` ✓(gui 不阻塞主 workspace)
- 所有变更路由强制安全校验 ✓;guard 零网络依赖 ✓

## 追加:GUI 构建/运行/打包统一入口脚本

1. [done] tools/gui.ps1:子命令 dev / run / build / check / installer / verify + 无参 usage;绝对路径,任意 cwd 可用;
   run/installer 前 Stop-Running(上次实例锁 exe → os error 5)
2. [done] tools/gui.cmd:cmd 下的薄包装(`tools\gui.cmd dev`),纯 ASCII(GBK 会嚼烂中文注释),透传退出码
3. [done] README/HowToUse 补脚本用法表,并写明"不要直接 cargo run(debug 走 devUrl 会拒连)"
4. [done] 实测:check ✓ / run ✓(截图确认窗口渲染出界面,非报错页)/ verify ✓(14 单测全绿);
   installer 未跑(首次会拉 NSIS 工具链)

## 追加:项目整档保存(更新包设置并入项目)

1. [done] main.rs:AppProfile 扩 6 字段 + 旧 settings 一次性迁移(迁移即落盘、删旧文件)+ 删 settings 命令
2. [done] App.svelte:更新包面板直接绑定当前项目(去全局 settings);函数内用 currentProject() 规避 $: 派生变量的同步滞后
3. [done] 文案 应用→项目;README/HowToUse 各补一句
4. [done] `npm run build` ✓ + `cargo check`(GUI crate)✓

## 追加:来自 Model Copy 生产集成的三项需求(2026-10-10)

1. [done] REQ-2(P1) swap 失败回滚 + 所有失败路径释放 .apply-lock(core)
2. [done] REQ-3(P2) stage/apply 拒绝路径 drain 请求体,401 稳定可见(net)
3. [done] REQ-1(P2) UpdateConfig.close_processes + worker 多进程等待编排(net/core;Model Copy 已有宿主侧中间件 workaround)
   详案(三语):plan/feature-requests-from-model-copy.md;实施取舍:plan/design.md 追加变更节

## 完成标准核验(三需求批次)

- `cargo build --workspace` ✓ / `cargo test --workspace` 21/21 ✓(存量 14 + 新增 7:
  swap 失败回滚+锁释放[windows]、进程名白名单、--close 解析、按名探活/等待/优雅关闭、
  真 TCP 大包体 401 可见)
- e2e.ps1 全绿 ✓,新增三场景:
  - REQ-2:锁 NOTES.txt(share Read)逼停覆盖段 → result 标注"已回滚"、VERSION 回 2.0.0、
    全部 stage-* 无 .apply-lock、旧版无参重拉(7600)可达;释放锁重推 2.0.0→3.0.0 ✓
  - REQ-3:错 token 推 17MB ×10 → 每次都是 error_for_status 401,0 次 "error sending request"
  - REQ-1:demo-gui.exe 常驻持锁 + `--close-processes demo-gui.exe` → 优雅关闭(taskkill 无 /F 即成,
    未走到 /F 兜底)→ 4 文件全量 swap → 主程序自恢复 3.0.0→4.0.0,demo-gui.exe 哈希换新 ✓
- GUI(tauri)零改动且不依赖 net/core 新接口,未重跑 gui check;wire 协议零变化 ✓
````

## File: tools/e2e.ps1
````powershell
# e2e.ps1 — ghost-updater 端到端验证
# 构建一次 demo-app + ghost-push,组装 v1/v2 两个 dist,真实走完
# probe → stage(哈希校验) → apply(分离拉起 apply worker) → 等旧进程退出 →
# 备份/覆盖/重启 → 轮询 started_at 确认重启 的全链路。
# 用法: powershell -ExecutionPolicy Bypass -File tools\e2e.ps1
$ErrorActionPreference = "Stop"

$Root   = Split-Path -Parent $PSScriptRoot
$E2E    = Join-Path $Root "target\e2e"
$Port   = 7601
$Token  = "e2e-token"
$Target = "127.0.0.1:$Port"

function Fail($msg) { Write-Host "[FAIL] $msg" -ForegroundColor Red; exit 1 }
function Ok($msg)   { Write-Host "[OK]   $msg" -ForegroundColor Green }

# 0. 清场(先杀残留进程,再删工作目录)
Get-Process ghost-demo -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
Get-Process demo-gui  -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
Start-Sleep -Milliseconds 300
if (Test-Path $E2E) { Remove-Item -Recurse -Force $E2E }
New-Item -ItemType Directory -Force -Path "$E2E\dist-v1", "$E2E\dist-v2", "$E2E\install", "$E2E\dist-v3", "$E2E\dist-v3-big", "$E2E\dist-v4" | Out-Null

# 1. 构建(demo-app 与 ghost-push 一次构建)
Write-Host "== cargo build =="
Push-Location $Root
cmd /c "cargo build --workspace >nul 2>&1"
if ($LASTEXITCODE -ne 0) { Pop-Location; Fail "cargo build" }
Pop-Location
$Demo  = Join-Path $Root "target\debug\ghost-demo.exe"
$Push  = Join-Path $Root "target\debug\ghost-push.exe"
Ok "构建完成"

# 2. 组装 dist:v1(装机)与 v2(新版)
Copy-Item $Demo "$E2E\dist-v1\ghost-demo.exe"
Set-Content -Encoding Ascii "$E2E\dist-v1\VERSION" "1.0.0"
Set-Content -Encoding Ascii "$E2E\dist-v1\NOTES.txt" "v1 notes"
Copy-Item $Demo "$E2E\dist-v2\ghost-demo.exe"
Set-Content -Encoding Ascii "$E2E\dist-v2\VERSION" "2.0.0"
Set-Content -Encoding Ascii "$E2E\dist-v2\NOTES.txt" "v2 notes"
# 安装目录 = v1 dist + 宿主自定义配置(config.json 不推,必须保留)
Copy-Item -Recurse -Force "$E2E\dist-v1\*" "$E2E\install\"
Set-Content -Encoding Ascii "$E2E\install\config.json" "{""keep"":true}"
Ok "dist 就绪 v1=1.0.0 v2=2.0.0"

# 3. 起 v1(带 Token)
$v1 = Start-Process -FilePath "$E2E\install\ghost-demo.exe" `
    -ArgumentList "--addr","$Target","--token",$Token `
    -WorkingDirectory $E2E -PassThru -WindowStyle Hidden
Start-Sleep -Milliseconds 1500
if ($v1.HasExited) { Fail "v1 启动失败" }

# 4. probe v1
$probe1 = Invoke-RestMethod "http://$Target/update/probe"
if ($probe1.version -ne "1.0.0") { Fail "probe v1 版本非 1.0.0: $($probe1.version)" }
Write-Host "       v1 probe: version=$($probe1.version) started_at=$($probe1.started_at) os=$($probe1.os) arch=$($probe1.arch)"
$oldStarted = $probe1.started_at

# 5. 无 token 的 stage 必须被拒(安全边界)
try {
    Invoke-RestMethod -Method Post "http://$Target/update/stage?sha256=00&main=x.exe" -InFile "$E2E\dist-v2\NOTES.txt" | Out-Null
    Fail "未带 token 的 stage 竟然成功"
} catch { Ok "未带 token 的 stage 被拒($($_.Exception.Response.StatusCode.value__))" }

# 6. ghost-push 推 v2(全链路)
Write-Host "== ghost-push v2 → $Target =="
& $Push push --target $Target --dist "$E2E\dist-v2" --token $Token --timeout-secs 60
if ($LASTEXITCODE -ne 0) { Fail "ghost-push 退出码 $LASTEXITCODE" }
Ok "ghost-push 成功"

# 7. 断言:版本变化 + started_at 变化 + 配置保留 + NOTES 换新 + 无残留备份
$probe2 = Invoke-RestMethod "http://$Target/update/probe"
if ($probe2.version -ne "2.0.0") { Fail "更新后版本非 2.0.0: $($probe2.version)" }
if ($probe2.started_at -eq $oldStarted) { Fail "started_at 未变化,疑似没重启" }
$notes = Get-Content "$E2E\install\NOTES.txt" -Raw
if ($notes.Trim() -ne "v2 notes") { Fail "NOTES.txt 未换新: $notes" }
$cfg = Get-Content "$E2E\install\config.json" -Raw
if ($cfg -notmatch '"keep"') { Fail "config.json 被误覆盖: $cfg" }
# 备份清理发生在 apply worker 健康检查之后(比 pusher 的重启确认晚 ~1.5s),轮询等它
$cleaned = $false
for ($i = 0; $i -lt 15; $i++) {
    if (-not (Test-Path "$E2E\install\.update_backup\backup-*")) { $cleaned = $true; break }
    Start-Sleep -Seconds 1
}
if (-not $cleaned) { Fail "成功更新后备份目录未清理" }
if (-not (Test-Path "$E2E\install\.update_backup\apply.log")) { Fail "缺 apply.log" }
$result = Get-Content "$E2E\install\.update_backup\result.json" -Raw | ConvertFrom-Json
if (-not $result.ok) { Fail "result.json ok=false: $($result.error)" }
Write-Host "       v2 probe: version=$($probe2.version) started_at=$($probe2.started_at)"
Ok "版本 1.0.0 → 2.0.0,config 保留,备份已清,result.json ok"

# 8. REQ-2:锁住安装目录文件逼停 swap → 自动回滚 + 锁释放,重推可成功
Copy-Item $Demo "$E2E\dist-v3\ghost-demo.exe"
Set-Content -Encoding Ascii "$E2E\dist-v3\VERSION" "3.0.0"
Set-Content -Encoding Ascii "$E2E\dist-v3\NOTES.txt" "v3 notes"
# 锁"写"不锁"读"(FileShare.Read):worker 备份读得动,覆盖写必败(os error 32)
$lock = [System.IO.File]::Open("$E2E\install\NOTES.txt", 'Open', 'Read', 'Read')
Write-Host "== REQ-2: 锁 NOTES.txt 后推 v3,期望失败并自动回滚 =="
$out = cmd /c "$Push push --target $Target --dist $E2E\dist-v3 --token $Token --timeout-secs 15 2>&1" | Out-String
$rc = $LASTEXITCODE
$lock.Close()
if ($rc -eq 0) { Fail "锁文件状态下 v3 竟推送成功: $($out.Trim())" }
$result = Get-Content -Raw -Encoding UTF8 "$E2E\install\.update_backup\result.json" | ConvertFrom-Json
if ($result.ok) { Fail "REQ-2 result.json ok=true: $($result.error)" }
if ($result.error -notmatch "已回滚") { Fail "REQ-2 result 未标注已回滚: $($result.error)" }
if ((Get-Content "$E2E\install\VERSION" -Raw).Trim() -ne "2.0.0") { Fail "REQ-2 回滚后 VERSION 非 2.0.0" }
$staleLock = Get-ChildItem "$E2E\.ghost-update" -Directory | Where-Object { Test-Path (Join-Path $_.FullName ".apply-lock") }
if ($staleLock) { Fail "REQ-2 失败后 .apply-lock 泄漏: $($staleLock.Name -join ',')" }
# 回滚后 worker 重拉旧版是无参启动,落默认端口 7600 —— 顺带验证服务已恢复
$rb = $null
for ($i = 0; $i -lt 10; $i++) {
    try { $rb = Invoke-RestMethod "http://127.0.0.1:7600/update/probe"; break } catch { Start-Sleep -Milliseconds 500 }
}
if (-not $rb -or $rb.version -ne "2.0.0") { Fail "REQ-2 回滚重拉不可达或版本非 2.0.0" }
Ok "REQ-2 swap 失败 → result 标注已回滚、无锁泄漏、旧版已重拉"

# 回滚重拉是无参启动,杀掉后带参重启,供后续场景继续用
Get-Process ghost-demo -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
Start-Sleep -Milliseconds 300
Start-Process -FilePath "$E2E\install\ghost-demo.exe" -ArgumentList "--addr","$Target","--token",$Token `
    -WorkingDirectory $E2E -PassThru -WindowStyle Hidden | Out-Null
Start-Sleep -Milliseconds 1500
Write-Host "== REQ-2: 释放锁后重推 v3,期望成功 =="
& $Push push --target $Target --dist "$E2E\dist-v3" --token $Token --timeout-secs 60
if ($LASTEXITCODE -ne 0) { Fail "REQ-2 重推 v3 失败" }
if ((Invoke-RestMethod "http://$Target/update/probe").version -ne "3.0.0") { Fail "REQ-2 重推后版本非 3.0.0" }
Ok "REQ-2 重推 v3 成功(2.0.0 → 3.0.0)"

# 9. REQ-3:错 token 推 17MB 大包 ×10,每次都必须是 stage 被拒 401,而非传输错误
Copy-Item -Recurse -Force "$E2E\dist-v3\*" "$E2E\dist-v3-big\"
[System.IO.File]::WriteAllBytes("$E2E\dist-v3-big\junk.bin", (New-Object byte[] (17MB)))
Write-Host "== REQ-3: 错 token 推 17MB ×10,期望每次都稳定看到 401 =="
for ($i = 1; $i -le 10; $i++) {
    $out = cmd /c "$Push push --target $Target --dist $E2E\dist-v3-big --token wrong-$i --timeout-secs 5 2>&1" | Out-String
    if ($LASTEXITCODE -eq 0) { Fail "REQ-3 第 $i 次错 token 竟成功: $($out.Trim())" }
    # 断言用 ASCII 锚点(cmd 捕获会按 GBK 解码 ghost-push 的 UTF-8 输出,中文正则不可靠):
    # "HTTP status client error" = error_for_status 路径(401 可见);"error sending request" = 传输错误
    if ($out -notmatch "401") { Fail "REQ-3 第 $i 次未见 401: $($out.Trim())" }
    if ($out -notmatch "HTTP status client error") { Fail "REQ-3 第 $i 次非 error_for_status 路径: $($out.Trim())" }
    if ($out -match "error sending request") { Fail "REQ-3 第 $i 次仍是传输错误(401 不可见): $($out.Trim())" }
}
Ok "REQ-3 错 token ×10 全部稳定 401,0 次传输错误"

# 10. REQ-1:双进程编排——demo-gui.exe 常驻并锁住自身,agent 优雅关闭后全量 swap 成功
Copy-Item $Demo "$E2E\install\demo-gui.exe"
Start-Process -FilePath "$E2E\install\demo-gui.exe" -ArgumentList "--addr","127.0.0.1:7701","--token",$Token `
    -WorkingDirectory $E2E -PassThru -WindowStyle Hidden | Out-Null
Start-Sleep -Milliseconds 1500
if (-not (Get-Process demo-gui -ErrorAction SilentlyContinue)) { Fail "demo-gui 启动失败" }
if ((Invoke-RestMethod "http://127.0.0.1:7701/update/probe").version -ne "3.0.0") { Fail "demo-gui probe 非 3.0.0" }
Get-Process ghost-demo -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
Start-Sleep -Milliseconds 300
Start-Process -FilePath "$E2E\install\ghost-demo.exe" -ArgumentList "--addr","$Target","--token",$Token,"--close-processes","demo-gui.exe" `
    -WorkingDirectory $E2E -PassThru -WindowStyle Hidden | Out-Null
Start-Sleep -Milliseconds 1500
Copy-Item $Demo "$E2E\dist-v4\ghost-demo.exe"
Copy-Item $Demo "$E2E\dist-v4\demo-gui.exe"
Set-Content -Encoding Ascii "$E2E\dist-v4\VERSION" "4.0.0"
Set-Content -Encoding Ascii "$E2E\dist-v4\NOTES.txt" "v4 notes"
Write-Host "== REQ-1: GUI(demo-gui)开着推 v4,期望优雅关闭后 swap 成功 =="
& $Push push --target $Target --dist "$E2E\dist-v4" --main ghost-demo.exe --token $Token --timeout-secs 60
if ($LASTEXITCODE -ne 0) { Fail "REQ-1 推 v4 失败" }
Start-Sleep -Seconds 1
if (Get-Process demo-gui -ErrorAction SilentlyContinue) { Fail "REQ-1 demo-gui 未被关闭" }
if ((Invoke-RestMethod "http://$Target/update/probe").version -ne "4.0.0") { Fail "REQ-1 推送后版本非 4.0.0" }
if ((Get-FileHash "$E2E\install\demo-gui.exe").Hash -ne (Get-FileHash "$E2E\dist-v4\demo-gui.exe").Hash) { Fail "REQ-1 demo-gui.exe 未被换新(锁未释放?)" }
Ok "REQ-1 demo-gui 被关闭、全量 swap 成功、主程序自恢复(3.0.0 → 4.0.0)"

# 11. 清理
Get-Process ghost-demo -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
Get-Process demo-gui  -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
Write-Host ""
Write-Host "E2E 全绿" -ForegroundColor Green
````

## File: .gitignore
````
/target
crates/ghost-updater-tauri/src-tauri/target
crates/ghost-updater-tauri/node_modules
crates/ghost-updater-tauri/webdist
*.update_backup/
.ghost-update/
````

## File: Cargo.toml
````toml
[workspace]
resolver = "3"
# ghost-updater-tauri 排除在外:generate_context! 编译期读 frontendDist(webdist),
# 需要先 npm run build;独立构建,见 crates/ghost-updater-tauri。
members = [
    "crates/ghost-updater-core",
    "crates/ghost-updater-net",
    "crates/ghost-updater-cli",
    "crates/demo-app",
]

[workspace.package]
version = "0.1.0"
edition = "2024"
license = "MIT"

[workspace.dependencies]
tokio = { version = "1", features = ["full"] }
axum = "0.8"
http-body-util = "0.1"
tower = { version = "0.5", features = ["util"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
sha2 = "0.10"
hex = "0.4"
tar = "0.4"
ed25519-dalek = { version = "2", features = ["rand_core"] }
rand = "0.8"
reqwest = { version = "0.12", default-features = false, features = ["json", "stream"] }
tokio-util = { version = "0.7", features = ["codec"] }
windows-sys = { version = "0.59", features = ["Win32_Foundation", "Win32_System_Threading"] }

[profile.release]
strip = true
lto = true
````

## File: HowToUseAutoUpdate.md
````markdown
# HowToUseAutoUpdate — 给你的 Rust 项目接入远程一键推更

> 仓库:https://github.com/orangepro2001/AutoUpdater
> 你只需要读完本文前两节,复制几段代码,你的服务就具备:远程推送新版 → 自动备份 → 自替换 → 自动重启 → 失败自动回滚 的完整能力。

---

## 0. 三十秒理解

- **你的程序**(宿主)在 main 里加一行守卫 + 把一组 HTTP 路由合并进现有服务(或独立监听一个端口),不新增部署复杂度。
- **推送端**(你自己的机器或 CI)跑 `ghost-push`,把新版 dist 目录推给目标:探测 → 上传暂存(整包 SHA256 校验)→ 触发自替换 → 轮询确认重启成功。
- 目标机器上的流程全自动:等旧进程退出 → 备份到 `.update_backup/` → 覆盖安装目录 → 拉起新版 → 1.5s 存活判定;**失败自动回滚旧版**。`config.json` 默认永不被覆盖(各机配置不同)。

```
你的项目 main.rs(宿主)          ghost-push(推送端)
  │ guard_apply_update()            │ 1. GET  /update/probe   探测版本
  │ .merge(ghost_updater_net::      │ 2. POST /update/stage   传包+SHA256
  │        routes(UpdateConfig))    │ 3. POST /update/apply   触发自替换
  ▼                                 │ 4. 轮询 probe 直到版本/started_at 变化
 新进程 ← apply worker(暂存区副本)─┘   (等旧进程退出→备份→覆盖→重启→回滚兜底)
```

安全边界(强制):配了 Token 走 `Bearer` 鉴权;只配了公钥则强制 ed25519 验签;**两者都不配时更新接口直接 503 拒绝服务**,不会裸奔。

---

## 1. 宿主接入(三步,五分钟)

### 第 1 步:加依赖

```toml
# 你项目的 Cargo.toml
[dependencies]
ghost-updater-core = { git = "https://github.com/orangepro2001/AutoUpdater.git" }
ghost-updater-net  = { git = "https://github.com/orangepro2001/AutoUpdater.git" }
# SSH 方式等价:git = "git@github.com:orangepro2001/AutoUpdater.git"
```

> core 会被 net 自动带进来,写上是为了 main.rs 里能直接 use。**不要**引 `ghost-updater-cli`——那是推送端才用的。

### 第 2 步:main 顶部加守卫(必须是最先执行的一行)

```rust
fn main() {
    // 必须在任何 runtime / 线程 / 业务初始化之前
    ghost_updater_core::guard_apply_update();

    // ↓↓ 你原有的启动代码原样保留 ↓↓
    // ...
}
```

这一行平时什么都不做;只有当本进程被更新流程从暂存区拉起时,它才会接管:完成换版、重启新版,然后直接退出,永不执行到你的业务代码。

### 第 3 步:合并更新路由

```rust
use ghost_updater_net::UpdateConfig;

let app = axum::Router::new()
    .route("/your/api", /* 你原有的路由 */)
    .merge(ghost_updater_net::routes(UpdateConfig {
        version: env!("CARGO_PKG_VERSION").to_string(),  // 宿主版本号
        auth_token: Some("换成长随机串".into()),          // 预共享 Token
        ..Default::default()
    }));
```

宿主不是 Axum 服务?独立起一个控制面 agent 即可(占用 1 个端口):

```rust
ghost_updater_net::start_agent("0.0.0.0:7600", env!("CARGO_PKG_VERSION"), Some("换成长随机串".into())).await?;
```

**多进程宿主(GUI + 常驻代理这类)**:再加一个 `close_processes: vec!["your-gui.exe".into()]`
(进程映像名列表,默认空 = 原有行为)。apply 受理且鉴权通过后,agent 会优雅关闭名单内进程
(Windows:`taskkill` 无 `/F` → 5s 宽限 → `/F` 兜底)再自退;apply worker 会等旧进程与
名单内进程全部退出后才动文件,避免 GUI 持有包内文件导致 swap 必败。名单经白名单校验
(仅字母数字与 `. _ -`),非法名字启动即 panic。

### 第 4 步(约定):dist 里放一个 VERSION 文件

你的发布产物(dist 目录)顶层放一个纯文本 `VERSION`(内容如 `2.1.0`)。推送端靠它判断"这次推送是否真的换版成功"。没有它也能推,但同版本重推时的成功判定会退化成"只确认重启"。

**完成。** 编译,把 exe(连同 VERSION/config.json 等)部署到目标机,正常启动。

---

## 2. 推送新版

在你的开发机/CI 上(装了 Rust):

```bash
# 一次性安装推送工具(装完得到 ghost-push 命令)
cargo install --git https://github.com/orangepro2001/AutoUpdater.git ghost-updater-cli

# 单目标推更
ghost-push push --target 192.168.1.60 --dist D:\build\dist --token 换成长随机串

# 多目标并发
ghost-push push --targets-file hosts.txt --dist .\dist --token 换成长随机串 --jobs 8
# hosts.txt 每行一个地址(IP 或 IP:端口,# 开头是注释);退出码 0 = 全部成功
```

**dist 目录要求**:顶层普通文件(exe/dll/资源),**不放子目录**;`ghost-demo.exe` 式的主程序在 dist 里必须唯一(`*.exe` 唯一,否则加 `--main xxx.exe` 指定);`config.json` 默认不推(目标机配置保留),要强推加 `--overwrite-config`。

成功标志:命令打印 `[成功] ip  旧版本 → 新版本` 且退出码 0。失败时打印明细,单独 `--target` 重试失败节点即可。

**目标机重启后连不上?** 先看目标机安装目录 `.update_backup\apply.log` 和 `result.json`——等旧进程超时、换文件失败、新版秒退(已自动回滚)都会记录在内。

---

## 3. 可选:验签模式(比 Token 更强)

Token 防的是"别人能调你的更新接口";验签额外保证"推送的包本身没被篡改"。

```bash
ghost-push keygen
# 输出:
# sign-key(seed)  = <64位hex,私钥,只有推送端保管>
# pubkey(hex)     = <64位hex,公钥,发给宿主>
```

宿主改为:

```rust
UpdateConfig {
    version: ...,
    pubkey_hex: Some("<公钥hex>".into()),   // 配了它,stage 强制验签
    auth_token: None,                        // 二选一;两个都配则 Token 优先
    ..Default::default()
}
```

推送时带私钥:

```bash
ghost-push push --target 192.168.1.60 --dist .\dist --sign-key <私钥hex>
```

---

## 4. 可选:图形界面推送台(Tauri)

```bash
git clone https://github.com/orangepro2001/AutoUpdater.git
cd AutoUpdater/crates/ghost-updater-tauri
npm install
../../tools/gui.cmd run      # 构建前端 + 开窗口
```

`tools/gui.ps1` 是这个 GUI 的构建/运行/打包入口(cmd 下用 `tools\gui.cmd`):`run` 看界面、
`dev` 开发热更新、`build` 只构建前端、`check` 前端+`cargo check`、`installer` 出 NSIS 安装包、
`verify` 主 workspace build/test + GUI check。

**别直接 `cargo run`**:debug 构建下 Tauri 会忽略已构建的前端,转去加载 `devUrl`
(http://localhost:5173),而那个 Vite 服务只有 `tauri dev` 会拉起,于是窗口报
`ERR_CONNECTION_REFUSED`。手工等价命令是 `cd src-tauri && cargo run --features tauri/custom-protocol`。

功能:项目整档保存(每套 = 项目名 + 默认端口 + 机器列表 + 更新包设置「dist 目录/主程序/覆盖 config/Token/签名私钥/轮询超时」,一并持久化 apps.json),下拉切换即整档切走推送现场;批量粘贴 IP 加机器、跨项目整批导入;dist 拖入即显示版本/文件集/整包哈希;单个/选中/全部/重试失败;实时滚动日志。同一台机器上多个应用各占一个端口,每个应用存成一个项目,机器按 ip:端口 逐台配置。

---

## 5. 硬约束与排障速查

| 事项 | 说明 |
|---|---|
| 更新包内容 | dist 顶层普通文件的 tar;**拒绝子目录、路径分隔符、`..`、符号链接**(防穿越) |
| 单包上限 | 2 GiB |
| 默认端口 | 7600(避开常见 Hyper-V 保留段;被占就换,连同 `--target ip:端口` 一起改) |
| 重启参数 | apply 时默认按**原样透传宿主本进程的启动参数**重启,`--addr/--token` 等不会丢 |
| 成功判定 | dist 的 VERSION 与目标旧版本不同 → 必须看到新版本;同版本重推/无 VERSION → started_at 变化即成功 |
| 传一半断网 | 暂存区校验不过直接丢弃,运行中的主进程不受影响;重推即可 |
| 被拒却报传输错误 | 不会了——stage/apply 拒绝前会把请求体读完再响应,流式上传大包途中被拒也能稳定看到 401/4xx |
| 排障入口 | 目标机 `<安装目录>/.update_backup/apply.log`、`result.json`;暂存区在 `<启动目录>/.ghost-update/` |
| 平台 | Windows 优先(生产验证);Unix 可用但等待超时后无 taskkill 兜底,超时即放弃更新(文件不动);`close_processes` 仅 Windows 生效 |
| 回滚 | 新版 1.5s 内退出 → 自动还原备份并重拉旧版;swap 中途失败(如文件被占用)→ 当场还原已覆盖文件,`result.json` 标注"已回滚";回滚也失败会写明日志,需人工看 `.update_backup/backup-*/` |
| apply 失败后 | 所有失败路径都会释放 `.apply-lock`,处理完占用源后**同包直接重推**即可,无需手工清锁 |

**CI 集成最小示例(GitHub Actions):**

```yaml
- run: cargo install --git https://github.com/orangepro2001/AutoUpdater.git ghost-updater-cli
- run: echo "${{ secrets.GHOST_TOKEN }}" | ghost-push push --target ${{ vars.TARGET }} --dist ./dist --token "${{ secrets.GHOST_TOKEN }}"
```

---

## 6. 原理一页纸(想改代码时再看)

- **workspace 结构**:`ghost-updater-core`(自替换,零网络依赖)/ `ghost-updater-net`(Axum 路由)/ `ghost-updater-cli`(推送编排 + bin)/ `demo-app`(示例宿主 + e2e)。
- **apply worker**:apply 接口从暂存区分离拉起新版 exe,参数 `--internal-apply-update --pid <旧进程> --install <安装目录> --staging <暂存目录> [--close <进程名,进程名>] -- <原启动参数>`;worker 完成换版后 exit,宿主的 `guard_apply_update()` 在 main 顶部截获的就是这次启动。
- **防半包**:stage 边收边算 SHA256,与查询参数不符即整目录丢弃;收完解包 tar 时再次白名单校验每个文件名。
- **防竞态**:推送端不靠"连接断开"判断重启(网络空窗是竞态),靠 probe 返回的 `started_at`(进程启动 Unix 秒)变化。
- **一键自测**:`powershell -ExecutionPolicy Bypass -File tools\e2e.ps1` 会在本机真实换版一轮并断言全链路。
````

## File: README.md
````markdown
# ghost-updater — 通用远程主动推更器

Rust workspace:把"远程推送新版、目标进程幽灵自替换、重启接力"标准化成三个可复用模块。
设计文档见 [plan/design.md](plan/design.md);完整实现参考(生产验证过):`C:\develop\File-Transfer\updater`。

```
┌──────────────────────┬──────────────────────┬─────────────────────┐
│ ghost-updater-core   │ ghost-updater-net    │ ghost-updater-cli   │
│ 自替换/锁/跨平台      │ Axum /update/* 路由  │ ghost-push CLI + 库 │
│ (零网络依赖)          │ probe/stage/apply    │ 推送编排,GUI 共用   │
└──────────────────────┴──────────────────────┴─────────────────────┘
        ghost-updater-tauri(推送端 GUI,Tauri 2 + Svelte,独立构建)
        demo-app(演示宿主 + e2e 测试床)
```

## 三个 crate 各自怎么用

### 1. 宿主接入(两行)

```rust
use ghost_updater_net::UpdateConfig;

fn main() {
    // 拦截内部触发的更新替换模式:如果是 apply worker,它会等旧进程退出、
    // 换文件、重启新版,然后直接 exit(永不返回);正常启动则立即 return
    ghost_updater_core::guard_apply_update();

    // ... 建你的 tokio runtime / 服务,然后把更新路由合并进来(新增 0 端口):
    let app = axum::Router::new()
        .route("/hello", get(|| async { "world" }))
        .merge(ghost_updater_net::routes(UpdateConfig {
            version: env!("CARGO_PKG_VERSION").into(),
            auth_token: Some("预共享Token".into()),   // 或者只用 pubkey_hex 验签
            ..Default::default()
        }));
    // 纯桌面/本地工具也可以独立监听一个控制面端口:
    // ghost_updater_net::start_agent("0.0.0.0:7600", "0.1.0", Some("token".into())).await
}
```

### 2. API(三段式)

| 路由 | 作用 |
|---|---|
| `GET /update/probe` | 当前 version / started_at / os / arch / 公钥指纹(公开) |
| `POST /update/stage?sha256=<hex>&main=<exe>` | tar 更新包流式写入暂存区,边收边算 SHA256;可选 `X-Signature`(ed25519 对摘要签名) |
| `POST /update/apply` | 校验暂存区 → 分离拉起暂存区新 exe(`--internal-apply-update`)→ 200 后 ~800ms 自退,接力给新 exe |

安全边界(强制,二选一):配了 `auth_token` 走 Bearer;只配了 `pubkey_hex` 走验签;
两者都没配时 stage/apply 返回 503,拒绝裸奔。probe 永远公开。
stage/apply 拒绝(401/503/4xx)前会把请求体读完(drain,上限即单包上限)再响应,
客户端在流式上传大包途中被拒时也能稳定看到状态码,而非随机连接重置。

apply worker 的动作(core):等旧进程退出(15s 超时,Windows `taskkill /F` 兜底)→
等 `close_processes` 名单内关联进程全部退出 → 备份到 `<安装目录>/.update_backup/backup-<ts>/` →
覆盖安装目录(`config.json` 等未列入清单的文件不动)→ 分离拉起新 exe
(`DETACHED_PROCESS|CREATE_NO_WINDOW`,stdio 不走管道,避免句柄继承死锁)→
1.5s 存活判定健康;失败自动回滚备份并重拉旧版。swap 失败(如文件被占用)会当场
把已覆盖的文件按备份还原(Err 时安装目录已回旧状态),且所有失败路径都会释放
`.apply-lock`,同包可直接重推。全程写 `.update_backup/apply.log` 与 `result.json`。

多进程宿主(REQ):`UpdateConfig.close_processes: Vec<String>` 填进程映像名
(如 `vec!["app-gui.exe".into()]`);apply 受理并通过鉴权后,agent 会优雅关闭名单内进程
(Windows:`taskkill` 无 `/F` → 5s 宽限 → `/F` 兜底)再自退;worker 等旧进程与名单内
进程全部退出后才动文件。空名单 = 原有零配置行为。

### 3. 推送端 CLI

```powershell
cargo build --workspace
# 生成 ed25519 密钥对(验签模式用)
target\debug\ghost-push.exe keygen
# 推更:目标探测 → 打包 dist(自动挑唯一 *.exe 作主程序)→ 暂存 → apply → 轮询确认
target\debug\ghost-push.exe push --target 192.168.1.60 --dist .\dist --token 预共享Token
# 多目标并发
target\debug\ghost-push.exe push --targets-file hosts.txt --dist .\dist --token t --jobs 8
```

成功判定(复刻生产规则):dist 有 VERSION 且与旧版本不同 → 必须看到新版本;
同版本重部署 / 无 VERSION → `started_at` 变化即确认重启。默认端口 7600(避开常见
Hyper-V 保留段)。`config.json` 默认不推送(各机配置不同),`--overwrite-config` 强推。

### 4. 推送端 GUI(Tauri 2 + Svelte)

```powershell
cd crates\ghost-updater-tauri
npm install
..\..\tools\gui.cmd run        # 构建前端 + 开窗口(等价于下面两步)
```

`tools\gui.ps1`(薄包装 `tools\gui.cmd`,任意目录可用)统一管这几件事:

| 命令 | 作用 |
|---|---|
| `run` | 构建前端 + debug 编译(内嵌 webdist)+ 开窗口,日常看界面用这个 |
| `dev` | 开发模式:`tauri dev`,自动起 Vite,改前端热更新 |
| `build` | 只重新构建前端 → `webdist` |
| `check` | 构建前端 + GUI crate `cargo check` |
| `installer` | 正式打包 → NSIS 安装包 |
| `verify` | 主 workspace `cargo build/test` + GUI check |

**不要直接 `cargo run`**:debug 构建下 Tauri 会忽略已构建的 `webdist`,改让窗口去加载
`tauri.conf.json` 的 `devUrl`(http://localhost:5173);而那个 Vite 开发服务器只有 `tauri dev`
才会自动拉起,所以裸 `cargo run` 必然报 `ERR_CONNECTION_REFUSED`。手工等价命令是
`cd src-tauri; cargo run --features tauri/custom-protocol`(`custom-protocol` 让编译期改用
内嵌 webdist,不再走 devUrl);脚本的 `run` 用的是官方 CLI 路径 `tauri build --debug --no-bundle`,
顺带把上一次运行残留的实例关掉(否则旧进程锁住 exe,cargo 覆盖报 os error 5)。

功能:项目整档保存(每套 = 项目名/默认端口/机器列表 + 该应用的更新包设置:dist 目录、
主程序、覆盖 config、Token、签名私钥、轮询超时,一并持久化 apps.json,下拉即切换整个
推送现场)、批量粘贴 IP 一次加机器、从其他项目整批导入机器、dist 选包预览(版本/
主程序/文件集/整包哈希)、单个/选中/全部/重试失败 更新、按主机着色的实时滚动日志。
同一台机器上多个应用各占一个端口,每个应用存成一个项目,机器按 ip:端口 逐台配置。

## 构建与测试

```powershell
cargo build --workspace        # 主 workspace(core/net/cli/demo-app)
cargo test  --workspace        # 14 个单测:swap/回滚/穿越拒绝/wait_pid/taskkill/鉴权/暂存…
powershell -ExecutionPolicy Bypass -File tools\e2e.ps1   # 端到端:真实换版 1.0.0→2.0.0
```

e2e 内容:组装 v1/v2 dist → 起 v1 → 校验无 token 的 stage 被拒 → ghost-push 推 v2 →
断言版本切换、`started_at` 变化、`config.json` 保留、备份清理、`result.json ok`。

## 边界与约定

- 更新包 = 顶层普通文件的 tar;含路径分隔符/`..`/符号链接的条目一律拒绝(防穿越)。
- 中途断网只会留下垃圾暂存目录,校验不过不入安装目录;运行中的主进程不受影响。
- 单包上限 2 GiB(axum `DefaultBodyLimit`)。
- Windows 优先(生产环境);Unix 走 `kill(pid,0)`,无 taskkill 兜底则超时放弃更新。
````
