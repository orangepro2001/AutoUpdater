//! demo-app:ghost-updater 嵌入式集成的最小宿主示例。
//!
//! 接入总量 = main 顶部一行 guard + Router 合并一行 routes(),其余全是业务代码。

use axum::routing::get;

fn main() {
    // 拦截内部触发的更新替换模式:如果是 apply worker,它会等旧进程退出、
    // 换文件、重启新版,然后直接 exit,永不返回到这里
    ghost_updater_core::guard_apply_update();

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async_main());
}

async fn async_main() {
    let addr: std::net::SocketAddr = parse_addr();
    let token = std::env::args()
        .position(|a| a == "--token")
        .and_then(|i| std::env::args().nth(i + 1));
    // REQ-1 多进程宿主:apply 受理后需优雅关闭的进程映像名,逗号分隔
    let close_processes: Vec<String> = std::env::args()
        .position(|a| a == "--close-processes")
        .and_then(|i| std::env::args().nth(i + 1))
        .map(|s| {
            s.split(',')
                .map(|n| n.trim().to_string())
                .filter(|n| !n.is_empty())
                .collect()
        })
        .unwrap_or_default();

    let app = axum::Router::new()
        .route("/hello", get(|| async { "world" }))
        .merge(ghost_updater_net::routes(ghost_updater_net::UpdateConfig {
            version: app_version(),
            auth_token: token,
            close_processes,
            ..Default::default()
        }));
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    println!("[demo] v{} 监听 {addr}", app_version());
    axum::serve(listener, app).await.unwrap();
}

fn parse_addr() -> std::net::SocketAddr {
    let a = std::env::args()
        .position(|arg| arg == "--addr")
        .and_then(|i| std::env::args().nth(i + 1))
        .unwrap_or_else(|| "127.0.0.1:7600".into());
    a.parse().unwrap_or_else(|_| panic!("--addr 非法: {a}"))
}

/// 版本号从 exe 旁 VERSION 文件读(换版即变化);缺省用包版本
fn app_version() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|p| std::fs::read_to_string(p.with_file_name("VERSION")).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string())
}
