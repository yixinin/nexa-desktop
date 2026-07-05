use crate::proxy::dns::IpMapping;
use crate::proxy::packet::{tcp_flags, Ipv4Packet, TcpPacket, IPPROTO_TCP};
use anyhow::Result;
use nexapipe_client::endpoint_group::EndpointGroup;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

fn target_arch_dir() -> &'static str {
    #[cfg(target_arch = "x86_64")]
    { "amd64" }
    #[cfg(target_arch = "x86")]
    { "x86" }
    #[cfg(target_arch = "arm")]
    { "arm" }
    #[cfg(target_arch = "aarch64")]
    { "arm64" }
    #[cfg(not(any(target_arch = "x86_64", target_arch = "x86", target_arch = "arm", target_arch = "aarch64")))]
    { "amd64" }
}

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

    pub fn is_admin() -> bool {
        std::process::Command::new("net")
            .arg("session")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    pub async fn is_available() -> bool {
        Self::is_admin()
    }

    pub async fn run(&self) -> Result<()> {
        tracing::info!("Starting TUN proxy with DNS hijacking");
        #[cfg(windows)]
        { self.run_wintun().await }
        #[cfg(not(windows))]
        { Err(anyhow::anyhow!("TUN is only supported on Windows")) }
    }
}

#[cfg(windows)]
impl TunProxy {
    async fn run_wintun(&self) -> Result<()> {
        tracing::info!("Loading WinTUN driver...");
        let arch_dir = target_arch_dir();
        let wintun_path = if let Ok(path) = std::env::var("WINTUN_PATH") {
            path
        } else {
            format!("wintun/bin/{}/wintun.dll", arch_dir)
        };
        let wintun = unsafe { wintun::load_from_path(&wintun_path) }
            .map_err(|e| anyhow::anyhow!(
                "Failed to load wintun.dll from {}: {:?}", wintun_path, e
            ))?;
        let adapter = wintun::Adapter::create(&wintun, &self.config.tunnel_name, "pipe-ui", None)
            .map_err(|e| anyhow::anyhow!("Failed to create TUN adapter: {:?}", e))?;
        self.configure_interface().await?;
        let session = adapter.start_session(0x400000)
            .map_err(|e| anyhow::anyhow!("Failed to start TUN session: {:?}", e))?;
        let session_arc = Arc::new(session);
        let session_rx = session_arc.clone();
        let connections_rx = self.connections.clone();
        let endpoint_group_rx = self.endpoint_group.clone();
        let ip_mapping_rx = self.ip_mapping.clone();
        let identification_rx = self.identification.clone();
        let stopped_rx = self.stopped.clone();
        let rx_handle = tokio::task::spawn_blocking(move || {
            loop {
                if stopped_rx.load(Ordering::Acquire) {
                    tracing::info!("TUN stopping due to stop flag");
                    break;
                }
                match session_rx.receive_blocking() {
                    Ok(packet_data) => {
                        let data = packet_data.bytes().to_vec();
                        let session = session_rx.clone();
                        let connections = connections_rx.clone();
                        let endpoint_group = endpoint_group_rx.clone();
                        let ip_mapping = ip_mapping_rx.clone();
                        let identification = identification_rx.clone();
                        tokio::spawn(async move {
                            if let Err(e) = handle_packet(&data, &session, &connections, &endpoint_group, &ip_mapping, &identification).await {
                                tracing::debug!("Packet handling error: {}", e);
                            }
                        });
                    }
                    Err(wintun::Error::ShuttingDown) => { break; }
                    Err(e) => { tracing::error!("TUN receive error: {:?}", e); break; }
                }
            }
        });
        rx_handle.await?;
        tracing::info!("TUN proxy stopped");
        Ok(())
    }

    async fn configure_interface(&self) -> Result<()> {
        let adapter_name = &self.config.tunnel_name;
        let output = std::process::Command::new("netsh")
            .args([
                "interface", "ip", "set", "address",
                &format!("name={}", adapter_name), "static", "10.0.0.1", "255.255.255.0", "10.0.0.1",
            ])
            .output()
            .map_err(|e| anyhow::anyhow!("Failed to run netsh: {}", e))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            tracing::warn!("netsh set address failed: {}", stderr);
        }
        Ok(())
    }
}

async fn handle_packet(
    data: &[u8],
    session: &Arc<wintun::Session>,
    connections: &Arc<Mutex<HashMap<ConnectionKey, Connection>>>,
    endpoint_group: &Arc<EndpointGroup>,
    ip_mapping: &Arc<IpMapping>,
    identification: &Arc<Mutex<u16>>,
) -> Result<()> {
    let ip_packet = match Ipv4Packet::parse(data) { Some(p) => p, None => return Ok(()) };
    if ip_packet.protocol != IPPROTO_TCP { return Ok(()); }
    let tcp_packet = match TcpPacket::parse(&ip_packet.payload) { Some(p) => p, None => return Ok(()) };
    let domain = match ip_mapping.lookup_domain(&ip_packet.dst_addr) { Some(d) => d, None => return Ok(()) };
    let conn_key = ConnectionKey {
        src_ip: u32::from(ip_packet.src_addr),
        src_port: tcp_packet.src_port,
        dst_ip: u32::from(ip_packet.dst_addr),
        dst_port: tcp_packet.dst_port,
    };
    handle_tcp(&conn_key, &tcp_packet, &ip_packet, &domain, session, connections, endpoint_group, identification).await
}

async fn handle_tcp(
    conn_key: &ConnectionKey,
    tcp: &TcpPacket,
    ip: &Ipv4Packet,
    domain: &str,
    session: &Arc<wintun::Session>,
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
        return handle_syn(conn_key, tcp, ip, domain, session, connections, endpoint_group, identification).await;
    }

    let conn_exists = connections.lock().contains_key(conn_key);
    if !conn_exists { return Ok(()); }

    let conn_state = {
        let conns = connections.lock();
        conns.get(conn_key).map(|c| (c.state, c.our_seq, c.our_ack))
    };
    let (state, our_seq, _our_ack) = match conn_state { Some(s) => s, None => return Ok(()) };

    match state {
        TcpState::SynReceived => {
            if is_ack && !has_data {
                let mut conns = connections.lock();
                if let Some(conn) = conns.get_mut(conn_key) { conn.state = TcpState::Established; }
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
                    if let Some(conn) = conns.get_mut(conn_key) { conn.nexapipe_send = send_stream; }
                }
                let new_ack = tcp.seq_num.wrapping_add(tcp.payload.len() as u32);
                let ack_packet = TcpPacket {
                    src_port: tcp.dst_port, dst_port: tcp.src_port,
                    seq_num: our_seq, ack_num: new_ack,
                    flags: tcp_flags::ACK, window: 65535, payload: Vec::new(),
                };
                let id = get_next_id(identification);
                let packet = ack_packet.build_with_ip(ip.dst_addr, ip.src_addr, id);
                send_tun_packet(session, &packet)?;
                {
                    let mut conns = connections.lock();
                    if let Some(conn) = conns.get_mut(conn_key) { conn.our_ack = new_ack; }
                }
                spawn_recv_task(conn_key.clone(), connections.clone(), ip.dst_addr, ip.src_addr, tcp.dst_port, tcp.src_port, identification.clone(), session.clone(), our_seq, new_ack);
            }
            if is_fin {
                let fin_ack = tcp.seq_num.wrapping_add(tcp.payload.len() as u32 + 1);
                let packet = TcpPacket {
                    src_port: tcp.dst_port, dst_port: tcp.src_port,
                    seq_num: our_seq, ack_num: fin_ack,
                    flags: tcp_flags::FIN | tcp_flags::ACK, window: 65535, payload: Vec::new(),
                };
                let id = get_next_id(identification);
                let data = packet.build_with_ip(ip.dst_addr, ip.src_addr, id);
                send_tun_packet(session, &data)?;
                {
                    let mut conns = connections.lock();
                    if let Some(conn) = conns.get_mut(conn_key) { conn.state = TcpState::LastAck; conn.our_ack = fin_ack; }
                }
            }
        }
        TcpState::LastAck => {
            if is_ack { connections.lock().remove(conn_key); }
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
    session: &Arc<wintun::Session>,
    connections: &Arc<Mutex<HashMap<ConnectionKey, Connection>>>,
    endpoint_group: &Arc<EndpointGroup>,
    identification: &Arc<Mutex<u16>>,
) -> Result<()> {
    let pooled_conn = match endpoint_group.get_connection(domain).await {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("Failed to get nexapipe connection: {}", e);
            let rst_packet = TcpPacket {
                src_port: tcp.dst_port, dst_port: tcp.src_port, seq_num: 0,
                ack_num: tcp.seq_num.wrapping_add(1), flags: tcp_flags::RST | tcp_flags::ACK, window: 0, payload: Vec::new(),
            };
            let id = get_next_id(identification);
            let data = rst_packet.build_with_ip(ip.dst_addr, ip.src_addr, id);
            let _ = send_tun_packet(session, &data);
            return Ok(());
        }
    };

    let conn = pooled_conn.into_inner();

    let (mut send, recv) = match conn.open_bi().await {
        Ok(streams) => streams,
        Err(e) => {
            tracing::error!("Failed to open nexapipe stream: {}", e);
            let rst_packet = TcpPacket {
                src_port: tcp.dst_port, dst_port: tcp.src_port, seq_num: 0,
                ack_num: tcp.seq_num.wrapping_add(1), flags: tcp_flags::RST | tcp_flags::ACK, window: 0, payload: Vec::new(),
            };
            let id = get_next_id(identification);
            let data = rst_packet.build_with_ip(ip.dst_addr, ip.src_addr, id);
            let _ = send_tun_packet(session, &data);
            return Ok(());
        }
    };

    let connect_request = format!("CONNECT {}:{} HTTP/1.1\r\nHost: {}:{}\r\n\r\n", domain, tcp.dst_port, domain, tcp.dst_port);
    if let Err(e) = send.write_all(connect_request.as_bytes()).await {
        tracing::error!("Failed to send CONNECT request: {}", e);
        return Ok(());
    }

    let our_seq = 1000u32;
    let our_ack = tcp.seq_num.wrapping_add(1);
    let syn_ack = TcpPacket {
        src_port: tcp.dst_port, dst_port: tcp.src_port,
        seq_num: our_seq, ack_num: our_ack,
        flags: tcp_flags::SYN | tcp_flags::ACK, window: 65535, payload: Vec::new(),
    };
    let id = get_next_id(identification);
    let data = syn_ack.build_with_ip(ip.dst_addr, ip.src_addr, id);
    send_tun_packet(session, &data)?;

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
    session: Arc<wintun::Session>,
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
    if recv.is_none() { return; }
    let mut recv = recv.unwrap();
    let mut our_seq = initial_seq;

    tokio::spawn(async move {
        let mut buf = [0u8; 8192];
        loop {
            match recv.read(&mut buf).await {
                Ok(None) => break,
                Ok(Some(n)) => {
                    let data = &buf[..n];
                    let tcp_packet = TcpPacket {
                        src_port: tun_port, dst_port: client_port,
                        seq_num: our_seq, ack_num: initial_ack,
                        flags: tcp_flags::PSH | tcp_flags::ACK, window: 65535, payload: data.to_vec(),
                    };
                    let id = get_next_id(&identification);
                    let packet = tcp_packet.build_with_ip(tun_ip, client_ip, id);
                    if let Err(_e) = send_tun_packet(&session, &packet) { break; }
                    our_seq = our_seq.wrapping_add(n as u32);
                }
                Err(_e) => break,
            }
        }
        let fin_packet = TcpPacket {
            src_port: tun_port, dst_port: client_port,
            seq_num: our_seq, ack_num: initial_ack,
            flags: tcp_flags::FIN | tcp_flags::ACK, window: 65535, payload: Vec::new(),
        };
        let id = get_next_id(&identification);
        let packet = fin_packet.build_with_ip(tun_ip, client_ip, id);
        let _ = send_tun_packet(&session, &packet);
        let mut conns = connections.lock();
        if let Some(conn) = conns.get_mut(&conn_key) { conn.state = TcpState::FinWait1; }
    });
}

#[cfg(windows)]
fn send_tun_packet(session: &Arc<wintun::Session>, data: &[u8]) -> Result<()> {
    let mut packet = session.allocate_send_packet(data.len() as u16)
        .map_err(|e| anyhow::anyhow!("Failed to allocate TUN packet: {:?}", e))?;
    packet.bytes_mut().copy_from_slice(data);
    session.send_packet(packet);
    Ok(())
}

fn get_next_id(identification: &Arc<Mutex<u16>>) -> u16 {
    let mut id = identification.lock();
    let current = *id;
    *id = id.wrapping_add(1);
    current
}
