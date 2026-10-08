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
