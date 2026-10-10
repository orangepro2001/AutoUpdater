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
