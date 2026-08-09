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

fn main() {
    tauri_build::build();

    #[cfg(target_os = "windows")]
    {
        let arch_dir = target_arch_dir();
        let wintun_src = format!("wintun/bin/{}/wintun.dll", arch_dir);
        let out_dir = env::var("OUT_DIR").unwrap();
        // OUT_DIR = target/{profile}/build/{pkg}-{hash}/out，向上三级即 target/{profile}（exe 所在目录）。
        // 注意：向上两级得到的是 target/{profile}/build，不是 exe 目录——
        // wintun-bindings 的 load_from_path("wintun.dll") 依赖 Windows DLL 搜索顺序命中 exe 目录。
        let profile_dir = Path::new(&out_dir)
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .parent()
            .unwrap();
        let build_dir = Path::new(&out_dir).parent().unwrap().parent().unwrap();

        let src_path = Path::new(&wintun_src);
        // 1) 复制到 exe 同目录（target/{profile}/wintun.dll）——系统 DLL 搜索的"应用程序目录"，
        //    使 tun crate / wintun-bindings 的 LoadLibrary("wintun.dll") 能命中签名文件。
        let exe_dst = profile_dir.join("wintun.dll");
        // 2) 保留 target/{profile}/build/ 下的副本（兼容旧布局 / WINTUN_PATH 注入）。
        let legacy_dst = build_dir
            .join("wintun")
            .join("bin")
            .join(arch_dir)
            .join("wintun.dll");

        if src_path.exists() {
            fs::create_dir_all(legacy_dst.parent().unwrap()).unwrap();
            if let Err(e) = fs::copy(src_path, &legacy_dst) {
                println!("cargo:warning=Failed to copy wintun.dll: {}", e);
            }
            // 复制到 exe 目录必须成功，否则运行时签名校验会走到错误路径。
            match fs::copy(src_path, &exe_dst) {
                Ok(_) => {
                    println!("cargo:rustc-env=WINTUN_PATH={}", exe_dst.display());
                    println!(
                        "cargo:info=Copied wintun.dll next to exe: {}",
                        exe_dst.display()
                    );
                }
                Err(e) => println!("cargo:warning=Failed to copy wintun.dll next to exe: {}", e),
            }
        } else {
            println!("cargo:warning=wintun.dll not found at {}. Please download from https://www.wintun.net/ and place it in wintun/bin/{}/", src_path.display(), arch_dir);
        }
    }
}
