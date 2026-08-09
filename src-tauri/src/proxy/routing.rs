//! 跨平台 TUN 接口地址与路由管理。
//!
//! 创建 TUN 设备后调用 `configure_interface` + `add_routes` 使虚拟网段可路由，
//! 停止时调用 `remove_routes` 清理。所有命令均为幂等操作，重复执行不会报错。
//!
//! - Windows: netsh（wintun 适配器需显式配置 IP）
//! - Linux:   ip addr / ip link / ip route（内核自动添加直连路由，显式 replace 兜底）
//! - macOS:   ifconfig / route（utun 接口需显式配置 IP 与路由）

use anyhow::Result;
use std::process::Command;

#[cfg(windows)]
use crate::proxy::tun_proxy::{TUN_IP, TUN_NETMASK};
#[cfg(target_os = "macos")]
use crate::proxy::tun_proxy::{TUN_IP, TUN_NETMASK};
#[cfg(target_os = "linux")]
use crate::proxy::tun_proxy::{TUN_IP, TUN_NETWORK};
#[cfg(windows)]
use std::net::UdpSocket;
#[cfg(windows)]
use std::time::Duration;

/// 配置 TUN 接口的 IP 地址并启用接口（幂等）。
pub fn configure_interface(interface: &str) -> Result<()> {
    #[cfg(windows)]
    {
        configure_interface_windows(interface)
    }

    #[cfg(target_os = "linux")]
    {
        configure_interface_linux(interface)
    }

    #[cfg(target_os = "macos")]
    {
        configure_interface_macos(interface)
    }

    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
    {
        Ok(())
    }
}

/// 确保虚拟网段路由存在（幂等）。设置接口地址后内核通常会自动添加直连路由，
/// 这里在部分平台上显式补充，防止自动路由缺失。
pub fn add_routes(interface: &str) -> Result<()> {
    #[cfg(windows)]
    {
        // netsh set address 已自动添加 10.0.0.0/24 直连路由
        let _ = interface;
        Ok(())
    }

    #[cfg(target_os = "linux")]
    {
        let output = Command::new("ip")
            .args(["route", "replace", TUN_NETWORK, "dev", interface])
            .output()
            .map_err(|e| anyhow::anyhow!("Failed to run ip route: {}", e))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("ip route replace failed: {}", stderr.trim());
        }
        Ok(())
    }

    #[cfg(target_os = "macos")]
    {
        // 路由已存在时返回 "File exists"，视为成功
        let output = Command::new("route")
            .args([
                "-n",
                "add",
                "-net",
                "10.0.0.0",
                "-netmask",
                TUN_NETMASK,
                "-interface",
                interface,
            ])
            .output()
            .map_err(|e| anyhow::anyhow!("Failed to run route: {}", e))?;
        if output.status.success()
            || String::from_utf8_lossy(&output.stderr).contains("File exists")
        {
            Ok(())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("route add failed: {}", stderr.trim())
        }
    }

    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
    {
        Ok(())
    }
}

/// 移除 TUN 路由（尽力而为，设备销毁时路由也会自动消失）。
pub fn remove_routes(interface: &str) -> Result<()> {
    #[cfg(windows)]
    {
        let _ = interface;
        Ok(())
    }

    #[cfg(target_os = "linux")]
    {
        let _ = Command::new("ip")
            .args(["route", "del", TUN_NETWORK, "dev", interface])
            .output();
        Ok(())
    }

    #[cfg(target_os = "macos")]
    {
        let _ = Command::new("route")
            .args([
                "-n",
                "delete",
                "-net",
                "10.0.0.0",
                "-netmask",
                TUN_NETMASK,
                "-interface",
                interface,
            ])
            .output();
        Ok(())
    }

    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
    {
        Ok(())
    }
}

#[cfg(windows)]
fn configure_interface_windows(interface: &str) -> Result<()> {
    // 最多重试 5 次：netsh 返回成功时地址应立即可用，但个别情况下 Windows
    // 需要一点时间才把地址挂到接口上，否则后续 bind 10.0.0.1:53 会报
    // WSAEADDRNOTAVAIL (10049)。每次先执行 netsh，再校验地址确实可绑定。
    for attempt in 1..=5 {
        let output = Command::new("netsh")
            .args([
                "interface",
                "ip",
                "set",
                "address",
                &format!("name={}", interface),
                "static",
                TUN_IP,
                TUN_NETMASK,
            ])
            .output()
            .map_err(|e| anyhow::anyhow!("Failed to run netsh: {}", e))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
            anyhow::bail!(
                "netsh set address failed (attempt {}): {} {}",
                attempt,
                stderr,
                stdout
            );
        }

        // 校验：10.0.0.1 必须已是本机可绑定地址（与 DNS 服务器绑定前提一致）。
        if is_local_address_bindable(TUN_IP) {
            tracing::info!(
                "Interface {} configured with {} netmask {}",
                interface,
                TUN_IP,
                TUN_NETMASK
            );
            return Ok(());
        }

        tracing::warn!(
            "netsh set address returned success but {} is not bindable yet (attempt {}), retrying...",
            TUN_IP,
            attempt
        );
        std::thread::sleep(Duration::from_millis(500));
    }

    anyhow::bail!(
        "Failed to assign {} to interface {} after 5 attempts: the TUN address is not local, DNS server cannot bind and routing will not work",
        TUN_IP,
        interface
    )
}

#[cfg(windows)]
fn is_local_address_bindable(ip: &str) -> bool {
    // 绑定一个临时 UDP socket 到该地址（端口 0 随机）。成功说明该地址
    // 确实挂在某个本地接口上；失败（通常是 WSAEADDRNOTAVAIL 10049）
    // 说明地址尚未就绪。
    UdpSocket::bind((ip, 0)).is_ok()
}

#[cfg(target_os = "linux")]
fn configure_interface_linux(interface: &str) -> Result<()> {
    // ip addr replace 是幂等操作，重复设置同一地址不会报错
    let output = Command::new("ip")
        .args([
            "addr",
            "replace",
            &format!("{}/24", TUN_IP),
            "dev",
            interface,
        ])
        .output()
        .map_err(|e| anyhow::anyhow!("Failed to run ip addr: {}", e))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("ip addr replace failed: {}", stderr.trim());
    }

    let output = Command::new("ip")
        .args(["link", "set", "dev", interface, "up"])
        .output()
        .map_err(|e| anyhow::anyhow!("Failed to run ip link: {}", e))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("ip link set up failed: {}", stderr.trim());
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn configure_interface_macos(interface: &str) -> Result<()> {
    // ifconfig utunX inet 10.0.0.1 255.255.255.0 — 重复执行会替换现有地址（幂等）
    let output = Command::new("ifconfig")
        .args([interface, "inet", TUN_IP, TUN_NETMASK])
        .output()
        .map_err(|e| anyhow::anyhow!("Failed to run ifconfig: {}", e))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("ifconfig inet failed: {}", stderr.trim());
    }

    let output = Command::new("ifconfig")
        .args([interface, "up"])
        .output()
        .map_err(|e| anyhow::anyhow!("Failed to run ifconfig up: {}", e))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("ifconfig up failed: {}", stderr.trim());
    }
    Ok(())
}
