//! ghost-updater 推送台(Tauri 2 桌面应用)。
//! 业务逻辑全部在 ghost-updater-cli 库(probe / push_update / collect_dist_files),
//! 这里只做:参数编排、进度事件转发给前端、主机列表持久化。

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use ghost_updater_cli::{collect_dist_files, normalize_addr, pack_dist_in_memory, probe, push_update, PushCtx, PushUpdateCfg};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tauri::{AppHandle, Emitter, Manager};

/// 主机列表条目(持久化到 app_config_dir/hosts.json)
#[derive(Serialize, Deserialize, Clone)]
struct HostEntry {
    addr: String,
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

fn hosts_file(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|e| format!("配置目录不可用: {e}"))?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建配置目录失败: {e}"))?;
    Ok(dir.join("hosts.json"))
}

#[tauri::command]
fn load_hosts(app: AppHandle) -> Vec<HostEntry> {
    let Ok(path) = hosts_file(&app) else { return vec![] };
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

#[tauri::command]
fn save_hosts(app: AppHandle, hosts: Vec<HostEntry>) -> Result<(), String> {
    let path = hosts_file(&app)?;
    let body = serde_json::to_string_pretty(&hosts).map_err(|e| e.to_string())?;
    std::fs::write(path, body).map_err(|e| format!("保存主机列表失败: {e}"))
}

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            cmd_probe,
            cmd_dist_info,
            cmd_update_host,
            load_hosts,
            save_hosts
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
