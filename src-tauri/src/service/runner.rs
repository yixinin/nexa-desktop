use crate::proxy::{ProxyManager, ProxyManagerConfig, ProxyNodeConfig, ConnectionConfig, ProxyLoadBalancingStrategy};
use crate::service::ipc::{IpcMessage, IpcResponse, IPC_SOCKET_PATH, NodeInput};
use anyhow::{Context, Result};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;

pub struct ServiceRunner {
    proxy_manager: Arc<tokio::sync::RwLock<Option<Arc<ProxyManager>>>>,
}

impl ServiceRunner {
    pub fn new() -> Self {
        Self {
            proxy_manager: Arc::new(tokio::sync::RwLock::new(None)),
        }
    }

    pub async fn run(&self) -> Result<()> {
        tracing::info!("Service runner starting");

        let listener = TcpListener::bind(IPC_SOCKET_PATH)
            .await
            .context("Failed to bind IPC socket")?;

        tracing::info!("IPC server listening on: {}", IPC_SOCKET_PATH);

        loop {
            let (mut stream, _) = listener.accept().await.context("Failed to accept connection")?;
            tracing::debug!("Client connected");

            let proxy_manager = self.proxy_manager.clone();
            tokio::spawn(async move {
                if let Err(e) = Self::handle_client(&mut stream, proxy_manager).await {
                    tracing::error!("Client handler error: {}", e);
                }
            });
        }
    }

    async fn handle_client(
        stream: &mut tokio::net::TcpStream,
        proxy_manager: Arc<tokio::sync::RwLock<Option<Arc<ProxyManager>>>>,
    ) -> Result<()> {
        let mut reader = BufReader::new(stream);
        let mut buffer = String::new();

        loop {
            buffer.clear();
            let n = reader
                .read_line(&mut buffer)
                .await
                .context("Failed to read from stream")?;

            if n == 0 {
                tracing::debug!("Client disconnected");
                break;
            }

            let msg: IpcMessage = serde_json::from_str(&buffer)
                .context("Failed to parse IPC message")?;

            tracing::debug!("Received IPC message: {:?}", msg);

            let response = match msg {
                IpcMessage::StartProxy(req) => Self::handle_start_proxy(
                    req.nodes,
                    req.domains,
                    req.local_addr,
                    req.dns_addr,
                    req.upstream_dns,
                    req.load_balancing,
                    &proxy_manager,
                )
                .await,
                IpcMessage::StopProxy => Self::handle_stop_proxy(&proxy_manager).await,
                IpcMessage::GetStatus => Self::handle_get_status(&proxy_manager).await,
                IpcMessage::GetNodeId => Self::handle_get_node_id(&proxy_manager).await,
                IpcMessage::InstallService => Self::handle_install_service().await,
                IpcMessage::UninstallService => Self::handle_uninstall_service().await,
            };

            let response_str = serde_json::to_string(&response)
                .context("Failed to serialize response")?;

            reader
                .get_mut()
                .write_all(response_str.as_bytes())
                .await
                .context("Failed to write response")?;
        }

        Ok(())
    }

    async fn handle_start_proxy(
        nodes: Vec<NodeInput>,
        _domains: Vec<String>,
        local_addr: Option<String>,
        dns_addr: Option<String>,
        upstream_dns: Option<String>,
        load_balancing: Option<String>,
        proxy_manager: &Arc<tokio::sync::RwLock<Option<Arc<ProxyManager>>>>,
    ) -> IpcResponse {
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
            return IpcResponse::Error("At least one node must be configured".to_string());
        }

        let load_balancing = load_balancing.unwrap_or_else(|| "round_robin".to_string());
        let load_balancing: ProxyLoadBalancingStrategy = match load_balancing.parse() {
            Ok(lb) => lb,
            Err(e) => return IpcResponse::Error(format!("Invalid load balancing strategy: {}", e)),
        };

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
            let mut pm = proxy_manager.write().await;
            *pm = Some(manager.clone());
        }

        let proxy_manager_clone = proxy_manager.clone();
        tokio::spawn(async move {
            if let Err(e) = manager.start().await {
                tracing::error!("Proxy manager failed: {}", e);
                let mut pm = proxy_manager_clone.write().await;
                *pm = None;
            }
        });

        IpcResponse::Ok("Proxy started".to_string())
    }

    async fn handle_stop_proxy(proxy_manager: &Arc<tokio::sync::RwLock<Option<Arc<ProxyManager>>>>) -> IpcResponse {
        let pm = proxy_manager.write().await;
        if let Some(manager) = pm.as_ref() {
            manager.stop().await;
        }
        IpcResponse::Ok("Proxy stopped".to_string())
    }

    async fn handle_get_status(proxy_manager: &Arc<tokio::sync::RwLock<Option<Arc<ProxyManager>>>>) -> IpcResponse {
        let pm = proxy_manager.read().await;
        if let Some(manager) = pm.as_ref() {
            match manager.get_mode() {
                Some(mode) => IpcResponse::Status {
                    running: true,
                    mode: format!("{:?}", mode),
                },
                None => IpcResponse::Status {
                    running: true,
                    mode: "starting".to_string(),
                },
            }
        } else {
            IpcResponse::Status {
                running: false,
                mode: "stopped".to_string(),
            }
        }
    }

    async fn handle_get_node_id(proxy_manager: &Arc<tokio::sync::RwLock<Option<Arc<ProxyManager>>>>) -> IpcResponse {
        let pm = proxy_manager.read().await;
        if let Some(manager) = pm.as_ref() {
            match manager.get_node_id().await {
                Some(id) => IpcResponse::NodeId(id),
                None => IpcResponse::Error("Node ID not available".to_string()),
            }
        } else {
            IpcResponse::Error("Proxy not running".to_string())
        }
    }

    async fn handle_install_service() -> IpcResponse {
        match crate::service::platform::install_service() {
            Ok(msg) => IpcResponse::Ok(msg),
            Err(e) => IpcResponse::Error(e),
        }
    }

    async fn handle_uninstall_service() -> IpcResponse {
        match crate::service::platform::uninstall_service() {
            Ok(msg) => IpcResponse::Ok(msg),
            Err(e) => IpcResponse::Error(e),
        }
    }
}
