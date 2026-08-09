//! 临时诊断程序：创建 wintun 适配器 → netsh 配置 10.0.0.1 → 校验可绑定 → 清理。
//! 仅用于验证 TUN 模式下 DNS 绑定 WSAEADDRNOTAVAIL (10049) 的根因与修复。
use std::net::UdpSocket;
use std::path::PathBuf;
use std::time::Duration;
use tun::AbstractDevice;

fn find_wintun() -> Option<PathBuf> {
    let exe_dir = std::env::current_exe()
        .ok()?
        .parent()
        .map(|p| p.to_path_buf())?;
    let arch = if cfg!(target_arch = "x86_64") {
        "amd64"
    } else {
        "x86"
    };
    for p in [
        exe_dir.join("wintun.dll"),
        exe_dir
            .join("wintun")
            .join("bin")
            .join(arch)
            .join("wintun.dll"),
        PathBuf::from(
            r"C:\Users\eason\rust\nexapipe\ui-desktop\src-tauri\target\debug\wintun.dll",
        ),
    ] {
        if p.exists() {
            return Some(p);
        }
    }
    None
}

fn bind_check(ip: &str) -> bool {
    UdpSocket::bind((ip, 0)).is_ok()
}

fn run_netsh(name: &str, args: &[&str]) -> std::io::Result<bool> {
    let mut cmd = std::process::Command::new("netsh");
    cmd.args(["interface", "ip", "set", "address", &format!("name={}", name)]);
    cmd.args(args);
    let out = cmd.output()?;
    println!(
        "  netsh {} -> success={} stdout=[{}] stderr=[{}]",
        args.join(" "),
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).trim(),
        String::from_utf8_lossy(&out.stderr).trim()
    );
    Ok(out.status.success())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let wt = find_wintun().ok_or("wintun.dll not found")?;
    println!("wintun.dll: {}", wt.display());

    let mut cfg = tun::Configuration::default();
    cfg.platform_config(|pc| {
        pc.wintun_file(wt.clone());
    });
    cfg.tun_name("pipe-tun-probe");
    cfg.mtu(1500);
    let dev = tun::create_as_async(&cfg)?;
    let name = dev.tun_name()?;
    println!("device ready: {} mtu={}", name, dev.mtu().unwrap_or(1500));

    // 场景 A：旧命令（带自指网关 10.0.0.1）
    println!("[A] with gateway (old command):");
    let _ = run_netsh(&name, &["static", "10.0.0.1", "255.255.255.0", "10.0.0.1"])?;
    std::thread::sleep(Duration::from_millis(300));
    println!("  bind 10.0.0.1:0 -> {}", bind_check("10.0.0.1"));
    println!(
        "  bind 10.0.0.1:53 -> {:?}",
        UdpSocket::bind("10.0.0.1:53")
            .map(|_| "OK")
            .map_err(|e| e.to_string())
    );

    // 场景 B：新命令（无网关）
    println!("[B] without gateway (new command):");
    let _ = run_netsh(&name, &["static", "10.0.0.1", "255.255.255.0"])?;
    std::thread::sleep(Duration::from_millis(300));
    println!("  bind 10.0.0.1:0 -> {}", bind_check("10.0.0.1"));
    println!(
        "  bind 10.0.0.1:53 -> {:?}",
        UdpSocket::bind("10.0.0.1:53")
            .map(|_| "OK")
            .map_err(|e| e.to_string())
    );

    // 场景 C：重试语义（连续 5 次 set + bind 校验，模拟新 configure_interface）
    println!("[C] retry loop (5x):");
    let mut ok = false;
    for attempt in 1..=5 {
        let _ = run_netsh(&name, &["static", "10.0.0.1", "255.255.255.0"])?;
        if bind_check("10.0.0.1") {
            println!("  attempt {}: 10.0.0.1 bindable", attempt);
            ok = true;
            break;
        }
        println!("  attempt {}: not bindable, sleep 500ms", attempt);
        std::thread::sleep(Duration::from_millis(500));
    }
    println!("[C] final bindable: {}", ok);

    println!("done, dropping device (adapter will be deleted)");
    drop(dev);
    std::thread::sleep(Duration::from_millis(800));
    Ok(())
}