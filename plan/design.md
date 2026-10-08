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