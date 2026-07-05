mod proxy;
pub mod service;

use proxy::{ProxyManager, ProxyManagerConfig, ProxyNodeConfig, ConnectionConfig, ProxyLoadBalancingStrategy};
use service::IpcClient;
use service::ipc::NodeInput;
use std::sync::Arc;
use tokio::sync::RwLock;

lazy_static::lazy_static! {
    static ref PROXY_MANAGER: Arc<RwLock<Option<Arc<ProxyManager>>>> = Arc::new(RwLock::new(None));
    static ref STARTUP_ERROR: Arc<RwLock<Option<String>>> = Arc::new(RwLock::new(None));
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
    use_service: Option<bool>,
) -> Result<String, String> {
    let use_service = use_service.unwrap_or(false);

    if use_service {
        match IpcClient::start_proxy(nodes.clone(), domains.clone(), local_addr.clone(), dns_addr.clone(), upstream_dns.clone(), load_balancing.clone()).await {
            Ok(msg) => return Ok(msg),
            Err(e) => {
                tracing::warn!("Failed to start proxy via service, falling back to process mode: {}", e);
            }
        }
    }

    let parsed_nodes: Vec<ProxyNodeConfig> = nodes.into_iter()
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
    let load_balancing: ProxyLoadBalancingStrategy = load_balancing.parse().map_err(|e: String| e)?;

    let local_addr = local_addr.unwrap_or_else(|| "127.0.0.1:8080".to_string());
    let dns_addr = dns_addr.unwrap_or_else(|| "10.0.0.1:53".to_string());
    let upstream_dns = upstream_dns.unwrap_or_else(|| "8.8.8.8:53".to_string());

    #[cfg(windows)]
    let tun_name = "pipe-ui-tun".to_string();

    let config = ProxyManagerConfig {
        nodes: parsed_nodes,
        local_proxy_addr: local_addr,
        dns_listen_addr: dns_addr,
        upstream_dns,
        load_balancing,
        #[cfg(windows)]
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
                tracing::warn!("Failed to stop proxy via service, trying process mode: {}", e);
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
                tracing::warn!("Failed to get status via service, falling back to process mode: {}", e);
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
                tracing::warn!("Failed to get node ID via service, falling back to process mode: {}", e);
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_target(false)
        .with_level(true)
        .init();

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
            get_startup_error
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
