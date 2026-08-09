//! 系统 DNS 配置 — 将系统 DNS 指向 TUN 虚拟 IP，使 DNS 查询进入本地 DNS 服务器。
//!
//! - Windows: netsh（与原有实现一致）
//! - Linux:   resolvectl（systemd-resolved）→ 回退直接覆写 /etc/resolv.conf（带备份）
//! - macOS:   networksetup（遍历所有网络服务）
//!
//! 注意：本地 DNS 服务器（proxy/dns.rs）必须先于 `set_system_dns` 启动，
//! 否则切换后的系统 DNS 查询将无人应答。

use anyhow::Result;
use std::process::Command;

/// 将系统 DNS 指向 TUN 虚拟 IP。
pub fn set_system_dns(interface: &str, dns_ip: &str) -> Result<()> {
    // 仅 Linux 分支需要 interface（resolvectl），其他平台用不到
    #[cfg(not(target_os = "linux"))]
    let _ = interface;

    #[cfg(windows)]
    {
        set_system_dns_windows(dns_ip)
    }

    #[cfg(target_os = "linux")]
    {
        set_system_dns_linux(interface, dns_ip)
    }

    #[cfg(target_os = "macos")]
    {
        set_system_dns_macos(dns_ip)
    }

    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
    {
        Ok(())
    }
}

/// 恢复系统 DNS 配置（尽力而为）。
pub fn restore_system_dns(interface: &str, dns_ip: &str) -> Result<()> {
    // 仅 Linux 分支需要 interface（resolvectl revert），其他平台用不到
    #[cfg(not(target_os = "linux"))]
    let _ = interface;

    #[cfg(windows)]
    {
        restore_system_dns_windows(dns_ip)
    }

    #[cfg(target_os = "linux")]
    {
        restore_system_dns_linux(interface, dns_ip)
    }

    #[cfg(target_os = "macos")]
    {
        restore_system_dns_macos(dns_ip)
    }

    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
    {
        Ok(())
    }
}

// ============================================================
// Windows — netsh
// ============================================================

#[cfg(windows)]
fn set_system_dns_windows(dns_ip: &str) -> Result<()> {
    // 优先 netsh（快）；失败时用 PowerShell 对所有活动适配器设置，
    // 避免 netsh "all" 在某些接口上不生效导致系统 DNS 未切换
    let output = Command::new("netsh")
        .args([
            "interface",
            "ip",
            "set",
            "dnsservers",
            "all",
            dns_ip,
            "primary",
        ])
        .output()
        .map_err(|e| anyhow::anyhow!("Failed to run netsh: {}", e))?;
    if output.status.success() {
        tracing::info!("System DNS set to {} via netsh", dns_ip);
        return Ok(());
    }
    tracing::warn!(
        "netsh set dns failed (exit {:?}): stdout={} stderr={}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout).trim(),
        String::from_utf8_lossy(&output.stderr).trim()
    );

    let ps_cmd = format!(
        "Get-NetAdapter | Where-Object {{ $_.Status -eq 'Up' }} | Set-DnsClientServerAddress -ServerAddresses {}",
        dns_ip
    );
    let output = Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &ps_cmd])
        .output()
        .map_err(|e| anyhow::anyhow!("Failed to run powershell: {}", e))?;
    if output.status.success() {
        tracing::info!("System DNS set to {} via PowerShell", dns_ip);
    } else {
        tracing::warn!(
            "PowerShell set dns failed (exit {:?}): stdout={} stderr={}",
            output.status.code(),
            String::from_utf8_lossy(&output.stdout).trim(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

#[cfg(windows)]
fn restore_system_dns_windows(_dns_ip: &str) -> Result<()> {
    let output = Command::new("netsh")
        .args(["interface", "ip", "set", "dnsservers", "all", "dhcp"])
        .output()
        .map_err(|e| anyhow::anyhow!("Failed to run netsh: {}", e))?;
    if output.status.success() {
        tracing::info!("System DNS restored to DHCP");
        return Ok(());
    }
    tracing::warn!(
        "netsh restore dns failed (exit {:?}): stdout={} stderr={}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout).trim(),
        String::from_utf8_lossy(&output.stderr).trim()
    );

    // 兜底：PowerShell 恢复所有活动适配器的 DNS 为 DHCP
    let output = Command::new("powershell")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "Get-NetAdapter | Where-Object { $_.Status -eq 'Up' } | Set-DnsClientServerAddress -ResetServerAddresses",
        ])
        .output()
        .map_err(|e| anyhow::anyhow!("Failed to run powershell: {}", e))?;
    if output.status.success() {
        tracing::info!("System DNS restored to DHCP via PowerShell");
    } else {
        tracing::warn!(
            "PowerShell restore dns failed (exit {:?}): stdout={} stderr={}",
            output.status.code(),
            String::from_utf8_lossy(&output.stdout).trim(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

// ============================================================
// Linux — resolvectl → 回退 /etc/resolv.conf
// ============================================================

#[cfg(target_os = "linux")]
const RESOLV_CONF_BACKUP: &str = "/etc/resolv.conf.nexapipe.bak";

#[cfg(target_os = "linux")]
fn set_system_dns_linux(interface: &str, dns_ip: &str) -> Result<()> {
    // 优先使用 resolvectl（systemd-resolved）
    if let Ok(output) = Command::new("resolvectl")
        .args(["dns", interface, dns_ip])
        .output()
    {
        if output.status.success() {
            tracing::info!("resolvectl set DNS for {}: {}", interface, dns_ip);
            return Ok(());
        }
    }

    // 回退：备份并覆写 /etc/resolv.conf
    let _ = std::fs::copy("/etc/resolv.conf", RESOLV_CONF_BACKUP);
    std::fs::write("/etc/resolv.conf", format!("nameserver {}\n", dns_ip))
        .map_err(|e| anyhow::anyhow!("Failed to write /etc/resolv.conf: {}", e))?;
    tracing::info!("Wrote /etc/resolv.conf with nameserver {}", dns_ip);
    Ok(())
}

#[cfg(target_os = "linux")]
fn restore_system_dns_linux(interface: &str, _dns_ip: &str) -> Result<()> {
    // 优先 revert resolvectl
    let _ = Command::new("resolvectl")
        .args(["revert", interface])
        .output();

    // 若存在 resolv.conf 备份则恢复
    if let Ok(backup) = std::fs::read(RESOLV_CONF_BACKUP) {
        let _ = std::fs::write("/etc/resolv.conf", backup);
        let _ = std::fs::remove_file(RESOLV_CONF_BACKUP);
    }
    Ok(())
}

// ============================================================
// macOS — networksetup
// ============================================================

#[cfg(target_os = "macos")]
fn set_system_dns_macos(dns_ip: &str) -> Result<()> {
    let services = network_services()?;
    if services.is_empty() {
        tracing::warn!("No network services found via networksetup");
        return Ok(());
    }
    for service in &services {
        match Command::new("networksetup")
            .args(["-setdnsservers", service, dns_ip])
            .output()
        {
            Ok(o) if o.status.success() => {
                tracing::info!("Set DNS for {}: {}", service, dns_ip)
            }
            Ok(o) => tracing::warn!(
                "networksetup failed for {}: {}",
                service,
                String::from_utf8_lossy(&o.stderr).trim()
            ),
            Err(e) => tracing::warn!("Failed to run networksetup for {}: {}", service, e),
        }
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn restore_system_dns_macos(_dns_ip: &str) -> Result<()> {
    let services = network_services().unwrap_or_default();
    for service in &services {
        // "Empty" 恢复为 DHCP / 系统默认
        let _ = Command::new("networksetup")
            .args(["-setdnsservers", service, "Empty"])
            .output();
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn network_services() -> Result<Vec<String>> {
    let output = Command::new("networksetup")
        .arg("-listallnetworkservices")
        .output()
        .map_err(|e| {
            anyhow::anyhow!("Failed to run networksetup -listallnetworkservices: {}", e)
        })?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(stdout
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('*'))
        .map(|l| l.trim().to_string())
        .collect())
}
