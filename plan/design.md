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