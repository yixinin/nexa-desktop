//! TUN 代理 — 跨平台实现（Windows: wintun / Linux: /dev/net/tun / macOS: utun）。
//!
//! 数据流：
//! ```text
//! APP → TUN 设备 → AsyncDevice::recv() → 解析 IP/TCP 包
//!                                            ↓
//!                         根据目标虚拟 IP 查 IpMapping 得到域名
//!                                            ↓
//!                             打开 iroh 双向流转发数据
//!                                            ↓
//!                     构造 TCP 应答包 → AsyncDevice::send() → TUN 设备 → APP
//! ```
//!
//! 虚拟网络（与 dns.rs 的 IpMapping 分配一致）：
//! - 10.0.0.1   = TUN 设备自身地址 / DNS 服务器地址
//! - 10.0.0.2+  = 代理域名映射的虚拟 IP（DNS 劫持返回）
//!
//! 系统路由：接口配置为 10.0.0.1/24 后内核自动添加 10.0.0.0/24 直连路由，
//! 虚拟 IP 的流量全部进入 TUN 设备；系统 DNS 指向 10.0.0.1（见 dns_config.rs）。
//!
//! 平台差异：
//! - Windows: tun crate 内部创建 wintun 适配器，需先加载 wintun.dll
//! - Linux:   tun crate 创建 /dev/net/tun 设备，名称任意（≤15 字符）
//! - macOS:   设备名必须是 utunN 形式，不指定名字时由系统自动分配

use crate::proxy::dns::{DnsServer, DnsServerConfig, IpMapping};
use crate::proxy::packet::{tcp_flags, Ipv4Packet, TcpPacket, IPPROTO_TCP};
use crate::proxy::{dns_config, routing};
use anyhow::Result;
use nexapipe_client::endpoint_group::EndpointGroup;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::net::Ipv4Addr;
#[cfg(windows)]
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tun::AbstractDevice;

/// TUN 设备自身 IP（同时是本地 DNS 服务器地址）
pub const TUN_IP: &str = "10.0.0.1";
pub const TUN_NETMASK: &str = "255.255.255.0";
/// 虚拟网段 — 必须与 dns.rs 的 VIRTUAL_IP_START/END（10.0.0.2 ~ 10.0.0.254）一致。
/// 仅 Linux 的 routing 模块使用，其他平台由接口地址自动生成直连路由。
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub const TUN_NETWORK: &str = "10.0.0.0/24";
const TUN_MTU: usize = 1500;
/// 读循环超时 — 用于及时响应 stop() 信号
const READ_POLL_TIMEOUT: Duration = Duration::from_millis(100);

#[derive(Debug, Clone, Copy, PartialEq)]
enum TcpState {
    SynReceived,
    Established,
    FinWait1,
    LastAck,
}

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
struct ConnectionKey {
    src_ip: u32,
    src_port: u16,
    dst_ip: u32,
    dst_port: u16,
}

struct Connection {
    state: TcpState,
    our_seq: u32,
    our_ack: u32,
    nexapipe_send: Option<iroh::endpoint::SendStream>,
    nexapipe_recv: Option<iroh::endpoint::RecvStream>,
}

#[derive(Debug, Clone)]
pub struct TunProxyConfig {
    pub tunnel_name: String,
    /// 系统 DNS 需要指向的地址（TUN 虚拟 IP，通常为 10.0.0.1）
    pub dns_ip: String,
    /// 本地 DNS 服务器配置（DNS 劫持）
    pub dns: DnsServerConfig,
}

pub struct TunProxy {
    config: TunProxyConfig,
    endpoint_group: Arc<EndpointGroup>,
    ip_mapping: Arc<IpMapping>,
    connections: Arc<Mutex<HashMap<ConnectionKey, Connection>>>,
    identification: Arc<Mutex<u16>>,
    stopped: Arc<AtomicBool>,
}

impl TunProxy {
    pub fn new(
        config: TunProxyConfig,
        endpoint_group: Arc<EndpointGroup>,
        ip_mapping: Arc<IpMapping>,
    ) -> Self {
        Self {
            config,
            endpoint_group,
            ip_mapping,
            connections: Arc::new(Mutex::new(HashMap::new())),
            identification: Arc::new(Mutex::new(0)),
            stopped: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn stop(&self) {
        tracing::info!("Stopping TUN proxy");
        self.stopped.store(true, Ordering::Release);
    }

    /// 当前进程是否有权限创建 TUN 设备（Windows: 管理员 / Unix: root）
    pub fn is_admin() -> bool {
        #[cfg(windows)]
        {
            std::process::Command::new("net")
                .arg("session")
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .map(|s| s.success())
                .unwrap_or(false)
        }
        #[cfg(unix)]
        {
            unsafe { libc::geteuid() == 0 }
        }
    }

    pub async fn is_available() -> bool {
        Self::is_admin()
    }

    /// 启动 TUN 代理：创建设备 → 配置地址/路由 → 启动本地 DNS → 切换系统 DNS
    /// → 运行包循环 → 清理恢复。
    ///
    /// 顺序至关重要：本地 DNS 服务器必须绑定在 10.0.0.1:53 上，
    /// 而该地址是 TUN 接口地址，必须先配置接口再启动 DNS 服务器，
    /// 否则 Windows 会报 WSAEADDRNOTAVAIL (10049) 绑定失败。
    pub async fn run(&self) -> Result<()> {
        tracing::info!("Starting TUN proxy with DNS hijacking");

        let device = Arc::new(self.create_device()?);
        let interface = device.tun_name()?;
        tracing::info!(
            "TUN device ready: {} (mtu {})",
            interface,
            device.mtu().unwrap_or(TUN_MTU as u16)
        );

        // 1. 配置接口地址 / 路由（幂等）——10.0.0.1 必须已存在于本机
        routing::configure_interface(&interface)?;
        routing::add_routes(&interface)?;

        // 2. 绑定本地 DNS 服务器（DNS 劫持）。
        //    必须先确认绑定成功，再把系统 DNS 指向 10.0.0.1，否则系统 DNS
        //    会指向一个没人监听的地址，导致整机 DNS（含 iroh relay 解析）挂掉。
        //    Windows 上接口地址未就绪时 bind 会报 WSAEADDRNOTAVAIL (10049)，
        //    这里直接失败退出（manager 会回退 local proxy），而不是带病运行。
        let dns_server = DnsServer::new(self.config.dns.clone(), self.ip_mapping.clone());
        let dns_socket = dns_server.bind().await.map_err(|e| {
            anyhow::anyhow!(
                "Failed to bind DNS server on {}: {}. Ensure the TUN interface {} is configured with {} and port 53 is free.",
                self.config.dns.listen_addr,
                e,
                interface,
                TUN_IP
            )
        })?;
        let dns_stopped = dns_server.stopped_flag();
        let dns_handle = tokio::spawn(async move {
            if let Err(e) = dns_server.run_with_socket(dns_socket).await {
                tracing::error!("DNS server failed: {}", e);
            }
        });

        // 3. 将系统 DNS 指向虚拟 IP — 必须在 DNS 服务器成功绑定之后
        if let Err(e) = dns_config::set_system_dns(&interface, &self.config.dns_ip) {
            tracing::warn!("Failed to set system DNS: {}, DNS hijack may not work", e);
        }

        let result = self.run_device(&device).await;

        // 清理（尽力而为）：先恢复系统 DNS，再停止本地 DNS 服务器，最后移除路由
        if let Err(e) = dns_config::restore_system_dns(&interface, &self.config.dns_ip) {
            tracing::warn!("Failed to restore system DNS: {}", e);
        }
        dns_stopped.store(true, Ordering::Release);
        if let Err(e) = dns_handle.await {
            tracing::debug!("DNS handle join error: {:?}", e);
        }
        if let Err(e) = routing::remove_routes(&interface) {
            tracing::warn!("Failed to remove TUN routes: {}", e);
        }
        result
    }

    /// 创建 TUN 设备。Windows 上需先定位 wintun.dll 并显式指定给 tun crate
    /// （不能依赖系统 DLL 搜索——wintun-bindings 的 load_from_path("wintun.dll")
    /// 在搜索失败时会误对 exe 自身做签名校验，导致 "The file is not signed."）；
    /// macOS 上不指定名字，由系统自动分配 utunN。
    fn create_device(&self) -> Result<tun::AsyncDevice> {
        #[cfg(windows)]
        let wintun_path = find_wintun_path().ok_or_else(|| {
            anyhow::anyhow!(
                "wintun.dll not found. 请确认 build.rs 已将其复制到 exe 目录，\
                 或设置 WINTUN_PATH 环境变量指向 wintun.dll"
            )
        })?;
        #[cfg(windows)]
        tracing::info!("Using wintun.dll at {}", wintun_path.display());

        let mut config = tun::Configuration::default();
        // 显式指定 wintun.dll 绝对路径：绕开 wintun-bindings 的系统搜索 + 签名校验 bug
        #[cfg(windows)]
        config.platform_config(|pc| {
            pc.wintun_file(wintun_path.clone());
        });
        #[cfg(not(target_os = "macos"))]
        config.tun_name(&self.config.tunnel_name);
        config.mtu(TUN_MTU as u16);
        // 地址/路由由 routing::configure_interface 统一配置（幂等），
        // 避免与 ip/ifconfig/netsh 命令冲突。Windows 下 tun crate 直接创建 wintun 适配器。

        let device = tun::create_as_async(&config)?;
        Ok(device)
    }

    /// 运行包处理循环：读 TUN → 解析 → 分发到 handle_packet。
    async fn run_device(&self, device: &Arc<tun::AsyncDevice>) -> Result<()> {
        let mut buf = vec![0u8; TUN_MTU];
        loop {
            if self.stopped.load(Ordering::Acquire) {
                tracing::info!("TUN stopping due to stop flag");
                break;
            }
            match tokio::time::timeout(READ_POLL_TIMEOUT, device.recv(&mut buf)).await {
                Ok(Ok(n)) => {
                    if n == 0 {
                        break;
                    }
                    let data = buf[..n].to_vec();
                    let device = device.clone();
                    let connections = self.connections.clone();
                    let endpoint_group = self.endpoint_group.clone();
                    let ip_mapping = self.ip_mapping.clone();
                    let identification = self.identification.clone();
                    tokio::spawn(async move {
                        if let Err(e) = handle_packet(
                            &data,
                            &device,
                            &connections,
                            &endpoint_group,
                            &ip_mapping,
                            &identification,
                        )
                        .await
                        {
                            tracing::debug!("Packet handling error: {}", e);
                        }
                    });
                }
                Ok(Err(e)) => {
                    tracing::error!("TUN receive error: {}", e);
                    break;
                }
                Err(_) => continue, // 超时，回到循环检查 stop 标志
            }
        }
        tracing::info!("TUN proxy stopped");
        Ok(())
    }
}

async fn handle_packet(
    data: &[u8],
    device: &Arc<tun::AsyncDevice>,
    connections: &Arc<Mutex<HashMap<ConnectionKey, Connection>>>,
    endpoint_group: &Arc<EndpointGroup>,
    ip_mapping: &Arc<IpMapping>,
    identification: &Arc<Mutex<u16>>,
) -> Result<()> {
    let ip_packet = match Ipv4Packet::parse(data) {
        Some(p) => p,
        None => return Ok(()),
    };
    if ip_packet.protocol != IPPROTO_TCP {
        return Ok(());
    }
    let tcp_packet = match TcpPacket::parse(&ip_packet.payload) {
        Some(p) => p,
        None => return Ok(()),
    };
    let domain = match ip_mapping.lookup_domain(&ip_packet.dst_addr) {
        Some(d) => d,
        None => return Ok(()),
    };
    let conn_key = ConnectionKey {
        src_ip: u32::from(ip_packet.src_addr),
        src_port: tcp_packet.src_port,
        dst_ip: u32::from(ip_packet.dst_addr),
        dst_port: tcp_packet.dst_port,
    };
    handle_tcp(
        &conn_key,
        &tcp_packet,
        &ip_packet,
        &domain,
        device,
        connections,
        endpoint_group,
        identification,
    )
    .await
}

async fn handle_tcp(
    conn_key: &ConnectionKey,
    tcp: &TcpPacket,
    ip: &Ipv4Packet,
    domain: &str,
    device: &Arc<tun::AsyncDevice>,
    connections: &Arc<Mutex<HashMap<ConnectionKey, Connection>>>,
    endpoint_group: &Arc<EndpointGroup>,
    identification: &Arc<Mutex<u16>>,
) -> Result<()> {
    let is_syn = (tcp.flags & tcp_flags::SYN) != 0;
    let is_ack = (tcp.flags & tcp_flags::ACK) != 0;
    let is_fin = (tcp.flags & tcp_flags::FIN) != 0;
    let is_rst = (tcp.flags & tcp_flags::RST) != 0;
    let has_data = !tcp.payload.is_empty();

    if is_rst {
        connections.lock().remove(conn_key);
        return Ok(());
    }

    if is_syn && !is_ack {
        return handle_syn(
            conn_key,
            tcp,
            ip,
            domain,
            device,
            connections,
            endpoint_group,
            identification,
        )
        .await;
    }

    let conn_exists = connections.lock().contains_key(conn_key);
    if !conn_exists {
        return Ok(());
    }

    let conn_state = {
        let conns = connections.lock();
        conns.get(conn_key).map(|c| (c.state, c.our_seq, c.our_ack))
    };
    let (state, our_seq, _our_ack) = match conn_state {
        Some(s) => s,
        None => return Ok(()),
    };

    match state {
        TcpState::SynReceived => {
            if is_ack && !has_data {
                let mut conns = connections.lock();
                if let Some(conn) = conns.get_mut(conn_key) {
                    conn.state = TcpState::Established;
                }
            }
        }
        TcpState::Established => {
            if has_data {
                let mut send_stream = {
                    let mut conns = connections.lock();
                    conns.get_mut(conn_key).and_then(|c| c.nexapipe_send.take())
                };
                if let Some(ref mut send) = send_stream {
                    if let Err(_e) = send.write_all(&tcp.payload).await {
                        connections.lock().remove(conn_key);
                        return Ok(());
                    }
                }
                {
                    let mut conns = connections.lock();
                    if let Some(conn) = conns.get_mut(conn_key) {
                        conn.nexapipe_send = send_stream;
                    }
                }
                let new_ack = tcp.seq_num.wrapping_add(tcp.payload.len() as u32);
                let ack_packet = TcpPacket {
                    src_port: tcp.dst_port,
                    dst_port: tcp.src_port,
                    seq_num: our_seq,
                    ack_num: new_ack,
                    flags: tcp_flags::ACK,
                    window: 65535,
                    payload: Vec::new(),
                };
                let id = get_next_id(identification);
                let packet = ack_packet.build_with_ip(ip.dst_addr, ip.src_addr, id);
                send_tun_packet(device, &packet).await?;
                {
                    let mut conns = connections.lock();
                    if let Some(conn) = conns.get_mut(conn_key) {
                        conn.our_ack = new_ack;
                    }
                }
                spawn_recv_task(
                    conn_key.clone(),
                    connections.clone(),
                    ip.dst_addr,
                    ip.src_addr,
                    tcp.dst_port,
                    tcp.src_port,
                    identification.clone(),
                    device.clone(),
                    our_seq,
                    new_ack,
                );
            }
            if is_fin {
                let fin_ack = tcp.seq_num.wrapping_add(tcp.payload.len() as u32 + 1);
                let packet = TcpPacket {
                    src_port: tcp.dst_port,
                    dst_port: tcp.src_port,
                    seq_num: our_seq,
                    ack_num: fin_ack,
                    flags: tcp_flags::FIN | tcp_flags::ACK,
                    window: 65535,
                    payload: Vec::new(),
                };
                let id = get_next_id(identification);
                let data = packet.build_with_ip(ip.dst_addr, ip.src_addr, id);
                send_tun_packet(device, &data).await?;
                {
                    let mut conns = connections.lock();
                    if let Some(conn) = conns.get_mut(conn_key) {
                        conn.state = TcpState::LastAck;
                        conn.our_ack = fin_ack;
                    }
                }
            }
        }
        TcpState::LastAck => {
            if is_ack {
                connections.lock().remove(conn_key);
            }
        }
        _ => {}
    }
    Ok(())
}

async fn handle_syn(
    conn_key: &ConnectionKey,
    tcp: &TcpPacket,
    ip: &Ipv4Packet,
    domain: &str,
    device: &Arc<tun::AsyncDevice>,
    connections: &Arc<Mutex<HashMap<ConnectionKey, Connection>>>,
    endpoint_group: &Arc<EndpointGroup>,
    identification: &Arc<Mutex<u16>>,
) -> Result<()> {
    let pooled_conn = match endpoint_group.get_connection(domain).await {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("Failed to get nexapipe connection: {}", e);
            let rst_packet = TcpPacket {
                src_port: tcp.dst_port,
                dst_port: tcp.src_port,
                seq_num: 0,
                ack_num: tcp.seq_num.wrapping_add(1),
                flags: tcp_flags::RST | tcp_flags::ACK,
                window: 0,
                payload: Vec::new(),
            };
            let id = get_next_id(identification);
            let data = rst_packet.build_with_ip(ip.dst_addr, ip.src_addr, id);
            let _ = send_tun_packet(device, &data).await;
            return Ok(());
        }
    };

    let conn = pooled_conn.into_inner();

    let (mut send, recv) = match conn.open_bi().await {
        Ok(streams) => streams,
        Err(e) => {
            tracing::error!("Failed to open nexapipe stream: {}", e);
            let rst_packet = TcpPacket {
                src_port: tcp.dst_port,
                dst_port: tcp.src_port,
                seq_num: 0,
                ack_num: tcp.seq_num.wrapping_add(1),
                flags: tcp_flags::RST | tcp_flags::ACK,
                window: 0,
                payload: Vec::new(),
            };
            let id = get_next_id(identification);
            let data = rst_packet.build_with_ip(ip.dst_addr, ip.src_addr, id);
            let _ = send_tun_packet(device, &data).await;
            return Ok(());
        }
    };

    let connect_request = format!(
        "CONNECT {}:{} HTTP/1.1\r\nHost: {}:{}\r\n\r\n",
        domain, tcp.dst_port, domain, tcp.dst_port
    );
    if let Err(e) = send.write_all(connect_request.as_bytes()).await {
        tracing::error!("Failed to send CONNECT request: {}", e);
        return Ok(());
    }

    let our_seq = 1000u32;
    let our_ack = tcp.seq_num.wrapping_add(1);
    let syn_ack = TcpPacket {
        src_port: tcp.dst_port,
        dst_port: tcp.src_port,
        seq_num: our_seq,
        ack_num: our_ack,
        flags: tcp_flags::SYN | tcp_flags::ACK,
        window: 65535,
        payload: Vec::new(),
    };
    let id = get_next_id(identification);
    let data = syn_ack.build_with_ip(ip.dst_addr, ip.src_addr, id);
    send_tun_packet(device, &data).await?;

    let conn_data = Connection {
        state: TcpState::SynReceived,
        our_seq: our_seq.wrapping_add(1),
        our_ack,
        nexapipe_send: Some(send),
        nexapipe_recv: Some(recv),
    };
    connections.lock().insert(*conn_key, conn_data);
    Ok(())
}

fn spawn_recv_task(
    conn_key: ConnectionKey,
    connections: Arc<Mutex<HashMap<ConnectionKey, Connection>>>,
    tun_ip: Ipv4Addr,
    client_ip: Ipv4Addr,
    tun_port: u16,
    client_port: u16,
    identification: Arc<Mutex<u16>>,
    device: Arc<tun::AsyncDevice>,
    initial_seq: u32,
    initial_ack: u32,
) {
    let recv = {
        let mut conns = connections.lock();
        match conns.get_mut(&conn_key) {
            Some(conn) => conn.nexapipe_recv.take(),
            None => return,
        }
    };
    if recv.is_none() {
        return;
    }
    let mut recv = recv.unwrap();
    let mut our_seq = initial_seq;

    tokio::spawn(async move {
        let mut buf = vec![0u8; 8192];
        loop {
            match recv.read(&mut buf).await {
                Ok(None) => break,
                Ok(Some(n)) => {
                    let data = &buf[..n];
                    let tcp_packet = TcpPacket {
                        src_port: tun_port,
                        dst_port: client_port,
                        seq_num: our_seq,
                        ack_num: initial_ack,
                        flags: tcp_flags::PSH | tcp_flags::ACK,
                        window: 65535,
                        payload: data.to_vec(),
                    };
                    let id = get_next_id(&identification);
                    let packet = tcp_packet.build_with_ip(tun_ip, client_ip, id);
                    if let Err(_e) = send_tun_packet(&device, &packet).await {
                        break;
                    }
                    our_seq = our_seq.wrapping_add(n as u32);
                }
                Err(_e) => break,
            }
        }
        let fin_packet = TcpPacket {
            src_port: tun_port,
            dst_port: client_port,
            seq_num: our_seq,
            ack_num: initial_ack,
            flags: tcp_flags::FIN | tcp_flags::ACK,
            window: 65535,
            payload: Vec::new(),
        };
        let id = get_next_id(&identification);
        let packet = fin_packet.build_with_ip(tun_ip, client_ip, id);
        let _ = send_tun_packet(&device, &packet).await;
        let mut conns = connections.lock();
        if let Some(conn) = conns.get_mut(&conn_key) {
            conn.state = TcpState::FinWait1;
        }
    });
}

async fn send_tun_packet(device: &Arc<tun::AsyncDevice>, data: &[u8]) -> Result<()> {
    device
        .send(data)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to send TUN packet: {}", e))?;
    Ok(())
}

fn get_next_id(identification: &Arc<Mutex<u16>>) -> u16 {
    let mut id = identification.lock();
    let current = *id;
    *id = id.wrapping_add(1);
    current
}

/// Windows: 定位 wintun.dll 的绝对路径（只查找，不加载）。
/// 按优先级：编译期注入的 WINTUN_PATH（build.rs 复制到 exe 目录的位置）
/// → exe 同目录 → exe/wintun/bin/{arch} → 打包资源目录 → 运行时 WINTUN_PATH。
#[cfg(windows)]
fn find_wintun_path() -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(p) = option_env!("WINTUN_PATH") {
        candidates.push(PathBuf::from(p));
    }
    if let Some(exe_dir) = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
    {
        let arch_dir = target_arch_dir();
        candidates.push(exe_dir.join("wintun.dll"));
        candidates.push(
            exe_dir
                .join("wintun")
                .join("bin")
                .join(arch_dir)
                .join("wintun.dll"),
        );
        candidates.push(
            exe_dir
                .join("resources")
                .join("wintun")
                .join("bin")
                .join(arch_dir)
                .join("wintun.dll"),
        );
    }
    if let Ok(path) = std::env::var("WINTUN_PATH") {
        candidates.push(PathBuf::from(path));
    }
    candidates.into_iter().find(|p| p.exists())
}

#[cfg(windows)]
fn target_arch_dir() -> &'static str {
    #[cfg(target_arch = "x86_64")]
    {
        "amd64"
    }
    #[cfg(target_arch = "x86")]
    {
        "x86"
    }
    #[cfg(target_arch = "arm")]
    {
        "arm"
    }
    #[cfg(target_arch = "aarch64")]
    {
        "arm64"
    }
    #[cfg(not(any(
        target_arch = "x86_64",
        target_arch = "x86",
        target_arch = "arm",
        target_arch = "aarch64"
    )))]
    {
        "amd64"
    }
}
