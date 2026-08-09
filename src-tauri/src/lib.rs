mod proxy;
pub mod service;

use proxy::{
    ConnectionConfig, ProxyLoadBalancingStrategy, ProxyManager, ProxyManagerConfig, ProxyNodeConfig,
};
use service::ipc::NodeInput;
use service::IpcClient;
use std::sync::Arc;
use tokio::sync::RwLock;

lazy_static::lazy_static! {
    static ref PROXY_MANAGER: Arc<RwLock<Option<Arc<ProxyManager>>>> = Arc::new(RwLock::new(None));
    static ref STARTUP_ERROR: Arc<RwLock<Option<String>>> = Arc::new(RwLock::new(None));
}

/// 日志目录：%APPDATA%/nexapipe/logs（Windows），其他平台用临时目录
pub fn log_dir() -> std::path::PathBuf {
    std::env::var("APPDATA")
        .map(|d| std::path::Path::new(&d).join("nexapipe").join("logs"))
        .unwrap_or_else(|_| std::env::temp_dir().join("nexapipe"))
}

/// 初始化日志：默认 info 级别（可用 RUST_LOG 覆盖），同时输出到
/// 控制台与日志文件（%APPDATA%/nexapipe/logs/{prefix}.log.YYYY-MM-DD）。
/// 返回的 guard 必须保持存活以保证日志刷新线程运行。
pub fn init_tracing(prefix: &str) -> tracing_appender::non_blocking::WorkerGuard {
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;

    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));

    let dir = log_dir();
    let _ = std::fs::create_dir_all(&dir);
    let file_appender = tracing_appender::rolling::daily(&dir, prefix);
    let (file_writer, guard) = tracing_appender::non_blocking(file_appender);

    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer().with_writer(std::io::stdout))
        .with(
            tracing_subscriber::fmt::layer()
                .with_writer(file_writer)
                .with_ansi(false),
        )
        .with(filter)
        .init();
    guard
}

#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}

#[tauri::command]
async fn start_proxy(
    nodes: Vec<NodeInput>,
    domains: Vec<String>,
    local_addr: Option<String>,
    dns_addr: Option<String>,
    upstream_dns: Option<String>,
    load_balancing: Option<String>,
    tun_name: Option<String>,
    use_service: Option<bool>,
) -> Result<String, String> {
    let use_service = use_service.unwrap_or(false);

    if use_service {
        match IpcClient::start_proxy(
            nodes.clone(),
            domains.clone(),
            local_addr.clone(),
            dns_addr.clone(),
            upstream_dns.clone(),
            load_balancing.clone(),
            tun_name.clone(),
        )
        .await
        {
            Ok(msg) => return Ok(msg),
            Err(e) => {
                tracing::warn!(
                    "Failed to start proxy via service, falling back to process mode: {}",
                    e
                );
            }
        }
    }

    let parsed_nodes: Vec<ProxyNodeConfig> = nodes
        .into_iter()
        .filter(|n| !n.ticket.is_empty() || !n.endpoint_id.is_empty())
        .map(|n| {
            let connection = if n.connection_type == "ticket" || !n.ticket.is_empty() {
                ConnectionConfig::Ticket(n.ticket)
            } else {
                ConnectionConfig::EndpointId(n.endpoint_id)
            };
            ProxyNodeConfig {
                connection,
                domains: n.domains,
            }
        })
        .collect();

    if parsed_nodes.is_empty() {
        return Err("At least one node must be configured".to_string());
    }

    let load_balancing = load_balancing.unwrap_or_else(|| "round_robin".to_string());
    let load_balancing: ProxyLoadBalancingStrategy =
        load_balancing.parse().map_err(|e: String| e)?;

    let local_addr = local_addr.unwrap_or_else(|| "127.0.0.1:8080".to_string());
    let dns_addr = dns_addr.unwrap_or_else(|| "10.0.0.1:53".to_string());
    let upstream_dns = upstream_dns.unwrap_or_else(|| "8.8.8.8:53".to_string());

    let tun_name = tun_name.unwrap_or_else(|| "pipe-tun".to_string());

    let config = ProxyManagerConfig {
        nodes: parsed_nodes,
        local_proxy_addr: local_addr,
        dns_listen_addr: dns_addr,
        upstream_dns,
        load_balancing,
        tun_name,
    };

    let manager = Arc::new(ProxyManager::new(config));

    {
        let mut proxy_manager = PROXY_MANAGER.write().await;
        *proxy_manager = Some(manager.clone());
    }

    let startup_error_clone = STARTUP_ERROR.clone();
    tokio::spawn(async move {
        if let Err(e) = manager.start().await {
            tracing::error!("Proxy manager failed: {}", e);
            *startup_error_clone.write().await = Some(e.to_string());
        }
    });

    Ok("Proxy started successfully (process mode)".to_string())
}

#[tauri::command]
async fn stop_proxy(use_service: Option<bool>) -> Result<String, String> {
    let use_service = use_service.unwrap_or(false);

    if use_service {
        match IpcClient::stop_proxy().await {
            Ok(msg) => return Ok(msg),
            Err(e) => {
                tracing::warn!(
                    "Failed to stop proxy via service, trying process mode: {}",
                    e
                );
            }
        }
    }

    let proxy_manager = PROXY_MANAGER.write().await;
    if let Some(manager) = proxy_manager.as_ref() {
        manager.stop().await;
    }
    Ok("Proxy stopped (process mode)".to_string())
}

#[tauri::command]
async fn get_proxy_status(use_service: Option<bool>) -> Result<String, String> {
    let use_service = use_service.unwrap_or(false);

    if use_service {
        match IpcClient::get_status().await {
            Ok((running, mode)) => return Ok(format!("{}:{}", running, mode)),
            Err(e) => {
                tracing::warn!(
                    "Failed to get status via service, falling back to process mode: {}",
                    e
                );
            }
        }
    }

    let proxy_manager = PROXY_MANAGER.read().await;
    if let Some(manager) = proxy_manager.as_ref() {
        match manager.get_mode() {
            Some(proxy::manager::ProxyMode::Tun) => Ok("true:tun".to_string()),
            Some(proxy::manager::ProxyMode::LocalProxy) => Ok("true:local_proxy".to_string()),
            None => Ok("false:starting".to_string()),
        }
    } else {
        Ok("false:stopped".to_string())
    }
}

#[tauri::command]
async fn get_node_id(use_service: Option<bool>) -> Result<String, String> {
    let use_service = use_service.unwrap_or(false);

    if use_service {
        match IpcClient::get_node_id().await {
            Ok(id) => return Ok(id),
            Err(e) => {
                tracing::warn!(
                    "Failed to get node ID via service, falling back to process mode: {}",
                    e
                );
            }
        }
    }

    let proxy_manager = PROXY_MANAGER.read().await;
    if let Some(manager) = proxy_manager.as_ref() {
        match manager.get_node_id().await {
            Some(id) => Ok(id),
            None => Err("Node ID not available".to_string()),
        }
    } else {
        Err("Proxy not running".to_string())
    }
}

#[tauri::command]
async fn install_service() -> Result<String, String> {
    match IpcClient::install_service().await {
        Ok(msg) => Ok(msg),
        Err(e) => Err(e.to_string()),
    }
}

#[tauri::command]
async fn uninstall_service() -> Result<String, String> {
    match IpcClient::uninstall_service().await {
        Ok(msg) => Ok(msg),
        Err(e) => Err(e.to_string()),
    }
}

#[tauri::command]
async fn is_service_running() -> Result<bool, String> {
    Ok(IpcClient::is_service_running().await)
}

#[tauri::command]
async fn get_startup_error() -> Result<Option<String>, String> {
    Ok(STARTUP_ERROR.read().await.clone())
}

/// 读取日志文件尾部（默认最近 200 行），供前端日志页展示。
#[tauri::command]
async fn get_logs(limit: Option<usize>) -> Result<Vec<String>, String> {
    use std::io::{Read, Seek};

    let limit = limit.unwrap_or(200);
    let dir = log_dir();

    // 取日志目录中修改时间最新的 nexapipe.log 文件（daily 滚动：nexapipe.log.YYYY-MM-DD）
    let newest = std::fs::read_dir(&dir)
        .map_err(|e| e.to_string())?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.is_file()
                && p.file_name()
                    .map(|n| n.to_string_lossy().starts_with("nexapipe.log"))
                    .unwrap_or(false)
        })
        .max_by_key(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok());

    let Some(path) = newest else {
        return Ok(vec![]);
    };

    // 只读取文件尾部，避免大文件时读入全部内容
    let mut file = std::fs::File::open(&path).map_err(|e| e.to_string())?;
    let file_len = file.metadata().map(|m| m.len()).unwrap_or(0);
    const MAX_TAIL: u64 = 512 * 1024; // 最多读尾部 512KB
    let offset = file_len.saturating_sub(MAX_TAIL);
    let mut content = String::new();
    if offset > 0 {
        file.seek(std::io::SeekFrom::Start(offset))
            .map_err(|e| e.to_string())?;
    }
    file.read_to_string(&mut content)
        .map_err(|e| e.to_string())?;

    let lines: Vec<String> = content.lines().map(|l| l.to_string()).collect();
    let start = lines.len().saturating_sub(limit);
    Ok(lines[start..].to_vec())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let _guard = init_tracing("nexapipe.log");

    // Increase tokio worker thread stack size to 4 MB (default 2 MB) to
    // prevent stack overflow from deeply nested async state machines in
    // iroh / proxy code.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .thread_stack_size(4 * 1024 * 1024)
        .enable_all()
        .build()
        .expect("failed to build tokio runtime");
    let handle = runtime.handle().clone();
    // Prevent the runtime from being dropped — it must live as long as the
    // process. Tauri holds the Handle and uses it for all async commands.
    std::mem::forget(runtime);
    tauri::async_runtime::set(handle);

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            greet,
            start_proxy,
            stop_proxy,
            get_proxy_status,
            get_node_id,
            install_service,
            uninstall_service,
            is_service_running,
            get_startup_error,
            get_logs
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
