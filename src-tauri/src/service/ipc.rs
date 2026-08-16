use serde::{Deserialize, Serialize};

pub const IPC_SOCKET_PATH: &str = "127.0.0.1:12345";

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct NodeInput {
    pub connection_type: String,
    pub ticket: String,
    pub endpoint_id: String,
    pub domains: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct StartProxyRequest {
    pub nodes: Vec<NodeInput>,
    pub domains: Vec<String>,
    pub local_addr: Option<String>,
    pub dns_addr: Option<String>,
    pub upstream_dns: Option<String>,
    pub load_balancing: Option<String>,
    pub tun_name: Option<String>,
    pub relay_mode: Option<String>,
    pub relay_url: Option<String>,
    pub force_relay: Option<bool>,
    pub two_factor_enabled: Option<bool>,
    pub two_factor_client_id: Option<String>,
    pub two_factor_secret: Option<String>,
    pub two_factor_algorithm: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum IpcMessage {
    StartProxy(StartProxyRequest),
    StopProxy,
    GetStatus,
    GetNodeId,
    InstallService,
    UninstallService,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum IpcResponse {
    Ok(String),
    Error(String),
    Status { running: bool, mode: String },
    NodeId(String),
}
