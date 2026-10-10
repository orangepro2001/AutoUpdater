//! ghost-updater-net:标准三段式更新 API,可嵌入任意 Axum 服务,也可独立起 agent。
//!
//!   GET  /update/probe  → 当前 version / started_at / os / arch / 公钥指纹
//!   POST /update/stage  → 更新包(tar)流式写入暂存目录,SHA256 整包校验
//!   POST /update/apply  → 200 确认后分离拉起暂存区新 exe 执行自替换,本进程随后退出
//!
//! 安全部(design:强制二选一):
//!   * 配了 auth_token → stage/apply 必须带 `Authorization: Bearer <token>`
//!   * 没配 token 但配了 pubkey_hex → stage 必须带有效 X-Signature(ed25519 对整包
//!     SHA256 摘要签名),apply 放行(只能触发已验签暂存的内容)
//!   * 两者都没配 → stage/apply 直接 503,拒绝裸奔;probe 永远公开
//!
//! 一键集成:
//!   Router::new().merge(ghost_updater_net::routes(UpdateConfig { .. }))
//! 或独立监听:ghost_updater_net::start_agent("0.0.0.0:9001", Some("token")).await;

use axum::body::Body;
use axum::extract::{DefaultBodyLimit, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use ghost_updater_core::{acquire_apply_lock, started_at, unix_secs, StageManifest, APPLY_FLAG};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::io::Write;

/// 暂存根目录名(cwd 相对;所有 stage-*/ 都在这下面)
pub const STAGING_ROOT: &str = ".ghost-update";
/// 单包体积上限(整包 tar 流式上传;默认 2 GiB 覆盖绝大多数安装包)
pub const MAX_PACKAGE_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// 进程内 stage 目录序号(时间戳 + 序号,防同秒并发撞名)
static STAGE_SEQ: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Default)]
pub struct UpdateConfig {
    /// 宿主当前版本号(probe 返回;宿主自己最清楚,如 CARGO_PKG_VERSION)
    pub version: String,
    /// 预共享 Token;Some 时 stage/apply 强制 Bearer 鉴权
    pub auth_token: Option<String>,
    /// ed25519 公钥 hex(64 字符);Some 时 stage 强制验签(token 未配时的另一条安全线)
    pub pubkey_hex: Option<String>,
    /// 安装目录(apply 覆盖目标;默认 current_exe 所在目录)
    pub install_dir: Option<PathBuf>,
    /// 暂存根目录(默认 cwd 相对 .ghost-update/)
    pub staging_root: Option<PathBuf>,
    /// apply 受理后需优雅关闭的本机进程映像名(REQ-1 多进程宿主编排;空 = 现状零改动)。
    /// agent 关闭 → 自退;worker 会再兜底等待名单内进程全部退出才动文件。
    pub close_processes: Vec<String>,
}

impl UpdateConfig {
    pub fn new(version: impl Into<String>) -> Self {
        Self {
            version: version.into(),
            auth_token: None,
            pubkey_hex: None,
            install_dir: None,
            staging_root: None,
            close_processes: Vec::new(),
        }
    }
}

#[derive(Clone)]
pub struct UpdateState {
    cfg: UpdateConfig,
    staging_root: PathBuf,
    pubkey: Option<VerifyingKey>,
    fingerprint: String,
}

/// 挂载到宿主 Router:`Router::new().merge(ghost_updater_net::routes(cfg))`
/// 公钥 hex 非法会在启动时直接 panic(配置错误越早暴露越好)。
pub fn routes(cfg: UpdateConfig) -> Router {
    let (pubkey, fingerprint) = match &cfg.pubkey_hex {
        Some(h) => {
            let bytes = decode_pubkey(h).unwrap_or_else(|e| panic!("pubkey_hex 非法: {e}"));
            let vk = VerifyingKey::from_bytes(&bytes).expect("pubkey_hex 非法: 不是有效 ed25519 公钥");
            let fp = {
                let d = Sha256::digest(bytes);
                hex::encode(&d[..8])
            };
            (Some(vk), fp)
        }
        None => (None, String::new()),
    };
    let staging_root = cfg.staging_root.clone().unwrap_or_else(|| PathBuf::from(STAGING_ROOT));
    if let Err(e) = ghost_updater_core::validate_process_names(&cfg.close_processes) {
        panic!("close_processes 非法: {e}");
    }
    Router::new()
        .route("/update/probe", get(probe))
        .route("/update/stage", post(stage))
        .route("/update/apply", post(apply))
        .layer(DefaultBodyLimit::max(MAX_PACKAGE_BYTES as usize))
        .with_state(UpdateState { cfg, staging_root, pubkey, fingerprint })
}

/// 独立后台 agent(纯桌面/本地工具场景,design:1 个端口控制面)
pub async fn start_agent(addr: &str, version: &str, auth_token: Option<String>) -> std::io::Result<()> {
    let app = routes(UpdateConfig { auth_token, ..UpdateConfig::new(version) });
    let listener = tokio::net::TcpListener::bind(addr).await?;
    eprintln!("[ghost-updater] agent 监听 {addr}");
    axum::serve(listener, app).await
}

// ---------- probe ----------

async fn probe(State(st): State<UpdateState>) -> Response {
    Json(json!({
        "version": st.cfg.version,
        "started_at": started_at(),
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "pubkey_fingerprint": st.fingerprint,
    }))
    .into_response()
}

// ---------- stage ----------

#[derive(Deserialize)]
struct StageQuery {
    /// 整包(tar)SHA256 hex,必填,防半包/坏包
    sha256: String,
    /// 主可执行文件名,必填,apply 用它重启
    main: String,
}

#[derive(Serialize)]
struct StageAck {
    ok: bool,
    staging: String,
    files: Vec<String>,
}

async fn stage(
    State(st): State<UpdateState>,
    Query(q): Query<StageQuery>,
    headers: HeaderMap,
    body: Body,
) -> Response {
    if let Err(r) = check_auth(&st, &headers) {
        drain_body(body).await;
        return r;
    }
    if q.sha256.len() != 64 || hex::decode(&q.sha256).is_err() {
        drain_body(body).await;
        return err(StatusCode::BAD_REQUEST, "sha256 必须是 64 位 hex");
    }
    if let Err(e) = ghost_updater_core::validate_file_names(std::slice::from_ref(&q.main)) {
        drain_body(body).await;
        return err(StatusCode::BAD_REQUEST, &format!("main 非法: {e}"));
    }

    // 流式落盘:边收边算哈希,中断只会留下垃圾暂存目录,不影响运行中的主进程
    // 必须绝对路径:apply worker 的 cwd 就是暂存目录,相对路径会在 worker 里自我套娃
    let staging = match std::path::absolute(st.staging_root.join(format!(
        "stage-{}-{}",
        unix_secs(),
        STAGE_SEQ.fetch_add(1, Ordering::Relaxed)
    ))) {
        Ok(p) => p,
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, &format!("解析暂存路径失败: {e}")),
    };
    if let Err(e) = std::fs::create_dir_all(&staging) {
        return err(StatusCode::INTERNAL_SERVER_ERROR, &format!("创建暂存目录失败: {e}"));
    }

    let pkg_path = staging.join("package.tar");
    let mut hasher = Sha256::new();
    if let Err(e) = stream_to_file(body, &pkg_path, &mut hasher).await {
        let _ = std::fs::remove_dir_all(&staging);
        return err(StatusCode::BAD_REQUEST, &format!("接收流失败: {e}"));
    }
    let digest = hex::encode(hasher.finalize());
    if !digest.eq_ignore_ascii_case(&q.sha256) {
        let _ = std::fs::remove_dir_all(&staging);
        return err(
            StatusCode::BAD_REQUEST,
            &format!("SHA256 不符(收到 {digest},声明 {}):疑似断传/坏包,暂存已清除", q.sha256),
        );
    }

    // 可选强制验签:对 32 字节摘要签名
    if let Some(vk) = &st.pubkey {
        let ok = headers
            .get("x-signature")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| hex::decode(s).ok())
            .and_then(|b| Signature::from_slice(&b).ok())
            .map(|sig| vk.verify(digest.as_bytes(), &sig).is_ok())
            .unwrap_or(false);
        if !ok {
            let _ = std::fs::remove_dir_all(&staging);
            return err(StatusCode::UNAUTHORIZED, "缺少有效 X-Signature(此节点已启用验签)");
        }
    }

    // 解包 tar(扁平文件集,拒绝目录穿越/非普通文件)
    let files = match unpack_flat(&pkg_path, &staging) {
        Ok(f) => f,
        Err(e) => {
            let _ = std::fs::remove_dir_all(&staging);
            return err(StatusCode::BAD_REQUEST, &format!("解包失败: {e}"));
        }
    };
    let _ = std::fs::remove_file(&pkg_path);
    if files.is_empty() {
        let _ = std::fs::remove_dir_all(&staging);
        return err(StatusCode::BAD_REQUEST, "包内没有文件");
    }
    if !files.iter().any(|f| *f == q.main) {
        let _ = std::fs::remove_dir_all(&staging);
        return err(StatusCode::BAD_REQUEST, &format!("包内缺少主程序 {main}", main = q.main));
    }

    let manifest = StageManifest {
        files: files.clone(),
        sha256: digest.to_ascii_lowercase(),
        main: q.main,
        staged_at: unix_secs(),
    };
    if let Err(e) = std::fs::write(
        staging.join("manifest.json"),
        serde_json::to_string(&manifest).unwrap_or_default(),
    ) {
        let _ = std::fs::remove_dir_all(&staging);
        return err(StatusCode::INTERNAL_SERVER_ERROR, &format!("写 manifest 失败: {e}"));
    }

    Json(StageAck { ok: true, staging: staging_dir_name(&staging), files }).into_response()
}

async fn stream_to_file(body: Body, path: &Path, hasher: &mut Sha256) -> Result<u64, String> {
    use http_body_util::BodyExt;
    let mut f = std::fs::File::create(path).map_err(|e| format!("建包文件失败: {e}"))?;
    let mut total: u64 = 0;
    let mut body = body;
    while let Some(frame) = body.frame().await {
        let bytes = frame.map_err(|e| format!("读 body: {e}"))?.into_data().map_err(|_| "body 帧非数据")?;
        hasher.update(&bytes);
        f.write_all(&bytes).map_err(|e| format!("写暂存: {e}"))?;
        total += bytes.len() as u64;
        if total > MAX_PACKAGE_BYTES {
            return Err(format!("超过单包上限 {MAX_PACKAGE_BYTES} 字节"));
        }
    }
    f.flush().map_err(|e| format!("flush 暂存: {e}"))?;
    Ok(total)
}

/// 把未消费的请求体读完再响应(REQ-3):axum/hyper 对未读 body 的请求发完响应即断连,
/// 客户端若仍在流式上传大包体,看到的是连接重置而非 401/4xx,错误表现随机、E2E 无法断言。
/// 上限 MAX_PACKAGE_BYTES(超限放弃、断连可接受),整体 30s 超时不伺候慢速攻击。
async fn drain_body(body: Body) {
    use http_body_util::BodyExt;
    let _ = tokio::time::timeout(std::time::Duration::from_secs(30), async move {
        let mut body = body;
        let mut total: u64 = 0;
        while let Some(frame) = body.frame().await {
            match frame {
                Ok(f) => {
                    if let Some(d) = f.data_ref() {
                        total += d.len() as u64;
                    }
                    if total > MAX_PACKAGE_BYTES {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    })
    .await;
}

/// 解包 tar 到 dir,只接受扁平普通文件;返回文件名列表。
fn unpack_flat(pkg: &Path, dir: &Path) -> Result<Vec<String>, String> {
    let mut ar = tar::Archive::new(std::fs::File::open(pkg).map_err(|e| format!("开包: {e}"))?);
    ar.set_preserve_permissions(false);
    ar.set_unpack_xattrs(false);
    let mut files = Vec::new();
    for entry in ar.entries().map_err(|e| format!("遍历 tar: {e}"))? {
        let mut e = entry.map_err(|e| format!("读 tar 条目: {e}"))?;
        let name = e.path().map_err(|e| format!("条目路径: {e}"))?.display().to_string();
        ghost_updater_core::validate_file_names(std::slice::from_ref(&name))
            .map_err(|_| format!("包内含路径成分,拒绝: {name}"))?;
        if e.header().entry_type() != tar::EntryType::Regular {
            return Err(format!("包内含非普通文件,拒绝: {name}"));
        }
        let mut out = std::fs::File::create(dir.join(&name)).map_err(|e| format!("建 {name}: {e}"))?;
        std::io::copy(&mut e, &mut out).map_err(|e| format!("解 {name}: {e}"))?;
        if !files.contains(&name) {
            files.push(name);
        }
    }
    Ok(files)
}

// ---------- apply ----------

#[derive(Deserialize, Default)]
struct ApplyReq {
    /// 暂存目录名(stage 返回的 staging;缺省 = 最新一个)
    staging: Option<String>,
    /// 透传给重启进程的参数(worker argv 的 `--` 之后);缺省 = 本进程原始启动参数
    #[serde(default)]
    relaunch_args: Vec<String>,
}

async fn apply(State(st): State<UpdateState>, headers: HeaderMap, body: String) -> Response {
    if let Err(r) = check_auth(&st, &headers) {
        return r;
    }
    let req: ApplyReq = if body.trim().is_empty() {
        ApplyReq::default()
    } else {
        match serde_json::from_str(&body) {
            Ok(r) => r,
            Err(e) => return err(StatusCode::BAD_REQUEST, &format!("请求体非法: {e}")),
        }
    };

    let staging_abs = match resolve_staging(&st.staging_root, &req.staging) {
        Ok(p) => p,
        Err(e) => return err(StatusCode::NOT_FOUND, &e),
    };
    let manifest: StageManifest = match std::fs::read_to_string(staging_abs.join("manifest.json"))
        .map_err(|e| format!("读 manifest 失败: {e}"))
        .and_then(|s| serde_json::from_str(&s).map_err(|e| format!("manifest 解析失败: {e}")))
    {
        Ok(m) => m,
        Err(e) => return err(StatusCode::CONFLICT, &format!("暂存区不可用: {e}(重新 stage)")),
    };
    if !staging_abs.join(&manifest.main).is_file() {
        return err(StatusCode::CONFLICT, &format!("暂存区缺少主程序 {}", manifest.main));
    }
    // 缺省透传本进程原始启动参数(重启后保持原 --addr/--token 等)
    let relaunch_args = if req.relaunch_args.is_empty() {
        std::env::args().skip(1).collect::<Vec<String>>()
    } else {
        req.relaunch_args.clone()
    };
    if let Err(e) = acquire_apply_lock(&staging_abs) {
        return err(StatusCode::CONFLICT, &format!("已有 apply 在进行: {e}"));
    }

    let install = st.cfg.install_dir.clone().unwrap_or_else(default_install_dir);
    let exe = staging_abs.join(&manifest.main);
    let mut argv: Vec<String> = vec![
        APPLY_FLAG.to_string(),
        "--pid".into(),
        std::process::id().to_string(),
        "--install".into(),
        install.display().to_string(),
        "--staging".into(),
        staging_abs.display().to_string(),
        "--from".into(),
        st.cfg.version.clone(),
    ];
    // REQ-1:worker 要等这些关联进程退出后才动文件,名单经 argv 交接
    if !st.cfg.close_processes.is_empty() {
        argv.push("--close".into());
        argv.push(st.cfg.close_processes.join(","));
    }
    if !relaunch_args.is_empty() {
        argv.push("--".into());
        argv.extend(relaunch_args);
    }
    if let Err(e) = ghost_updater_core::start_detached(&exe, &staging_abs, &argv, Some(&staging_abs.join("worker.log"))) {
        ghost_updater_core::release_lock(&staging_abs);
        return err(StatusCode::INTERNAL_SERVER_ERROR, &format!("拉起 apply worker 失败: {e}"));
    }

    // ack 已代表受理;留出响应回程时间后:优雅关闭 close_processes → 自退,
    // 接力棒交给暂存区里的新 exe(worker 会等 --pid 即本进程退出)
    let install_for_log = install.clone();
    let close_list = st.cfg.close_processes.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(800)).await;
        if !close_list.is_empty() {
            ghost_updater_core::append_apply_log(
                &install_for_log,
                &format!("apply 受理,优雅关闭关联进程: {}", close_list.join(", ")),
            );
            let closed = tokio::task::spawn_blocking(move || {
                ghost_updater_core::close_processes_graceful(&close_list, std::time::Duration::from_secs(5))
            })
            .await;
            match closed {
                Ok(Ok(())) => ghost_updater_core::append_apply_log(&install_for_log, "关联进程已全部关闭,代理自退"),
                Ok(Err(e)) => ghost_updater_core::append_apply_log(
                    &install_for_log,
                    &format!("关联进程关闭失败: {e}(代理自退,worker 会兜底等待/强杀)"),
                ),
                Err(e) => ghost_updater_core::append_apply_log(&install_for_log, &format!("关闭任务异常: {e}")),
            }
        }
        std::process::exit(0);
    });
    Json(json!({ "ok": true, "staging": staging_dir_name(&staging_abs), "pid": std::process::id() }))
        .into_response()
}

// ---------- 公共 ----------

fn check_auth(st: &UpdateState, headers: &HeaderMap) -> Result<(), Response> {
    if let Some(token) = &st.cfg.auth_token {
        let ok = headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            == Some(&format!("Bearer {token}"));
        if !ok {
            return Err(err(StatusCode::UNAUTHORIZED, "Bearer token 缺失或不符"));
        }
        return Ok(());
    }
    if st.pubkey.is_some() {
        // 验签模式:stage 在收完整包后强制验签;apply 只能触发已验签内容
        return Ok(());
    }
    Err(err(StatusCode::SERVICE_UNAVAILABLE, "本节点未配置任何鉴权(token/公钥),更新接口已禁用"))
}

/// 最近一次 stage 的暂存目录(名字里带时间戳,字典序即时间序;返回绝对路径)
fn resolve_staging(root: &Path, name: &Option<String>) -> Result<PathBuf, String> {
    match name {
        Some(n) => {
            if n.contains('/') || n.contains('\\') || n.contains("..") {
                return Err("staging 名非法".into());
            }
            let p = root.join(n);
            if !p.is_dir() {
                return Err(format!("暂存目录不存在: {n}"));
            }
            std::path::absolute(p).map_err(|e| format!("解析暂存路径失败: {e}"))
        }
        None => {
            let mut dirs: Vec<PathBuf> = std::fs::read_dir(root)
                .map_err(|e| format!("暂存根目录不存在: {e}"))?
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.is_dir() && p.join("manifest.json").is_file())
                .collect();
            dirs.sort();
            let latest = dirs.pop().ok_or_else(|| "没有任何已暂存的更新".to_string())?;
            std::path::absolute(latest).map_err(|e| format!("解析暂存路径失败: {e}"))
        }
    }
}

fn staging_dir_name(p: &Path) -> String {
    p.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default()
}

fn default_install_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."))
}

fn decode_pubkey(hex_str: &str) -> Result<[u8; 32], String> {
    let b = hex::decode(hex_str.trim()).map_err(|e| format!("hex: {e}"))?;
    let n = b.len();
    b.try_into().map_err(|_| format!("长度 {n} ≠ 32 字节"))
}

fn err(code: StatusCode, msg: &str) -> Response {
    (code, Json(json!({ "ok": false, "error": msg }))).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use tower::ServiceExt;

    /// 每个测试独立暂存根,避免并发测试抢 cwd / 同一目录
    fn app_with(cfg: UpdateConfig, tag: &str) -> (Router, PathBuf) {
        let root = unique_cwd(tag).join("stage-root");
        std::fs::create_dir_all(&root).unwrap();
        let app = routes(UpdateConfig { staging_root: Some(root.clone()), ..cfg });
        (app, root)
    }

    async fn probe_version(app: Router) -> (StatusCode, serde_json::Value) {
        let r = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/update/probe")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let code = r.status();
        let bytes = http_body_util::BodyExt::collect(r.into_body()).await.unwrap().to_bytes();
        (code, serde_json::from_slice(&bytes).unwrap())
    }

    fn test_tar(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut b = tar::Builder::new(Vec::new());
        for (name, data) in files {
            let mut h = tar::Header::new_gnu();
            h.set_size(data.len() as u64);
            h.set_mode(0o644);
            h.set_cksum();
            b.append_data(&mut h, name, *data).unwrap();
        }
        b.into_inner().unwrap()
    }

    /// 手工拼 512 字节 tar 头 + 数据(Builder 会拒收 `..` 等恶意名,恶意包测试用)
    fn raw_tar_entry(name: &str, data: &[u8], typeflag: u8) -> Vec<u8> {
        let mut blk = [0u8; 512];
        blk[..name.len()].copy_from_slice(name.as_bytes());
        blk[100..108].copy_from_slice(b"0000644\0");
        blk[108..116].copy_from_slice(b"0000000\0");
        blk[116..124].copy_from_slice(b"0000000\0");
        let size = format!("{:011o}\0", data.len());
        blk[124..124 + size.len()].copy_from_slice(size.as_bytes());
        blk[156] = typeflag;
        blk[257..263].copy_from_slice(b"ustar\0");
        blk[263..265].copy_from_slice(b"00");
        for b in &mut blk[148..156] {
            *b = b' ';
        }
        let sum: u32 = blk.iter().map(|b| *b as u32).sum();
        let chk = format!("{:06o}\0 ", sum);
        blk[148..156].copy_from_slice(chk.as_bytes());
        let mut out = blk.to_vec();
        out.extend_from_slice(data);
        let pad = (512 - data.len() % 512) % 512;
        out.extend(std::iter::repeat_n(0u8, pad));
        out
    }

    fn sha(data: &[u8]) -> String {
        hex::encode(Sha256::digest(data))
    }

    async fn stage_req(
        app: Router,
        uri: String,
        headers: HeaderMap,
        body: Vec<u8>,
    ) -> (StatusCode, serde_json::Value) {
        let mut builder = axum::http::Request::builder().method("POST").uri(uri);
        for (k, v) in headers.iter() {
            builder = builder.header(k, v);
        }
        let r = app.oneshot(builder.body(Body::from(body)).unwrap()).await.unwrap();
        let code = r.status();
        let bytes = http_body_util::BodyExt::collect(r.into_body()).await.unwrap().to_bytes();
        (code, serde_json::from_slice(&bytes).unwrap_or(json!(null)))
    }

    fn auth(value: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(axum::http::header::AUTHORIZATION, value.parse().unwrap());
        h
    }

    #[tokio::test]
    async fn probe_reports_version_and_platform() {
        let (app, _root) = app_with(UpdateConfig::new("9.9.9-test"), "probe");
        let (code, body) = probe_version(app).await;
        assert_eq!(code, StatusCode::OK);
        assert_eq!(body["version"], "9.9.9-test");
        assert_eq!(body["os"], std::env::consts::OS);
        assert!(body["started_at"].as_u64().unwrap() > 0);
    }

    #[tokio::test]
    async fn stage_without_auth_config_is_503() {
        let (app, _root) = app_with(UpdateConfig::new("1.0.0"), "noauth");
        let (code, _) = stage_req(app, "/update/stage?sha256=x&main=a.exe".into(), HeaderMap::new(), vec![]).await;
        assert_eq!(code, StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn stage_wrong_token_is_401() {
        let (app, _root) = app_with(UpdateConfig { auth_token: Some("secret".into()), ..UpdateConfig::new("1.0.0") }, "badtok");
        let (code, _) = stage_req(app, "/update/stage?sha256=x&main=a.exe".into(), auth("Bearer wrong"), vec![]).await;
        assert_eq!(code, StatusCode::UNAUTHORIZED);
    }

    /// REQ-3 验收:拒绝路径必须把请求体 drain 完再回 401,否则 hyper 对未消费 body
    /// 的请求发完响应即断连,仍在流式上传的客户端只会看到连接重置。真实 TCP 全双工验证
    /// (内存 oneshot 测不出连接行为):客户端边写 3MiB 边等,无 drain 时写/读必炸一个。
    #[tokio::test]
    async fn stage_wrong_token_big_body_still_401_over_tcp() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let (app, _root) = app_with(UpdateConfig { auth_token: Some("t".into()), ..UpdateConfig::new("1.0.0") }, "drain401");
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
        let head = "POST /update/stage?sha256=ab&main=a.exe HTTP/1.1\r\n\
                    Host: e2e\r\nAuthorization: Bearer wrong\r\nContent-Length: 3145728\r\n\
                    Connection: close\r\n\r\n";
        s.write_all(head.as_bytes()).await.unwrap();
        let chunk = vec![0u8; 65536];
        for _ in 0..48 {
            s.write_all(&chunk).await.unwrap();
        }
        let mut buf = Vec::new();
        let _ = s.read_to_end(&mut buf).await;
        let head_str = String::from_utf8_lossy(&buf[..buf.len().min(2048)]).to_string();
        assert!(head_str.starts_with("HTTP/1.1 401"), "应读到 401,实际: {head_str}");
    }

    #[tokio::test]
    async fn stage_sha_mismatch_rejects_and_cleans() {
        let (app, root) = app_with(UpdateConfig { auth_token: Some("t".into()), ..UpdateConfig::new("1.0.0") }, "shabad");
        let tar = test_tar(&[("app.exe", b"hello")]);
        let bad_sha = sha(b"other");
        let (code, body) = stage_req(app, format!("/update/stage?sha256={bad_sha}&main=app.exe"), auth("Bearer t"), tar).await;
        assert_eq!(code, StatusCode::BAD_REQUEST);
        assert!(body["error"].as_str().unwrap().contains("SHA256 不符"));
        // 坏包暂存已清除
        let left = std::fs::read_dir(&root).unwrap().filter_map(|e| e.ok()).count();
        assert_eq!(left, 0);
    }

    #[tokio::test]
    async fn stage_happy_path_writes_manifest() {
        let (app, root) = app_with(UpdateConfig { auth_token: Some("t".into()), ..UpdateConfig::new("1.0.0") }, "happy");
        let tar = test_tar(&[("app.exe", b"new-bin"), ("notes.txt", b"hi")]);
        let good = sha(&tar);
        let (code, body) = stage_req(app, format!("/update/stage?sha256={good}&main=app.exe"), auth("Bearer t"), tar).await;
        assert_eq!(code, StatusCode::OK, "body={body}");
        assert_eq!(body["ok"], true);
        let name = body["staging"].as_str().unwrap().to_string();
        let dir = root.join(&name);
        assert_eq!(std::fs::read(dir.join("app.exe")).unwrap(), b"new-bin");
        let m: StageManifest =
            serde_json::from_str(&std::fs::read_to_string(dir.join("manifest.json")).unwrap()).unwrap();
        assert_eq!(m.main, "app.exe");
        assert_eq!(m.files.len(), 2);
        assert!(!dir.join("package.tar").exists()); // 包文件已清理
    }

    #[tokio::test]
    async fn stage_rejects_traversal_and_non_regular_entries() {
        let (app, root) = app_with(UpdateConfig { auth_token: Some("t".into()), ..UpdateConfig::new("1.0.0") }, "evil");

        let evil = raw_tar_entry("..\\evil.exe", b"x", b'0');
        let s = sha(&evil);
        let (code, _) = stage_req(app.clone(), format!("/update/stage?sha256={s}&main=evil.exe"), auth("Bearer t"), evil).await;
        assert_eq!(code, StatusCode::BAD_REQUEST);

        let link = raw_tar_entry("link.exe", b"", b'2'); // symlink
        let s = sha(&link);
        let (code, _) = stage_req(app.clone(), format!("/update/stage?sha256={s}&main=link.exe"), auth("Bearer t"), link).await;
        assert_eq!(code, StatusCode::BAD_REQUEST);

        // 两个恶意包都不留暂存
        let left = std::fs::read_dir(&root).unwrap().filter_map(|e| e.ok()).count();
        assert_eq!(left, 0);
    }

    #[tokio::test]
    async fn apply_without_staging_is_404() {
        let (app, _root) = app_with(UpdateConfig { auth_token: Some("t".into()), ..UpdateConfig::new("1.0.0") }, "apply404");
        let (code, _) = stage_req(app, "/update/apply".into(), auth("Bearer t"), vec![]).await;
        assert_eq!(code, StatusCode::NOT_FOUND);
    }

    fn unique_cwd(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("ghost-net-{tag}-{}-{}", std::process::id(), unix_secs()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }
}
