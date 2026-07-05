use std::env;
use std::fs;
use std::path::Path;

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
    #[cfg(not(any(target_arch = "x86_64", target_arch = "x86", target_arch = "arm", target_arch = "aarch64")))]
    {
        "amd64"
    }
}

fn main() {
    tauri_build::build();

    #[cfg(target_os = "windows")]
    {
        let arch_dir = target_arch_dir();
        let wintun_src = format!("wintun/bin/{}/wintun.dll", arch_dir);
        let out_dir = env::var("OUT_DIR").unwrap();
        let target_dir = Path::new(&out_dir).parent().unwrap().parent().unwrap();

        let src_path = Path::new(&wintun_src);
        let dst_path = target_dir.join("wintun").join("bin").join(arch_dir).join("wintun.dll");

        if src_path.exists() {
            fs::create_dir_all(dst_path.parent().unwrap()).unwrap();
            match fs::copy(src_path, &dst_path) {
                Ok(_) => {
                    println!("cargo:rustc-env=WINTUN_PATH={}", dst_path.display());
                    println!("cargo:info=Copied wintun.dll for {} architecture", arch_dir);
                }
                Err(e) => println!("cargo:warning=Failed to copy wintun.dll: {}", e),
            }
        } else {
            println!("cargo:warning=wintun.dll not found at {}. Please download from https://www.wintun.net/ and place it in wintun/bin/{}/", src_path.display(), arch_dir);
        }
    }
}
