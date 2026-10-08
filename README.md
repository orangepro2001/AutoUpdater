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

apply worker 的动作(core):等旧进程退出(15s 超时,Windows `taskkill /F` 兜底)→
备份到 `<安装目录>/.update_backup/backup-<ts>/` → 覆盖安装目录(`config.json` 等未列入
清单的文件不动)→ 分离拉起新 exe(`DETACHED_PROCESS|CREATE_NO_WINDOW`,stdio 不走管道,
避免句柄继承死锁)→ 1.5s 存活判定健康;失败自动回滚备份并重拉旧版。
全程写 `.update_backup/apply.log` 与 `result.json`。

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
npm run build          # 产 webdist(src-tauri 编译期要读)
cd src-tauri
cargo run              # 或 cargo tauri build 出 NSIS 安装包
```

功能:dist 选包预览(版本/主程序/文件集/整包哈希)、主机列表(探测/版本/状态,
持久化 hosts.json)、单个/选中/全部/重试失败 更新、按主机着色的实时滚动日志。

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
