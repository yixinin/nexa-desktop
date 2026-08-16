use crate::proxy::dns::{DnsServerConfig, IpMapping};
use crate::proxy::local_proxy::{LocalProxyConfig, LocalProxyWrapper};
use crate::proxy::tun_proxy::{TunProxy, TunProxyConfig, TUN_IP};
use anyhow::Result;
use iroh::endpoint::presets;
use iroh::{RelayMap, RelayUrl, Endpoint};
use nexapipe_client::auth::{TotpAlgorithm, TwoFactorAuth};
use nexapipe_client::endpoint_group::{EndpointGroup, NodeConfig};
use nexapipe_client::lb::LoadBalancingStrategy;
use std::str::FromStr;
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
    pub relay_mode: String,
    pub relay_url: String,
    pub force_relay: bool,
    pub two_factor_enabled: bool,
    pub two_factor_client_id: String,
    pub two_factor_secret: String,
    pub two_factor_algorithm: String,
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

        // Build iroh endpoint with relay configuration
        let mut ep_builder = Endpoint::builder(presets::N0);
        match self.config.relay_mode.as_str() {
            "disabled" => {
                tracing::info!("Relay mode: disabled (direct connections only)");
                ep_builder = ep_builder.relay_mode(iroh::RelayMode::Disabled);
            }
            "default" => {
                tracing::info!("Relay mode: default (all N0 relays)");
            }
            "custom" => {
                if !self.config.relay_url.is_empty() {
                    tracing::info!("Relay mode: custom, url={}", self.config.relay_url);
                    let relay_url = RelayUrl::from_str(&self.config.relay_url)
                        .map_err(|e| anyhow::anyhow!("Invalid relay URL: {}", e))?;
                    ep_builder = ep_builder.relay_mode(
                        iroh::RelayMode::Custom(RelayMap::from_iter(vec![relay_url])),
                    );
                } else {
                    tracing::warn!("Relay mode is custom but no URL provided, using pinned default");
                }
            }
            "pinned" | _ => {
                // 默认行为：固定 relay 到 aps1-1（亚太南），防止 relay 切换导致 WS 断线。
                let pinned_url = "https://aps1-1.relay.n0.iroh.link.";
                let relay_url = RelayUrl::from_str(pinned_url)
                    .expect("PINNED_RELAY_URL must be a valid relay URL");
                tracing::info!("Relay mode: pinned to {}", pinned_url);
                ep_builder = ep_builder.relay_mode(
                    iroh::RelayMode::Custom(RelayMap::from_iter(vec![relay_url])),
                );
            }
        }

        let iroh_endpoint = ep_builder.bind().await
            .map_err(|e| anyhow::anyhow!("Failed to bind iroh endpoint: {}", e))?;
        tracing::info!("Iroh endpoint bound, node_id={}", iroh_endpoint.id());
        if self.config.force_relay {
            tracing::info!("Force relay enabled: direct connections will be disabled");
        }

        let endpoint_group =
            EndpointGroup::new_with_nodes_and_endpoint(nodes.clone(), None, self.config.load_balancing.into(), iroh_endpoint)
                .await
                .map_err(|e| anyhow::anyhow!(e))?;

        // 2FA：若启用且配置了 secret，则每个新建连接会先执行认证握手。
        if self.config.two_factor_enabled && !self.config.two_factor_secret.trim().is_empty() {
            let auth = TwoFactorAuth::new(
                &self.config.two_factor_client_id,
                &self.config.two_factor_secret,
                TotpAlgorithm::from_name(&self.config.two_factor_algorithm),
            )
            .map_err(|e| anyhow::anyhow!("Invalid 2FA config: {}", e))?;
            endpoint_group.set_two_factor(Some(auth)).await;
            tracing::info!(
                "2FA enabled, client_id: {}",
                self.config.two_factor_client_id
            );
        }

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
