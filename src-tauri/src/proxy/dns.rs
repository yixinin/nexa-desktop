use anyhow::Result;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::net::UdpSocket;

/// 虚拟 IP 起始地址（10.0.0.2 ~ 10.0.0.254）
const VIRTUAL_IP_START: u32 = 0x0A000002; // 10.0.0.2
const VIRTUAL_IP_END: u32 = 0x0A0000FE; // 10.0.0.254

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

    /// 绑定 DNS 监听 socket（eager bind）。
    ///
    /// 调用方必须先确认绑定成功，再把系统 DNS 指向 listen_addr，否则系统 DNS
    /// 会指向一个没人监听的地址，导致整机 DNS 解析失败（iroh relay 也会随之断连）。
    /// Windows 上若 TUN 接口地址（10.0.0.1）尚未就绪，这里会返回
    /// WSAEADDRNOTAVAIL (10049)。
    pub async fn bind(&self) -> Result<Arc<UdpSocket>> {
        let socket = Arc::new(UdpSocket::bind(&self.config.listen_addr).await?);
        tracing::info!("DNS server listening on: {}", self.config.listen_addr);
        Ok(socket)
    }

    /// 使用已绑定的 socket 运行 DNS 服务器主循环。
    pub async fn run_with_socket(&self, socket: Arc<UdpSocket>) -> Result<()> {
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
            )
            .await
            {
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

    tracing::debug!("DNS query: {} (type {})", query.domain, query.qtype);

    // 检查域名是否在代理列表中（支持子域后缀匹配）
    if should_proxy_domain(&query.domain, proxy_domains) {
        let virtual_ip = ip_mapping.allocate(&query.domain);
        tracing::info!("DNS hijack: {} -> {}", query.domain, virtual_ip);

        // A 记录返回虚拟 IP；AAAA 等其余类型返回 NOERROR 空应答，
        // 避免向上游泄漏查询且不干扰浏览器解析
        let response = if query.qtype == 1 {
            build_dns_response(data, &query, virtual_ip)
        } else {
            build_empty_dns_response(&query)
        };
        socket.send_to(&response, client_addr).await?;
        return Ok(());
    }

    // 非代理域名：转发到上游 DNS（配置的上游不可达时自动兜底其他公共 DNS）
    forward_to_upstream(socket, data, client_addr, upstream).await
}

/// 兜底上游 DNS 列表（当用户配置的上游超时/不可达时依次尝试）。
/// 默认 8.8.8.8 在部分网络（如国内直连）不可达，会导致 iroh relay 域名
/// 解析失败（"No addressing information available"），故按序兜底到国内可达的公共 DNS。
const FALLBACK_UPSTREAMS: &[&str] = &[
    "223.5.5.5:53",       // 阿里 DNS
    "114.114.114.114:53", // 114 DNS
    "1.1.1.1:53",         // Cloudflare
];

/// 依次向配置的上游与兜底上游转发 DNS 查询，返回第一个成功应答；
/// 每个上游等待 1.5s，且只接受来自所查询上游的响应（防伪造）。
async fn forward_to_upstream(
    socket: &Arc<UdpSocket>,
    data: &[u8],
    client_addr: std::net::SocketAddr,
    configured_upstream: &str,
) -> Result<()> {
    // 去重构建上游列表：用户配置优先，兜底随后
    let mut upstreams: Vec<String> = Vec::new();
    if !upstreams.iter().any(|u| u == configured_upstream) {
        upstreams.push(configured_upstream.to_string());
    }
    for fb in FALLBACK_UPSTREAMS {
        if !upstreams.iter().any(|u| u == fb) {
            upstreams.push(fb.to_string());
        }
    }

    for upstream in upstreams {
        let upstream_addr: std::net::SocketAddr = match upstream.parse() {
            Ok(a) => a,
            Err(_) => continue,
        };
        let upstream_socket = match UdpSocket::bind("0.0.0.0:0").await {
            Ok(s) => s,
            Err(_) => continue,
        };
        if upstream_socket.send_to(data, upstream_addr).await.is_err() {
            continue;
        }
        let mut buf = [0u8; 4096];
        match tokio::time::timeout(
            tokio::time::Duration::from_millis(1500),
            upstream_socket.recv_from(&mut buf),
        )
        .await
        {
            // 只接受来自所查询上游的响应，防止伪造/无关 UDP 包被转发给客户端
            Ok(Ok((len, src))) if src == upstream_addr => {
                socket.send_to(&buf[..len], client_addr).await?;
                return Ok(());
            }
            _ => continue, // 超时/来源不符/错误：尝试下一个上游
        }
    }

    tracing::debug!("All upstream DNS servers failed for query");
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

/// 检查域名是否应该被代理（DOMAIN-SUFFIX 语义）：
/// - `example.com` 匹配 `example.com` 及其所有子域名（如 `fn.example.com`）
/// - `*.example.com` / `.example.com` 兼容写法，语义相同
fn should_proxy_domain(host: &str, proxy_domains: &[String]) -> bool {
    let host_lower = host.to_lowercase();
    for domain in proxy_domains {
        let domain_lower = domain
            .trim_start_matches('*')
            .trim_start_matches('.')
            .to_lowercase();
        if host_lower == domain_lower || host_lower.ends_with(&format!(".{}", domain_lower)) {
            return true;
        }
    }
    false
}

/// 构造 NOERROR 空应答（用于 AAAA 等劫持域名的非 A 记录查询）
fn build_empty_dns_response(dns_query: &DnsQuery) -> Vec<u8> {
    let mut response = Vec::with_capacity(dns_query.question_section.len() + 16);
    // Header：ID + QR=1, RD=1, RA=1, RCODE=0 + QDCOUNT=1, ANCOUNT=0, NSCOUNT=0, ARCOUNT=0
    response.extend_from_slice(&dns_query.id.to_be_bytes());
    response.extend_from_slice(&[0x81, 0x80]);
    response.extend_from_slice(&1u16.to_be_bytes());
    response.extend_from_slice(&0u16.to_be_bytes());
    response.extend_from_slice(&0u16.to_be_bytes());
    response.extend_from_slice(&0u16.to_be_bytes());
    // Question section（原样复制）
    response.extend_from_slice(&dns_query.question_section);
    response
}
