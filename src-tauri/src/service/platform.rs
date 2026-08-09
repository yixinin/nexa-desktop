pub const SERVICE_NAME: &str = "pipe-service";
pub const SERVICE_DISPLAY_NAME: &str = "Pipe Service";

pub fn install_service() -> Result<String, String> {
    let exe_path =
        std::env::current_exe().map_err(|e| format!("Failed to get executable path: {}", e))?;
    let exe_str = exe_path.to_string_lossy().to_string();

    #[cfg(windows)]
    return install_service_windows(&exe_str);

    #[cfg(target_os = "linux")]
    return install_service_linux(&exe_str);

    #[cfg(target_os = "macos")]
    return install_service_macos(&exe_str);

    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
    return Err("Service only supported on Windows, Linux, and macOS".to_string());
}

pub fn uninstall_service() -> Result<String, String> {
    #[cfg(windows)]
    return uninstall_service_windows();

    #[cfg(target_os = "linux")]
    return uninstall_service_linux();

    #[cfg(target_os = "macos")]
    return uninstall_service_macos();

    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
    return Err("Service only supported on Windows, Linux, and macOS".to_string());
}

pub fn is_service_running() -> bool {
    #[cfg(windows)]
    return is_service_running_windows();

    #[cfg(target_os = "linux")]
    return is_service_running_linux();

    #[cfg(target_os = "macos")]
    return is_service_running_macos();

    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
    return false;
}

#[cfg(windows)]
fn install_service_windows(exe_path: &str) -> Result<String, String> {
    use std::process::Command;

    let output = Command::new("sc")
        .args([
            "create",
            SERVICE_NAME,
            "binPath=",
            exe_path,
            "start=",
            "auto",
            "displayName=",
            SERVICE_DISPLAY_NAME,
        ])
        .output()
        .map_err(|e| format!("Failed to run sc command: {}", e))?;

    if output.status.success() {
        Ok("Service installed".to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!("Failed to install service: {}", stderr))
    }
}

#[cfg(windows)]
fn uninstall_service_windows() -> Result<String, String> {
    use std::process::Command;

    let _ = Command::new("sc").args(["stop", SERVICE_NAME]).output();

    let output = Command::new("sc")
        .args(["delete", SERVICE_NAME])
        .output()
        .map_err(|e| format!("Failed to run sc command: {}", e))?;

    if output.status.success() {
        Ok("Service uninstalled".to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!("Failed to uninstall service: {}", stderr))
    }
}

#[cfg(windows)]
fn is_service_running_windows() -> bool {
    use std::process::Command;

    let output = Command::new("sc")
        .args(["query", SERVICE_NAME])
        .output()
        .ok()
        .and_then(|o| {
            let stdout = String::from_utf8_lossy(&o.stdout);
            Some(stdout.contains("RUNNING"))
        })
        .unwrap_or(false);
    output
}

#[cfg(target_os = "linux")]
fn install_service_linux(exe_path: &str) -> Result<String, String> {
    use std::fs::{self, File};
    use std::io::Write;
    use std::process::Command;

    let unit_content = format!(
        r#"[Unit]
Description={}
After=network.target

[Service]
Type=simple
ExecStart={} --daemon
Restart=always
RestartSec=5
User=root

[Install]
WantedBy=multi-user.target
"#,
        SERVICE_DISPLAY_NAME, exe_path
    );

    let unit_path = PathBuf::from("/etc/systemd/system").join(format!("{}.service", SERVICE_NAME));

    fs::create_dir_all("/etc/systemd/system")
        .map_err(|e| format!("Failed to create systemd directory: {}", e))?;

    let mut file = File::create(&unit_path)
        .map_err(|e| format!("Failed to create systemd unit file: {}", e))?;

    file.write_all(unit_content.as_bytes())
        .map_err(|e| format!("Failed to write systemd unit file: {}", e))?;

    let output = Command::new("systemctl")
        .args(["daemon-reload"])
        .output()
        .map_err(|e| format!("Failed to run systemctl daemon-reload: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Failed to reload systemd: {}", stderr));
    }

    let output = Command::new("systemctl")
        .args(["enable", SERVICE_NAME])
        .output()
        .map_err(|e| format!("Failed to run systemctl enable: {}", e))?;

    if output.status.success() {
        Ok("Service installed".to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!("Failed to enable service: {}", stderr))
    }
}

#[cfg(target_os = "linux")]
fn uninstall_service_linux() -> Result<String, String> {
    use std::fs;
    use std::path::Path;
    use std::process::Command;

    let _ = Command::new("systemctl")
        .args(["stop", SERVICE_NAME])
        .output();

    let _ = Command::new("systemctl")
        .args(["disable", SERVICE_NAME])
        .output();

    let unit_path = Path::new("/etc/systemd/system").join(format!("{}.service", SERVICE_NAME));
    let _ = fs::remove_file(unit_path);

    let _ = Command::new("systemctl").args(["daemon-reload"]).output();

    Ok("Service uninstalled".to_string())
}

#[cfg(target_os = "linux")]
fn is_service_running_linux() -> bool {
    use std::process::Command;

    let output = Command::new("systemctl")
        .args(["is-active", SERVICE_NAME])
        .output()
        .ok()?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout.trim() == "active"
}

#[cfg(target_os = "macos")]
fn install_service_macos(exe_path: &str) -> Result<String, String> {
    use std::fs::{self, File};
    use std::io::Write;
    use std::process::Command;

    let plist_content = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>com.pipe.{}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{}</string>
        <string>--daemon</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <key>StandardOutPath</key>
    <string>/var/log/pipe-service.log</string>
    <key>StandardErrorPath</key>
    <string>/var/log/pipe-service.log</string>
</dict>
</plist>
"#,
        SERVICE_NAME, exe_path
    );

    let plist_path =
        PathBuf::from("/Library/LaunchDaemons").join(format!("com.pipe.{}.plist", SERVICE_NAME));

    fs::create_dir_all("/Library/LaunchDaemons")
        .map_err(|e| format!("Failed to create LaunchDaemons directory: {}", e))?;

    let mut file =
        File::create(&plist_path).map_err(|e| format!("Failed to create plist file: {}", e))?;

    file.write_all(plist_content.as_bytes())
        .map_err(|e| format!("Failed to write plist file: {}", e))?;

    let output = Command::new("launchctl")
        .args(["load", plist_path.to_str().unwrap()])
        .output()
        .map_err(|e| format!("Failed to run launchctl load: {}", e))?;

    if output.status.success() {
        Ok("Service installed".to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!("Failed to load launchd plist: {}", stderr))
    }
}

#[cfg(target_os = "macos")]
fn uninstall_service_macos() -> Result<String, String> {
    use std::fs;
    use std::path::Path;
    use std::process::Command;

    let plist_path =
        Path::new("/Library/LaunchDaemons").join(format!("com.pipe.{}.plist", SERVICE_NAME));

    let _ = Command::new("launchctl")
        .args(["unload", plist_path.to_str().unwrap()])
        .output();
    let _ = fs::remove_file(&plist_path);

    Ok("Service uninstalled".to_string())
}

#[cfg(target_os = "macos")]
fn is_service_running_macos() -> bool {
    use std::process::Command;

    let output = Command::new("launchctl")
        .args(["list", format!("com.pipe.{}", SERVICE_NAME).as_str()])
        .output()
        .ok()?;

    output.status.success()
        && !String::from_utf8_lossy(&output.stdout).contains("Could not find service")
}

pub fn start_service() -> Result<String, String> {
    #[cfg(windows)]
    return start_service_windows();

    #[cfg(target_os = "linux")]
    return start_service_linux();

    #[cfg(target_os = "macos")]
    return start_service_macos();

    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
    return Err("Service only supported on Windows, Linux, and macOS".to_string());
}

pub fn stop_service() -> Result<String, String> {
    #[cfg(windows)]
    return stop_service_windows();

    #[cfg(target_os = "linux")]
    return stop_service_linux();

    #[cfg(target_os = "macos")]
    return stop_service_macos();

    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
    return Err("Service only supported on Windows, Linux, and macOS".to_string());
}

#[cfg(windows)]
fn start_service_windows() -> Result<String, String> {
    use std::process::Command;

    let output = Command::new("sc")
        .args(["start", SERVICE_NAME])
        .output()
        .map_err(|e| format!("Failed to run sc command: {}", e))?;

    if output.status.success() {
        Ok("Service started".to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!("Failed to start service: {}", stderr))
    }
}

#[cfg(windows)]
fn stop_service_windows() -> Result<String, String> {
    use std::process::Command;

    let output = Command::new("sc")
        .args(["stop", SERVICE_NAME])
        .output()
        .map_err(|e| format!("Failed to run sc command: {}", e))?;

    if output.status.success() {
        Ok("Service stopped".to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!("Failed to stop service: {}", stderr))
    }
}

#[cfg(target_os = "linux")]
fn start_service_linux() -> Result<String, String> {
    use std::process::Command;

    let output = Command::new("systemctl")
        .args(["start", SERVICE_NAME])
        .output()
        .map_err(|e| format!("Failed to run systemctl start: {}", e))?;

    if output.status.success() {
        Ok("Service started".to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!("Failed to start service: {}", stderr))
    }
}

#[cfg(target_os = "linux")]
fn stop_service_linux() -> Result<String, String> {
    use std::process::Command;

    let output = Command::new("systemctl")
        .args(["stop", SERVICE_NAME])
        .output()
        .map_err(|e| format!("Failed to run systemctl stop: {}", e))?;

    if output.status.success() {
        Ok("Service stopped".to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!("Failed to stop service: {}", stderr))
    }
}

#[cfg(target_os = "macos")]
fn start_service_macos() -> Result<String, String> {
    use std::path::Path;
    use std::process::Command;

    let plist_path =
        Path::new("/Library/LaunchDaemons").join(format!("com.pipe.{}.plist", SERVICE_NAME));

    let output = Command::new("launchctl")
        .args(["start", format!("com.pipe.{}", SERVICE_NAME).as_str()])
        .output()
        .map_err(|e| format!("Failed to run launchctl start: {}", e))?;

    if output.status.success() {
        Ok("Service started".to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!("Failed to start service: {}", stderr))
    }
}

#[cfg(target_os = "macos")]
fn stop_service_macos() -> Result<String, String> {
    use std::process::Command;

    let output = Command::new("launchctl")
        .args(["stop", format!("com.pipe.{}", SERVICE_NAME).as_str()])
        .output()
        .map_err(|e| format!("Failed to run launchctl stop: {}", e))?;

    if output.status.success() {
        Ok("Service stopped".to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!("Failed to stop service: {}", stderr))
    }
}
