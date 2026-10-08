//! ghost-push:ghost-updater 推送端命令行。
//!
//!   ghost-push push --target <host[:9600]> ... --dist <dir> [--main x.exe]
//!                   [--token t] [--sign-key <hex>] [--overwrite-config]
//!                   [--jobs N] [--timeout-secs 120]
//!   ghost-push keygen
//!
//! 多目标并发推送,每目标进度行实时滚动,全部成功退出码 0。

use ghost_updater_cli::{keygen, normalize_addr, probe, push_update, PushCtx, PushUpdateCfg};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = run(args);
    std::process::exit(code);
}

fn run(args: Vec<String>) -> i32 {
    match args.first().map(|s| s.as_str()) {
        Some("keygen") => match keygen() {
            Ok((seed, pubkey)) => {
                println!("sign-key(seed)  = {seed}");
                println!("pubkey(hex)     = {pubkey}");
                println!("服务端 pubkey_hex 配公钥;推送端 --sign-key 配私钥。");
                0
            }
            Err(e) => {
                eprintln!("keygen 失败: {e}");
                1
            }
        },
        Some("push") => tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(cmd_push(&args[1..])),
        _ => {
            print_usage();
            2
        }
    }
}

fn print_usage() {
    eprintln!(
        "用法:\n  ghost-push push --target <host[:9600]> [--target ...] [--targets-file <f>]\n                  --dist <dir> [--main x.exe] [--token <t>] [--sign-key <hex>]\n                  [--overwrite-config] [--jobs <N>] [--timeout-secs <s>]\n  ghost-push keygen"
    );
}

#[derive(Default)]
struct Opts {
    targets: Vec<String>,
    dist: Option<PathBuf>,
    main: Option<String>,
    token: Option<String>,
    sign_key: Option<String>,
    overwrite_config: bool,
    jobs: usize,
    timeout_secs: u64,
}

fn parse_opts(args: &[String]) -> Result<Opts, String> {
    let mut o = Opts { jobs: 4, timeout_secs: 120, ..Default::default() };
    let mut i = 0;
    while i < args.len() {
        let need = |i: usize| -> Result<String, String> {
            args.get(i + 1).cloned().ok_or_else(|| format!("{} 缺值", args[i]))
        };
        match args[i].as_str() {
            "--target" => o.targets.push(normalize_addr(&need(i)?)),
            "--targets-file" => {
                let f = need(i)?;
                for line in std::fs::read_to_string(&f).map_err(|e| format!("读 {f}: {e}"))?.lines() {
                    let l = line.trim();
                    if !l.is_empty() && !l.starts_with('#') {
                        o.targets.push(normalize_addr(l));
                    }
                }
            }
            "--dist" => o.dist = Some(PathBuf::from(need(i)?)),
            "--main" => o.main = Some(need(i)?),
            "--token" => o.token = Some(need(i)?),
            "--sign-key" => o.sign_key = Some(need(i)?),
            "--overwrite-config" => o.overwrite_config = true,
            "--jobs" => o.jobs = need(i)?.parse().map_err(|_| "--jobs 非法")?,
            "--timeout-secs" => o.timeout_secs = need(i)?.parse().map_err(|_| "--timeout-secs 非法")?,
            other => return Err(format!("未知参数: {other}")),
        }
        i += 2;
    }
    if o.dist.is_none() {
        return Err("缺少 --dist".into());
    }
    if o.targets.is_empty() {
        return Err("缺少 --target / --targets-file".into());
    }
    o.targets.sort();
    o.targets.dedup();
    Ok(o)
}

async fn cmd_push(args: &[String]) -> i32 {
    let opts = match parse_opts(args) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("参数错误: {e}\n");
            print_usage();
            return 2;
        }
    };
    let ctx = match PushCtx::new(opts.sign_key.as_deref()) {
        Ok(c) => Arc::new(c),
        Err(e) => {
            eprintln!("{e}");
            return 2;
        }
    };
    let dist = opts.dist.clone().unwrap();
    let (targets, jobs, token, main, overwrite_config, poll_timeout) = (
        opts.targets.clone(),
        opts.jobs.max(1),
        opts.token.clone(),
        opts.main.clone(),
        opts.overwrite_config,
        Duration::from_secs(opts.timeout_secs),
    );

    println!("== 目标 {} 台,并发 {jobs} ==", targets.len());
    for t in &targets {
        match probe(&ctx, t, token.as_deref()).await {
            Ok(p) => println!("  [在线] {t}  v{} ({} {})", p.version, p.os, p.arch),
            Err(e) => println!("  [离线] {t}  {e}"),
        }
    }

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<(String, String)>();
    let printer = tokio::spawn(async move {
        while let Some((addr, line)) = rx.recv().await {
            println!("[{addr}] {line}");
        }
    });

    let sem = Arc::new(tokio::sync::Semaphore::new(jobs));
    let mut handles = Vec::new();
    for target in targets {
        let permit = sem.clone().acquire_owned().await.unwrap();
        let ctx = ctx.clone();
        let tx = tx.clone();
        let dist = dist.clone();
        let (main, token, overwrite_config, poll_timeout) =
            (main.clone(), token.clone(), overwrite_config, poll_timeout);
        handles.push(tokio::spawn(async move {
            let (etx, mut erx) = tokio::sync::mpsc::channel::<String>(256);
            let fwd = {
                let tx = tx.clone();
                let addr = target.clone();
                tokio::spawn(async move {
                    while let Some(line) = erx.recv().await {
                        let _ = tx.send((addr.clone(), line));
                    }
                })
            };
            let cfg = PushUpdateCfg {
                addr: target.clone(),
                dist_dir: dist,
                main,
                token,
                sign_key_hex: None, // 签名在 PushCtx 统一持有
                overwrite_config,
                poll_timeout,
                on_event: Some(etx),
            };
            let report = push_update(&ctx, cfg).await;
            let _ = fwd.await;
            drop(permit);
            report
        }));
    }

    let mut results = Vec::new();
    for h in handles {
        if let Ok(r) = h.await {
            results.push(r);
        }
    }
    drop(tx);
    let _ = printer.await;

    println!("== 结果 ==");
    let mut failed = 0;
    for r in &results {
        if r.ok {
            println!("  [成功] {}  {} → {}", r.addr, r.old_version, r.new_version);
        } else {
            failed += 1;
            println!("  [失败] {}  {}  {}", r.addr, r.old_version, r.detail);
        }
    }
    if failed > 0 {
        println!("失败 {failed}/{};可用 --target 单独重试失败节点", results.len());
        1
    } else {
        println!("全部成功({})", results.len());
        0
    }
}
