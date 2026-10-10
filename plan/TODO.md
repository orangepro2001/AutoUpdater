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
