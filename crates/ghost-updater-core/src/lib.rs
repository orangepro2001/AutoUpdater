//! ghost-updater-core:远程自替换核心(apply 侧)。
//!
//! 宿主 main 顶部一行 `ghost_updater_core::guard_apply_update();` 即完成接入:
//! 若本进程是被 `/update/apply` 从暂存区拉起的 apply worker(参数含
//! `--internal-apply-update`),它会在本函数内完成整个换版流程然后 exit;
//! 否则立即返回,走正常业务逻辑。
//!
//! apply worker 流程(全程阻塞 std,必须在 tokio runtime 建立前执行):
//!   1. 解析 --pid/--install/--staging(/--from/--close),读暂存区 manifest.json
//!   2. 等旧进程(pid)退出:轮询 process_alive,超时 taskkill /F 兜底
//!   2.5 等 --close 列出的关联进程退出(多进程宿主:agent 侧已优雅关闭,这里兜底等待/强杀)
//!   3. 备份安装目录将被覆盖的文件 → <install>/.update_backup/backup-<ts>/
//!   4. 用暂存文件覆盖安装目录(整体预校验后再动,保留未列入清单的配置文件;
//!      覆盖段中途失败当场按备份逐文件还原,Err 时 install 已回到旧状态)
//!   5. 分离拉起安装目录的新 exe(DETACHED|CREATE_NO_WINDOW,stdio 直弃,
//!      严禁管道捕获 —— 孙进程继承管道写句柄会导致等 EOF 永久卡死),1.5s
//!      存活判定为健康
//!   6. 失败回滚:还原备份、删本次新增文件、重拉旧 exe;apply.log/result.json 记录全程;
//!      所有失败路径(含备份/覆盖段)统一由 run_apply 收口释放 .apply-lock

use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// 本进程启动时刻(Unix 秒)。懒初始化一次,此后不变;
/// 推送端凭 started_at 变化确认目标已重启(网络空窗判定是竞态,不可靠)。
static STARTED_AT: AtomicU64 = AtomicU64::new(0);

pub fn started_at() -> u64 {
    let mut v = STARTED_AT.load(Ordering::Relaxed);
    if v == 0 {
        v = unix_secs();
        STARTED_AT.store(v, Ordering::Relaxed);
    }
    v
}

/// 更新数据目录名(位于安装目录下;备份/日志/结果都在这)
pub const UPDATE_DIR_NAME: &str = ".update_backup";
/// 锁文件名(写入暂存目录,create_new 原子防并发 apply)
pub const APPLY_LOCK_NAME: &str = ".apply-lock";
/// apply worker 的标记参数
pub const APPLY_FLAG: &str = "--internal-apply-update";

/// 暂存区清单(stage 侧由 ghost-updater-net 写入)
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct StageManifest {
    /// 扁平文件名(纯文件名,无路径分隔符)
    pub files: Vec<String>,
    /// 整包(tar)SHA256 hex
    pub sha256: String,
    /// 主可执行文件名(apply 后用它重启;必须 ∈ files)
    pub main: String,
    pub staged_at: u64,
}

/// 等待 pid 退出:轮询 process_alive,超时后 Windows 用 taskkill /F 兜底再等 1s。
/// 返回 true = 已确认退出。非 Windows 无 taskkill,超时即放弃(返回 false,放弃更新)。
pub fn wait_pid_exit(pid: u32, timeout: Duration) -> bool {
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        if !process_alive(pid) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("taskkill")
            .args(["/F", "/PID", &pid.to_string()])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
        std::thread::sleep(Duration::from_secs(1));
        !process_alive(pid)
    }
    #[cfg(not(windows))]
    {
        let _ = pid;
        false
    }
}

/// 进程是否存活。Windows: OpenProcess + GetExitCodeProcess == STILL_ACTIVE;
/// Unix: kill(pid, 0),EPERM 也算存活(存在但无权限发信号)。
pub fn process_alive(pid: u32) -> bool {
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE, STILL_ACTIVE};
        use windows_sys::Win32::System::Threading::{
            GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
        };
        // SAFETY:句柄 API,只查询不写入;h 用后即关
        unsafe {
            let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if h.is_null() || h == INVALID_HANDLE_VALUE {
                return false;
            }
            let mut code: u32 = 0;
            let ok = GetExitCodeProcess(h, &mut code);
            CloseHandle(h);
            ok != 0 && code == STILL_ACTIVE as u32
        }
    }
    #[cfg(unix)]
    {
        // SAFETY: kill(2) 是纯信号系统调用,只读 pid
        let r = unsafe { libc::kill(pid as i32, 0) };
        r == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }
}

/// 进程映像名白名单:只允许 ASCII 字母数字与 `. _ -`,杜绝 taskkill/tasklist 参数注入。
pub fn validate_process_names(names: &[String]) -> Result<(), String> {
    for n in names {
        if n.is_empty()
            || !n
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-')
        {
            return Err(format!("进程名只允许字母数字与 ._-: {n:?}"));
        }
    }
    Ok(())
}

/// 进程映像名是否在运行。Windows: tasklist CSV 首列精确比对(GBK 输出里文件名段
/// 是 ASCII,utf8_lossy 不损伤);非 Windows 恒 false(close/wait 均为 no-op)。
pub fn process_running_by_name(name: &str) -> bool {
    #[cfg(windows)]
    {
        let Ok(out) = std::process::Command::new("tasklist")
            .args(["/FI", &format!("IMAGENAME eq {name}"), "/FO", "CSV", "/NH"])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .output()
        else {
            return false;
        };
        String::from_utf8_lossy(&out.stdout).lines().any(|line| {
            let first = line.split(',').next().unwrap_or("").trim_matches('"');
            first.eq_ignore_ascii_case(name)
        })
    }
    #[cfg(not(windows))]
    {
        let _ = name;
        false
    }
}

fn taskkill_image(name: &str, force: bool) {
    let mut args = vec!["/IM".to_string(), name.to_string()];
    if force {
        args.push("/F".to_string());
    }
    let _ = std::process::Command::new("taskkill")
        .args(&args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
}

/// 优雅关闭一批进程(REQ-1):Windows 先 `taskkill /IM`(无 /F = WM_CLOSE),
/// grace 内轮询,残留再 `/F` 兜底。Err = 仍有进程关不掉(需人工介入)。
/// 非 Windows no-op(REQ:可先留空实现)。
pub fn close_processes_graceful(names: &[String], grace: Duration) -> Result<(), String> {
    validate_process_names(names)?;
    #[cfg(windows)]
    {
        let mut stubborn = Vec::new();
        for n in names {
            taskkill_image(n, false);
            let deadline = std::time::Instant::now() + grace;
            while process_running_by_name(n) && std::time::Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(300));
            }
            if process_running_by_name(n) {
                taskkill_image(n, true);
                std::thread::sleep(Duration::from_millis(500));
                if process_running_by_name(n) {
                    stubborn.push(n.clone());
                }
            }
        }
        if stubborn.is_empty() {
            Ok(())
        } else {
            Err(format!("以下进程未能关闭: {}", stubborn.join(", ")))
        }
    }
    #[cfg(not(windows))]
    {
        let _ = (names, grace);
        Ok(())
    }
}

/// 等待一批进程映像名全部退出:轮询,超时逐个 taskkill /F 兜底(镜像 wait_pid_exit 语义)。
/// 名单非法视为配置错误,返回 false(放弃更新)。非 Windows 恒 true(与 no-op close 配套)。
pub fn wait_processes_exit(names: &[String], timeout: Duration) -> bool {
    if validate_process_names(names).is_err() {
        return false;
    }
    #[cfg(windows)]
    {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            if names.iter().all(|n| !process_running_by_name(n)) {
                return true;
            }
            if std::time::Instant::now() >= deadline {
                break;
            }
            std::thread::sleep(Duration::from_millis(300));
        }
        for n in names {
            taskkill_image(n, true);
        }
        std::thread::sleep(Duration::from_secs(1));
        names.iter().all(|n| !process_running_by_name(n))
    }
    #[cfg(not(windows))]
    {
        let _ = (names, timeout);
        true
    }
}

/// main 顶部拦截:检测 `--internal-apply-update`,是 apply worker 就地完成换版并 exit,
/// 永不返回;否则直接 return,宿主继续正常启动。
pub fn guard_apply_update() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some((worker_args, relaunch_args)) = split_at_flag(&args) else { return };
    let code = match run_apply(&worker_args, &relaunch_args) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("ghost-updater: apply 失败: {e}");
            1
        }
    };
    std::process::exit(code);
}

/// argv 按 APPLY_FLAG 切成 (worker 参数, `--` 后的重启透传参数)。
pub fn split_at_flag(args: &[String]) -> Option<(Vec<String>, Vec<String>)> {
    let pos = args.iter().position(|a| a == APPLY_FLAG)?;
    let rest = &args[pos + 1..];
    match rest.iter().position(|a| a == "--") {
        Some(sep) => Some((rest[..sep].to_vec(), rest[sep + 1..].to_vec())),
        None => Some((rest.to_vec(), Vec::new())),
    }
}

struct ApplyArgs {
    pid: Option<u32>,
    install: PathBuf,
    staging: PathBuf,
    from_version: String,
    /// 需一并等待退出的关联进程映像名(net 侧 --close 传入;agent 已做过优雅关闭)
    close: Vec<String>,
}

fn parse_apply_args(argv: &[String]) -> Result<ApplyArgs, String> {
    let mut pid = None;
    let mut install = None;
    let mut staging = None;
    let mut from = String::new();
    let mut close = Vec::new();
    let mut i = 0;
    while i < argv.len() {
        let val = |i: usize| argv.get(i + 1).ok_or_else(|| format!("{} 缺值", argv[i]));
        match argv[i].as_str() {
            "--pid" => pid = Some(val(i)?.parse().map_err(|_| "--pid 非法")?),
            "--install" => install = Some(PathBuf::from(val(i)?)),
            "--staging" => staging = Some(PathBuf::from(val(i)?)),
            "--from" => from = val(i)?.clone(),
            "--close" => {
                close = val(i)?
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
                validate_process_names(&close)?;
            }
            other => return Err(format!("未知参数: {other}")),
        }
        i += 2;
    }
    Ok(ApplyArgs {
        pid,
        install: install.ok_or("缺少 --install")?,
        staging: staging.ok_or("缺少 --staging")?,
        from_version: from,
        close,
    })
}

fn run_apply(argv: &[String], relaunch_args: &[String]) -> Result<(), String> {
    let a = parse_apply_args(argv)?;
    let r = run_apply_inner(&a, relaunch_args);
    // 锁收口:成功/任何失败路径统一在这里释放,失败不再泄漏 .apply-lock(REQ-2)
    release_lock(&a.staging);
    r
}

fn run_apply_inner(a: &ApplyArgs, relaunch_args: &[String]) -> Result<(), String> {
    let update_dir = a.install.join(UPDATE_DIR_NAME);
    std::fs::create_dir_all(&update_dir)
        .map_err(|e| format!("创建 {} 失败: {e}", update_dir.display()))?;
    let log_path = update_dir.join("apply.log");

    let manifest: StageManifest = serde_json::from_str(
        &std::fs::read_to_string(a.staging.join("manifest.json"))
            .map_err(|e| format!("读 manifest.json 失败: {e}"))?,
    )
    .map_err(|e| format!("manifest.json 解析失败: {e}"))?;
    validate_file_names(&manifest.files)?;
    if !manifest.files.iter().any(|f| *f == manifest.main) {
        return Err(format!("main exe({}) 不在文件集内", manifest.main));
    }

    log_line(
        &log_path,
        &format!(
            "apply 开始: pid={:?} install={} staging={} files={} from={}",
            a.pid,
            a.install.display(),
            a.staging.display(),
            manifest.files.len(),
            a.from_version,
        ),
    );

    // 1. 等旧进程退出(服务端 ack 后 ~0.8s 自退;这里兜底强杀)
    if let Some(pid) = a.pid {
        if !wait_pid_exit(pid, Duration::from_secs(15)) {
            let msg = format!("旧进程(pid={pid}) 15s 内未退出,放弃更新(文件未动)");
            log_line(&log_path, &msg);
            write_result(&update_dir, false, &a.from_version, &manifest.files, &msg);
            return Err(msg);
        }
    }
    std::thread::sleep(Duration::from_millis(300)); // 句柄释放缓冲

    // 1.5 等关联进程退出(REQ-1 多进程宿主:agent 侧已优雅关闭,这里兜底等待 + taskkill /F)
    if !a.close.is_empty() {
        if !wait_processes_exit(&a.close, Duration::from_secs(10)) {
            let msg = format!(
                "关联进程({}) 10s 内未退出,放弃更新(文件未动)",
                a.close.join(",")
            );
            log_line(&log_path, &msg);
            write_result(&update_dir, false, &a.from_version, &manifest.files, &msg);
            return Err(msg);
        }
    }

    // 2+3. 备份 + 覆盖(分段:备份段失败文件未动;覆盖段失败由 overwrite_files 当场还原,
    //      只还原已覆盖的文件 —— 不碰未覆盖的旧文件,避免 remove 旧文件被外部句柄锁卡死)
    let backup_dir = update_dir.join(format!("backup-{}", unix_secs()));
    if let Err(e) = ensure_staged(&a.staging, &manifest.files)
        .and_then(|_| backup_files(&a.install, &manifest.files, &backup_dir))
    {
        let msg = format!("暂存/备份失败: {e}(文件未动)");
        log_line(&log_path, &msg);
        write_result(&update_dir, false, &a.from_version, &manifest.files, &msg);
        return Err(msg);
    }
    if let Err(e) = overwrite_files(&a.install, &a.staging, &manifest.files, &backup_dir) {
        log_line(&log_path, &format!("文件替换失败: {e};重拉旧版恢复服务"));
        if !manifest.main.is_empty() {
            let _ = start_detached(&a.install.join(&manifest.main), &a.install, &[], None);
        }
        let msg = format!("文件替换失败: {e}");
        write_result(&update_dir, false, &a.from_version, &manifest.files, &msg);
        return Err(msg);
    }
    log_line(
        &log_path,
        &format!(
            "文件替换完成({} 个),备份: {}",
            manifest.files.len(),
            backup_dir.display()
        ),
    );

    // 4. 用安装目录的新 exe 重启(绝不能用暂存副本 —— 暂存区随时可能被清理)
    let new_exe = a.install.join(&manifest.main);
    let started = start_detached(&new_exe, &a.install, relaunch_args, None);
    match started {
        Ok(mut child) => {
            // 健康检查:1.5s 后仍存活才算成功(秒退 = 启动失败 → 回滚)
            std::thread::sleep(Duration::from_millis(1500));
            if child_still_alive(&mut child) {
                let _ = std::fs::remove_dir_all(&backup_dir); // 成功清备份
                log_line(&log_path, &format!("更新成功,新进程 pid={} 存活", child.id()));
                write_result(&update_dir, true, &a.from_version, &manifest.files, "");
                Ok(())
            } else {
                let msg = "新进程 1.5s 内退出(健康检查失败)".to_string();
                log_line(&log_path, &format!("{msg};回滚备份并重试启动旧版"));
                rollback(&a.install, &backup_dir, &manifest.files, &manifest.main, &log_path);
                write_result(&update_dir, false, &a.from_version, &manifest.files, &msg);
                Err(msg)
            }
        }
        Err(e) => {
            log_line(&log_path, &format!("拉起新版失败: {e};回滚备份并重试启动旧版"));
            rollback(&a.install, &backup_dir, &manifest.files, &manifest.main, &log_path);
            let msg = format!("拉起新版失败: {e}(已回滚)");
            write_result(&update_dir, false, &a.from_version, &manifest.files, &msg);
            Err(msg)
        }
    }
}

fn rollback(install: &Path, backup_dir: &Path, files: &[String], main: &str, log_path: &Path) {
    match restore_backup(install, backup_dir, files) {
        Ok(()) => log_line(log_path, "回滚完成"),
        Err(e) => log_line(
            log_path,
            &format!("回滚失败(需人工介入,备份在 {}): {e}", backup_dir.display()),
        ),
    }
    if !main.is_empty() {
        let _ = start_detached(&install.join(main), install, &[], None);
    }
}

/// 暂存区预检:文件名扁平 + 源文件齐全。失败 = 安装目录保证未动。
fn ensure_staged(staging: &Path, files: &[String]) -> Result<(), String> {
    validate_file_names(files)?;
    for f in files {
        if !staging.join(f).is_file() {
            return Err(format!("暂存区缺少文件: {}", staging.join(f).display()));
        }
    }
    Ok(())
}

/// 备份安装目录现存文件(不覆盖;失败时 install 未动过)。
pub fn backup_files(install: &Path, files: &[String], backup_dir: &Path) -> Result<(), String> {
    validate_file_names(files)?;
    std::fs::create_dir_all(backup_dir).map_err(|e| format!("创建备份目录失败: {e}"))?;
    for f in files {
        let dst = install.join(f);
        if dst.is_file() {
            std::fs::copy(&dst, backup_dir.join(f)).map_err(|e| format!("备份 {f} 失败: {e}"))?;
        }
    }
    Ok(())
}

/// 用暂存文件覆盖安装目录(调用前必须已 backup_files)。
/// 覆盖段中途失败:已覆盖的文件当场按备份逐个还原、本次新增(备份里没有的)删除,
/// 返回 Err 时 install 已回到旧状态 —— 只动确实覆盖过的文件,未覆盖的旧文件
/// (可能仍被外部句柄锁着)绝不 remove,所以还原不会被锁卡死。
pub fn overwrite_files(
    install: &Path,
    staging: &Path,
    files: &[String],
    backup_dir: &Path,
) -> Result<(), String> {
    validate_file_names(files)?;
    let mut done: Vec<&String> = Vec::new();
    for f in files {
        match std::fs::copy(staging.join(f), install.join(f)) {
            Ok(_) => done.push(f),
            Err(e) => {
                let mut bad: Vec<String> = Vec::new();
                for f2 in done {
                    let bak = backup_dir.join(f2);
                    let r = if bak.is_file() {
                        std::fs::copy(&bak, install.join(f2)).map(|_| ())
                    } else {
                        std::fs::remove_file(install.join(f2)) // 本次新增的,删掉
                    };
                    if let Err(e2) = r {
                        bad.push(format!("还原 {f2}: {e2}"));
                    }
                }
                return Err(if bad.is_empty() {
                    format!("覆盖 {f} 失败: {e}(已回滚到旧状态)")
                } else {
                    format!("覆盖 {f} 失败: {e}(回滚失败,需人工介入: {})", bad.join("; "))
                });
            }
        }
    }
    Ok(())
}

/// 备份 + 覆盖:先整体校验暂存源齐全且文件名扁平,再备份安装目录现存文件,最后覆盖。
/// 返回后 install = 暂存内容;未列入 files 的既有文件(如 config.json)不动。
/// 覆盖段失败的还原语义见 overwrite_files(Err 时 install 已回旧状态)。
pub fn swap_files(install: &Path, staging: &Path, files: &[String], backup_dir: &Path) -> Result<(), String> {
    ensure_staged(staging, files)?;
    backup_files(install, files, backup_dir)?;
    overwrite_files(install, staging, files, backup_dir)
}

/// 回滚:备份里有的恢复原样;备份里没有的(本次新增)删除。
pub fn restore_backup(install: &Path, backup_dir: &Path, files: &[String]) -> Result<(), String> {
    validate_file_names(files)?;
    for f in files {
        let dst = install.join(f);
        let src = backup_dir.join(f);
        if src.is_file() {
            if dst.is_file() {
                std::fs::remove_file(&dst).map_err(|e| format!("删新文件 {f} 失败: {e}"))?;
            }
            std::fs::copy(&src, &dst).map_err(|e| format!("还原 {f} 失败: {e}"))?;
        } else if dst.is_file() {
            std::fs::remove_file(&dst).map_err(|e| format!("删新增 {f} 失败: {e}"))?;
        }
    }
    Ok(())
}

/// 文件集安全校验:纯文件名,拒绝路径分隔符与 ..(文件必须直接落在安装目录)
pub fn validate_file_names(files: &[String]) -> Result<(), String> {
    for f in files {
        if f.is_empty()
            || f.contains('/')
            || f.contains('\\')
            || f.contains("..")
            || f.contains(':')
            || Path::new(f).is_absolute()
        {
            return Err(format!("文件名不允许包含路径成分: {f}"));
        }
    }
    Ok(())
}

/// 分离拉起 exe(cwd=workdir):DETACHED_PROCESS | CREATE_NO_WINDOW。
/// log 为 Some 时 stdout/stderr 追加到该文件(apply worker 无控制台,崩溃线索全靠它),
/// 否则 stdio 全弃。绝不能用 output()/status() 等管道捕获 —— 若新进程又派生
/// 继承句柄的孙进程,父进程永远等不到 EOF,apply 卡死。
pub fn start_detached(
    exe: &Path,
    workdir: &Path,
    args: &[String],
    log: Option<&Path>,
) -> Result<std::process::Child, String> {
    let mut cmd = std::process::Command::new(exe);
    cmd.args(args).current_dir(workdir).stdin(std::process::Stdio::null());
    match log {
        Some(p) => {
            if let Some(parent) = p.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let f = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(p)
                .map_err(|e| format!("开 worker 日志 {} 失败: {e}", p.display()))?;
            cmd.stdout(std::process::Stdio::from(f.try_clone().map_err(|e| e.to_string())?))
                .stderr(std::process::Stdio::from(f));
        }
        None => {
            cmd.stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // DETACHED_PROCESS(0x8) | CREATE_NO_WINDOW(0x0800_0000)
        cmd.creation_flags(0x0000_0008 | 0x0800_0000);
    }
    cmd.spawn().map_err(|e| format!("spawn {} 失败: {e}", exe.display()))
}

fn child_still_alive(child: &mut std::process::Child) -> bool {
    matches!(child.try_wait(), Ok(None))
}

/// apply 并发锁:create_new 原子建 `.apply-lock`;已存在 = 有 apply 在进行。
pub fn acquire_apply_lock(staging: &Path) -> Result<(), String> {
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(staging.join(APPLY_LOCK_NAME))
        .map(|_| ())
        .map_err(|e| format!("暂存区已被锁定(或锁文件创建失败): {e}"))
}

pub fn release_lock(staging: &Path) {
    let _ = std::fs::remove_file(staging.join(APPLY_LOCK_NAME));
}

fn log_line(path: &Path, msg: &str) {
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "[{}] {}", unix_secs(), msg);
    }
}

/// 宿主侧(agent)向 apply.log 追记一行(如 close_processes 关闭动作),与 worker 日志同文件。
pub fn append_apply_log(install: &Path, msg: &str) {
    let dir = install.join(UPDATE_DIR_NAME);
    let _ = std::fs::create_dir_all(&dir);
    log_line(&dir.join("apply.log"), msg);
}

fn write_result(update_dir: &Path, ok: bool, from: &str, files: &[String], error: &str) {
    let body = serde_json::json!({
        "ok": ok,
        "from": from,
        "time": unix_secs(),
        "files": files,
        "error": error,
    });
    let _ = std::fs::write(update_dir.join("result.json"), body.to_string());
}

pub fn unix_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir()
            .join(format!("ghost-core-{}-{tag}-{}", std::process::id(), unix_secs()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn put(dir: &Path, name: &str, content: &[u8]) {
        std::fs::write(dir.join(name), content).unwrap();
    }

    fn get(dir: &Path, name: &str) -> Vec<u8> {
        std::fs::read(dir.join(name)).unwrap()
    }

    #[test]
    fn swap_replaces_and_keeps_backup() {
        let install = temp_dir("swap-install");
        let staging = temp_dir("swap-staging");
        let backup = temp_dir("swap-backup");
        put(&install, "app.exe", b"old");
        put(&install, "app.dll", b"old-dll");
        put(&install, "config.json", b"keep-me");
        put(&staging, "app.exe", b"new");
        put(&staging, "app.dll", b"new-dll");
        put(&staging, "app.h", b"new-header");
        let files = ["app.exe".to_string(), "app.dll".to_string(), "app.h".to_string()];

        swap_files(&install, &staging, &files, &backup).unwrap();

        assert_eq!(get(&install, "app.exe"), b"new");
        assert_eq!(get(&install, "app.dll"), b"new-dll");
        assert_eq!(get(&install, "app.h"), b"new-header");
        assert_eq!(get(&install, "config.json"), b"keep-me"); // 配置保留
        assert_eq!(get(&backup, "app.exe"), b"old");
        assert_eq!(get(&backup, "app.dll"), b"old-dll");
        assert!(!backup.join("app.h").exists()); // 备份只含原有文件
    }

    #[test]
    fn swap_fails_cleanly_when_staging_incomplete() {
        let install = temp_dir("swapf-install");
        let staging = temp_dir("swapf-staging");
        let backup = temp_dir("swapf-backup");
        put(&install, "app.exe", b"old");
        put(&staging, "app.exe", b"new");
        let files = ["app.exe".to_string(), "app.dll".to_string()];
        // app.dll 缺失 → 整体拒绝,安装目录不动
        assert!(swap_files(&install, &staging, &files, &backup).is_err());
        assert_eq!(get(&install, "app.exe"), b"old");
        assert!(!install.join("app.dll").exists());
        assert!(!backup.join("app.exe").exists()); // 备份未落盘
    }

    #[test]
    fn swap_rejects_path_traversal() {
        let install = temp_dir("trav-install");
        let staging = temp_dir("trav-staging");
        let backup = temp_dir("trav-backup");
        put(&staging, "evil", b"x");
        assert!(swap_files(&install, &staging, &["..\\evil".to_string()], &backup).is_err());
        assert!(swap_files(&install, &staging, &["sub/evil".to_string()], &backup).is_err());
        assert!(swap_files(&install, &staging, &["C:\\evil".to_string()], &backup).is_err());
    }

    #[test]
    fn restore_recovers_old_and_removes_new() {
        let install = temp_dir("rest-install");
        let staging = temp_dir("rest-staging");
        let backup = temp_dir("rest-backup");
        put(&install, "app.exe", b"old");
        put(&staging, "app.exe", b"new");
        put(&staging, "extra.dll", b"new-extra");
        let files = vec!["app.exe".to_string(), "extra.dll".to_string()];
        swap_files(&install, &staging, &files, &backup).unwrap();
        assert_eq!(get(&install, "app.exe"), b"new");
        assert!(install.join("extra.dll").exists());

        restore_backup(&install, &backup, &files).unwrap();
        assert_eq!(get(&install, "app.exe"), b"old");
        assert!(!install.join("extra.dll").exists());
    }

    #[test]
    fn wait_pid_exit_confirms_short_lived_child() {
        let pid = dummy_child(1).id();
        assert!(wait_pid_exit(pid, Duration::from_secs(5)));
        assert!(!process_alive(pid));
    }

    #[cfg(windows)]
    #[test]
    fn taskkill_fallback_kills_stubborn_child() {
        let pid = dummy_child(30).id();
        std::thread::sleep(Duration::from_millis(300));
        assert!(process_alive(pid));
        // timeout=0:轮询直接超时 → taskkill /F 兜底强杀
        assert!(wait_pid_exit(pid, Duration::ZERO));
        assert!(!process_alive(pid));
    }

    #[cfg(not(windows))]
    #[test]
    fn wait_pid_exit_zero_timeout_gives_up() {
        assert!(!wait_pid_exit(u32::MAX - 1, Duration::ZERO));
    }

    #[test]
    fn split_at_flag_parses_worker_and_relaunch_args() {
        let args: Vec<String> = [
            "--pid",
            "42",
            APPLY_FLAG,
            "--install",
            "D:\\app",
            "--staging",
            "S",
            "--",
            "--serve",
            ":80",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let (w, r) = split_at_flag(&args).unwrap();
        assert!(w.contains(&"--install".to_string()));
        assert_eq!(r, vec!["--serve".to_string(), ":80".to_string()]);
        assert!(split_at_flag(&["--foo".to_string()]).is_none());
    }

    #[test]
    fn parse_close_processes_arg() {
        let args: Vec<String> = ["--install", "I", "--staging", "S", "--close", "gui.exe, helper.exe ,"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let a = parse_apply_args(&args).unwrap();
        assert_eq!(a.close, vec!["gui.exe".to_string(), "helper.exe".to_string()]);
        assert!(parse_apply_args(&["--close".to_string(), "a b.exe".to_string()]).is_err());
    }

    #[test]
    fn process_names_whitelist_rejects_injection() {
        validate_process_names(&["demo-gui.exe".into(), "My_App-2.exe".into()]).unwrap();
        for bad in ["a b.exe", "a\"b", "a/b", "a\\b", "a;b", "", "a|b"] {
            assert!(validate_process_names(&[bad.to_string()]).is_err(), "{bad}");
        }
    }

    #[cfg(windows)]
    #[test]
    fn process_running_by_name_detects_cmd_child() {
        let _c = dummy_child(5);
        assert!(process_running_by_name("cmd.exe"));
        assert!(!process_running_by_name("ghost-updater-no-such-proc.exe"));
    }

    #[cfg(windows)]
    #[test]
    fn wait_processes_exit_fast_for_absent_name() {
        let t = std::time::Instant::now();
        assert!(wait_processes_exit(&["ghost-updater-no-such-proc.exe".into()], Duration::from_secs(5)));
        assert!(t.elapsed() < Duration::from_secs(2), "不存在名不应等到超时");
    }

    #[cfg(windows)]
    #[test]
    fn close_processes_graceful_ok_for_absent_name() {
        assert!(close_processes_graceful(
            &["ghost-updater-no-such-proc.exe".into()],
            Duration::from_millis(500)
        )
        .is_ok());
    }

    /// REQ-2 验收:锁住安装目录文件逼停覆盖段 → Err 含"已回滚"、install 回旧状态、
    /// 暂存区无 .apply-lock、result.json ok:false。share_mode(1)=FILE_SHARE_READ:
    /// 备份可读、覆盖写必败(os error 32),正好落在覆盖段失败分支。
    #[cfg(windows)]
    #[test]
    fn swap_failure_rolls_back_and_releases_lock() {
        use std::os::windows::fs::OpenOptionsExt;
        let install = temp_dir("roll-install");
        let staging = temp_dir("roll-staging");
        put(&install, "app.txt", b"old-app");
        put(&install, "locked.bin", b"old-locked");
        put(&staging, "app.txt", b"new-app");
        put(&staging, "locked.bin", b"new-locked");
        std::fs::write(
            staging.join("manifest.json"),
            serde_json::to_string(&StageManifest {
                files: vec!["app.txt".into(), "locked.bin".into()],
                sha256: "0".into(),
                main: "app.txt".into(), // 非 exe,回滚重拉会静默失败,单测无副作用
                staged_at: 0,
            })
            .unwrap(),
        )
        .unwrap();
        acquire_apply_lock(&staging).unwrap();

        let _guard = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(1) // FILE_SHARE_READ:他人可读不可写
            .open(install.join("locked.bin"))
            .unwrap();

        let argv: Vec<String> = [
            "--install",
            install.to_str().unwrap(),
            "--staging",
            staging.to_str().unwrap(),
            "--from",
            "1.0.0",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let err = run_apply(&argv, &[]).unwrap_err();
        assert!(err.contains("已回滚"), "err={err}");
        assert_eq!(get(&install, "app.txt"), b"old-app"); // 已覆盖的被还原
        assert_eq!(get(&install, "locked.bin"), b"old-locked"); // 未覆盖的原样
        assert!(!staging.join(APPLY_LOCK_NAME).exists()); // 锁已释放,可重推
        let result = std::fs::read_to_string(install.join(UPDATE_DIR_NAME).join("result.json")).unwrap();
        assert!(result.contains("\"ok\":false") && result.contains("已回滚"), "result={result}");
    }

    fn dummy_child(sleep_secs: u32) -> std::process::Child {
        #[cfg(windows)]
        let mut cmd = {
            let mut c = std::process::Command::new("cmd");
            c.args(["/C", "ping", "-n", &sleep_secs.to_string(), "127.0.0.1"]);
            c
        };
        #[cfg(not(windows))]
        let mut cmd = {
            let mut c = std::process::Command::new("sleep");
            c.arg(sleep_secs.to_string());
            c
        };
        cmd.stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
        cmd.spawn().unwrap()
    }
}
