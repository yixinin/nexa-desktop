use crate::service::ipc::{IpcMessage, IpcResponse, IPC_SOCKET_PATH, StartProxyRequest, NodeInput};
use anyhow::Result;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;

pub struct IpcClient;

impl IpcClient {
    pub async fn send_message(msg: IpcMessage) -> Result<IpcResponse> {
        tracing::debug!("Connecting to service on: {}", IPC_SOCKET_PATH);

        let mut stream = TcpStream::connect(IPC_SOCKET_PATH)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to connect to service: {}", e))?;

        tracing::debug!("Connected to service, sending message");

        let msg_str = serde_json::to_string(&msg)?;

        stream
            .write_all(msg_str.as_bytes())
            .await
            .map_err(|e| anyhow::anyhow!("Failed to send message: {}", e))?;

        stream.flush().await?;

        tracing::debug!("Waiting for response...");
        let mut reader = BufReader::new(stream);
        let mut buffer = String::new();

        reader
            .read_line(&mut buffer)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to read response: {}", e))?;

        let response: IpcResponse = serde_json::from_str(&buffer)
            .map_err(|e| anyhow::anyhow!("Failed to parse response: {}", e))?;

        tracing::debug!("Received response: {:?}", response);
        Ok(response)
    }

    pub async fn start_proxy(
        nodes: Vec<NodeInput>,
        domains: Vec<String>,
        local_addr: Option<String>,
        dns_addr: Option<String>,
        upstream_dns: Option<String>,
        load_balancing: Option<String>,
    ) -> Result<String> {
        let response = Self::send_message(IpcMessage::StartProxy(StartProxyRequest {
            nodes,
            domains,
            local_addr,
            dns_addr,
            upstream_dns,
            load_balancing,
        })).await?;

        match response {
            IpcResponse::Ok(msg) => Ok(msg),
            IpcResponse::Error(e) => Err(anyhow::anyhow!("{}", e)),
            _ => Err(anyhow::anyhow!("Unexpected response")),
        }
    }

    pub async fn stop_proxy() -> Result<String> {
        let response = Self::send_message(IpcMessage::StopProxy).await?;

        match response {
            IpcResponse::Ok(msg) => Ok(msg),
            IpcResponse::Error(e) => Err(anyhow::anyhow!("{}", e)),
            _ => Err(anyhow::anyhow!("Unexpected response")),
        }
    }

    pub async fn get_status() -> Result<(bool, String)> {
        let response = Self::send_message(IpcMessage::GetStatus).await?;

        match response {
            IpcResponse::Status { running, mode } => Ok((running, mode)),
            IpcResponse::Error(e) => Err(anyhow::anyhow!("{}", e)),
            _ => Err(anyhow::anyhow!("Unexpected response")),
        }
    }

    pub async fn get_node_id() -> Result<String> {
        let response = Self::send_message(IpcMessage::GetNodeId).await?;

        match response {
            IpcResponse::NodeId(id) => Ok(id),
            IpcResponse::Error(e) => Err(anyhow::anyhow!("{}", e)),
            _ => Err(anyhow::anyhow!("Unexpected response")),
        }
    }

    pub async fn install_service() -> Result<String> {
        let response = Self::send_message(IpcMessage::InstallService).await?;

        match response {
            IpcResponse::Ok(msg) => Ok(msg),
            IpcResponse::Error(e) => Err(anyhow::anyhow!("{}", e)),
            _ => Err(anyhow::anyhow!("Unexpected response")),
        }
    }

    pub async fn uninstall_service() -> Result<String> {
        let response = Self::send_message(IpcMessage::UninstallService).await?;

        match response {
            IpcResponse::Ok(msg) => Ok(msg),
            IpcResponse::Error(e) => Err(anyhow::anyhow!("{}", e)),
            _ => Err(anyhow::anyhow!("Unexpected response")),
        }
    }

    pub async fn is_service_running() -> bool {
        Self::get_status().await.is_ok()
    }
}
