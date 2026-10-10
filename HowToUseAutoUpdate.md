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
