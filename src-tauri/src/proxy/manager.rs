use crate::proxy::dns::{DnsServerConfig, IpMapping};
use crate::proxy::local_proxy::{LocalProxyConfig, LocalProxyWrapper};
use crate::proxy::tun_proxy::{TunProxy, TunProxyConfig, TUN_IP};
use anyhow::Result;
use nexapipe_client::endpoint_group::{EndpointGroup, NodeConfig};
use nexapipe_client::lb::LoadBalancingStrategy;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ProxyMode {
    Tun,
    LocalProxy,
}

#[derive(Debug, Clone)]
pub enum ConnectionConfig {
    Ticket(String),
    EndpointId(String),
}

#[derive(Debug, Clone)]
pub struct ProxyNodeConfig {
    pub connection: ConnectionConfig,
    pub domains: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProxyLoadBalancingStrategy {
    RoundRobin,
    Random,
}

impl std::str::FromStr for ProxyLoadBalancingStrategy {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "round_robin" => Ok(Self::RoundRobin),
            "random" => Ok(Self::Random),
            _ => Err(format!("Unknown load balancing strategy: {}", s)),
        }
    }
}

impl From<ProxyLoadBalancingStrategy> for LoadBalancingStrategy {
    fn from(strategy: ProxyLoadBalancingStrategy) -> Self {
        match strategy {
            ProxyLoadBalancingStrategy::RoundRobin => LoadBalancingStrategy::RoundRobin,
            ProxyLoadBalancingStrategy::Random => LoadBalancingStrategy::Random,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ProxyManagerConfig {
    pub nodes: Vec<ProxyNodeConfig>,
    pub local_proxy_addr: String,
    pub dns_listen_addr: String,
    pub upstream_dns: String,
    pub load_balancing: ProxyLoadBalancingStrategy,
    pub tun_name: String,
}

struct ProxyInstance {
    tun_proxy: Option<Arc<TunProxy>>,
    local_proxy: Option<Arc<LocalProxyWrapper>>,
    endpoint_group: Option<Arc<EndpointGroup>>,
}

impl ProxyInstance {
    async fn stop(self) {
        tracing::info!("Stopping proxy instance");

        // 本地 DNS 服务器由 tun_proxy::run 内部管理，停止 TUN 时自动清理
        if let Some(tun_proxy) = self.tun_proxy {
            tun_proxy.stop();
        }

        if let Some(local_proxy) = self.local_proxy {
            local_proxy.stop().await;
        }

        if let Some(endpoint_group) = self.endpoint_group {
            endpoint_group.close_all().await;
        }
    }
}

pub struct ProxyManager {
    config: ProxyManagerConfig,
    ip_mapping: Arc<IpMapping>,
    mode: parking_lot::Mutex<Option<ProxyMode>>,
    instance: parking_lot::Mutex<Option<ProxyInstance>>,
}

impl ProxyManager {
    pub fn new(config: ProxyManagerConfig) -> Self {
        Self {
            config,
            ip_mapping: Arc::new(IpMapping::new()),
            mode: parking_lot::Mutex::new(None),
            instance: parking_lot::Mutex::new(None),
        }
    }

    pub fn get_mode(&self) -> Option<ProxyMode> {
        self.mode.lock().clone()
    }

    pub async fn stop(&self) {
        tracing::info!("ProxyManager stopping");
        let instance = self.instance.lock().take();
        if let Some(instance) = instance {
            instance.stop().await;
        }
        *self.mode.lock() = None;
    }

    pub async fn start(&self) -> Result<()> {
        tracing::info!("Initializing proxy manager...");

        let all_domains: Vec<String> = self
            .config
            .nodes
            .iter()
            .flat_map(|node| node.domains.iter().cloned())
            .collect();

        let nodes: Vec<NodeConfig> = self
            .config
            .nodes
            .iter()
            .map(|node| match &node.connection {
                ConnectionConfig::Ticket(t) => NodeConfig {
                    server_node_id: None,
                    server_ticket: Some(t.clone()),
                    domains: node.domains.clone(),
                },
                ConnectionConfig::EndpointId(e) => NodeConfig {
                    server_node_id: Some(e.clone()),
                    server_ticket: None,
                    domains: node.domains.clone(),
                },
            })
            .collect();

        let endpoint_group =
            EndpointGroup::new_with_nodes(nodes.clone(), None, self.config.load_balancing.into())
                .await
                .map_err(|e| anyhow::anyhow!(e))?;

        let endpoint_group = Arc::new(endpoint_group);

        tracing::info!("EndpointGroup initialized with {} nodes", nodes.len());

        if TunProxy::is_available().await {
            tracing::info!("Admin privileges available, starting TUN + DNS hijack mode");

            // 本地 DNS 服务器与系统 DNS 切换由 tun_proxy::run 内部完成
            //（接口配置后启动 DNS，避免 WSAEADDRNOTAVAIL），此处只需构造配置。
            let dns_ip = self
                .config
                .dns_listen_addr
                .split(':')
                .next()
                .unwrap_or(TUN_IP)
                .to_string();
            let tun_config = TunProxyConfig {
                tunnel_name: self.config.tun_name.clone(),
                dns_ip,
                dns: DnsServerConfig {
                    listen_addr: self.config.dns_listen_addr.clone(),
                    upstream_dns: self.config.upstream_dns.clone(),
                    proxy_domains: all_domains.clone(),
                },
            };
            let tun_proxy = Arc::new(TunProxy::new(
                tun_config,
                endpoint_group.clone(),
                self.ip_mapping.clone(),
            ));

            *self.instance.lock() = Some(ProxyInstance {
                tun_proxy: Some(tun_proxy.clone()),
                local_proxy: None,
                endpoint_group: Some(endpoint_group.clone()),
            });
            *self.mode.lock() = Some(ProxyMode::Tun);

            // run() 内部负责设备创建、接口/路由/DNS 配置，退出时自动恢复
            match tun_proxy.run().await {
                Ok(()) => {
                    tracing::info!("TUN proxy stopped successfully");
                    // Properly close endpoints before clearing instance
                    let instance = self.instance.lock().take();
                    if let Some(instance) = instance {
                        instance.stop().await;
                    }
                    *self.mode.lock() = None;
                    return Ok(());
                }
                Err(e) => {
                    tracing::warn!("TUN proxy failed: {}, falling back to local proxy", e);
                    // Properly close endpoints before clearing instance
                    let instance = self.instance.lock().take();
                    if let Some(instance) = instance {
                        instance.stop().await;
                    }
                }
            }
        } else {
            tracing::warn!("Admin privileges not available, falling back to local proxy mode");
        }

        *self.mode.lock() = Some(ProxyMode::LocalProxy);
        tracing::info!("Starting local proxy...");

        let local_proxy_config = LocalProxyConfig {
            listen_addr: self.config.local_proxy_addr.clone(),
            proxy_domains: all_domains,
            nodes,
            load_balancing: self.config.load_balancing.into(),
        };

        let local_proxy = Arc::new(LocalProxyWrapper::new(local_proxy_config).await?);

        *self.instance.lock() = Some(ProxyInstance {
            tun_proxy: None,
            local_proxy: Some(local_proxy.clone()),
            endpoint_group: Some(endpoint_group.clone()),
        });

        // Spawn the local proxy run loop as a separate task. This keeps the
        // large async state machine (handle_local_connection, handle_tls_tunnel,
        // etc.) off the current task's stack, preventing stack overflow on
        // tokio worker threads (default 2 MB stack).
        let run_local_proxy = local_proxy.clone();
        tokio::spawn(async move {
            if let Err(e) = run_local_proxy.run().await {
                tracing::error!("Local proxy error: {}", e);
            }
        });

        // Local proxy 已在后台任务中运行，start() 直接返回；
        // 停止由 ProxyManager::stop() 负责，切勿在此清理实例。
        Ok(())
    }

    pub async fn get_node_id(&self) -> Option<String> {
        None
    }
}
