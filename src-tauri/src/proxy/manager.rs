use crate::proxy::dns::{DnsServer, DnsServerConfig, IpMapping};
use crate::proxy::local_proxy::{LocalProxyWrapper, LocalProxyConfig};
use nexapipe_client::endpoint_group::{EndpointGroup, NodeConfig};
use nexapipe_client::lb::LoadBalancingStrategy;
use anyhow::Result;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

#[cfg(windows)]
use crate::proxy::tun_proxy::{TunProxy, TunProxyConfig};

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
    #[cfg(windows)]
    pub tun_name: String,
}

struct ProxyInstance {
    #[cfg(windows)]
    tun_proxy: Option<Arc<TunProxy>>,
    local_proxy: Option<Arc<LocalProxyWrapper>>,
    dns_stopped: Option<Arc<AtomicBool>>,
    dns_handle: Option<tokio::task::JoinHandle<()>>,
    endpoint_group: Option<Arc<EndpointGroup>>,
}

impl ProxyInstance {
    async fn stop(self) {
        tracing::info!("Stopping proxy instance");

        if let Some(dns_stopped) = self.dns_stopped {
            dns_stopped.store(true, Ordering::Release);
        }

        if let Some(dns_handle) = self.dns_handle {
            if let Err(e) = dns_handle.await {
                tracing::debug!("DNS handle join error: {:?}", e);
            }
        }

        #[cfg(windows)]
        {
            if let Some(tun_proxy) = self.tun_proxy {
                tun_proxy.stop();
            }
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
        #[cfg(windows)]
        {
            let _ = self.restore_system_dns().await;
        }
    }

    pub async fn start(&self) -> Result<()> {
        tracing::info!("Initializing proxy manager...");

        let all_domains: Vec<String> = self.config.nodes.iter()
            .flat_map(|node| node.domains.iter().cloned())
            .collect();

        let nodes: Vec<NodeConfig> = self.config.nodes.iter()
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

        let endpoint_group = EndpointGroup::new_with_nodes(
            nodes.clone(),
            None,
            self.config.load_balancing.into(),
        ).await.map_err(|e| anyhow::anyhow!(e))?;

        let endpoint_group = Arc::new(endpoint_group);

        tracing::info!("EndpointGroup initialized with {} nodes", nodes.len());

        #[cfg(windows)]
        {
            if TunProxy::is_available().await {
                tracing::info!("Admin privileges available, starting TUN + DNS hijack mode");

                let dns_config = DnsServerConfig {
                    listen_addr: self.config.dns_listen_addr.clone(),
                    upstream_dns: self.config.upstream_dns.clone(),
                    proxy_domains: all_domains.clone(),
                };
                let dns_server = DnsServer::new(dns_config, self.ip_mapping.clone());
                let dns_stopped = dns_server.stopped_flag();

                let dns_handle = tokio::spawn(async move {
                    if let Err(e) = dns_server.run().await {
                        tracing::error!("DNS server failed: {}", e);
                    }
                });

                if let Err(e) = self.set_system_dns().await {
                    tracing::warn!("Failed to set system DNS: {}, DNS hijack may not work", e);
                }

                let tun_config = TunProxyConfig {
                    tunnel_name: self.config.tun_name.clone(),
                };
                let tun_proxy = Arc::new(TunProxy::new(
                    tun_config,
                    endpoint_group.clone(),
                    self.ip_mapping.clone(),
                ));

                *self.instance.lock() = Some(ProxyInstance {
                    tun_proxy: Some(tun_proxy.clone()),
                    local_proxy: None,
                    dns_stopped: Some(dns_stopped),
                    dns_handle: Some(dns_handle),
                    endpoint_group: Some(endpoint_group.clone()),
                });
                *self.mode.lock() = Some(ProxyMode::Tun);

                match tun_proxy.run().await {
                    Ok(()) => {
                        tracing::info!("TUN proxy stopped successfully");
                        let _ = self.restore_system_dns().await;
                        *self.instance.lock() = None;
                        *self.mode.lock() = None;
                        return Ok(());
                    }
                    Err(e) => {
                        tracing::warn!(
                            "TUN proxy failed: {}, falling back to local proxy",
                            e
                        );
                        let _ = self.restore_system_dns().await;
                        *self.instance.lock() = None;
                    }
                }
            } else {
                tracing::warn!(
                    "Admin privileges not available, falling back to local proxy mode"
                );
            }
        }

        #[cfg(not(windows))]
        {
            tracing::warn!("TUN is only supported on Windows, using local proxy");
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
            #[cfg(windows)]
            tun_proxy: None,
            local_proxy: Some(local_proxy.clone()),
            dns_stopped: None,
            dns_handle: None,
            endpoint_group: None,
        });

        local_proxy.run().await?;

        *self.instance.lock() = None;
        *self.mode.lock() = None;
        Ok(())
    }

    pub async fn get_node_id(&self) -> Option<String> {
        None
    }

    #[cfg(windows)]
    async fn set_system_dns(&self) -> Result<()> {
        let dns_addr = self
            .config
            .dns_listen_addr
            .split(':')
            .next()
            .unwrap_or("10.0.0.1");

        let output = std::process::Command::new("netsh")
            .args([
                "interface",
                "ip",
                "set",
                "dnsservers",
                "all",
                dns_addr,
                "primary",
            ])
            .output()
            .map_err(|e| anyhow::anyhow!("Failed to run netsh: {}", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            tracing::warn!("netsh set dns failed: {}", stderr);
        } else {
            tracing::info!("System DNS set to {}", dns_addr);
        }

        Ok(())
    }

    #[cfg(windows)]
    async fn restore_system_dns(&self) -> Result<()> {
        let output = std::process::Command::new("netsh")
            .args(["interface", "ip", "set", "dnsservers", "all", "dhcp"])
            .output()
            .map_err(|e| anyhow::anyhow!("Failed to run netsh: {}", e))?;

        if output.status.success() {
            tracing::info!("System DNS restored to DHCP");
        }

        Ok(())
    }
}
