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
