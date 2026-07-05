use anyhow::Result;
use nexapipe_client::endpoint_group::{EndpointGroup, NodeConfig};
use nexapipe_client::lb::LoadBalancingStrategy;
use nexapipe_client::LocalProxy;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct LocalProxyConfig {
    pub listen_addr: String,
    pub proxy_domains: Vec<String>,
    pub nodes: Vec<NodeConfig>,
    pub load_balancing: LoadBalancingStrategy,
}

pub struct LocalProxyWrapper {
    inner: Arc<LocalProxy>,
}

impl LocalProxyWrapper {
    pub async fn new(config: LocalProxyConfig) -> Result<Self> {
        let endpoint_group =
            EndpointGroup::new_with_nodes(config.nodes, None, config.load_balancing)
                .await
                .map_err(|e| anyhow::anyhow!(e))?;

        let inner = LocalProxy::new(&config.listen_addr, config.proxy_domains, endpoint_group)
            .await
            .map_err(|e| anyhow::anyhow!(e))?;

        tracing::info!("Local proxy created on: {}", config.listen_addr);

        Ok(Self {
            inner: Arc::new(inner),
        })
    }

    pub async fn run(&self) -> Result<()> {
        self.inner.run().await.map_err(|e| anyhow::anyhow!(e))
    }

    pub fn stop(&self) {
        tracing::info!("Stopping local proxy");
        self.inner.stop();
    }
}
