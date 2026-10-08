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
