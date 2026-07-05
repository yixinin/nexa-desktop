use anyhow::Result;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::net::UdpSocket;

/// 虚拟 IP 起始地址（10.0.0.2 ~ 10.0.0.254）
const VIRTUAL_IP_START: u32 = 0x0A000002; // 10.0.0.2
const VIRTUAL_IP_END: u32 = 0x0A0000FE;   // 10.0.0.254

/// IP -> 域名 映射表
#[derive(Debug)]
pub struct IpMapping {
    ip_to_domain: Mutex<HashMap<u32, String>>,
    domain_to_ip: Mutex<HashMap<String, u32>>,
    next_ip: Mutex<u32>,
}

impl IpMapping {
    pub fn new() -> Self {
        Self {
            ip_to_domain: Mutex::new(HashMap::new()),
            domain_to_ip: Mutex::new(HashMap::new()),
            next_ip: Mutex::new(VIRTUAL_IP_START),
        }
    }

    /// 为域名分配虚拟 IP（如果已分配则返回已有的 IP）
    pub fn allocate(&self, domain: &str) -> Ipv4Addr {
        let domain_lower = domain.to_lowercase();

        // 检查是否已分配
        {
            let domain_to_ip = self.domain_to_ip.lock();
            if let Some(&ip) = domain_to_ip.get(&domain_lower) {
                return Ipv4Addr::from(ip);
            }
        }

        // 分配新 IP
        let mut next_ip = self.next_ip.lock();
        let ip = *next_ip;

        // IP 用尽则回绕
        if *next_ip >= VIRTUAL_IP_END {
            *next_ip = VIRTUAL_IP_START;
        } else {
            *next_ip += 1;
        }

        self.ip_to_domain.lock().insert(ip, domain_lower.clone());
        self.domain_to_ip.lock().insert(domain_lower, ip);

        Ipv4Addr::from(ip)
    }

    /// 通过 IP 查找域名
    pub fn lookup_domain(&self, ip: &Ipv4Addr) -> Option<String> {
        let ip_u32 = u32::from(*ip);
        self.ip_to_domain.lock().get(&ip_u32).cloned()
    }

}

/// DNS 服务器配置
#[derive(Debug, Clone)]
pub struct DnsServerConfig {
    pub listen_addr: String,
    pub upstream_dns: String,
    pub proxy_domains: Vec<String>,
}

/// DNS 服务器 - 实现 DNS 劫持
pub struct DnsServer {
    config: DnsServerConfig,
    ip_mapping: Arc<IpMapping>,
    stopped: Arc<AtomicBool>,
}

impl DnsServer {
    pub fn new(config: DnsServerConfig, ip_mapping: Arc<IpMapping>) -> Self {
        Self {
            config,
            ip_mapping,
            stopped: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn stopped_flag(&self) -> Arc<AtomicBool> {
        self.stopped.clone()
    }

    /// 启动 DNS 服务器
    pub async fn run(&self) -> Result<()> {
        let socket = Arc::new(UdpSocket::bind(&self.config.listen_addr).await?);
        tracing::info!("DNS server listening on: {}", self.config.listen_addr);

        let upstream = self.config.upstream_dns.clone();
        let proxy_domains = Arc::new(self.config.proxy_domains.clone());
        let ip_mapping = self.ip_mapping.clone();
        let stopped = self.stopped.clone();

        loop {
            if stopped.load(Ordering::Acquire) {
                tracing::info!("DNS server stopping");
                break;
            }

            let mut buf = [0u8; 4096];
            match tokio::time::timeout(
                tokio::time::Duration::from_millis(100),
                socket.recv_from(&mut buf),
            ).await {
                Ok(Ok((len, addr))) => {
                    let data = buf[..len].to_vec();
                    let socket = socket.clone();
                    let upstream = upstream.clone();
                    let proxy_domains = proxy_domains.clone();
                    let ip_mapping = ip_mapping.clone();

                    tokio::spawn(async move {
                        if let Err(e) = handle_dns_query(
                            &socket,
                            &data,
                            addr,
                            &upstream,
                            &proxy_domains,
                            &ip_mapping,
                        )
                        .await
                        {
                            tracing::debug!("DNS query handler error: {}", e);
                        }
                    });
                }
                Ok(Err(e)) => {
                    tracing::error!("DNS recv error: {}", e);
                }
                Err(_) => {
                    continue;
                }
            }
        }
        Ok(())
    }
}

/// 处理 DNS 查询
async fn handle_dns_query(
    socket: &Arc<UdpSocket>,
    data: &[u8],
    client_addr: std::net::SocketAddr,
    upstream: &str,
    proxy_domains: &[String],
    ip_mapping: &IpMapping,
) -> Result<()> {
    if data.len() < 12 {
        return Ok(());
    }

    // 解析 DNS 查询
    let query = match parse_dns_query(data) {
        Ok(q) => q,
        Err(e) => {
            tracing::debug!("Failed to parse DNS query: {}", e);
            return Ok(());
        }
    };

    tracing::debug!(
        "DNS query: {} (type {})",
        query.domain,
        query.qtype
    );

    // 检查域名是否在代理列表中
    if query.qtype == 1 && should_proxy_domain(&query.domain, proxy_domains) {
        // DNS 劫持：返回虚拟 IP
        let virtual_ip = ip_mapping.allocate(&query.domain);
        tracing::info!("DNS hijack: {} -> {}", query.domain, virtual_ip);

        let response = build_dns_response(data, &query, virtual_ip);
        socket.send_to(&response, client_addr).await?;
        return Ok(());
    }

    // 非代理域名：转发到上游 DNS
    let upstream_socket = UdpSocket::bind("0.0.0.0:0").await?;
    upstream_socket.send_to(data, upstream).await?;

    let mut buf = [0u8; 4096];
    match tokio::time::timeout(
        tokio::time::Duration::from_secs(5),
        upstream_socket.recv_from(&mut buf),
    )
    .await
    {
        Ok(Ok((len, _))) => {
            socket.send_to(&buf[..len], client_addr).await?;
        }
        Ok(Err(e)) => {
            tracing::debug!("Upstream DNS error: {}", e);
        }
        Err(_) => {
            tracing::debug!("Upstream DNS timeout");
        }
    }

    Ok(())
}

/// DNS 查询结构
#[derive(Debug)]
struct DnsQuery {
    id: u16,
    domain: String,
    qtype: u16,
    question_section: Vec<u8>,
}

/// 解析 DNS 查询
fn parse_dns_query(data: &[u8]) -> Result<DnsQuery> {
    if data.len() < 12 {
        return Err(anyhow::anyhow!("DNS packet too short"));
    }

    let id = u16::from_be_bytes([data[0], data[1]]);
    let qdcount = u16::from_be_bytes([data[4], data[5]]);

    if qdcount == 0 {
        return Err(anyhow::anyhow!("No question in DNS query"));
    }

    let mut offset = 12;
    let mut labels = Vec::new();
    let question_start = offset;

    loop {
        if offset >= data.len() {
            return Err(anyhow::anyhow!("DNS packet truncated"));
        }

        let label_len = data[offset] as usize;
        if label_len == 0 {
            offset += 1;
            break;
        }

        if offset + 1 + label_len > data.len() {
            return Err(anyhow::anyhow!("DNS label out of bounds"));
        }

        let label = std::str::from_utf8(&data[offset + 1..offset + 1 + label_len])
            .map_err(|_| anyhow::anyhow!("Invalid DNS label"))?;
        labels.push(label.to_string());
        offset += 1 + label_len;
    }

    if offset + 4 > data.len() {
        return Err(anyhow::anyhow!("DNS query type/class truncated"));
    }

    let qtype = u16::from_be_bytes([data[offset], data[offset + 1]]);
    let question_end = offset + 4;

    Ok(DnsQuery {
        id,
        domain: labels.join("."),
        qtype,
        question_section: data[question_start..question_end].to_vec(),
    })
}

/// 构造 DNS 响应（A 记录）
fn build_dns_response(query: &[u8], dns_query: &DnsQuery, ip: Ipv4Addr) -> Vec<u8> {
    let mut response = Vec::with_capacity(query.len() + 16);

    // Header
    response.extend_from_slice(&dns_query.id.to_be_bytes()); // ID
    // Flags: QR=1, Opcode=0, AA=0, TC=0, RD=1, RA=1, Z=0, RCODE=0
    response.extend_from_slice(&[0x81, 0x80]);
    // QDCOUNT=1
    response.extend_from_slice(&1u16.to_be_bytes());
    // ANCOUNT=1
    response.extend_from_slice(&1u16.to_be_bytes());
    // NSCOUNT=0
    response.extend_from_slice(&0u16.to_be_bytes());
    // ARCOUNT=0
    response.extend_from_slice(&0u16.to_be_bytes());

    // Question section（直接复制原始查询的问题部分）
    response.extend_from_slice(&dns_query.question_section);

    // Answer section
    // 名称指针（指向问题部分的域名）
    response.extend_from_slice(&[0xC0, 0x0C]);
    // TYPE=A
    response.extend_from_slice(&1u16.to_be_bytes());
    // CLASS=IN
    response.extend_from_slice(&1u16.to_be_bytes());
    // TTL=60
    response.extend_from_slice(&60u32.to_be_bytes());
    // RDLENGTH=4
    response.extend_from_slice(&4u16.to_be_bytes());
    // RDATA=IP
    response.extend_from_slice(&ip.octets());

    response
}

/// 检查域名是否应该被代理
fn should_proxy_domain(host: &str, proxy_domains: &[String]) -> bool {
    let host_lower = host.to_lowercase();
    for domain in proxy_domains {
        if domain.starts_with('*') {
            let suffix = &domain[1..];
            if host_lower.ends_with(suffix) {
                return true;
            }
        } else if host_lower == domain.to_lowercase() {
            return true;
        }
    }
    false
}
